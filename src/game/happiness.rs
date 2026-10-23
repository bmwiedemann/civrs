//! Citizen happiness, civil disorder and "We love the King day" celebrations.
//!
//! A port of the happiness section of the city routine (CIV.EXE 0xf3f8..0xfbe6). The original
//! keeps two counters, happy (ds:0x70e2) and unhappy (ds:0x70e4); content citizens are the
//! working citizens that are neither. Each step changes the counters directly and then calls
//! `rebalance_moods` (0xc91:0x6df4, [`Moods::rebalance`]), which clamps them and lets a surplus
//! over the working citizens cancel out. Order: base (difficulty and empire size, then the
//! specialists), luxuries, buildings (Colosseum, Cathedral, Temple), martial law or military
//! unhappiness, wonders. Each step the HAPPY tab of the city screen draws as a row of
//! citizens is recorded in [`Happiness::steps`].

use super::map::CityId;
use super::Game;
use crate::rules::{self, advance_ids as adv, improvement_ids as imp, unit_ids, wonder_ids as won, Govt, ImprovementId};

/// One row of the city screen's HAPPY tab and the pictograms the original draws right of its
/// citizens (CIV.EXE 0xf505..0xfbd0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HappyRow {
    /// Difficulty and empire size, specialists taken out (always drawn).
    Base,
    /// Two luxuries per happy citizen; drawn only when someone is happy (luxury icon at x 208).
    Luxuries,
    /// Drawn when the city has a Temple, Colosseum or Cathedral: the pictograms of the
    /// Colosseum, the Cathedral (only once Religion is known) and the Temple, from x 208 16 px
    /// apart.
    Buildings(Vec<ImprovementId>),
    /// Despotism..Communism (always drawn): the unit types of the first three units imposing
    /// martial law, drawn from x 208 leftwards 2 px apart.
    MartialLaw(Vec<u8>),
    /// Republic/Democracy (always drawn): one unhappy face per citizen made unhappy by units
    /// abroad, from x 208 leftwards 2 px apart.
    Military(i32),
    /// "WONDERS"; drawn only when the wonders changed happy − unhappy.
    Wonders,
}

impl HappyRow {
    pub fn label(&self) -> &'static str {
        match self {
            HappyRow::Base => "Base",
            HappyRow::Luxuries => "Luxuries",
            HappyRow::Buildings(_) => "Buildings",
            HappyRow::MartialLaw(_) => "Martial law",
            HappyRow::Military(_) => "Military",
            HappyRow::Wonders => "Wonders",
        }
    }
}

/// The citizens after one step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HappyStep {
    pub row: HappyRow,
    pub happy: i32,
    pub content: i32,
    pub unhappy: i32,
    /// Unhappiness beyond the city size still pending (ds:0x6542): the first `extra` unhappy
    /// figures are drawn with colour 5 recoloured to 12 (0xc91:0x6eca).
    pub extra: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Happiness {
    pub happy: i32,
    pub content: i32,
    pub unhappy: i32,
    /// ds:0x6542 after the last step (see [`HappyStep::extra`]).
    pub extra: i32,
    pub disorder: bool,
    pub celebration: bool,
    /// The rows of the city screen's HAPPY tab, in order (at most 5).
    pub steps: Vec<HappyStep>,
}

/// The happiness counters of the original while one city is evaluated.
#[derive(Clone, Copy, Debug)]
struct Moods {
    /// ds:0x70e2
    happy: i32,
    /// ds:0x70e4
    unhappy: i32,
    /// ds:0x6542: unhappiness the base step asked for beyond the city size.
    extra: i32,
    size: i32,
    specialists: i32,
}

impl Moods {
    /// `rebalance_moods(city, specialists)` (CIV.EXE 0x13704 = 0xc91:0x6df4): clamps happy to
    /// 0..size; while extra unhappiness is pending and fewer citizens are unhappy than that,
    /// one of it becomes an unhappy citizen; clamps unhappy to 0..size; then, while happy +
    /// unhappy exceed the working citizens, removes one unhappy citizen together with one extra
    /// (while any is left) or else one happy citizen.
    fn rebalance(&mut self) {
        let size = self.size;
        self.happy = self.happy.clamp(0, size);
        while self.extra != 0 && self.unhappy < self.extra {
            self.extra -= 1;
            self.unhappy += 1;
        }
        self.unhappy = self.unhappy.clamp(0, size);
        let workers = (size - self.specialists).clamp(0, 99);
        while self.happy + self.unhappy > workers {
            if self.extra != 0 {
                self.extra -= 1;
            } else {
                self.happy = (self.happy - 1).clamp(0, size);
            }
            self.unhappy = (self.unhappy - 1).clamp(0, size);
        }
    }

    fn content(&self) -> i32 {
        (self.size - self.specialists - self.happy - self.unhappy).max(0)
    }

    fn step(&self, row: HappyRow) -> HappyStep {
        HappyStep { row, happy: self.happy, content: self.content(), unhappy: self.unhappy, extra: self.extra }
    }
}

impl Game {
    /// Number of cities a human civilization can have before the empire-size penalty starts:
    /// (7 − level) * (government / 2 + 2), i.e. ×2 under Anarchy/Despotism, ×3 under
    /// Monarchy/Communism, ×4 under the Republic/Democracy (CIV.EXE 0xf40a).
    pub fn empire_size_threshold(&self, slot: super::Slot) -> i32 {
        let diff = self.setup.difficulty as i32;
        ((14 - 2 * diff) / 2) * (self.player(slot).govt as i32 / 2 + 2)
    }

    /// Unhappy citizens the city starts from, before any clamping (CIV.EXE 0xf3fe..0xf496).
    /// Human: size + (cities + city % t − t) / t + level − 6 with t the
    /// [empire-size threshold](Self::empire_size_threshold), i.e. 6 − level content citizens
    /// and one more unhappy citizen for each further t cities, the city slot number spreading
    /// the step over the cities (a city's penalty rises once cities + slot % t reach 2t, 3t,
    /// ...). AI: size − 3, whatever the level and the empire size.
    pub fn base_unhappy(&self, c: CityId) -> i32 {
        let city = self.city(c);
        let size = city.size as i32;
        let p = self.player(city.owner);
        if !p.human {
            return size - 3;
        }
        let t = self.empire_size_threshold(city.owner);
        let cities = self.all_cities().filter(|o| o.owner == city.owner).count() as i32;
        let content = rules::DIFFICULTY[self.setup.difficulty as usize].content_base as i32;
        // integer division truncates towards zero as `idiv` does
        size + (cities + c as i32 % t - t) / t - content
    }

    /// Unhappy citizens a Temple pacifies in this city (CIV.EXE 0xf6ca): 2 with Mysticism, else
    /// 1 with Ceremonial Burial; the Oracle adds the same again, but at least 1 (it adds 2 with
    /// Mysticism, else 1, even without Ceremonial Burial).
    pub fn temple_effect(&self, c: CityId) -> i32 {
        let city = self.city(c);
        let p = self.player(city.owner);
        let mystic = p.knows(adv::MYSTICISM);
        let mut t = if mystic {
            2
        } else if p.knows(adv::CEREMONIAL_BURIAL) {
            1
        } else {
            0
        };
        if self.owner_has_wonder(city.owner, won::ORACLE) {
            t += if mystic { 2 } else { 1 };
        }
        t
    }

    /// Unhappy citizens a Cathedral pacifies in this city: 4, or 6 with Michelangelo's Chapel;
    /// nothing while the owner does not know Religion (CIV.EXE 0xf625).
    pub fn cathedral_effect(&self, c: CityId) -> i32 {
        let city = self.city(c);
        if !city.has_improvement(imp::CATHEDRAL) || !self.player(city.owner).knows(adv::RELIGION) {
            return 0;
        }
        if self.owner_has_wonder(city.owner, won::MICHELANGELOS_CHAPEL) {
            6
        } else {
            4
        }
    }

    /// The city lies on the continent of the city holding wonder `w`.
    pub fn same_continent_as_wonder(&self, c: CityId, w: rules::WonderId) -> bool {
        let Some(wc) = self.wonders_built[w as usize].and_then(|wc| self.city_opt(wc)) else { return false };
        let city = self.city(c);
        self.map.tile(wc.x, wc.y).continent == self.map.tile(city.x, city.y).continent
    }

    /// Unit types of the owner's units in the city square that impose martial law: every unit
    /// with an attack strength, ships and aircraft included, in stack order (CIV.EXE 0xf818
    /// walks at most 32 units of the stack). The original also counts the two garrison units
    /// stashed in the record of a human city (+0x1a/+0x1b, 0xf900); civrs keeps those units on
    /// the map, so they are counted here directly.
    pub fn martial_law_kinds(&self, c: CityId) -> Vec<u8> {
        let city = self.city(c);
        self.all_units().filter(|u| (u.x, u.y) == (city.x, city.y) && u.owner == city.owner).take(32).filter(|u| u.info().att > 0).map(|u| u.kind).collect()
    }

    /// Military units of the owner inside the city that impose martial law (capped at 3 by
    /// the caller).
    pub fn martial_law_units(&self, c: CityId) -> i32 {
        self.martial_law_kinds(c).len() as i32
    }

    /// Units supported by the city that count as "abroad" (ds:0x6546).
    pub fn aggressive_units_abroad(&self, c: CityId) -> i32 {
        self.all_units().filter(|u| u.home == Some(c) && self.aggressive_abroad(c, u)).count() as i32
    }

    /// The unit, supported by city `c`, counts as abroad for the Republic/Democracy
    /// unhappiness (CIV.EXE 0xd33f): a unit type before the Diplomat with an attack strength
    /// that is an aircraft (anywhere, even at home) or stands outside its home city's square
    /// (in any other city or fortress too).
    pub fn aggressive_abroad(&self, c: CityId, u: &super::Unit) -> bool {
        let city = self.city(c);
        u.kind < unit_ids::DIPLOMAT && u.info().att > 0 && (u.is_air() || (u.x, u.y) != (city.x, city.y))
    }

    /// Unhappy citizens per unit abroad under the owner's government (CIV.EXE 0xf98e): 1 under
    /// the Republic, 2 under Democracy, one less with Women's Suffrage; 0 otherwise.
    pub fn unhappy_per_unit_abroad(&self, slot: super::Slot) -> i32 {
        let govt = self.player(slot).govt;
        if (govt as u8) < Govt::Republic as u8 {
            return 0;
        }
        let mut k = if self.owner_has_wonder(slot, won::WOMENS_SUFFRAGE) { 0 } else { 1 };
        if govt == Govt::Democracy {
            k += 1;
        }
        k
    }

    /// Unhappy citizens a unit supported by city `c` causes, as the supported-units panel shows
    /// them (CIV.EXE 0x10030).
    pub fn unit_unhappiness(&self, c: CityId, u: &super::Unit) -> i32 {
        if !self.aggressive_abroad(c, u) {
            return 0;
        }
        self.unhappy_per_unit_abroad(self.city(c).owner)
    }

    pub fn city_happiness(&self, c: CityId) -> Happiness {
        let city = self.city(c);
        let slot = city.owner;
        let p = self.player(slot);
        let mut m = Moods { happy: 0, unhappy: self.base_unhappy(c), extra: 0, size: city.size as i32, specialists: city.specialists.len() as i32 };
        let mut steps = Vec::new();
        // base: unhappiness beyond the city size is kept aside (0xf496), then the specialists
        // leave the working citizens (0xf4d9)
        if m.unhappy > m.size {
            m.extra = m.unhappy - m.size;
            m.unhappy = m.size;
        }
        m.rebalance();
        steps.push(m.step(HappyRow::Base));
        // luxuries: happy = luxuries / 2, the surplus over the content citizens turning unhappy
        // ones content (0xf543)
        m.happy = self.city_yield(c).lux / 2;
        m.rebalance();
        if m.happy != 0 {
            steps.push(m.step(HappyRow::Luxuries));
        }
        // buildings (0xf5d4): Colosseum 3, Cathedral 4/6, Temple 1/2 (+ Oracle)
        let mut pics = Vec::new();
        if city.has_improvement(imp::COLOSSEUM) {
            m.unhappy -= 3;
            pics.push(imp::COLOSSEUM);
        }
        if p.knows(adv::RELIGION) && city.has_improvement(imp::CATHEDRAL) {
            m.unhappy -= self.cathedral_effect(c);
            pics.push(imp::CATHEDRAL);
        }
        if city.has_improvement(imp::TEMPLE) {
            m.unhappy -= self.temple_effect(c);
            pics.push(imp::TEMPLE);
        }
        m.rebalance();
        if [imp::TEMPLE, imp::COLOSSEUM, imp::CATHEDRAL].iter().any(|i| city.has_improvement(*i)) {
            steps.push(m.step(HappyRow::Buildings(pics)));
        }
        // military (0xf7fe): martial law under Anarchy..Communism, one unhappy citizen content
        // per unit with an attack strength in the city, at most 3; under the Republic and
        // Democracy unhappiness for each unit abroad
        if (p.govt as u8) < Govt::Republic as u8 {
            let kinds = self.martial_law_kinds(c);
            let n = (kinds.len() as i32).min(3);
            m.unhappy -= n.clamp(0, m.unhappy);
            m.rebalance();
            steps.push(m.step(HappyRow::MartialLaw(kinds.into_iter().take(3).collect())));
        } else {
            let n = self.unhappy_per_unit_abroad(slot) * self.aggressive_units_abroad(c);
            m.unhappy += n;
            m.rebalance();
            steps.push(m.step(HappyRow::Military(n)));
        }
        // wonders (0xfa91), all applied before one rebalance: the Hanging Gardens and the Cure
        // for Cancer each add a happy citizen in all the owner's cities, Shakespeare's Theatre
        // leaves no unhappy citizen in its city, J.S. Bach's Cathedral takes away 2 unhappy
        // citizens on its continent
        let before = m.happy - m.unhappy;
        if self.owner_has_wonder(slot, won::HANGING_GARDENS) {
            m.happy += 1;
        }
        if self.owner_has_wonder(slot, won::CURE_FOR_CANCER) {
            m.happy += 1;
        }
        if city.has_wonder(won::SHAKESPEARES_THEATRE) && !self.wonder_obsolete(won::SHAKESPEARES_THEATRE) {
            m.unhappy = 0;
        }
        if self.owner_has_wonder(slot, won::JS_BACHS_CATHEDRAL) && self.same_continent_as_wonder(c, won::JS_BACHS_CATHEDRAL) {
            m.unhappy -= 2;
        }
        m.rebalance();
        if m.happy - m.unhappy != before {
            steps.push(m.step(HappyRow::Wonders));
        }
        // disorder (0x125eb): fewer happy than unhappy citizens; celebration (0x12972): no
        // unhappy citizen, size 3 or more, at least half (rounded up) happy, not in Anarchy.
        // Neither is evaluated for the barbarians (0x125e1).
        let disorder = slot != 0 && m.happy < m.unhappy;
        let celebration = slot != 0 && m.unhappy == 0 && m.size > 2 && m.happy >= (m.size + 1) / 2 && p.govt != Govt::Anarchy;
        Happiness { happy: m.happy, content: m.content(), unhappy: m.unhappy, extra: m.extra, disorder, celebration, steps }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{GameSetup, UnitId};
    use crate::rules::Difficulty;

    fn fixture(difficulty: Difficulty) -> (Game, CityId) {
        let mut g = Game::new(GameSetup { seed: 42, difficulty, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler: UnitId = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        (g, c)
    }

    fn emperor_fixture() -> (Game, CityId) {
        fixture(Difficulty::Emperor)
    }

    /// Size `n`, every citizen working, no luxuries, no units in the city.
    fn set_size(g: &mut Game, c: CityId, n: u8) {
        let slot = g.city(c).owner;
        g.player_mut(slot).lux = 0;
        g.player_mut(slot).tax = 60;
        g.city_mut(c).size = n;
        g.city_mut(c).specialists.clear();
        let (x, y) = g.city(c).pos();
        for id in g.units_at(x, y) {
            g.remove_unit(id);
        }
    }

    fn row<'a>(h: &'a Happiness, label: &str) -> Option<&'a HappyStep> {
        h.steps.iter().find(|s| s.row.label() == label)
    }

    #[test]
    fn rebalance_follows_the_original() {
        // happy cancels against unhappy once they exceed the working citizens
        let mut m = Moods { happy: 3, unhappy: 4, extra: 0, size: 5, specialists: 0 };
        m.rebalance();
        assert_eq!((m.happy, m.unhappy, m.content()), (2, 3, 0));
        // specialists take unhappy citizens first, consuming pending extra unhappiness
        let mut m = Moods { happy: 0, unhappy: 4, extra: 2, size: 4, specialists: 1 };
        m.rebalance();
        assert_eq!((m.happy, m.unhappy, m.extra), (0, 3, 1));
        // pending extra turns back into unhappy citizens while there are fewer unhappy
        let mut m = Moods { happy: 0, unhappy: 0, extra: 2, size: 4, specialists: 0 };
        m.rebalance();
        assert_eq!((m.unhappy, m.extra), (1, 1));
    }

    #[test]
    fn content_base_by_level_and_empire_size() {
        // a lone human city: 6 − level content citizens
        for (d, content) in [(Difficulty::Chieftain, 6), (Difficulty::Prince, 4), (Difficulty::Emperor, 2)] {
            let (mut g, c) = fixture(d);
            set_size(&mut g, c, 8);
            let h = g.city_happiness(c);
            assert_eq!((h.steps[0].content, h.steps[0].unhappy), (content, 8 - content), "{d:?}");
        }
        // Emperor under Despotism: threshold (7 − 4) * 2 = 6 cities
        let (mut g, c) = emperor_fixture();
        let slot = g.city(c).owner;
        assert_eq!(g.player(slot).govt, Govt::Despotism);
        assert_eq!(g.empire_size_threshold(slot), 6);
        g.player_mut(slot).govt = Govt::Republic;
        assert_eq!(g.empire_size_threshold(slot), 12);
        g.player_mut(slot).govt = Govt::Despotism;
        set_size(&mut g, c, 4);
        let (x, y) = g.city(c).pos();
        let mut others = Vec::new();
        for i in 1..12 {
            others.push(g.add_city(slot, format!("C{i}"), x + 3 * i, y + 20));
        }
        // 12 cities: (12 + slot % 6 − 6) / 6 = 1 for every city
        assert_eq!(g.base_unhappy(c), 4 + 1 + 4 - 6);
        // a city's penalty grows once cities + slot % 6 reach 12
        let last = *others.last().unwrap();
        g.remove_city(last);
        let expect = |id: CityId| 4 + (11 + id as i32 % 6 - 6) / 6 + 4 - 6;
        g.city_mut(others[0]).size = 4;
        assert_eq!(g.base_unhappy(others[0]), expect(others[0]));
        // AI cities: three content citizens, whatever the level
        g.player_mut(slot).human = false;
        assert_eq!(g.base_unhappy(c), 4 - 3);
    }

    #[test]
    fn extra_unhappiness_beyond_the_size() {
        let (mut g, c) = emperor_fixture();
        let slot = g.city(c).owner;
        set_size(&mut g, c, 1);
        let (x, y) = g.city(c).pos();
        for i in 1..30 {
            g.add_city(slot, format!("C{i}"), x + 3 * (i % 20), (y + 10 + 2 * (i / 20)) % 50);
        }
        // 30 cities, threshold 6: unhappy 1 + (30 + c % 6 − 6) / 6 − 2 >= 3
        let base = g.base_unhappy(c);
        assert_eq!(base, 1 + (30 + c as i32 % 6 - 6) / 6 - 2);
        assert!(base >= 3, "{base}");
        let h = g.city_happiness(c);
        // unhappy 1, extra 2; the rebalance then turns one extra into an unhappy citizen that
        // the size clamp drops again
        assert_eq!((h.steps[0].unhappy, h.steps[0].extra), (1, base - 2));
        assert!(h.disorder);
    }

    #[test]
    fn disorder_at_size_six_without_temple() {
        let (mut g, c) = emperor_fixture();
        let slot = g.city(c).owner;
        set_size(&mut g, c, 6);
        let h = g.city_happiness(c);
        assert!(h.disorder, "{h:?}");
        assert_eq!((h.content, h.unhappy), (2, 4));
        // rows: base and martial law; no luxury row (nobody happy), no buildings row
        let labels: Vec<&str> = h.steps.iter().map(|s| s.row.label()).collect();
        assert_eq!(labels, vec!["Base", "Martial law"]);
        // a temple with Mysticism plus three units restore order
        g.city_mut(c).add_improvement(imp::TEMPLE);
        g.player_mut(slot).learn(adv::MYSTICISM);
        let (x, y) = g.city(c).pos();
        for _ in 0..4 {
            g.add_unit(slot, unit_ids::PHALANX, x, y);
        }
        let h2 = g.city_happiness(c);
        assert_eq!(row(&h2, "Buildings").unwrap().row, HappyRow::Buildings(vec![imp::TEMPLE]));
        assert_eq!(row(&h2, "Buildings").unwrap().unhappy, 2);
        // martial law: at most 3 units
        assert_eq!(row(&h2, "Martial law").unwrap().row, HappyRow::MartialLaw(vec![unit_ids::PHALANX; 3]));
        assert_eq!((h2.content, h2.unhappy), (6, 0));
        assert!(!h2.disorder);
    }

    #[test]
    fn luxuries_make_content_citizens_happy_first() {
        let (mut g, c) = fixture(Difficulty::Chieftain);
        set_size(&mut g, c, 8);
        // 6 content, 2 unhappy; 8 luxuries make 4 happy
        let mut m = Moods { happy: 0, unhappy: 2, extra: 0, size: 8, specialists: 0 };
        m.rebalance();
        m.happy = 8 / 2;
        m.rebalance();
        assert_eq!((m.happy, m.content(), m.unhappy), (4, 2, 2));
        // 16 luxuries: the 6 content citizens become happy, the 2 further steps turn one
        // unhappy citizen happy (a happy and an unhappy one cancel)
        m.happy = 16 / 2;
        m.rebalance();
        assert_eq!((m.happy, m.content(), m.unhappy), (7, 0, 1));
        let h = g.city_happiness(c);
        assert_eq!(h.steps.len(), 2, "{h:?}");
    }

    #[test]
    fn units_abroad_under_the_republic() {
        let (mut g, c) = fixture(Difficulty::Chieftain);
        let slot = g.city(c).owner;
        set_size(&mut g, c, 4);
        g.player_mut(slot).govt = Govt::Democracy;
        let (x, y) = g.city(c).pos();
        let home = g.add_unit(slot, unit_ids::LEGION, x, y);
        g.unit_mut(home).home = Some(c);
        let away = g.add_unit(slot, unit_ids::LEGION, x + 1, y);
        g.unit_mut(away).home = Some(c);
        let fighter = g.add_unit(slot, unit_ids::FIGHTER, x, y);
        g.unit_mut(fighter).home = Some(c);
        // the unit at home doesn't count, the aircraft counts even at home
        assert_eq!(g.aggressive_units_abroad(c), 2);
        assert_eq!(g.unit_unhappiness(c, g.unit(away)), 2);
        let h = g.city_happiness(c);
        assert_eq!(row(&h, "Military").unwrap().row, HappyRow::Military(4));
        assert_eq!((h.content, h.unhappy), (0, 4));
        g.player_mut(slot).govt = Govt::Republic;
        assert_eq!(g.unit_unhappiness(c, g.unit(away)), 1);
    }

    #[test]
    fn temple_and_cathedral_follow_the_civilopedia() {
        let (mut g, c) = emperor_fixture();
        let slot = g.city(c).owner;
        g.player_mut(slot).advances = 0;
        g.city_mut(c).add_improvement(imp::TEMPLE);
        g.city_mut(c).add_improvement(imp::CATHEDRAL);
        assert_eq!((g.temple_effect(c), g.cathedral_effect(c)), (0, 0));
        g.player_mut(slot).learn(adv::CEREMONIAL_BURIAL);
        g.player_mut(slot).learn(adv::RELIGION);
        assert_eq!((g.temple_effect(c), g.cathedral_effect(c)), (1, 4));
        g.player_mut(slot).learn(adv::MYSTICISM);
        g.city_mut(c).add_wonder(won::MICHELANGELOS_CHAPEL);
        g.wonders_built[won::MICHELANGELOS_CHAPEL as usize] = Some(c);
        assert_eq!((g.temple_effect(c), g.cathedral_effect(c)), (2, 6));
        // Michelangelo's Chapel is no Cathedral of its own
        g.city_mut(c).remove_improvement(imp::CATHEDRAL);
        assert_eq!(g.cathedral_effect(c), 0);
        // the Oracle adds 1 even without Ceremonial Burial (0xf710)
        g.player_mut(slot).advances = 0;
        g.city_mut(c).add_wonder(won::ORACLE);
        g.wonders_built[won::ORACLE as usize] = Some(c);
        assert_eq!(g.temple_effect(c), 1);
        g.player_mut(slot).learn(adv::MYSTICISM);
        assert_eq!(g.temple_effect(c), 4);
    }

    #[test]
    fn wonder_happiness_follows_the_civilopedia() {
        let (mut g, c) = emperor_fixture();
        let slot = g.city(c).owner;
        g.player_mut(slot).advances = 0;
        set_size(&mut g, c, 1);
        let base = g.city_happiness(c);
        assert!(row(&base, "Wonders").is_none());
        // the Hanging Gardens make a citizen happy, not just content
        g.city_mut(c).add_wonder(won::HANGING_GARDENS);
        g.wonders_built[won::HANGING_GARDENS as usize] = Some(c);
        let h = g.city_happiness(c);
        assert_eq!(h.happy, base.happy + 1, "{h:?}");
        assert_eq!(h.steps.last().unwrap().row, HappyRow::Wonders);
        // J.S.Bach's Cathedral only helps cities on its continent
        let (x, y) = g.city(c).pos();
        let far = g.add_city(slot, "Far".into(), x + 6, y);
        g.city_mut(far).add_wonder(won::JS_BACHS_CATHEDRAL);
        g.wonders_built[won::JS_BACHS_CATHEDRAL as usize] = Some(far);
        let cont = g.map.tile(x, y).continent;
        g.map.get_mut(x + 6, y).unwrap().continent = cont;
        assert!(g.same_continent_as_wonder(c, won::JS_BACHS_CATHEDRAL));
        g.map.get_mut(x + 6, y).unwrap().continent = cont.wrapping_add(1);
        assert!(!g.same_continent_as_wonder(c, won::JS_BACHS_CATHEDRAL));
    }

    #[test]
    fn celebration_needs_a_government() {
        let (mut g, c) = fixture(Difficulty::Chieftain);
        let slot = g.city(c).owner;
        set_size(&mut g, c, 4);
        g.city_mut(c).add_wonder(won::HANGING_GARDENS);
        g.wonders_built[won::HANGING_GARDENS as usize] = Some(c);
        g.city_mut(c).add_wonder(won::CURE_FOR_CANCER);
        g.wonders_built[won::CURE_FOR_CANCER as usize] = Some(c);
        let h = g.city_happiness(c);
        assert_eq!((h.happy, h.unhappy), (2, 0));
        assert!(h.celebration);
        g.player_mut(slot).govt = Govt::Anarchy;
        assert!(!g.city_happiness(c).celebration);
    }
}
