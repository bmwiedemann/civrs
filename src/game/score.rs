//! Score, ratings and demographics.
//!
//! Score = 2 * happy + content (specialists count as content) + 20 per wonder + 5 per future
//! tech + 3 per peace turn (after 1 AD) - 10 per polluted tile within the player's city
//! radii + spaceship bonus (success % * habitation modules / 2 once the human's ship landed,
//! CIV.EXE o 0x1c3cc).
//! The civilization rating is `total * (difficulty + 1) / 50` percent (CIV.EXE o 0x2bc9, the
//! Hall of Fame line), i.e. 1000 points at Prince = 60%.

use super::map::Slot;
use super::Game;
use crate::rules;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScoreBreakdown {
    pub happy: i32,
    pub content: i32,
    pub wonders: i32,
    pub future_tech: i32,
    pub peace: i32,
    pub pollution: i32,
    pub spaceship: i32,
    pub total: i32,
    /// Civilization score percentage shown on F9.
    pub percent: i32,
}

/// One statistic of the F11 demographics (`demographics`, ovl 12 +0xd6d, o 0x10ee2): the value
/// of every civilization (index = slot), the viewer's own text, the unit `print_rank` appends to
/// the leader's value, and whether the lowest value ranks first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DemographicRow {
    pub label: &'static str,
    pub values: [i32; 8],
    pub text: String,
    pub unit: &'static str,
    pub lower_better: bool,
}

/// `print_rank` (o 0x11add) over the active civilizations (`alive`): the viewer's rank (1 +
/// the civs with a higher value), the leading value and its civ (the first highest, else the
/// viewer). For lower-is-better rows each value is negated as the loop reaches its civ, so civs
/// before the viewer compare against the viewer's value unnegated, as in CIV.EXE.
pub fn demographic_rank(values: &[i32; 8], lower_better: bool, viewer: Slot, alive: &[Slot]) -> (usize, i32, Slot) {
    let mut v = *values;
    let (mut rank, mut best, mut best_civ) = (1, -9999, viewer);
    for &c in alive {
        let c = c as usize;
        if lower_better {
            v[c] = -v[c];
        }
        if v[c] > v[viewer as usize] {
            rank += 1;
        }
        if best < v[c] {
            best = v[c];
            best_civ = c as Slot;
        }
    }
    (rank, best, best_civ)
}

/// `format_population` (o 0x1d75d) of a 1 + 2 + .. + size sum: "1,240,000" for 124, "50,000"
/// for 5.
pub fn population_text(n: i32) -> String {
    if n < 100 {
        format!("{n}0,000")
    } else {
        format!("{},{:02}0,000", n / 100, n % 100)
    }
}

/// Index into `rules::RATINGS` (best first) for a rating percentage (CIV.EXE o 0x2dd2): the
/// level is one less than the largest `i` in 1..=20 with `i * i / 3 < percent`, and the ruler
/// shown is entry `19 - level` (the last one when no `i` qualifies).
pub fn rating_index(percent: i32) -> usize {
    let level = (1..=20).filter(|i| i * i / 3 < percent).max().map(|i| i - 1).unwrap_or(0).min(19);
    (19 - level) as usize
}

/// Index into `rules::ADJECTIVES` (best first) for a score percentage.
pub fn adjective_index(percent: i32) -> usize {
    ((100 - percent.clamp(0, 100)) * 6 / 100) as usize
}

impl Game {
    pub fn score(&self, slot: Slot) -> ScoreBreakdown {
        let mut s = ScoreBreakdown::default();
        for c in self.all_cities().filter(|c| c.owner == slot) {
            let h = self.city_happiness(c.id);
            s.happy += h.happy;
            s.content += h.content + c.specialists.len() as i32;
        }
        s.wonders = (1..=21u8).filter(|w| self.wonders_built[*w as usize].and_then(|c| self.city_opt(c)).map(|c| c.owner == slot).unwrap_or(false)).count() as i32;
        let p = self.player(slot);
        s.future_tech = p.future_techs as i32;
        s.peace = p.peace_turns as i32;
        // civ_score (ovl 20 +0xca9): -10 per polluted square of the whole world (ds:0x6e94),
        // peace 3 per turn up to 100 and only in AD years
        s.pollution = self.polluted_tiles as i32;
        s.spaceship = self.spaceship_score(slot);
        s.total = s.happy * 2 + s.content + s.wonders * 20 + s.future_tech * 5 + self.peace_points(slot) - s.pollution * 10 + s.spaceship;
        if let Some(bonus) = self.conquest_bonus().filter(|b| *b > s.total.max(0)) {
            s.total = bonus;
        }
        s.total = s.total.max(0);
        s.percent = s.total * (self.setup.difficulty as i32 + 1) / 50;
        s
    }

    /// Peace points of the score: 3 per turn of peace, at most 100, only in AD years.
    pub fn peace_points(&self, slot: Slot) -> i32 {
        if self.year() > 0 { (3 * self.player(slot).peace_turns as i32).clamp(0, 100) } else { 0 }
    }

    /// The conquest victory's "Bonus Score" (o 0x1c5e3): 100 per rival (ds:0xd2f4 = civs - 1)
    /// + 2 per turn left before turn 550 + 400; it replaces a lower total.
    pub fn conquest_bonus(&self) -> Option<i32> {
        (self.ended == Some(crate::game::EndReason::Conquest)).then(|| 100 * (self.setup.num_civs as i32 - 1) + 2 * (550 - self.turn as i32) + 400)
    }

    /// Polluted tiles inside the radius of any city of the player.
    pub fn polluted_tiles_near(&self, slot: Slot) -> i32 {
        let mut seen = std::collections::HashSet::new();
        for c in self.all_cities().filter(|c| c.owner == slot) {
            for idx in 0..21u8 {
                if let Some((x, y)) = self.radius_tile(c.id, idx) {
                    if self.map.tile(x, y).has(super::tile_flags::POLLUTION) {
                        seen.insert((x, y));
                    }
                }
            }
        }
        seen.len() as i32
    }

    /// Military/economic power estimate used by the AI and demographics.
    pub fn power(&self, slot: Slot) -> i32 {
        let units: i32 = self.all_units().filter(|u| u.owner == slot).map(|u| u.info().att as i32 + u.info().def as i32).sum();
        let cities: i32 = self.all_cities().filter(|c| c.owner == slot).map(|c| c.size as i32 * 2).sum();
        units + cities
    }

    /// The thirteen rows of F11 for `viewer` (o 0x10ee2). Every city is recalculated and summed
    /// per owner into: size (from 1), size + happy - unhappy, 2 * lux + tax, food - 2 * size,
    /// food + shields + trade, shields, the pollution points above 0, the size of cities with a
    /// Granary and again with an Aqueduct, and with a Library and again with a University; land
    /// is the non-ocean squares of rows 2..47 last held by the civ's units.
    pub fn demographics(&self, viewer: Slot) -> Vec<DemographicRow> {
        use rules::{advance_ids as adv, improvement_ids as imp};
        let mut size = [1i32; 8];
        let (mut approval, mut trade, mut surplus, mut output, mut shields) = ([0i32; 8], [0i32; 8], [0i32; 8], [0i32; 8], [0i32; 8]);
        let (mut pollution, mut health, mut lit, mut land) = ([0i32; 8], [0i32; 8], [0i32; 8], [0i32; 8]);
        for x in 0..rules::MAP_W as i32 {
            for y in 2..48 {
                if self.map.tile(x, y).terrain != rules::Terrain::Ocean {
                    land[self.last_occupant(x, y) as usize & 7] += 1;
                }
            }
        }
        for c in self.all_cities() {
            let o = c.owner as usize & 7;
            let n = c.size as i32;
            let y = self.city_yield(c.id);
            let h = self.city_happiness(c.id);
            size[o] += n;
            approval[o] += n + h.happy - h.unhappy;
            trade[o] += 2 * y.lux + y.tax;
            surplus[o] += y.food - 2 * n;
            output[o] += y.food + y.shields + y.trade;
            shields[o] += y.shields;
            pollution[o] += self.pollution_points(c.id).max(0);
            let with = |i| if c.has_improvement(i) { n } else { 0 };
            health[o] += with(imp::GRANARY) + with(imp::AQUEDUCT);
            lit[o] += with(imp::LIBRARY) + with(imp::UNIVERSITY);
        }
        let per_civ = |f: &dyn Fn(usize) -> i32| -> [i32; 8] { std::array::from_fn(f) };
        let knows = |c: usize, a: u8| self.player(c as Slot).knows(a);
        let me = viewer as usize;
        let mut rows = Vec::new();
        let mut row = |label, values: [i32; 8], text: String, unit, lower_better| rows.push(DemographicRow { label, values, text, unit, lower_better });

        let v = per_civ(&|c| 50 * approval[c] / size[c]);
        row("Approval Rating:", v, format!("{}%", v[me]), "%", false);
        let v = per_civ(&|c| self.all_cities().filter(|k| k.owner as usize == c).map(|k| (1..=k.size as i32).sum::<i32>()).sum());
        row("Population:", v, population_text(v[me]), "0000", false);
        row("GNP:", trade, format!("{} million $", trade[me]), "M$", false);
        row("Mfg. Goods:", shields, format!("{} Mtons", shields[me]), " Mtons", false);
        row("Land Area:", land, format!("{},000 sq.miles", land[me]), ",000", false);
        let v = per_civ(&|c| {
            let mut l = (2 * size[c] + lit[c]) * 2 / size[c];
            for a in [adv::ALPHABET, adv::WRITING, adv::LITERACY, adv::UNIVERSITY] {
                if knows(c, a) {
                    l *= 2;
                }
            }
            l.clamp(0, 100)
        });
        row("Literacy:", v, format!("{}%", v[me]), "%", false);
        let disease = per_civ(&|c| 50 * size[c] / (size[c] + health[c]) / if knows(c, adv::MEDICINE) { 2 } else { 1 });
        row("Disease:", disease, format!("{}%", disease[me]), "%", true);
        row("Pollution:", pollution, format!("{}0 tons/year", pollution[me]), "0 tons", true);
        let v = per_civ(&|c| (1500 / (disease[c] + 20) - 10 * pollution[c] / size[c]).clamp(20, 99));
        row("Life Expectancy:", v, format!("{} years", v[me]), " yrs", false);
        let family = per_civ(&|c| 40 * surplus[c] / size[c] + 20);
        row("Family Size:", family.map(|f| f / 10), format!("{}.{} children", family[me] / 10, family[me] % 10), "", false);
        let v = per_civ(&|c| 10 * self.units_of(c as Slot).len() as i32 / size[c]);
        row("Military Service:", v, format!("{} years", v[me]), " yrs", false);
        let v = per_civ(&|c| {
            let advances = self.player(c as Slot).research_count as i32;
            if 30000 / (advances + 1) > trade[c] { trade[c] * advances / size[c] } else { trade[c] / size[c] * advances }
        });
        row("Annual Income:", v, format!("{}$ per capita", v[me]), "$", false);
        let v = per_civ(&|c| 10 * output[c] / size[c]);
        row("Productivity:", v, v[me].to_string(), "", false);
        rows
    }

    /// Rating name ("your rating exceeds even that of ...") for a player's score.
    pub fn rating_name(&self, slot: Slot) -> &'static str {
        rules::RATINGS[rating_index(self.score(slot).percent)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;
    use crate::rules::unit_ids;

    #[test]
    fn score_non_negative_and_ranked() {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        g.found_city(settler).unwrap();
        let s = g.score(slot);
        assert!(s.total >= 0);
        assert!(s.percent >= 0);
        assert!(rating_index(s.percent) < rules::RATINGS.len());
        // 17 * 17 / 3 = 96 < 100: level 16, Thomas Jefferson; 20 * 20 / 3 = 133 tops the list
        assert_eq!(rules::RATINGS[rating_index(100)], "Thomas Jefferson");
        assert_eq!(rating_index(134), 0);
        assert_eq!(rating_index(0), 19);
        let d = g.demographics(slot);
        assert_eq!(d.len(), 13);
        let (rank, _, best) = demographic_rank(&d[1].values, false, slot, &g.alive_slots());
        assert_eq!((rank, best), (1, slot), "only civ with a city is the largest");
    }
}
