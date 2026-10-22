//! Citizen happiness, civil disorder and "We love the King day" celebrations.
//!
//! Order of evaluation (as the city screen's "Happy" view shows it): base content by
//! difficulty, luxuries, buildings (Temple/Colosseum/Cathedral), wonders, martial law or
//! military unhappiness under Republic/Democracy.

use super::map::CityId;
use super::Game;
use crate::rules::{self, improvement_ids as imp, wonder_ids as won};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Happiness {
    pub happy: i32,
    pub content: i32,
    pub unhappy: i32,
    pub disorder: bool,
    pub celebration: bool,
    /// (label, happy, content, unhappy) after each step for the "Happy" city-screen view.
    pub steps: Vec<(&'static str, i32, i32, i32)>,
}

impl Happiness {
    /// Make up to `n` unhappy citizens content.
    fn pacify(&mut self, n: i32) -> i32 {
        let m = n.min(self.unhappy).max(0);
        self.unhappy -= m;
        self.content += m;
        m
    }
    /// One "step" of improvement: content -> happy, else unhappy -> content.
    fn improve_one(&mut self) -> bool {
        if self.content > 0 {
            self.content -= 1;
            self.happy += 1;
            true
        } else if self.unhappy > 0 {
            self.unhappy -= 1;
            self.content += 1;
            true
        } else {
            false
        }
    }
    /// One step of degradation: content -> unhappy, else happy -> content.
    fn worsen_one(&mut self) -> bool {
        if self.content > 0 {
            self.content -= 1;
            self.unhappy += 1;
            true
        } else if self.happy > 0 {
            self.happy -= 1;
            self.content += 1;
            true
        } else {
            false
        }
    }
}

impl Game {
    /// Citizens born content in the cities of `slot` (difficulty based; AI gets one more).
    /// The first citizen is always content [approx], otherwise a new city at Emperor would be in
    /// permanent disorder and could never build anything.
    pub fn content_base(&self, slot: super::Slot) -> i32 {
        let diff = self.setup.difficulty as usize;
        let mut base = rules::DIFFICULTY[diff].content_base as i32;
        if !self.player(slot).human {
            base += 1;
        }
        base.max(1)
    }

    /// Unhappy citizens a Temple pacifies in this city.
    pub fn temple_effect(&self, c: CityId) -> i32 {
        let city = self.city(c);
        let p = self.player(city.owner);
        let mut t = 1;
        if p.knows(rules::advance_ids::MYSTICISM) {
            t += 1;
        }
        if self.owner_has_wonder(city.owner, won::ORACLE) {
            t *= 2;
        }
        t
    }

    /// Military units of the owner inside the city that can impose martial law.
    pub fn martial_law_units(&self, c: CityId) -> i32 {
        let city = self.city(c);
        self.all_units().filter(|u| (u.x, u.y) == (city.x, city.y) && u.owner == city.owner && u.is_military() && u.is_land()).count() as i32
    }

    /// Aggressive units supported by the city that are away from home (Republic/Democracy).
    pub fn aggressive_units_abroad(&self, c: CityId) -> i32 {
        self.all_units().filter(|u| u.home == Some(c) && self.aggressive_abroad(c, u)).count() as i32
    }

    /// The unit, supported by city `c`, is a military unit with an attack away from home.
    pub fn aggressive_abroad(&self, c: CityId, u: &super::Unit) -> bool {
        let city = self.city(c);
        if !u.is_military() || u.info().att == 0 {
            return false;
        }
        let in_own_city = self.city_at(u.x, u.y).map(|cc| self.city(cc).owner == city.owner).unwrap_or(false);
        let in_fortress = self.map.tile(u.x, u.y).has(super::tile_flags::FORTRESS) && super::Map::dist((u.x, u.y), (city.x, city.y)) <= 2;
        !in_own_city && !in_fortress
    }

    /// Unhappy citizens a unit supported by city `c` causes (Republic 1, Democracy 2, one less
    /// with Women's Suffrage).
    pub fn unit_unhappiness(&self, c: CityId, u: &super::Unit) -> i32 {
        let slot = self.city(c).owner;
        let g = rules::GOVERNMENTS[self.player(slot).govt as usize];
        if g.martial_law_max > 0 || !self.aggressive_abroad(c, u) {
            return 0;
        }
        let mut per = g.unhappy_per_aggressive_unit as i32;
        if self.owner_has_wonder(slot, won::WOMENS_SUFFRAGE) {
            per -= 1;
        }
        per.max(0)
    }

    pub fn city_happiness(&self, c: CityId) -> Happiness {
        let city = self.city(c);
        let slot = city.owner;
        let p = self.player(slot);
        let citizens = city.size as i32 - city.specialists.len() as i32;
        let base = self.content_base(slot);
        let mut h = Happiness::default();
        h.content = citizens.min(base).max(0);
        h.unhappy = (citizens - h.content).max(0);
        h.steps.push(("Base", h.happy, h.content, h.unhappy));
        // luxuries: 2 per step
        let y = self.city_yield(c);
        let mut lux = y.lux;
        while lux >= 2 && h.improve_one() {
            lux -= 2;
        }
        h.steps.push(("Luxuries", h.happy, h.content, h.unhappy));
        // buildings
        let mut make_content = 0;
        if city.has_improvement(imp::TEMPLE) {
            make_content += self.temple_effect(c);
        }
        if city.has_improvement(imp::COLOSSEUM) {
            make_content += 3;
        }
        if city.has_improvement(imp::CATHEDRAL) || self.owner_has_wonder(slot, won::MICHELANGELOS_CHAPEL) {
            make_content += 3;
        }
        h.pacify(make_content);
        h.steps.push(("Buildings", h.happy, h.content, h.unhappy));
        // wonders
        let mut wonder_content = 0;
        if self.owner_has_wonder(slot, won::HANGING_GARDENS) {
            wonder_content += 1;
        }
        if self.owner_has_wonder(slot, won::CURE_FOR_CANCER) {
            wonder_content += 1;
        }
        if self.owner_has_wonder(slot, won::JS_BACHS_CATHEDRAL) {
            wonder_content += 2;
        }
        h.pacify(wonder_content);
        if city.has_wonder(won::SHAKESPEARES_THEATRE) && !self.wonder_obsolete(won::SHAKESPEARES_THEATRE) {
            h.pacify(h.unhappy);
        }
        h.steps.push(("Wonders", h.happy, h.content, h.unhappy));
        // military
        let g = rules::GOVERNMENTS[p.govt as usize];
        if g.martial_law_max > 0 {
            let units_in = self.martial_law_units(c);
            h.pacify(units_in.min(g.martial_law_max as i32));
            h.steps.push(("Martial law", h.happy, h.content, h.unhappy));
        } else {
            let abroad = self.aggressive_units_abroad(c);
            let mut per = g.unhappy_per_aggressive_unit as i32;
            if self.owner_has_wonder(slot, won::WOMENS_SUFFRAGE) {
                per -= 1;
            }
            let mut n = abroad * per.max(0);
            while n > 0 && h.worsen_one() {
                n -= 1;
            }
            h.steps.push(("Military", h.happy, h.content, h.unhappy));
        }
        h.disorder = h.unhappy > h.happy;
        h.celebration = !h.disorder && city.size >= 3 && h.unhappy == 0 && h.happy * 2 >= city.size as i32;
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{GameSetup, UnitId};
    use crate::rules::{unit_ids, Difficulty};

    fn emperor_fixture() -> (Game, CityId) {
        let mut g = Game::new(GameSetup { seed: 42, difficulty: Difficulty::Emperor, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler: UnitId = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        (g, c)
    }

    #[test]
    fn disorder_at_size_six_without_temple() {
        let (mut g, c) = emperor_fixture();
        let slot = g.city(c).owner;
        g.player_mut(slot).lux = 0;
        g.player_mut(slot).tax = 60;
        g.city_mut(c).size = 6;
        g.fix_workers(c);
        // all citizens working, no entertainers
        let n = g.city(c).specialists.len();
        for _ in 0..n {
            g.city_mut(c).specialists.pop();
            g.city_mut(c).size -= 1;
        }
        let h = g.city_happiness(c);
        assert!(h.disorder, "{h:?}");
        assert!(h.steps.iter().any(|s| s.0 == "Luxuries"));
        assert!(h.steps.iter().any(|s| s.0 == "Martial law"));
        // a temple with Mysticism plus a few units restore order
        g.city_mut(c).add_improvement(imp::TEMPLE);
        g.player_mut(slot).learn(rules::advance_ids::MYSTICISM);
        let (x, y) = g.city(c).pos();
        g.add_unit(slot, unit_ids::PHALANX, x, y);
        g.add_unit(slot, unit_ids::PHALANX, x, y);
        g.add_unit(slot, unit_ids::PHALANX, x, y);
        let h2 = g.city_happiness(c);
        assert!(h2.unhappy < h.unhappy);
    }
}
