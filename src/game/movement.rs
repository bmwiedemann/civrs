//! Unit movement: terrain costs, zones of control, boarding ships, triggering combat.

use super::map::{tile_flags, Dir8, Map, UnitId};
use super::unit::UnitOrder;
use super::{Game, MoveOutcome, RuleError};
use crate::rules::{self, unit_ids, Domain, Terrain};

impl Game {
    /// Movement cost in thirds for `unit` moving from (fx,fy) to (tx,ty), ignoring legality.
    pub fn move_cost(&self, unit: &super::Unit, from: (i32, i32), to: (i32, i32)) -> u8 {
        let ft = self.map.tile(from.0, from.1);
        let tt = self.map.tile(to.0, to.1);
        if unit.is_air() || unit.is_sea() {
            return 3;
        }
        let from_rail = ft.has(tile_flags::RAILROAD) || ft.city.is_some();
        let to_rail = tt.has(tile_flags::RAILROAD) || tt.city.is_some();
        if from_rail && to_rail && !self.is_enemy_at(unit.owner, to.0, to.1) {
            return 0;
        }
        let from_road = ft.has(tile_flags::ROAD) || from_rail;
        let to_road = tt.has(tile_flags::ROAD) || to_rail;
        if from_road && to_road {
            return 1;
        }
        if ft.terrain == Terrain::River && tt.terrain == Terrain::River && !Dir8::from_delta(to.0 - from.0, to.1 - from.1).map(|d| d.is_diagonal()).unwrap_or(true) {
            return 1;
        }
        if tt.is_ocean() {
            return 3;
        }
        tt.terrain.info().move_cost * 3
    }

    /// Would moving the land unit from `from` to `to` violate zones of control?
    pub fn zoc_blocks(&self, unit: &super::Unit, from: (i32, i32), to: (i32, i32)) -> bool {
        if !unit.is_land() || matches!(unit.kind, unit_ids::DIPLOMAT | unit_ids::CARAVAN) {
            return false;
        }
        // entering own city or tile with own unit is always allowed
        if let Some(c) = self.city_at(to.0, to.1) {
            if self.city(c).owner == unit.owner {
                return false;
            }
        }
        if self.all_units().any(|u| u.owner == unit.owner && u.id != unit.id && (u.x, u.y) == to && !u.is_air()) {
            return false;
        }
        let adj_enemy = |p: (i32, i32)| {
            Map::neighbours8(p.0, p.1).any(|(nx, ny)| {
                self.all_units().any(|u| u.x == nx && u.y == ny && u.is_land() && u.owner != unit.owner && (unit.owner == 0 || u.owner == 0 || self.player(unit.owner).at_war(u.owner)))
            })
        };
        adj_enemy(from) && adj_enemy(to)
    }

    /// Try to move the unit one step. Handles combat, city capture, huts and boarding.
    pub fn try_move(&mut self, id: UnitId, d: Dir8) -> Result<MoveOutcome, RuleError> {
        let u = self.unit(id).clone();
        if u.moves_left == 0 {
            return Err(RuleError::NoMoves);
        }
        let (dx, dy) = d.delta();
        let (tx, ty) = (u.x + dx, u.y + dy);
        if !Map::in_bounds(tx, ty) {
            return Ok(MoveOutcome::Blocked);
        }
        let target = self.map.tile(tx, ty);
        let slot = u.owner;
        let foreign_owner = self.owner_at(tx, ty).filter(|o| *o != slot);

        // Interaction with foreign units/cities.
        if let Some(o) = foreign_owner {
            let at_war = slot == 0 || o == 0 || self.player(slot).at_war(o);
            let city_here = self.city_at(tx, ty);
            let units_here = self.units_at(tx, ty);
            if !at_war {
                // Caravan / diplomat may enter foreign cities at peace.
                if city_here.is_some() && matches!(u.kind, unit_ids::CARAVAN | unit_ids::DIPLOMAT) {
                    return Ok(MoveOutcome::EnteredForeignCity(city_here.unwrap()));
                }
                return Ok(MoveOutcome::Blocked);
            }
            // At war: attack if there are units; capture empty city with a ground unit.
            if !units_here.is_empty() {
                if u.info().att == 0 {
                    if city_here.is_some() && matches!(u.kind, unit_ids::DIPLOMAT | unit_ids::CARAVAN) {
                        return Ok(MoveOutcome::EnteredForeignCity(city_here.unwrap()));
                    }
                    return Ok(MoveOutcome::Blocked);
                }
                // only fighters may attack air units (unless the air unit is in a city/carrier)
                let all_air = units_here.iter().all(|t| self.unit(*t).is_air());
                if all_air && city_here.is_none() && u.kind != unit_ids::FIGHTER {
                    return Err(RuleError::Fighter);
                }
                if u.is_land() && target.is_ocean() {
                    return Ok(MoveOutcome::Blocked);
                }
                if u.is_sea() && target.is_land() && city_here.is_none() {
                    return Ok(MoveOutcome::Blocked);
                }
                if u.carried_by.is_some() && u.is_land() {
                    return Err(RuleError::Amphib);
                }
                if u.kind == unit_ids::NUCLEAR {
                    self.nuke(id, tx, ty);
                    return Ok(MoveOutcome::Attacked { won: true, at: (tx, ty), defender: unit_ids::NUCLEAR });
                }
                return Ok(self.attack(id, tx, ty));
            }
            if let Some(c) = city_here {
                if u.kind == unit_ids::NUCLEAR {
                    self.nuke(id, tx, ty);
                    return Ok(MoveOutcome::Attacked { won: true, at: (tx, ty), defender: unit_ids::NUCLEAR });
                }
                if !u.is_land() || u.info().att == 0 && !matches!(u.kind, unit_ids::DIPLOMAT | unit_ids::CARAVAN) {
                    if matches!(u.kind, unit_ids::DIPLOMAT | unit_ids::CARAVAN) {
                        return Ok(MoveOutcome::EnteredForeignCity(c));
                    }
                    return Err(RuleError::Occupy);
                }
                if matches!(u.kind, unit_ids::DIPLOMAT | unit_ids::CARAVAN) {
                    return Ok(MoveOutcome::EnteredForeignCity(c));
                }
                return Ok(self.capture_city(id, c));
            }
        }

        // Terrain legality.
        match u.info().domain {
            Domain::Land => {
                if target.is_ocean() {
                    // board a ship with room
                    let ship = self.units_at(tx, ty).into_iter().find(|s| {
                        let su = self.unit(*s);
                        su.owner == slot && su.is_sea() && su.info().capacity > 0 && self.cargo_count(*s) < su.info().capacity as usize
                    });
                    match ship {
                        Some(s) => {
                            if u.moves_left < 3 && u.moves_left < self.move_cost(&u, (u.x, u.y), (tx, ty)) {
                                // partial move allowed
                            }
                            self.place_unit(id, tx, ty, 3);
                            self.unit_mut(id).carried_by = Some(s);
                            self.unit_mut(id).order = UnitOrder::Sentry;
                            self.unit_mut(id).moves_left = 0;
                            self.refresh_vision(slot);
                            return Ok(MoveOutcome::Boarded(s));
                        }
                        None => return Ok(MoveOutcome::Blocked),
                    }
                }
                if self.zoc_blocks(&u, (u.x, u.y), (tx, ty)) {
                    return Err(RuleError::Zoc);
                }
            }
            Domain::Sea => {
                if target.is_land() {
                    let own_city = self.city_at(tx, ty).map(|c| self.city(c).owner == slot).unwrap_or(false);
                    if !own_city {
                        return Ok(MoveOutcome::Blocked);
                    }
                }
            }
            Domain::Air => {}
        }

        let cost = self.move_cost(&u, (u.x, u.y), (tx, ty));
        if cost > u.moves_left {
            // Civ1: a unit with fewer points than needed may still move with some probability
            let p = (u.moves_left as u32 * 100) / cost.max(1) as u32;
            if !self.rng.chance(p.max(1)) {
                self.unit_mut(id).moves_left = 0;
                return Ok(MoveOutcome::Blocked);
            }
        }
        let had_hut = target.has(tile_flags::HUT);
        self.place_unit(id, tx, ty, cost);
        // land unit leaving a ship
        if u.carried_by.is_some() && target.is_land() {
            self.unit_mut(id).carried_by = None;
        }
        // units aboard move with ship
        if u.is_sea() {
            let cargo: Vec<UnitId> = self.all_units().filter(|c| c.carried_by == Some(id)).map(|c| c.id).collect();
            for c in cargo {
                let cu = self.unit_mut(c);
                cu.x = tx;
                cu.y = ty;
            }
        }
        // aircraft landing in a city / carrier refuels
        if u.is_air() {
            let carrier = self.units_at(tx, ty).into_iter().find(|s| *s != id && self.unit(*s).kind == unit_ids::CARRIER && self.unit(*s).owner == slot);
            if self.city_at(tx, ty).map(|c| self.city(c).owner == slot).unwrap_or(false) {
                self.unit_mut(id).fuel = 0;
                self.unit_mut(id).carried_by = None;
            } else if let Some(cr) = carrier {
                self.unit_mut(id).fuel = 0;
                self.unit_mut(id).carried_by = Some(cr);
            } else {
                self.unit_mut(id).carried_by = None;
            }
        }
        self.refresh_vision(slot);
        if had_hut {
            self.map.get_mut(tx, ty).unwrap().set(tile_flags::HUT, false);
            self.enter_hut(id);
            return Ok(MoveOutcome::EnteredHut);
        }
        Ok(MoveOutcome::Moved)
    }

    /// Move a unit to a tile and deduct movement cost (thirds).
    pub fn place_unit(&mut self, id: UnitId, x: i32, y: i32, cost: u8) {
        let u = self.unit_mut(id);
        u.x = x;
        u.y = y;
        u.moves_left = u.moves_left.saturating_sub(cost);
        if u.order == UnitOrder::Fortified || u.order == UnitOrder::Fortify {
            u.order = UnitOrder::None;
        }
    }

    pub fn cargo_count(&self, ship: UnitId) -> usize {
        self.all_units().filter(|u| u.carried_by == Some(ship)).count()
    }

    pub fn cargo_of(&self, ship: UnitId) -> Vec<UnitId> {
        self.all_units().filter(|u| u.carried_by == Some(ship)).map(|u| u.id).collect()
    }

    /// Can the unit (by rules of terrain only) enter the tile? Used by pathfinding.
    pub fn can_enter_terrain(&self, unit: &super::Unit, x: i32, y: i32) -> bool {
        if !Map::in_bounds(x, y) {
            return false;
        }
        let t = self.map.tile(x, y);
        match unit.info().domain {
            Domain::Land => t.is_land() || self.units_at(x, y).iter().any(|s| {
                let su = self.unit(*s);
                su.owner == unit.owner && su.is_sea() && su.info().capacity > 0
            }),
            Domain::Sea => t.is_ocean() || self.city_at(x, y).map(|c| self.city(c).owner == unit.owner).unwrap_or(false),
            Domain::Air => true,
        }
    }

    /// Simple A* path for goto; returns the next step direction.
    pub fn next_goto_step(&self, id: UnitId) -> Option<Dir8> {
        let u = self.unit(id);
        let goal = u.goto?;
        if (u.x, u.y) == goal {
            return None;
        }
        let path = crate::ai::path::astar(self, u, goal)?;
        if path.len() < 2 {
            return None;
        }
        Dir8::from_delta(path[1].0 - u.x, path[1].1 - u.y)
    }

    /// Reset movement points at the start of a player's turn; handles fortify completion.
    pub fn reset_moves(&mut self, slot: super::Slot) {
        let magellan = self.owner_has_wonder(slot, rules::wonder_ids::MAGELLANS_EXPEDITION);
        for u in self.units.iter_mut().flatten() {
            if u.owner != slot {
                continue;
            }
            let mut m = u.full_moves();
            if u.is_sea() && magellan {
                m += 3;
            }
            u.moves_left = m;
            if u.order == UnitOrder::Fortify {
                u.order = UnitOrder::Fortified;
            }
            if u.order == UnitOrder::Skip {
                u.order = UnitOrder::None;
            }
        }
        // sentried units wake up when an enemy unit comes adjacent
        let ids: Vec<UnitId> = self.all_units().filter(|u| u.owner == slot && u.order == UnitOrder::Sentry && u.carried_by.is_none()).map(|u| u.id).collect();
        for id in ids {
            let (x, y) = self.unit(id).pos();
            let enemy_near = Map::neighbours8(x, y).any(|(nx, ny)| self.is_enemy_at(slot, nx, ny));
            if enemy_near {
                self.unit_mut(id).order = UnitOrder::None;
            }
        }
    }
}
