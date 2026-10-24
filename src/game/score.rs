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

/// One row of the demographics report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DemographicEntry {
    /// Category name from `rules::DEMOGRAPHIC_CATEGORIES`.
    pub category: &'static str,
    /// The player's value in that category.
    pub value: i64,
    /// 1 = best among living civilisations.
    pub rank: usize,
    /// Number of civilisations ranked.
    pub of: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Demographics {
    pub entries: Vec<DemographicEntry>,
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

    /// Percentage of the player's citizens that are happy.
    pub fn happiness_ratio(&self, slot: Slot) -> i64 {
        let mut happy = 0i64;
        let mut total = 0i64;
        for c in self.all_cities().filter(|c| c.owner == slot) {
            happy += self.city_happiness(c.id).happy as i64;
            total += c.size as i64;
        }
        if total == 0 { 0 } else { happy * 100 / total }
    }

    /// Value of a demographics category for a player.
    fn demographic_value(&self, slot: Slot, category: usize) -> i64 {
        match category {
            0 => self.player(slot).gold as i64,
            1 => self.power(slot) as i64,
            2 => self.player(slot).advance_count() as i64 + self.player(slot).future_techs as i64,
            3 => self.happiness_ratio(slot),
            _ => self.total_population(slot),
        }
    }

    /// Rank (1 = best) of the player among living civilisations in each category.
    pub fn demographics(&self, slot: Slot) -> Demographics {
        let civs = self.alive_slots();
        let mut d = Demographics::default();
        for (i, name) in rules::DEMOGRAPHIC_CATEGORIES.iter().enumerate() {
            let mine = self.demographic_value(slot, i);
            let better = civs.iter().filter(|s| **s != slot && self.demographic_value(**s, i) > mine).count();
            d.entries.push(DemographicEntry { category: name, value: mine, rank: better + 1, of: civs.len().max(1) });
        }
        d
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
        assert_eq!(d.entries.len(), 5);
        for e in &d.entries {
            assert!(e.rank >= 1 && e.rank <= e.of);
        }
        assert_eq!(d.entries[4].rank, 1, "only civ with a city is the largest");
    }
}
