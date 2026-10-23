//! What a city builds next: CIV.EXE's production advisor 0xa5e:0x41d (`choose_production`, file
//! 0xa9fd..0xbab3), shared by the computer players, the human's AUTO cities and the advisors'
//! recommendations, plus the per-continent census of the AI turn (0x1579a) that it reads.
//!
//! Every candidate the production menu offers gets a value, **lower is better**; the computer
//! takes the lowest of all, the human's AUTO city the lowest improvement (if it is 500 or less).
//!
//! Not modelled: the two garrison units a city record stores for unseen cities (+0x1a/+0x1b,
//! counted by the census and the advisor as units on the continent; civrs has no such store),
//! and CIV.EXE's habit of counting ships whose ocean body has the same id as the continent.

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

/// The census 0x1579a takes at the start of each civilization's turn, for every civilization.
#[derive(Clone, Debug, Default)]
pub struct Census {
    /// ds:0x6cac: summed defence of the civ's units (Nuclear excluded) standing on land of
    /// the continent (0x158b1).
    pub defence: [[i32; CONTINENTS]; 8],
    /// ds:0x8076: summed attack of the same units.
    pub attack: [[i32; CONTINENTS]; 8],
    /// ds:0x6b96: cities of the civ on the continent.
    pub cities: [[i32; CONTINENTS]; 8],
    /// ds:0xdcfe: land squares of the continent.
    pub squares: [i32; CONTINENTS],
    /// ds:0xb1ee: Plains, Grassland and River squares in columns 2..77, rows 2..47 (counted by
    /// the world generator, overlay 7 0x9d55..0x9ebf).
    pub sites: [i32; CONTINENTS],
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

    /// The census of 0x1579a (units 0x15801, cities 0x159be) for all civilizations at once.
    pub fn census(&self) -> Census {
        let mut c = Census::default();
        for u in self.all_units() {
            if u.owner as usize >= 8 || u.kind == unit_ids::NUCLEAR {
                continue;
            }
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
                c.cities[city.owner as usize][continent_index(self.map.tile(city.x, city.y).continent)] += 1;
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

    /// ds:0xe498 bit 1: a peace treaty between the two.
    fn exe_peace(&self, a: Slot, b: Slot) -> bool {
        self.player(a).relation(b) == super::player::Relation::Peace
    }

    /// ds:0xe498 low bits == 1: contact without a treaty, i.e. war.
    fn exe_war(&self, a: Slot, b: Slot) -> bool {
        self.player(a).relation(b) == super::player::Relation::War
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
                // ds:0xe498 bit 0x100 (cease-fire?) also counts; civrs has no such state
                if !weak && self.exe_war(slot, o as Slot) {
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

    /// ds:0x2a6e bit `slot` (0x15f3f): the civ has forces on a continent where the human has
    /// two or more cities.
    fn faces_human(&self, census: &Census, slot: Slot) -> bool {
        let h = self.human_slot as usize;
        h < 8 && (1..15).any(|k| census.defence[slot as usize][k] != 0 && census.cities[h][k] > 1)
    }

    /// ds:0x6ed4 (0x15ca1): the civ is at war with someone.
    fn exe_at_war(&self, slot: Slot) -> bool {
        (1..8).any(|o| o != slot && self.exe_war(slot, o))
    }

    /// ds:0x8078 (power ranking 0x3121..0x330e): the human is the strongest civilization, has
    /// more than four cities and no nuclear weapon, after turn 200. Strength is |gold|/32 + 8 *
    /// citizens + advances + the cost of every unit (in rows of ten shields).
    pub fn world_gangs_up(&self) -> bool {
        let h = self.human_slot;
        if self.turn <= 200 || self.cities_of(h).len() <= 4 || self.all_units().any(|u| u.owner == h && u.kind == unit_ids::NUCLEAR) {
            return false;
        }
        let power = |s: Slot| -> i32 {
            let p = self.player(s);
            if !p.alive {
                return 0;
            }
            let citizens: i32 = self.all_cities().filter(|c| c.owner == s).map(|c| c.size as i32).sum();
            let units: i32 = self.all_units().filter(|u| u.owner == s).map(|u| u.info().cost as i32).sum();
            p.gold / 32 + 8 * citizens + p.research_count as i32 + units
        };
        let ph = power(h);
        // the ranking takes the first of equals
        (1..8).all(|o| o == h || if o < h { power(o) < ph } else { power(o) <= ph })
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
    /// Palace keeps it.
    pub fn ai_production(&mut self, c: CityId) -> Production {
        if self.city(c).production == Production::Improvement(imp::PALACE) {
            return Production::Improvement(imp::PALACE);
        }
        match self.build_advice(c) {
            Ok(a) => a.best.map(|b| b.0).unwrap_or(self.city(c).production),
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
            let p = self.ai_production(c);
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
        let city = self.city(c).clone();
        let slot = city.owner;
        let human = self.human_slot;
        let p = self.player(slot).clone();
        let k = continent_index(self.map.tile(city.x, city.y).continent);
        let census = self.census();
        let policy = self.continent_policy(&census, slot, k);
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
        for u in self.all_units().filter(|u| u.owner == slot) {
            units += 1;
            owned[u.kind as usize] += 1;
            let t = self.map.tile(u.x, u.y);
            if t.is_land() && continent_index(t.continent) == k {
                here[u.kind as usize] += 1;
            }
        }
        for other in self.all_cities().filter(|o| o.owner == slot) {
            if let Production::Unit(u) = other.production {
                in_production[u as usize] += 1;
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
        let civilized = p.civ().civilized as i32;
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
                // a free trade route slot: the less trade, the better; 3 at best
                let mut v = 99;
                if city.trade_routes.len() < 3 {
                    v = (10 - trade / 2 + 2 * city.trade_routes.len() as i32).max(3);
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
                let r = self.rng.below(5);
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
    fn policies_of_the_continent_census() {
        let (g, c) = ai_city();
        let slot = g.city(c).owner;
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
