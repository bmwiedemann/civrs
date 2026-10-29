//! Unit movement: terrain costs, zones of control, boarding ships, triggering combat.

use super::map::{tile_flags, Dir8, Map, Slot, UnitId};
use super::unit::UnitOrder;
use super::{Game, MoveOutcome, RuleError};
use crate::rules::{self, unit_ids, Domain};

impl Game {
    /// Movement cost in thirds for `unit` moving from (fx,fy) to (tx,ty), ignoring legality.
    ///
    /// As move_units has it (CIV.EXE 0x62dd..0x6334): a road on both squares (a city square
    /// has one, found_city 0x1b375) costs 1; it is free when the square left has a railroad
    /// and the unit is the human's and not on a goto. Only the square left needs the railroad
    /// (so rail into a city or onto a road is free, out of a city never, as cities get no
    /// railroad); computer units and goto moves pay the road's 1 on railroads.
    pub fn move_cost(&self, unit: &super::Unit, from: (i32, i32), to: (i32, i32)) -> u8 {
        let ft = self.map.tile(from.0, from.1);
        let tt = self.map.tile(to.0, to.1);
        if unit.is_air() || unit.is_sea() {
            return 3;
        }
        let road = |t: &super::map::Tile| t.has(tile_flags::ROAD) || t.has(tile_flags::RAILROAD) || t.city.is_some();
        if road(&ft) && road(&tt) {
            let free = ft.has(tile_flags::RAILROAD) && ft.city.is_none() && self.is_human(unit.owner) && unit.goto.is_none();
            return if free { 0 } else { 1 };
        }
        if tt.is_ocean() {
            return 3;
        }
        tt.terrain.info().move_cost * 3
    }

    /// Would moving the land unit from `from` to `to` violate zones of control? As CIV.EXE's
    /// move code has it (0x5ac7): only onto an empty square, never off a ship, and only when
    /// both squares are `zoc_blocked` (0x9581): no city there and a unit of another civ on a
    /// neighbouring square of the same kind (`enemy_adjacent` 0x95ac, land beside land),
    /// treaty or not, barbarians too. The two tests come first, for every land unit, and show
    /// the human what they find (`zoc_blocked_look`); the exemptions follow (0x5b01, 0x5b08).
    pub fn zoc_blocks(&mut self, unit: &super::Unit, from: (i32, i32), to: (i32, i32)) -> bool {
        if !unit.is_land() || self.all_units().any(|u| u.id != unit.id && (u.x, u.y) == to) {
            return false;
        }
        if !(self.zoc_blocked_look(unit.owner, from.0, from.1) && self.zoc_blocked_look(unit.owner, to.0, to.1)) {
            return false;
        }
        !matches!(unit.kind, unit_ids::DIPLOMAT | unit_ids::CARAVAN) && !self.map.tile(from.0, from.1).is_ocean()
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
        // whatever the unit does next to a foreign city, CIV.EXE's unit_look has let that
        // city's garrison out by now (0x828f)
        let (ux, uy, owner) = {
            let u = self.unit(id);
            (u.x, u.y, u.owner)
        };
        self.reveal_garrisons_near(owner, ux, uy);
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
            let hostile = self.may_fight(slot, o);
            let city_here = self.city_at(tx, ty);
            let units_here = self.units_at(tx, ty);
            // a Diplomat meeting a lone foreign unit outside a city may bribe it, at war or not
            // (CIV.EXE 0x5da5, o 0x1dbae): the human is asked; the computer pays when half its
            // treasury covers the price; then the Diplomat moves on to its new unit
            // a Diplomat or Caravan may not go from the sea into a foreign city (0x5b91); a
            // Diplomat goes in whatever the treaties, before any question (0x5c04)
            if city_here.is_some() && matches!(u.kind, unit_ids::CARAVAN | unit_ids::DIPLOMAT) && self.map.is_ocean(u.x, u.y) {
                return Ok(MoveOutcome::Blocked);
            }
            if let Some(c) = city_here.filter(|_| u.kind == unit_ids::DIPLOMAT) {
                return Ok(MoveOutcome::EnteredForeignCity(c));
            }
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
            // bribe_unit refuses a stack, and a Diplomat never attacks (0x5f45)
            if u.kind == unit_ids::DIPLOMAT && !units_here.is_empty() {
                return Ok(MoveOutcome::Blocked);
            }
            if !hostile {
                // Caravan / diplomat may enter foreign cities at peace.
                if city_here.is_some() && matches!(u.kind, unit_ids::CARAVAN | unit_ids::DIPLOMAT) {
                    return Ok(MoveOutcome::EnteredForeignCity(city_here.unwrap()));
                }
                // the human may attack a treaty partner, or take its empty city, after
                // "Break treaty." (senate_check_attack 0x1a3b4 from combat 0x199fd and from the
                // territory rule 0x6575); the computer keeps the peace
                let acts = !units_here.is_empty() || (city_here.is_some() && u.is_land() && u.info().att > 0);
                if !(acts && self.is_human(slot) && self.treaty(slot, o)) {
                    return Ok(MoveOutcome::Blocked);
                }
                // an attack asks in combat, after the move code's questions (below)
                if units_here.is_empty() && self.treaty_break_ok != Some(o) {
                    // the territory rule drops the unit's goto before it asks, whatever the
                    // answer (0x656d)
                    self.clear_goto(id);
                    return Ok(MoveOutcome::BreakTreaty(o));
                }
                if units_here.is_empty() {
                    // the territory rule ends the treaty (0x6583); a fight ends it in combat
                    // (the answer holds until the fight, past "Attack at N/3 strength?")
                    self.treaty_break_ok = None;
                    self.clear_diplo_flag(slot, o, super::diplomacy::flags::PEACE);
                }
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
                // no attack from a ship: a land unit on an ocean square (0x5ec3)
                if u.is_land() && self.map.is_ocean(u.x, u.y) {
                    return Err(RuleError::Amphib);
                }
                // below one full move the human is asked to confirm the weakened attack, and
                // a computer's unit attacks only with two thirds of a move or more (CIV.EXE
                // 0x5fb2..0x603a)
                if !self.is_human(slot) && u.moves_left < 2 {
                    return Ok(MoveOutcome::Blocked);
                }
                if self.is_human(slot) && u.moves_left < 3 && !self.weak_attack_ok {
                    return Ok(MoveOutcome::WeakAttack(u.moves_left));
                }
                // then combat's senate_check_attack (0x199fd) asks about a treaty partner
                if !hostile && self.treaty_break_ok != Some(o) {
                    return Ok(MoveOutcome::BreakTreaty(o));
                }
                self.weak_attack_ok = false;
                self.treaty_break_ok = None;
                // move_units counts the units lost in the fight (ds:0x70d8, o 0x6075..0x60ae)
                self.counting_losses = true;
                let out = if u.kind == unit_ids::NUCLEAR {
                    self.nuke(id, tx, ty);
                    MoveOutcome::Attacked { won: true, at: (tx, ty), defender: unit_ids::NUCLEAR }
                } else {
                    let out = self.attack(id, tx, ty);
                    // the human sees what is left of a city he attacked, won or lost (0x61f8)
                    if slot == self.human_slot {
                        if let Some(c) = self.city_at(tx, ty) {
                            self.note_city_size(c);
                        }
                    }
                    out
                };
                self.counting_losses = false;
                return Ok(out);
            }
            if let Some(c) = city_here {
                if u.kind == unit_ids::NUCLEAR {
                    self.counting_losses = true;
                    self.nuke(id, tx, ty);
                    self.counting_losses = false;
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

        // the territory rule (CIV.EXE 0x64bb..0x65b5): a military land unit stepping onto
        // developed land of a treaty partner's city; the human is asked first ("Break
        // treaty."), a computer's unit stops at the human's border
        if let Some(o) = self.territory_crossed(&u, (tx, ty)) {
            if self.is_human(slot) && self.treaty(slot, o) {
                if self.treaty_break_ok != Some(o) {
                    // (the goto goes before the question, whatever the answer, 0x656d)
                    self.clear_goto(id);
                    return Ok(MoveOutcome::BreakTreaty(o));
                }
                self.treaty_break_ok = None;
                self.clear_diplo_flag(slot, o, super::diplomacy::flags::PEACE);
            } else if !self.is_human(slot) && self.treaty(o, slot) {
                self.unit_mut(id).moves_left = 0;
                return Ok(MoveOutcome::Blocked);
            }
        }

        // Terrain legality.
        let mut boarded = None;
        match u.info().domain {
            Domain::Land => {
                if target.is_ocean() {
                    // board a ship with room (0x5e0d): the unit is sentried, loses the fortify
                    // bits and gets exactly one move (moves = 3, 0x5e2c), which the step to the
                    // sea then costs without a roll (0x634a), and the move goes on as any other
                    let Some(s) = self.boarding_ship(&u, (tx, ty)) else { return Ok(MoveOutcome::Blocked) };
                    let um = self.unit_mut(id);
                    um.sentry_keeping_job(false);
                    um.moves_left = 3;
                    boarded = Some(s);
                } else if self.zoc_blocks(&u, (u.x, u.y), (tx, ty)) {
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
        let moves = self.unit(id).moves_left;
        if cost > moves && u.is_land() && moves != 3 && self.rng.below(cost as u32) > moves as u32 {
            self.unit_mut(id).moves_left = 0;
            return Ok(MoveOutcome::Blocked);
        }
        let had_hut = target.has(tile_flags::HUT);
        // the step keeps the unit's successor in its old square's ring (0x6799): a ship's
        // cargo is taken from there
        let old_next = self.unit(id).next;
        // a step onto a square another civ owns on the site/owner layer clears it (0x6815)
        let owner_there = self.map.claim_at(tx, ty);
        if (1..8).contains(&owner_there) && owner_there != slot {
            self.map.set_claim(tx, ty, 0);
        }
        self.place_unit(id, tx, ty, cost);
        // a unit reaching its goto square forgets it, and a computer's unit stops there
        // (CIV.EXE 0x6848..0x6878); coming ashore from the sea ends the move of all but
        // aircraft, a ship's into port too (0x687d..0x68a8)
        if u.goto == Some((tx, ty)) {
            self.clear_goto(id);
            self.set_heading(id, 0xff);
            if !self.is_human(slot) {
                self.unit_mut(id).moves_left = 0;
            }
        }
        if self.map.is_ocean(u.x, u.y) && target.is_land() && !u.is_air() {
            self.unit_mut(id).moves_left = 0;
        }
        self.take_cargo(id, old_next, (u.x, u.y), (tx, ty));
        // aircraft landing in a city / carrier refuels; landing ends the aircraft's turn, its
        // goto and its return to base (bit 0x02, CIV.EXE 0x6c71..0x6d00)
        if u.is_air() {
            let lands = self.city_at(tx, ty).is_some() || self.units_at(tx, ty).into_iter().any(|s| s != id && self.unit(s).kind == unit_ids::CARRIER && self.unit(s).owner == slot);
            if lands {
                self.unit_mut(id).moves_left = 0;
                self.clear_goto(id);
                if self.unit(id).order == UnitOrder::Home {
                    self.unit_mut(id).order = UnitOrder::None;
                }
            }
            // a Carrier in the stack takes any number of air units, Nuclear included
            // (stack_has_type 0x918d, 0x6cd1; the table gives it no capacity)
            if self.city_at(tx, ty).map(|c| self.city(c).owner == slot).unwrap_or(false) || self.on_carrier(id) {
                self.unit_mut(id).fuel = 0;
            }
        }
        // a unit that enters a foreign city is seen by its owner (0x6a65)
        if let Some(c) = self.city_at(tx, ty) {
            let o = self.city(c).owner;
            if o != slot {
                self.unit_mut(id).seen_by |= 1 << o;
            }
        }
        self.reveal_garrisons_near(slot, tx, ty);
        self.refresh_vision(slot);
        // unit_look: who sees whom, and the civilizations whose units are next to it meet (0x8491)
        let audiences = self.audience_pending;
        self.mark_sightings(id);
        self.look_for_contacts(id);
        self.unit_look_goals(id);
        if had_hut {
            self.map.get_mut(tx, ty).unwrap().set(tile_flags::HUT, false);
            // an audience the look opened in a computer's unit loop comes first (unit_look
            // 0x6ae2, then hut_outcome 0x6b05): the hut waits for the human's answer
            if self.ai_memory.moving == Some(slot) && self.audience_pending & !audiences != 0 {
                self.deferred_hut = Some(id);
            } else {
                self.enter_hut(id);
            }
            return Ok(MoveOutcome::EnteredHut);
        }
        Ok(boarded.map_or(MoveOutcome::Moved, MoveOutcome::Boarded))
    }

    /// The ship a land unit of `u`'s owner boards stepping onto the sea square `to`: the
    /// first transport of the owner's stack there, when the stack has room (CIV.EXE 0x5e0d,
    /// `stack_free_capacity` 0x9231 > 0). Which ship carries it is not kept: every move of a
    /// ship takes along what its stack holds (`take_cargo`).
    pub fn boarding_ship(&self, u: &super::Unit, to: (i32, i32)) -> Option<UnitId> {
        let head = self.units_at(to.0, to.1).into_iter().find(|h| self.unit(*h).owner == u.owner)?;
        if self.stack_free_capacity(head) <= 0 {
            return None;
        }
        self.stack_ring(head, 128).into_iter().find(|s| self.unit(*s).carries_land_units())
    }

    /// `stack_free_capacity` (CIV.EXE 0x9231) of `head`'s stack: the capacities of its
    /// transports (AI role 5) less one for each of its land units. Not copied: the walk
    /// (`for_each_in_stack` 0x92a7) stops after ten units.
    pub fn stack_free_capacity(&self, head: UnitId) -> i32 {
        self.stack_ring(head, 128)
            .into_iter()
            .map(|s| {
                let su = self.unit(s);
                let room = if su.info().role == crate::rules::AiRole::Transport { su.info().capacity as i32 } else { 0 };
                room - su.is_land() as i32
            })
            .sum()
    }

    /// Is unit `id` aboard a ship? CIV.EXE keeps no link from a unit to its ship: a land unit
    /// on an ocean square is a passenger (`wake_if_not_at_sea_cb` 0x92fe, the amphibious
    /// attack 0x5ec3), an aircraft rides a Carrier of its stack (`on_carrier`).
    pub fn aboard(&self, id: UnitId) -> bool {
        let u = self.unit(id);
        (u.is_land() && self.map.is_ocean(u.x, u.y)) || (u.is_air() && self.on_carrier(id))
    }

    /// Does `id`'s stack ring hold a Carrier (`stack_has_type(civ, unit, 0x17)` 0x918d)? Not
    /// copied: the walk stops after ten units.
    pub fn on_carrier(&self, id: UnitId) -> bool {
        self.stack_ring(id, 128).into_iter().any(|s| s != id && self.unit(s).kind == unit_ids::CARRIER)
    }

    /// The units a ship that stepped from `from` to `to` takes along (move_units
    /// 0x68ad..0x6a62): only a ship with a capacity, or a Carrier. CIV.EXE keeps no cargo
    /// list; it walks the old square's stack ring from the ship's old successor `first`
    /// (+0xa, 0x6799) and takes the units of its domain, land units onto transports and
    /// aircraft onto a Carrier (0x6916): every one from the sea, else (a ship leaving a city)
    /// the sentried ones, and of a computer's stack also every one neither fortifying nor
    /// fortified (status & 0xc) that is not the last left on the square (+0xa = 0xff,
    /// 0x6964). Each one out of the old ring and into the new (0x697a/0x698e) loses its
    /// heading (+8, 0x69b9), is sentried by a ship with a table capacity, so not by the
    /// Carrier (0x69d6), keeping its other status bits (`Unit::sentry_job`), and with a range gets a full tank (+5 = range - 1, 0x6a02: civrs's 0
    /// turns flown). The walk ends after 20 links, at the end of the ring, or once the
    /// capacity is reached; a Carrier and a barbarian ship have no limit (0x6a1e..0x6a62).
    /// Units over the capacity stay where they are, at sea too.
    fn take_cargo(&mut self, id: UnitId, first: Option<UnitId>, from: (i32, i32), to: (i32, i32)) {
        let u = self.unit(id).clone();
        let carrier = u.kind == unit_ids::CARRIER;
        let capacity = u.info().capacity as i32;
        if !u.is_sea() || (capacity == 0 && !carrier) {
            return;
        }
        let at_sea = self.map.is_ocean(from.0, from.1);
        let human = self.is_human(u.owner);
        let (mut cur, mut links, mut loaded) = (first, 0, 0);
        while let Some(c) = cur {
            let Some(cu) = self.unit_opt(c).cloned().filter(|cu| cu.owner == u.owner && cu.pos() == from) else { break };
            let after = cu.next;
            let domain = if carrier { cu.is_air() } else { cu.is_land() };
            let fortified = matches!(cu.order, UnitOrder::Fortify | UnitOrder::Fortified);
            if domain && (cu.order == UnitOrder::Sentry || at_sea || (!human && !fortified && cu.next.is_some())) {
                self.set_heading(c, 0xff);
                self.relocate_unit(c, to.0, to.1);
                let m = self.unit_mut(c);
                m.seen_by = 0;
                // `or status,1` (0x69e5): a job or fortification stays under the sentry bit
                if capacity != 0 {
                    m.sentry_keeping_job(true);
                }
                if m.info().range != 0 {
                    m.fuel = 0;
                }
                loaded += 1;
            }
            links += 1;
            if links >= 20 || !(capacity > loaded || carrier || u.owner == 0) {
                break;
            }
            cur = after;
        }
    }

    /// Move a unit to a tile and deduct movement cost (thirds).
    pub fn place_unit(&mut self, id: UnitId, x: i32, y: i32, cost: u8) {
        // out of the old square's stack ring, into the new one's (0x67ae/0x67ce)
        self.relocate_unit(id, x, y);
        let human = self.is_human(self.unit(id).owner);
        let u = self.unit_mut(id);
        u.moves_left = u.moves_left.saturating_sub(cost);
        if u.order == UnitOrder::Fortified || u.order == UnitOrder::Fortify || u.order.is_work() {
            u.order = UnitOrder::None;
        }
        // a computer's unit but an aircraft keeps only its veteran bits (`and status,0x30`,
        // 0x6810), the human's keeps them all: the job under a boarder's sentry goes
        if !human && !u.is_air() {
            u.sentry_job = None;
        }
        // a move resets the work counter (unit +5 = the type's fuel, 0x6d00) and who has seen
        // the unit (+9, 0x67e4)
        u.order_turns = 0;
        u.seen_by = 0;
        let owner = u.owner;
        self.note_occupant(owner, x, y);
    }

    /// Has civilization `slot` seen unit `id` since its last step (unit +9)? Its owner
    /// always has.
    pub fn unit_seen_by(&self, id: UnitId, slot: Slot) -> bool {
        self.unit_opt(id).is_some_and(|u| u.owner == slot || u.seen_by & (1 << slot) != 0)
    }

    /// Does `slot` see the units on (x, y)? As draw_tile_with_units (0x1b85a) decides it: the
    /// square's first unit (0x1bade) belongs to `slot` or has its bit.
    pub fn units_shown_to(&self, slot: Slot, x: i32, y: i32) -> bool {
        self.units_at(x, y).first().is_some_and(|u| self.unit_seen_by(*u, slot))
    }

    /// The sightings of `unit_look` (CIV.EXE 0x8038) for unit `id` at its square:
    /// - next to it (0x8255..0x8413): a foreign city sees a land unit, and the human's unit
    ///   sees the city's size (0x82a9); the first unit of a
    ///   foreign stack sees it unless it stands in a city, and is seen by it unless that stack
    ///   is in a city (cities hide their units);
    /// - two squares away, for a unit that sees 5x5 (ships: ocean squares only, 0x878d): the
    ///   first unit of a foreign stack outside a city is seen, never a Submarine (0x87f6);
    /// - two squares away whatever the unit (0x8546..0x8715): unless it is a Submarine, a
    ///   foreign stack's first unit that sees 5x5 (a ship only when this unit is at sea) wakes
    ///   from sentry (that unit alone, 0x85cd), loses its goto and sees this unit unless it is
    ///   in a city; otherwise one
    ///   of the human's cities there sees it, when the human knows this unit's square.
    pub fn mark_sightings(&mut self, id: UnitId) {
        let Some(u) = self.unit_opt(id).cloned() else { return };
        let l = u.owner;
        let human = self.human_slot;
        let in_city = self.city_at(u.x, u.y).is_some();
        let far = u.sight_range() > 1;
        let at_sea = self.map.is_ocean(u.x, u.y);
        let human_knows_here = self.explored(human, u.x, u.y);
        let mut seen = 0u8;
        let mut marked: Vec<UnitId> = Vec::new();
        let mut woken: Vec<UnitId> = Vec::new();
        let mut known: Vec<super::CityId> = Vec::new();
        for dy in -2..=2i32 {
            for dx in -2..=2i32 {
                let (nx, ny) = (Map::wrap_x(u.x + dx), u.y + dy);
                if (dx, dy) == (0, 0) || !Map::in_bounds(nx, ny) {
                    continue;
                }
                let city = self.city_at(nx, ny);
                let head = self.units_at(nx, ny).first().copied().filter(|o| self.unit(*o).owner != l);
                if dx.abs() <= 1 && dy.abs() <= 1 {
                    if let Some(c) = city {
                        let o = self.city(c).owner;
                        if o != l && u.is_land() {
                            seen |= 1 << o;
                        }
                        // the human's unit sees the size of a foreign city next to it (0x82a9)
                        if o != l && l == human {
                            known.push(c);
                        }
                    }
                    if let Some(h) = head {
                        if !in_city {
                            seen |= 1 << self.unit(h).owner;
                        }
                        if city.is_none() {
                            marked.push(h);
                        }
                    }
                    continue;
                }
                if let Some(h) = head {
                    if far && !(u.is_sea() && !self.map.is_ocean(nx, ny)) && city.is_none() && self.unit(h).kind != unit_ids::SUBMARINE {
                        marked.push(h);
                    }
                    let hu = self.unit(h);
                    if u.kind != unit_ids::SUBMARINE {
                        if hu.sight_range() > 1 && !(hu.is_sea() && !at_sea) {
                            woken.push(h);
                            if !in_city {
                                seen |= 1 << hu.owner;
                            }
                        } else if hu.owner == human && city.is_some() && human_knows_here {
                            seen |= 1 << human;
                        }
                    }
                }
                if city.is_some_and(|c| self.city(c).owner == human) && human_knows_here {
                    seen |= 1 << human;
                }
            }
        }
        self.unit_mut(id).seen_by |= seen;
        for c in known {
            self.note_city_size(c);
        }
        for h in marked {
            self.unit_mut(h).seen_by |= 1 << l;
        }
        // only the first unit of the stack: 0x85cd clears its sentry bit whatever it is (no
        // stack walk, no passenger test: no land unit sees two squares far)
        for h in woken {
            let hu = self.unit_mut(h);
            hu.wake_sentry();
            hu.goto = None;
        }
    }

    /// Does the human watch unit `id` step to `to` (CIV.EXE 0x6721)? When it is his own or he
    /// has seen it, and also (0x6607, 0x6672, bits CIV.EXE ORs in before the step clears
    /// them) when it is a barbarian on land leaving a square he knows near one of his cities,
    /// or it enters a foreign city he knows while he has an embassy with either side.
    pub fn human_watches_move(&self, id: UnitId, to: (i32, i32)) -> bool {
        let Some(u) = self.unit_opt(id) else { return false };
        let human = self.human_slot;
        if self.unit_seen_by(id, human) {
            return true;
        }
        if u.owner == 0 && !self.map.is_ocean(to.0, to.1) && self.explored(human, u.x, u.y) {
            if let (Some(c), _) = self.nearest_city_exe(u.x, u.y) {
                if self.city(c).owner == human {
                    return true;
                }
            }
        }
        if let Some(c) = self.city_at(to.0, to.1) {
            let o = self.city(c).owner;
            let embassy = self.player(human).has_embassy(u.owner) || self.player(human).has_embassy(o);
            if o != u.owner && self.explored(human, to.0, to.1) && embassy {
                return true;
            }
        }
        false
    }

    /// What a ship at sea has aboard: the land units of its stack, for a Carrier the
    /// aircraft (the units its next move takes along, capacity aside).
    pub fn cargo_of(&self, ship: UnitId) -> Vec<UnitId> {
        let s = self.unit(ship);
        if !self.map.is_ocean(s.x, s.y) {
            return Vec::new();
        }
        let carrier = s.kind == unit_ids::CARRIER;
        self.stack_ring(ship, 128).into_iter().filter(|o| if carrier { self.unit(*o).is_air() } else { self.unit(*o).is_land() }).collect()
    }

    pub fn cargo_count(&self, ship: UnitId) -> usize {
        self.cargo_of(ship).len()
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

    /// The next step of a unit's goto: CIV.EXE's `goto_direction` (0x1db04, see `goto`), for
    /// the human's units as for the computer's. `None` when there is no step; the goto may
    /// have been dropped then (arrived, or no way found).
    pub fn next_goto_step(&mut self, id: UnitId) -> Option<Dir8> {
        self.goto_step(id)
    }

    /// Ships of `slot` get one more move per turn from the Lighthouse or Magellan's Expedition;
    /// having both gives no more (CIV.EXE 0x40e2).
    pub fn extra_sea_move(&self, slot: super::Slot) -> bool {
        self.owner_has_wonder(slot, rules::wonder_ids::LIGHTHOUSE) || self.owner_has_wonder(slot, rules::wonder_ids::MAGELLANS_EXPEDITION)
    }

    /// `unit_look` (0x8038) of one unit: the garrisons of the foreign cities next to it come
    /// out, it sees and is seen, meets other civilizations and notes its civ's goals.
    pub fn unit_look(&mut self, id: UnitId) {
        let Some((slot, x, y)) = self.unit_opt(id).map(|u| (u.owner, u.x, u.y)) else { return };
        // the human's unit shows him the improvements of its 3x3 (5x5 with a sight of two,
        // a ship's outer ring on ocean only) (0x8038..0x807e)
        if slot == self.human_slot {
            let u = self.unit(id);
            let (r, sea) = (u.sight_range(), u.is_sea());
            for yy in y - r..=y + r {
                for xx in x - r..=x + r {
                    let outer = (xx - x).abs() > 1 || (yy - y).abs() > 1;
                    if Map::in_bounds(xx, yy) && !(sea && outer && !self.map.is_ocean(xx, yy)) {
                        self.map.update_seen(xx, yy);
                    }
                }
            }
        }
        self.reveal_garrisons_near(slot, x, y);
        self.mark_sightings(id);
        self.look_for_contacts(id);
        self.unit_look_goals(id);
    }

    /// Give the units of `slot` their moves again at the start of its `move_units`
    /// (CIV.EXE 0x40a4..0x412e, after the civ's `ai_civ_turn` and `ai_reset_goals`).
    ///
    /// The units then look around and finish fortifying one by one as the unit loop comes
    /// to them (0x41b7 unit_look, 0x4239 `complete_fortify`): the computer's and the
    /// barbarians' in `ai::unit_ai::move_units`, the human's in his loop
    /// (`Game::next_active_unit_from`).
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
            if u.order == UnitOrder::Skip {
                u.order = UnitOrder::None;
            }
        }
        // a sentried unit wakes only when a foreign unit looks around next to it, treaty or
        // not (unit_look 0x835f, 0x85cd): its own civ's turn start wakes nothing
    }

    /// A fortifying unit the unit loop comes to with moves (CIV.EXE 0x4239..0x431f): the
    /// fortification completes with the turn's moves spent; a computer's (or barbarian's)
    /// unit in a city of size 3 or more makes it its home (0x425b..0x42ae), and a unit
    /// fortified in its home city stashes the city's garrison when no foreign unit is next
    /// to it (0x42b2..0x431f). The loop goes on to the next unit.
    pub fn complete_fortify(&mut self, id: UnitId) {
        let Some(u) = self.unit_opt(id) else { return };
        let (slot, x, y) = (u.owner, u.x, u.y);
        let um = self.unit_mut(id);
        um.order = UnitOrder::Fortified;
        um.moves_left = 0;
        let Some(c) = self.city_at(x, y) else { return };
        if slot != self.human_slot && self.city(c).size >= 3 {
            self.unit_mut(id).home = Some(c);
        }
        if self.unit(id).home == Some(c) && !self.foreign_unit_adjacent(slot, x, y) {
            self.stash_garrison(c);
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
    fn new_units_take_the_lowest_free_record_and_wait_in_record_order() {
        // create_unit 0x8b51 fills the first free record of the owner's table; move_units
        // offers the units by record, so a new unit in a freed record comes first
        let mut g = Game::new(GameSetup { seed: 5, ..Default::default() }, None);
        let slot = g.human_slot;
        let first = g.units_of(slot)[0];
        let (x, y) = g.unit(first).pos();
        let a = g.add_unit(slot, unit_ids::MILITIA, x, y);
        let b = g.add_unit(slot, unit_ids::MILITIA, x, y);
        let records: Vec<u8> = g.units_of(slot).iter().map(|u| g.unit(*u).record).collect();
        assert_eq!(records, (0..records.len() as u8).collect::<Vec<_>>());
        let ra = g.unit(a).record;
        g.remove_unit(a);
        // another civ's unit takes no record of the human's
        let other = (1..8).find(|s| *s != slot && g.player(*s).alive).unwrap();
        g.add_unit(other, unit_ids::MILITIA, x, y);
        let c = g.add_unit(slot, unit_ids::SETTLERS, x, y);
        assert_eq!(g.unit(c).record, ra);
        assert!(c > b, "a later id");
        let order = g.units_of(slot);
        assert!(order.iter().position(|u| *u == c) < order.iter().position(|u| *u == b));
        for u in g.units_of(slot) {
            g.unit_mut(u).moves_left = 0;
        }
        g.unit_mut(b).moves_left = 3;
        g.unit_mut(c).moves_left = 3;
        g.active_unit = None;
        assert_eq!(g.next_active_unit(), Some(c));
    }

    #[test]
    fn the_next_unit_is_one_in_the_map_view_else_the_view_moves_to_it() {
        // move_units 0x434f: a first round only takes units in columns vx+1..vx+14 and rows
        // vy+1..vy+9 of the view; then the first other one, with the view centred on it
        let mut g = Game::new(GameSetup { seed: 5, ..Default::default() }, None);
        let slot = g.human_slot;
        for u in g.units_of(slot) {
            g.unit_mut(u).moves_left = 0;
        }
        let a = g.add_unit(slot, unit_ids::MILITIA, 20, 20);
        let b = g.add_unit(slot, unit_ids::MILITIA, 40, 30);
        g.active_unit = None;
        g.view = Some((30, 21));
        assert_eq!(g.next_active_unit(), Some(b), "b is in the view, a is not");
        assert_eq!(g.view, Some((30, 21)));
        g.view = Some((30, 20));
        assert_eq!(g.next_active_unit(), Some(a), "row vy+10 is outside: a comes first by record");
        assert_eq!(g.view, Some((13, 14)));
    }

    #[test]
    fn off_the_screen_the_loop_goes_on_from_its_mark_not_from_the_last_unit() {
        // move_units 0x4150 / 0x4381: with nobody left in the view, a unit outside it is
        // only taken once the loop has passed its mark (127 at the turn's start, then the
        // record before the last unit it took off the screen, 0x43d8): after the unit of
        // record 1 the one of record 0 comes before the one of record 2 (as CIV.EXE did in a
        // differential run from an edited CIVIL4)
        let mut g = Game::new(GameSetup { seed: 5, ..Default::default() }, None);
        let slot = g.human_slot;
        for u in g.units_of(slot) {
            g.remove_unit(u);
        }
        let a = g.add_unit(slot, unit_ids::MILITIA, 20, 10);
        let b = g.add_unit(slot, unit_ids::MILITIA, 40, 30);
        let c = g.add_unit(slot, unit_ids::MILITIA, 20, 20);
        assert_eq!([a, b, c].map(|u| g.unit(u).record), [0, 1, 2]);
        g.reset_unit_loop();
        g.view = Some(Game::view_centred_on(40, 30));
        g.active_unit = None;
        assert_eq!(g.next_active_unit(), Some(b), "b is in the view");
        g.active_unit = Some(b);
        g.unit_mut(b).moves_left = 0;
        assert_eq!(g.next_active_unit(), Some(a), "from the mark 127 on: record 0 first");
        assert_eq!(g.unit_loop.mark, 127, "taken off the screen: the record before a, 0 - 1");
        g.active_unit = Some(a);
        g.unit_mut(a).moves_left = 0;
        assert_eq!(g.next_active_unit(), Some(c));
    }

    #[test]
    fn a_moving_unit_keeps_two_squares_from_the_view_edge() {
        // scroll_to 0x9505: columns vx+2..vx+13 and rows vy+2..vy+9 keep the view, any other
        // square gets the origin (x - 8, y - 6)
        assert_eq!(Game::view_scrolled_to((5, 23), 12, 27), (5, 23));
        assert_eq!(Game::view_scrolled_to((5, 23), 7, 24), (79, 18));
        assert_eq!(Game::view_scrolled_to((5, 23), 7, 33), (79, 27));
        assert_eq!(Game::view_scrolled_to((5, 23), 19, 25), (11, 19));
        assert_eq!(Game::view_scrolled_to((75, 10), 2, 15), (75, 10), "across the date line");
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
    fn fortifying_completes_with_the_next_turns_moves_spent() {
        // CIV.EXE 0x4239: flag 0x04 becomes 0x08 and the moves go to 0, for the human as his
        // loop passes the unit
        let (mut g, id) = militia_before_mountains(1, 3);
        let slot = g.unit(id).owner;
        g.unit_mut(id).order = crate::game::UnitOrder::Fortify;
        g.reset_moves(slot);
        assert_eq!((g.unit(id).order, g.unit(id).moves_left), (crate::game::UnitOrder::Fortify, 3), "not before the loop comes");
        g.run_human_loop();
        assert_eq!(g.unit(id).order, crate::game::UnitOrder::Fortified);
        assert_eq!(g.unit(id).moves_left, 0);
        g.reset_moves(slot);
        assert_eq!(g.unit(id).moves_left, 3);
        // a computer's unit neither: its loop completes it (ai::unit_ai::move_units)
        let ai = (1..8).find(|s| *s != slot && g.player(*s).in_game).unwrap();
        let (x, y) = g.unit(id).pos();
        let b = g.add_unit(ai, unit_ids::MILITIA, x, y + 3);
        g.unit_mut(b).order = crate::game::UnitOrder::Fortify;
        g.reset_moves(ai);
        assert_eq!((g.unit(b).order, g.unit(b).moves_left), (crate::game::UnitOrder::Fortify, 3));
        g.complete_fortify(b);
        assert_eq!((g.unit(b).order, g.unit(b).moves_left), (crate::game::UnitOrder::Fortified, 0));
    }

    #[test]
    fn a_goto_that_fails_for_want_of_moves_is_kept() {
        // 0x636f: the step's roll fails, the moves go and the goto stays for the next turn
        let mut failed = 0;
        for seed in 0..40 {
            let (mut g, id) = militia_before_mountains(seed, 2);
            let (x, y) = g.unit(id).pos();
            for dy in -1..=1 {
                let t = g.map.get_mut(x + 1, y + dy).unwrap();
                t.terrain = Terrain::Mountains;
                t.flags = 0;
            }
            g.set_goto(id, x + 2, y);
            assert_eq!(g.run_auto_unit(id), crate::game::AutoStop::Done);
            if g.unit(id).pos() == (x, y) {
                failed += 1;
                assert_eq!(g.unit(id).moves_left, 0);
                assert_eq!((g.unit(id).order, g.unit(id).goto), (crate::game::UnitOrder::Goto, Some((x + 2, y))));
            }
        }
        assert!(failed > 0);
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
    fn carriers_take_any_number_of_aircraft_and_no_land_units() {
        // the table gives the Carrier no capacity: aircraft land on it whatever their number
        // (0x6cd1) and its moves take them all along (0x6a53)
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
        for n in 0..12 {
            let f = g.add_unit(slot, unit_ids::FIGHTER, x + 1, y);
            assert_eq!(g.try_move(f, Dir8::W), Ok(MoveOutcome::Moved));
            assert!(g.aboard(f), "fighter {n}");
            // landing on a Carrier ends the aircraft's turn (CIV.EXE 0x6cea)
            assert_eq!(g.unit(f).moves_left, 0);
        }
        let f = g.add_unit(slot, unit_ids::FIGHTER, x + 1, y + 1);
        assert_eq!(g.try_move(f, Dir8::N), Ok(MoveOutcome::Moved));
        assert_eq!(g.unit(f).moves_left, g.unit(f).full_moves() - 3, "flying on over the sea");
        assert_eq!(g.cargo_count(carrier), 12);
        assert_eq!(g.try_move(carrier, Dir8::E), Ok(MoveOutcome::Moved));
        assert_eq!(g.units_at(x + 1, y).len(), 14, "the carrier, its 12 fighters and the one flying on");
    }

    #[test]
    fn ships_but_submarines_bombard_land_units() {
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        let slot = g.human_slot;
        let enemy = g.civ_slots().into_iter().find(|s| *s != slot).unwrap();
        g.set_relation(slot, enemy, crate::game::Relation::War);
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
        assert!(g.aboard(sentried));
        assert_eq!(g.unit(idle).pos(), (x, y));
    }

    #[test]
    fn boarding_takes_the_one_move_it_is_given_and_a_moving_ship_resentries_its_cargo() {
        // 0x5e22: sentried, moves = 3, the step to the sea costs them; 0x69e5: a transport's
        // cargo is sentried on every move, woken or not
        use crate::game::UnitOrder;
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        let slot = g.human_slot;
        let (x, y) = (40, 25);
        g.map.get_mut(x, y).unwrap().terrain = Terrain::Grassland;
        g.map.get_mut(x + 1, y).unwrap().terrain = Terrain::Ocean;
        g.map.get_mut(x + 2, y).unwrap().terrain = Terrain::Ocean;
        let ship = g.add_unit(slot, unit_ids::SAIL, x + 1, y);
        let c = g.add_unit(slot, unit_ids::CAVALRY, x, y);
        g.unit_mut(c).goto = Some((x + 1, y));
        assert_eq!(g.try_move(c, Dir8::E), Ok(MoveOutcome::Boarded(ship)));
        let u = g.unit(c);
        assert_eq!((u.pos(), u.moves_left, u.order, u.goto), ((x + 1, y), 0, UnitOrder::Sentry, None));
        assert!(g.aboard(c));
        g.unit_mut(c).order = UnitOrder::None;
        assert_eq!(g.try_move(ship, Dir8::E), Ok(MoveOutcome::Moved));
        assert_eq!((g.unit(c).pos(), g.unit(c).order), ((x + 2, y), UnitOrder::Sentry));
    }

    #[test]
    fn a_moving_carrier_fills_its_aircraft_s_tanks() {
        // 0x6a02: +5 = range - 1 for every carried unit with a range
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        let slot = g.human_slot;
        let (x, y) = (40, 25);
        g.map.get_mut(x, y).unwrap().terrain = Terrain::Ocean;
        g.map.get_mut(x + 1, y).unwrap().terrain = Terrain::Ocean;
        let carrier = g.add_unit(slot, unit_ids::CARRIER, x, y);
        let bomber = g.add_unit(slot, unit_ids::BOMBER, x, y);
        g.unit_mut(bomber).fuel = 1;
        assert_eq!(g.try_move(carrier, Dir8::E), Ok(MoveOutcome::Moved));
        assert_eq!((g.unit(bomber).pos(), g.unit(bomber).fuel), ((x + 1, y), 0));
    }

    #[test]
    fn a_computer_carrier_takes_its_aircraft_along_awake() {
        // 0x69d6: only a ship with a table capacity (not the Carrier) sentries what it takes;
        // the last unit left in the city stays (0x6964)
        use crate::game::UnitOrder;
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        let ai = g.civ_slots().into_iter().find(|s| *s != g.human_slot).unwrap();
        let (x, y) = (40, 25);
        g.map.get_mut(x, y).unwrap().terrain = Terrain::Grassland;
        g.map.get_mut(x + 1, y).unwrap().terrain = Terrain::Ocean;
        g.add_city(ai, "Port".into(), x, y);
        let carrier = g.add_unit(ai, unit_ids::CARRIER, x, y);
        let fighter = g.add_unit(ai, unit_ids::FIGHTER, x, y);
        let last = g.add_unit(ai, unit_ids::FIGHTER, x, y);
        // the ring: carrier, last, fighter
        assert_eq!(g.stack_ring(carrier, 10), vec![carrier, last, fighter]);
        assert_eq!(g.try_move(carrier, Dir8::E), Ok(MoveOutcome::Moved));
        assert_eq!((g.unit(last).pos(), g.unit(fighter).pos()), ((x + 1, y), (x, y)));
        assert!(g.aboard(last));
        assert_eq!(g.unit(last).order, UnitOrder::None);
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

    /// Sea squares (x..x+2, 25) east of a land square at x - 1.
    fn sea_lane() -> (Game, i32, i32) {
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        let (x, y) = (40, 25);
        for xx in x - 1..=x + 2 {
            let t = g.map.get_mut(xx, y).unwrap();
            t.terrain = if xx < x { Terrain::Grassland } else { Terrain::Ocean };
            t.city = None;
        }
        (g, x, y)
    }

    #[test]
    fn a_ship_at_sea_takes_its_stack_in_ring_order_up_to_its_capacity() {
        // 0x6939: from the sea every land unit of the stack goes, sentried or not, whichever
        // ship it boarded; 0x6a4e: no more than the capacity, the rest stays at sea
        use crate::game::UnitOrder;
        let (mut g, x, y) = sea_lane();
        let slot = g.human_slot;
        let sail = g.add_unit(slot, unit_ids::SAIL, x, y);
        let trireme = g.add_unit(slot, unit_ids::TRIREME, x, y);
        let land: Vec<UnitId> = (0..5).map(|_| g.add_unit(slot, unit_ids::MILITIA, x, y)).collect();
        g.unit_mut(land[0]).order = UnitOrder::None;
        // ring: sail, then the others in reverse: trireme last
        let ring = g.stack_ring(trireme, 10);
        assert_eq!(ring[1], sail);
        let expected: Vec<UnitId> = ring[2..4].to_vec();
        assert_eq!(g.try_move(trireme, Dir8::E), Ok(MoveOutcome::Moved));
        let moved: Vec<UnitId> = land.iter().copied().filter(|l| g.unit(*l).pos() == (x + 1, y)).collect();
        assert_eq!(moved.len(), 2, "a Trireme takes two");
        assert!(expected.iter().all(|e| moved.contains(e)), "the first two after the ship");
        assert!(moved.iter().all(|m| g.unit(*m).order == UnitOrder::Sentry));
        assert_eq!(g.try_move(sail, Dir8::E), Ok(MoveOutcome::Moved));
        assert_eq!(g.units_at(x + 1, y).len(), 7);
        // two Triremes with three units aboard have room for one more
        let (mut g, x, y) = sea_lane();
        let t = [g.add_unit(slot, unit_ids::TRIREME, x, y), g.add_unit(slot, unit_ids::TRIREME, x, y)];
        for _ in 0..3 {
            g.add_unit(slot, unit_ids::MILITIA, x, y);
        }
        assert_eq!(g.stack_free_capacity(t[0]), 1);
        let a = g.add_unit(slot, unit_ids::MILITIA, x - 1, y);
        let b = g.add_unit(slot, unit_ids::MILITIA, x - 1, y);
        assert!(matches!(g.try_move(a, Dir8::E), Ok(MoveOutcome::Boarded(_))));
        assert_eq!(g.try_move(b, Dir8::E), Ok(MoveOutcome::Blocked));
    }

    #[test]
    fn a_ship_leaving_port_takes_the_sentried_units_and_a_computer_s_all_but_the_last() {
        // 0x6932..0x6964: in port the human's ship takes sentried units only; a computer's also
        // every unit not fortifying or fortified, working ones too, but not the last one left
        use crate::game::UnitOrder;
        let (mut g, x, y) = sea_lane();
        let human = g.human_slot;
        let ai = g.civ_slots().into_iter().find(|s| *s != human).unwrap();
        g.add_city(human, "Port".into(), x - 1, y);
        let ship = g.add_unit(human, unit_ids::SAIL, x - 1, y);
        let units: Vec<UnitId> = (0..5).map(|_| g.add_unit(human, unit_ids::MILITIA, x - 1, y)).collect();
        for u in &units[..4] {
            g.unit_mut(*u).order = UnitOrder::Sentry;
        }
        assert_eq!(g.try_move(ship, Dir8::E), Ok(MoveOutcome::Moved));
        let gone = units.iter().filter(|u| g.unit(**u).pos() == (x, y)).count();
        assert_eq!(gone, 3, "three of the four sentried, the awake one stays");
        assert_eq!(g.unit(units[4]).pos(), (x - 1, y));
        let (mut g, x, y) = sea_lane();
        g.add_city(ai, "Haven".into(), x - 1, y);
        let ship = g.add_unit(ai, unit_ids::TRANSPORT, x - 1, y);
        let fortified = g.add_unit(ai, unit_ids::MILITIA, x - 1, y);
        g.unit_mut(fortified).order = UnitOrder::Fortified;
        let settlers = g.add_unit(ai, unit_ids::SETTLERS, x - 1, y);
        g.unit_mut(settlers).order = UnitOrder::Road;
        let idle = g.add_unit(ai, unit_ids::MILITIA, x - 1, y);
        assert_eq!(g.try_move(ship, Dir8::E), Ok(MoveOutcome::Moved));
        assert_eq!((g.unit(settlers).pos(), g.unit(idle).pos(), g.unit(fortified).pos()), ((x, y), (x, y), (x - 1, y)));
        // the fortified unit gone, the last one stays
        g.relocate_unit(fortified, x + 2, y);
        g.unit_mut(ship).moves_left = 3;
        assert_eq!(g.try_move(ship, Dir8::W), Ok(MoveOutcome::Moved));
        g.unit_mut(ship).moves_left = 3;
        let left: Vec<UnitId> = [settlers, idle].into_iter().filter(|u| g.unit(*u).pos() == (x - 1, y)).collect();
        assert_eq!(left.len(), 2);
        for u in &left {
            g.unit_mut(*u).order = UnitOrder::None;
        }
        assert_eq!(g.try_move(ship, Dir8::E), Ok(MoveOutcome::Moved));
        assert_eq!(left.iter().filter(|u| g.unit(**u).pos() == (x, y)).count(), 1);
    }

    #[test]
    fn a_working_unit_taken_aboard_keeps_its_job_under_the_sentry_bit() {
        // 0x69e5 `or status,1`: a computer's Settlers at work goes along and keeps its job and
        // counter; 'u' clears bit 0 only (wake_cb 0x936e), so the job is back after landing
        use crate::game::UnitOrder;
        let (mut g, x, y) = sea_lane();
        let human = g.human_slot;
        let ai = g.civ_slots().into_iter().find(|s| *s != human).unwrap();
        g.add_city(ai, "Haven".into(), x - 1, y);
        let ship = g.add_unit(ai, unit_ids::TRANSPORT, x - 1, y);
        let settlers = g.add_unit(ai, unit_ids::SETTLERS, x - 1, y);
        let militia = g.add_unit(ai, unit_ids::MILITIA, x - 1, y);
        g.unit_mut(settlers).order = UnitOrder::Irrigate;
        g.unit_mut(settlers).order_turns = 2;
        g.unit_mut(militia).order = UnitOrder::Fortify;
        let idle = g.add_unit(ai, unit_ids::MILITIA, x - 1, y);
        assert_eq!(g.try_move(ship, Dir8::E), Ok(MoveOutcome::Moved));
        let s = g.unit(settlers);
        assert_eq!((s.pos(), s.order, s.sentry_job, s.order_turns), ((x, y), UnitOrder::Sentry, Some(UnitOrder::Irrigate), 2));
        assert_eq!(s.order_under_sentry(), UnitOrder::Irrigate);
        assert_eq!(g.unit(militia).pos(), (x - 1, y), "fortifying: stays (0x694a)");
        assert_eq!(g.unit(idle).order, UnitOrder::Sentry);
        // the ship's move from the sea keeps it so
        g.unit_mut(ship).moves_left = 3;
        assert_eq!(g.try_move(ship, Dir8::E), Ok(MoveOutcome::Moved));
        assert_eq!(g.unit(settlers).order_under_sentry(), UnitOrder::Irrigate);
        // the save keeps it (version 30)
        let dir = std::env::temp_dir().join(format!("civrs-sentry-job-{}", std::process::id()));
        let path = dir.join("s.sav");
        crate::save::save_game(&path, &g).unwrap();
        let l = crate::save::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!((l.unit(settlers).order, l.unit(settlers).sentry_job), (UnitOrder::Sentry, Some(UnitOrder::Irrigate)));
        g.unload(ship);
        let s = g.unit(settlers);
        assert_eq!((s.order, s.order_turns, s.moves_left), (UnitOrder::Irrigate, 2, s.full_moves()));
        assert_eq!(g.unit(idle).order, UnitOrder::None);
        // reset_unit (0x9403) clears the counter of a job under the sentry bit too
        g.unit_mut(settlers).sentry_keeping_job(true);
        g.reset_unit(settlers);
        assert_eq!((g.unit(settlers).order, g.unit(settlers).order_turns), (UnitOrder::None, 0));
    }

    #[test]
    fn a_ship_leaves_the_awake_units_it_brought_into_port() {
        use crate::game::UnitOrder;
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        let slot = g.human_slot;
        let (x, y) = (40, 25);
        g.map.get_mut(x, y).unwrap().terrain = Terrain::Grassland;
        g.map.get_mut(x + 1, y).unwrap().terrain = Terrain::Ocean;
        g.add_city(slot, "Port".into(), x, y);
        let ship = g.add_unit(slot, unit_ids::SAIL, x + 1, y);
        let (awake, asleep) = (g.add_unit(slot, unit_ids::MILITIA, x + 1, y), g.add_unit(slot, unit_ids::MILITIA, x + 1, y));
        for u in [awake, asleep] {
            g.unit_mut(u).order = UnitOrder::Sentry;
        }
        assert_eq!(g.try_move(ship, Dir8::W), Ok(MoveOutcome::Moved));
        // in port the units can be given orders like any other
        g.unit_mut(awake).order = UnitOrder::None;
        g.unit_mut(ship).moves_left = 3;
        assert_eq!(g.try_move(ship, Dir8::E), Ok(MoveOutcome::Moved));
        assert_eq!(g.unit(awake).pos(), (x, y));
        assert_eq!(g.unit(asleep).pos(), (x + 1, y));
    }

    /// A human Diplomat at (39,25) beside a city of a civ at peace with the human at (40,25).
    fn diplomat_before_city() -> (Game, UnitId, crate::game::Slot) {
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        let human = g.human_slot;
        let other = g.civ_slots().into_iter().find(|s| *s != human).unwrap();
        for xx in 38..=41 {
            for yy in 24..=26 {
                let t = g.map.get_mut(xx, yy).unwrap();
                t.terrain = Terrain::Grassland;
                t.city = None;
            }
        }
        g.add_city(other, "Abroad".into(), 40, 25);
        g.make_peace(human, other);
        let dip = g.add_unit(human, unit_ids::DIPLOMAT, 39, 25);
        (g, dip, other)
    }

    #[test]
    fn diplomats_enter_foreign_cities_from_land_only() {
        let (mut g, dip, other) = diplomat_before_city();
        g.add_unit(other, unit_ids::PHALANX, 40, 25);
        assert!(matches!(g.try_move(dip, Dir8::E), Ok(MoveOutcome::EnteredForeignCity(_))));
        g.map.get_mut(39, 25).unwrap().terrain = Terrain::Ocean;
        assert_eq!(g.try_move(dip, Dir8::E), Ok(MoveOutcome::Blocked));
    }

    #[test]
    fn diplomats_stop_at_foreign_stacks_without_a_treaty_question() {
        let (mut g, dip, other) = diplomat_before_city();
        for _ in 0..2 {
            g.add_unit(other, unit_ids::PHALANX, 39, 24);
        }
        assert_eq!(g.try_move(dip, Dir8::N), Ok(MoveOutcome::Blocked));
    }
}

#[cfg(test)]
mod zoc_tests {
    use crate::game::map::Dir8;
    use crate::game::{Game, GameSetup, MoveOutcome, RuleError, UnitOrder};
    use crate::rules::{unit_ids, Terrain};

    /// A human Legion at (40,25) on open grassland, the units of everyone else removed.
    fn open_field() -> (Game, crate::game::map::UnitId) {
        let mut g = Game::new(GameSetup { seed: 1, ..Default::default() }, None);
        for y in 20..31 {
            for x in 35..46 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Grassland;
                t.flags = 0;
                t.city = None;
            }
        }
        let ids: Vec<_> = g.all_units().map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        let id = g.add_unit(g.human_slot, unit_ids::LEGION, 40, 25);
        g.unit_mut(id).moves_left = 3;
        (g, id)
    }

    #[test]
    fn zoc_stops_a_move_between_squares_next_to_a_foreign_unit() {
        let (mut g, id) = open_field();
        let other = if g.human_slot == 2 { 3 } else { 2 };
        g.add_unit(other, unit_ids::MILITIA, 41, 26);
        assert_eq!(g.try_move(id, Dir8::E), Err(RuleError::Zoc));
        // away from the foreign unit is fine
        let mut away = g.clone();
        assert_eq!(away.try_move(id, Dir8::N), Ok(MoveOutcome::Moved));
        // onto one of our own units, or out of a city, is fine too
        let mut stacked = g.clone();
        stacked.add_unit(stacked.human_slot, unit_ids::SETTLERS, 41, 25);
        assert_eq!(stacked.try_move(id, Dir8::E), Ok(MoveOutcome::Moved));
        let mut city = g.clone();
        city.add_city(city.human_slot, "Rome".into(), 40, 25);
        assert_eq!(city.try_move(id, Dir8::E), Ok(MoveOutcome::Moved));
        // a foreign ship off the coast exerts none on land
        let mut coast = g.clone();
        let militia = coast.all_units().find(|u| u.owner == other).unwrap().id;
        coast.remove_unit(militia);
        coast.map.get_mut(41, 26).unwrap().terrain = Terrain::Ocean;
        coast.add_unit(other, unit_ids::TRIREME, 41, 26);
        assert_eq!(coast.try_move(id, Dir8::E), Ok(MoveOutcome::Moved));
    }

    #[test]
    fn the_zoc_test_shows_the_human_the_foreign_unit_and_city_it_finds() {
        // enemy_adjacent 0x9645..0x971c: the move is refused, what the test found is shown
        let (mut g, id) = open_field();
        let h = g.human_slot;
        let other = if h == 2 { 3 } else { 2 };
        let c = g.add_city(other, "Far".into(), 41, 24);
        g.city_mut(c).size = 3;
        g.city_mut(c).known_size = 0;
        let guard = g.add_unit(other, unit_ids::MILITIA, 41, 24);
        g.unit_mut(guard).seen_by = 1 << other;
        g.map.get_mut(41, 24).unwrap().explored &= !(1 << h);
        assert_eq!(g.try_move(id, Dir8::E), Err(RuleError::Zoc));
        assert_eq!(g.city(c).known_size, 3);
        assert_ne!(g.unit(guard).seen_by & 1 << h, 0);
        assert!(g.map.tile(41, 24).explored_by(h));
    }

    #[test]
    fn coming_ashore_ends_the_move_and_a_reached_goto_is_dropped() {
        // CIV.EXE 0x687d: from the sea onto land the moves are gone
        let (mut g, id) = open_field();
        let slot = g.human_slot;
        g.map.get_mut(41, 25).unwrap().terrain = Terrain::Ocean;
        let _ship = g.add_unit(slot, unit_ids::TRIREME, 41, 25);
        let l = g.add_unit(slot, unit_ids::LEGION, 41, 25);
        g.unit_mut(l).moves_left = 3;
        assert_eq!(g.try_move(l, Dir8::E), Ok(MoveOutcome::Moved));
        assert_eq!(g.unit(l).moves_left, 0);
        // 0x6848: the goto square reached, the goto goes; a computer's unit stops there
        for y in [24, 25] {
            g.map.get_mut(40, y).unwrap().set(crate::game::tile_flags::ROAD, true);
        }
        g.unit_mut(id).goto = Some((40, 24));
        assert_eq!(g.try_move(id, Dir8::N), Ok(MoveOutcome::Moved));
        assert_eq!(g.unit(id).goto, None);
        assert_eq!(g.unit(id).moves_left, 2);
        let other = if slot == 2 { 3 } else { 2 };
        let c = g.add_unit(other, unit_ids::LEGION, 44, 30);
        g.unit_mut(c).goto = Some((44, 29));
        assert_eq!(g.try_move(c, Dir8::N), Ok(MoveOutcome::Moved));
        assert_eq!(g.unit(c).moves_left, 0);
    }

    #[test]
    fn railroads_are_free_for_the_human_off_a_goto_from_a_railroad_square() {
        use crate::game::tile_flags as tf;
        // CIV.EXE 0x62dd..0x6334: road on both squares, railroad on the square left
        let (mut g, id) = open_field();
        for x in 40..=42 {
            g.map.get_mut(x, 25).unwrap().set(tf::ROAD, true);
        }
        g.map.get_mut(40, 25).unwrap().set(tf::RAILROAD, true);
        let u = g.unit(id).clone();
        assert_eq!(g.move_cost(&u, (40, 25), (41, 25)), 0, "rail onto a road");
        assert_eq!(g.move_cost(&u, (41, 25), (40, 25)), 1, "road onto rail");
        let mut on_goto = u.clone();
        on_goto.goto = Some((45, 25));
        assert_eq!(g.move_cost(&on_goto, (40, 25), (41, 25)), 1);
        let other = if g.human_slot == 2 { 3 } else { 2 };
        let mut computer = u.clone();
        computer.owner = other;
        assert_eq!(g.move_cost(&computer, (40, 25), (41, 25)), 1);
        // a city square has a road but no railroad
        g.add_city(g.human_slot, "Rome".into(), 42, 25);
        assert_eq!(g.move_cost(&u, (42, 25), (41, 25)), 1);
        assert_eq!(g.move_cost(&u, (41, 25), (42, 25)), 1);
    }

    #[test]
    fn a_far_seeing_stack_two_squares_away_wakes_only_its_first_unit() {
        // 0x85cd: the first unit (0x1a68:0x145e) loses its sentry bit and goto, no stack walk
        // (only the 3x3 one walks the ring, 0x836f); the looker is seen by its civ
        use crate::game::UnitOrder;
        let (mut g, id) = open_field();
        let me = g.human_slot;
        let other = if me == 2 { 3 } else { 2 };
        let bombers: Vec<crate::game::UnitId> = (0..3).map(|_| g.add_unit(other, unit_ids::BOMBER, 42, 25)).collect();
        for b in &bombers {
            g.unit_mut(*b).order = UnitOrder::Sentry;
            g.unit_mut(*b).goto = Some((30, 30));
        }
        let ring = g.units_at(42, 25);
        g.unit_mut(id).seen_by = 1 << me;
        g.mark_sightings(id);
        let head = g.unit(ring[0]);
        assert_eq!((head.order, head.goto), (UnitOrder::None, None));
        for b in &ring[1..] {
            assert_eq!((g.unit(*b).order, g.unit(*b).goto), (UnitOrder::Sentry, Some((30, 30))));
        }
        assert!(g.unit_seen_by(id, other));
        g.unit_look_goals(id);
        assert!(ring[1..].iter().all(|b| g.unit(*b).order == UnitOrder::Sentry));
    }

    #[test]
    fn the_human_sees_a_foreign_city_s_size_only_next_to_it() {
        // city +8: 0 at founding (o 0x1b562); a unit of the human next to it sets it (0x82a9);
        // a capture by the human resets it, a loss to a computer player sets it (0x14144)
        let (mut g, id) = open_field();
        let me = g.human_slot;
        let other = if me == 2 { 3 } else { 2 };
        let c = g.add_city(other, "Far".into(), 42, 25);
        g.city_mut(c).size = 4;
        assert_eq!(g.city_size_shown(c), None);
        g.mark_sightings(id);
        assert_eq!(g.city_size_shown(c), None, "two squares away");
        assert_eq!(g.try_move(id, Dir8::E), Ok(MoveOutcome::Moved));
        assert_eq!(g.city_size_shown(c), Some(4));
        g.city_mut(c).size = 6;
        assert_eq!(g.city_size_shown(c), Some(4), "what he saw last");
        g.city_changed_hands(c, other, me);
        g.city_mut(c).owner = me;
        assert_eq!(g.city(c).known_size, 0);
        assert_eq!(g.city_size_shown(c), Some(6));
        g.city_mut(c).owner = other;
        g.city_changed_hands(c, me, other);
        assert_eq!(g.city_size_shown(c), Some(6));
        // a foreign city founded within two squares of one of his is known at once (o 0x1b734)
        let mine = g.add_city(me, "Home".into(), 36, 21);
        let near = g.add_city(other, "Near".into(), 38, 22);
        assert_eq!(g.city_size_shown(near), Some(1));
        let _ = mine;
    }

    #[test]
    fn the_human_s_loop_moves_gotos_and_looks_as_it_comes_to_each_unit() {
        // move_units 0x41b7: each unit looks as the loop passes its record, until the first
        // Settlers waits (0x45c9); a goto moves when the loop takes it (0x44b3 -> 0x4bbe)
        let (mut g, a) = open_field();
        let me = g.human_slot;
        let other = if me == 2 { 3 } else { 2 };
        let b = g.add_unit(me, unit_ids::MILITIA, 36, 22);
        g.set_goto(b, 37, 22);
        let c = g.add_unit(me, unit_ids::MILITIA, 43, 27);
        g.unit_mut(c).order = UnitOrder::Fortified;
        let d = g.add_unit(me, unit_ids::SETTLERS, 38, 24);
        let e = g.add_unit(me, unit_ids::MILITIA, 37, 27);
        assert_eq!([a, b, c, d, e].map(|u| g.unit(u).record), [0, 1, 2, 3, 4]);
        let near_c = g.add_city(other, "Far".into(), 44, 27);
        let near_e = g.add_city(other, "Near".into(), 36, 27);
        g.city_mut(near_c).size = 4;
        // (on turn 0 the first unit waiting closes the latch, FIRSTMOVE 0x4599)
        g.turn = 1;
        g.reset_unit_loop();
        g.view = Some(Game::view_centred_on(40, 25));
        g.active_unit = None;
        assert_eq!(g.next_active_unit(), Some(a));
        g.unit_mut(a).moves_left = 0;
        g.active_unit = Some(a);
        assert_eq!(g.next_active_unit(), Some(b), "the goto comes in record order");
        assert!(g.is_auto_unit(b));
        assert_eq!(g.unit(b).pos(), (36, 22), "and has not moved before");
        assert_eq!(g.city_size_shown(near_c), None, "c has not looked yet");
        assert_eq!(g.run_auto_unit(b), crate::game::AutoStop::Done);
        assert_eq!((g.unit(b).pos(), g.unit(b).order), ((37, 22), UnitOrder::None));
        g.unit_mut(b).moves_left = 0;
        g.active_unit = Some(b);
        assert_eq!(g.next_active_unit(), Some(d));
        assert_eq!(g.city_size_shown(near_c), Some(4), "the fortified unit looked as the loop passed it");
        assert!(g.unit_loop.looks_done, "the Settlers closed the latch");
        g.unit_mut(d).moves_left = 0;
        g.active_unit = Some(d);
        assert_eq!(g.next_active_unit(), Some(e));
        assert_eq!(g.city_size_shown(near_e), None, "no look after the Settlers waited");
    }

    #[test]
    fn the_human_s_loop_fortifies_and_works_as_it_comes_to_each_unit() {
        // 0x4239: a fortifying unit is fortified as the loop passes its record; 0x44bd: a
        // Settlers at work is taken wherever it is (no centring) and presses its job's key
        use crate::game::map::tile_flags;
        let (mut g, a) = open_field();
        let me = g.human_slot;
        g.turn = 1;
        let s = g.add_unit(me, unit_ids::SETTLERS, 45, 30);
        let f = g.add_unit(me, unit_ids::MILITIA, 41, 25);
        g.unit_mut(s).order = UnitOrder::Road;
        g.unit_mut(s).order_turns = 1;
        g.unit_mut(f).order = UnitOrder::Fortify;
        g.reset_moves(me);
        assert_eq!(g.unit(s).moves_left, 3, "no work at the turn's start");
        g.reset_unit_loop();
        let view = Game::view_centred_on(40, 25);
        g.view = Some(view);
        g.active_unit = None;
        assert_eq!(g.next_active_unit(), Some(a));
        assert_eq!(g.unit(f).order, UnitOrder::Fortify, "the loop has not come to it yet");
        g.unit_mut(a).moves_left = 0;
        g.active_unit = Some(a);
        assert_eq!(g.next_active_unit(), Some(s));
        assert_eq!((g.view, g.unit_loop.centred), (Some(view), false));
        assert!(g.is_auto_unit(s));
        assert_eq!(g.run_auto_unit(s), crate::game::AutoStop::Done);
        // Grassland: a road takes 2 turns, the second done now
        assert!(g.map.tile(45, 30).has(tile_flags::ROAD));
        assert_eq!((g.unit(s).order, g.unit(s).moves_left), (UnitOrder::None, 0));
        g.active_unit = Some(s);
        assert_eq!(g.next_active_unit(), None);
        assert_eq!((g.unit(f).order, g.unit(f).moves_left), (UnitOrder::Fortified, 0));
        // a job no longer possible goes, and the unit waits for orders with its moves
        g.reset_moves(me);
        g.unit_mut(s).order = UnitOrder::Road;
        g.unit_mut(s).order_turns = 1;
        assert!(matches!(g.run_auto_unit(s), crate::game::AutoStop::Refused(_)));
        assert_eq!((g.unit(s).order, g.unit(s).moves_left, g.unit(s).order_turns), (UnitOrder::None, 3, 0));
    }

    #[test]
    fn an_aircraft_returning_to_base_is_taken_outside_the_view() {
        // bit 0x02 skips the view test (0x4345 -> 0x43f4): no centring, no new mark
        let (mut g, a) = open_field();
        let me = g.human_slot;
        g.turn = 1;
        let f = g.add_unit(me, unit_ids::FIGHTER, 45, 30);
        g.unit_mut(f).order = UnitOrder::Home;
        g.unit_mut(a).moves_left = 0;
        g.reset_unit_loop();
        let view = Game::view_centred_on(40, 25);
        g.view = Some(view);
        g.active_unit = None;
        assert_eq!(g.next_active_unit(), Some(f));
        assert!(g.is_auto_unit(f));
        assert_eq!((g.view, g.unit_loop.mark), (Some(view), 127));
    }

    #[test]
    fn a_ship_next_to_a_foreign_city_makes_no_city_goals_and_wakes_ships() {
        use crate::game::goals::goal;
        // 0x82c5: only a land unit gives the city's owner its goals; 0x92fe: a sentried ship
        // next to the looker wakes, a land unit aboard does not
        let (mut g, _) = open_field();
        let me = g.human_slot;
        let other = if me == 2 { 3 } else { 2 };
        for x in 35..46 {
            g.map.get_mut(x, 20).unwrap().terrain = Terrain::Ocean;
        }
        let c = g.add_city(other, "Port".into(), 40, 21);
        let ship = g.add_unit(me, unit_ids::TRIREME, 41, 20);
        g.reset_goals(other);
        g.unit_look_goals(ship);
        assert!(!g.ai_memory.goals[other as usize].iter().any(|q| q.kind == goal::ATTACK && (q.x as i32, q.y as i32) == g.city(c).pos()));
        let sail = g.add_unit(other, unit_ids::TRIREME, 42, 20);
        let rider = g.add_unit(other, unit_ids::MILITIA, 42, 20);
        g.unit_mut(sail).order = crate::game::UnitOrder::Sentry;
        g.unit_mut(rider).order = crate::game::UnitOrder::Sentry;
        g.unit_look_goals(ship);
        assert_eq!(g.unit(sail).order, crate::game::UnitOrder::None);
        assert_eq!(g.unit(rider).order, crate::game::UnitOrder::Sentry);
        let walker = g.add_unit(me, unit_ids::LEGION, 39, 22);
        g.unit_look_goals(walker);
        assert!(g.ai_memory.goals[other as usize].iter().any(|q| q.kind == goal::ATTACK && (q.x as i32, q.y as i32) == g.city(c).pos()));
    }

    #[test]
    fn a_sentry_wakes_when_the_foreigner_looks_not_at_its_own_turn() {
        // unit_look 0x835f wakes the stack next to the looker; nothing else clears sentry
        let (mut g, id) = open_field();
        let me = g.human_slot;
        let other = if me == 2 { 3 } else { 2 };
        g.unit_mut(id).order = crate::game::UnitOrder::Sentry;
        let m = g.add_unit(other, unit_ids::MILITIA, 41, 25);
        g.reset_moves(me);
        assert_eq!(g.unit(id).order, crate::game::UnitOrder::Sentry);
        // the computer's loop looks with its sentried unit too, before passing it over
        g.unit_mut(m).order = crate::game::UnitOrder::Sentry;
        crate::ai::unit_ai::move_units(&mut g, other, &mut |_| {});
        assert_eq!(g.unit(id).order, crate::game::UnitOrder::None);
        assert_eq!(g.unit(m).pos(), (41, 25));
    }
}
