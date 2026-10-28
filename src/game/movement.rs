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
    /// treaty or not, barbarians too.
    pub fn zoc_blocks(&self, unit: &super::Unit, from: (i32, i32), to: (i32, i32)) -> bool {
        if !unit.is_land() || matches!(unit.kind, unit_ids::DIPLOMAT | unit_ids::CARAVAN) {
            return false;
        }
        if self.map.tile(from.0, from.1).is_ocean() || self.all_units().any(|u| u.id != unit.id && (u.x, u.y) == to) {
            return false;
        }
        self.zoc_blocked(unit.owner, from.0, from.1) && self.zoc_blocked(unit.owner, to.0, to.1)
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
                if u.carried_by.is_some() && u.is_land() {
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
                            self.mark_sightings(id);
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
        // onto transports and aircraft onto a Carrier, up to its capacity (CIV.EXE 0x6932); a
        // computer's ship takes every unit that is not fortified or fortifying (0x693f), and
        // they travel sentried (0x69e5). Deviation: civrs leaves units at work behind. Only at
        // sea does the whole stack go along, so what it brought in is chosen again here.
        if u.is_sea() && u.info().capacity > 0 && self.map.tile(u.x, u.y).is_land() {
            for o in self.all_units().filter(|o| o.carried_by == Some(id)).map(|o| o.id).collect::<Vec<_>>() {
                self.unit_mut(o).carried_by = None;
            }
            let air = u.kind == unit_ids::CARRIER;
            let computer = !self.is_human(slot);
            let waiting: Vec<UnitId> = self
                .all_units()
                .filter(|o| o.owner == slot && o.id != id && (o.x, o.y) == (u.x, u.y) && o.carried_by.is_none())
                .filter(|o| o.order == UnitOrder::Sentry || (computer && !matches!(o.order, UnitOrder::Fortify | UnitOrder::Fortified) && o.order_turns == 0))
                .filter(|o| if air { o.is_air() } else { o.is_land() })
                .map(|o| o.id)
                .collect();
            let room = (u.info().capacity as usize).saturating_sub(self.cargo_count(id));
            for o in waiting.into_iter().take(room) {
                let ou = self.unit_mut(o);
                ou.carried_by = Some(id);
                ou.order = UnitOrder::Sentry;
                self.ai_memory.heading.remove(&o);
            }
        }
        self.place_unit(id, tx, ty, cost);
        // land unit leaving a ship
        if u.carried_by.is_some() && target.is_land() {
            self.unit_mut(id).carried_by = None;
        }
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
        // units aboard move with ship; in port they are units of the city again, to be
        // activated, and go along only when sentried
        if u.is_sea() {
            let cargo: Vec<UnitId> = self.all_units().filter(|c| c.carried_by == Some(id)).map(|c| c.id).collect();
            let port = target.is_land();
            for c in cargo {
                let cu = self.unit_mut(c);
                cu.x = tx;
                cu.y = ty;
                cu.seen_by = 0;
                if port {
                    cu.carried_by = None;
                }
            }
        }
        // aircraft landing in a city / carrier refuels; landing ends the aircraft's turn and its
        // goto (CIV.EXE 0x6c71..0x6d00)
        if u.is_air() {
            let lands = self.city_at(tx, ty).is_some() || self.units_at(tx, ty).into_iter().any(|s| s != id && self.unit(s).kind == unit_ids::CARRIER && self.unit(s).owner == slot);
            if lands {
                self.unit_mut(id).moves_left = 0;
                self.clear_goto(id);
            }
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
        self.mark_sightings(id);
        self.look_for_contacts(id);
        self.unit_look_goals(id);
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
        if u.order == UnitOrder::Fortified || u.order == UnitOrder::Fortify || u.order.is_work() {
            u.order = UnitOrder::None;
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
    ///   foreign stack whose first unit sees 5x5 (a ship only when this unit is at sea) wakes
    ///   from sentry, loses its goto and sees this unit unless it is in a city; otherwise one
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
        for h in woken {
            let hu = self.unit_mut(h);
            if hu.order == UnitOrder::Sentry && hu.carried_by.is_none() {
                hu.order = UnitOrder::None;
            }
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

    /// Reset movement points at the start of a player's turn; handles fortify completion.
    ///
    /// CIV.EXE's unit loop at the start of a civilization's turn (0x4174..0x4327) also looks
    /// around with every unit (unit_look 0x41e3, which lets out the garrison of a foreign city
    /// next to it) and, for a unit that finishes fortifying in a city: a computer's unit is
    /// rehomed to a city of size 3 or more (0x42a4), and a unit fortified in its home city
    /// stashes the city's garrison when no foreign unit is next to it (0x42b2..0x431f).
    pub fn reset_moves(&mut self, slot: super::Slot) {
        let extra_sea_move = self.extra_sea_move(slot);
        let mut fortified = Vec::new();
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
                // the fortification completes with the turn's moves spent (CIV.EXE 0x4239)
                u.order = UnitOrder::Fortified;
                u.moves_left = 0;
                fortified.push(u.id);
            }
            if u.order == UnitOrder::Skip {
                u.order = UnitOrder::None;
            }
        }
        let looking: Vec<(UnitId, (i32, i32))> = self.all_units().filter(|u| u.owner == slot).map(|u| (u.id, u.pos())).collect();
        for (id, (x, y)) in looking {
            self.reveal_garrisons_near(slot, x, y);
            self.mark_sightings(id);
            self.look_for_contacts(id);
            self.unit_look_goals(id);
        }
        for id in fortified {
            // an earlier stash may have taken the unit off the map already
            let Some((x, y)) = self.unit_opt(id).map(|u| u.pos()) else { continue };
            let Some(c) = self.city_at(x, y) else { continue };
            if slot != self.human_slot && self.city(c).size >= 3 {
                self.unit_mut(id).home = Some(c);
            }
            if self.unit(id).home == Some(c) && !self.foreign_unit_adjacent(slot, x, y) {
                self.stash_garrison(c);
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
    fn fortifying_completes_with_the_next_turns_moves_spent() {
        // CIV.EXE 0x4239: flag 0x04 becomes 0x08 and the moves go to 0
        let (mut g, id) = militia_before_mountains(1, 3);
        let slot = g.unit(id).owner;
        g.unit_mut(id).order = crate::game::UnitOrder::Fortify;
        g.reset_moves(slot);
        assert_eq!(g.unit(id).order, crate::game::UnitOrder::Fortified);
        assert_eq!(g.unit(id).moves_left, 0);
        g.reset_moves(slot);
        assert_eq!(g.unit(id).moves_left, 3);
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
            // landing on a Carrier ends the aircraft's turn (CIV.EXE 0x6cea)
            assert_eq!(g.unit(f).moves_left, 0);
        }
        let f = g.add_unit(slot, unit_ids::FIGHTER, x + 1, y + 1);
        assert_eq!(g.try_move(f, Dir8::N), Ok(MoveOutcome::Moved));
        assert_eq!(g.unit(f).moves_left, g.unit(f).full_moves() - 3, "flying on over the sea");
        assert_eq!(g.cargo_count(carrier), 8);
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
            g.unit_mut(u).carried_by = Some(ship);
            g.unit_mut(u).order = UnitOrder::Sentry;
        }
        assert_eq!(g.try_move(ship, Dir8::W), Ok(MoveOutcome::Moved));
        assert_eq!(g.unit(awake).carried_by, None);
        // in port the units can be given orders like any other
        g.unit_mut(awake).order = UnitOrder::None;
        g.unit_mut(ship).moves_left = 3;
        assert_eq!(g.try_move(ship, Dir8::E), Ok(MoveOutcome::Moved));
        assert_eq!(g.unit(awake).pos(), (x, y));
        assert_eq!((g.unit(asleep).pos(), g.unit(asleep).carried_by), ((x + 1, y), Some(ship)));
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
    use crate::game::{Game, GameSetup, MoveOutcome, RuleError};
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
    fn coming_ashore_ends_the_move_and_a_reached_goto_is_dropped() {
        // CIV.EXE 0x687d: from the sea onto land the moves are gone
        let (mut g, id) = open_field();
        let slot = g.human_slot;
        g.map.get_mut(41, 25).unwrap().terrain = Terrain::Ocean;
        let ship = g.add_unit(slot, unit_ids::TRIREME, 41, 25);
        let l = g.add_unit(slot, unit_ids::LEGION, 41, 25);
        g.unit_mut(l).carried_by = Some(ship);
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
}
