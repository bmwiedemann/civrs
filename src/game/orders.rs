//! Unit orders: fortify, sentry, goto, terraforming, pillage, found city, disband, home.

use super::map::{tile_flags, Map, UnitId, CityId};
use super::unit::UnitOrder;
use super::{Game, RuleError};
use crate::rules::{self, unit_ids, Terraform, Terrain};

impl Game {
    pub fn give_order(&mut self, id: UnitId, o: UnitOrder) -> Result<(), RuleError> {
        let u = self.unit(id).clone();
        let t = self.map.tile(u.x, u.y);
        let is_settlers = u.kind == unit_ids::SETTLERS;
        match o {
            UnitOrder::None | UnitOrder::Skip | UnitOrder::Sentry | UnitOrder::Fortify => {
                if o == UnitOrder::Fortify && (u.info().def == 0 || u.is_air() || is_settlers) {
                    return Err(RuleError::Other("This unit cannot fortify."));
                }
                self.unit_mut(id).order = o;
                self.unit_mut(id).goto = None;
            }
            UnitOrder::Fortified | UnitOrder::Goto => {
                self.unit_mut(id).order = o;
            }
            UnitOrder::Road => {
                if !is_settlers {
                    return Err(RuleError::Settlers);
                }
                if t.is_ocean() {
                    return Err(RuleError::Other("Roads cannot be built at sea."));
                }
                if t.has(tile_flags::ROAD) {
                    if !self.player(u.owner).knows(rules::advance_ids::RAILROAD) || t.has(tile_flags::RAILROAD) {
                        return Err(RuleError::Other("A road is already here."));
                    }
                    self.start_work(id, UnitOrder::Road, rules::RAILROAD_TURNS);
                } else {
                    if t.terrain == Terrain::River && !self.player(u.owner).knows(rules::advance_ids::BRIDGE_BUILDING) {
                        return Err(RuleError::Other("Bridge Building is required to build roads across rivers."));
                    }
                    self.start_work(id, UnitOrder::Road, rules::ROAD_TURNS);
                }
            }
            UnitOrder::Irrigate => {
                if !is_settlers {
                    return Err(RuleError::Settlers);
                }
                let info = t.terrain.info();
                match info.irrigate {
                    Terraform::No => return Err(RuleError::NoIrr),
                    Terraform::Bonus(_) => {
                        if t.has(tile_flags::IRRIGATION) {
                            return Err(RuleError::Other("This square is already irrigated."));
                        }
                        if !self.irrigation_water(u.x, u.y) {
                            return Err(RuleError::NoWater);
                        }
                    }
                    Terraform::Becomes(_) => {}
                }
                self.start_work(id, UnitOrder::Irrigate, info.irrigate_turns);
            }
            UnitOrder::Mine => {
                if !is_settlers {
                    return Err(RuleError::Settlers);
                }
                let info = t.terrain.info();
                match info.mine {
                    Terraform::No => return Err(RuleError::NoMine),
                    Terraform::Bonus(_) => {
                        if t.has(tile_flags::MINE) {
                            return Err(RuleError::Other("A mine is already here."));
                        }
                    }
                    Terraform::Becomes(_) => {}
                }
                self.start_work(id, UnitOrder::Mine, info.mine_turns);
            }
            UnitOrder::Fortress => {
                if !is_settlers {
                    return Err(RuleError::Settlers);
                }
                if !self.player(u.owner).knows(rules::advance_ids::CONSTRUCTION) {
                    return Err(RuleError::Other("Construction is required to build fortresses."));
                }
                if t.is_ocean() || t.city.is_some() || t.has(tile_flags::FORTRESS) {
                    return Err(RuleError::Other("A fortress cannot be built here."));
                }
                self.start_work(id, UnitOrder::Fortress, rules::FORTRESS_TURNS);
            }
            UnitOrder::CleanPollution => {
                if !is_settlers {
                    return Err(RuleError::Settlers);
                }
                if !t.has(tile_flags::POLLUTION) {
                    return Err(RuleError::Other("There is no pollution here."));
                }
                self.start_work(id, UnitOrder::CleanPollution, rules::CLEAN_POLLUTION_TURNS);
            }
            UnitOrder::Pillage => {
                if u.info().att == 0 || !u.is_land() {
                    return Err(RuleError::Pillage);
                }
                if t.flags & (tile_flags::ROAD | tile_flags::RAILROAD | tile_flags::IRRIGATION | tile_flags::MINE) == 0 {
                    return Err(RuleError::Pillage);
                }
                self.start_work(id, UnitOrder::Pillage, 1);
            }
        }
        Ok(())
    }

    fn start_work(&mut self, id: UnitId, o: UnitOrder, turns: u8) {
        let u = self.unit_mut(id);
        u.order = o;
        u.order_turns = turns.max(1);
        u.moves_left = 0;
        u.goto = None;
    }

    /// Is there water (ocean, river, irrigated tile) orthogonally adjacent (city tiles count)?
    pub fn irrigation_water(&self, x: i32, y: i32) -> bool {
        if self.map.terrain(x, y) == Terrain::River {
            return true;
        }
        Map::neighbours4(x, y).any(|(nx, ny)| {
            let t = self.map.tile(nx, ny);
            t.is_ocean() || t.terrain == Terrain::River || t.has(tile_flags::IRRIGATION) || (t.city.is_some() && t.terrain.info().irrigate != Terraform::No)
        })
    }

    /// Advance terraforming work for all units of a slot (called at turn start).
    pub fn progress_work(&mut self, slot: super::Slot) {
        let ids: Vec<UnitId> = self.all_units().filter(|u| u.owner == slot && u.order_turns > 0).map(|u| u.id).collect();
        for id in ids {
            let u = self.unit(id).clone();
            if u.order_turns > 1 {
                self.unit_mut(id).order_turns -= 1;
                self.unit_mut(id).moves_left = 0;
                continue;
            }
            let (x, y) = (u.x, u.y);
            let t = self.map.tile(x, y);
            match u.order {
                UnitOrder::Road => {
                    let tm = self.map.get_mut(x, y).unwrap();
                    if tm.has(tile_flags::ROAD) {
                        tm.set(tile_flags::RAILROAD, true);
                    } else {
                        tm.set(tile_flags::ROAD, true);
                    }
                }
                UnitOrder::Irrigate => {
                    match t.terrain.info().irrigate {
                        Terraform::Becomes(nt) => {
                            let tm = self.map.get_mut(x, y).unwrap();
                            tm.terrain = nt;
                            tm.set(tile_flags::MINE, false);
                            tm.special = false;
                        }
                        _ => {
                            let tm = self.map.get_mut(x, y).unwrap();
                            tm.set(tile_flags::IRRIGATION, true);
                            tm.set(tile_flags::MINE, false);
                        }
                    }
                }
                UnitOrder::Mine => {
                    match t.terrain.info().mine {
                        Terraform::Becomes(nt) => {
                            let tm = self.map.get_mut(x, y).unwrap();
                            tm.terrain = nt;
                            tm.set(tile_flags::IRRIGATION, false);
                            tm.special = false;
                        }
                        _ => {
                            let tm = self.map.get_mut(x, y).unwrap();
                            tm.set(tile_flags::MINE, true);
                            tm.set(tile_flags::IRRIGATION, false);
                        }
                    }
                }
                UnitOrder::Fortress => {
                    self.map.get_mut(x, y).unwrap().set(tile_flags::FORTRESS, true);
                }
                UnitOrder::CleanPollution => {
                    let tm = self.map.get_mut(x, y).unwrap();
                    if tm.has(tile_flags::POLLUTION) {
                        tm.set(tile_flags::POLLUTION, false);
                        self.polluted_tiles = self.polluted_tiles.saturating_sub(1);
                    }
                }
                UnitOrder::Pillage => {
                    let tm = self.map.get_mut(x, y).unwrap();
                    if tm.has(tile_flags::IRRIGATION) || tm.has(tile_flags::MINE) {
                        tm.set(tile_flags::IRRIGATION, false);
                        tm.set(tile_flags::MINE, false);
                    } else if tm.has(tile_flags::RAILROAD) {
                        tm.set(tile_flags::RAILROAD, false);
                    } else {
                        tm.set(tile_flags::ROAD, false);
                    }
                }
                _ => {}
            }
            let um = self.unit_mut(id);
            um.order = UnitOrder::None;
            um.order_turns = 0;
        }
    }

    /// Found a city with a Settlers unit, or add it to an existing city on the tile.
    pub fn found_city(&mut self, id: UnitId) -> Result<CityId, RuleError> {
        let u = self.unit(id).clone();
        if u.kind != unit_ids::SETTLERS {
            return Err(RuleError::Settlers);
        }
        let t = self.map.tile(u.x, u.y);
        if let Some(c) = t.city {
            let city = self.city(c);
            if city.owner != u.owner {
                return Err(RuleError::Other("This is not your city."));
            }
            if city.size >= 10 && !city.has_improvement(rules::improvement_ids::AQUEDUCT) {
                return Err(RuleError::AddCity);
            }
            self.city_mut(c).size += 1;
            self.remove_unit(id);
            self.auto_assign(c);
            return Ok(c);
        }
        if t.is_ocean() {
            return Err(RuleError::Other("Cities cannot be built at sea."));
        }
        if Map::neighbours8(u.x, u.y).any(|(nx, ny)| self.city_at(nx, ny).is_some()) {
            return Err(RuleError::Other("Too close to another city."));
        }
        if self.cities_of(u.owner).len() >= 128 {
            return Err(RuleError::Other("PMAX"));
        }
        let name = self.next_city_name(u.owner);
        let c = self.add_city(u.owner, name, u.x, u.y);
        self.remove_unit(id);
        self.refresh_vision(u.owner);
        self.push_event(super::GameEvent::CityFounded { city: c });
        Ok(c)
    }

    pub fn disband(&mut self, id: UnitId) {
        let u = self.unit(id).clone();
        if let Some(c) = self.city_at(u.x, u.y) {
            // 50% of the unit's cost goes to the city's production
            let shields = u.info().cost as i32 * 10 / 2;
            if self.city(c).owner == u.owner {
                self.city_mut(c).shields += shields;
            }
        }
        self.remove_unit(id);
        self.refresh_vision(u.owner);
    }

    pub fn set_goto(&mut self, id: UnitId, x: i32, y: i32) {
        let u = self.unit_mut(id);
        u.goto = Some((x, y));
        u.order = UnitOrder::Goto;
    }

    pub fn set_home(&mut self, id: UnitId) -> Result<(), RuleError> {
        let u = self.unit(id).clone();
        match self.city_at(u.x, u.y) {
            Some(c) if self.city(c).owner == u.owner => {
                self.unit_mut(id).home = Some(c);
                Ok(())
            }
            _ => Err(RuleError::Other("The unit must be in one of your cities.")),
        }
    }

    /// Wake up all units carried by this ship (so they can be ordered off).
    pub fn unload(&mut self, id: UnitId) {
        for c in self.cargo_of(id) {
            let cu = self.unit_mut(c);
            cu.order = UnitOrder::None;
        }
    }

    pub fn wait_unit(&mut self, id: UnitId) {
        // just advance the active pointer; the unit keeps its moves
        self.active_unit = Some(id);
        self.active_unit = self.next_active_unit();
    }

    pub fn skip_unit(&mut self, id: UnitId) {
        self.unit_mut(id).order = UnitOrder::Skip;
        self.unit_mut(id).moves_left = 0;
    }
}
