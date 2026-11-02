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
use super::{AutoStop, Game, RuleError};
use crate::rules::{self, unit_ids, Terraform, Terrain};

impl UnitOrder {
    /// Settlers jobs, which the unit loop repeats until they are done (`run_auto_unit`).
    pub fn is_work(self) -> bool {
        matches!(self, UnitOrder::Road | UnitOrder::Irrigate | UnitOrder::Mine | UnitOrder::Fortress | UnitOrder::CleanPollution)
    }

    /// The status bits of a job (unit +0: 0x02 road, 0x40 irrigation, 0x80 mine, 0xc0
    /// fortress, 0x82 pollution).
    pub(crate) fn job_bits(self) -> u8 {
        match self {
            UnitOrder::Road => 0x02,
            UnitOrder::Irrigate => 0x40,
            UnitOrder::Mine => 0x80,
            UnitOrder::Fortress => 0xc0,
            UnitOrder::CleanPollution => 0x82,
            _ => 0,
        }
    }

    /// The order left when job `refused` is refused: a refusal clears that job's bits alone
    /// ('p' 0x508c, 'f' 0x51f3, 'r' 0x52eb, 'i' 0x5454, 'm' 0x574b), so a computer's
    /// Settlers asked for another job keeps what is left of its own.
    pub fn after_refusal(self, refused: UnitOrder) -> UnitOrder {
        if !self.is_work() {
            return self;
        }
        UnitOrder::of_job_bits(self.job_bits() & !refused.job_bits())
    }

    /// The job that status bits & 0xc2 stand for, as a save is read (0x42, irrigation with a
    /// road, stands for none).
    pub(crate) fn of_job_bits(b: u8) -> UnitOrder {
        match b & 0xc2 {
            0x02 => UnitOrder::Road,
            0x40 => UnitOrder::Irrigate,
            0x80 => UnitOrder::Mine,
            0xc0 => UnitOrder::Fortress,
            0x82 | 0xc2 => UnitOrder::CleanPollution,
            _ => UnitOrder::None,
        }
    }
}

/// The special resource of a square whose terrain changed: specials are a pattern of the
/// square (0x1a68:0x183c, rows 2..47), whatever the terrain; Grassland and River squares have
/// none of their own (their resource is the shield pattern).
pub(super) fn positional_special(seed: u32, x: i32, y: i32, t: Terrain) -> bool {
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
            // 's' (0x4ddf) sets the sentry bit and ends the unit's turn (moves = 0); 'u' gives
            // the moves back (`unload`)
            UnitOrder::Sentry => {
                let um = self.unit_mut(id);
                // `or status,1`: the other bits stay under it
                um.sentry_keeping_job(true);
                um.goto = None;
                um.moves_left = 0;
                Ok(())
            }
            UnitOrder::None | UnitOrder::Skip => {
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
                    self.refuse_job(id, o);
                }
                r
            }
        }
    }

    /// One turn of Settlers work on order `o`: the key handlers 'r' 0x5288, 'i' 0x53ca,
    /// 'm' 0x559a, 'f' 0x51be and 'p' 0x5048. Checks the job, spends the unit's moves, counts
    /// the turn (unit +5) and finishes the job when the count reaches its time.
    ///
    /// Not copied: the handlers OR their bit into the status without clearing another job's
    /// ('r' 0x532f, 'i' 0x54d5, 'm' 0x5670, 'f' 0x520b, 'p' 0x50ba), so a computer's Settlers
    /// whose unit routine (0x17748, which never reads the status) switches jobs merges them,
    /// e.g. 'm' while irrigating gives 0xc0 = fortress. The human's never can (his loop asks
    /// for keys only with no job bits, 0x44ac, and presses the job's own key); civrs's new job
    /// replaces the old one, the counter going on as in CIV.EXE, but for `exe_quirks`, which
    /// keeps the merged bits (`Game::exe_job_bits`): the job done clears its own and leaves
    /// the rest.
    /// A refused job `o` clears its bits and the work counter (0x5091).
    fn refuse_job(&mut self, id: UnitId, o: UnitOrder) {
        let bits = self.exe_job_bits.remove(&id);
        let um = self.unit_mut(id);
        um.order = match bits {
            Some(b) => UnitOrder::of_job_bits(b & !o.job_bits()),
            None => um.order.after_refusal(o),
        };
        um.order_turns = 0;
        if let Some(b) = bits {
            let left = b & !o.job_bits();
            if left & 0xc2 != um.order.job_bits() {
                self.exe_job_bits.insert(id, left);
            }
        }
    }

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
                // (civrs refuses roads and fortresses at sea; CIV.EXE only reads the flags
                // and the River, 0x52b6..0x5304, so a human's Settlers woken aboard with a
                // road job kept under its sentry bit, whose loop presses 'r' for it (0x44bd),
                // lays a road on the ocean (+0x28e ocean cost 1: 2 turns). Not copied: the
                // job is refused as at 0x52eb, the unit waits for orders with its moves)
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
        // `exe_job_bits`: the key ORs its bits into those of the job before
        let bits = self.exe_quirks.then(|| self.exe_job_bits.get(&id).copied().unwrap_or(u.order.job_bits()) | o.job_bits());
        let um = self.unit_mut(id);
        um.order = o;
        um.moves_left = 0;
        um.goto = None;
        um.order_turns = um.order_turns.saturating_add(1);
        let done = um.order_turns >= turns;
        if done {
            um.order = UnitOrder::None;
            um.order_turns = 0;
        }
        self.exe_job_bits.remove(&id);
        if let Some(b) = bits {
            // the finished job clears its own bits (0x512a, 0x5280, 0x53c2, 0x5592, 0x574b)
            let b = if done { b & !o.job_bits() } else { b };
            if done {
                self.unit_mut(id).order = UnitOrder::of_job_bits(b);
            }
            if b != self.unit(id).order.job_bits() {
                self.exe_job_bits.insert(id, b);
            }
        }
        if !done {
            return Ok(());
        }
        self.finish_work(x, y, o);
        // add_improvement (0x1bcd9) shows the human his own work at once; cleaning pollution
        // shows anyone's (0x5101); a new terrain's lost irrigation and mine wait for a look
        let terraform = matches!(o, UnitOrder::Irrigate | UnitOrder::Mine) && self.map.tile(x, y).terrain != t.terrain;
        if o == UnitOrder::CleanPollution || (u.owner == self.human_slot && !terraform) {
            self.map.update_seen(x, y);
        }
        // a finished road, railroad or fortress has the unit look around again (unit_look
        // 0x53aa, 0x5268: meetings and goals as after a move)
        if matches!(o, UnitOrder::Road | UnitOrder::Fortress) {
            self.unit_look(id);
        }
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
            // the map shows the loss when the nearest city is the human's (0x4d58)
            if owner == human {
                self.map.update_seen(u.x, u.y);
            }
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

    /// Settlers work for all of `slot`'s working units at once: each presses its order's key
    /// again (0x4492), a turn of work that may finish the job; a job no longer possible is
    /// dropped. In the game the human's Settlers do this as his loop comes to them
    /// (`run_auto_unit`), and the computer's working units ask the unit routine instead
    /// (`ai::unit_ai::move_units`); this form serves the tests.
    pub fn progress_work(&mut self, slot: super::Slot) {
        let ids: Vec<UnitId> = self.all_units().filter(|u| u.owner == slot && u.order.is_work()).map(|u| u.id).collect();
        for id in ids {
            let o = self.unit(id).order;
            if self.work_step(id, o).is_err() {
                self.refuse_job(id, o);
            }
        }
    }

    /// 'b' (0x5753): found a city with a Settlers unit, or add it to the city on the square
    /// (one more citizen, placed only by the city's next run of the city routine).
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
            // the new citizen gets no square or specialist here (0x57cc..0x57fb: size + 1,
            // the unit deleted): the city's next placement run takes it in
            self.city_mut(c).size += 1;
            self.remove_unit(id);
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
        // the computer asks its production advisor at once (found_city o 0x1b375), while the
        // Settlers still stand in the new city: 'b' deletes the unit only after found_city
        // returns (0x5829 -> 0x57e9), so the advisor counts them among the civ's units and
        // the city is not empty
        if !self.player(u.owner).human {
            let p = self.ai_production(c);
            self.city_mut(c).production = p;
        }
        self.remove_unit(id);
        // the human's new city opens its screen (process_city_and_redraw, o 0x1b7ff), whose
        // city routine stores its trade (0xf13d)
        if self.player(u.owner).human {
            self.note_trade(c);
        }
        self.refresh_vision(u.owner);
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

    /// One press of 'h' (CIV.EXE 0x4f28) by the human, by hand or by the unit loop for an
    /// aircraft in return-to-base mode (0x4528..0x4556 press it again on every move): a unit
    /// on the square of the nearest city (0x1d43:0xf8, any owner) makes it its home; an
    /// aircraft elsewhere gets bit 0x02 (`UnitOrder::Home`, 0x4f94) and the direction to its
    /// nearest base (`dir_to_base`), to be moved as a direction key moves it (0x4f99); with
    /// no base the bit goes (0x5040) and the unit stays as it is. Any other unit, nothing (no
    /// message). The computer's 'h' is `Command::Home`.
    pub fn press_home(&mut self, id: UnitId) -> Option<super::Dir8> {
        let u = self.unit(id).clone();
        let (near, d) = self.nearest_city_exe(u.x, u.y);
        if d == 0 {
            self.unit_mut(id).home = near;
            return None;
        }
        if !u.is_air() {
            return None;
        }
        let k = self.dir_to_base(id);
        let um = self.unit_mut(id);
        if k == 0 {
            if um.order == UnitOrder::Home {
                um.order = UnitOrder::None;
            }
            return None;
        }
        um.order = UnitOrder::Home;
        Some(super::Dir8::ALL[k - 1])
    }

    /// The human's unit `id` that moves or works by itself (`is_auto_unit`), as his loop
    /// does when it comes to it (0x4492..0x4556 -> 0x4bbe) and again while the unit has moves
    /// left after a key (0x6d7f -> 0x442c):
    ///
    /// - a Settlers at work presses its job's key ('r' 0x02, 'i' 0x40, 'm' 0x80, 'f' 0xc0,
    ///   'p' 0x82; 0x44bd..0x452f): a turn of work (`work_step`), or, when the job is no
    ///   longer possible, the order goes (`AutoStop::Refused`) and the unit, its moves
    ///   kept, waits for orders at once;
    /// - an aircraft returning to base presses 'h' before every step (0x4528..0x4556,
    ///   `press_home`);
    /// - a goto (which goes before both, 0x4bce) takes its next step (goto_direction
    ///   0x1db0:0x4). It ends at its square, when no step is left or a step is refused (the
    ///   zone of control clears it, 0x5b44); a step that needs the player's answer (a caravan
    ///   at one of his cities, a Diplomat at a foreign city or unit) ends it too and comes
    ///   back as `AutoStop::Menu`; so does a treaty partner's land or unit ("We have signed
    ///   a peace treaty", `MoveOutcome::BreakTreaty`), whose land has dropped the goto before
    ///   the question (0x656d) and taken the step's cost (0x62b1), so after either answer
    ///   the unit waits for orders with the moves it has left (0x6d7f -> 0x442c), one step
    ///   further after "Break treaty.". A land unit whose step fails for want of moves (0x636f,
    ///   before the territory rule) keeps its goto for the next turn and is not asked. A job kept beside the goto (a Settlers woken to
    ///   both from under its sentry bit, or an imported save) rides along, the human's step
    ///   keeping the bits (0x6810 skips him), and once the goto is over the moves left go to
    ///   the job's key.
    ///
    /// CIV.EXE presses the key again for as long as the unit keeps its order and moves; when
    /// a step neither moves the unit nor spends or clears anything (an aircraft standing on a
    /// city with its return bit, a refused flight) that never ends. civrs stops such a unit
    /// instead: on a city's square its order goes, elsewhere its turn ends.
    pub fn run_auto_unit(&mut self, id: UnitId) -> AutoStop {
        use super::MoveOutcome as M;
        if let Some(o) = self.unit_opt(id).filter(|u| u.goto.is_none() && u.moves_left > 0).map(|u| u.order).filter(|o| o.is_work()) {
            return match self.work_step(id, o) {
                Ok(()) => AutoStop::Done,
                Err(e) => {
                    // a refused job drops the order bits and the counter (0x5091, 0x543f)
                    let um = self.unit_mut(id);
                    um.order = UnitOrder::None;
                    um.order_turns = 0;
                    AutoStop::Refused(e)
                }
            };
        }
        let mut stop = AutoStop::Done;
        for _ in 0..40 {
            let Some(u) = self.unit_opt(id) else { break };
            // a job kept beside a goto waits for its end (0x4bce goes before the key of 0x44bd)
            let job_on_goto = u.order.is_work() && u.goto.is_some();
            if !(matches!(u.order, UnitOrder::Goto | UnitOrder::Home) || job_on_goto) || u.moves_left == 0 {
                break;
            }
            let dest = u.goto;
            if u.order == UnitOrder::Home {
                let Some(dir) = self.press_home(id) else { break };
                if !matches!(self.try_move(id, dir), Ok(M::Moved) | Ok(M::EnteredHut)) {
                    break;
                }
                self.view_follows(id);
                continue;
            }
            let Some(dir) = self.next_goto_step(id) else {
                self.clear_goto(id);
                break;
            };
            match self.try_move(id, dir) {
                Ok(M::Moved) | Ok(M::Boarded(_)) => {
                    self.view_follows(id);
                    if self.unit_opt(id).is_some_and(|u| Some(u.pos()) == dest) {
                        self.clear_goto(id);
                    }
                }
                Ok(M::EnteredHut) => self.view_follows(id),
                // the move routine asks a Diplomat on goto as it asks one moved by hand, and
                // cancels the goto (0x5c15 into o 0x1d575, 0x5dc1)
                Ok(out @ (M::CaravanArrived(_) | M::EnteredForeignCity(_) | M::BribeOffer(_))) => {
                    self.clear_goto(id);
                    stop = AutoStop::Menu(out, dir);
                    break;
                }
                // "We have signed a peace treaty" (senate_check_attack 0x1a3b4): the human is
                // asked as for a move by hand; the territory rule has dropped the goto (0x656d),
                // a fight drops it on "Cancel action." (0x19a1a)
                Ok(out @ M::BreakTreaty(_)) => {
                    stop = AutoStop::Menu(out, dir);
                    break;
                }
                // a land unit short of moves fails its step with its moves gone (0x636f) and
                // keeps the goto
                Ok(M::Blocked) if self.unit_opt(id).is_some_and(|u| u.moves_left == 0) => break,
                Ok(_) | Err(_) => {
                    if self.unit_opt(id).is_some() {
                        self.clear_goto(id);
                    }
                    break;
                }
            }
        }
        // the goto is over and moves are left: the loop comes back to the unit (0x6d7f ->
        // 0x442c) and presses its job's key (0x44bd)
        if !matches!(stop, AutoStop::Menu(..)) && self.unit_opt(id).is_some_and(|u| u.goto.is_none() && u.moves_left > 0 && u.order.is_work()) {
            return self.run_auto_unit(id);
        }
        // what CIV.EXE would press again without end (above)
        if let Some(u) = self.unit_opt(id).filter(|u| matches!(u.order, UnitOrder::Goto | UnitOrder::Home) && u.moves_left > 0 && !matches!(stop, AutoStop::Menu(..))) {
            if self.city_at(u.x, u.y).is_some() {
                self.clear_goto(id);
                self.unit_mut(id).order = UnitOrder::None;
            } else {
                self.unit_mut(id).moves_left = 0;
            }
        }
        stop
    }

    /// The human's view keeps his moving unit in sight (`scroll_to` 0x9505 on the square it
    /// moves to, 0x6761).
    fn view_follows(&mut self, id: UnitId) {
        if let (Some(u), Some(view)) = (self.unit_opt(id), self.view) {
            self.view = Some(Self::view_scrolled_to(view, u.x, u.y));
        }
    }

    /// `dir_to_nearest_base` 0xa0c9: the direction (1..8, `goto::DX`/`DY`) that brings an
    /// aircraft nearest (2 x max + min, wrapped) to the nearest own city or, if nearer, own
    /// Carrier; 0 for none.
    pub fn dir_to_base(&self, id: UnitId) -> usize {
        use super::goto::{DX, DY};
        let u = self.unit(id);
        let cheb = |a: (i32, i32), b: (i32, i32)| Map::delta_x(a.0, b.0).abs().max((a.1 - b.1).abs());
        let mut base: Option<((i32, i32), i32)> = None;
        for p in self.all_cities().filter(|c| c.owner == u.owner).map(|c| c.pos()).chain(self.all_units().filter(|o| o.owner == u.owner && o.kind == unit_ids::CARRIER).map(|o| o.pos())) {
            let d = cheb(u.pos(), p);
            if base.map(|b| d < b.1).unwrap_or(true) {
                base = Some((p, d));
            }
        }
        let Some((b, _)) = base else { return 0 };
        let metric2 = |dx: i32, dy: i32| {
            let (a, c) = (Map::delta_x(0, dx).abs(), dy.abs());
            if a > c {
                2 * a + c
            } else {
                2 * c + a
            }
        };
        let mut best = metric2(u.x - b.0, u.y - b.1);
        let mut pick = 0;
        for k in 1..=8 {
            let (nx, ny) = (Map::wrap_x(u.x + DX[k]), u.y + DY[k]);
            if !Map::in_bounds(nx, ny) {
                continue;
            }
            if self.unit_owner_at(nx, ny).map(|o| o != u.owner).unwrap_or(false) {
                continue;
            }
            if self.city_at(nx, ny).is_some() && self.last_occupant(nx, ny) != u.owner {
                continue;
            }
            let d = metric2(nx - b.0, ny - b.1);
            if d < best {
                best = d;
                pick = k;
            }
        }
        pick
    }

    /// 'u' (CIV.EXE 0x4dfc), for the human and the computer alike: a ship that carries units
    /// (a Carrier included) and does not stand alone wakes the sentried units of its stack
    /// (`for_each_in_stack` 0x92a7 with `wake_cb` 0x9352): the sentry bit goes, the moves are
    /// full again (3 x the type's moves) and aircraft get their fuel back; every unit the walk
    /// visits, the ship and awake ones too, forgets its goto and heading (0x939e). Returns the unit the order loop goes on with
    /// (`next_unit_in_stack` 0x7d8f): of the stack's units but ships, the first after the
    /// ship in record order (round the table), else the ship itself; `None` when the key does
    /// nothing. Not copied: CIV.EXE then runs the end-of-unit fuel and Trireme code on the
    /// record before that unit, which may be any unit (0x4e71 -> 0x6dad -> 0x750f).
    pub fn unload(&mut self, id: UnitId) -> Option<UnitId> {
        if !self.unit(id).carries_units() {
            return None;
        }
        let stack = self.stack_from(id);
        if stack.len() < 2 {
            return None;
        }
        for &c in &stack {
            // +6 and +8 = 0xff before the bit goes, so no kept goto comes back
            self.clear_goto(c);
            self.set_heading(c, 0xff);
            if self.unit(c).order != UnitOrder::Sentry {
                continue;
            }
            let cu = self.unit_mut(c);
            // `and status,0xfe` (0x936e): a job kept under the sentry bit comes back
            cu.wake_sentry();
            cu.moves_left = cu.full_moves();
            if cu.info().range > 0 {
                cu.fuel = 0;
            }
        }
        let rec = self.unit(id).record as i32;
        // 0x7d8f follows the ring for 16 units at most
        let next = self
            .stack_ring(id, 17)
            .iter()
            .skip(1)
            .filter(|o| !self.unit(**o).is_sea())
            .map(|o| (((self.unit(*o).record as i32 - rec) + 0x80) % 0x80, *o))
            .min_by_key(|(d, _)| *d)
            .map(|(_, o)| o);
        Some(next.unwrap_or(id))
    }

    /// 'w' (0x4dd2): the unit keeps its moves and the loop goes on to the next unit in record
    /// order, without preferring the units in the map view.
    pub fn wait_unit(&mut self, id: UnitId) {
        self.active_unit = Some(id);
        self.active_unit = self.next_active_unit_after_wait();
    }

    /// Clear a unit's orders, as a click on it in the city screen does (CIV.EXE 0x122d3):
    /// a sentried or fortified unit also gets its full moves back (flags & 9, 0x122ea), then
    /// every order but the veteran bits is cleared (flags &= 0x30, 0x12335, a job kept under
    /// the sentry bit too) and so is the goto. The work counter (+5) stays.
    pub fn wake_unit(&mut self, id: UnitId) {
        let u = self.unit_mut(id);
        if matches!(u.order, UnitOrder::Sentry | UnitOrder::Fortified) {
            u.moves_left = u.full_moves();
        }
        u.order = UnitOrder::None;
        u.sentry_job = None;
        u.goto = None;
    }

    /// Wake a unit clicked in the map's stack box (`pick_unit_from_stack`, CIV.EXE 0xa075): a
    /// Settlers job (flags 0xc2) gives back the full moves, then every order but the veteran
    /// bits is cleared (flags &= 0x30) and so is the goto. The work counter (+5) stays
    /// (0xa075..0xa0a4 do not write it): the Settlers' next job goes on counting from it.
    pub fn wake_picked_unit(&mut self, id: UnitId) {
        let u = self.unit_mut(id);
        // (a job under the sentry bit too)
        if u.order_under_sentry().is_work() {
            u.moves_left = u.full_moves();
        }
        u.order = UnitOrder::None;
        u.sentry_job = None;
        u.goto = None;
        self.pick_unit(id);
    }

    /// The unit taken from the map's stack box becomes the loop's unit (0x4b29..0x4b36): the
    /// loop goes on from it, it is the new mark, and it waits for orders (`unit_waits`).
    pub fn pick_unit(&mut self, id: UnitId) {
        let rec = self.unit(id).record & 127;
        self.unit_loop.last = rec;
        self.unit_loop.mark = rec.wrapping_sub(1) & 127;
        self.unit_loop.round = false;
        self.unit_waits(id);
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

    /// Settlers joining a city add a citizen and nothing else (0x57cc): no square, no
    /// specialist until the city's next placement run.
    #[test]
    fn settlers_joining_a_city_leave_its_squares_alone() {
        let (mut g, s) = settlers_on(Terrain::Grassland);
        let slot = g.human_slot;
        let c = g.add_city(slot, "Join".into(), 30, 20);
        g.city_mut(c).size = 3;
        g.auto_assign(c);
        let (worked, specialists) = (g.city(c).worked, g.city(c).specialists.clone());
        assert_eq!(g.found_city(s), Ok(c));
        assert_eq!(g.city(c).size, 4);
        assert_eq!((g.city(c).worked, &g.city(c).specialists), (worked, &specialists));
        assert!(g.unit_opt(s).is_none());
    }

    #[test]
    fn a_refused_job_clears_its_own_bits_alone() {
        // 'r' 0x52eb: and status,0xfd; 'i' 0x5454: and status,0xbf
        assert_eq!(UnitOrder::Irrigate.after_refusal(UnitOrder::Road), UnitOrder::Irrigate);
        assert_eq!(UnitOrder::Fortress.after_refusal(UnitOrder::Irrigate), UnitOrder::Mine);
        assert_eq!(UnitOrder::Irrigate.after_refusal(UnitOrder::Irrigate), UnitOrder::None);
        let (mut g, s) = settlers_on(Terrain::Grassland);
        g.map.get_mut(30, 20).unwrap().set(tile_flags::ROAD, true);
        g.unit_mut(s).order = UnitOrder::Irrigate;
        g.unit_mut(s).order_turns = 2;
        assert!(g.give_order(s, UnitOrder::Road).is_err());
        assert_eq!((g.unit(s).order, g.unit(s).order_turns), (UnitOrder::Irrigate, 0));
    }

    #[test]
    fn exe_quirks_a_goal_set_on_merged_job_bits_keeps_the_high_ones() {
        // ai_set_goto's `and status,0xf0` (0x1641b): irrigation 0x40 with road 0x02 -> 0x40
        let (mut g, s) = settlers_on(Terrain::Grassland);
        g.exe_quirks = true;
        g.exe_job_bits.insert(s, 0x42);
        g.unit_mut(s).order = UnitOrder::None;
        g.ai_set_goto(s, (33, 20));
        assert_eq!((g.unit(s).order, g.unit(s).goto), (UnitOrder::Irrigate, Some((33, 20))));
        assert!(!g.exe_job_bits.contains_key(&s));
    }

    #[test]
    fn exe_quirks_a_finished_job_leaves_the_bits_of_the_one_before() {
        // the keys OR their bits in (0x532f); the road done clears 0x02 alone (0x53c2)
        let (mut g, s) = settlers_on(Terrain::Grassland);
        g.exe_quirks = true;
        g.unit_mut(s).order = UnitOrder::Irrigate;
        g.unit_mut(s).order_turns = 3;
        assert!(g.give_order(s, UnitOrder::Road).is_ok());
        assert!(g.map.tile(30, 20).has(tile_flags::ROAD));
        assert_eq!((g.unit(s).order, g.unit(s).order_turns), (UnitOrder::Irrigate, 0));
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
    fn a_unit_woken_in_the_stack_box_gets_its_moves_back_from_a_job() {
        let mut g = Game::new(GameSetup { seed: 3, ..Default::default() }, None);
        let id = g.units_of(g.human_slot)[0];
        let u = g.unit_mut(id);
        u.order = UnitOrder::Road;
        u.moves_left = 0;
        u.order_turns = 2;
        g.wake_picked_unit(id);
        assert_eq!(g.unit(id).order, UnitOrder::None);
        assert_eq!(g.unit(id).moves_left, g.unit(id).full_moves());
        // the work counter stays (0xa09f leaves +5)
        assert_eq!(g.unit(id).order_turns, 2);
        // a sentry keeps its moves as they are (0xa075 tests flags 0xc2 only)
        let u = g.unit_mut(id);
        u.order = UnitOrder::Sentry;
        u.moves_left = 0;
        g.wake_picked_unit(id);
        assert_eq!(g.unit(id).moves_left, 0);
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
    fn a_finished_road_has_the_settlers_look_around() {
        // unit_look 0x53aa after the road: a foreign unit that came next to the working
        // Settlers is met then, not only at the next move
        let (mut g, _) = settlers_on(Terrain::Grassland);
        let h = g.human_slot;
        let (a, b) = if h == 2 { (3, 4) } else if h == 3 { (2, 4) } else { (2, 3) };
        let s = g.add_unit(a, unit_ids::SETTLERS, 31, 21);
        g.give_order(s, UnitOrder::Road).unwrap();
        g.add_unit(b, unit_ids::MILITIA, 32, 21);
        assert_eq!(g.diplo(a, b) & crate::game::diplomacy::flags::CONTACT, 0);
        g.reset_moves(a);
        g.progress_work(a);
        assert!(g.map.tile(31, 21).has(tile_flags::ROAD));
        assert_ne!(g.diplo(a, b) & crate::game::diplomacy::flags::CONTACT, 0, "the two met");
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
    fn work_counter_carries_over_to_another_job_and_a_move() {
        // the counter (unit +5) is only zeroed by a finished or refused job; a move keeps it
        // (only an aircraft's landing writes it, 0x6a0e / 0x6d00; CIV.EXE's seed-13 run has
        // a computer's Settlers keep 4 across its goto steps)
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
        let count = g.unit(s).order_turns;
        assert!(count > 0);
        // (the human's step keeps the job bits, 0x6810 being the computer's)
        g.place_unit(s, 31, 20, 0);
        assert_eq!((g.unit(s).order, g.unit(s).order_turns), (UnitOrder::Mine, count));
    }

    #[test]
    fn h_sends_the_human_s_aircraft_home_until_it_lands() {
        // 0x4f28: no message; a unit in a city is rehomed, an aircraft elsewhere gets bit 0x02
        // and steps toward its base, the unit loop pressing 'h' again until it lands (0x6c96)
        let (mut g, s) = settlers_on(Terrain::Plains);
        let slot = g.human_slot;
        let f = g.add_unit(slot, unit_ids::FIGHTER, 33, 20);
        // with no base the human's bit is just cleared (0x5040)
        g.unit_mut(f).order = UnitOrder::Home;
        assert_eq!(g.press_home(f), None);
        assert_eq!((g.unit(f).order, g.unit(f).pos()), (UnitOrder::None, (33, 20)));
        let c = g.found_city(s).unwrap();
        let m = g.add_unit(slot, unit_ids::MILITIA, 31, 20);
        assert_eq!(g.press_home(m), None);
        assert_eq!(g.unit(m).order, UnitOrder::None);
        assert!(g.press_home(f).is_some());
        assert_eq!(g.unit(f).order, UnitOrder::Home);
        assert_eq!(g.run_auto_unit(f), crate::game::AutoStop::Done);
        assert_eq!((g.unit(f).pos(), g.unit(f).order, g.unit(f).moves_left), ((30, 20), UnitOrder::None, 0));
        g.unit_mut(f).home = None;
        g.press_home(f);
        assert_eq!(g.unit(f).home, Some(c));
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
    fn sentry_ends_the_turn_and_unload_wakes_the_stack_with_full_moves() {
        // 's' 0x4ddf zeroes the moves; 'u' 0x4dfc / wake_cb 0x9352 gives them back to every
        // sentried unit of the ship's stack and goes on with the passenger after the ship
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        let slot = g.human_slot;
        let (x, y) = (30, 20);
        g.map.get_mut(x, y).unwrap().terrain = Terrain::Ocean;
        for u in g.units_of(slot) {
            g.unit_mut(u).moves_left = 0;
        }
        let a = g.add_unit(slot, unit_ids::MILITIA, x, y);
        let ship = g.add_unit(slot, unit_ids::SAIL, x, y);
        let b = g.add_unit(slot, unit_ids::MILITIA, x, y);
        assert!(g.unit(a).record < g.unit(ship).record && g.unit(ship).record < g.unit(b).record);
        for p in [a, b] {
            g.give_order(p, UnitOrder::Sentry).unwrap();
            assert_eq!((g.unit(p).order, g.unit(p).moves_left), (UnitOrder::Sentry, 0));
        }
        assert_eq!(g.unload(ship), Some(b));
        for p in [a, b] {
            assert_eq!((g.unit(p).order, g.unit(p).moves_left), (UnitOrder::None, 3));
        }
        // the passengers at sea want orders: the loop goes on from b, then a
        g.active_unit = Some(ship);
        g.unit_mut(ship).moves_left = 0;
        assert_eq!(g.next_active_unit_at(b), Some(b));
        g.unit_mut(b).moves_left = 0;
        assert_eq!(g.next_active_unit_at(b), Some(a));
        // a ship alone and a unit that carries nobody do nothing
        let lone = g.add_unit(slot, unit_ids::SAIL, x + 2, y);
        assert_eq!(g.unload(lone), None);
        assert_eq!(g.unload(a), None);
    }

    #[test]
    fn wait_goes_on_to_the_next_unit_in_or_out_of_the_view() {
        // 'w' 0x4dd7 sets the loop's round-done flag: the next unit in record order is taken
        // even outside the map view, which then centres on it
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        let slot = g.human_slot;
        for u in g.units_of(slot) {
            g.unit_mut(u).moves_left = 0;
        }
        let a = g.add_unit(slot, unit_ids::MILITIA, 30, 20);
        let far = g.add_unit(slot, unit_ids::MILITIA, 60, 40);
        let near = g.add_unit(slot, unit_ids::MILITIA, 31, 20);
        g.view = Some((25, 15));
        g.active_unit = Some(a);
        assert_eq!(g.next_active_unit(), Some(near));
        g.view = Some((25, 15));
        g.wait_unit(a);
        assert_eq!(g.active_unit, Some(far));
        assert_eq!(g.view, Some(Game::view_centred_on(60, 40)));
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
        // a Settlers sentried with its job kept loses the job but keeps its work counter
        let u = g.add_unit(slot, unit_ids::SETTLERS, x, y);
        g.unit_mut(u).order = UnitOrder::Sentry;
        g.unit_mut(u).sentry_job = Some(UnitOrder::Road);
        g.unit_mut(u).order_turns = 2;
        g.wake_unit(u);
        assert_eq!((g.unit(u).order, g.unit(u).sentry_job, g.unit(u).order_turns), (UnitOrder::None, None, 2));
    }
}
