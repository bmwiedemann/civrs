//! Pollution and global warming.
//!
//! Pollution per city per turn (CIV.EXE 0x12de9): `shields / f - 20 + size * techs / 4`, where
//! `f` is 3 with a Recycling Center, else 2 with a Hydro or Nuclear Plant or a Hoover Dam on the
//! continent, else 1 (0xdc68), and `techs` counts Industrialization, Automobile, Mass Production
//! and Plastics, none with Mass Transit (0xcc43). A square is polluted when twice that beats
//! `random(256 - advances * difficulty / 2)`.

use super::events::GameEvent;
use super::map::{tile_flags, CityId};
use super::Game;
use crate::rules::{advance_ids as adv, improvement_ids as imp, wonder_ids as won, Terrain};

/// Tiles changed by one episode of global warming.
pub const WARMING_TILES: usize = 10;

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
        self.city_yield(c).shields / self.pollution_divisor(c) - 20 + city.size as i32 * techs / 4
    }

    /// Roll for pollution in a city; pollutes a random worked land tile on success.
    pub fn pollution_roll(&mut self, c: CityId) {
        let points = self.pollution_points(c);
        if points <= 0 {
            return;
        }
        let owner = self.city(c).owner;
        let range = 256 - self.player(owner).research_count as i32 * self.setup.difficulty as i32 / 2;
        if 2 * points <= self.rng.below(range.max(1) as u32) as i32 {
            return;
        }
        let mut cands = Vec::new();
        for idx in 1..21u8 {
            if !self.city(c).works(idx) {
                continue;
            }
            if let Some((x, y)) = self.radius_tile(c, idx) {
                let t = self.map.tile(x, y);
                if t.is_land() && !t.has(tile_flags::POLLUTION) {
                    cands.push((x, y));
                }
            }
        }
        if let Some(&(x, y)) = self.rng.pick(&cands) {
            self.map.get_mut(x, y).unwrap().set(tile_flags::POLLUTION, true);
            self.polluted_tiles += 1;
            self.push_event(GameEvent::Pollution { city: c, at: (x, y) });
        }
    }

    /// "Risk of MELTDOWN!": a city in disorder with a Nuclear Plant melts down one turn in
    /// three until its owner knows Fusion Power (CIV.EXE 0x12f46).
    pub fn meltdown_roll(&mut self, c: CityId) {
        let city = self.city(c);
        if !city.disorder || !city.has_improvement(imp::NUCLEAR_PLANT) || self.player(city.owner).knows(adv::FUSION_POWER) {
            return;
        }
        if self.rng.below(3) == 0 {
            self.meltdown(c);
        }
    }

    /// The Nuclear Plant is lost; every city on the 3x3 squares around loses half its size and
    /// the other land squares are polluted two times in three (0x1a5d9).
    pub fn meltdown(&mut self, c: CityId) {
        self.city_mut(c).remove_improvement(imp::NUCLEAR_PLANT);
        let (cx, cy) = self.city(c).pos();
        for (x, y) in std::iter::once((cx, cy)).chain(super::Map::neighbours8(cx, cy)) {
            let Some(t) = self.map.get(x, y) else { continue };
            if let Some(hit) = t.city {
                let loss = self.city(hit).size as i32 / 2;
                if loss > 0 {
                    self.shrink_city(hit, loss);
                }
            } else if self.rng.below(3) != 0 && t.is_land() && !t.has(tile_flags::POLLUTION) {
                self.map.get_mut(x, y).unwrap().set(tile_flags::POLLUTION, true);
                self.polluted_tiles += 1;
            }
        }
        self.push_event(GameEvent::Meltdown { city: c });
    }

    /// Polluted tiles needed before global warming can strike (rises after each episode).
    pub fn warming_threshold(&self) -> u16 {
        8 + 4 * self.global_warming_count as u16
    }

    /// Global warming check for the whole world (once per turn).
    pub fn global_warming_check(&mut self) {
        if self.polluted_tiles < self.warming_threshold() {
            return;
        }
        if !self.rng.chance(self.polluted_tiles.min(100) as u32) {
            return;
        }
        self.global_warming();
    }

    /// Apply one episode of global warming: coastal and low land turns drier / wetter.
    pub fn global_warming(&mut self) {
        let mut changed = 0;
        let mut tries = 0;
        while changed < WARMING_TILES && tries < 2000 {
            tries += 1;
            let x = self.rng.below(crate::rules::MAP_W as u32) as i32;
            let y = self.rng.below(crate::rules::MAP_H as u32) as i32;
            let t = self.map.tile(x, y);
            if t.is_ocean() || t.city.is_some() {
                continue;
            }
            let coastal = self.map.is_coastal(x, y);
            let new = match t.terrain {
                Terrain::Grassland => Some(Terrain::Plains),
                Terrain::Plains => Some(Terrain::Desert),
                Terrain::Forest => Some(Terrain::Jungle),
                Terrain::Jungle => Some(Terrain::Swamp),
                Terrain::Swamp if coastal => Some(Terrain::Ocean),
                Terrain::Tundra => Some(Terrain::Plains),
                Terrain::Arctic => Some(Terrain::Tundra),
                _ => None,
            };
            let Some(nt) = new else { continue };
            // prefer coastal/low land: inland tiles only change half of the time
            if !coastal && self.rng.chance(50) {
                continue;
            }
            let tm = self.map.get_mut(x, y).unwrap();
            tm.terrain = nt;
            tm.special = false;
            if nt == Terrain::Ocean {
                tm.flags = 0;
                tm.worked_by = None;
            } else {
                tm.set(tile_flags::IRRIGATION, false);
                tm.set(tile_flags::MINE, false);
            }
            changed += 1;
        }
        if changed > 0 {
            self.map.compute_continents();
        }
        self.global_warming_count = self.global_warming_count.saturating_add(1);
        self.push_event(GameEvent::GlobalWarming);
    }
}

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
}
