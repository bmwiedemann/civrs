//! Tile yields, city resource totals, trade split, corruption, upkeep, production management,
//! buying/selling and tax rates.
//!
//! Formulas marked [approx] are reconstructions kept in small functions so they can be tuned.

use super::city::Production;
use super::map::{tile_flags, CityId, Map, Slot};
use super::{Game, RuleError, Specialist};
use crate::rules::{self, improvement_ids as imp, unit_ids, wonder_ids as won, Domain, Govt, ImprovementId, Terrain, WonderId};

/// Resources produced by a city this turn (before/after modifiers).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CityYield {
    pub food: i32,
    /// Food consumed by citizens and settlers.
    pub food_upkeep: i32,
    pub shields: i32,
    /// Shields spent on unit upkeep.
    pub shield_upkeep: i32,
    pub trade: i32,
    pub corruption: i32,
    pub tax: i32,
    pub lux: i32,
    pub sci: i32,
    /// Gold spent on improvement maintenance.
    pub gold_upkeep: i32,
}

impl CityYield {
    pub fn food_surplus(&self) -> i32 {
        self.food - self.food_upkeep
    }
    pub fn shield_surplus(&self) -> i32 {
        self.shields - self.shield_upkeep
    }
}

/// Sale price of an improvement in gold (Civ1: one gold per shield of its cost).
pub fn sell_price(id: ImprovementId) -> i32 {
    rules::IMPROVEMENTS[id as usize].cost as i32 * 10
}

/// Gold needed to complete `remaining` shields of a production item (Civ1 formulas).
pub fn buy_price(prod: Production, remaining: i32, nothing_yet: bool) -> i32 {
    let r = remaining.max(0);
    if r == 0 {
        return 0;
    }
    match prod {
        Production::Unit(_) => {
            let base = 2 * r + r * r / 20;
            if nothing_yet { base * 2 } else { base }
        }
        Production::Improvement(_) => 2 * r,
        Production::Wonder(_) => 4 * r,
    }
}

impl Game {
    /// Does the slot own a non-obsolete wonder?
    pub fn owner_has_wonder(&self, slot: Slot, w: WonderId) -> bool {
        match self.wonders_built[w as usize] {
            Some(c) => self.city_opt(c).map(|c| c.owner == slot).unwrap_or(false) && !self.wonder_obsolete(w),
            None => false,
        }
    }

    /// Wonder is obsolete as soon as any civilisation knows the obsoleting advance ("until the
    /// development of ...", CIV.EXE 0xc91:0x6c8d checks civs 1..7, the owner among them).
    pub fn wonder_obsolete(&self, w: WonderId) -> bool {
        match rules::WONDERS[w as usize].obsolete_by {
            Some(a) => (1..8).any(|s| self.player(s).knows(a)),
            None => false,
        }
    }

    /// Yield (food, shields, trade) of a tile when worked by a city of `slot`.
    pub fn tile_yield(&self, slot: Slot, x: i32, y: i32, city: Option<CityId>) -> (i32, i32, i32) {
        let Some(t) = self.map.get(x, y) else { return (0, 0, 0) };
        let info = t.terrain.info();
        let (mut f, mut s, mut tr) = match (&info.special, t.special) {
            (Some(sp), true) => (sp.food as i32, sp.shields as i32, sp.trade as i32),
            _ => (info.food as i32, info.shields as i32, info.trade as i32),
        };
        // Grassland and River give their shield only on the squares of the pattern
        if matches!(t.terrain, Terrain::Grassland | Terrain::River) {
            s = t.shield as i32;
        }
        let is_centre = city.is_some() && t.city == city;
        let govt = self.player(slot).govt;
        // irrigation / mines (the city centre counts as irrigated where irrigation is possible)
        let irrigated = t.has(tile_flags::IRRIGATION) || (is_centre && info.irrigate != rules::Terraform::No);
        if irrigated {
            if let rules::Terraform::Bonus(b) = info.irrigate {
                f += b as i32;
            }
        }
        if t.has(tile_flags::MINE) {
            if let rules::Terraform::Bonus(b) = info.mine {
                s += b as i32;
            }
        }
        // roads: +1 trade on desert/grassland/plains (the centre always has a road)
        if (t.has(tile_flags::ROAD) || t.has(tile_flags::RAILROAD) || is_centre)
            && matches!(t.terrain, Terrain::Desert | Terrain::Grassland | Terrain::Plains)
        {
            tr += 1;
        }
        if t.has(tile_flags::RAILROAD) && s > 0 {
            s += s / 2;
        }
        if is_centre && s < 1 {
            s = 1;
        }
        // Colossus: +1 trade on every square of its city that has trade, before the government
        // rules (CIV.EXE 0x134dc)
        if tr > 0 {
            if let Some(c) = city {
                if self.city_opt(c).map(|cc| cc.has_wonder(won::COLOSSUS)).unwrap_or(false) && !self.wonder_obsolete(won::COLOSSUS) {
                    tr += 1;
                }
            }
        }
        // government
        let g = rules::GOVERNMENTS[govt as usize];
        let celebrating = city.and_then(|c| self.city_opt(c)).map(|c| c.celebrating).unwrap_or(false);
        if g.tile_penalty && !celebrating {
            if f > 2 {
                f -= 1;
            }
            if s > 2 {
                s -= 1;
            }
            if tr > 2 {
                tr -= 1;
            }
        }
        // Republic/Democracy trade bonus; a celebrating Monarchy/Communism gets the same
        if (g.trade_bonus || (celebrating && matches!(govt, Govt::Monarchy | Govt::Communism))) && tr >= 1 {
            tr += 1;
        }
        if t.has(tile_flags::POLLUTION) {
            f /= 2;
            s /= 2;
            tr /= 2;
        }
        (f, s, tr)
    }

    /// Tile coordinates for a city radius index.
    pub fn radius_tile(&self, c: CityId, idx: u8) -> Option<(i32, i32)> {
        let city = self.city_opt(c)?;
        let (dx, dy) = rules::CITY_RADIUS[idx as usize];
        let (x, y) = (city.x + dx as i32, city.y + dy as i32);
        if Map::in_bounds(x, y) {
            Some((x, y))
        } else {
            None
        }
    }

    /// Can the city work radius tile `idx` (not worked by another city, not enemy-occupied)?
    pub fn tile_workable(&self, c: CityId, idx: u8) -> bool {
        let Some((x, y)) = self.radius_tile(c, idx) else { return false };
        let t = self.map.tile(x, y);
        // the computer players know the whole map; the human works only what was explored
        let owner = self.city(c).owner;
        if idx != 0 && !t.explored_by(owner) && self.player(owner).human {
            return false;
        }
        if t.worked_by.is_some() && t.worked_by != Some(c) {
            return false;
        }
        if idx != 0 && t.city.is_some() {
            return false;
        }
        if self.all_units().any(|u| (u.x, u.y) == (x, y) && u.owner != owner && u.is_military()) {
            return false;
        }
        true
    }

    /// Units drawing shield upkeep from the city (every unit homed there, Settlers included).
    pub fn supported_units(&self, c: CityId) -> Vec<super::UnitId> {
        self.all_units().filter(|u| u.home == Some(c)).map(|u| u.id).collect()
    }

    /// The city gets power from its own plant or a Hoover Dam on its continent.
    pub fn city_has_power(&self, c: CityId) -> bool {
        let city = self.city(c);
        city.has_improvement(imp::POWER_PLANT)
            || city.has_improvement(imp::HYDRO_PLANT)
            || city.has_improvement(imp::NUCLEAR_PLANT)
            || (self.owner_has_wonder(city.owner, won::HOOVER_DAM) && self.same_continent_as_wonder(c, won::HOOVER_DAM))
    }

    /// Shields after Factory (+50%), Mfg. Plant (+100%, not added to the Factory) and power
    /// (+50%, only with one of them), in quarters as CIV.EXE 0xdc5d..0xddd5 counts.
    fn boosted_shields(&self, c: CityId, shields: i32) -> i32 {
        let city = self.city(c);
        let factory = if city.has_improvement(imp::MFG_PLANT) {
            4
        } else if city.has_improvement(imp::FACTORY) {
            2
        } else {
            0
        };
        let power = if self.city_has_power(c) { 2.min(factory) } else { 0 };
        shields + power * shields / 4 + factory * shields / 4
    }

    /// Full resource accounting for a city.
    pub fn city_yield(&self, c: CityId) -> CityYield {
        let city = self.city(c);
        let slot = city.owner;
        let p = self.player(slot);
        let mut y = CityYield::default();
        for idx in 0..21u8 {
            if !city.works(idx) {
                continue;
            }
            if let Some((x, yy)) = self.radius_tile(c, idx) {
                let (f, s, t) = self.tile_yield(slot, x, yy, Some(c));
                y.food += f;
                y.shields += s;
                y.trade += t;
            }
        }
        // trade routes
        for r in &city.trade_routes {
            if self.city_opt(*r).is_some() {
                y.trade += self.trade_route_value(c, *r);
            }
        }
        // food upkeep: 2 per citizen plus settlers (1; 2 under Republic/Democracy)
        let g = rules::GOVERNMENTS[p.govt as usize];
        let settlers = self.all_units().filter(|u| u.home == Some(c) && u.kind == unit_ids::SETTLERS).count() as i32;
        y.food_upkeep = city.size as i32 * 2 + settlers * g.settler_food as i32;
        // shield upkeep: 1 per supported unit beyond the free ones
        let supported = self.supported_units(c).len() as i32;
        let free = g.free_units_per_city as i32;
        y.shield_upkeep = (supported - free).max(0);
        y.shields = self.boosted_shields(c, y.shields);
        // corruption and the tax/lux/sci split
        y.corruption = self.corruption(c, y.trade);
        let net = y.trade - y.corruption;
        if p.govt == Govt::Anarchy {
            y.lux = net;
        } else {
            y.lux = net * p.lux as i32 / 100;
            y.sci = net * p.sci() as i32 / 100;
            y.tax = net - y.lux - y.sci;
        }
        // specialists: 2 each
        for s in &city.specialists {
            match s {
                Specialist::Entertainer => y.lux += 2,
                Specialist::Taxman => y.tax += 2,
                Specialist::Scientist => y.sci += 2,
            }
        }
        self.trade_boosts(c, &mut y);
        y.gold_upkeep = self.city_upkeep(c);
        // civil disorder: no production, taxes or science (food and luxuries continue)
        if city.disorder {
            y.shields = 0;
            y.tax = 0;
            y.sci = 0;
        }
        y
    }

    /// Buildings and wonders that raise taxes, luxuries and science (CIV.EXE 0xf2f2..0xf3f5):
    /// Marketplace and Bank each add half of what the city has so far; Library and University
    /// add half of the base science, plus a third each with Isaac Newton's College; Copernicus
    /// doubles the city's science and the SETI Program adds half of it (0x12cc0).
    fn trade_boosts(&self, c: CityId, y: &mut CityYield) {
        let city = self.city(c);
        let slot = city.owner;
        for i in [imp::MARKETPLACE, imp::BANK] {
            if city.has_improvement(i) {
                y.tax += y.tax / 2;
                y.lux += y.lux / 2;
            }
        }
        let base_sci = y.sci;
        let newton = self.owner_has_wonder(slot, won::ISAAC_NEWTONS_COLLEGE);
        for i in [imp::LIBRARY, imp::UNIVERSITY] {
            if city.has_improvement(i) {
                y.sci += base_sci / 2;
                if newton {
                    y.sci += base_sci / 3;
                }
            }
        }
        if city.has_wonder(won::COPERNICUS_OBSERVATORY) && !self.wonder_obsolete(won::COPERNICUS_OBSERVATORY) {
            y.sci *= 2;
        }
        if self.owner_has_wonder(slot, won::SETI_PROGRAM) {
            y.sci += y.sci / 2;
        }
    }

    /// Trade a route between two cities yields per turn: `(ta + tb + 4) / 8`, doubled for a
    /// foreign city.
    pub fn trade_route_value(&self, a: CityId, b: CityId) -> i32 {
        let (Some(ca), Some(cb)) = (self.city_opt(a), self.city_opt(b)) else { return 0 };
        let ta = self.raw_trade(a);
        let tb = self.raw_trade(b);
        let mut v = (ta + tb + 4) / 8;
        if ca.owner != cb.owner {
            v *= 2;
        }
        v.max(1)
    }

    /// Trade from worked tiles only (no routes), used by the route formulas.
    pub fn raw_trade(&self, c: CityId) -> i32 {
        let city = self.city(c);
        let mut t = 0;
        for idx in 0..21u8 {
            if city.works(idx) {
                if let Some((x, y)) = self.radius_tile(c, idx) {
                    t += self.tile_yield(city.owner, x, y, Some(c)).2;
                }
            }
        }
        t
    }

    /// Trade lost to corruption: `trade * distance * factor / 100` [approx], Communism uses a
    /// flat distance, a Courthouse halves it, the Palace city has none.
    pub fn corruption(&self, c: CityId, trade: i32) -> i32 {
        let city = self.city(c);
        let p = self.player(city.owner);
        let g = rules::GOVERNMENTS[p.govt as usize];
        if g.corruption == 0 || city.has_improvement(imp::PALACE) {
            return 0;
        }
        let dist = if p.govt == Govt::Communism {
            8
        } else {
            match p.capital.and_then(|cap| self.city_opt(cap)) {
                Some(cap) => Map::dist((city.x, city.y), (cap.x, cap.y)),
                None => 16,
            }
        };
        let mut corr = trade * dist * g.corruption as i32 / 100;
        if city.has_improvement(imp::COURTHOUSE) {
            corr /= 2;
        }
        corr.clamp(0, trade)
    }

    /// Gold per turn for the city's improvements.
    pub fn city_upkeep(&self, c: CityId) -> i32 {
        let city = self.city(c);
        if !self.pays_upkeep(city.owner) {
            return 0;
        }
        city.improvement_list().iter().filter(|i| **i != imp::PALACE).map(|i| self.improvement_upkeep(city.owner, *i)).sum()
    }

    /// Only the human player pays for buildings, and not under Anarchy (CIV.EXE 0x12ddd, 0x1307a).
    pub fn pays_upkeep(&self, slot: Slot) -> bool {
        let p = self.player(slot);
        p.human && p.govt != Govt::Anarchy
    }

    /// Gold per turn for improvement `i` of `slot` (CIV.EXE 0x13084): the table value; Barracks
    /// cost 1 more from Prince, again from Emperor, and after each of Gunpowder and Combustion.
    pub fn improvement_upkeep(&self, slot: Slot, i: ImprovementId) -> i32 {
        let mut u = rules::IMPROVEMENTS[i as usize].upkeep as i32;
        if i == imp::BARRACKS {
            let diff = self.setup.difficulty as i32;
            let p = self.player(slot);
            u += (diff >= 2) as i32 + (diff >= 4) as i32;
            u += p.knows(rules::advance_ids::GUNPOWDER) as i32 + p.knows(rules::advance_ids::COMBUSTION) as i32;
        }
        u
    }

    /// Total per-turn gold change for a player.
    pub fn income(&self, slot: Slot) -> i32 {
        self.cities_of(slot).iter().map(|c| {
            let y = self.city_yield(*c);
            y.tax - y.gold_upkeep
        }).sum()
    }

    /// Total beakers per turn for a player.
    pub fn science_income(&self, slot: Slot) -> i32 {
        self.cities_of(slot).iter().map(|c| self.city_yield(*c).sci).sum()
    }

    /// Heuristic value of a tile for the worker auto-assignment [approx].
    fn tile_value(&self, c: CityId, idx: u8) -> i32 {
        let slot = self.city(c).owner;
        let Some((x, y)) = self.radius_tile(c, idx) else { return -1 };
        let (f, s, t) = self.tile_yield(slot, x, y, Some(c));
        f * 7 + s * 3 + t * 2
    }

    /// A city without any trade pays no upkeep and learns nothing: swap its least valuable
    /// worker onto the best ocean / river / road tile, as long as the city keeps growing (or
    /// loses no food by it).
    fn ensure_trade(&mut self, c: CityId) {
        let slot = self.city(c).owner;
        let yields = |g: &Game, idx: u8| g.radius_tile(c, idx).map(|(x, y)| g.tile_yield(slot, x, y, Some(c))).unwrap_or((0, 0, 0));
        if self.city_yield(c).trade > 0 {
            return;
        }
        let city = self.city(c);
        let worst = (1..21u8).filter(|i| city.works(*i)).min_by_key(|i| self.tile_value(c, *i));
        let best = (1..21u8).filter(|i| !city.works(*i) && self.tile_workable(c, *i) && yields(self, *i).2 > 0).max_by_key(|i| self.tile_value(c, *i));
        let (Some(w), Some(b)) = (worst, best) else { return };
        let food_change = yields(self, b).0 - yields(self, w).0;
        if food_change >= 0 || self.city_yield(c).food_surplus() + food_change > 0 {
            self.release_tile(c, w);
            let (x, y) = self.radius_tile(c, b).unwrap();
            self.map.get_mut(x, y).unwrap().worked_by = Some(c);
            self.city_mut(c).set_worked(b, true);
        }
    }

    /// Automatically assign workers (food first, then shields, then trade).
    pub fn auto_assign(&mut self, c: CityId) {
        let size = self.city(c).size as usize;
        // clear
        for idx in 1..21u8 {
            if let Some((x, y)) = self.radius_tile(c, idx) {
                if self.map.tile(x, y).worked_by == Some(c) {
                    self.map.get_mut(x, y).unwrap().worked_by = None;
                }
            }
        }
        self.city_mut(c).worked = 0;
        self.city_mut(c).specialists.clear();
        for _ in 0..size {
            self.add_worker(c);
        }
        self.ensure_trade(c);
    }

    /// Put one more citizen to work on the best free tile (or make an entertainer).
    pub fn add_worker(&mut self, c: CityId) {
        let mut best: Option<(u8, i32)> = None;
        for idx in 1..21u8 {
            if self.city(c).works(idx) || !self.tile_workable(c, idx) {
                continue;
            }
            let v = self.tile_value(c, idx);
            if best.map(|(_, bv)| v > bv).unwrap_or(true) {
                best = Some((idx, v));
            }
        }
        match best {
            Some((idx, _)) => {
                let (x, y) = self.radius_tile(c, idx).unwrap();
                self.map.get_mut(x, y).unwrap().worked_by = Some(c);
                self.city_mut(c).set_worked(idx, true);
            }
            None => self.city_mut(c).specialists.push(Specialist::Entertainer),
        }
    }

    /// Remove one citizen: a specialist first, otherwise the worker on the worst tile.
    pub fn remove_worker(&mut self, c: CityId) {
        if self.city_mut(c).specialists.pop().is_some() {
            return;
        }
        let mut worst: Option<(u8, i32)> = None;
        for idx in 1..21u8 {
            if !self.city(c).works(idx) {
                continue;
            }
            let v = self.tile_value(c, idx);
            if worst.map(|(_, wv)| v < wv).unwrap_or(true) {
                worst = Some((idx, v));
            }
        }
        if let Some((idx, _)) = worst {
            self.release_tile(c, idx);
        }
    }

    fn release_tile(&mut self, c: CityId, idx: u8) {
        self.city_mut(c).set_worked(idx, false);
        if let Some((x, y)) = self.radius_tile(c, idx) {
            if let Some(t) = self.map.get_mut(x, y) {
                if t.worked_by == Some(c) {
                    t.worked_by = None;
                }
            }
        }
    }

    /// Make the worker layout consistent with the city size and the current map state
    /// (tiles taken by enemies or other cities are released; citizens are added/removed).
    pub fn fix_workers(&mut self, c: CityId) {
        for idx in 1..21u8 {
            if self.city(c).works(idx) {
                let ok = match self.radius_tile(c, idx) {
                    Some((x, y)) => self.map.tile(x, y).worked_by == Some(c) || self.map.tile(x, y).worked_by.is_none(),
                    None => false,
                } && self.tile_workable(c, idx);
                if ok {
                    if let Some((x, y)) = self.radius_tile(c, idx) {
                        self.map.get_mut(x, y).unwrap().worked_by = Some(c);
                    }
                } else {
                    self.release_tile(c, idx);
                    self.city_mut(c).specialists.push(Specialist::Entertainer);
                }
            }
        }
        let size = self.city(c).size as usize;
        loop {
            let have = self.city(c).workers() as usize + self.city(c).specialists.len();
            if have == size {
                break;
            }
            if have > size {
                self.remove_worker(c);
            } else {
                self.add_worker(c);
            }
        }
        // the human's own layout is left alone
        if !self.player(self.city(c).owner).human {
            self.ensure_trade(c);
        }
    }

    /// Toggle working a radius tile by the city (a worker becomes an entertainer or vice versa).
    pub fn toggle_tile(&mut self, c: CityId, idx: u8) {
        if idx == 0 || idx >= 21 {
            return;
        }
        let works = self.city(c).works(idx);
        if self.radius_tile(c, idx).is_none() {
            return;
        }
        if works {
            self.release_tile(c, idx);
            self.city_mut(c).specialists.push(Specialist::Entertainer);
        } else {
            if !self.tile_workable(c, idx) {
                return;
            }
            if self.city(c).specialists.is_empty() {
                return;
            }
            self.city_mut(c).specialists.pop();
            self.city_mut(c).set_worked(idx, true);
            let (x, y) = self.radius_tile(c, idx).unwrap();
            self.map.get_mut(x, y).unwrap().worked_by = Some(c);
        }
    }

    pub fn set_specialist(&mut self, c: CityId, i: usize, s: Specialist) {
        if let Some(sp) = self.city_mut(c).specialists.get_mut(i) {
            *sp = s;
        }
    }

    // ---- production ----------------------------------------------------------------------

    /// Gold needed to complete the current production right now.
    pub fn buy_cost(&self, c: CityId) -> i32 {
        let city = self.city(c);
        let remaining = city.production.cost() - city.shields;
        buy_price(city.production, remaining, city.shields <= 0)
    }

    /// Pay gold to complete the current production (delivered at the end of the turn).
    pub fn buy(&mut self, c: CityId) -> Result<(), RuleError> {
        let city = self.city(c);
        if city.bought {
            return Err(RuleError::Other("Already purchased this turn."));
        }
        if city.disorder {
            return Err(RuleError::Other("Cannot buy during civil disorder."));
        }
        if let Production::Wonder(w) = city.production {
            if self.wonders_built[w as usize].is_some() {
                return Err(RuleError::Other("That wonder has already been built."));
            }
        }
        if let Production::Improvement(i) = city.production {
            if city.has_improvement(i) {
                return Err(RuleError::Other("That improvement has already been built."));
            }
        }
        let cost = self.buy_cost(c);
        if cost <= 0 {
            return Err(RuleError::Other("Production is already complete."));
        }
        let slot = city.owner;
        if self.player(slot).gold < cost {
            return Err(RuleError::NotEnoughGold);
        }
        self.player_mut(slot).gold -= cost;
        let total = self.city(c).production.cost();
        let cm = self.city_mut(c);
        cm.shields = total;
        cm.bought = true;
        Ok(())
    }

    /// Change what the city builds. Switching between unit / improvement / wonder loses half
    /// of the accumulated shields (once per turn).
    pub fn set_production(&mut self, c: CityId, prod: Production) -> Result<(), RuleError> {
        let city = self.city(c);
        if city.production == prod {
            return Ok(());
        }
        if city.bought {
            return Err(RuleError::Other("Production cannot be changed after a purchase."));
        }
        if !self.available_production(c).contains(&prod) {
            return Err(RuleError::Other("That cannot be built here."));
        }
        // CIV.EXE keeps every shield on a change, whatever the new item (the CHANGE handler
        // at 0x117e8 only stores the new choice; no code halves the shield store)
        self.city_mut(c).production = prod;
        Ok(())
    }

    /// Can the city of `slot` build unit type `k` right now?
    pub fn can_build_unit(&self, c: CityId, k: u8) -> bool {
        let city = self.city(c);
        let p = self.player(city.owner);
        let u = &rules::UNITS[k as usize];
        if let Some(a) = u.prereq {
            if !p.knows(a) {
                return false;
            }
        }
        if let Some(o) = u.obsolete_by {
            if p.knows(o) {
                return false;
            }
        }
        if u.domain == Domain::Sea && !self.map.is_coastal(city.x, city.y) {
            return false;
        }
        if k == unit_ids::NUCLEAR && !self.nukes_enabled {
            return false;
        }
        true
    }

    /// Can the city build improvement `i` right now?
    pub fn can_build_improvement(&self, c: CityId, i: ImprovementId) -> bool {
        let city = self.city(c);
        let p = self.player(city.owner);
        if i == 0 || i as usize >= rules::IMPROVEMENTS.len() || city.has_improvement(i) {
            return false;
        }
        let info = &rules::IMPROVEMENTS[i as usize];
        if let Some(a) = info.prereq {
            if !p.knows(a) {
                return false;
            }
        }
        // the production menu's rules (CIV.EXE 0xacd7..0xadbc): no Courthouse in the capital, no
        // Factory beside a Mfg. Plant, a Bank needs a Marketplace, a University a Library, a Power
        // Plant a Factory, and no plant at all where a Hoover Dam on the continent supplies power
        let hoover = self.owner_has_wonder(city.owner, won::HOOVER_DAM) && self.same_continent_as_wonder(c, won::HOOVER_DAM);
        let plant = city.has_improvement(imp::POWER_PLANT) || city.has_improvement(imp::HYDRO_PLANT) || city.has_improvement(imp::NUCLEAR_PLANT);
        match i {
            imp::PALACE => p.capital.and_then(|cap| self.city_opt(cap)).is_none(),
            imp::COURTHOUSE => !city.has_improvement(imp::PALACE),
            imp::FACTORY => !city.has_improvement(imp::MFG_PLANT),
            imp::BANK => city.has_improvement(imp::MARKETPLACE),
            imp::UNIVERSITY => city.has_improvement(imp::LIBRARY),
            imp::HYDRO_PLANT => {
                !hoover
                    && !plant
                    && Map::neighbours8(city.x, city.y).chain(std::iter::once((city.x, city.y))).any(|(x, y)| matches!(self.map.terrain(x, y), Terrain::River | Terrain::Mountains))
            }
            imp::POWER_PLANT => !hoover && !plant && city.has_improvement(imp::FACTORY),
            imp::NUCLEAR_PLANT => !hoover && !plant,
            imp::SS_STRUCTURAL | imp::SS_COMPONENT | imp::SS_MODULE => {
                if !self.apollo_built || p.spaceship.launched() {
                    return false;
                }
                let s = p.spaceship;
                match i {
                    imp::SS_STRUCTURAL => s.structurals < super::spaceship::MAX_STRUCTURALS,
                    imp::SS_COMPONENT => s.components < super::spaceship::MAX_COMPONENTS,
                    _ => s.modules < super::spaceship::MAX_MODULES,
                }
            }
            _ => true,
        }
    }

    /// Can the city start (or continue) building wonder `w`?
    pub fn can_build_wonder(&self, c: CityId, w: WonderId) -> bool {
        let city = self.city(c);
        let p = self.player(city.owner);
        if w == 0 || w as usize >= rules::WONDERS.len() || self.wonders_built[w as usize].is_some() {
            return false;
        }
        let info = &rules::WONDERS[w as usize];
        if let Some(a) = info.prereq {
            if !p.knows(a) {
                return false;
            }
        }
        if let Some(o) = info.obsolete_by {
            if p.knows(o) {
                return false;
            }
        }
        true
    }

    /// Everything the city may build now: units, then improvements, then wonders.
    pub fn available_production(&self, c: CityId) -> Vec<Production> {
        let mut v = Vec::new();
        for k in 0..rules::UNITS.len() as u8 {
            if self.can_build_unit(c, k) {
                v.push(Production::Unit(k));
            }
        }
        for i in 1..rules::IMPROVEMENTS.len() as u8 {
            if self.can_build_improvement(c, i) {
                v.push(Production::Improvement(i));
            }
        }
        for w in 1..rules::WONDERS.len() as u8 {
            if self.can_build_wonder(c, w) {
                v.push(Production::Wonder(w));
            }
        }
        v
    }

    /// What a city builds after finishing an improvement/wonder: the cheapest available
    /// improvement, else the best defensive unit it can build [approx].
    /// Whether the city's production can no longer be completed: an improvement it already
    /// has or a wonder built somewhere (the original flashes the CHANGE button then).
    pub fn production_invalid(&self, c: CityId) -> bool {
        let city = self.city(c);
        match city.production {
            Production::Improvement(i) => city.has_improvement(i),
            Production::Wonder(w) => self.wonders_built[w as usize].is_some(),
            Production::Unit(_) => false,
        }
    }

    pub fn default_production(&self, c: CityId) -> Production {
        let mut best: Option<(i32, Production)> = None;
        for i in 1..rules::IMPROVEMENTS.len() as u8 {
            if i == imp::PALACE || i >= imp::SS_STRUCTURAL {
                continue;
            }
            if self.can_build_improvement(c, i) {
                let cost = rules::IMPROVEMENTS[i as usize].cost as i32;
                if best.map(|(bc, _)| cost < bc).unwrap_or(true) {
                    best = Some((cost, Production::Improvement(i)));
                }
            }
        }
        if let Some((_, p)) = best {
            return p;
        }
        self.best_defender_type(c)
    }

    /// Best defensive land unit the city can build.
    pub fn best_defender_type(&self, c: CityId) -> Production {
        let mut best = (0, unit_ids::MILITIA);
        for k in 0..rules::UNITS.len() as u8 {
            let u = &rules::UNITS[k as usize];
            if u.domain == Domain::Land && u.def > 0 && k != unit_ids::SETTLERS && self.can_build_unit(c, k) && u.def as i32 > best.0 {
                best = (u.def as i32, k);
            }
        }
        Production::Unit(best.1)
    }

    /// Improvements that may be sold in the city (everything but the Palace).
    pub fn sellable_improvements(&self, c: CityId) -> Vec<ImprovementId> {
        self.city(c).improvement_list().into_iter().filter(|i| *i != imp::PALACE).collect()
    }

    /// Sell an improvement for gold (one sale per city per turn). Returns the gold received.
    pub fn sell_improvement(&mut self, c: CityId, id: ImprovementId) -> Result<i32, RuleError> {
        let city = self.city(c);
        let slot = city.owner;
        if !city.has_improvement(id) {
            return Err(RuleError::Other("The city does not have that improvement."));
        }
        if id == imp::PALACE {
            return Err(RuleError::Other("The Palace cannot be sold."));
        }
        if self.player(slot).sold_cities.contains(&c) {
            return Err(RuleError::Other("Only one improvement may be sold per turn."));
        }
        let gold = sell_price(id);
        self.city_mut(c).remove_improvement(id);
        let p = self.player_mut(slot);
        p.gold += gold;
        p.sold_cities.push(c);
        self.push_event(super::GameEvent::ImprovementSold { city: c, improvement: id, gold });
        Ok(gold)
    }

    pub fn rename_city(&mut self, c: CityId, name: &str) -> Result<(), RuleError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(RuleError::Other("A city needs a name."));
        }
        if self.all_cities().any(|o| o.id != c && o.name.eq_ignore_ascii_case(name)) {
            return Err(RuleError::Other("There is already a city of that name."));
        }
        self.city_mut(c).name = name.chars().take(13).collect();
        Ok(())
    }

    // ---- rates ----------------------------------------------------------------------------

    /// Highest tax/lux/sci rate allowed under the player's government.
    pub fn max_rate(&self, slot: Slot) -> u8 {
        rules::GOVERNMENTS[self.player(slot).govt as usize].max_rate
    }

    /// Set tax and luxury rates (science is the remainder); all three must be multiples of 10
    /// within the government's maximum. Anarchy forces 0/100/0.
    pub fn set_rates(&mut self, slot: Slot, tax: u8, lux: u8) -> Result<(), RuleError> {
        if tax % 10 != 0 || lux % 10 != 0 || tax as u32 + lux as u32 > 100 {
            return Err(RuleError::Other("Rates must be multiples of 10% adding up to 100%."));
        }
        let sci = 100 - tax - lux;
        if self.player(slot).govt == Govt::Anarchy {
            return if tax == 0 && lux == 100 {
                let p = self.player_mut(slot);
                p.tax = 0;
                p.lux = 100;
                Ok(())
            } else {
                Err(RuleError::Other("Under Anarchy all trade goes to luxuries."))
            };
        }
        let max = self.max_rate(slot);
        if tax > max || lux > max || sci > max {
            return Err(RuleError::Other("Rate exceeds the maximum for this government."));
        }
        let p = self.player_mut(slot);
        p.tax = tax;
        p.lux = lux;
        Ok(())
    }

    /// Bring the player's rates within the limits of its (new) government.
    pub fn clamp_rates(&mut self, slot: Slot) {
        let govt = self.player(slot).govt;
        if govt == Govt::Anarchy {
            return;
        }
        let max = self.max_rate(slot) as i32;
        let (mut tax, mut lux) = (self.player(slot).tax as i32, self.player(slot).lux as i32);
        tax = tax / 10 * 10;
        lux = lux / 10 * 10;
        if tax + lux > 100 {
            lux = 100 - tax;
        }
        let mut sci = 100 - tax - lux;
        // shift excess from the largest rate into the smallest until all fit
        for _ in 0..20 {
            if tax <= max && lux <= max && sci <= max {
                break;
            }
            if tax > max {
                tax -= 10;
                if sci < max { sci += 10 } else { lux += 10 }
            } else if sci > max {
                sci -= 10;
                if tax < max { tax += 10 } else { lux += 10 }
            } else {
                lux -= 10;
                if tax < max { tax += 10 } else { sci += 10 }
            }
        }
        let p = self.player_mut(slot);
        p.tax = tax.clamp(0, 100) as u8;
        p.lux = lux.clamp(0, 100) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{GameSetup, UnitId};

    pub(crate) fn fixture() -> (Game, CityId) {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler: UnitId = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).expect("settler");
        let c = g.found_city(settler).expect("found city");
        // deterministic fixture: no starting gold or knowledge
        g.player_mut(slot).gold = 0;
        g.player_mut(slot).advances = 0;
        (g, c)
    }

    #[test]
    fn size_one_city_feeds_itself() {
        let (g, c) = fixture();
        let y = g.city_yield(c);
        assert!(y.food >= 2, "food {}", y.food);
        assert!(y.shields >= 1);
        assert_eq!(y.food_upkeep, 2);
        assert_eq!(y.gold_upkeep, 0, "palace has no upkeep");
    }

    #[test]
    fn buy_cost_and_buy() {
        let (mut g, c) = fixture();
        let cost = g.buy_cost(c);
        assert!(cost > 0);
        // Militia: 10 shields, nothing yet -> (2*10 + 100/20) * 2 = 50
        assert_eq!(cost, 50);
        let slot = g.city(c).owner;
        assert_eq!(g.buy(c), Err(RuleError::NotEnoughGold));
        g.player_mut(slot).gold = 100;
        assert!(g.buy(c).is_ok());
        assert_eq!(g.player(slot).gold, 50);
        assert_eq!(g.city(c).shields, 10);
        assert!(g.city(c).bought);
        assert!(g.set_production(c, Production::Improvement(imp::BARRACKS)).is_err());
    }

    #[test]
    fn change_keeps_shields() {
        let (mut g, c) = fixture();
        g.city_mut(c).shields = 8;
        g.set_production(c, Production::Improvement(imp::BARRACKS)).unwrap();
        assert_eq!(g.city(c).shields, 8);
        g.set_production(c, Production::Unit(unit_ids::MILITIA)).unwrap();
        assert_eq!(g.city(c).shields, 8);
    }

    #[test]
    fn available_production_basics() {
        let (g, c) = fixture();
        let v = g.available_production(c);
        assert!(v.contains(&Production::Unit(unit_ids::MILITIA)));
        assert!(v.contains(&Production::Unit(unit_ids::SETTLERS)));
        assert!(v.contains(&Production::Improvement(imp::BARRACKS)));
        assert!(!v.contains(&Production::Improvement(imp::PALACE)), "capital exists");
        assert!(!v.contains(&Production::Improvement(imp::GRANARY)), "needs Pottery");
        assert!(!v.contains(&Production::Improvement(imp::SS_STRUCTURAL)));
        assert!(!v.iter().any(|p| matches!(p, Production::Wonder(_))), "no wonder without advances");
    }

    #[test]
    fn sell_once_per_turn() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        g.city_mut(c).add_improvement(imp::BARRACKS);
        g.city_mut(c).add_improvement(imp::TEMPLE);
        assert_eq!(g.sell_improvement(c, imp::BARRACKS), Ok(40));
        assert_eq!(g.player(slot).gold, 40);
        assert!(g.sell_improvement(c, imp::TEMPLE).is_err());
        assert!(g.sell_improvement(c, imp::PALACE).is_err());
    }

    #[test]
    fn rates_respect_government() {
        let (mut g, _c) = fixture();
        let slot = g.human_slot;
        assert!(g.set_rates(slot, 60, 0).is_ok());
        assert!(g.set_rates(slot, 70, 0).is_err(), "despotism max 60");
        assert!(g.set_rates(slot, 0, 0).is_err(), "science would be 100");
        assert!(g.set_rates(slot, 55, 0).is_err());
        g.player_mut(slot).govt = Govt::Anarchy;
        assert!(g.set_rates(slot, 50, 0).is_err());
        assert!(g.set_rates(slot, 0, 100).is_ok());
        g.player_mut(slot).govt = Govt::Despotism;
        g.clamp_rates(slot);
        let p = g.player(slot);
        assert!(p.tax <= 60 && p.lux <= 60 && p.sci() <= 60);
    }

    #[test]
    fn workers_follow_size() {
        let (mut g, c) = fixture();
        g.city_mut(c).size = 4;
        g.fix_workers(c);
        let city = g.city(c);
        assert_eq!(city.workers() as usize + city.specialists.len(), 4);
        g.city_mut(c).size = 2;
        g.fix_workers(c);
        let city = g.city(c);
        assert_eq!(city.workers() as usize + city.specialists.len(), 2);
    }

    /// Rebuild the fixture city's radius: `centre` under the city, ocean at radius index 4,
    /// plain grassland elsewhere (no trade without the ocean).
    fn island_radius(g: &mut Game, c: CityId, centre: Terrain) {
        for idx in 0..21u8 {
            let (x, y) = g.radius_tile(c, idx).unwrap();
            let t = g.map.get_mut(x, y).unwrap();
            t.terrain = match idx {
                0 => centre,
                4 => Terrain::Ocean,
                _ => Terrain::Grassland,
            };
            t.flags = 0;
            t.special = false;
            t.shield = false;
        }
    }

    #[test]
    fn only_the_human_needs_explored_tiles() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        let (x, y) = g.radius_tile(c, 9).unwrap();
        g.map.get_mut(x, y).unwrap().explored &= !(1 << slot);
        assert!(!g.tile_workable(c, 9));
        g.player_mut(slot).human = false;
        assert!(g.tile_workable(c, 9));
    }

    #[test]
    fn a_city_without_trade_works_the_ocean_unless_it_stops_growing() {
        let (mut g, c) = fixture();
        g.city_mut(c).size = 2;
        // a hills centre (2 food, no trade) leaves room: one worker moves to the ocean and the
        // city still grows
        island_radius(&mut g, c, Terrain::Hills);
        g.auto_assign(c);
        assert!(g.city(c).works(4));
        assert!(g.city_yield(c).trade > 0 && g.city_yield(c).food_surplus() > 0);
        // a forest centre: the ocean would stop growth, so the city keeps farming
        island_radius(&mut g, c, Terrain::Forest);
        g.auto_assign(c);
        assert!(!g.city(c).works(4));
        assert!(g.city_yield(c).food_surplus() > 0);
    }

    #[test]
    fn a_wonder_is_obsolete_once_any_civ_learns_the_advance() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        g.city_mut(c).add_wonder(won::COLOSSUS);
        g.wonders_built[won::COLOSSUS as usize] = Some(c);
        assert!(g.owner_has_wonder(slot, won::COLOSSUS));
        let other = g.civ_slots().into_iter().find(|s| *s != slot).unwrap();
        g.player_mut(other).learn(crate::rules::advance_ids::ELECTRICITY);
        assert!(g.wonder_obsolete(won::COLOSSUS));
        assert!(!g.owner_has_wonder(slot, won::COLOSSUS));
    }

    #[test]
    fn factory_mfg_plant_and_power_in_quarters() {
        let (mut g, c) = fixture();
        let b = |g: &Game| (g.boosted_shields(c, 10), g.boosted_shields(c, 3));
        assert_eq!(b(&g), (10, 3));
        g.city_mut(c).add_improvement(imp::POWER_PLANT);
        assert_eq!(b(&g), (10, 3), "power needs a factory");
        g.city_mut(c).add_improvement(imp::FACTORY);
        assert_eq!(b(&g), (20, 5));
        g.city_mut(c).remove_improvement(imp::POWER_PLANT);
        assert_eq!(b(&g), (15, 4));
        g.city_mut(c).add_improvement(imp::MFG_PLANT);
        assert_eq!(b(&g), (20, 6));
        g.city_mut(c).add_improvement(imp::HYDRO_PLANT);
        assert_eq!(b(&g), (25, 7));
    }

    #[test]
    fn trade_buildings_and_science_wonders() {
        let (mut g, c) = fixture();
        let boost = |g: &Game, tax, sci| {
            let mut y = CityYield { tax, lux: tax, sci, ..Default::default() };
            g.trade_boosts(c, &mut y);
            (y.tax, y.lux, y.sci)
        };
        g.city_mut(c).add_improvement(imp::MARKETPLACE);
        g.city_mut(c).add_improvement(imp::BANK);
        g.city_mut(c).add_improvement(imp::LIBRARY);
        g.city_mut(c).add_improvement(imp::UNIVERSITY);
        // the Bank adds half of what the Marketplace left: 10 -> 15 -> 22
        assert_eq!(boost(&g, 10, 12), (22, 22, 24));
        let other = g.add_city(g.city(c).owner, "Other".into(), g.city(c).x + 6, g.city(c).y);
        g.city_mut(other).add_wonder(won::ISAAC_NEWTONS_COLLEGE);
        g.wonders_built[won::ISAAC_NEWTONS_COLLEGE as usize] = Some(other);
        // Isaac Newton's College helps the libraries and universities of every city
        assert_eq!(boost(&g, 10, 12).2, 32);
        g.city_mut(c).add_wonder(won::COPERNICUS_OBSERVATORY);
        g.wonders_built[won::COPERNICUS_OBSERVATORY as usize] = Some(c);
        assert_eq!(boost(&g, 10, 12).2, 64);
        g.city_mut(other).add_wonder(won::SETI_PROGRAM);
        g.wonders_built[won::SETI_PROGRAM as usize] = Some(other);
        assert_eq!(boost(&g, 10, 12).2, 96);
    }

    #[test]
    fn colossus_adds_trade_on_every_trade_square() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        let (x, y) = g.city(c).pos();
        g.map.get_mut(x, y).unwrap().terrain = Terrain::Grassland;
        g.map.get_mut(x + 1, y).unwrap().terrain = Terrain::Forest;
        g.map.get_mut(x - 1, y).unwrap().terrain = Terrain::Ocean;
        g.map.get_mut(x - 1, y).unwrap().special = false;
        let trade = |g: &Game| [(x, y), (x + 1, y), (x - 1, y)].map(|(tx, ty)| g.tile_yield(slot, tx, ty, Some(c)).2);
        assert_eq!(trade(&g), [1, 0, 2]);
        g.city_mut(c).add_wonder(won::COLOSSUS);
        g.wonders_built[won::COLOSSUS as usize] = Some(c);
        // the ocean's 3 is cut back to 2 by Despotism
        assert_eq!(trade(&g), [2, 0, 2]);
    }

    #[test]
    fn great_wall_brings_peace_offers_not_walls() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        assert!(!g.offered_peace(slot));
        g.city_mut(c).add_wonder(won::GREAT_WALL);
        g.wonders_built[won::GREAT_WALL as usize] = Some(c);
        assert!(g.offered_peace(slot));
        assert!(!g.city_has_walls(g.city(c)));
        let other = g.civ_slots().into_iter().find(|s| *s != slot).unwrap();
        g.player_mut(other).learn(crate::rules::advance_ids::GUNPOWDER);
        assert!(!g.offered_peace(slot));
    }

    #[test]
    fn production_menu_rules_of_civ_exe() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        for a in [crate::rules::advance_ids::CODE_OF_LAWS, crate::rules::advance_ids::BANKING, crate::rules::advance_ids::UNIVERSITY, crate::rules::advance_ids::REFINING] {
            g.player_mut(slot).learn(a);
        }
        // the capital needs no Courthouse
        assert!(g.city(c).has_improvement(imp::PALACE));
        assert!(!g.can_build_improvement(c, imp::COURTHOUSE));
        assert!(!g.can_build_improvement(c, imp::BANK));
        g.city_mut(c).add_improvement(imp::MARKETPLACE);
        assert!(g.can_build_improvement(c, imp::BANK));
        assert!(!g.can_build_improvement(c, imp::UNIVERSITY));
        g.city_mut(c).add_improvement(imp::LIBRARY);
        assert!(g.can_build_improvement(c, imp::UNIVERSITY));
        assert!(!g.can_build_improvement(c, imp::POWER_PLANT));
        g.city_mut(c).add_improvement(imp::FACTORY);
        assert!(g.can_build_improvement(c, imp::POWER_PLANT));
        g.city_mut(c).add_wonder(won::HOOVER_DAM);
        g.wonders_built[won::HOOVER_DAM as usize] = Some(c);
        assert!(!g.can_build_improvement(c, imp::POWER_PLANT));
    }

    #[test]
    fn apollo_shows_its_owner_every_city() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        let other = g.civ_slots().into_iter().find(|s| *s != slot).unwrap();
        let (x, y) = g.city(c).pos();
        let (fx, fy) = (x + 12, y);
        g.add_city(other, "Far".into(), fx, fy);
        g.map.get_mut(fx, fy).unwrap().explored &= !(1 << slot);
        g.map.get_mut(fx + 1, fy).unwrap().explored &= !(1 << slot);
        g.city_mut(c).add_wonder(won::APOLLO_PROGRAM);
        g.wonders_built[won::APOLLO_PROGRAM as usize] = Some(c);
        g.apollo_reveal(slot);
        assert!(g.map.tile(fx, fy).explored_by(slot));
        assert!(!g.map.tile(fx + 1, fy).explored_by(slot));
    }
}
