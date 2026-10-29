//! What a city builds next: CIV.EXE's production advisor 0xa5e:0x41d (`choose_production`, file
//! 0xa9fd..0xbab3), shared by the computer players, the human's AUTO cities and the advisors'
//! recommendations, plus the per-continent census of the AI turn (0x1579a) that it reads.
//!
//! Every candidate the production menu offers gets a value, **lower is better**; the computer
//! takes the lowest of all, the human's AUTO city the lowest improvement (if it is 500 or less).
//!
//! The garrison a human city keeps in its record (+0x1a/+0x1b, `City::garrison`) counts in the
//! census and in the advisor's unit totals as units in the city.
//!
//! Not modelled: CIV.EXE's habit of counting ships whose ocean body has the same id as the
//! continent.

use super::city::Production;
use super::map::{CityId, Slot};
use super::Game;
use crate::rules::{self, advance_ids as adv, improvement_ids as imp, unit_ids, AiRole, Domain, Govt, Terrain};

/// AI continent policies (ds:0xe698[civ][continent], set at 0x15cd1).
pub const POLICY_NONE: u8 = 0;
/// An enemy stronger than us, or a barbarian city, or nothing of ours on the continent.
pub const POLICY_HOSTILE: u8 = 1;
/// An enemy (or the human we are at peace with) outguns our cities there.
pub const POLICY_THREATENED: u8 = 2;
/// The continent is full: expand overseas (needs Map Making).
pub const POLICY_OFFENSIVE: u8 = 5;

/// CIV.EXE's tables hold 16 continents per civilization; civrs numbers landmasses by size, so
/// the 15th largest and all smaller ones share the last entry.
pub const CONTINENTS: usize = 16;

pub fn continent_index(cont: u8) -> usize {
    (cont as usize).min(CONTINENTS - 1)
}

/// The census 0x1579a takes at the start of each civilization's turn. Each civilization's rows
/// are written only by its own AI turn (which CIV.EXE runs for the human and the barbarians
/// too), so a reader sees every other civilization as it stood at the start of that one's last
/// turn, before its units moved: the human's army that has just landed shows only after his
/// next turn starts. CIV.EXE saves these tables (SVE 0x0788); civrs in its version-20 trailer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Census {
    /// ds:0x6cac: summed defence of the civ's units (Nuclear excluded) standing on land of
    /// the continent (0x158b1).
    pub defence: [[i32; CONTINENTS]; 8],
    /// ds:0x807a: summed attack of the same units (the table 0x1495:0xe0b reads; the catalogue's
    /// ds:0x8076 is four bytes off).
    pub attack: [[i32; CONTINENTS]; 8],
    /// ds:0x6b96: cities of the civ on the continent.
    pub cities: [[i32; CONTINENTS]; 8],
    /// ds:0xdcfe: land squares of the continent.
    pub squares: [i32; CONTINENTS],
    /// ds:0xb1ee: Plains, Grassland and River squares in columns 2..77, rows 2..47 (counted by
    /// the world generator, overlay 7 0x9d55..0x9ebf).
    pub sites: [i32; CONTINENTS],
    /// ds:0x6e82: military strength, attack plus defence of every unit but the Nuclear, on
    /// land or not, and of the stashed garrison of the civ's cities (0x15848, 0x15acc).
    pub strength: [i32; 8],
    /// ds:0xd2f8: the citizens of the civ's cities (0x15a3e), read by the power ranking and
    /// the plunder of a captured city.
    pub citizens: [i32; 8],
    /// ds:0x2a6e bit civ (0x15f3f): forces of the civ on a continent where the human has two
    /// or more cities.
    pub faces_human: u8,
}

/// The best candidates of one advisor run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildAdvice {
    /// Lowest value of all candidates (what the computer builds).
    pub best: Option<(Production, i32)>,
    /// Lowest unit (the military advisor's recommendation, [bp-0xc]).
    pub unit: Option<(Production, i32)>,
    /// Lowest improvement, wonder, Caravan or Diplomat (the domestic advisor's, [bp-0x48]).
    pub improvement: Option<(Production, i32)>,
}

impl Game {
    /// Settlers eat 2 food from Monarchy on, 1 under Anarchy and Despotism (CIV.EXE 0xcc27).
    pub fn settler_food(&self, slot: Slot) -> i32 {
        if self.player(slot).govt as u8 > Govt::Despotism as u8 {
            2
        } else {
            1
        }
    }

    /// The census tables as the civilizations' last AI turns left them (`Census`).
    pub fn census(&self) -> Census {
        self.ai_memory.census.clone()
    }

    /// The census of 0x1579a (units 0x15801, cities 0x159be) for all civilizations as they
    /// stand now.
    pub fn live_census(&self) -> Census {
        let mut c = Census::default();
        for u in self.all_units() {
            if u.owner as usize >= 8 || u.kind == unit_ids::NUCLEAR {
                continue;
            }
            c.strength[u.owner as usize] += u.info().att as i32 + u.info().def as i32;
            let t = self.map.tile(u.x, u.y);
            if t.is_ocean() {
                continue;
            }
            let k = continent_index(t.continent);
            c.defence[u.owner as usize][k] += u.info().def as i32;
            c.attack[u.owner as usize][k] += u.info().att as i32;
        }
        for city in self.all_cities() {
            if (city.owner as usize) < 8 {
                let k = continent_index(self.map.tile(city.x, city.y).continent);
                c.cities[city.owner as usize][k] += 1;
                c.citizens[city.owner as usize] += city.size as i32;
                // the stashed garrison (0x15a95..0x15b2a)
                for (kind, _) in city.garrison_units() {
                    let t = &rules::UNITS[kind as usize];
                    c.defence[city.owner as usize][k] += t.def as i32;
                    c.attack[city.owner as usize][k] += t.att as i32;
                    c.strength[city.owner as usize] += t.att as i32 + t.def as i32;
                }
            }
        }
        for (i, t) in self.map.tiles.iter().enumerate() {
            if !t.is_land() {
                continue;
            }
            let k = continent_index(t.continent);
            c.squares[k] += 1;
            let (x, y) = (i % rules::MAP_W, i / rules::MAP_W);
            if (2..78).contains(&x) && (2..48).contains(&y) && matches!(t.terrain, Terrain::Plains | Terrain::Grassland | Terrain::River) {
                c.sites[k] += 1;
            }
        }
        c
    }

    /// The census part of `ai_civ_turn` (0x157a2..0x15b2a, 0x15f3f): `slot`'s rows of the
    /// tables (continent forces and cities, strength) and its bit of ds:0x2a6e, read with the
    /// human's rows as his last turn left them. The other civilizations' rows stay.
    pub fn take_census(&mut self, slot: Slot) {
        let live = self.live_census();
        let s = slot as usize;
        if s >= 8 {
            return;
        }
        let m = &mut self.ai_memory.census;
        m.defence[s] = live.defence[s];
        m.attack[s] = live.attack[s];
        m.cities[s] = live.cities[s];
        m.strength[s] = live.strength[s];
        m.citizens[s] = live.citizens[s];
        m.squares = live.squares;
        m.sites = live.sites;
        let h = self.human_slot as usize;
        let faces = h < 8 && (1..15).any(|k| m.defence[s][k] != 0 && m.cities[h][k] > 1);
        m.faces_human = m.faces_human & !(1 << s) | (faces as u8) << s;
    }

    /// Empty census rows for `slot`, whose civilization is gone or starts afresh. (CIV.EXE keeps
    /// a destroyed civ's rows from its last AI turn, which the other civs' policies go on
    /// counting, and a new civ in the slot inherits them until its first AI turn.)
    pub fn clear_census(&mut self, slot: Slot) {
        let s = slot as usize;
        if s >= 8 {
            return;
        }
        let m = &mut self.ai_memory.census;
        m.defence[s] = [0; CONTINENTS];
        m.attack[s] = [0; CONTINENTS];
        m.cities[s] = [0; CONTINENTS];
        m.strength[s] = 0;
        m.citizens[s] = 0;
        m.faces_human &= !(1 << s);
    }

    /// Every civilization's census rows as things stand now (a game from an older save).
    pub fn take_census_all(&mut self) {
        for s in 0..8 {
            self.take_census(s);
        }
    }

    /// The continent policy of `slot`'s last AI turn (ds:0xe698[civ][continent]); none before
    /// its first.
    pub fn stored_policy(&self, slot: Slot, k: usize) -> u8 {
        self.ai_memory.policies.get(slot as usize).copied().flatten().map(|p| p[k.min(CONTINENTS - 1)]).unwrap_or(POLICY_NONE)
    }

    /// ds:0xe498 bit 1: a peace treaty between the two.
    fn exe_peace(&self, a: Slot, b: Slot) -> bool {
        self.treaty(a, b)
    }

    /// ds:0xe498 low bits == 1: contact without a treaty, i.e. war.
    fn exe_war(&self, a: Slot, b: Slot) -> bool {
        self.at_war(a, b)
    }

    /// The continent policy of 0x15cd1..0x15f5f for `slot` on continent index `k` (1..14;
    /// the others are always 0, 0x15c87).
    pub fn continent_policy(&self, census: &Census, slot: Slot, k: usize) -> u8 {
        if k == 0 || k >= 15 || slot as usize >= 8 {
            return POLICY_NONE;
        }
        let s = slot as usize;
        let (mut hostile, mut threat, mut presence) = (0, 0, 0);
        let (mut others_def, mut others_cities) = (0, 0);
        // the scan includes the civilization itself (its own forces count twice below)
        for o in 1..8usize {
            let od = census.defence[o][k];
            if od != 0 {
                let weak = census.attack[s][k] / 4 >= census.attack[o][k] && census.cities[o][k] == 0;
                // a treaty it means to break (bit 0x100) counts as war (0x15dd9)
                if !weak && self.planning_war(slot, o as Slot) {
                    if od >= census.attack[s][k] && census.attack[o][k] >= census.defence[s][k] && census.cities[s][k] != 0 {
                        threat += 1;
                    } else {
                        hostile += 1;
                    }
                }
                // the human at peace whose army there outweighs ours
                if o as Slot == self.human_slot && self.exe_peace(slot, o as Slot) && census.attack[s][k] / 2 + census.defence[s][k] < census.attack[o][k] {
                    threat += 1;
                }
                presence |= if self.exe_peace(slot, o as Slot) { 1 } else { 2 };
            }
            others_def += od;
            others_cities += census.cities[o][k];
        }
        if census.cities[0][k] != 0 {
            hostile += 1;
        }
        let mut policy = POLICY_NONE;
        if 2 * (census.defence[s][k] + others_def) > census.squares[k] || (census.cities[s][k] + others_cities) * 6 + 2 > census.sites[k] {
            if self.player(slot).knows(adv::MAP_MAKING) {
                policy = POLICY_OFFENSIVE;
            }
        }
        if hostile != 0 {
            policy = POLICY_HOSTILE;
        }
        if threat != 0 {
            policy = POLICY_THREATENED;
        }
        if census.defence[s][k] == 0 && census.cities[s][k] == 0 && presence != 1 {
            policy = POLICY_HOSTILE;
        }
        policy
    }

    /// ds:0x2a6e bit `slot` (0x15f3f, read at 0xb3f3): the civ's last AI turn found forces of
    /// it on a continent where the human has two or more cities.
    fn faces_human(&self, census: &Census, slot: Slot) -> bool {
        census.faces_human & (1 << slot) != 0
    }

    /// ds:0x6ed4 (0x15ca1): the civ is at war with someone. CIV.EXE keeps one word for all
    /// civilizations, so the advisor (0xb7c7) reads the war of whichever civ's AI turn ran last
    /// (the civ before for cities processed at the start of a turn); civrs asks the city's own,
    /// but for `exe_quirks` (`AiMemory::war_word`).
    fn exe_at_war(&self, slot: Slot) -> bool {
        if self.exe_quirks {
            return self.ai_memory.war_word;
        }
        (1..8).any(|o| o != slot && self.exe_war(slot, o))
    }

    /// ds:0x8078 as the last power ranking set it (`Game::rank_powers`): the human is the
    /// strongest civilization, has more than four cities and no nuclear weapon, after turn 200.
    pub fn world_gangs_up(&self) -> bool {
        self.ai_memory.gang_up
    }

    /// The first production of a new city (overlay 20 o 0x1b375): Militia, Musketeers once the
    /// civilization knows Gunpowder, Riflemen with Conscription.
    pub fn first_production(&self, slot: Slot) -> Production {
        let p = self.player(slot);
        Production::Unit(if p.knows(adv::CONSCRIPTION) {
            unit_ids::RIFLEMEN
        } else if p.knows(adv::GUNPOWDER) {
            unit_ids::MUSKETEERS
        } else {
            unit_ids::MILITIA
        })
    }

    /// The computer's choice (0xb841..0xb86e): the lowest value of all, but a city building a
    /// Palace keeps it. The Palace is tested after the valuation (0xb857), so the advisor's
    /// early answers (Militia, barbarian troops) still replace it and its random draws happen.
    pub fn ai_production(&mut self, c: CityId) -> Production {
        self.ai_production_with(c, false)
    }

    /// `city_rechoose_production` 0x18c4c, the computer's choice after a unit is built (0xe3b1),
    /// on alert (0xec34) and when a continent's policy changes (0x18bef): the city's own unit
    /// is taken off the count of cities building it (ds:0xd308, 0x18c7c) before the advisor
    /// runs.
    pub fn ai_rechoose(&mut self, c: CityId) -> Production {
        self.ai_production_with(c, true)
    }

    fn ai_production_with(&mut self, c: CityId, rechoose: bool) -> Production {
        let advice = self.build_advice_with(c, rechoose);
        let current = self.city(c).production;
        match advice {
            Ok(_) if current == Production::Improvement(imp::PALACE) => current,
            Ok(a) => a.best.map(|b| b.0).unwrap_or(current),
            Err(p) => p,
        }
    }

    /// After a capture a computer captor re-chooses in all its cities (capture_city 0x13d76 for
    /// the new city, steal_advance 0x14468 -> 0x18bef(civ, -1) for the rest).
    pub fn rechoose_after_capture(&mut self, captor: Slot) {
        if self.player(captor).human {
            return;
        }
        for c in self.cities_of(captor) {
            let p = self.ai_rechoose(c);
            self.city_mut(c).production = p;
        }
    }

    /// An AUTO city of the human (0xb871..0xb896): the best improvement if its value is 500 or
    /// less; `None` ("no change", 0x63) turns AUTO off.
    pub fn auto_production(&mut self, c: CityId) -> Option<Production> {
        match self.build_advice(c) {
            Ok(a) => a.improvement.filter(|(_, v)| *v <= 500).map(|(p, _)| p),
            Err(p) => Some(p),
        }
    }

    /// CIV.EXE 0xa9fd: value every item of the production menu. `Err` is an answer given before
    /// any valuation: Militia for a computer civilization under Despotism or Anarchy that has no
    /// unit at all (0xb0e7), Legion or (once anybody knows Explosives) Knights for barbarians.
    pub fn build_advice(&mut self, c: CityId) -> Result<BuildAdvice, Production> {
        self.build_advice_with(c, false)
    }

    /// `build_advice`; with `rechoose` the city's own unit does not count as in production.
    fn build_advice_with(&mut self, c: CityId, rechoose: bool) -> Result<BuildAdvice, Production> {
        // choose_production starts with the city routine in mode -1 (0xaa1b)
        self.city_routine_quiet(c);
        let city = self.city(c).clone();
        let slot = city.owner;
        let human = self.human_slot;
        let p = self.player(slot).clone();
        let k = continent_index(self.map.tile(city.x, city.y).continent);
        let census = self.census();
        // the policy the civ's last AI turn set (0xaf56)
        let policy = self.stored_policy(slot, k);
        // [bp-0x13c]: a war footing on this continent
        let war = policy == POLICY_HOSTILE || policy == POLICY_THREATENED || (self.world_gangs_up() && policy == POLICY_OFFENSIVE);
        let y = self.city_yield(c);
        let shields = self.city_shields(c);
        let surplus = y.food - 2 * city.size as i32 - self.settler_food(slot) * self.supported_units(c).iter().filter(|u| self.unit(**u).kind == unit_ids::SETTLERS).count() as i32;
        // per unit type: units of the civ (ds:0xb3be), cities building it (ds:0xd308), units on
        // this continent ([bp-0x44]) and other cities here building it, plus one ([bp-0x11a])
        let mut owned = [0i32; 28];
        let mut in_production = [0i32; 28];
        let mut here = [0i32; 28];
        let mut building_here = [1i32; 28];
        let mut units = 0;
        for (k, c) in self.player(slot).unit_counter.iter().enumerate() {
            owned[k] = *c as i32;
        }
        for u in self.all_units().filter(|u| u.owner == slot) {
            units += 1;
            let t = self.map.tile(u.x, u.y);
            if t.is_land() && continent_index(t.continent) == k {
                here[u.kind as usize] += 1;
            }
        }
        for other in self.all_cities().filter(|o| o.owner == slot) {
            // the garrison stashed in cities on this continent (0xb044..0xb07f): units of the
            // civ and here, but not in ds:0xb3be, which create_unit / delete_unit keep
            if continent_index(self.map.tile(other.x, other.y).continent) == k {
                for (kind, _) in other.garrison_units() {
                    units += 1;
                    here[kind as usize] += 1;
                }
            }
            if let Production::Unit(u) = other.production {
                in_production[u as usize] += !(rechoose && other.id == c) as i32;
                if other.id != c && continent_index(self.map.tile(other.x, other.y).continent) == k {
                    building_here[u as usize] += 1;
                }
            }
        }
        if units == 0 && !p.human && p.govt as u8 <= Govt::Despotism as u8 {
            return Err(Production::Unit(unit_ids::MILITIA));
        }
        if slot == 0 {
            let explosives = self.civ_slots().iter().any(|o| self.player(*o).knows(adv::EXPLOSIVES));
            return Err(Production::Unit(if explosives { unit_ids::KNIGHTS } else { unit_ids::LEGION }));
        }
        // [bp-0x8]: the city needs defenders: 2 with nobody inside, 1 with fewer Defend-role
        // units than size/4 (+1 on a war footing)
        let inside = self.units_at(city.x, city.y);
        let need = if inside.is_empty() {
            2
        } else {
            let defenders = inside.iter().filter(|u| self.unit(**u).info().role == AiRole::Defend).count() as i32;
            if city.size as i32 / 4 + war as i32 > defenders {
                1
            } else {
                0
            }
        };
        let military_supported = self.supported_units(c).iter().filter(|u| self.unit(**u).kind < unit_ids::DIPLOMAT).count() as i32;
        let civilized = self.traits(slot).civilized as i32;
        let barracks = city.has_improvement(imp::BARRACKS);
        let mut advice = BuildAdvice::default();
        let candidates = self.available_production(c);
        for item in candidates {
            let value = match item {
                Production::Unit(u) if u < unit_ids::DIPLOMAT => {
                    let Some(v) = self.unit_value(u, &census, slot, k, policy, need, surplus, units, &owned, &in_production, &here, &building_here) else { continue };
                    let t = &rules::UNITS[u as usize];
                    let mut v = v;
                    // cost and strength: 16 (Settlers: 6, 4 with a Granary, minus the leader's
                    // "civilized" trait) * (cost + 2) / ((attack + defence) * (moves + 1))
                    let base = if u == unit_ids::SETTLERS { (if city.has_improvement(imp::GRANARY) { 4 } else { 6 }) - civilized } else { 16 };
                    v = base * (t.cost as i32 + 2) * v / ((t.att as i32 + t.def as i32) * (t.moves as i32 + 1)).max(1);
                    if t.role == AiRole::SeaAttack {
                        v *= if war { 4 } else { 2 };
                    }
                    if t.domain == Domain::Air {
                        v *= 2;
                    }
                    if units > 120 {
                        v *= 4;
                    }
                    if (!war && shields <= military_supported) || !self.exe_at_war(slot) {
                        v *= 2;
                    }
                    v
                }
                _ => {
                    let (v, military) = self.domestic_value(item, &census, &city, slot, human, &y, shields, surplus, &in_production);
                    // what fits the footing counts ten times, the rest twenty
                    let mut v = v * if war == military { 10 } else { 20 };
                    v = 3 * v / (civilized + 3);
                    if barracks {
                        v += v / 4;
                    }
                    v
                }
            };
            if advice.best.map(|(_, b)| value < b).unwrap_or(value < 999) {
                advice.best = Some((item, value));
            }
            let unit = matches!(item, Production::Unit(u) if u < unit_ids::DIPLOMAT);
            let slot_best = if unit { &mut advice.unit } else { &mut advice.improvement };
            if slot_best.map(|(_, b)| value < b).unwrap_or(value < 999) {
                *slot_best = Some((item, value));
            }
        }
        Ok(advice)
    }

    /// The unit part of the valuation (0xb569..0xb6df), before the cost factor; `None` skips
    /// the unit.
    #[allow(clippy::too_many_arguments)]
    fn unit_value(
        &self,
        u: u8,
        census: &Census,
        slot: Slot,
        k: usize,
        policy: u8,
        need: i32,
        surplus: i32,
        units: i32,
        owned: &[i32; 28],
        in_production: &[i32; 28],
        here: &[i32; 28],
        building_here: &[i32; 28],
    ) -> Option<i32> {
        let p = self.player(slot);
        let t = &rules::UNITS[u as usize];
        let ui = u as usize;
        // the Carrier is never built (0xb519)
        if u == unit_ids::CARRIER {
            return None;
        }
        // Settlers while the city has food to spare, or when the civilization has none and no
        // other city on the continent builds them
        if u == unit_ids::SETTLERS && self.settler_food(slot) >= surplus && (owned[0] != 0 || building_here[0] > 1) {
            return None;
        }
        // Militia only under Despotism and Anarchy
        if u == unit_ids::MILITIA && p.govt as u8 > Govt::Despotism as u8 {
            return None;
        }
        if u == unit_ids::NUCLEAR && (owned[ui] >= 4 || in_production[ui] >= 2) {
            return None;
        }
        // Fighters against the human's Bombers, when not at peace with the human
        if u == unit_ids::FIGHTER {
            let bombers = self.all_units().filter(|b| b.owner == self.human_slot && b.kind == unit_ids::BOMBER).count() as i32;
            if owned[ui] + in_production[ui] >= bombers / 2 || self.exe_peace(slot, self.human_slot) {
                return None;
            }
        }
        let mut v = if t.domain == Domain::Land {
            2 * building_here[ui] + here[ui]
        } else {
            // ships and aircraft only where our forces defend with 8 or more
            let defence = census.defence[slot as usize][k];
            if defence < 8 {
                return None;
            }
            (2 * in_production[ui] + owned[ui]) * defence / (units + 1)
        };
        if need != 0 {
            if t.role != AiRole::Defend {
                return None;
            }
            v = 2 / need;
        }
        let role = t.role as u8;
        if t.role != AiRole::Defend && t.role != AiRole::Settle {
            v *= 2;
            if policy != role {
                v *= 2;
            }
        }
        if t.role == AiRole::Transport {
            if policy != POLICY_OFFENSIVE {
                return None;
            }
            v = 3 * v / 2;
        }
        Some(v)
    }

    /// Improvements, wonders, Caravans and Diplomats (jump table cs:0xe6d = 0xb44d, entries
    /// 0xb1ab..0xb44a); returns the value before the footing factor and whether it is a
    /// military item ([bp-0x132]). Everything not in the table is worth 99.
    #[allow(clippy::too_many_arguments)]
    fn domestic_value(&mut self, item: Production, census: &Census, city: &super::City, slot: Slot, human: Slot, y: &super::economy::CityYield, shields: i32, surplus: i32, in_production: &[i32; 28]) -> (i32, bool) {
        let size = city.size as i32;
        let trade = y.trade;
        let disorder = city.disorder;
        let i = match item {
            Production::Unit(unit_ids::CARAVAN) => {
                // the third trade route slot free (0xb33e): the less trade, the better, +2 for
                // each of the first two in use; 3 at best
                let mut v = 99;
                if city.route_slot(2).is_none() {
                    let used = (0..2).filter(|i| city.route_slot(*i).is_some()).count() as i32;
                    v = (10 - trade / 2 + 2 * used).max(3);
                }
                if slot == human && city.auto_build {
                    v = 99;
                }
                return (v, false);
            }
            Production::Unit(unit_ids::DIPLOMAT) => {
                // to steal from the human: not at peace, none being built, and the human knows
                // something in a random fifth of the advance table that we do not
                if self.exe_peace(slot, human) || in_production[unit_ids::DIPLOMAT as usize] > 0 {
                    return (99, false);
                }
                let r = self.rng.random(0xb3e0, 5);
                if !self.faces_human(census, slot) {
                    return (99, false);
                }
                let word = |s: Slot| (self.player(s).advances >> (16 * r)) as u16;
                let (ours, theirs) = (word(slot), word(human));
                if ours | theirs == ours {
                    return (99, false);
                }
                let v = (self.player(slot).research_count as i32 - self.player(human).research_count as i32 + 10).clamp(5, 10);
                return (v, false);
            }
            Production::Unit(_) => return (99, false),
            Production::Improvement(i) => i,
            Production::Wonder(_) => return (99, false),
        };
        match i {
            imp::BARRACKS => (5, true),
            imp::GRANARY => (if shields >= 3 { 4 } else { 8 }, false),
            imp::TEMPLE => (if disorder { -4 } else { 99 }, false),
            imp::MARKETPLACE => (if disorder { -3 } else { 10 - trade / 2 }, false),
            imp::LIBRARY | imp::BANK => (10 - trade / 3, false),
            imp::COURTHOUSE => (12 - 2 * y.corruption, false),
            imp::CITY_WALLS => (10 - size / 2 - if city.has_improvement(imp::PALACE) { 4 } else { 0 }, true),
            imp::AQUEDUCT => {
                if 10 - surplus / 2 > size {
                    (99, false)
                } else {
                    (14 - size - surplus / 2, false)
                }
            }
            imp::CATHEDRAL => (if disorder { -2 } else { 99 }, false),
            imp::UNIVERSITY => (10 - trade / 4, false),
            imp::COLOSSEUM => (if disorder { -1 } else { 99 }, false),
            imp::FACTORY | imp::HYDRO_PLANT => (13 - shields / 2, false),
            imp::MFG_PLANT | imp::RECYCLING_CNTR | imp::NUCLEAR_PLANT => (12 - shields / 8, false),
            imp::POWER_PLANT => (12 - shields / 5, false),
            imp::SS_STRUCTURAL => {
                // nothing once either ship is launched; 2 while the human builds one, else 4
                if self.player(slot).spaceship.launched() || self.player(human).spaceship.launched() {
                    (99, false)
                } else if self.player(human).spaceship.structurals + self.player(human).spaceship.components + self.player(human).spaceship.modules > 0 {
                    (2, false)
                } else {
                    (4, false)
                }
            }
            // Palace, Mass Transit, SDI Defense, Components and Modules
            _ => (99, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;

    fn ai_city() -> (Game, CityId) {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = (1..8).find(|s| *s != g.human_slot && g.player(*s).alive && !g.units_of(*s).is_empty()).unwrap();
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        (g, c)
    }

    #[test]
    fn new_cities_start_on_the_best_known_defender_of_the_three() {
        let (mut g, c) = ai_city();
        let slot = g.city(c).owner;
        assert_eq!(g.first_production(slot), Production::Unit(unit_ids::MILITIA));
        g.player_mut(slot).learn(adv::GUNPOWDER);
        assert_eq!(g.first_production(slot), Production::Unit(unit_ids::MUSKETEERS));
        g.player_mut(slot).learn(adv::CONSCRIPTION);
        assert_eq!(g.first_production(slot), Production::Unit(unit_ids::RIFLEMEN));
    }

    #[test]
    fn the_advisor_of_a_new_city_still_sees_its_settlers() {
        // 'b' deletes the Settlers after found_city has asked the advisor (0x5829 -> 0x57e9):
        // a civ whose last unit founds the city does not take the no-units Militia shortcut,
        // and the city is not empty (no 2-defender need)
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = (1..8).find(|s| *s != g.human_slot && g.player(*s).alive && !g.units_of(*s).is_empty()).unwrap();
        let units = g.units_of(slot);
        let settler = *units.iter().find(|id| g.unit(**id).kind == unit_ids::SETTLERS).unwrap();
        for id in units.into_iter().filter(|id| *id != settler) {
            g.remove_unit(id);
        }
        g.player_mut(slot).learn(adv::BRONZE_WORKING);
        let c = g.found_city(settler).unwrap();
        assert!(g.units_of(slot).is_empty());
        assert_ne!(g.city(c).production, Production::Unit(unit_ids::MILITIA));
    }

    #[test]
    fn an_empty_city_builds_a_defender() {
        let (mut g, c) = ai_city();
        let slot = g.city(c).owner;
        // no unit inside: only Defend-role units are valued (2 / need = 1)
        for id in g.units_at(g.city(c).x, g.city(c).y) {
            g.remove_unit(id);
        }
        g.add_unit(slot, unit_ids::SETTLERS, 0, 0);
        let a = g.build_advice(c).unwrap();
        let (best, _) = a.best.unwrap();
        assert!(matches!(best, Production::Unit(u) if rules::UNITS[u as usize].role == AiRole::Defend), "{best:?}");
    }

    #[test]
    fn a_computer_civ_without_units_under_despotism_builds_militia() {
        let (mut g, c) = ai_city();
        let slot = g.city(c).owner;
        for id in g.units_of(slot) {
            g.remove_unit(id);
        }
        assert_eq!(g.ai_production(c), Production::Unit(unit_ids::MILITIA));
        // even a city building the Palace: the Palace test comes after the advisor (0xb857)
        g.city_mut(c).production = Production::Improvement(imp::PALACE);
        assert_eq!(g.ai_production(c), Production::Unit(unit_ids::MILITIA));
    }

    #[test]
    fn rechoosing_does_not_count_the_citys_own_unit() {
        let (mut g, c) = ai_city();
        let slot = g.city(c).owner;
        g.player_mut(slot).learn(adv::BRONZE_WORKING);
        // 0x18c4c takes the city's unit off the count first: the advisor then values as if
        // the city built no unit
        g.city_mut(c).production = Production::Improvement(imp::BARRACKS);
        let idle = g.build_advice_with(c, false);
        g.city_mut(c).production = Production::Unit(unit_ids::PHALANX);
        assert_eq!(g.build_advice_with(c, true), idle);
    }

    #[test]
    fn improvement_values_follow_the_jump_table() {
        let (mut g, c) = ai_city();
        let slot = g.city(c).owner;
        let city = g.city(c).clone();
        let y = g.city_yield(c);
        let none = [0; 28];
        let census = g.census();
        // Barracks 5 (military), Granary 8 below 3 shields, Temple only in disorder
        assert_eq!(g.domestic_value(Production::Improvement(imp::BARRACKS), &census, &city, slot, g.human_slot, &y, 2, 2, &none), (5, true));
        assert_eq!(g.domestic_value(Production::Improvement(imp::GRANARY), &census, &city, slot, g.human_slot, &y, 2, 2, &none), (8, false));
        assert_eq!(g.domestic_value(Production::Improvement(imp::GRANARY), &census, &city, slot, g.human_slot, &y, 3, 2, &none), (4, false));
        assert_eq!(g.domestic_value(Production::Improvement(imp::TEMPLE), &census, &city, slot, g.human_slot, &y, 2, 2, &none).0, 99);
        let mut rioting = city.clone();
        rioting.disorder = true;
        assert_eq!(g.domestic_value(Production::Improvement(imp::TEMPLE), &census, &rioting, slot, g.human_slot, &y, 2, 2, &none).0, -4);
        // City Walls: 10 - size/2, 4 less in the capital (the first city has the Palace)
        assert_eq!(g.domestic_value(Production::Improvement(imp::CITY_WALLS), &census, &city, slot, g.human_slot, &y, 2, 2, &none), (10 - 4, true));
        assert_eq!(g.domestic_value(Production::Wonder(1), &census, &city, slot, g.human_slot, &y, 2, 2, &none).0, 99);
    }

    #[test]
    fn the_human_auto_city_takes_improvements_only() {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        g.player_mut(slot).learn(adv::POTTERY);
        // a Granary (8 * 20 * 3 / (civilized + 3)) is under 500
        assert_eq!(g.auto_production(c), Some(Production::Improvement(imp::GRANARY)));
    }

    #[test]
    fn each_civ_takes_only_its_own_census_rows() {
        let (mut g, c) = ai_city();
        let slot = g.city(c).owner;
        let h = g.human_slot;
        let (x, y) = (g.city(c).x, g.city(c).y);
        let k = continent_index(g.map.tile(x, y).continent);
        g.take_census_all();
        let before = g.census();
        // the human lands an army and builds a Nuclear: nobody sees it before his own turn
        for _ in 0..3 {
            g.add_unit(h, unit_ids::KNIGHTS, x + 1, y);
        }
        g.add_unit(h, unit_ids::NUCLEAR, x + 1, y);
        g.take_census(slot);
        assert_eq!(g.census().attack[h as usize], before.attack[h as usize]);
        assert_eq!(g.military_strength(h), before.strength[h as usize]);
        // his turn: the Knights count (4 + 2 each), the Nuclear not
        g.take_census(h);
        assert_eq!(g.census().attack[h as usize][k], before.attack[h as usize][k] + 12);
        assert_eq!(g.military_strength(h), before.strength[h as usize] + 18);
        // the computer's rows stay as its own turn left them
        assert_eq!(g.census().cities[slot as usize], before.cities[slot as usize]);
    }

    #[test]
    fn policies_of_the_continent_census() {
        let (mut g, c) = ai_city();
        let slot = g.city(c).owner;
        assert_eq!(g.census().cities[slot as usize], [0; CONTINENTS], "nothing before the civ's AI turn");
        g.take_census(slot);
        let census = g.census();
        let k = continent_index(g.map.tile(g.city(c).x, g.city(c).y).continent);
        assert!(census.cities[slot as usize][k] >= 1);
        assert!(census.squares[k] > 0);
        // a fresh continent with room to spare and nobody hostile around
        let pol = g.continent_policy(&census, slot, k);
        assert!(pol == POLICY_NONE || pol == POLICY_OFFENSIVE, "{pol}");
        assert_eq!(g.continent_policy(&census, slot, 0), POLICY_NONE);
    }
}
