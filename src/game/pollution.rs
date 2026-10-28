//! Pollution and global warming.
//!
//! Pollution per city per turn (CIV.EXE 0x12de9): `shields / f - 20 + size * techs / 4`, where
//! `f` is 3 with a Recycling Center, else 2 with a Hydro or Nuclear Plant or a Hoover Dam on the
//! continent, else 1 (0xdc68), and `techs` counts Industrialization, Automobile, Mass Production
//! and Plastics, none with Mass Transit (0xcc43). A square is polluted when twice that beats
//! `random(256 - advances * difficulty / 2)`. Only the human player's cities pollute.
//! Global warming (0x3ae7, overlay 7 +0x1ba8) follows a counter that tracks the number of
//! polluted squares.

use super::events::GameEvent;
use super::map::{tile_flags, CityId};
use super::Game;
use crate::rules::{advance_ids as adv, improvement_ids as imp, wonder_ids as won, Terrain};

impl Game {
    /// Industrial pollution divisor: Recycling Center 3, clean power 2, otherwise 1.
    pub fn pollution_divisor(&self, c: CityId) -> i32 {
        let city = self.city(c);
        if city.has_improvement(imp::RECYCLING_CNTR) {
            3
        } else if city.has_improvement(imp::HYDRO_PLANT)
            || city.has_improvement(imp::NUCLEAR_PLANT)
            || (self.owner_has_wonder(city.owner, won::HOOVER_DAM) && self.same_continent_as_wonder(c, won::HOOVER_DAM))
        {
            2
        } else {
            1
        }
    }

    /// Pollution points of the city this turn (0 or less: no pollution).
    pub fn pollution_points(&self, c: CityId) -> i32 {
        let city = self.city(c);
        let p = self.player(city.owner);
        let techs = if city.has_improvement(imp::MASS_TRANSIT) {
            0
        } else {
            [adv::INDUSTRIALIZATION, adv::AUTOMOBILE, adv::MASS_PRODUCTION, adv::PLASTICS].iter().filter(|a| p.knows(**a)).count() as i32
        };
        // the city's shields ds:0x70dc, which civil disorder does not touch
        self.city_yield_raw(c).shields / self.pollution_divisor(c) - 20 + city.size as i32 * techs / 4
    }

    /// Roll for pollution in a city (CIV.EXE 0x12de9): polluted when twice the points beat
    /// `random(256 - research_count * difficulty / 2)`; the square is then one of the 20 of
    /// the city radius around the centre, picked by `random(20)` from the table ds:0x2498 /
    /// ds:0x24c2 — worked or not — and the city gets no pollution this turn when that square
    /// is already polluted, Ocean or a city. Computer players' cities never pollute: CIV.EXE
    /// 0x12ddd skips this for every civ but the human's.
    pub fn pollution_roll(&mut self, c: CityId) {
        if !self.player(self.city(c).owner).human {
            return;
        }
        let points = self.pollution_points(c);
        let owner = self.city(c).owner;
        let range = 256 - self.player(owner).research_count as i32 * self.setup.difficulty as i32 / 2;
        if 2 * points <= self.rng.below(range.max(1) as u32) as i32 {
            return;
        }
        let (dx, dy) = POLLUTION_SQUARES[self.rng.below(20) as usize];
        let (cx, cy) = self.city(c).pos();
        let (x, y) = (super::Map::wrap_x(cx + dx as i32), cy + dy as i32);
        let Some(t) = self.map.get(x, y) else { return };
        if t.has(tile_flags::POLLUTION) || t.is_ocean() || t.city.is_some() {
            return;
        }
        self.map.get_mut(x, y).unwrap().set(tile_flags::POLLUTION, true);
        self.polluted_tiles += 1;
        self.push_event(GameEvent::Pollution { city: c, at: (x, y) });
    }

    /// "Risk of MELTDOWN!": a city in disorder with a Nuclear Plant melts down one turn in
    /// three until its owner knows Fusion Power (CIV.EXE 0x12f46, human cities only like
    /// pollution; the third is rolled before Fusion Power is looked at).
    pub fn meltdown_roll(&mut self, c: CityId) {
        let city = self.city(c);
        if !self.player(city.owner).human || !city.disorder || !city.has_improvement(imp::NUCLEAR_PLANT) {
            return;
        }
        if self.rng.below(3) == 0 && !self.player(self.city(c).owner).knows(adv::FUSION_POWER) {
            self.meltdown(c);
        }
    }

    /// The Nuclear Plant is lost, then `nuclear_fallout` at the city.
    pub fn meltdown(&mut self, c: CityId) {
        self.city_mut(c).remove_improvement(imp::NUCLEAR_PLANT);
        let (cx, cy) = self.city(c).pos();
        self.nuclear_fallout(cx, cy);
        self.push_event(GameEvent::Meltdown { city: c });
    }

    /// `nuclear_fallout` (CIV.EXE 0x1a5d9): every city on the 3x3 squares around (cx, cy)
    /// loses half its size (rounded down) and the other land squares are polluted two times in
    /// three. Returns the cities hit.
    pub fn nuclear_fallout(&mut self, cx: i32, cy: i32) -> Vec<CityId> {
        let mut hit_cities = Vec::new();
        for (x, y) in std::iter::once((super::Map::wrap_x(cx), cy)).chain(super::Map::neighbours8(cx, cy)) {
            let Some(t) = self.map.get(x, y) else { continue };
            if let Some(hit) = t.city {
                let loss = self.city(hit).size as i32 / 2;
                if loss > 0 {
                    self.shrink_city(hit, loss);
                }
                hit_cities.push(hit);
            } else if self.rng.below(3) != 0 && t.is_land() && !t.has(tile_flags::POLLUTION) {
                self.map.get_mut(x, y).unwrap().set(tile_flags::POLLUTION, true);
                self.polluted_tiles += 1;
            }
        }
        hit_cities
    }

    /// The end-of-turn warming step (CIV.EXE 0x3ae7): `warming_counter` moves one step
    /// towards `2 * polluted - 4 * warmings` (kept in 0..99); "Global Warming feared!" when it
    /// is exactly 12 with more than 6 polluted squares; above 16 the world warms and the
    /// counter starts again from 0.
    pub fn global_warming_check(&mut self) {
        let target = 2 * self.polluted_tiles as i32 - 4 * self.global_warming_count as i32;
        let counter = self.warming_counter as i32;
        self.warming_counter = (counter + (target - counter).signum()).clamp(0, 99) as u16;
        if self.warming_counter == 12 && self.polluted_tiles > 6 {
            self.push_event(GameEvent::GlobalWarmingFeared);
        }
        if self.warming_counter > 16 {
            self.global_warming();
            self.warming_counter = 0;
        }
    }

    /// One global warming (overlay 7 +0x1ba8 with `n` = the warmings before this one): every
    /// Desert, Plains, Grassland and Forest square with at least `7 - n` Ocean neighbours
    /// turns to Swamp (Forest to Jungle) and loses irrigation and mine; of the others, those
    /// with `(11x + 13y) & 7 == n` dry up, Desert and Plains to Desert, Grassland and Forest
    /// to Plains, keeping their improvements. City squares change too; a changed square's hut
    /// is spent (es:0x740 |= 1).
    pub fn global_warming(&mut self) {
        self.push_event(GameEvent::GlobalWarming);
        let n = self.global_warming_count as i32;
        let seed = self.map.seed;
        for x in 0..crate::rules::MAP_W as i32 {
            for y in 0..crate::rules::MAP_H as i32 {
                let t = self.map.tile(x, y).terrain;
                if !matches!(t, Terrain::Desert | Terrain::Plains | Terrain::Grassland | Terrain::Forest) {
                    continue;
                }
                let oceans = super::Map::neighbours8(x, y).filter(|&(nx, ny)| self.map.is_ocean(nx, ny)).count() as i32;
                let tm = self.map.get_mut(x, y).unwrap();
                let nt = if oceans >= 7 - n {
                    tm.set(tile_flags::IRRIGATION | tile_flags::MINE, false);
                    if t == Terrain::Forest { Terrain::Jungle } else { Terrain::Swamp }
                } else if (11 * x + 13 * y) & 7 == n {
                    if matches!(t, Terrain::Desert | Terrain::Plains) { Terrain::Desert } else { Terrain::Plains }
                } else {
                    continue;
                };
                tm.terrain = nt;
                tm.set(tile_flags::HUT, false);
                tm.special = super::orders::positional_special(seed, x, y, nt);
            }
        }
        self.global_warming_count = self.global_warming_count.saturating_add(1);
    }
}

/// The 20 squares around a city in the order of CIV.EXE's table ds:0x2498 (dx) / ds:0x24c2
/// (dy), which `pollution_roll` picks from.
const POLLUTION_SQUARES: [(i8, i8); 20] = [
    (0, -1), (1, 0), (0, 1), (-1, 0), (1, -1), (1, 1), (-1, 1), (-1, -1),
    (0, -2), (2, 0), (0, 2), (-2, 0), (-1, -2), (1, -2), (2, -1), (2, 1), (1, 2), (-1, 2), (-2, 1), (-2, -1),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;
    use crate::rules::unit_ids;

    #[test]
    fn pollution_grows_with_industry_and_population() {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        let shields = g.city_yield(c).shields;
        assert_eq!(g.pollution_points(c), shields - 20);
        for a in [adv::INDUSTRIALIZATION, adv::AUTOMOBILE, adv::MASS_PRODUCTION, adv::PLASTICS] {
            g.player_mut(slot).learn(a);
        }
        let size = g.city(c).size as i32;
        assert_eq!(g.pollution_points(c), shields - 20 + size);
        g.city_mut(c).add_improvement(imp::MASS_TRANSIT);
        assert_eq!(g.pollution_points(c), shields - 20);
        // a Recycling Center replaces the Hydro Plant's divisor
        g.city_mut(c).add_improvement(imp::HYDRO_PLANT);
        assert_eq!(g.pollution_divisor(c), 2);
        g.city_mut(c).add_improvement(imp::RECYCLING_CNTR);
        assert_eq!(g.pollution_divisor(c), 3);
        let before = g.global_warming_count;
        g.global_warming();
        assert_eq!(g.global_warming_count, before + 1);
    }

    #[test]
    fn meltdown_halves_the_city_and_loses_the_plant() {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        g.city_mut(c).size = 8;
        g.fix_workers(c);
        g.city_mut(c).add_improvement(imp::NUCLEAR_PLANT);
        g.meltdown_roll(c);
        assert!(g.city(c).has_improvement(imp::NUCLEAR_PLANT), "only in disorder");
        g.meltdown(c);
        assert_eq!(g.city(c).size, 4);
        assert!(!g.city(c).has_improvement(imp::NUCLEAR_PLANT));
        assert!(g.take_events().iter().any(|e| *e == GameEvent::Meltdown { city: c }));
    }

    #[test]
    fn computer_cities_never_pollute() {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        g.city_mut(c).size = 12;
        g.fix_workers(c);
        for a in [adv::INDUSTRIALIZATION, adv::AUTOMOBILE, adv::MASS_PRODUCTION, adv::PLASTICS] {
            g.player_mut(slot).learn(a);
        }
        // force a huge pollution score through the city size, then hand the city to the AI
        g.city_mut(c).size = 120;
        assert!(g.pollution_points(c) > 0);
        g.player_mut(slot).human = false;
        let before = g.polluted_tiles;
        for _ in 0..50 {
            g.pollution_roll(c);
        }
        assert_eq!(g.polluted_tiles, before);
        g.player_mut(slot).human = true;
        for _ in 0..50 {
            g.pollution_roll(c);
        }
        assert!(g.polluted_tiles > before);
    }

    #[test]
    fn warming_counter_steps_towards_twice_the_pollution() {
        // 0x3ae7: one step a turn towards 2 * polluted - 4 * warmings, warning at 12, warming
        // above 16
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        g.polluted_tiles = 9;
        let mut feared = 0;
        for turn in 1..=16 {
            g.global_warming_check();
            assert_eq!(g.warming_counter, turn);
            feared += g.take_events().iter().filter(|e| **e == GameEvent::GlobalWarmingFeared).count();
        }
        assert_eq!((feared, g.global_warming_count), (1, 0));
        g.global_warming_check();
        assert_eq!((g.warming_counter, g.global_warming_count), (0, 1));
        // after one warming 9 squares only pull the counter towards 2*9 - 4 = 14
        for _ in 0..40 {
            g.global_warming_check();
        }
        assert_eq!((g.warming_counter, g.global_warming_count), (14, 1));
        g.polluted_tiles = 0;
        g.global_warming_check();
        assert_eq!(g.warming_counter, 13);
    }

    #[test]
    fn global_warming_floods_coasts_and_dries_the_pattern() {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        // the fourth warming: 4 Ocean neighbours flood a square, (11x + 13y) & 7 == 3 dries it
        g.global_warming_count = 3;
        let before = g.map.clone();
        g.global_warming();
        let mut swamped = 0;
        for x in 0..80 {
            for y in 0..50 {
                let (a, b) = (before.tile(x, y), g.map.tile(x, y));
                let temperate = matches!(a.terrain, Terrain::Desert | Terrain::Plains | Terrain::Grassland | Terrain::Forest);
                let oceans = crate::game::Map::neighbours8(x, y).filter(|&(nx, ny)| before.is_ocean(nx, ny)).count();
                let want = if !temperate {
                    a.terrain
                } else if oceans >= 4 {
                    swamped += 1;
                    if a.terrain == Terrain::Forest { Terrain::Jungle } else { Terrain::Swamp }
                } else if (11 * x + 13 * y) & 7 == 3 {
                    if matches!(a.terrain, Terrain::Desert | Terrain::Plains) { Terrain::Desert } else { Terrain::Plains }
                } else {
                    a.terrain
                };
                assert_eq!(b.terrain, want, "({x},{y})");
            }
        }
        assert!(swamped > 0);
    }

    #[test]
    fn pollution_lands_on_any_square_of_the_radius() {
        // 0x12e4b: random(20) of the table ds:0x2498, worked or not; a city square, Ocean or
        // a polluted square means no pollution this turn
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        for a in [adv::INDUSTRIALIZATION, adv::AUTOMOBILE, adv::MASS_PRODUCTION, adv::PLASTICS] {
            g.player_mut(slot).learn(a);
        }
        g.city_mut(c).size = 120;
        let (cx, cy) = g.city(c).pos();
        for _ in 0..400 {
            g.pollution_roll(c);
        }
        let worked: Vec<(i32, i32)> = (1..21u8).filter(|&i| g.city(c).works(i)).filter_map(|i| g.radius_tile(c, i)).collect();
        let polluted: Vec<(i32, i32)> = (-2..=2).flat_map(|dx| (-2..=2).map(move |dy| (cx + dx, cy + dy))).filter(|&(x, y)| g.map.tile(x, y).has(tile_flags::POLLUTION)).collect();
        assert!(polluted.iter().any(|p| !worked.contains(p)), "{polluted:?}");
        assert!(polluted.iter().all(|&(x, y)| g.map.tile(x, y).is_land() && (x, y) != (cx, cy)));
    }
}
