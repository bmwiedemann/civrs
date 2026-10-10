//! Pollution and global warming.
//!
//! Pollution chance per city per turn [approx]: production beyond 20 shields (reduced by a
//! Recycling Center to a third, halved by Hydro/Nuclear plants or the Hoover Dam) plus
//! population pollution `size * techs / 2` where `techs` counts Industrialization, Automobile,
//! Mass Production and Plastics (removed entirely by Mass Transit). The result is a percentage.

use super::events::GameEvent;
use super::map::{tile_flags, CityId};
use super::Game;
use crate::rules::{advance_ids as adv, improvement_ids as imp, wonder_ids as won, Terrain};

/// Tiles changed by one episode of global warming.
pub const WARMING_TILES: usize = 10;

impl Game {
    /// Percent chance that the city pollutes a tile this turn.
    pub fn pollution_chance(&self, c: CityId) -> i32 {
        let city = self.city(c);
        let p = self.player(city.owner);
        let y = self.city_yield(c);
        let mut prod = (y.shields - 20).max(0);
        if city.has_improvement(imp::RECYCLING_CNTR) {
            prod /= 3;
        }
        if city.has_improvement(imp::HYDRO_PLANT) || city.has_improvement(imp::NUCLEAR_PLANT) || self.owner_has_wonder(city.owner, won::HOOVER_DAM) {
            prod /= 2;
        }
        let techs = [adv::INDUSTRIALIZATION, adv::AUTOMOBILE, adv::MASS_PRODUCTION, adv::PLASTICS].iter().filter(|a| p.knows(**a)).count() as i32;
        let mut pop = city.size as i32 * techs / 2;
        if city.has_improvement(imp::MASS_TRANSIT) {
            pop = 0;
        }
        (prod + pop).clamp(0, 100)
    }

    /// Roll for pollution in a city; pollutes a random worked land tile on success.
    pub fn pollution_roll(&mut self, c: CityId) {
        let chance = self.pollution_chance(c);
        if chance <= 0 || !self.rng.chance(chance as u32) {
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
    fn pollution_chance_grows_with_industry() {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        assert_eq!(g.pollution_chance(c), 0);
        g.player_mut(slot).learn(adv::INDUSTRIALIZATION);
        g.player_mut(slot).learn(adv::AUTOMOBILE);
        g.city_mut(c).size = 8;
        g.fix_workers(c);
        assert!(g.pollution_chance(c) > 0);
        g.city_mut(c).add_improvement(imp::MASS_TRANSIT);
        assert_eq!(g.pollution_chance(c), 0);
        let before = g.global_warming_count;
        g.global_warming();
        assert_eq!(g.global_warming_count, before + 1);
    }
}
