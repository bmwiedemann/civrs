//! The histories the end of the game shows: the replay log that overlay 9's `replay()`
//! (o 0xcb75) plays back, written by `replay_log` (0xa36a, far ds:0x63ea +0x26fc, length
//! ds:0x40dc), and the power history of `power_ranking` (0x3121, far es:0x63e0) that the power
//! graph (o 0x106e8) draws.

use super::{CityId, Game, Slot};
use crate::rules;
use serde::{Deserialize, Serialize};

/// Record kinds, the high nibble of a record's first byte; the arguments follow the turn.
pub mod kind {
    /// owner (0xff: the city was destroyed), city name, x, y: `found_city` (o 0x1b4e4),
    /// `destroy_city` (0xa793).
    pub const CITY: u8 = 1;
    /// a << 4 | b: the treaty between a and b broken (`clear_diplo_flag` 0x15428).
    pub const WAR: u8 = 2;
    /// a << 4 | b: a treaty signed (`set_diplo_flag` 0x153b4).
    pub const PEACE: u8 = 3;
    /// civ, advance: the first civilization to learn it after turn 0 (0xc767).
    pub const DISCOVERY: u8 = 5;
    /// civ, unit type: the first unit of the type anyone built (0xdf46).
    pub const FIRST_UNIT: u8 = 6;
    /// civ, government (`set_government` 0x15060).
    pub const GOVERNMENT: u8 = 8;
    /// captor, city name, x, y: a city taken and kept (`capture_city` 0x13f97).
    pub const CAPTURE: u8 = 9;
    /// civ, wonder (1-based): a wonder completed (0xe4dd).
    pub const WONDER: u8 = 10;
    /// the human's cities, population high, low byte (every 50 turns, 0x2848).
    pub const CENSUS: u8 = 11;
    /// four bytes of civs by rank, two to a byte, strongest first (every 50 turns, 0x28d6).
    pub const RANKING: u8 = 12;
    /// civ, destroyer: a civilization destroyed (`civ_destroyed` o 0x50c8).
    pub const DESTROYED: u8 = 13;
}

/// The log takes no record that would reach 4096 bytes (0xa376).
pub const LOG_LIMIT: usize = 0x1000;
/// Rows of the power history: one per four turns, up to turn 600.
pub const POWER_ROWS: usize = 150;

/// The replay log and the power history. Stored after the game body in version-18 saves.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct History {
    /// The records: kind << 4 | turn >> 8, the turn's low byte, then the arguments.
    pub log: Vec<u8>,
    /// The city names the records refer to. CIV.EXE stores the index into its 256-name table
    /// (seg 0x2583); civrs keeps the names it has logged.
    pub names: Vec<String>,
    /// clamp(strength / 8, 0, 255) at [8 * row + civ], row = (turn / 4) % 150; empty until the
    /// first ranking.
    pub power: Vec<u8>,
    /// Unit types someone has built (bit 3 of far ds:0x63f4 +0x4b0 + 2 * type).
    pub units_built: u32,
}

/// One record of the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub turn: u32,
    pub kind: u8,
    pub args: Vec<u8>,
}

/// The number of argument bytes `replay()` reads for a record kind.
pub fn arg_count(k: u8) -> usize {
    match k {
        kind::CITY | kind::CAPTURE | kind::RANKING => 4,
        kind::WAR | kind::PEACE => 1,
        kind::CENSUS => 3,
        kind::DISCOVERY | kind::FIRST_UNIT | kind::GOVERNMENT | kind::WONDER | kind::DESTROYED => 2,
        _ => 0,
    }
}

impl History {
    /// The records in the order they were logged.
    pub fn records(&self) -> Vec<Record> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 1 < self.log.len() {
            let k = self.log[i] >> 4;
            let turn = ((self.log[i] as u32 & 15) << 8) | self.log[i + 1] as u32;
            let n = arg_count(k);
            let args = self.log.get(i + 2..i + 2 + n).map(|a| a.to_vec()).unwrap_or_default();
            out.push(Record { turn, kind: k, args });
            i += 2 + n;
        }
        out
    }

    /// A logged city name.
    pub fn name(&self, i: u8) -> &str {
        self.names.get(i as usize).map(|s| s.as_str()).unwrap_or("")
    }

    /// The power history byte of `civ` in `row`.
    pub fn power_at(&self, row: usize, civ: Slot) -> u8 {
        self.power.get(8 * row + civ as usize).copied().unwrap_or(0)
    }
}

impl Game {
    /// `replay_log(kind, nargs, ...)` (0xa36a): appends a record stamped with this turn, unless
    /// the log would reach `LOG_LIMIT`.
    pub fn replay_log(&mut self, k: u8, args: &[u8]) {
        let log = &mut self.history.log;
        if args.len() + log.len() + 2 >= LOG_LIMIT {
            return;
        }
        log.push((k << 4).wrapping_add((self.turn >> 8) as u8));
        log.push(self.turn as u8);
        log.extend_from_slice(args);
    }

    /// The name index a record stores for `name`.
    fn replay_name(&mut self, name: &str) -> u8 {
        let names = &mut self.history.names;
        if let Some(i) = names.iter().position(|n| n == name) {
            return i as u8;
        }
        if names.len() < 256 {
            names.push(name.to_string());
        }
        (names.len() - 1) as u8
    }

    /// A city record (`kind::CITY` or `kind::CAPTURE`) for city `c` with `civ` in front.
    pub fn replay_city(&mut self, k: u8, civ: u8, c: CityId) {
        let (name, x, y) = {
            let city = self.city(c);
            (city.name.clone(), city.x, city.y)
        };
        let n = self.replay_name(&name);
        self.replay_log(k, &[civ, n, x as u8, y as u8]);
    }

    /// Names a city. The replay shows a city under its current name, as CIV.EXE writes a new
    /// name into its name table.
    pub fn set_city_name(&mut self, c: CityId, name: String) {
        let old = std::mem::replace(&mut self.city_mut(c).name, name.clone());
        if let Some(n) = self.history.names.iter_mut().find(|n| **n == old) {
            *n = name;
        }
    }

    /// The first unit of a type anyone completes is logged (0xdf0c..0xdf6a).
    pub fn replay_unit_built(&mut self, civ: Slot, k: u8) {
        let bit = 1u32 << (k as u32 & 31);
        if self.history.units_built & bit == 0 {
            self.replay_log(kind::FIRST_UNIT, &[civ, k]);
            self.history.units_built |= bit;
        }
    }

    /// A civilization's strength in `power_ranking` (0x3121): |gold| / 32 (truncated) + 8 *
    /// citizens (ds:0xd2f8) + research count + the cost of every unit it has; 0 for one that is
    /// out of the game.
    pub fn power_strength(&self, s: Slot) -> i32 {
        let p = self.player(s);
        if !p.in_game || !p.alive {
            return 0;
        }
        let citizens: i32 = self.all_cities().filter(|c| c.owner == s).map(|c| c.size as i32).sum();
        let units: i32 = self.all_units().filter(|u| u.owner == s).map(|u| u.info().cost as i32).sum();
        p.gold / 32 + 8 * citizens + p.research_count as i32 + units
    }

    /// The power history entry of `power_ranking` (0x31be..0x3214), at the end of every turn
    /// before turn 600: clamp(strength / 8, 0, 255) for civs 1..7 in row (turn / 4) % 150, so
    /// the last turn of each four stays.
    pub fn record_power(&mut self) {
        if self.turn >= 600 {
            return;
        }
        if self.history.power.len() < 8 * POWER_ROWS {
            self.history.power.resize(8 * POWER_ROWS, 0);
        }
        let row = (self.turn as usize / 4) % POWER_ROWS;
        for s in 1..8u8 {
            let v = (self.power_strength(s) / 8).clamp(0, 255) as u8;
            self.history.power[8 * row + s as usize] = v;
        }
    }

    /// The block of the new turn that runs every 50 turns (0x2769..0x28db): the research
    /// catch-up bonus is set again, then the replay gets the human's cities and population and
    /// the civilizations by power rank. (The newspaper and the BC status report of the block are
    /// shown elsewhere.)
    pub fn fifty_turn_block(&mut self) {
        if !self.turn.is_multiple_of(50) {
            return;
        }
        self.update_research_catch_up();
        let h = self.human_slot;
        let cities = self.cities_of(h).len() as u8;
        let pop: i32 = self.all_cities().filter(|c| c.owner == h).map(|c| c.size as i32 * (c.size as i32 + 1) / 2).sum();
        self.replay_log(kind::CENSUS, &[cities, (pop / 256) as u8, pop as u8]);
        // rank r (7 = strongest) puts the civ in word r / 2, the odd ranks in the high nibble;
        // the words go into the record from the third down
        let mut w = [0u8; 4];
        for s in self.alive_slots() {
            let r = self.power_rank(s) as usize;
            if r % 2 == 1 {
                w[r / 2] |= s << 4;
            } else {
                w[r / 2] |= s;
            }
        }
        self.replay_log(kind::RANKING, &[w[3], w[2], w[1], w[0]]);
    }
}

/// Plural and adjective of a computer tribe in the replay (the leader table, 0xd8d2).
pub fn tribe_names(civ: u8) -> (&'static str, &'static str) {
    let c = &rules::CIVS[civ as usize % rules::CIVS.len()];
    (c.plural, c.adjective)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;

    fn game() -> Game {
        Game::new(GameSetup { seed: 7, ..Default::default() }, None)
    }

    #[test]
    fn records_carry_the_turn_and_stop_at_the_limit() {
        let mut g = game();
        g.set_turn(300);
        g.replay_log(kind::WONDER, &[2, 5]);
        assert_eq!(g.history.log, vec![0xa1, 300u32 as u8, 2, 5]);
        assert_eq!(g.history.records(), vec![Record { turn: 300, kind: kind::WONDER, args: vec![2, 5] }]);
        while g.history.log.len() + 4 < LOG_LIMIT {
            g.replay_log(kind::WONDER, &[2, 5]);
        }
        let n = g.history.log.len();
        g.replay_log(kind::WONDER, &[2, 5]);
        assert_eq!(g.history.log.len(), n, "a record that would reach 4096 bytes is dropped");
        assert!(n < LOG_LIMIT);
    }

    #[test]
    fn cities_are_logged_when_founded_taken_and_destroyed() {
        let mut g = game();
        g.history = History::default();
        let c = g.add_city(2, "Alpha".into(), 20, 20);
        g.set_city_name(c, "Beta".into());
        g.remove_city(c);
        let r = g.history.records();
        assert_eq!(r[0].kind, kind::CITY);
        assert_eq!(r[0].args, vec![2, 0, 20, 20]);
        assert_eq!(r[1].args, vec![0xff, 0, 20, 20]);
        assert_eq!(g.history.name(0), "Beta", "the replay shows the current name");
    }

    #[test]
    fn power_history_keeps_the_last_turn_of_four() {
        let mut g = game();
        for t in 8..12 {
            g.set_turn(t);
            g.player_mut(2).gold = 800 * t as i32;
            g.record_power();
        }
        let s = g.power_strength(2);
        assert_eq!(g.history.power_at(2, 2), (s / 8).clamp(0, 255) as u8);
        g.set_turn(600);
        g.player_mut(2).gold = 0;
        g.record_power();
        assert_eq!(g.history.power_at(0, 2), 0, "nothing from turn 600 on");
    }

    #[test]
    fn the_fifty_turn_block_logs_census_and_ranking() {
        let mut g = game();
        g.history = History::default();
        g.set_turn(49);
        g.fifty_turn_block();
        assert!(g.history.log.is_empty());
        g.set_turn(50);
        g.fifty_turn_block();
        let r = g.history.records();
        assert_eq!(r.iter().map(|r| r.kind).collect::<Vec<_>>(), vec![kind::CENSUS, kind::RANKING]);
        // every civ in the game is in the ranking once
        let mut civs: Vec<u8> = r[1].args.iter().flat_map(|b| [b >> 4, b & 15]).filter(|c| *c != 0).collect();
        civs.sort();
        assert_eq!(civs, g.alive_slots());
        let strongest = r[1].args[0] >> 4;
        assert_eq!(g.power_rank(strongest), 7);
    }

    #[test]
    fn only_the_first_unit_of_a_type_is_logged() {
        let mut g = game();
        g.history = History::default();
        g.replay_unit_built(3, 4);
        g.replay_unit_built(2, 4);
        assert_eq!(g.history.records().len(), 1);
        assert_eq!(g.history.records()[0].args, vec![3, 4]);
    }
}
