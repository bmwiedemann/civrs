//! Combat, nuclear attacks, city capture, civil war and civilisation destruction.
//!
//! Strengths are expressed x8 so that the integer modifiers (x1.5, x2, x3, terrain halves)
//! stay exact. Resolution rolls `rng.below(A)` against `rng.below(D)` until one side is
//! strictly higher (Civ1's single-round combat).

use super::map::{tile_flags, CityId, Map, Slot, UnitId};
use super::{EndReason, Game, GameEvent, MoveOutcome, Player, Unit, UnitOrder};
use crate::rules::{self, improvement_ids as imp, unit_ids, Govt};

impl Game {
    /// Attack strength x8 (veteran bonus included).
    pub fn attack_strength(&self, u: &Unit) -> i32 {
        let mut a = u.info().att as i32 * 8;
        if u.veteran {
            a += a / 2;
        }
        a
    }

    /// Defence strength x8 including terrain, fortification, fortress, walls and veteran status.
    ///
    /// `attacker` selects the walls rule: City Walls triple the defence (terrain included,
    /// instead of the fortification bonus) of every unit but aircraft, except against the
    /// units with attack 12, Artillery and Bombers (CIV.EXE 0x198ca).
    pub fn defense_strength(&self, u: &Unit, attacker: Option<&Unit>) -> i32 {
        let t = self.map.tile(u.x, u.y);
        let mut d = u.info().def as i32 * 8;
        if u.veteran {
            d += d / 2;
        }
        let walls = t.city.and_then(|c| self.city_opt(c)).map(|c| self.city_has_walls(c)).unwrap_or(false)
            && !u.is_air()
            && attacker.map(|a| a.info().att != 12).unwrap_or(true);
        if walls {
            return d * t.terrain.info().defense as i32 / 2 * 3;
        }
        // terrain multiplier in halves (Grassland 2 = x1, Forest 3 = x1.5, Hills 4 = x2 ...)
        if u.is_land() {
            d = d * t.terrain.info().defense as i32 / 2;
        }
        let in_city = t.city.is_some();
        let fortified = matches!(u.order, UnitOrder::Fortified | UnitOrder::Fortify);
        if (fortified || in_city) && u.is_land() {
            d += d / 2;
        }
        if t.has(tile_flags::FORTRESS) && !in_city {
            d *= 2;
        }
        d
    }

    /// The odds CIV.EXE's `combat` (0x19724) returns with `resolve` = 0, as the computer's
    /// move scoring asks for them (0x1826e): attack * 8 / (defence + 1) with the routine's
    /// strengths, x8 fixed point. Attack: attack * 8, for a barbarian attacker x (difficulty
    /// + 1) / 4 against the human and / 2 against others, none against a civilization with
    /// one city and halved against a Palace; +50% veteran; x moves / 3 below one full move.
    /// Defence: defence x terrain (in halves) x 8 in a fortress (tile bit 0x20), x 6 fortified,
    /// else x 4; aircraft and ships defence * 8; City Walls defence x terrain x 12 unless the
    /// attacker has attack 12 or the defender flies; +50% veteran. The difficulty factors of the
    /// resolution (0x199c2..) are not part of the odds.
    pub fn combat_odds(&self, attacker: UnitId, defender: UnitId) -> i32 {
        let a = self.unit(attacker);
        let d = self.unit(defender);
        let t = self.map.tile(d.x, d.y);
        let terrain = t.terrain.info().defense as i32;
        let def = d.info().def as i32;
        let mut att = a.info().att as i32 * 8;
        let mut dv = if t.has(tile_flags::FORTRESS) {
            def * terrain * 8
        } else if d.order == UnitOrder::Fortified {
            def * terrain * 12 / 2
        } else {
            def * terrain * 4
        };
        if a.owner == 0 {
            att = if d.owner == self.human_slot { att * (self.setup.difficulty as i32 + 1) / 4 } else { att / 2 };
        }
        if !d.is_land() {
            dv = def * 8;
        }
        if let Some(c) = t.city.and_then(|c| self.city_opt(c)) {
            if self.city_has_walls(c) && a.info().att != 12 && !d.is_air() {
                dv = terrain * def * 12;
            }
            if a.owner == 0 {
                if self.cities_of(d.owner).len() == 1 {
                    att = 0;
                }
                if c.has_improvement(imp::PALACE) {
                    att /= 2;
                }
            }
        }
        if a.veteran {
            att += att / 2;
        }
        if d.veteran {
            dv += dv / 2;
        }
        if a.moves_left < 3 {
            att = att * a.moves_left as i32 / 3;
        }
        att * 8 / (dv + 1)
    }

    /// City Walls (the Great Wall is a diplomatic wonder in CIV.EXE, not walls).
    pub fn city_has_walls(&self, city: &super::City) -> bool {
        city.has_improvement(imp::CITY_WALLS)
    }

    /// Chance in percent that `attacker` beats the best defender on (x,y) (0 when no defender).
    pub fn win_chance(&self, attacker: &Unit, x: i32, y: i32) -> i32 {
        let Some(def_id) = self.best_defender(x, y, Some(attacker)) else { return 100 };
        let d = self.defense_strength(self.unit(def_id), Some(attacker)).max(1);
        let a = self.attack_strength(attacker).max(0);
        if a + d == 0 {
            return 0;
        }
        a * 100 / (a + d)
    }

    /// Resolve an attack by unit `id` on tile (x,y). Returns the outcome from the attacker's view.
    pub fn attack(&mut self, id: UnitId, x: i32, y: i32) -> MoveOutcome {
        let attacker = self.unit(id).clone();
        if attacker.kind == unit_ids::NUCLEAR {
            return self.nuke(id, x, y);
        }
        let targets = self.units_at(x, y);
        if targets.is_empty() {
            return MoveOutcome::Blocked;
        }
        let Some(def_id) = self.best_defender(x, y, Some(&attacker)) else { return MoveOutcome::Blocked };
        let defender = self.unit(def_id).clone();
        let mut a = self.attack_strength(&attacker);
        let d = self.defense_strength(&defender, Some(&attacker)).max(1) as u32;
        // barbarian attackers (CIV.EXE 0x1981e, 0x19908; applied there before the veteran
        // bonus): (difficulty + 1) quarters against the human, half against the computer; at a
        // city nothing against its owner's only city, half again against a Palace, and the
        // attack must win a second roll (0x19a4b)
        let mut second_roll = false;
        if attacker.owner == 0 {
            a = if defender.owner == self.human_slot { a * (self.setup.difficulty as i32 + 1) / 4 } else { a / 2 };
            if let Some(c) = self.city_at(x, y) {
                if self.cities_of(defender.owner).len() == 1 {
                    a = 0;
                }
                if self.city(c).has_improvement(imp::PALACE) {
                    a /= 2;
                }
                second_roll = true;
            }
        }
        let won = if attacker.owner == 0 && a <= 0 {
            false
        } else {
            let a = a.max(1) as u32;
            let won = loop {
                let ra = self.rng.below(a);
                let rd = self.rng.below(d);
                if ra > rd {
                    break true;
                }
                if rd > ra {
                    break false;
                }
            };
            won && !(second_roll && {
                let rd = self.rng.below(d);
                self.rng.below(a) < rd
            })
        };
        self.last_combat = Some(((attacker.x, attacker.y), (x, y), won));
        self.fight_diplomacy(attacker.owner, defender.owner);
        // a computer city that is attacked, whoever wins, goes on alert (city status 0x10,
        // CIV.EXE 0x6114..0x6141): its next turn re-chooses production and buys
        if let Some(c) = self.city_at(x, y) {
            if self.city(c).owner != self.human_slot {
                self.city_mut(c).auto_build = true;
            }
        }
        // an attack costs one full move, so Armor and Mech. Inf. may attack three times; a
        // Bomber's attack ends its turn (READ.ME §2, CIV.EXE 0x60e4)
        self.unit_mut(id).moves_left = if attacker.kind == unit_ids::BOMBER { 0 } else { attacker.moves_left.saturating_sub(3) };
        if won {
            // beating a barbarian Diplomat (the leader) pays 100$ ransom, "*LEADER" for the
            // human (CIV.EXE 0x1a018); the stack then dies as usual
            if defender.owner == 0 && defender.kind == unit_ids::DIPLOMAT {
                self.player_mut(attacker.owner).gold += 100;
                self.push_event(GameEvent::BarbarianLeaderCaptured { slot: attacker.owner, gold: 100 });
            }
            let t = self.map.tile(x, y);
            let stack_safe = t.city.is_some() || t.has(tile_flags::FORTRESS);
            let victims: Vec<UnitId> = if stack_safe { vec![def_id] } else { self.units_at(x, y) };
            let mut losers: Vec<Slot> = Vec::new();
            for v in victims {
                // a ship's cargo goes down with it (`remove_unit`)
                let Some(vu) = self.unit_opt(v).cloned() else { continue };
                self.players[vu.owner as usize].stats.units_lost += 1;
                self.push_event(GameEvent::UnitLost { slot: vu.owner, kind: vu.kind, at: (x, y), reason: "destroyed".into() });
                self.remove_unit(v);
                if !losers.contains(&vu.owner) {
                    losers.push(vu.owner);
                }
            }
            self.players[attacker.owner as usize].stats.units_killed += 1;
            if let Some(c) = t.city {
                if let Some(city) = self.city_opt(c) {
                    if !self.city_has_walls(city) && city.size > 1 {
                        // only the size drops (CIV.EXE 0x618a): the extra worker goes at the
                        // city's next turn, the human's with "Population decrease" (0xd70d)
                        self.city_mut(c).size -= 1;
                        self.push_event(GameEvent::CityShrank(c));
                    }
                }
            }
            if !attacker.veteran && self.rng.chance(50) {
                self.unit_mut(id).veteran = true;
                self.push_event(GameEvent::UnitVeteran(id));
            }
            self.refresh_vision(attacker.owner);
            for l in losers {
                self.refresh_vision(l);
                self.check_destroyed(l);
            }
            MoveOutcome::Attacked { won: true, at: (x, y), defender: defender.kind }
        } else {
            self.players[attacker.owner as usize].stats.units_lost += 1;
            self.players[defender.owner as usize].stats.units_killed += 1;
            self.push_event(GameEvent::UnitLost { slot: attacker.owner, kind: attacker.kind, at: (attacker.x, attacker.y), reason: "lost in combat".into() });
            self.remove_unit(id);
            if !defender.veteran && self.rng.chance(50) {
                self.unit_mut(def_id).veteran = true;
                self.push_event(GameEvent::UnitVeteran(def_id));
            }
            self.refresh_vision(attacker.owner);
            self.check_destroyed(attacker.owner);
            MoveOutcome::Attacked { won: false, at: (x, y), defender: defender.kind }
        }
    }

    /// Is tile (x,y) protected by an SDI Defense (a city within 3 tiles that has one, owned by
    /// anyone other than `attacker`)?
    pub fn sdi_protects(&self, attacker: Slot, x: i32, y: i32) -> bool {
        self.all_cities().any(|c| c.owner != attacker && c.has_improvement(imp::SDI_DEFENSE) && Map::dist((c.x, c.y), (x, y)) <= 3)
    }

    /// Nuclear attack by unit `id` on tile (x,y): every unit in the 3x3 area dies, a city there
    /// loses half its population, land tiles are polluted. SDI Defense within 3 tiles of the
    /// target shoots the missile down (the nuclear unit is lost either way).
    pub fn nuke(&mut self, id: UnitId, x: i32, y: i32) -> MoveOutcome {
        let attacker = self.unit(id).clone();
        let by = attacker.owner;
        self.unit_mut(id).moves_left = 0;
        if self.sdi_protects(by, x, y) {
            self.push_event(GameEvent::UnitLost { slot: by, kind: attacker.kind, at: (x, y), reason: "destroyed by SDI Defense".into() });
            self.remove_unit(id);
            self.refresh_vision(by);
            return MoveOutcome::Attacked { won: false, at: (x, y), defender: unit_ids::NUCLEAR };
        }
        self.remove_unit(id);
        let mut losers: Vec<Slot> = Vec::new();
        for yy in (y - 1)..=(y + 1) {
            for xx in (x - 1)..=(x + 1) {
                if !Map::in_bounds(xx, yy) {
                    continue;
                }
                for v in self.units_at(xx, yy) {
                    let Some(vu) = self.unit_opt(v).cloned() else { continue };
                    self.players[vu.owner as usize].stats.units_lost += 1;
                    self.push_event(GameEvent::UnitLost { slot: vu.owner, kind: vu.kind, at: (xx, yy), reason: "nuclear attack".into() });
                    self.remove_unit(v);
                    if !losers.contains(&vu.owner) {
                        losers.push(vu.owner);
                    }
                }
                let t = self.map.tile(xx, yy);
                if t.is_land() && t.city.is_none() && !t.has(tile_flags::POLLUTION) {
                    self.map.get_mut(xx, yy).unwrap().set(tile_flags::POLLUTION, true);
                    self.polluted_tiles += 1;
                }
            }
        }
        let city = self.city_at(x, y);
        if let Some(c) = city {
            let size = self.city(c).size;
            self.city_mut(c).size = (size / 2).max(1);
            self.auto_assign(c);
            self.push_event(GameEvent::CityShrank(c));
            let owner = self.city(c).owner;
            if !losers.contains(&owner) {
                losers.push(owner);
            }
        }
        self.last_combat = Some(((attacker.x, attacker.y), (x, y), true));
        // the victims hold a vendetta against the attacker and remember the bomb (0x1a5ba)
        for l in losers.iter().copied().filter(|l| *l != by) {
            self.raise_flag(l, by, super::diplomacy::flags::VENDETTA | super::diplomacy::flags::NUCLEAR);
        }
        self.push_event(GameEvent::NuclearStrike { at: (x, y), by, city });
        self.refresh_vision(by);
        for l in losers {
            self.refresh_vision(l);
            self.check_destroyed(l);
        }
        MoveOutcome::Attacked { won: true, at: (x, y), defender: unit_ids::NUCLEAR }
    }

    /// Gold taken with a city (CIV.EXE 0x143fd): `owner's gold * size / (all the owner's
    /// citizens + 1)`.
    pub fn plunder_value(&self, c: CityId) -> i32 {
        let city = self.city(c);
        let total: i32 = self.all_cities().filter(|cc| cc.owner == city.owner).map(|cc| cc.size as i32).sum();
        let gold = self.player(city.owner).gold.max(0);
        (gold * city.size as i32 / (total + 1)).clamp(0, gold)
    }

    /// Ground unit `id` enters undefended enemy city `c`.
    pub fn capture_city(&mut self, id: UnitId, c: CityId) -> MoveOutcome {
        // the captor next to the city has let its garrison out (unit_look 0x828f), so a
        // stashed unit is never captured with the record
        self.unstash_garrison(c);
        let attacker = self.unit(id).clone();
        let city = self.city(c).clone();
        let from = city.owner;
        let by = attacker.owner;
        let was_capital = self.player(from).capital == Some(c);
        // the leaders remember (capture_city 0x13da6, 0x13e5c): a city taken by the human
        // marks its old owner's last contact -2, one taken from the human makes the captor
        // ready to talk (-1)
        if by == self.human_slot && from != 0 {
            self.player_mut(from).last_contact = -2;
        }
        if from == self.human_slot && by != 0 {
            self.player_mut(by).last_contact = -1;
        }
        if city.size <= 1 || (by == 0 && attacker.info().att <= 2) {
            // size-1 cities are destroyed, and so is any city a barbarian with attack 2 or less
            // takes (CIV.EXE 0x13e89; ds:0x28bc is only set by a Diplomat's incite)
            let name = city.name.clone();
            self.remove_city(c);
            self.push_event(GameEvent::CityDestroyed { name, at: (city.x, city.y), by, from });
            self.place_unit(id, city.x, city.y, 3);
            self.players[from as usize].stats.cities_lost += 1;
            if was_capital {
                self.lose_capital(from);
                self.civil_war(from, (city.x, city.y));
            }
            self.refresh_vision(by);
            self.refresh_vision(from);
            self.city_lost_check(from, by);
            return MoveOutcome::DestroyedCity;
        }
        // the captor's city-site goals around it are dropped (0x13faf)
        self.remove_goals_near(by, super::goals::goal::SETTLE, city.x, city.y, 4);
        // the plunder leaves the loser even when the barbarians take the city (0x13f52)
        let gold = self.plunder_value(c);
        self.player_mut(from).gold -= gold;
        if by != 0 {
            self.player_mut(by).gold += gold;
        }
        // steal an advance (steal_advance 0x14468): a computer captor takes the one it values
        // most, the human picks one from a menu
        let mut adv = None;
        if self.is_human(by) {
            let options = crate::ai::tech_ai::stealable(self, by, from);
            if !options.is_empty() {
                self.push_event(GameEvent::NeedsDecision(super::Decision::StealAdvance { slot: by, from, options }));
            }
        } else if let Some(a) = crate::ai::tech_ai::best_to_steal(self, by, from) {
            self.grant_advance(by, a, super::science::AdvanceSource::From(from));
            adv = Some(a);
            self.push_event(GameEvent::AdvanceAcquired { slot: by, adv: a, source: "conquest".into() });
        }
        // improvements: each has a 1/3 chance of being destroyed (wonders survive); Palace is lost
        let doomed: Vec<u8> = city.improvement_list().into_iter().filter(|i| *i == imp::PALACE || self.rng.chance(33)).collect();
        {
            let cm = self.city_mut(c);
            cm.owner = by;
            cm.size -= 1;
            cm.disorder = false;
            cm.disorder_turns = 0;
            cm.celebrating = false;
            // CIV.EXE 0x13ef0/0x13fb7 clear (among others) the AUTO and garrison-out bits
            cm.auto_build = false;
            cm.garrison_out = false;
            cm.trade_routes.clear();
            cm.specialists.clear();
            cm.shields = 0;
            cm.caravan_shields = 0;
            cm.production = super::Production::Unit(unit_ids::MILITIA);
            for i in doomed {
                cm.remove_improvement(i);
            }
        }
        // units supported by the city are disbanded (Civ1), units of the loser inside are gone
        let homed: Vec<UnitId> = self.all_units().filter(|u| (u.home == Some(c) && u.owner == from) || ((u.x, u.y) == (city.x, city.y) && u.owner == from)).map(|u| u.id).collect();
        for h in homed {
            self.remove_unit(h);
        }
        // other cities' trade routes to this city are cancelled
        for other in self.cities.iter_mut().flatten() {
            other.trade_routes.retain(|r| *r != c);
        }
        self.place_unit(id, city.x, city.y, 3);
        self.unit_mut(id).order = UnitOrder::None;
        self.auto_assign(c);
        self.rechoose_after_capture(by);
        self.players[by as usize].stats.cities_captured += 1;
        self.players[from as usize].stats.cities_lost += 1;
        self.push_event(GameEvent::CityCaptured { city: c, by, from, gold, adv });
        if was_capital {
            self.lose_capital(from);
            self.civil_war(from, (city.x, city.y));
        }
        self.refresh_vision(by);
        self.refresh_vision(from);
        self.city_lost_check(from, by);
        MoveOutcome::CapturedCity(c)
    }

    /// Bookkeeping when a civilisation loses its capital: no palace, spaceship cancelled
    /// (CIV.EXE 0x13e43: the captured city had the Palace).
    fn lose_capital(&mut self, slot: Slot) {
        self.player_mut(slot).capital = None;
        self.cancel_spaceship(slot);
    }

    /// Civil war when a capital falls: if the loser still has at least 6 cities, the half of
    /// them farthest from the old capital defects to a new civilisation in a free slot.
    pub fn civil_war_check(&mut self, from: Slot, by: Slot) {
        if from == 0 || !self.player(from).in_game || !self.player(from).alive {
            return;
        }
        let cities = self.cities_of(from);
        if cities.len() < 6 {
            return;
        }
        // old capital position is unknown here: use the captor's city nearest to the loser's
        // remaining cities as the centre the rebellion spreads away from
        let centre = self
            .all_cities()
            .filter(|c| c.owner == by)
            .map(|c| (c.x, c.y))
            .min_by_key(|p| cities.iter().map(|c| Map::dist(*p, self.city(*c).pos())).sum::<i32>())
            .unwrap_or_else(|| self.city(cities[0]).pos());
        self.civil_war(from, centre);
    }

    /// Split the civilisation `from`: the half of its cities farthest from `centre` defects to a
    /// new civilisation in a free slot (no-op when fewer than 6 cities or no slot is free).
    pub fn civil_war(&mut self, from: Slot, centre: (i32, i32)) {
        if from == 0 || !self.player(from).in_game || !self.player(from).alive {
            return;
        }
        let mut cities = self.cities_of(from);
        if cities.len() < 6 {
            return;
        }
        let Some(new_slot) = (1..8u8).find(|s| !self.players[*s as usize].in_game) else { return };
        cities.sort_by_key(|c| std::cmp::Reverse(Map::dist(centre, self.city(*c).pos())));
        let n = cities.len() / 2;
        let rebels: Vec<CityId> = cities[..n].to_vec();
        // new civilisation from the free slot's pair
        let pair = rules::civs_for_slot(new_slot);
        let used: Vec<u8> = self.players.iter().filter(|p| p.in_game).map(|p| p.civ).collect();
        let civ = if !used.contains(&pair[1]) && self.rng.chance(50) { pair[1] } else if !used.contains(&pair[0]) { pair[0] } else { pair[1] };
        let old = self.player(from).clone();
        let mut p = Player::new(new_slot, civ, false);
        p.in_game = true;
        p.alive = true;
        p.govt = Govt::Despotism;
        p.anarchy_turns = 0;
        p.advances = old.advances;
        p.future_techs = old.future_techs;
        p.gold = old.gold / 2;
        p.tax = 50;
        p.lux = 0;
        self.player_mut(from).gold -= old.gold / 2;
        // the new civilization starts with clean diplomacy words both ways (o 0x4a7c), at war
        // with the old one, which it holds a vendetta against, and talked to 8 turns ago
        // (civil war o 0x14e61..0x14e86)
        p.last_contact = self.turn as i32 - 8;
        for s in 0..8usize {
            p.diplo[s] = 0;
            self.players[s].diplo[new_slot as usize] = 0;
        }
        p.diplo[from as usize] = super::diplomacy::flags::CONTACT | super::diplomacy::flags::VENDETTA;
        self.players[from as usize].diplo[new_slot as usize] = super::diplomacy::flags::CONTACT;
        self.players[new_slot as usize] = p;
        // transfer the rebel cities (the farthest becomes the new capital) and their units
        for (i, c) in rebels.iter().enumerate() {
            let (x, y) = self.city(*c).pos();
            {
                let cm = self.city_mut(*c);
                cm.owner = new_slot;
                cm.disorder = false;
                cm.celebrating = false;
                cm.trade_routes.clear();
                if i == 0 {
                    cm.add_improvement(imp::PALACE);
                }
            }
            if i == 0 {
                self.players[new_slot as usize].capital = Some(*c);
            }
            let moving: Vec<UnitId> = self.all_units().filter(|u| u.owner == from && (u.home == Some(*c) || (u.x, u.y) == (x, y))).map(|u| u.id).collect();
            for id in moving {
                // re-created for the new civilization: the counters move along
                let kind = self.unit(id).kind as usize % 28;
                self.players[from as usize].unit_counter[kind] -= 1;
                self.players[new_slot as usize].unit_counter[kind] += 1;
                self.unit_mut(id).owner = new_slot;
            }
            // trade routes of the loser pointing at rebel cities are cut
            for other in self.cities.iter_mut().flatten() {
                if other.owner == from {
                    other.trade_routes.retain(|r| r != c);
                }
            }
            self.auto_assign(*c);
        }
        copy_explored(&mut self.map, new_slot, from);
        self.refresh_vision(new_slot);
        self.refresh_vision(from);
        self.push_event(GameEvent::CivilWar { slot: from, new_slot });
    }

    /// Mark a civilisation destroyed when it has no cities and no settlers left (the computer's
    /// turn, 0x15f98, and the loss of units).
    pub fn check_destroyed(&mut self, slot: Slot) {
        if slot == 0 || !self.player(slot).in_game || !self.player(slot).alive {
            return;
        }
        let has_city = self.all_cities().any(|c| c.owner == slot);
        let has_settlers = self.all_units().any(|u| u.owner == slot && u.kind == unit_ids::SETTLERS);
        if has_city || has_settlers {
            return;
        }
        // whoever has just fought them is the destroyer when known
        let by = self.last_combat.and_then(|(a, _, _)| self.owner_at(a.0, a.1)).filter(|b| *b != slot).unwrap_or(0);
        self.destroy_civ(slot, by);
    }

    /// A city of `slot` was lost to `by` (captured, destroyed, defected, incited): a computer
    /// civilization without cities is destroyed even if it still has units (`civ_destroyed`
    /// o5 0x4fe1, called by 0x61b5, 0xd159, 0xe041 and 0x14356); the human keeps the
    /// cities-and-settlers rule.
    pub fn city_lost_check(&mut self, slot: Slot, by: Slot) {
        if slot == 0 || !self.player(slot).in_game || !self.player(slot).alive {
            return;
        }
        if slot == self.human_slot {
            self.check_destroyed(slot);
            return;
        }
        if self.all_cities().any(|c| c.owner == slot) {
            return;
        }
        self.destroy_civ(slot, by);
    }

    /// `civ_destroyed` (o5 0x4fe1): the message, all units deleted, then `civ_init` brings in
    /// the slot's other tribe when it can (`respawn_civ`); otherwise the slot leaves the game
    /// and the human alone left has conquered the world.
    fn destroy_civ(&mut self, slot: Slot, by: Slot) {
        self.player_mut(slot).alive = false;
        self.player_mut(slot).destroyed_turn = Some(self.turn);
        let ids = self.units_of(slot);
        for id in ids {
            self.remove_unit(id);
        }
        let (adjective, civ) = (self.player(slot).adjective().to_string(), self.player(slot).civ);
        self.push_event(GameEvent::CivDestroyed { slot, by, adjective, civ });
        if self.active_unit.map(|a| self.unit_opt(a).is_none()).unwrap_or(false) {
            self.active_unit = None;
        }
        if slot == self.human_slot {
            if self.ended.is_none() {
                self.ended = Some(EndReason::Destroyed);
                self.push_event(GameEvent::GameEnded(EndReason::Destroyed));
            }
            return;
        }
        if self.respawn_civ(slot) {
            return;
        }
        if self.alive_slots().len() == 1 && self.player(self.human_slot).alive && self.ended.is_none() {
            self.ended = Some(EndReason::Conquest);
            self.push_event(GameEvent::GameEnded(EndReason::Conquest));
        }
    }

    /// `civ_init` (o5 0x493c) after turn 0 for a destroyed computer civilization: the slot's
    /// other tribe (civ ^ 8) starts afresh, once per slot (ds:0xdf1e) and only up to turn
    /// 350 - 50 * difficulty. It gets a new start square (random squares (8 + random(64),
    /// 8 + random(34)) until the k-th is land, has a site value of at least 12 - k/32, lies
    /// 10 - k/64 from the nearest city, on a continent with at least 32 - turn/16 - k/64 site
    /// squares and, after 1 AD, no city, and has no hut; 2000 tries), one Settlers there, each
    /// advance the human knows and a computer civilization found with probability 1/2
    /// (civrs keeps no first-discoverer table: some computer civilization knows it), Despotism,
    /// no gold, science (civilized trait + 3) tenths, tax the rest but one tenth, and clean
    /// diplomacy both ways. Barbarian units within 8 squares are deleted.
    pub fn respawn_civ(&mut self, slot: Slot) -> bool {
        let old = self.player(slot).clone();
        let civ = old.civ ^ 8;
        if civ == old.start_civ || old.human || self.turn as i32 > 350 - 50 * self.setup.difficulty as i32 {
            return false;
        }
        let Some((x, y)) = self.respawn_square() else { return false };
        let mut p = Player::new(slot, civ, false);
        p.in_game = true;
        p.alive = true;
        p.govt = Govt::Despotism;
        p.start_civ = old.start_civ;
        p.last_contact = -1;
        let sci = (p.civ().civilized as i32 + 3).clamp(0, 9);
        p.tax = (10 * (9 - sci)) as u8;
        p.lux = 10;
        for s in 0..8usize {
            self.players[s].diplo[slot as usize] = 0;
        }
        self.players[slot as usize] = p;
        // ai_clear_persistent_goals (0x189b9) and the continent strategy table (o5 0x49bb)
        self.ai_memory.persistent[slot as usize] = [super::goals::Goal::FREE; super::goals::PERSISTENT_GOALS];
        self.ai_memory.policies[slot as usize] = None;
        let human = self.human_slot;
        for a in 0..rules::advances::REAL_ADVANCE_COUNT as u8 {
            let found_by_ai = (1..8u8).any(|s| s != human && s != slot && self.player(s).knows(a));
            if self.player(human).knows(a) && found_by_ai && self.rng.below(2) == 0 {
                self.grant_advance(slot, a, super::science::AdvanceSource::Discovered);
            }
        }
        self.map.explore(slot, x, y, 1);
        self.add_unit(slot, unit_ids::SETTLERS, x, y);
        let barbarians: Vec<UnitId> = self.all_units().filter(|u| u.owner == 0 && Map::civ_distance((u.x, u.y), (x, y)) <= 8).map(|u| u.id).collect();
        for id in barbarians {
            self.remove_unit(id);
        }
        // civ_init ends by zeroing the unit counters (o5 0x4fc2)
        self.players[slot as usize].unit_counter = [0; 28];
        self.players[slot as usize].units_lost_by_type = [0; 28];
        self.refresh_vision(slot);
        true
    }

    /// The start-square search of `civ_init` after turn 0 (o5 0x4bd3..0x4d8b); `None` after
    /// 2000 tries.
    fn respawn_square(&mut self) -> Option<(i32, i32)> {
        use crate::rules::Terrain;
        let mut sites = [0i32; 16];
        let mut cities = [0i32; 16];
        for y in 2..48 {
            for x in 2..78 {
                let t = self.map.tile(x, y);
                if t.is_land() && matches!(t.terrain, Terrain::Plains | Terrain::Grassland | Terrain::River) {
                    sites[t.continent as usize & 15] += 1;
                }
            }
        }
        for c in self.all_cities().filter(|c| c.owner != 0) {
            cities[self.map.tile(c.x, c.y).continent as usize & 15] += 1;
        }
        let turn = self.turn as i32;
        let mut k = 0;
        loop {
            let x = 8 + self.rng.below(64) as i32;
            let y = 8 + self.rng.below(34) as i32;
            let near = self.nearest_city_exe(x, y);
            let near = if near.0.is_none() { 30 } else { near.1 };
            let site = self.map.site_at(x, y) as i32;
            k += 1;
            if k > 2000 {
                return None;
            }
            let t = self.map.tile(x, y);
            let cont = t.continent as usize & 15;
            if !t.is_land() || 12 - k / 32 > site || near < 10 - k / 64 || sites[cont] < 32 - (turn / 16 + k / 64) {
                continue;
            }
            if self.year() > 0 && cities[cont] != 0 {
                continue;
            }
            if t.has(tile_flags::HUT) {
                continue;
            }
            return Some((x, y));
        }
    }
}

/// Copy another slot's explored area to a slot (new civilisation born from a civil war).
fn copy_explored(map: &mut Map, slot: Slot, from: Slot) {
    for t in map.tiles.iter_mut() {
        if t.explored_by(from) {
            t.explored |= 1 << slot;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{tile_flags, GameSetup, Relation};
    use crate::rules::{improvement_ids as imp, unit_ids, Terrain};

    #[test]
    fn a_destroyed_computer_civ_returns_once_as_its_other_tribe() {
        let mut g = Game::new(GameSetup { seed: 3, ..Default::default() }, None);
        g.turn = 10;
        let s = (1..8u8).find(|s| *s != g.human_slot && g.player(*s).alive).unwrap();
        let first = g.player(s).civ;
        for id in g.units_of(s) {
            g.remove_unit(id);
        }
        g.check_destroyed(s);
        assert!(g.player(s).alive, "civ_init brings in the slot's other tribe");
        assert_eq!(g.player(s).civ, first ^ 8);
        assert_eq!(g.units_of(s).len(), 1);
        assert_eq!(g.unit(g.units_of(s)[0]).kind, unit_ids::SETTLERS);
        // the second time the slot is gone for good
        for id in g.units_of(s) {
            g.remove_unit(id);
        }
        g.check_destroyed(s);
        assert!(!g.player(s).alive);
    }

    #[test]
    fn no_new_civ_after_turn_350_less_50_per_level() {
        let mut g = Game::new(GameSetup { seed: 3, ..Default::default() }, None);
        g.turn = 351;
        let s = (1..8u8).find(|s| *s != g.human_slot && g.player(*s).alive).unwrap();
        for id in g.units_of(s) {
            g.remove_unit(id);
        }
        g.check_destroyed(s);
        assert!(!g.player(s).alive);
    }

    fn flat_game() -> Game {
        flat_game_with(7)
    }

    fn flat_game_with(num_civs: u8) -> Game {
        let mut g = Game::new(GameSetup { seed: 7, num_civs, ..Default::default() }, None);
        // slot 2 is used as the opponent in these tests whether or not the setup picked it
        g.players[2].in_game = true;
        g.players[2].alive = true;
        // clear a flat grassland arena with nothing on it
        for y in 5..30 {
            for x in 5..30 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Grassland;
                t.flags = 0;
                t.special = false;
                t.shield = false;
            }
        }
        let ids: Vec<UnitId> = g.all_units().filter(|u| (5..30).contains(&u.x) && (5..30).contains(&u.y)).map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        g.set_relation(1, 2, Relation::War);
        g
    }

    #[test]
    fn attack_3_vs_2_on_grassland_wins_55_to_70_percent() {
        let mut g = flat_game();
        let mut wins = 0;
        let trials = 10_000;
        for _ in 0..trials {
            let a = g.add_unit(1, unit_ids::LEGION, 10, 10);
            let d = g.add_unit(2, unit_ids::PHALANX, 11, 10);
            assert_eq!(g.defense_strength(g.unit(d), Some(g.unit(a))), 16);
            match g.attack(a, 11, 10) {
                MoveOutcome::Attacked { won, .. } => {
                    if won {
                        wins += 1;
                    }
                }
                o => panic!("unexpected {o:?}"),
            }
            for id in g.units_at(10, 10).into_iter().chain(g.units_at(11, 10)) {
                g.remove_unit(id);
            }
            g.events.clear();
        }
        let pct = wins * 100 / trials;
        assert!((55..=70).contains(&pct), "attacker won {pct}%");
    }

    #[test]
    fn walls_triple_defence_except_against_artillery_and_bombers() {
        let mut g = flat_game();
        let c = g.add_city(2, "Wallville".into(), 20, 20);
        let d = g.add_unit(2, unit_ids::PHALANX, 20, 20);
        let a = g.add_unit(1, unit_ids::LEGION, 21, 20);
        let plain = g.defense_strength(g.unit(d), Some(g.unit(a)));
        assert_eq!(plain, 24, "in-city bonus x1.5");
        g.city_mut(c).add_improvement(imp::CITY_WALLS);
        // x3 of the base defence, in place of the in-city bonus
        assert_eq!(g.defense_strength(g.unit(d), Some(g.unit(a))), 48);
        let ship = g.add_unit(1, unit_ids::FRIGATE, 22, 22);
        assert_eq!(g.defense_strength(g.unit(d), Some(g.unit(ship))), 48);
        for k in [unit_ids::ARTILLERY, unit_ids::BOMBER] {
            let ignores = g.add_unit(1, k, 21, 21);
            assert_eq!(g.defense_strength(g.unit(d), Some(g.unit(ignores))), plain);
        }
        // fortress doubles outside cities
        g.map.get_mut(12, 12).unwrap().set(tile_flags::FORTRESS, true);
        let f = g.add_unit(2, unit_ids::PHALANX, 12, 12);
        assert_eq!(g.defense_strength(g.unit(f), Some(g.unit(a))), 32);
    }

    #[test]
    fn capturing_a_size_3_city_transfers_ownership_and_plunders() {
        let mut g = flat_game();
        let c = g.add_city(2, "Loot".into(), 20, 20);
        // a second city keeps civ 2 in the game (losing its last one would destroy it)
        g.add_city(2, "Home".into(), 40, 30);
        g.city_mut(c).size = 3;
        g.city_mut(c).add_improvement(imp::TEMPLE);
        g.auto_assign(c);
        g.player_mut(2).gold = 300;
        g.player_mut(2).learn(crate::rules::advance_ids::ALPHABET);
        g.player_mut(2).learn(crate::rules::advance_ids::POTTERY);
        let before = g.player(1).gold;
        let a = g.add_unit(1, unit_ids::LEGION, 19, 20);
        let out = g.try_move(a, crate::game::Dir8::E).unwrap();
        assert_eq!(out, MoveOutcome::CapturedCity(c));
        let city = g.city(c);
        assert_eq!(city.owner, 1);
        assert_eq!(city.size, 2);
        assert!(!city.has_improvement(imp::PALACE));
        // CIV.EXE 0x143fd: gold * size / (citizens + 1) = 300 * 3 / (3 + 1 + 1)
        assert_eq!(g.player(1).gold - before, 180);
        assert_eq!(g.player(2).gold, 120);
        assert_eq!(g.unit(a).pos(), (20, 20));
        assert!(g.events.iter().any(|e| matches!(e, GameEvent::CityCaptured { city, by: 1, from: 2, gold: 180, .. } if *city == c)));
    }

    #[test]
    fn size_one_city_is_destroyed_and_weak_barbarians_raze() {
        let mut g = flat_game();
        let c = g.add_city(2, "Tiny".into(), 20, 20);
        let a = g.add_unit(1, unit_ids::LEGION, 19, 20);
        assert_eq!(g.try_move(a, crate::game::Dir8::E).unwrap(), MoveOutcome::DestroyedCity);
        assert!(g.city_opt(c).is_none());
        // a barbarian with attack 2 or less razes what it takes (CIV.EXE 0x13e89)
        let c2 = g.add_city(2, "Big".into(), 25, 25);
        g.city_mut(c2).size = 5;
        let b = g.add_unit(0, unit_ids::CAVALRY, 24, 25);
        assert_eq!(g.try_move(b, crate::game::Dir8::E).unwrap(), MoveOutcome::DestroyedCity);
        assert!(g.city_opt(c2).is_none());
        // a stronger one keeps the city, and the plunder is lost to both sides
        let c3 = g.add_city(2, "Rich".into(), 15, 15);
        g.city_mut(c3).size = 5;
        g.player_mut(2).gold = 300;
        let l = g.add_unit(0, unit_ids::LEGION, 14, 15);
        assert_eq!(g.try_move(l, crate::game::Dir8::E).unwrap(), MoveOutcome::CapturedCity(c3));
        assert_eq!(g.city(c3).owner, 0);
        assert_eq!(g.city(c3).size, 4);
        assert!(g.player(2).gold < 300);
        assert_eq!(g.player(0).gold, 0);
    }

    #[test]
    fn barbarian_leader_alone_is_ransomed() {
        // the leader fights (defence 1) and pays the ransom only when beaten
        let mut ransomed = 0;
        for seed in 0..20 {
            let mut g = flat_game();
            g.rng = crate::game::Rng::new(seed);
            let l = g.add_unit(0, unit_ids::DIPLOMAT, 11, 10);
            g.unit_mut(l).leader = true;
            let a = g.add_unit(1, unit_ids::LEGION, 10, 10);
            let gold = g.player(1).gold;
            let MoveOutcome::Attacked { won, .. } = g.try_move(a, crate::game::Dir8::E).unwrap() else { panic!("no fight") };
            let paid = g.events.iter().any(|e| matches!(e, GameEvent::BarbarianLeaderCaptured { slot: 1, gold: 100 }));
            assert_eq!(won, paid);
            assert_eq!(g.player(1).gold, gold + if won { 100 } else { 0 });
            assert_eq!(g.unit_opt(l).is_none(), won);
            ransomed += won as usize;
        }
        assert!(ransomed > 0);
    }

    #[test]
    fn nuke_kills_units_halves_city_and_pollutes() {
        let mut g = flat_game();
        let c = g.add_city(2, "Target".into(), 20, 20);
        g.city_mut(c).size = 8;
        g.add_unit(2, unit_ids::MUSKETEERS, 20, 20);
        g.add_unit(2, unit_ids::MUSKETEERS, 21, 21);
        let n = g.add_unit(1, unit_ids::NUCLEAR, 18, 20);
        g.nukes_enabled = true;
        let out = g.nuke(n, 20, 20);
        assert!(matches!(out, MoveOutcome::Attacked { won: true, .. }));
        assert!(g.units_at(20, 20).is_empty() && g.units_at(21, 21).is_empty());
        assert_eq!(g.city(c).size, 4);
        assert!(g.map.tile(19, 19).has(tile_flags::POLLUTION));
        assert!(!g.map.tile(20, 20).has(tile_flags::POLLUTION), "city tile itself is not polluted");
        assert!(g.unit_opt(n).is_none());
        // SDI blocks the next one
        g.city_mut(c).add_improvement(imp::SDI_DEFENSE);
        let n2 = g.add_unit(1, unit_ids::NUCLEAR, 18, 20);
        g.add_unit(2, unit_ids::MUSKETEERS, 20, 20);
        assert!(matches!(g.nuke(n2, 20, 20), MoveOutcome::Attacked { won: false, .. }));
        assert_eq!(g.units_at(20, 20).len(), 1);
        assert_eq!(g.city(c).size, 4);
    }

    #[test]
    fn capital_loss_with_six_cities_splits_the_empire() {
        // 3 civilisations leave slots free for the rebels
        let mut g = flat_game_with(3);
        // slot 2 gets 7 cities in a row; the capital is the first one
        let mut ids = Vec::new();
        for i in 0..7 {
            let c = g.add_city(2, format!("C{i}"), 6 + i * 3, 10);
            g.city_mut(c).size = 3;
            ids.push(c);
        }
        assert_eq!(g.player(2).capital, Some(ids[0]));
        let a = g.add_unit(1, unit_ids::LEGION, 6, 11);
        assert_eq!(g.try_move(a, crate::game::Dir8::N).unwrap(), MoveOutcome::CapturedCity(ids[0]));
        let ev = g.events.iter().find_map(|e| match e {
            GameEvent::CivilWar { slot: 2, new_slot } => Some(*new_slot),
            _ => None,
        });
        let new_slot = ev.expect("civil war event");
        assert!(g.player(new_slot).in_game && g.player(new_slot).alive);
        assert_eq!(g.player(new_slot).relation(2), Relation::War);
        let rebels = g.cities_of(new_slot);
        assert_eq!(rebels.len(), 3);
        // rebels are the farthest cities from the fallen capital
        assert!(rebels.iter().all(|c| g.city(*c).x >= 18));
        assert!(g.player(new_slot).capital.is_some());
        assert_eq!(g.cities_of(2).len(), 3);
    }

    #[test]
    fn a_bombers_attack_ends_its_turn_armor_attacks_again() {
        let mut g = flat_game();
        for (k, left) in [(unit_ids::BOMBER, 0), (unit_ids::ARMOR, 6)] {
            let d = g.add_unit(2, unit_ids::SETTLERS, 20, 20);
            let a = g.add_unit(1, k, 21, 20);
            g.unit_mut(a).moves_left = g.unit(a).full_moves();
            g.attack(a, 20, 20);
            if let Some(u) = g.unit_opt(a) {
                assert_eq!(u.moves_left, left, "{k}");
            }
            if g.unit_opt(d).is_some() {
                g.remove_unit(d);
            }
        }
    }

    /// Slot 2 with its capital at (8, 20) and a size-4 city "Target" at (20, 20), a Diplomat of
    /// slot 1 next to it.
    fn diplomat_game() -> (Game, CityId, UnitId) {
        let mut g = flat_game();
        g.add_city(2, "Capital".into(), 8, 20);
        let c = g.add_city(2, "Target".into(), 20, 20);
        g.city_mut(c).size = 4;
        g.auto_assign(c);
        let d = g.add_unit(1, unit_ids::DIPLOMAT, 19, 20);
        (g, c, d)
    }

    #[test]
    fn a_city_losing_a_defender_shrinks_but_keeps_its_workers_until_its_turn() {
        let (mut g, c, d) = diplomat_game();
        g.remove_unit(d);
        let workers = g.city(c).workers();
        assert_eq!(workers, 4);
        loop {
            g.add_unit(2, unit_ids::MILITIA, 20, 20);
            let a = g.add_unit(1, unit_ids::ARMOR, 19, 20);
            if let MoveOutcome::Attacked { won: true, .. } = g.attack(a, 20, 20) {
                break;
            }
        }
        assert_eq!(g.city(c).size, 3);
        assert_eq!(g.city(c).workers(), workers);
        assert!(g.city(c).auto_build, "on alert");
    }

    #[test]
    fn diplomat_menu_greys_embassy_stolen_and_palace_lines() {
        let (mut g, c, _) = diplomat_game();
        assert_eq!(g.diplomat_menu_disabled(1, c), [false; 6]);
        g.establish_embassy(1, 2);
        g.city_mut(c).tech_stolen = true;
        g.city_mut(c).add_improvement(imp::PALACE);
        assert_eq!(g.diplomat_menu_disabled(1, c), [true, false, true, false, true, false]);
        let cap = g.player(2).capital.unwrap();
        assert_eq!(g.incite_cost(cap), None, "no revolt in the capital itself");
    }

    #[test]
    fn diplomat_incites_a_revolt_at_the_original_price() {
        use crate::game::turn::{DiplomatAction, DiplomatResult};
        let (mut g, c, d) = diplomat_game();
        g.player_mut(2).gold = 300;
        g.player_mut(1).gold = 2000;
        let guard = g.add_unit(2, unit_ids::PHALANX, 20, 20);
        g.city_mut(c).add_improvement(imp::TEMPLE);
        // CIV.EXE o 0x1d9b7: (300 + 1000) / (12 + 3) * 4 = 344, halved in disorder
        assert_eq!(g.incite_cost(c), Some(344));
        g.city_mut(c).disorder = true;
        assert_eq!(g.incite_cost(c), Some(172));
        let gold = g.player(1).gold;
        let r = g.diplomat_action(d, DiplomatAction::InciteRevolt { subvert: false }, c).unwrap();
        assert_eq!(r, DiplomatResult::Incited { cost: 172 });
        let city = g.city(c);
        assert_eq!(city.owner, 1);
        assert_eq!(city.size, 3);
        assert!(city.has_improvement(imp::TEMPLE), "improvements survive a revolt");
        assert_eq!(g.unit(guard).owner, 1, "the garrison changes sides");
        assert!(g.unit_opt(d).is_none());
        // plunder 300 * 4 / (5 + 1) = 200
        assert_eq!(g.player(1).gold, gold - 172 + 200);
    }

    #[test]
    fn diplomat_subverts_only_at_peace_and_keeps_it() {
        use crate::game::turn::DiplomatAction;
        let (mut g, c, d) = diplomat_game();
        g.player_mut(1).gold = 5000;
        assert!(g.diplomat_action(d, DiplomatAction::InciteRevolt { subvert: true }, c).is_err());
        g.set_relation(1, 2, Relation::Peace);
        let (cost, plunder) = (g.incite_cost(c).unwrap(), g.plunder_value(c));
        g.diplomat_action(d, DiplomatAction::InciteRevolt { subvert: true }, c).unwrap();
        assert_eq!(g.relation(1, 2), Relation::Peace);
        assert_eq!(g.city(c).owner, 1);
        assert_eq!(g.player(1).gold, 5000 - 2 * cost + plunder);
    }

    #[test]
    fn diplomat_steals_once_per_city() {
        use crate::game::turn::{DiplomatAction, DiplomatResult};
        use crate::rules::advance_ids as adv;
        let (mut g, c, d) = diplomat_game();
        let known: Vec<_> = g.player(1).known_advances();
        for a in known {
            g.player_mut(2).learn(a);
        }
        g.player_mut(2).learn(adv::ALPHABET);
        let fresh = g.player(2).known_advances().into_iter().filter(|a| !g.player(1).knows(*a)).collect::<Vec<_>>();
        let Ok(DiplomatResult::Stole(a)) = g.diplomat_action(d, DiplomatAction::StealTech, c) else { panic!("nothing stolen") };
        assert!(fresh.contains(&a) && g.player(1).knows(a));
        assert!(g.city(c).tech_stolen);
        let d2 = g.add_unit(1, unit_ids::DIPLOMAT, 19, 20);
        assert!(g.diplomat_action(d2, DiplomatAction::StealTech, c).is_err());
        // a city with nothing new to steal leaves the diplomat in place
        let c2 = g.add_city(2, "Poor".into(), 24, 24);
        let d3 = g.add_unit(1, unit_ids::DIPLOMAT, 23, 24);
        g.player_mut(1).learn(adv::ALPHABET);
        let all: Vec<_> = g.player(2).known_advances();
        for a in all {
            g.player_mut(1).learn(a);
        }
        assert_eq!(g.diplomat_action(d3, DiplomatAction::StealTech, c2).unwrap(), DiplomatResult::NoTechnology);
        assert!(g.unit_opt(d3).is_some());
    }

    #[test]
    fn diplomat_sabotages_an_improvement_when_nothing_is_stored() {
        use crate::game::turn::{DiplomatAction, DiplomatResult};
        let (mut g, c, d) = diplomat_game();
        g.city_mut(c).shields = 0;
        g.city_mut(c).add_improvement(imp::TEMPLE);
        assert_eq!(g.diplomat_action(d, DiplomatAction::Sabotage, c).unwrap(), DiplomatResult::Sabotaged(Some(imp::TEMPLE)));
        assert!(!g.city(c).has_improvement(imp::TEMPLE));
    }

    /// An unhappy city loses its food and shields to unrest and, when its owner has more than
    /// three cities (ds:0x6b66), defects to the civ of the happiest city before it (CIV.EXE
    /// o 0x1bc5d, 0x1bd9d).
    #[test]
    fn unrest_and_defection_to_an_admired_city() {
        use crate::game::events::Disaster;
        use crate::rules::wonder_ids as won;
        let mut g = flat_game();
        let admired = g.add_city(2, "Eden".into(), 10, 10);
        g.city_mut(admired).add_wonder(won::HANGING_GARDENS);
        g.wonders_built[won::HANGING_GARDENS as usize] = Some(admired);
        g.add_city(1, "Capital".into(), 25, 25);
        let c = g.add_city(1, "Unrest".into(), 14, 10);
        g.city_mut(c).size = 10;
        g.auto_assign(c);
        g.city_mut(c).food = 7;
        g.city_mut(c).shields = 9;
        let h = g.city_happiness(c);
        assert!(h.happy < h.unhappy, "{h:?}");
        assert!(g.city_happiness(admired).happy > g.city_happiness(admired).unhappy);
        g.city_unrest(c);
        assert_eq!((g.city(c).food, g.city(c).shields), (0, 0));
        assert_eq!(g.city(c).owner, 1, "no defection with two cities");
        assert!(g.events.iter().any(|e| matches!(e, GameEvent::Disaster { kind: Disaster::Unrest { demand: Some(imp::TEMPLE), .. }, .. })));
        // later slots are not compared, so two more cities only change the count
        g.add_city(1, "Third".into(), 25, 5);
        g.add_city(1, "Fourth".into(), 5, 25);
        g.city_unrest(c);
        assert_eq!(g.city(c).owner, 2);
        assert!(g.events.iter().any(|e| matches!(e, GameEvent::CityDefected { city, admired: a, from: 1 } if *city == c && *a == admired)));
    }

    /// The human (slot h) and an opponent o on the flat arena, everything at war.
    fn sightings_game() -> (Game, u8, u8) {
        let mut g = flat_game();
        let h = g.human_slot;
        let o = if h == 2 { 3 } else { 2 };
        g.players[o as usize].in_game = true;
        g.players[o as usize].alive = true;
        for (a, b) in [(h, o), (1, 2)] {
            g.set_relation(a, b, Relation::War);
        }
        (g, h, o)
    }

    /// CIV.EXE unit +9: created seen by its owner (0x8bee); a step clears it (0x67e4); unit_look
    /// marks both sides of a neighbourhood (0x835b/0x840f) and the mark stays while the unit
    /// stands still, even after the observer has gone (draw_tile_with_units 0x1b85a).
    #[test]
    fn units_stay_seen_until_they_move() {
        let (mut g, h, o) = sightings_game();
        let enemy = g.add_unit(o, unit_ids::LEGION, 15, 15);
        assert_eq!(g.unit(enemy).seen_by, 1 << o);
        assert!(!g.units_shown_to(h, 15, 15));
        let scout = g.add_unit(h, unit_ids::MILITIA, 13, 15);
        g.try_move(scout, crate::game::Dir8::E).unwrap();
        assert!(g.units_shown_to(h, 15, 15), "seen from next door");
        assert!(g.unit_seen_by(scout, o), "and the scout by the enemy");
        // the scout walks away: the enemy is still on the map where it was seen
        g.unit_mut(scout).moves_left = 3;
        g.try_move(scout, crate::game::Dir8::W).unwrap();
        g.unit_mut(scout).moves_left = 3;
        g.try_move(scout, crate::game::Dir8::W).unwrap();
        assert!(!g.visible_to(h, 15, 15));
        assert!(g.units_shown_to(h, 15, 15));
        // once it steps out of sight it is gone from the human's map
        g.place_unit(enemy, 16, 15, 0);
        g.mark_sightings(enemy);
        assert!(!g.units_shown_to(h, 16, 15));
    }

    /// Cities hide their units: a unit in a city is not seen by its neighbours, nor is the
    /// stack of a neighbouring city marked (0x833d, 0x83f1); a foreign city sees an adjacent
    /// land unit (0x82c5), and the human's cities see two squares away (0x86b3).
    #[test]
    fn cities_hide_their_units_and_see_around_them() {
        let (mut g, h, o) = sightings_game();
        g.add_city(o, "Fort".into(), 15, 15);
        let guard = g.add_unit(o, unit_ids::PHALANX, 15, 15);
        let scout = g.add_unit(h, unit_ids::MILITIA, 14, 15);
        g.mark_sightings(scout);
        assert!(!g.unit_seen_by(guard, h), "the garrison stays hidden");
        assert!(g.unit_seen_by(scout, o), "the city sees the land unit next to it");
        g.add_city(h, "Watch".into(), 20, 20);
        let raider = g.add_unit(o, unit_ids::LEGION, 22, 21);
        g.map.explore(h, 22, 21, 0);
        g.mark_sightings(raider);
        assert!(g.unit_seen_by(raider, h), "seen by the human's city two squares away");
    }

    /// Two squares away only units with 5x5 sight see, and never a Submarine (0x87f6); a
    /// Submarine next to a unit is seen like any other (0x840f).
    #[test]
    fn far_sight_sees_two_squares_but_not_submarines() {
        let (mut g, h, o) = sightings_game();
        for y in 10..20 {
            for x in 10..20 {
                g.map.get_mut(x, y).unwrap().terrain = Terrain::Ocean;
            }
        }
        let far = (0..crate::rules::UNITS.len() as u8).find(|k| crate::rules::UNITS[*k as usize].sight & 2 != 0 && crate::rules::UNITS[*k as usize].domain == crate::rules::Domain::Sea && *k != unit_ids::SUBMARINE).unwrap();
        let ship = g.add_unit(h, far, 12, 12);
        let near = g.add_unit(h, unit_ids::TRIREME, 16, 12);
        let frigate = g.add_unit(o, unit_ids::FRIGATE, 14, 12);
        let sub = g.add_unit(o, unit_ids::SUBMARINE, 12, 14);
        let sub2 = g.add_unit(o, unit_ids::SUBMARINE, 17, 12);
        for id in [ship, near] {
            g.mark_sightings(id);
        }
        assert!(g.unit_seen_by(frigate, h), "a ship two squares off is seen by 5x5 sight");
        assert!(!g.unit_seen_by(sub, h), "a Submarine two squares off is not");
        assert!(g.unit_seen_by(sub2, h), "a Submarine next to a unit is");
    }

    #[test]
    fn a_bribed_unit_is_seen_by_its_former_owner() {
        let (mut g, h, o) = sightings_game();
        let t = g.add_unit(o, unit_ids::LEGION, 15, 15);
        g.player_mut(h).gold = 1000;
        let n = g.bribe_unit(h, t);
        assert_eq!(g.unit(n).seen_by, 1 << h | 1 << o);
    }
}
