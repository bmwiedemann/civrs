//! Unit orders: fortify, sentry, goto, terraforming, pillage, found city, disband, home.
//!
//! The order keys of CIV.EXE's command loop `move_units` (0x403e, dispatch 0x4bbe..0x58c1).
//! Settlers work is not a countdown there: a working unit keeps its order bits (unit +0: 0x02
//! road, 0x40 irrigation, 0x80 mine, 0xc0 fortress, 0x82 pollution) and every turn the loop
//! presses the order's key again for it (0x4492); each press re-checks the job, zeroes the
//! moves, adds one to the work counter (unit +5, `Unit::order_turns` here) and finishes the
//! job once the counter reaches the job's time — so the turn of the order is the first turn
//! of work. The counter goes back to 0 when a job is done or refused and when the unit moves
//! (0x6d00), but not when the unit is given another job.

use super::map::{tile_flags, Map, UnitId, CityId};
use super::unit::UnitOrder;
use super::{Game, RuleError};
use crate::rules::{self, unit_ids, Terraform, Terrain};

impl UnitOrder {
    /// Settlers jobs, which the turn loop repeats until they are done (`progress_work`).
    pub fn is_work(self) -> bool {
        matches!(self, UnitOrder::Road | UnitOrder::Irrigate | UnitOrder::Mine | UnitOrder::Fortress | UnitOrder::CleanPollution)
    }
}

/// The special resource of a square whose terrain changed: specials are a pattern of the
/// square (0x1a68:0x183c, rows 2..47), whatever the terrain; Grassland and River squares have
/// none of their own (their resource is the shield pattern).
fn positional_special(seed: u32, x: i32, y: i32, t: Terrain) -> bool {
    (2..48).contains(&y) && Map::special_at(seed, x, y) && t != Terrain::Grassland && t != Terrain::River
}

impl Game {
    pub fn give_order(&mut self, id: UnitId, o: UnitOrder) -> Result<(), RuleError> {
        let u = self.unit(id).clone();
        let is_settlers = u.kind == unit_ids::SETTLERS;
        match o {
            // 'f' on Settlers builds a fortress (0x5159)
            UnitOrder::Fortify if is_settlers => self.give_order(id, UnitOrder::Fortress),
            // 'f' (0x5137): land units only; the unit is done for the turn and becomes
            // fortified the next time the loop reaches it (0x4239)
            UnitOrder::Fortify => {
                if !u.is_land() {
                    return Err(RuleError::Other("This unit cannot fortify."));
                }
                let um = self.unit_mut(id);
                um.order = o;
                um.goto = None;
                um.moves_left = 0;
                Ok(())
            }
            // 's' (0x4ddf) also zeroes the moves; civrs leaves them, as the computer players'
            // ship loading (ai/naval_ai.rs) sentries units it still moves
            UnitOrder::None | UnitOrder::Skip | UnitOrder::Sentry => {
                self.unit_mut(id).order = o;
                self.unit_mut(id).goto = None;
                Ok(())
            }
            UnitOrder::Fortified | UnitOrder::Goto => {
                self.unit_mut(id).order = o;
                Ok(())
            }
            UnitOrder::Pillage => self.pillage(id),
            _ => {
                let r = self.work_step(id, o);
                if r.is_err() && is_settlers {
                    // a refused job drops the order bits and the counter (0x5091)
                    let um = self.unit_mut(id);
                    if um.order.is_work() {
                        um.order = UnitOrder::None;
                    }
                    um.order_turns = 0;
                }
                r
            }
        }
    }

    /// One turn of Settlers work on order `o`: the key handlers 'r' 0x5288, 'i' 0x53ca,
    /// 'm' 0x559a, 'f' 0x51be and 'p' 0x5048. Checks the job, spends the unit's moves, counts
    /// the turn (unit +5) and finishes the job when the count reaches its time.
    fn work_step(&mut self, id: UnitId, o: UnitOrder) -> Result<(), RuleError> {
        let u = self.unit(id).clone();
        if u.kind != unit_ids::SETTLERS {
            return Err(RuleError::Settlers);
        }
        let (x, y) = (u.x, u.y);
        let t = self.map.tile(x, y);
        let info = t.terrain.info();
        let knows = |g: &Game, a: u8| g.player(u.owner).knows(a);
        let turns = match o {
            UnitOrder::Road => {
                // (civrs refuses roads and fortresses at sea; CIV.EXE only reads the flags)
                if t.is_ocean() {
                    return Err(RuleError::Other("Roads cannot be built at sea."));
                }
                let road = t.has(tile_flags::ROAD);
                // a railroad needs RailRoad, a road and no city on the square; nothing goes
                // over a railroad (0x52b6..0x52d9)
                if (road && (!knows(self, rules::advance_ids::RAILROAD) || t.city.is_some())) || t.has(tile_flags::RAILROAD) {
                    return Err(RuleError::Other("A road is already here."));
                }
                // roads and railroads on a River need Bridge Building (0x5304)
                if t.terrain == Terrain::River && !knows(self, rules::advance_ids::BRIDGE_BUILDING) {
                    return Err(RuleError::Other("Bridge Building is required to build roads across rivers."));
                }
                if road {
                    rules::railroad_turns(t.terrain)
                } else {
                    rules::road_turns(t.terrain)
                }
            }
            UnitOrder::Irrigate => {
                // an irrigated square is refused before the table is read (0x53f6)
                if t.has(tile_flags::IRRIGATION) {
                    return Err(RuleError::Other("This square is already irrigated."));
                }
                match info.irrigate {
                    Terraform::No => return Err(RuleError::NoIrr),
                    Terraform::Bonus(_) if !self.irrigation_water(x, y) => return Err(RuleError::NoWater),
                    _ => {}
                }
                info.irrigate_turns
            }
            UnitOrder::Mine => {
                if t.has(tile_flags::MINE) {
                    return Err(RuleError::Other("A mine is already here."));
                }
                if info.mine == Terraform::No {
                    return Err(RuleError::NoMine);
                }
                info.mine_turns
            }
            UnitOrder::Fortress => {
                if !knows(self, rules::advance_ids::CONSTRUCTION) {
                    return Err(RuleError::Other("Construction is required to build fortresses."));
                }
                if t.city.is_some() || t.has(tile_flags::FORTRESS) || t.is_ocean() {
                    return Err(RuleError::Other("A fortress cannot be built here."));
                }
                rules::fortress_turns(t.terrain)
            }
            UnitOrder::CleanPollution => {
                if !t.has(tile_flags::POLLUTION) {
                    return Err(RuleError::Other("There is no pollution here."));
                }
                rules::CLEAN_POLLUTION_TURNS
            }
            _ => return Ok(()),
        };
        let um = self.unit_mut(id);
        um.order = o;
        um.moves_left = 0;
        um.goto = None;
        um.order_turns = um.order_turns.saturating_add(1);
        if um.order_turns < turns {
            return Ok(());
        }
        um.order = UnitOrder::None;
        um.order_turns = 0;
        self.finish_work(x, y, o);
        Ok(())
    }

    /// The square's change when a job is done (0x537b road/railroad, 0x5510 irrigation,
    /// 0x56ab mine, 0x5244 fortress, 0x50e9 pollution).
    fn finish_work(&mut self, x: i32, y: i32, o: UnitOrder) {
        let seed = self.map.seed;
        let info = self.map.tile(x, y).terrain.info();
        let result = match o {
            UnitOrder::Irrigate => info.irrigate,
            UnitOrder::Mine => info.mine,
            _ => Terraform::No,
        };
        let tm = self.map.get_mut(x, y).unwrap();
        match (o, result) {
            (UnitOrder::Road, _) => {
                if tm.has(tile_flags::ROAD) {
                    tm.set(tile_flags::RAILROAD, true);
                } else {
                    tm.set(tile_flags::ROAD, true);
                }
            }
            // a new terrain (set_pixel 0x1a68:0x17d5): irrigation and mine go (bits 6), the
            // square's hut bit is spent (es:0x740 |= 1)
            (_, Terraform::Becomes(nt)) => {
                tm.terrain = nt;
                tm.set(tile_flags::IRRIGATION | tile_flags::MINE | tile_flags::HUT, false);
                tm.special = positional_special(seed, x, y, nt);
            }
            (UnitOrder::Irrigate, _) => {
                tm.set(tile_flags::MINE, false);
                tm.set(tile_flags::IRRIGATION, true);
            }
            (UnitOrder::Mine, _) => {
                tm.set(tile_flags::IRRIGATION, false);
                tm.set(tile_flags::MINE, true);
            }
            (UnitOrder::Fortress, _) => tm.set(tile_flags::FORTRESS, true),
            (UnitOrder::CleanPollution, _) => {
                if tm.has(tile_flags::POLLUTION) {
                    tm.set(tile_flags::POLLUTION, false);
                    self.polluted_tiles = self.polluted_tiles.saturating_sub(1);
                }
            }
            _ => {}
        }
    }

    /// 'P' (0x4c80): done at once, for the unit's whole turn. Not on a city square and not
    /// for Diplomats, Caravans and Fighters (types >= 0x1a and 0xe, *PILLAGE); only with
    /// moves left. Irrigation and mine go together, else the railroad, else the road; a
    /// fortress stays. It ends any treaty between the human and the owner of the nearest city
    /// (0x1495:0xaa7(human, owner, 2)); the human is asked first (`pillage_treaty`).
    pub fn pillage(&mut self, id: UnitId) -> Result<(), RuleError> {
        let u = self.unit(id).clone();
        let t = self.map.tile(u.x, u.y);
        if t.city.is_some() || u.kind >= unit_ids::DIPLOMAT || u.kind == unit_ids::FIGHTER {
            return Err(RuleError::Pillage);
        }
        if u.moves_left == 0 {
            return Err(RuleError::NoMoves);
        }
        let tm = self.map.get_mut(u.x, u.y).unwrap();
        if tm.has(tile_flags::IRRIGATION) || tm.has(tile_flags::MINE) {
            tm.set(tile_flags::IRRIGATION | tile_flags::MINE, false);
        } else if tm.has(tile_flags::RAILROAD) {
            tm.set(tile_flags::RAILROAD, false);
        } else {
            tm.set(tile_flags::ROAD, false);
        }
        if let (Some(c), _) = self.nearest_city_exe(u.x, u.y) {
            let owner = self.city(c).owner;
            let human = self.human_slot;
            self.clear_diplo_flag(human, owner, super::diplomacy::flags::PEACE);
            // the owner of the nearest city wants the pillager attacked (0x4db2)
            if owner != u.owner {
                self.add_goal(owner, u.x, u.y, super::goals::goal::ATTACK, 4);
            }
        }
        let um = self.unit_mut(id);
        um.moves_left = 0;
        um.goto = None;
        if um.order.is_work() || um.order == UnitOrder::Pillage {
            um.order = UnitOrder::None;
        }
        Ok(())
    }

    /// The civ whose treaty with the human a pillage on the unit's square would end, for the
    /// question senate_check_attack (0x1a3b4) asks before 'P' goes ahead (0x4cea): the owner
    /// of the nearest city when the human has peace with it.
    pub fn pillage_treaty(&self, id: UnitId) -> Option<super::Slot> {
        let u = self.unit(id);
        let (c, _) = self.nearest_city_exe(u.x, u.y);
        let owner = self.city(c?).owner;
        (owner != self.human_slot && self.treaty(self.human_slot, owner)).then_some(owner)
    }

    /// Water for irrigation (0x77fd): the square is a River, or an orthogonal neighbour is
    /// Ocean, River or irrigated — city squares never count, irrigated or not.
    pub fn irrigation_water(&self, x: i32, y: i32) -> bool {
        if self.map.terrain(x, y) == Terrain::River {
            return true;
        }
        Map::neighbours4(x, y).any(|(nx, ny)| {
            let t = self.map.tile(nx, ny);
            t.city.is_none() && (t.is_ocean() || t.terrain == Terrain::River || t.has(tile_flags::IRRIGATION))
        })
    }

    /// The turn loop's part of Settlers work (at the start of the slot's turn, after the moves
    /// are restored): every working unit presses its order's key again (0x4492), a turn of
    /// work that may finish the job; a job no longer possible is dropped.
    pub fn progress_work(&mut self, slot: super::Slot) {
        let ids: Vec<UnitId> = self.all_units().filter(|u| u.owner == slot && u.order.is_work()).map(|u| u.id).collect();
        for id in ids {
            let o = self.unit(id).order;
            if self.work_step(id, o).is_err() {
                let um = self.unit_mut(id);
                um.order = UnitOrder::None;
                um.order_turns = 0;
            }
        }
    }

    /// 'b' (0x5753): found a city with a Settlers unit, or add it to the city on the square.
    /// A city of size 10 or more takes no Settlers, Aqueduct or not (*ADDCITY). A new city
    /// needs land in rows 2..47 and a free one of the 128 city records (*PMAX, o 0x1b375);
    /// nothing keeps it off the squares next to another city.
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
            if city.size >= 10 {
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
        if !(2..48).contains(&u.y) {
            return Err(RuleError::Other("Cities cannot be built this close to the poles."));
        }
        if self.all_cities().count() >= 128 {
            return Err(RuleError::Other("PMAX"));
        }
        let name = self.next_city_name(u.owner);
        let c = self.add_city(u.owner, name, u.x, u.y);
        // found_city o 0x1b375 drops the city-site goals within 4 (0x1b38c)
        self.remove_goals_near(u.owner, super::goals::goal::SETTLE, u.x, u.y, 4);
        self.remove_unit(id);
        self.refresh_vision(u.owner);
        // the computer asks its production advisor at once (found_city o 0x1b375)
        if !self.player(u.owner).human {
            let p = self.ai_production(c);
            self.city_mut(c).production = p;
        }
        self.push_event(super::GameEvent::CityFounded { city: c });
        Ok(c)
    }

    /// 'D' (0x4c5a): the unit is deleted (0x7e6:0xf0c, cargo and all); unlike in later
    /// games no shields go to a city.
    pub fn disband(&mut self, id: UnitId) {
        let u = self.unit(id).clone();
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

    /// Clear a unit's orders, as a click on it in the city screen does (CIV.EXE 0x122d3):
    /// a sentried or fortified unit also gets its full moves back.
    pub fn wake_unit(&mut self, id: UnitId) {
        let u = self.unit_mut(id);
        if matches!(u.order, UnitOrder::Sentry | UnitOrder::Fortified) {
            u.moves_left = u.full_moves();
        }
        u.order = UnitOrder::None;
        u.order_turns = 0;
        u.goto = None;
    }

    pub fn skip_unit(&mut self, id: UnitId) {
        self.unit_mut(id).order = UnitOrder::Skip;
        self.unit_mut(id).moves_left = 0;
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{Game, GameSetup, UnitOrder};
    use crate::rules::unit_ids;

    use crate::game::map::tile_flags;
    use crate::game::RuleError;
    use crate::rules::{advance_ids as adv, Terrain};

    /// A game with a Settlers unit of the human on a bare land square of terrain `t` (the
    /// neighbours made Plains), and a city a few squares away.
    fn settlers_on(t: Terrain) -> (Game, crate::game::UnitId) {
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        let slot = g.human_slot;
        let (x, y) = (30, 20);
        for dx in -3..=3 {
            for dy in -3..=3 {
                let tm = g.map.get_mut(x + dx, y + dy).unwrap();
                tm.terrain = if (dx, dy) == (0, 0) { t } else { Terrain::Plains };
                tm.flags = 0;
                tm.special = false;
                tm.city = None;
            }
        }
        let s = g.add_unit(slot, unit_ids::SETTLERS, x, y);
        (g, s)
    }

    /// Run the start-of-turn work step `n` times, restoring the moves first like the turn loop.
    fn turns(g: &mut Game, n: usize) {
        let slot = g.human_slot;
        for _ in 0..n {
            g.reset_moves(slot);
            g.progress_work(slot);
        }
    }

    #[test]
    fn road_work_counts_the_turn_of_the_order() {
        // CIV.EXE 0x5369: the key press itself is the first turn of work, so a road on
        // Grassland (2x move cost 1) is done on the next turn, on Hills (2x2) on the third
        let (mut g, s) = settlers_on(Terrain::Grassland);
        let (x, y) = g.unit(s).pos();
        g.give_order(s, UnitOrder::Road).unwrap();
        assert_eq!((g.unit(s).order, g.unit(s).order_turns, g.unit(s).moves_left), (UnitOrder::Road, 1, 0));
        turns(&mut g, 1);
        assert!(g.map.tile(x, y).has(tile_flags::ROAD));
        assert_eq!((g.unit(s).order, g.unit(s).order_turns), (UnitOrder::None, 0));

        let (mut g, s) = settlers_on(Terrain::Hills);
        g.give_order(s, UnitOrder::Road).unwrap();
        turns(&mut g, 2);
        assert!(!g.map.tile(30, 20).has(tile_flags::ROAD));
        turns(&mut g, 1);
        assert!(g.map.tile(30, 20).has(tile_flags::ROAD));
    }

    #[test]
    fn railroad_needs_bridge_building_on_rivers_and_no_city() {
        let (mut g, s) = settlers_on(Terrain::River);
        let slot = g.human_slot;
        g.player_mut(slot).learn(adv::RAILROAD);
        g.map.get_mut(30, 20).unwrap().set(tile_flags::ROAD, true);
        assert!(g.give_order(s, UnitOrder::Road).is_err());
        g.player_mut(slot).learn(adv::BRIDGE_BUILDING);
        g.give_order(s, UnitOrder::Road).unwrap();
        // 4x move cost 1
        turns(&mut g, 3);
        assert!(g.map.tile(30, 20).has(tile_flags::RAILROAD));
        // a city square takes no railroad (0x52cf)
        let (mut g, s) = settlers_on(Terrain::Plains);
        g.player_mut(slot).learn(adv::RAILROAD);
        let s2 = g.add_unit(slot, unit_ids::SETTLERS, 30, 20);
        g.found_city(s2).unwrap();
        assert!(g.map.tile(30, 20).has(tile_flags::ROAD));
        assert!(g.give_order(s, UnitOrder::Road).is_err());
        assert_eq!(g.unit(s).order, UnitOrder::None);
    }

    #[test]
    fn fortress_takes_move_cost_plus_four_turns() {
        let (mut g, s) = settlers_on(Terrain::Hills);
        let slot = g.human_slot;
        // 'f' on Settlers is the fortress, which needs Construction
        assert!(g.give_order(s, UnitOrder::Fortify).is_err());
        g.player_mut(slot).learn(adv::CONSTRUCTION);
        g.give_order(s, UnitOrder::Fortify).unwrap();
        assert_eq!(g.unit(s).order, UnitOrder::Fortress);
        turns(&mut g, 4);
        assert!(!g.map.tile(30, 20).has(tile_flags::FORTRESS));
        turns(&mut g, 1);
        assert!(g.map.tile(30, 20).has(tile_flags::FORTRESS));
    }

    #[test]
    fn work_counter_carries_over_to_another_job_and_a_move_resets_it() {
        // the counter (unit +5) is only zeroed by a finished or refused job and by a move
        let (mut g, s) = settlers_on(Terrain::Plains);
        g.map.get_mut(30, 19).unwrap().terrain = Terrain::River;
        g.give_order(s, UnitOrder::Mine).unwrap();
        turns(&mut g, 3);
        assert_eq!(g.unit(s).order_turns, 4);
        g.reset_moves(g.human_slot);
        // the 5th turn of work reaches the 5 of irrigation: done at once
        g.give_order(s, UnitOrder::Irrigate).unwrap();
        assert!(g.map.tile(30, 20).has(tile_flags::IRRIGATION));
        assert_eq!((g.unit(s).order, g.unit(s).order_turns), (UnitOrder::None, 0));
        g.give_order(s, UnitOrder::Mine).unwrap();
        g.place_unit(s, 31, 20, 0);
        assert_eq!((g.unit(s).order, g.unit(s).order_turns), (UnitOrder::None, 0));
    }

    #[test]
    fn irrigation_water_ignores_city_squares() {
        // 0x77fd skips city squares entirely, even irrigated ones
        let (mut g, s) = settlers_on(Terrain::Plains);
        let slot = g.human_slot;
        let s2 = g.add_unit(slot, unit_ids::SETTLERS, 31, 20);
        g.found_city(s2).unwrap();
        g.map.get_mut(31, 20).unwrap().set(tile_flags::IRRIGATION, true);
        assert_eq!(g.give_order(s, UnitOrder::Irrigate), Err(RuleError::NoWater));
        g.map.get_mut(30, 21).unwrap().set(tile_flags::IRRIGATION, true);
        g.give_order(s, UnitOrder::Irrigate).unwrap();
    }

    #[test]
    fn terraforming_keeps_the_square_s_special_pattern() {
        // Forest -> Plains: the special is a pattern of the square (0x1a68:0x183c)
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        let seed = g.map.seed;
        let (x, y) = (0..80).flat_map(|x| (2..48).map(move |y| (x, y))).find(|&(x, y)| crate::game::Map::special_at(seed, x, y)).unwrap();
        let tm = g.map.get_mut(x, y).unwrap();
        tm.terrain = Terrain::Forest;
        tm.special = true;
        tm.city = None;
        let s = g.add_unit(g.human_slot, unit_ids::SETTLERS, x, y);
        g.give_order(s, UnitOrder::Irrigate).unwrap();
        turns(&mut g, 4);
        let t = g.map.tile(x, y);
        assert_eq!(t.terrain, Terrain::Plains);
        assert!(t.special);
    }

    #[test]
    fn pillage_is_immediate_and_spares_the_fortress() {
        let (mut g, s) = settlers_on(Terrain::Grassland);
        let slot = g.human_slot;
        let tm = g.map.get_mut(30, 20).unwrap();
        tm.set(tile_flags::ROAD | tile_flags::RAILROAD | tile_flags::IRRIGATION | tile_flags::MINE | tile_flags::FORTRESS, true);
        // Settlers may pillage (0x4c80 bars only types >= Diplomat and the Fighter)
        g.give_order(s, UnitOrder::Pillage).unwrap();
        let t = g.map.tile(30, 20);
        assert!(!t.has(tile_flags::IRRIGATION) && !t.has(tile_flags::MINE) && t.has(tile_flags::RAILROAD));
        assert_eq!(g.unit(s).moves_left, 0);
        assert_eq!(g.give_order(s, UnitOrder::Pillage), Err(RuleError::NoMoves));
        let m = g.add_unit(slot, unit_ids::MILITIA, 30, 20);
        g.give_order(m, UnitOrder::Pillage).unwrap();
        let m2 = g.add_unit(slot, unit_ids::MILITIA, 30, 20);
        g.give_order(m2, UnitOrder::Pillage).unwrap();
        let t = g.map.tile(30, 20);
        assert!(!t.has(tile_flags::RAILROAD) && !t.has(tile_flags::ROAD) && t.has(tile_flags::FORTRESS));
        let d = g.add_unit(slot, unit_ids::DIPLOMAT, 30, 20);
        assert_eq!(g.give_order(d, UnitOrder::Pillage), Err(RuleError::Pillage));
    }

    #[test]
    fn add_to_city_stops_at_ten_and_cities_may_touch() {
        let (mut g, s) = settlers_on(Terrain::Plains);
        let slot = g.human_slot;
        let c = g.found_city(s).unwrap();
        // a city may be founded next to another one (0x5753, o 0x1b375 check no distance)
        let s2 = g.add_unit(slot, unit_ids::SETTLERS, 31, 20);
        g.found_city(s2).unwrap();
        g.city_mut(c).size = 10;
        g.city_mut(c).add_improvement(crate::rules::improvement_ids::AQUEDUCT);
        let s3 = g.add_unit(slot, unit_ids::SETTLERS, 30, 20);
        assert_eq!(g.found_city(s3), Err(RuleError::AddCity));
    }

    #[test]
    fn only_land_units_fortify_and_orders_end_the_turn() {
        let (mut g, s) = settlers_on(Terrain::Plains);
        let slot = g.human_slot;
        let m = g.add_unit(slot, unit_ids::MILITIA, 30, 20);
        g.give_order(m, UnitOrder::Fortify).unwrap();
        assert_eq!(g.unit(m).moves_left, 0);
        let ship = g.add_unit(slot, unit_ids::TRIREME, 30, 20);
        assert!(g.give_order(ship, UnitOrder::Fortify).is_err());
        let _ = s;
    }

    #[test]
    fn wake_unit_restores_moves_of_sentried_and_fortified() {
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        let slot = g.human_slot;
        let s = g.units_of(slot)[0];
        let (x, y) = (g.unit(s).x, g.unit(s).y);
        for (order, moves) in [(UnitOrder::Fortified, 3), (UnitOrder::Sentry, 3), (UnitOrder::Fortify, 0)] {
            let u = g.add_unit(slot, unit_ids::MILITIA, x, y);
            g.unit_mut(u).order = order;
            g.unit_mut(u).moves_left = 0;
            g.wake_unit(u);
            assert_eq!(g.unit(u).order, UnitOrder::None);
            assert_eq!(g.unit(u).moves_left, moves);
        }
    }
}
