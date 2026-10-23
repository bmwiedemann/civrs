//! Unit movement: terrain costs, zones of control, boarding ships, triggering combat.

use super::map::{tile_flags, Dir8, Map, UnitId};
use super::unit::UnitOrder;
use super::{Game, MoveOutcome, RuleError};
use crate::rules::{self, unit_ids, Domain};

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
        self.step(id, d, true)
    }

    /// Complete a move that stopped with `MoveOutcome::CaravanArrived` ("Keep moving").
    pub fn move_on(&mut self, id: UnitId, d: Dir8) -> Result<MoveOutcome, RuleError> {
        self.step(id, d, false)
    }

    fn step(&mut self, id: UnitId, d: Dir8, ask_caravan: bool) -> Result<MoveOutcome, RuleError> {
        let u = self.unit(id).clone();
        if u.moves_left == 0 {
            return Err(RuleError::NoMoves);
        }
        let (dx, dy) = d.delta();
        let (tx, ty) = (Map::wrap_x(u.x + dx), u.y + dy);
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
            // a Diplomat meeting a lone foreign unit outside a city may bribe it, at war or not
            // (CIV.EXE 0x5da5, o 0x1dbae): the human is asked; the computer pays when half its
            // treasury covers the price; then the Diplomat moves on to its new unit
            if u.kind == unit_ids::DIPLOMAT && city_here.is_none() && units_here.len() == 1 {
                let target_id = units_here[0];
                if self.is_human(slot) {
                    return Ok(MoveOutcome::BribeOffer(target_id));
                }
                if self.player(slot).gold / 2 < self.bribe_cost(target_id) {
                    return Ok(MoveOutcome::Blocked);
                }
                self.bribe_unit(slot, target_id);
                return self.step(id, d, ask_caravan);
            }
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
                // ships bombard units on land, all but the Submarine (CIV.EXE 0x5efb)
                if u.kind == unit_ids::SUBMARINE && target.is_land() {
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

        // a human's caravan coming into one of its own cities may stop to trade or help build
        // a wonder (CIV.EXE 0x5c37)
        if ask_caravan && u.kind == unit_ids::CARAVAN && self.is_human(slot) {
            if let Some(c) = self.city_at(tx, ty).filter(|c| self.caravan_menu_wanted(&u, *c)) {
                return Ok(MoveOutcome::CaravanArrived(c));
            }
        }

        // Terrain legality.
        match u.info().domain {
            Domain::Land => {
                if target.is_ocean() {
                    // board a ship with room
                    let ship = self.units_at(tx, ty).into_iter().find(|s| {
                        let su = self.unit(*s);
                        su.owner == slot && su.carries_land_units() && self.cargo_count(*s) < su.info().capacity as usize
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
        // CIV.EXE 0x634a: a land unit with exactly one full move point always gets through;
        // with any other amount short of the cost the move fails (and ends the unit's turn)
        // when random(cost) exceeds the points left
        if cost > u.moves_left && u.is_land() && u.moves_left != 3 && self.rng.below(cost as u32) > u.moves_left as u32 {
            self.unit_mut(id).moves_left = 0;
            return Ok(MoveOutcome::Blocked);
        }
        let had_hut = target.has(tile_flags::HUT);
        // a ship leaving a city takes along the sentried units of its kind there, land units
        // onto transports and aircraft onto a Carrier, up to its capacity (CIV.EXE 0x6932)
        if u.is_sea() && u.info().capacity > 0 && self.map.tile(u.x, u.y).is_land() {
            let air = u.kind == unit_ids::CARRIER;
            let waiting: Vec<UnitId> = self
                .all_units()
                .filter(|o| o.owner == slot && o.id != id && (o.x, o.y) == (u.x, u.y) && o.carried_by.is_none() && o.order == UnitOrder::Sentry)
                .filter(|o| if air { o.is_air() } else { o.is_land() })
                .map(|o| o.id)
                .collect();
            let room = (u.info().capacity as usize).saturating_sub(self.cargo_count(id));
            for o in waiting.into_iter().take(room) {
                self.unit_mut(o).carried_by = Some(id);
            }
        }
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
            // a Carrier takes up to 8 air units, Nuclear included
            let carrier = self.units_at(tx, ty).into_iter().find(|s| {
                *s != id && self.unit(*s).kind == unit_ids::CARRIER && self.unit(*s).owner == slot && self.cargo_count(*s) < self.unit(*s).info().capacity as usize
            });
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
                su.owner == unit.owner && su.carries_land_units()
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

    /// Ships of `slot` get one more move per turn from the Lighthouse or Magellan's Expedition;
    /// having both gives no more (CIV.EXE 0x40e2).
    pub fn extra_sea_move(&self, slot: super::Slot) -> bool {
        self.owner_has_wonder(slot, rules::wonder_ids::LIGHTHOUSE) || self.owner_has_wonder(slot, rules::wonder_ids::MAGELLANS_EXPEDITION)
    }

    /// Reset movement points at the start of a player's turn; handles fortify completion.
    pub fn reset_moves(&mut self, slot: super::Slot) {
        let extra_sea_move = self.extra_sea_move(slot);
        for u in self.units.iter_mut().flatten() {
            if u.owner != slot {
                continue;
            }
            let mut m = u.full_moves();
            if u.is_sea() && extra_sea_move {
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

#[cfg(test)]
mod tests {
    use crate::game::map::{Dir8, UnitId};
    use crate::game::{Game, GameSetup, MoveOutcome};
    use crate::rules::{unit_ids, Terrain};

    /// A Militia west of a bare mountain square, with `moves` thirds left.
    fn militia_before_mountains(seed: u64, moves: u8) -> (Game, UnitId) {
        let mut g = Game::new(GameSetup { seed, ..Default::default() }, None);
        let slot = g.human_slot;
        let (x, y) = g.unit(g.units_of(slot)[0]).pos();
        let id = g.add_unit(slot, unit_ids::MILITIA, x, y);
        let t = g.map.get_mut(x + 1, y).unwrap();
        t.terrain = Terrain::Mountains;
        t.flags = 0;
        g.unit_mut(id).moves_left = moves;
        (g, id)
    }

    #[test]
    fn one_full_move_point_always_enters_mountains() {
        for seed in 0..20 {
            let (mut g, id) = militia_before_mountains(seed, 3);
            assert_eq!(g.try_move(id, Dir8::E), Ok(MoveOutcome::Moved), "seed {seed}");
            assert_eq!(g.unit(id).moves_left, 0);
        }
    }

    #[test]
    fn partial_move_points_sometimes_fail() {
        let failed = (0..40)
            .filter(|seed| {
                let (mut g, id) = militia_before_mountains(*seed, 2);
                g.try_move(id, Dir8::E) == Ok(MoveOutcome::Blocked)
            })
            .count();
        // random(9) > 2 fails two times in three
        assert!((15..40).contains(&failed), "{failed} of 40 failed");
    }

    #[test]
    fn lighthouse_and_magellan_give_ships_one_move_together() {
        use crate::rules::wonder_ids as won;
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        let (x, y) = g.city(c).pos();
        let ship = g.add_unit(slot, unit_ids::TRIREME, x, y);
        let full = g.unit(ship).full_moves();
        g.reset_moves(slot);
        assert_eq!(g.unit(ship).moves_left, full);
        for w in [won::LIGHTHOUSE, won::MAGELLANS_EXPEDITION] {
            g.city_mut(c).add_wonder(w);
            g.wonders_built[w as usize] = Some(c);
            g.reset_moves(slot);
            assert_eq!(g.unit(ship).moves_left, full + 3);
        }
    }

    #[test]
    fn cruisers_see_two_squares_but_only_over_ocean() {
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        let slot = g.human_slot;
        let (x, y) = (40, 25);
        for yy in y - 4..=y + 4 {
            for xx in x - 4..=x + 4 {
                let t = g.map.get_mut(xx, yy).unwrap();
                t.terrain = Terrain::Ocean;
                t.city = None;
            }
        }
        g.map.get_mut(x + 2, y).unwrap().terrain = Terrain::Grassland;
        g.map.get_mut(x + 1, y + 1).unwrap().terrain = Terrain::Grassland;
        g.units.iter_mut().flatten().filter(|u| u.owner == slot).for_each(|u| { u.x = 0; u.y = 0; });
        g.add_unit(slot, unit_ids::CRUISER, x, y);
        g.refresh_vision(slot);
        assert!(g.visible_to(slot, x + 1, y + 1), "land next to the ship");
        assert!(g.visible_to(slot, x - 2, y + 2), "ocean two squares away");
        assert!(!g.visible_to(slot, x + 2, y), "land two squares away");
        assert!(!g.visible_to(slot, x + 3, y), "three squares away");
    }

    #[test]
    fn carriers_take_eight_aircraft_and_no_land_units() {
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        let slot = g.human_slot;
        let (x, y) = (40, 25);
        for xx in x - 1..=x + 1 {
            let t = g.map.get_mut(xx, y).unwrap();
            t.terrain = Terrain::Ocean;
            t.city = None;
        }
        g.map.get_mut(x - 1, y).unwrap().terrain = Terrain::Grassland;
        let carrier = g.add_unit(slot, unit_ids::CARRIER, x, y);
        let militia = g.add_unit(slot, unit_ids::MILITIA, x - 1, y);
        assert_eq!(g.try_move(militia, Dir8::E), Ok(MoveOutcome::Blocked));
        for n in 0..9 {
            let f = g.add_unit(slot, unit_ids::FIGHTER, x + 1, y);
            assert_eq!(g.try_move(f, Dir8::W), Ok(MoveOutcome::Moved));
            assert_eq!(g.unit(f).carried_by.is_some(), n < 8, "fighter {n}");
        }
        assert_eq!(g.cargo_count(carrier), 8);
    }

    #[test]
    fn ships_but_submarines_bombard_land_units() {
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        let slot = g.human_slot;
        let enemy = g.civ_slots().into_iter().find(|s| *s != slot).unwrap();
        g.players[slot as usize].relations[enemy as usize] = crate::game::Relation::War;
        g.players[enemy as usize].relations[slot as usize] = crate::game::Relation::War;
        let (x, y) = (40, 25);
        for xx in x - 1..=x {
            let t = g.map.get_mut(xx, y).unwrap();
            t.terrain = if xx == x { Terrain::Ocean } else { Terrain::Grassland };
            t.city = None;
        }
        g.add_unit(enemy, unit_ids::SETTLERS, x - 1, y);
        let sub = g.add_unit(slot, unit_ids::SUBMARINE, x, y);
        assert_eq!(g.try_move(sub, Dir8::W), Ok(MoveOutcome::Blocked));
        let bb = g.add_unit(slot, unit_ids::BATTLESHIP, x, y);
        assert!(matches!(g.try_move(bb, Dir8::W), Ok(MoveOutcome::Attacked { .. })));
        assert_eq!(g.unit(bb).pos(), (x, y));
    }

    #[test]
    fn sentried_units_board_a_ship_leaving_their_city() {
        use crate::game::UnitOrder;
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        let slot = g.human_slot;
        let (x, y) = (40, 25);
        g.map.get_mut(x, y).unwrap().terrain = Terrain::Grassland;
        g.map.get_mut(x + 1, y).unwrap().terrain = Terrain::Ocean;
        g.add_city(slot, "Port".into(), x, y);
        let ship = g.add_unit(slot, unit_ids::SAIL, x, y);
        let sentried = g.add_unit(slot, unit_ids::MILITIA, x, y);
        g.unit_mut(sentried).order = UnitOrder::Sentry;
        let idle = g.add_unit(slot, unit_ids::MILITIA, x, y);
        assert_eq!(g.try_move(ship, Dir8::E), Ok(MoveOutcome::Moved));
        assert_eq!(g.unit(sentried).pos(), (x + 1, y));
        assert_eq!(g.unit(sentried).carried_by, Some(ship));
        assert_eq!(g.unit(idle).pos(), (x, y));
    }

    #[test]
    fn diplomats_bribe_lone_units() {
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        let human = g.human_slot;
        let other = g.civ_slots().into_iter().find(|s| *s != human).unwrap();
        let (x, y) = (40, 25);
        for xx in x - 1..=x + 1 {
            let t = g.map.get_mut(xx, y).unwrap();
            t.terrain = Terrain::Grassland;
            t.city = None;
        }
        // no capital: distance 16, so (gold + 750) / 18 x cost, halved for a Phalanx
        for c in g.all_cities().map(|c| c.id).collect::<Vec<_>>() {
            g.remove_city(c);
        }
        g.player_mut(other).gold = 150;
        let phalanx = g.add_unit(other, unit_ids::PHALANX, x, y);
        assert_eq!(g.bribe_cost(phalanx), 900 / 18 * 2 / 2);
        // the human is asked first
        let dip = g.add_unit(human, unit_ids::DIPLOMAT, x - 1, y);
        assert_eq!(g.try_move(dip, Dir8::E), Ok(MoveOutcome::BribeOffer(phalanx)));
        // a computer player with enough gold buys the unit and moves on to it
        g.remove_unit(dip);
        let third = g.civ_slots().into_iter().find(|s| *s != human && *s != other).unwrap();
        g.player_mut(third).gold = 100;
        let dip = g.add_unit(third, unit_ids::DIPLOMAT, x + 1, y);
        assert_eq!(g.try_move(dip, Dir8::W), Ok(MoveOutcome::Moved));
        let at: Vec<_> = g.units_at(x, y).into_iter().map(|u| (g.unit(u).owner, g.unit(u).kind)).collect();
        assert!(at.contains(&(third, unit_ids::PHALANX)) && at.contains(&(third, unit_ids::DIPLOMAT)), "{at:?}");
        assert!(!at.iter().any(|(o, _)| *o == other));
        assert_eq!(g.player(third).gold, 50);
    }
}
