//! Tile yields, city resource totals, trade split, corruption, upkeep, production management,
//! buying/selling and tax rates.
//!
//! Formulas marked [approx] are reconstructions kept in small functions so they can be tuned.

use super::city::Production;
use super::map::{tile_flags, CityId, Map, Slot};
use super::{Game, RuleError, Specialist};
use crate::rules::{self, improvement_ids as imp, unit_ids, wonder_ids as won, Domain, Govt, ImprovementId, Terrain, WonderId};

/// The city radius in CIV.EXE's order (ds:0x2498 dx / ds:0x24c2 dy), which decides ties in the
/// worker assignment; the city square is last (index 20).
#[rustfmt::skip]
pub const EXE_RADIUS: [(i8, i8); 21] = [
    (0, -1), (1, 0), (0, 1), (-1, 0), (1, -1), (1, 1), (-1, 1), (-1, -1),
    (0, -2), (2, 0), (0, 2), (-2, 0), (-1, -2), (1, -2), (2, -1), (2, 1),
    (1, 2), (-1, 2), (-2, 1), (-2, -1), (0, 0),
];

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
        // railroads: +50% (rounded down) of food, shields and trade, after the Colossus and before
        // the government rules (CIV.EXE 0x134fc)
        if t.has(tile_flags::RAILROAD) {
            f += f / 2;
            s += s / 2;
            tr += tr / 2;
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
        // pollution halves each value, rounding up (CIV.EXE 0x1357d)
        if t.has(tile_flags::POLLUTION) {
            f = (f + 1) / 2;
            s = (s + 1) / 2;
            tr = (tr + 1) / 2;
        }
        (f, s, tr)
    }

    /// Tile coordinates for a city radius index.
    pub fn radius_tile(&self, c: CityId, idx: u8) -> Option<(i32, i32)> {
        let city = self.city_opt(c)?;
        let (dx, dy) = rules::CITY_RADIUS[idx as usize];
        let (x, y) = (Map::wrap_x(city.x + dx as i32), city.y + dy as i32);
        if Map::in_bounds(x, y) {
            Some((x, y))
        } else {
            None
        }
    }

    /// Can the city work radius tile `idx` (not worked by another city, no foreign unit on it)?
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
        // any foreign unit, military or not (CIV.EXE 0xd4fa)
        if self.all_units().any(|u| (u.x, u.y) == (x, y) && u.owner != owner) {
            return false;
        }
        true
    }

    /// The units the city supports, each with whether it costs a shield (CIV.EXE 0xfc6e):
    /// Diplomats and Caravans never do; under Anarchy and Despotism the others are free up to
    /// the city's size; under the other governments each costs one. The garrison stashed in
    /// the city record counts first towards the free ones (0xfbf5, 0xd28b).
    pub fn shield_costs(&self, c: CityId) -> Vec<(super::UnitId, bool)> {
        let city = self.city(c);
        let free = rules::GOVERNMENTS[self.player(city.owner).govt as usize].free_units_up_to_size;
        let mut paying = city.garrison_count();
        self.supported_units(c)
            .into_iter()
            .map(|id| {
                if matches!(self.unit(id).kind, unit_ids::DIPLOMAT | unit_ids::CARAVAN) {
                    return (id, false);
                }
                paying += 1;
                (id, !free || paying > city.size as i32)
            })
            .collect()
    }

    /// Shields the stashed garrison costs (CIV.EXE 0xd28b..0xd2ec, 0xfbf5..0xfc56): only the
    /// units beyond the city's size, whatever the government; a city under the Monarchy or
    /// later supports its two stashed units for nothing from size 2 on.
    pub fn garrison_upkeep(&self, c: CityId) -> i32 {
        let city = self.city(c);
        (city.garrison_count() - city.size as i32).max(0)
    }

    /// Would one more supported unit (not a Diplomat or Caravan) cost the city a shield?
    pub fn next_unit_costs_shield(&self, c: CityId) -> bool {
        let city = self.city(c);
        let paying = city.garrison_count() + self.supported_units(c).into_iter().filter(|id| !matches!(self.unit(*id).kind, unit_ids::DIPLOMAT | unit_ids::CARAVAN)).count() as i32;
        !rules::GOVERNMENTS[self.player(city.owner).govt as usize].free_units_up_to_size || paying + 1 > city.size as i32
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

    /// The city's shields after Factory and power, disorder or not (CIV.EXE ds:0x70dc after
    /// 0xddd5), as the worker assignment and the production advisor see them.
    pub fn city_shields(&self, c: CityId) -> i32 {
        let shields: i32 = (0..21u8).filter(|i| self.city(c).works(*i)).map(|i| self.radius_yield(c, i).1).sum();
        self.boosted_shields(c, shields)
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
        // trade routes (CIV.EXE 0xf141): each adds (partner's trade after corruption + this
        // city's trade so far + 4) / 8, or / 16 between cities of one civilization
        for r in &city.trade_routes {
            if let Some(other) = self.city_opt(*r) {
                let div = if other.owner == slot { 16 } else { 8 };
                y.trade += (self.visible_trade(*r) + y.trade + 4) / div;
            }
        }
        // food upkeep: 2 per citizen plus settlers (1; 2 under Republic/Democracy)
        let g = rules::GOVERNMENTS[p.govt as usize];
        let settlers = self.all_units().filter(|u| u.home == Some(c) && u.kind == unit_ids::SETTLERS).count() as i32;
        y.food_upkeep = city.size as i32 * 2 + settlers * g.settler_food as i32;
        y.shield_upkeep = self.shield_costs(c).iter().filter(|(_, cost)| *cost).count() as i32 + self.garrison_upkeep(c);
        y.shields = self.boosted_shields(c, y.shields);
        // corruption and the tax/lux/sci split (CIV.EXE 0xf1df): luxuries and taxes rounded to
        // the nearest unit, science gets the rest
        y.corruption = self.corruption(c, y.trade, false);
        let net = y.trade - y.corruption;
        if p.govt == Govt::Anarchy {
            y.lux = net;
        } else {
            y.lux = ((net * (p.lux / 10) as i32 + 5) / 10).clamp(0, y.trade);
            y.tax = ((net * (p.tax / 10) as i32 + 5) / 10).clamp(0, (net - y.lux).max(0));
            y.sci = net - y.lux - y.tax;
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
    /// doubles the city's science. (The SETI Program's half goes to the beakers only,
    /// `city_science` 0x12cc0.)
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

    /// Trade of the worked tiles less their corruption: what trade routes and caravans count
    /// for a partner city (CIV.EXE city +0xa, set at 0xf13d).
    pub fn visible_trade(&self, c: CityId) -> i32 {
        let t = self.raw_trade(c);
        t - self.corruption(c, t, true)
    }

    /// Distance used for corruption (CIV.EXE 0xca04): 0 in a city with a Palace, else the
    /// travel distance to the nearest own Palace (32 without one); a flat 10 under Communism.
    fn corruption_distance(&self, c: CityId) -> i32 {
        let city = self.city(c);
        if self.player(city.owner).govt == Govt::Communism {
            return 10;
        }
        if city.has_improvement(imp::PALACE) {
            return 0;
        }
        self.all_cities().filter(|o| o.owner == city.owner && o.has_improvement(imp::PALACE)).map(|o| Map::civ_distance(o.pos(), city.pos())).fold(32, i32::min)
    }

    /// Trade lost to corruption (CIV.EXE 0xf0c4 / 0xf1df): `trade * distance * 3 / (20 *
    /// government + 80)`, none under Democracy, halved by a Courthouse. The first pass over the
    /// tiles' trade (`tiles_only`, for the city's visible trade) also halves it with a Palace,
    /// which only matters under Communism.
    pub fn corruption(&self, c: CityId, trade: i32, tiles_only: bool) -> i32 {
        let city = self.city(c);
        let govt = self.player(city.owner).govt;
        if govt == Govt::Democracy {
            return 0;
        }
        let mut corr = trade * self.corruption_distance(c) * 3 / (20 * govt as i32 + 80);
        if city.has_improvement(imp::COURTHOUSE) || tiles_only && city.has_improvement(imp::PALACE) {
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

    /// Food, shields and trade of radius square `idx` for the city (0 off the map).
    fn radius_yield(&self, c: CityId, idx: u8) -> (i32, i32, i32) {
        let slot = self.city(c).owner;
        self.radius_tile(c, idx).map(|(x, y)| self.tile_yield(slot, x, y, Some(c))).unwrap_or((0, 0, 0))
    }

    /// Re-place all citizens from scratch (a new, joined or captured city; CIV.EXE reaches the
    /// same state through a cleared worked-square mask, found_city o 0x1b375).
    pub fn auto_assign(&mut self, c: CityId) {
        for idx in 1..21u8 {
            if self.city(c).works(idx) {
                self.release_tile(c, idx);
            }
        }
        self.city_mut(c).specialists.clear();
        self.assign_workers(c, false);
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

    fn take_tile(&mut self, c: CityId, idx: u8) {
        self.city_mut(c).set_worked(idx, true);
        if let Some((x, y)) = self.radius_tile(c, idx) {
            self.map.get_mut(x, y).unwrap().worked_by = Some(c);
        }
    }

    /// Make the worker layout consistent with the city size and the map (CIV.EXE's assignment
    /// outside the turn: the city screen, the production advisor); see [`Game::assign_workers`].
    pub fn fix_workers(&mut self, c: CityId) {
        self.assign_workers(c, false);
    }

    /// CIV.EXE's worker assignment, part of the city routine (0xd454..0xdc5d). `turn` is its
    /// mode 0, the city's turn: then a computer city forgets its specialists (they go back to
    /// work), or in disorder gives up all its squares and keeps them (0xcd2c..0xcd61).
    ///
    /// Squares not explored, occupied by a foreign unit or worked by another city are dropped
    /// (0xd454). The human's squares are kept, as are a computer city's except every fourth
    /// turn ((city + turn) & 3 == 0) when it is not in disorder and no unit of the human stands
    /// in its radius: then it starts over (0xd5e5). More working squares than citizens (the
    /// city shrank) also starts over. Then, while there are more citizens than specialists:
    /// the city square first; while food + unplaced / 2 < 2 * size + settlers' food, and always
    /// below size 3, the square with the most food (ties: most 2 * shields + trade; the last
    /// citizen of a city without shields skips squares without them, 0xd7cf..0xd98b); a
    /// computer city in disorder adds an entertainer (0xd98e); the rest each take the square
    /// maximising food * (16 / surplus) + shields * (3 * size / (shields - support)) + trade *
    /// (2 * size / trade), each divisor a running total clamped to 1..99 (0xd9b0..0xdb95). Citizens left
    /// over (no square worth anything) are specialists; new ones are entertainers (0xdbf2).
    pub fn assign_workers(&mut self, c: CityId, turn: bool) {
        let slot = self.city(c).owner;
        let human = self.player(slot).human;
        let size = self.city(c).size as i32;
        let disorder = self.city(c).disorder;
        // CIV.EXE radius order (ds:0x2498 / ds:0x24c2) mapped to ours; the centre comes last
        let order: Vec<u8> = EXE_RADIUS.iter().map(|d| rules::CITY_RADIUS.iter().position(|r| r == d).unwrap() as u8).collect();
        // unavailable squares (0xd454..0xd5b1); a unit of the human in the radius of a computer
        // city freezes its layout (ds:0x70e8)
        let mut taken = [false; 21];
        let mut human_near = false;
        for &idx in &order[..20] {
            if !self.tile_workable(c, idx) {
                taken[idx as usize] = true;
                if self.city(c).works(idx) {
                    self.release_tile(c, idx);
                }
                if let Some((x, y)) = self.radius_tile(c, idx) {
                    if self.units_at(x, y).iter().any(|u| self.unit(*u).owner == self.human_slot && self.human_slot != slot) {
                        human_near = true;
                    }
                }
            }
        }
        let mut specialists = self.city(c).specialists.len() as i32;
        if turn && !human {
            if disorder {
                for idx in 1..21u8 {
                    if self.city(c).works(idx) {
                        self.release_tile(c, idx);
                    }
                }
            } else {
                specialists = 0;
            }
        }
        let (mut food, mut shields, mut trade) = (0, 0, 0);
        macro_rules! add {
            ($idx:expr) => {{
                let (a, b, d) = self.radius_yield(c, $idx);
                food += a;
                shields += b;
                trade += d;
            }};
        }
        let mut remaining = size + 1;
        let keep = human || (c as u32 + self.turn) & 3 != 0 || disorder || human_near;
        let mut assign = !keep;
        if keep {
            let worked = 1 + self.city(c).workers() as i32;
            if worked > remaining {
                // "Population decrease in CITY." for the human's city in its turn (0xd70d..0xd769,
                // ds:0x255f) and a fresh start
                if turn && slot == self.human_slot {
                    self.push_event(super::GameEvent::PopulationDecrease(c));
                }
                for idx in 1..21u8 {
                    if self.city(c).works(idx) {
                        self.release_tile(c, idx);
                    }
                }
                assign = true;
            } else {
                for &idx in &order {
                    if self.city(c).works(idx) {
                        taken[idx as usize] = true;
                        add!(idx);
                        remaining -= 1;
                    }
                }
                assign = remaining != specialists && remaining != 0;
            }
        } else {
            for idx in 1..21u8 {
                if self.city(c).works(idx) {
                    self.release_tile(c, idx);
                }
            }
        }
        if assign {
            if remaining > 0 && !taken[0] {
                taken[0] = true;
                add!(0);
                remaining -= 1;
            }
            let settlers = self.supported_units(c).iter().filter(|u| self.unit(**u).kind == unit_ids::SETTLERS).count() as i32;
            let eaten = 2 * size + self.settler_food(slot) * settlers;
            // food first
            while (eaten > food + remaining / 2 || size < 3) && remaining > specialists {
                let mut best: Option<(u8, i32, i32)> = None;
                for &idx in &order {
                    if taken[idx as usize] {
                        continue;
                    }
                    let (f, s, t) = self.radius_yield(c, idx);
                    if remaining == 1 && shields == 0 && s == 0 {
                        continue;
                    }
                    let rest = 2 * s + t;
                    let (bf, br) = best.map(|b| (b.1, b.2)).unwrap_or((0, 0));
                    if f > bf || (f == bf && rest > br) {
                        best = Some((idx, f, rest));
                    }
                }
                let Some((idx, ..)) = best else { break };
                taken[idx as usize] = true;
                self.take_tile(c, idx);
                add!(idx);
                remaining -= 1;
            }
            // (CIV.EXE does this in any mode, but only its turn pass ever has citizens to place
            // in a rioting computer city; civrs also places grown citizens outside it)
            if turn && !human && disorder {
                specialists += 1;
            }
            let support = self.shield_costs(c).iter().filter(|(_, cost)| *cost).count() as i32 + self.garrison_upkeep(c);
            while remaining > specialists {
                let fw = 16 / (food - eaten).clamp(1, 99);
                let sw = 3 * size / (shields - support).clamp(1, 99);
                let tw = 2 * size / trade.clamp(1, 99);
                let mut best: Option<(u8, i32)> = None;
                for &idx in &order {
                    if taken[idx as usize] {
                        continue;
                    }
                    let (f, s, t) = self.radius_yield(c, idx);
                    let v = f * fw + s * sw + t * tw;
                    if v > best.map(|b| b.1).unwrap_or(0) {
                        best = Some((idx, v));
                    }
                }
                let Some((idx, _)) = best else { break };
                taken[idx as usize] = true;
                self.take_tile(c, idx);
                add!(idx);
                remaining -= 1;
            }
        }
        // the citizens left are specialists, keeping their kinds; new ones entertain
        let cm = self.city_mut(c);
        let n = remaining.max(0) as usize;
        cm.specialists.truncate(n);
        while cm.specialists.len() < n {
            cm.specialists.push(Specialist::Entertainer);
        }
    }

    /// A click on radius square `idx` in the city screen (CIV.EXE 0x123b4..0x125aa): a worked
    /// square becomes an entertainer; an unworked one takes a specialist, or with none the whole
    /// city is placed anew (the worker count exceeds the citizens and the screen recomputes from
    /// 0xcd65). The city square only recomputes.
    pub fn toggle_tile(&mut self, c: CityId, idx: u8) {
        if idx == 0 || idx >= 21 {
            self.fix_workers(c);
            return;
        }
        let Some((x, y)) = self.radius_tile(c, idx) else { return };
        let owner = self.city(c).owner;
        if !self.map.tile(x, y).explored_by(owner) {
            return;
        }
        if self.city(c).works(idx) {
            self.release_tile(c, idx);
            self.city_mut(c).specialists.push(Specialist::Entertainer);
        } else {
            if !self.tile_workable(c, idx) {
                return;
            }
            self.take_tile(c, idx);
            if self.city_mut(c).specialists.pop().is_none() {
                self.fix_workers(c);
            }
        }
    }

    pub fn set_specialist(&mut self, c: CityId, i: usize, s: Specialist) {
        if let Some(sp) = self.city_mut(c).specialists.get_mut(i) {
            *sp = s;
        }
    }

    // ---- production ----------------------------------------------------------------------

    /// Shields per row of the production box and food per row of the food box (CIV.EXE
    /// 0xcccc..0xcd28, [bp-0x4a] of the city routine): 10 for the human; for a computer civ
    /// 16 - 2 * difficulty, and 2 less from 1 AD on above Prince while the human is the
    /// strongest civilization (rank 7).
    pub fn cost_factor(&self, slot: Slot) -> i32 {
        if self.player(slot).human {
            return 10;
        }
        let diff = self.setup.difficulty as i32;
        let mut f = 16 - 2 * diff;
        if self.year >= 0 && diff > 1 && self.power_rank(self.human_slot) == 7 {
            f -= 2;
        }
        f
    }

    /// Shields `p` costs in city `c`: the table cost times the owner's `cost_factor`.
    pub fn production_cost(&self, c: CityId, p: Production) -> i32 {
        p.cost() * self.cost_factor(self.city(c).owner) / 10
    }

    /// Food needed to grow (CIV.EXE 0xd195): `(size + 1) * cost_factor`.
    pub fn food_box(&self, c: CityId) -> i32 {
        let city = self.city(c);
        (city.size as i32 + 1) * self.cost_factor(city.owner)
    }

    /// Gold needed to complete the current production right now.
    pub fn buy_cost(&self, c: CityId) -> i32 {
        let city = self.city(c);
        let remaining = self.production_cost(c, city.production) - city.shields;
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
        let total = self.production_cost(c, self.city(c).production);
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
        // Nuclear: the Manhattan Project and Nuclear Fission as well (CIV.EXE 0xabb2)
        if k == unit_ids::NUCLEAR && (!self.nukes_enabled || !p.knows(crate::rules::advance_ids::NUCLEAR_FISSION)) {
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
            // Apollo built and the ship not launched, however full it is (CIV.EXE 0xac63)
            imp::SS_STRUCTURAL | imp::SS_COMPONENT | imp::SS_MODULE => self.apollo_built && !p.spaceship.launched(),
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
    use crate::game::{GameEvent, GameSetup, UnitId};

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
    fn more_workers_than_citizens_tell_the_human_at_the_city_turn_only() {
        let (mut g, c) = fixture();
        g.city_mut(c).size = 3;
        g.auto_assign(c);
        assert_eq!(g.city(c).workers(), 3);
        g.take_events();
        g.city_mut(c).size = 2;
        // outside the turn (city screen, advisor) the workers are re-placed silently
        let mut quiet = g.clone();
        quiet.assign_workers(c, false);
        assert!(!quiet.take_events().iter().any(|e| matches!(e, GameEvent::PopulationDecrease(_))));
        g.assign_workers(c, true);
        assert_eq!(g.take_events(), vec![GameEvent::PopulationDecrease(c)]);
        assert_eq!(g.city(c).workers() as usize + g.city(c).specialists.len(), 2);
        // a computer city says nothing
        let slot = g.city(c).owner;
        g.city_mut(c).size = 3;
        g.auto_assign(c);
        g.city_mut(c).size = 2;
        g.player_mut(slot).human = false;
        g.human_slot = (slot + 1) % 8;
        g.take_events();
        g.assign_workers(c, true);
        assert!(!g.take_events().iter().any(|e| matches!(e, GameEvent::PopulationDecrease(_))));
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
    fn below_size_three_food_comes_first_then_the_score_wants_trade() {
        let (mut g, c) = fixture();
        g.city_mut(c).size = 2;
        // grassland (2 food) beats the ocean (1 food, 2 trade) while the city is small
        island_radius(&mut g, c, Terrain::Hills);
        g.auto_assign(c);
        assert!(!g.city(c).works(4));
        assert_eq!(g.city(c).workers(), 2);
        // fed and without trade, food * 16 / surplus loses to trade * 2 * size (0xd9bb)
        g.city_mut(c).size = 6;
        g.auto_assign(c);
        assert!(g.city(c).works(4));
        assert_eq!(g.city(c).workers(), 6);
        assert!(g.city(c).specialists.is_empty());
    }

    #[test]
    fn citizens_without_a_square_worth_anything_entertain() {
        let (mut g, c) = fixture();
        for idx in 1..21u8 {
            let (x, y) = g.radius_tile(c, idx).unwrap();
            let t = g.map.get_mut(x, y).unwrap();
            t.terrain = Terrain::Arctic;
            t.flags = 0;
            t.special = false;
        }
        g.city_mut(c).size = 3;
        g.auto_assign(c);
        assert_eq!(g.city(c).workers(), 0);
        assert_eq!(g.city(c).specialists, vec![Specialist::Entertainer; 3]);
    }

    #[test]
    fn the_humans_layout_is_kept_and_new_citizens_placed() {
        let (mut g, c) = fixture();
        g.city_mut(c).size = 3;
        g.auto_assign(c);
        // move one worker by hand: worked square -> entertainer -> another square
        let worked = (1..21u8).find(|i| g.city(c).works(*i)).unwrap();
        g.toggle_tile(c, worked);
        assert_eq!(g.city(c).specialists.len(), 1);
        let free = (1..21u8).find(|i| !g.city(c).works(*i) && *i != worked && g.tile_workable(c, *i)).unwrap();
        g.toggle_tile(c, free);
        assert!(g.city(c).specialists.is_empty() && g.city(c).works(free) && !g.city(c).works(worked));
        let layout = g.city(c).worked;
        g.assign_workers(c, true);
        assert_eq!(g.city(c).worked, layout);
        // a new citizen gets a square, the others stay
        g.city_mut(c).size = 4;
        g.fix_workers(c);
        assert_eq!(g.city(c).worked & layout, layout);
        assert_eq!(g.city(c).workers(), 4);
        // the city square only recomputes
        g.toggle_tile(c, 0);
        assert_eq!(g.city(c).workers(), 4);
        // an unworked square without a specialist to take it: everything is placed anew
        let other = (1..21u8).find(|i| !g.city(c).works(*i) && g.tile_workable(c, *i)).unwrap();
        g.toggle_tile(c, other);
        assert_eq!(g.city(c).workers() as usize + g.city(c).specialists.len(), 4);
    }

    #[test]
    fn computer_cities_start_over_every_fourth_turn_and_entertain_in_disorder() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        g.player_mut(slot).human = false;
        g.city_mut(c).size = 4;
        g.auto_assign(c);
        let best = g.city(c).worked;
        // a worse layout: an entertainer instead of a worker
        let worked = (1..21u8).find(|i| g.city(c).works(*i)).unwrap();
        g.toggle_tile(c, worked);
        // not the fourth turn: the entertainer goes back to work (specialists are forgotten)
        g.turn = (1..4).find(|t| (c as u32 + t) & 3 != 0).unwrap();
        g.assign_workers(c, true);
        assert!(g.city(c).specialists.is_empty());
        assert_eq!(g.city(c).workers(), 4);
        // the fourth turn: a fresh start gives the best layout again
        g.turn = (4..8).find(|t| (c as u32 + t) & 3 == 0).unwrap();
        g.assign_workers(c, true);
        assert_eq!(g.city(c).worked, best);
        // in disorder all squares are given up and one more entertainer is kept back each
        // turn, unless the hunger for food already put that citizen to work (0xd98e)
        g.city_mut(c).disorder = true;
        g.assign_workers(c, true);
        assert_eq!(g.city(c).specialists, vec![Specialist::Entertainer]);
        g.assign_workers(c, true);
        let n = g.city(c).specialists.len();
        assert!((1..=2).contains(&n));
        assert_eq!(g.city(c).workers() as usize + n, 4);
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
        assert_eq!(boost(&g, 10, 12).2, 64, "SETI counts in the beakers, not in the city");
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

    #[test]
    fn unit_support_by_government_as_in_civ_exe() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        let (x, y) = g.city(c).pos();
        g.city_mut(c).size = 2;
        for k in [unit_ids::MILITIA, unit_ids::MILITIA, unit_ids::DIPLOMAT, unit_ids::MILITIA] {
            let id = g.add_unit(slot, k, x, y);
            g.unit_mut(id).home = Some(c);
        }
        let paid = |g: &Game| g.shield_costs(c).iter().filter(|(_, p)| *p).count();
        g.player_mut(slot).govt = Govt::Despotism;
        assert_eq!(paid(&g), 1, "two free for a size-2 city, the Diplomat free");
        g.player_mut(slot).govt = Govt::Monarchy;
        assert_eq!(paid(&g), 3);
        assert!(g.next_unit_costs_shield(c));
    }

    #[test]
    fn railroads_add_half_of_food_shields_and_trade() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        let (x, y) = g.city(c).pos();
        g.player_mut(slot).govt = Govt::Monarchy;
        let t = g.map.get_mut(x + 1, y).unwrap();
        t.terrain = Terrain::Grassland;
        t.special = false;
        t.shield = true;
        t.flags = tile_flags::ROAD | tile_flags::IRRIGATION;
        assert_eq!(g.tile_yield(slot, x + 1, y, Some(c)), (3, 1, 1));
        g.map.get_mut(x + 1, y).unwrap().flags |= tile_flags::RAILROAD;
        assert_eq!(g.tile_yield(slot, x + 1, y, Some(c)), (4, 1, 1));
    }

    #[test]
    fn a_new_city_destroys_fortress_and_mine() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        let (x, y) = (g.city(c).x + 6, g.city(c).y);
        let t = g.map.get_mut(x, y).unwrap();
        t.terrain = Terrain::Hills;
        t.flags = tile_flags::FORTRESS | tile_flags::MINE;
        g.add_city(slot, "Hilltop".into(), x, y);
        let t = g.map.tile(x, y);
        assert!(!t.has(tile_flags::FORTRESS) && !t.has(tile_flags::MINE) && t.has(tile_flags::ROAD));
    }

    #[test]
    fn pollution_halves_rounding_up() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        let (x, y) = g.city(c).pos();
        let t = g.map.get_mut(x + 1, y).unwrap();
        t.terrain = Terrain::Forest;
        t.special = false;
        t.flags = tile_flags::POLLUTION;
        // Forest 1/2/0 -> 1/1/0
        assert_eq!(g.tile_yield(slot, x + 1, y, Some(c)), (1, 1, 0));
    }
    /// CIV.EXE 0xcccc..0xcd28: the human 10, the computer 16 - 2 * difficulty, 2 less from
    /// 1 AD above Prince while the human is the strongest; costs and the food box scale with it.
    #[test]
    fn computer_costs_and_food_box_follow_the_cost_factor() {
        use crate::rules::Difficulty;
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let human = g.human_slot;
        let ai = (1..8).find(|s| *s != human && g.player(*s).alive && !g.units_of(*s).is_empty()).unwrap();
        let settler = g.units_of(ai).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        let militia = Production::Unit(unit_ids::MILITIA);
        assert_eq!(g.cost_factor(human), 10);
        assert_eq!((g.cost_factor(ai), g.production_cost(c, militia), g.food_box(c)), (16, 16, 32));
        g.setup.difficulty = Difficulty::Emperor;
        assert_eq!((g.cost_factor(ai), g.production_cost(c, militia), g.food_box(c)), (8, 8, 16));
        // the human strongest from 1 AD on: 2 less
        g.year = 1;
        g.player_mut(human).gold = 30_000;
        assert_eq!(g.cost_factor(ai), 6);
        g.setup.difficulty = Difficulty::Warlord;
        assert_eq!(g.cost_factor(ai), 14, "not at Warlord");
    }

}
