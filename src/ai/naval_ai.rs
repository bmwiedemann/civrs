//! Everything across water for computer players: the per-civilisation overseas plan (expand
//! to other continents once the home landmass is full, invade enemies across the sea), the
//! loading of colonists and troops in port, the voyage itself and the landing.
//!
//! Transport ships (Trireme, Sail, Frigate, Transport) are driven entirely from here; the
//! rest of the AI (`unit_ai`) skips them. Land units aboard a ship are skipped by `unit_ai`
//! too, so the only way they move is when this module lands them.
//!
//! State that cannot be derived from the game (which ship is waiting for cargo since when,
//! where a loaded ship is heading) lives in `Missions`, owned by the `Ai` driver. Everything
//! here re-validates that state every turn so a lost entry (e.g. after loading a saved game)
//! only costs a re-plan, never a stuck unit.

use super::unit_ai::{self, best_city_site, city_target, defenders_needed, site_ok, step, Progress, SITE_RADIUS};
use crate::game::turn::TurnStep;
use crate::game::{CityId, Dir8, Game, Map, MoveOutcome, Slot, Unit, UnitId, UnitOrder};
use crate::rules::{self, unit_ids, AiRole, MAP_H, MAP_W};
use std::collections::{BTreeMap, VecDeque};

/// Longest voyage (Chebyshev distance from the port) the AI plans.
pub const MAX_VOYAGE: i32 = 25;
/// Turns a loaded ship waits in port for an escort before sailing without one.
const LOAD_WAIT: u32 = 4;
/// Turns after which a voyage that has not landed is abandoned (cargo is put ashore at home).
const MISSION_TIMEOUT: u32 = 40;
/// Upper bound on moves per ship per turn (guards against loops).
const MAX_ACTIONS: usize = 24;
/// Minimum `site_score` for an overseas city site (the home rule is 80; overseas we are
/// slightly pickier because the colony has to stand on its own).
const MIN_OVERSEAS_SCORE: i32 = 90;
/// Idle attackers needed before a seaborne invasion is considered.
const INVASION_ARMY: usize = 4;

/// What a loaded ship carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cargo {
    /// Settlers (+ escort) founding a city overseas.
    Colonists,
    /// Attackers (+ defender) landing next to an enemy city.
    Troops,
}

/// Standing orders of one transport ship.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mission {
    /// Waiting in port; `since` is the turn the first unit boarded (or the wait began).
    Loading { since: u32 },
    /// Sailing to the ocean tile `sea`, then landing the cargo on the adjacent `land` tile.
    /// `target` is the planned city site / the enemy city.
    Sail { cargo: Cargo, sea: (i32, i32), land: (i32, i32), target: (i32, i32), since: u32 },
}

/// Mission per transport ship (BTreeMap so iteration order is deterministic).
pub type Missions = BTreeMap<UnitId, Mission>;

/// The civilisation's overseas plan for this turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// No acceptable free city site is left near any of our cities.
    pub home_full: bool,
    /// The home landmass is full and we still want more cities: settle overseas.
    pub colonise: bool,
    /// Coastal cities should build (and keep) a transport ship plus colonists / troops.
    pub wants_transport: bool,
    /// Civilisation we want to invade across the sea.
    pub invasion: Option<Slot>,
}

impl Plan {
    /// What ships should carry this turn (an invasion takes precedence over colonists).
    pub fn cargo(&self) -> Option<Cargo> {
        if self.invasion.is_some() {
            Some(Cargo::Troops)
        } else if self.colonise {
            Some(Cargo::Colonists)
        } else {
            None
        }
    }
}

/// Attackers a ship of `cap` berths carries (one berth is kept for the escort on big ships).
fn troop_quota(cap: usize) -> usize {
    if cap <= 2 {
        cap
    } else {
        (cap - 1).min(4)
    }
}

/// A planned landing: the ship stops on `sea`, the cargo steps onto `land` and heads for `target`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Landing {
    pub sea: (i32, i32),
    pub land: (i32, i32),
    pub target: (i32, i32),
}

// ---- small queries ---------------------------------------------------------------------------

/// Is this a ship that carries land units (Trireme, Sail, Frigate, Transport)?
pub fn is_transport(u: &Unit) -> bool {
    u.is_sea() && u.info().role == AiRole::Transport && u.info().capacity > 0
}

pub fn is_transport_kind(k: u8) -> bool {
    let t = &rules::UNITS[k as usize];
    t.domain == rules::Domain::Sea && t.role == AiRole::Transport && t.capacity > 0
}

/// Ocean tile with at least one land neighbour (where a Trireme may end its turn safely).
pub fn near_land(g: &Game, x: i32, y: i32) -> bool {
    g.map.land_mask8(x, y) != 0
}

/// Best transport type the civilisation could build now (highest capacity), if any.
pub fn transport_kind(g: &Game, slot: Slot) -> Option<u8> {
    let p = g.player(slot);
    [unit_ids::TRANSPORT, unit_ids::FRIGATE, unit_ids::SAIL, unit_ids::TRIREME].into_iter().find(|k| {
        let t = &rules::UNITS[*k as usize];
        t.prereq.map(|a| p.knows(a)).unwrap_or(true) && !t.obsolete_by.map(|a| p.knows(a)).unwrap_or(false)
    })
}

/// Our transport ships.
fn transports_of(g: &Game, slot: Slot) -> Vec<UnitId> {
    g.all_units().filter(|u| u.owner == slot && is_transport(u)).map(|u| u.id).collect()
}

/// Own city on the tile, if any.
fn own_city_at(g: &Game, slot: Slot, p: (i32, i32)) -> Option<CityId> {
    g.city_at(p.0, p.1).filter(|c| g.city(*c).owner == slot)
}

/// Land units on `p` that are not aboard anything.
fn free_land_units_at(g: &Game, slot: Slot, p: (i32, i32)) -> Vec<UnitId> {
    g.all_units().filter(|u| u.owner == slot && u.pos() == p && u.is_land() && u.carried_by.is_none()).map(|u| u.id).collect()
}

/// Can a ship of `kind` owned by `slot` pass through (x,y)? (ocean, or an own city)
fn sea_tile_ok(g: &Game, slot: Slot, x: i32, y: i32) -> bool {
    let Some(t) = g.map.get(x, y) else { return false };
    if t.is_land() {
        return t.city.map(|c| g.city(c).owner == slot).unwrap_or(false);
    }
    true
}

/// May a Trireme end its turn on (x,y)? (own city or next to land; otherwise 50 % loss)
fn trireme_safe(g: &Game, slot: Slot, x: i32, y: i32) -> bool {
    near_land(g, x, y) || own_city_at(g, slot, (x, y)).is_some()
}

/// Grid of tiles holding foreign units or cities (impassable for our ships).
fn foreign_grid(g: &Game, slot: Slot) -> Vec<bool> {
    let mut blocked = vec![false; MAP_W * MAP_H];
    for u in g.all_units() {
        if u.owner != slot && Map::in_bounds(u.x, u.y) {
            blocked[Map::idx(u.x, u.y)] = true;
        }
    }
    for c in g.all_cities() {
        if c.owner != slot {
            blocked[Map::idx(c.x, c.y)] = true;
        }
    }
    blocked
}

/// Moves per turn of a ship (Magellan's Expedition adds one).
fn moves_per_turn(g: &Game, u: &Unit) -> usize {
    u.info().moves as usize + if g.owner_has_wonder(u.owner, rules::wonder_ids::MAGELLANS_EXPEDITION) { 1 } else { 0 }
}

// ---- water reachability ----------------------------------------------------------------------

/// Breadth-first search over water for a ship of `kind` owned by `slot`.
///
/// For Triremes the search state is the tile plus the number of moves already spent in the
/// current turn (`used` at the start), so that every turn ends next to land (the engine's
/// hazard rule) while open sea up to two tiles wide can still be crossed. Other ships are
/// unconstrained. Foreign units and cities are impassable except on `goal`.
///
/// Returns the reachable-tile grid and, when `goal` was reached, the path from `start` to it.
fn sea_search(g: &Game, slot: Slot, kind: u8, start: (i32, i32), used: usize, per_turn: usize, goal: Option<(i32, i32)>) -> (Vec<bool>, Option<Vec<(i32, i32)>>) {
    let n = MAP_W * MAP_H;
    let mut reach = vec![false; n];
    if !Map::in_bounds(start.0, start.1) {
        return (reach, None);
    }
    let trireme = kind == unit_ids::TRIREME;
    let k = if trireme { per_turn.max(1) } else { 1 };
    let used = if trireme { used.min(k - 1) } else { 0 };
    if let Some(gl) = goal {
        if !Map::in_bounds(gl.0, gl.1) || (trireme && !trireme_safe(g, slot, gl.0, gl.1)) {
            return (reach, None);
        }
    }
    let blocked = foreign_grid(g, slot);
    let mut parent: Vec<u32> = vec![u32::MAX; n * k];
    let mut seen = vec![false; n * k];
    let si = Map::idx(start.0, start.1) * k + used;
    seen[si] = true;
    reach[Map::idx(start.0, start.1)] = true;
    let mut q = VecDeque::new();
    q.push_back(si);
    while let Some(cur) = q.pop_front() {
        let tile = cur / k;
        let spent = cur % k;
        let (cx, cy) = ((tile % MAP_W) as i32, (tile / MAP_W) as i32);
        if goal == Some((cx, cy)) {
            let mut path = vec![(cx, cy)];
            let mut p = cur;
            while parent[p] != u32::MAX {
                p = parent[p] as usize;
                let t = p / k;
                path.push(((t % MAP_W) as i32, (t / MAP_W) as i32));
            }
            path.reverse();
            return (reach, Some(path));
        }
        let last_move = spent + 1 == k;
        for (nx, ny) in Map::neighbours8(cx, cy) {
            let ni = Map::idx(nx, ny);
            let is_goal = goal == Some((nx, ny));
            if (blocked[ni] && !is_goal) || !sea_tile_ok(g, slot, nx, ny) {
                continue;
            }
            if trireme && last_move && !trireme_safe(g, slot, nx, ny) {
                continue;
            }
            let ns = ni * k + if last_move { 0 } else { spent + 1 };
            if seen[ns] {
                continue;
            }
            seen[ns] = true;
            reach[ni] = true;
            parent[ns] = cur as u32;
            q.push_back(ns);
        }
    }
    (reach, None)
}

/// Every water tile (and own city) a fresh ship of `kind` could reach from `from` (an ocean
/// tile or an own coastal city) without crossing a tile held by a foreign unit or city.
/// Returns a `MAP_W * MAP_H` grid; `from` itself is marked.
pub fn water_reach(g: &Game, slot: Slot, kind: u8, from: (i32, i32)) -> Vec<bool> {
    sea_search(g, slot, kind, from, 0, rules::UNITS[kind as usize].moves as usize, None).0
}

/// Is any tile of landmass `cont` adjacent to a reachable water tile?
pub fn continent_reachable(g: &Game, reach: &[bool], cont: u8) -> bool {
    for y in 0..MAP_H as i32 {
        for x in 0..MAP_W as i32 {
            if reach[Map::idx(x, y)] && g.map.tile(x, y).is_ocean() && Map::neighbours8(x, y).any(|(nx, ny)| g.map.tile(nx, ny).continent == cont) {
                return true;
            }
        }
    }
    false
}

/// Shortest water path for the ship from where it stands to `goal` (both ends included),
/// honouring the moves it has already spent this turn (Trireme turn ends must be next to land).
pub fn sea_path(g: &Game, u: &Unit, goal: (i32, i32)) -> Option<Vec<(i32, i32)>> {
    if u.pos() == goal {
        return Some(vec![goal]);
    }
    let per_turn = moves_per_turn(g, u);
    let used = per_turn.saturating_sub(u.moves_left as usize / 3);
    sea_search(g, u.owner, u.kind, u.pos(), used, per_turn, Some(goal)).1
}

/// Sail the ship towards `goal` for as long as it has moves this turn.
pub fn sail_toward(g: &mut Game, id: UnitId, goal: (i32, i32), sink: &mut dyn FnMut(TurnStep)) -> Progress {
    let Some(u) = g.unit_opt(id).cloned() else { return Progress::Dead };
    if u.pos() == goal {
        return Progress::Arrived;
    }
    if u.moves_left == 0 {
        return Progress::Partial;
    }
    let Some(path) = sea_path(g, &u, goal) else { return Progress::Blocked };
    let mut actions = 0;
    for next in path.iter().skip(1) {
        let Some(cur) = g.unit_opt(id).cloned() else { return Progress::Dead };
        if cur.moves_left == 0 {
            return Progress::Partial;
        }
        actions += 1;
        if actions > MAX_ACTIONS {
            return Progress::Partial;
        }
        let Some(d) = Dir8::from_delta(next.0 - cur.x, next.1 - cur.y) else { return Progress::Blocked };
        match step(g, id, d, sink) {
            Ok(MoveOutcome::Moved) | Ok(MoveOutcome::EnteredHut) => {
                if g.unit_opt(id).map(|u| u.pos() == goal).unwrap_or(false) {
                    return Progress::Arrived;
                }
            }
            Ok(MoveOutcome::Blocked) => return Progress::Blocked,
            Ok(other) => return Progress::Outcome(other),
            Err(_) => return Progress::Blocked,
        }
    }
    if g.unit_opt(id).map(|u| u.pos() == goal).unwrap_or(false) {
        Progress::Arrived
    } else {
        Progress::Partial
    }
}

// ---- destinations ----------------------------------------------------------------------------

/// Per-landmass facts used to rank destinations.
struct Continents {
    size: Vec<u32>,
    any_city: Vec<bool>,
    own_city: Vec<bool>,
}

fn continents(g: &Game, slot: Slot) -> Continents {
    let mut c = Continents { size: vec![0; 256], any_city: vec![false; 256], own_city: vec![false; 256] };
    for t in &g.map.tiles {
        c.size[t.continent as usize] += 1;
    }
    for city in g.all_cities() {
        let k = g.map.tile(city.x, city.y).continent as usize;
        c.any_city[k] = true;
        if city.owner == slot {
            c.own_city[k] = true;
        }
    }
    c
}

/// Landing spot for a target tile: a land tile of the target's landmass within `max_walk` of
/// it that holds no foreign unit/city and has a reachable water tile next to it. Picks the
/// shortest walk, then the easiest terrain, then the shortest voyage from `port`.
fn landing_for(g: &Game, slot: Slot, reach: &[bool], port: (i32, i32), target: (i32, i32), max_walk: i32) -> Option<Landing> {
    let cont = g.map.tile(target.0, target.1).continent;
    let mut best: Option<(Landing, i32)> = None;
    for y in (target.1 - max_walk)..=(target.1 + max_walk) {
        for x in (target.0 - max_walk)..=(target.0 + max_walk) {
            let Some(t) = g.map.get(x, y) else { continue };
            if !t.is_land() || t.continent != cont {
                continue;
            }
            if t.city.is_some() || g.owner_at(x, y).map(|o| o != slot).unwrap_or(false) {
                continue;
            }
            let terrain_cost = t.terrain.info().move_cost as i32;
            let walk = Map::dist((x, y), target);
            for (sx, sy) in Map::neighbours8(x, y) {
                if !reach[Map::idx(sx, sy)] || !g.map.tile(sx, sy).is_ocean() {
                    continue;
                }
                let score = walk * 10 + (terrain_cost - 1) * 4 + Map::dist((sx, sy), port);
                if best.map(|(_, s)| score < s).unwrap_or(true) {
                    best = Some((Landing { sea: (sx, sy), land: (x, y), target }, score));
                }
            }
        }
    }
    best.map(|(l, _)| l)
}

/// Best city site on another landmass within `MAX_VOYAGE` of `port`, reachable over the water
/// described by `reach`. Prefers landmasses without any city, then ones where we have none.
pub fn overseas_site(g: &Game, slot: Slot, port: (i32, i32), reach: &[bool]) -> Option<Landing> {
    let home = g.map.tile(port.0, port.1).continent;
    let conts = continents(g, slot);
    let mut cands: Vec<((i32, i32), i32)> = Vec::new();
    for y in (port.1 - MAX_VOYAGE)..=(port.1 + MAX_VOYAGE) {
        for x in (port.0 - MAX_VOYAGE)..=(port.0 + MAX_VOYAGE) {
            let Some(t) = g.map.get(x, y) else { continue };
            if !t.is_land() || t.continent == home || t.continent == 0 {
                continue;
            }
            let base = crate::game::mapgen::site_score(&g.map, x, y);
            if base < MIN_OVERSEAS_SCORE {
                continue;
            }
            let k = t.continent as usize;
            let mut s = base - Map::dist((x, y), port) * 3;
            if !conts.any_city[k] {
                s += 40;
            } else if !conts.own_city[k] {
                s += 15;
            }
            if conts.size[k] < 4 {
                s -= 30;
            }
            if !site_ok(g, slot, (x, y)) {
                continue;
            }
            cands.push(((x, y), s));
        }
    }
    cands.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (site, _) in cands.into_iter().take(8) {
        if let Some(l) = landing_for(g, slot, reach, port, site, 3) {
            return Some(l);
        }
    }
    None
}

/// Does `enemy` hold a city on a landmass where we have none (so only a landing can reach it)?
pub fn overseas_enemy(g: &Game, slot: Slot, enemy: Slot) -> bool {
    let conts = continents(g, slot);
    g.all_cities().any(|c| c.owner == enemy && !conts.own_city[g.map.tile(c.x, c.y).continent as usize])
}

/// Landing next to the nearest city of `enemy` on a landmass other than the port's, reachable
/// over `reach`.
pub fn invasion_landing(g: &Game, slot: Slot, enemy: Slot, port: (i32, i32), reach: &[bool]) -> Option<Landing> {
    let home = g.map.tile(port.0, port.1).continent;
    let mut targets: Vec<(i32, i32)> = g
        .all_cities()
        .filter(|c| c.owner == enemy && g.map.tile(c.x, c.y).continent != home && Map::dist(c.pos(), port) <= MAX_VOYAGE + 5)
        .map(|c| c.pos())
        .collect();
    targets.sort_by_key(|p| (Map::dist(*p, port), *p));
    for t in targets.into_iter().take(4) {
        if let Some(l) = landing_for(g, slot, reach, port, t, 3) {
            return Some(l);
        }
    }
    None
}

// ---- the plan --------------------------------------------------------------------------------

/// Attackers standing idle in our cities (the pool for an invasion force).
fn idle_attackers(g: &Game, slot: Slot) -> usize {
    g.all_units().filter(|u| u.owner == slot && u.is_land() && u.info().role == AiRole::Attack && u.carried_by.is_none() && own_city_at(g, slot, u.pos()).is_some()).count()
}

/// Evaluate the overseas plan for `slot` this turn.
pub fn plan(g: &Game, slot: Slot) -> Plan {
    let mut plan = Plan::default();
    let cities = g.cities_of(slot);
    if cities.is_empty() || g.turn < 15 {
        return plan;
    }
    let Some(kind) = transport_kind(g, slot) else { return plan };
    let ports: Vec<(i32, i32)> = cities.iter().map(|c| g.city(*c).pos()).filter(|p| g.map.is_coastal(p.0, p.1)).collect();
    if ports.is_empty() {
        return plan;
    }
    // "full" is judged from the cities able to produce Settlers (size 3+); a fresh size-1
    // colony next to open land does not stop the expeditions
    plan.home_full = cities.iter().filter(|c| g.city(**c).size >= 3 || cities.len() == 1).all(|c| best_city_site(g, slot, g.city(*c).pos(), SITE_RADIUS).is_none());
    if idle_attackers(g, slot) >= INVASION_ARMY {
        plan.invasion = g
            .enemies_of(slot)
            .into_iter()
            .filter(|e| *e != 0 && g.player(*e).alive && overseas_enemy(g, slot, *e))
            .min_by_key(|e| g.all_cities().filter(|c| c.owner == *e).map(|c| ports.iter().map(|p| Map::dist(*p, c.pos())).min().unwrap_or(999)).min().unwrap_or(999));
    }
    plan.colonise = plan.home_full && cities.len() < city_target(g, slot) + 3;
    // only worth it when some port actually has somewhere to go; an invasion without a
    // landing falls back to colonising
    for _ in 0..2 {
        let Some(cargo) = plan.cargo() else { break };
        if ports.iter().take(3).any(|p| port_has_destination(g, slot, kind, *p, cargo, plan.invasion)) {
            plan.wants_transport = true;
            break;
        }
        if plan.invasion.is_some() {
            plan.invasion = None;
        } else {
            plan.colonise = false;
        }
    }
    plan
}

/// Can a ship of `kind` sailing from `port` find a landing for this kind of cargo?
pub fn port_has_destination(g: &Game, slot: Slot, kind: u8, port: (i32, i32), cargo: Cargo, enemy: Option<Slot>) -> bool {
    let reach = water_reach(g, slot, kind, port);
    match cargo {
        Cargo::Colonists => overseas_site(g, slot, port, &reach).is_some(),
        Cargo::Troops => enemy.map(|e| invasion_landing(g, slot, e, port, &reach).is_some()).unwrap_or(false),
    }
}

// ---- cargo bookkeeping -----------------------------------------------------------------------

fn settlers_aboard(g: &Game, ship: UnitId) -> usize {
    g.cargo_of(ship).iter().filter(|c| g.unit(**c).kind == unit_ids::SETTLERS).count()
}

fn troops_aboard(g: &Game, ship: UnitId) -> usize {
    g.cargo_of(ship).iter().filter(|c| g.unit(**c).info().role == AiRole::Attack).count()
}

fn defender_aboard(g: &Game, ship: UnitId) -> bool {
    g.cargo_of(ship).iter().any(|c| g.unit(*c).info().role == AiRole::Defend)
}

/// A ship of ours waiting in a port on the unit's landmass that still wants a Settlers
/// (none aboard, none standing by in that city). Used by idle Settlers to walk to the harbour.
pub fn colonist_port(g: &Game, u: &Unit) -> Option<(i32, i32)> {
    let slot = u.owner;
    let cont = g.map.tile(u.x, u.y).continent;
    transports_of(g, slot)
        .into_iter()
        .filter_map(|s| {
            let ship = g.unit(s);
            let c = own_city_at(g, slot, ship.pos())?;
            let city = g.city(c);
            if g.map.tile(city.x, city.y).continent != cont || ship.order != UnitOrder::Sentry {
                return None;
            }
            if g.cargo_count(s) >= ship.info().capacity as usize || settlers_aboard(g, s) > 0 {
                return None;
            }
            let waiting = free_land_units_at(g, slot, city.pos()).into_iter().filter(|o| *o != u.id && g.unit(*o).kind == unit_ids::SETTLERS).count();
            if waiting > 0 {
                return None;
            }
            Some(city.pos())
        })
        .min_by_key(|p| Map::dist(*p, u.pos()))
}

/// Transport ship of ours sentried in the city, if any.
pub fn transport_in_port(g: &Game, c: CityId) -> Option<UnitId> {
    let city = g.city(c);
    transports_of(g, city.owner).into_iter().find(|s| g.unit(*s).pos() == city.pos())
}

/// Cargo the ship in port `c` still needs: `(settlers, defender, attackers)` wanted counts.
/// Lets `city_ai` build exactly what the expedition lacks.
pub fn port_needs(g: &Game, c: CityId, plan: &Plan) -> Option<(usize, bool, usize)> {
    let ship = transport_in_port(g, c)?;
    let cap = g.unit(ship).info().capacity as usize;
    let settlers = settlers_aboard(g, ship);
    let troops = troops_aboard(g, ship);
    let defender = defender_aboard(g, ship);
    let want_settlers = if plan.cargo() == Some(Cargo::Colonists) && settlers == 0 { 1 } else { 0 };
    let want_defender = (settlers > 0 || troops >= 2) && !defender && cap > troop_quota(cap);
    let want_attackers = if plan.cargo() == Some(Cargo::Troops) { troop_quota(cap).saturating_sub(troops) } else { 0 };
    Some((want_settlers, want_defender, want_attackers))
}

/// Shield surplus the city would keep after taking over the support of one more unit.
fn surplus_after_support(g: &Game, c: CityId) -> i32 {
    let y = g.city_yield(c);
    let free = rules::GOVERNMENTS[g.player(g.city(c).owner).govt as usize].free_units_per_city as i32;
    let supported = g.supported_units(c).len() as i32;
    y.shield_surplus() - if supported + 1 > free { 1 } else { 0 }
}

/// Re-home a unit standing in city `c` there when that city can support it better than its
/// current home. A city short of shields disbands its farthest unit first, which is exactly
/// a ship on a voyage (and the cargo with it), so expeditions are homed in rich ports.
fn rehome(g: &mut Game, id: UnitId, c: CityId) {
    let u = g.unit(id).clone();
    if u.home == Some(c) || u.pos() != g.city(c).pos() {
        return;
    }
    let here = surplus_after_support(g, c);
    let home = u.home.and_then(|h| g.city_opt(h)).map(|h| g.city_yield(h.id).shield_surplus() - 1).unwrap_or(-1);
    if here >= 1 && here > home {
        let _ = g.set_home(id);
    }
}

/// Put a land unit standing in the port aboard the ship (both on the city tile, so no move).
fn board(g: &mut Game, unit: UnitId, ship: UnitId) {
    let u = g.unit_mut(unit);
    u.carried_by = Some(ship);
    u.order = UnitOrder::Sentry;
    u.goto = None;
}

/// Set the cargo ashore in port (reverse of `board`).
fn unload_in_port(g: &mut Game, ship: UnitId) {
    for c in g.cargo_of(ship) {
        let u = g.unit_mut(c);
        u.carried_by = None;
        u.order = UnitOrder::None;
        u.goto = None;
    }
}

/// Load whatever the plan wants from the city onto the ship. Returns true if anything boarded.
fn load_in_port(g: &mut Game, ship: UnitId, c: CityId, plan: &Plan) -> bool {
    let slot = g.unit(ship).owner;
    let pos = g.city(c).pos();
    let cap = g.unit(ship).info().capacity as usize;
    let mut boarded = false;
    let free = free_land_units_at(g, slot, pos);
    let idle = |g: &Game, id: UnitId| {
        let u = g.unit(id);
        u.order_turns == 0 && !matches!(u.order, UnitOrder::Goto)
    };
    // colonists: one Settlers (two on big ships)
    if plan.cargo() == Some(Cargo::Colonists) {
        let max_settlers = (cap / 2).max(1);
        for id in &free {
            if g.cargo_count(ship) >= cap || settlers_aboard(g, ship) >= max_settlers {
                break;
            }
            if g.unit(*id).kind == unit_ids::SETTLERS && g.unit(*id).carried_by.is_none() && idle(g, *id) {
                board(g, *id, ship);
                boarded = true;
            }
        }
    }
    // troops: attackers, leaving one berth for the escort on bigger ships
    if plan.cargo() == Some(Cargo::Troops) {
        for id in &free {
            if g.cargo_count(ship) >= cap || troops_aboard(g, ship) >= troop_quota(cap) {
                break;
            }
            let u = g.unit(*id);
            if u.is_land() && u.info().role == AiRole::Attack && u.carried_by.is_none() && idle(g, *id) {
                board(g, *id, ship);
                boarded = true;
            }
        }
    }
    // escort: one defender, only when the city keeps its garrison
    if (settlers_aboard(g, ship) > 0 || troops_aboard(g, ship) >= 2) && !defender_aboard(g, ship) && g.cargo_count(ship) < cap {
        let garrison: Vec<UnitId> = free_land_units_at(g, slot, pos).into_iter().filter(|id| g.unit(*id).info().role == AiRole::Defend && g.unit(*id).is_military()).collect();
        if garrison.len() > defenders_needed(g, c) {
            // take the weakest spare defender
            if let Some(id) = garrison.into_iter().filter(|id| idle(g, *id)).min_by_key(|id| g.unit(*id).info().def) {
                board(g, id, ship);
                boarded = true;
            }
        }
    }
    boarded
}

// ---- landing ---------------------------------------------------------------------------------

/// Land tiles next to the ship on landmass `cont` free of foreign units/cities, best first.
fn landing_tiles(g: &Game, slot: Slot, at: (i32, i32), cont: u8, prefer: (i32, i32)) -> Vec<(i32, i32)> {
    let mut v: Vec<(i32, i32)> = Map::neighbours8(at.0, at.1)
        .filter(|(x, y)| {
            let t = g.map.tile(*x, *y);
            t.is_land() && (cont == 0 || t.continent == cont) && t.city.is_none() && !g.owner_at(*x, *y).map(|o| o != slot).unwrap_or(false)
        })
        .collect();
    v.sort_by_key(|p| (if *p == prefer { 0 } else { 1 }, Map::dist(*p, prefer), *p));
    v
}

/// Step every unit aboard onto land next to the ship. Returns the number of units landed.
fn disembark(g: &mut Game, ship: UnitId, land: (i32, i32), target: Option<(i32, i32)>, sink: &mut dyn FnMut(TurnStep)) -> usize {
    let Some(s) = g.unit_opt(ship).cloned() else { return 0 };
    let slot = s.owner;
    let cont = g.map.tile(land.0, land.1).continent;
    let tiles = landing_tiles(g, slot, s.pos(), cont, land);
    let mut landed = 0;
    for c in g.cargo_of(ship) {
        let Some(u) = g.unit_opt(c).cloned() else { continue };
        if u.moves_left == 0 {
            continue;
        }
        for t in &tiles {
            let Some(d) = Dir8::from_delta(t.0 - u.x, t.1 - u.y) else { continue };
            match step(g, c, d, sink) {
                Ok(MoveOutcome::Moved) | Ok(MoveOutcome::EnteredHut) => {
                    if let Some(um) = g.units.get_mut(c as usize).and_then(|u| u.as_mut()) {
                        um.carried_by = None;
                        um.order = UnitOrder::None;
                        um.goto = if um.kind == unit_ids::SETTLERS { target } else { None };
                    }
                    landed += 1;
                    break;
                }
                Ok(MoveOutcome::Blocked) => {
                    // a failed partial move used up the unit's turn
                    if g.unit_opt(c).map(|u| u.moves_left == 0).unwrap_or(true) {
                        break;
                    }
                }
                _ => break,
            }
        }
    }
    landed
}

// ---- the ships -------------------------------------------------------------------------------

/// Nearest own coastal city reachable by the ship (by water path length), with the path length.
fn nearest_port(g: &Game, u: &Unit) -> Option<(CityId, usize)> {
    let slot = u.owner;
    let mut ports: Vec<CityId> = g.all_cities().filter(|c| c.owner == slot && g.map.is_coastal(c.x, c.y)).map(|c| c.id).collect();
    ports.sort_by_key(|c| (Map::dist(g.city(*c).pos(), u.pos()), *c));
    let mut best: Option<(CityId, usize)> = None;
    for c in ports.into_iter().take(6) {
        if let Some(p) = sea_path(g, u, g.city(c).pos()) {
            if best.map(|(_, l)| p.len() < l).unwrap_or(true) {
                best = Some((c, p.len()));
            }
        }
    }
    best
}

/// Port where cargo is waiting for a ship: Settlers standing by (colonists) or the most idle
/// attackers (invasion); falls back to the nearest reachable own port.
fn loading_port(g: &Game, u: &Unit, plan: &Plan) -> Option<CityId> {
    let slot = u.owner;
    let score = |c: CityId| -> i32 {
        let pos = g.city(c).pos();
        let free = free_land_units_at(g, slot, pos);
        let settlers = free.iter().filter(|id| g.unit(**id).kind == unit_ids::SETTLERS).count() as i32;
        let attackers = free.iter().filter(|id| g.unit(**id).info().role == AiRole::Attack).count() as i32;
        let others = transports_of(g, slot).iter().filter(|s| **s != u.id && g.unit(**s).pos() == pos).count() as i32;
        let mut s = -Map::dist(pos, u.pos());
        if plan.invasion.is_some() {
            s += attackers * 8;
        } else if plan.home_full {
            s += settlers * 20;
        }
        s - others * 30
    };
    let mut ports: Vec<CityId> = g.all_cities().filter(|c| c.owner == slot && g.map.is_coastal(c.x, c.y)).map(|c| c.id).collect();
    if ports.is_empty() {
        return None;
    }
    let cargo = plan.cargo()?;
    ports.sort_by_key(|c| (-score(*c), *c));
    ports.into_iter().take(4).find(|c| {
        let p = g.city(*c).pos();
        sea_path(g, u, p).is_some() && port_has_destination(g, slot, u.kind, p, cargo, plan.invasion)
    })
}

/// Bring a loaded ship with nowhere to go home and put the cargo ashore. Returns true when
/// the cargo has been unloaded.
fn abort_voyage(g: &mut Game, id: UnitId, sink: &mut dyn FnMut(TurnStep)) -> bool {
    let Some(u) = g.unit_opt(id).cloned() else { return true };
    let slot = u.owner;
    if own_city_at(g, slot, u.pos()).is_some() {
        unload_in_port(g, id);
        return true;
    }
    if let Some((c, _)) = nearest_port(g, &u) {
        let p = g.city(c).pos();
        if sail_toward(g, id, p, sink) == Progress::Arrived {
            unload_in_port(g, id);
            return true;
        }
        return false;
    }
    // no port at all: put them ashore on any land of ours next to the ship
    let conts = continents(g, slot);
    let shore = Map::neighbours8(u.x, u.y).find(|(x, y)| {
        let t = g.map.tile(*x, *y);
        t.is_land() && conts.own_city[t.continent as usize]
    });
    if let Some(l) = shore {
        return disembark(g, id, l, None, sink) > 0;
    }
    false
}

/// Pick a destination for the loaded ship from where it is.
fn plan_voyage(g: &Game, u: &Unit, cargo: Cargo, plan: &Plan) -> Option<Landing> {
    let reach = water_reach(g, u.owner, u.kind, u.pos());
    match cargo {
        Cargo::Colonists => overseas_site(g, u.owner, u.pos(), &reach),
        Cargo::Troops => plan.invasion.and_then(|e| invasion_landing(g, u.owner, e, u.pos(), &reach)),
    }
}

/// Explore the coast / wait in port (the behaviour of a transport the plan has no use for).
fn idle_ship(g: &mut Game, id: UnitId, sink: &mut dyn FnMut(TurnStep)) {
    let Some(u) = g.unit_opt(id).cloned() else { return };
    let slot = u.owner;
    if u.moves_left == 0 {
        return;
    }
    if let Some(t) = unit_ai::nearest_unexplored(g, &u, 14) {
        if sail_toward(g, id, t, sink) != Progress::Blocked {
            return;
        }
    }
    // (Skip rather than Sentry: a sentried ship in port is one waiting for colonists)
    if own_city_at(g, slot, u.pos()).is_some() {
        let _ = g.give_order(id, UnitOrder::Skip);
        return;
    }
    if let Some((c, _)) = nearest_port(g, &u) {
        let p = g.city(c).pos();
        if sail_toward(g, id, p, sink) == Progress::Arrived {
            let _ = g.give_order(id, UnitOrder::Skip);
        }
    }
}

/// One turn of one transport ship.
fn act_transport(g: &mut Game, id: UnitId, plan: &Plan, missions: &mut Missions, sink: &mut dyn FnMut(TurnStep)) {
    let Some(u) = g.unit_opt(id).cloned() else { return };
    let slot = u.owner;
    let turn = g.turn;
    // ships with guns take a cheap shot first
    if u.info().att > 0 && u.moves_left > 0 && g.cargo_count(id) == 0 {
        unit_ai::opportunistic_attack(g, id, 70, sink);
    }
    let Some(u) = g.unit_opt(id).cloned() else { return };
    let in_port = own_city_at(g, slot, u.pos());
    let mission = missions.get(&id).copied();

    // --- at sea / sailing with cargo
    if let Some(Mission::Sail { cargo, sea, land, target, since }) = mission {
        if g.cargo_count(id) == 0 {
            missions.remove(&id);
        } else if turn.saturating_sub(since) > MISSION_TIMEOUT {
            if abort_voyage(g, id, sink) {
                missions.remove(&id);
            }
            return;
        } else {
            // destination still usable? (someone may have settled / parked a ship there)
            let land_ok = {
                let t = g.map.tile(land.0, land.1);
                t.is_land() && t.city.is_none() && !g.owner_at(land.0, land.1).map(|o| o != slot).unwrap_or(false)
            };
            let sea_ok = !g.owner_at(sea.0, sea.1).map(|o| o != slot).unwrap_or(false);
            let target_ok = match cargo {
                Cargo::Colonists => site_ok(g, slot, target) || g.map.tile(target.0, target.1).continent == g.map.tile(land.0, land.1).continent,
                Cargo::Troops => g.city_at(target.0, target.1).map(|c| unit_ai::hostile(g, slot, g.city(c).owner)).unwrap_or(false),
            };
            let mut dest = if land_ok && sea_ok && target_ok { Some(Landing { sea, land, target }) } else { None };
            if dest.is_none() {
                dest = plan_voyage(g, &u, cargo, plan);
                if let Some(l) = dest {
                    missions.insert(id, Mission::Sail { cargo, sea: l.sea, land: l.land, target: l.target, since });
                }
            }
            let Some(l) = dest else {
                if abort_voyage(g, id, sink) {
                    missions.remove(&id);
                }
                return;
            };
            match sail_toward(g, id, l.sea, sink) {
                Progress::Arrived => {
                    let site = if cargo == Cargo::Colonists { Some(l.target) } else { None };
                    disembark(g, id, l.land, site, sink);
                    if g.cargo_count(id) == 0 {
                        missions.remove(&id);
                    }
                }
                Progress::Blocked => {
                    // try a fresh destination from here next turn; if none, go home
                    match plan_voyage(g, &u, cargo, plan) {
                        Some(n) if n.sea != l.sea => {
                            missions.insert(id, Mission::Sail { cargo, sea: n.sea, land: n.land, target: n.target, since });
                        }
                        _ => {
                            if abort_voyage(g, id, sink) {
                                missions.remove(&id);
                            }
                        }
                    }
                }
                Progress::Dead => {
                    missions.remove(&id);
                }
                _ => {}
            }
            return;
        }
    }

    // --- in port: load and maybe sail
    if let Some(c) = in_port {
        let Some(wanted) = plan.cargo().filter(|_| plan.wants_transport) else {
            if g.cargo_count(id) > 0 {
                unload_in_port(g, id);
            }
            missions.remove(&id);
            idle_ship(g, id, sink);
            return;
        };
        // this harbour leads nowhere: try another one
        if !port_has_destination(g, slot, u.kind, u.pos(), wanted, plan.invasion) {
            if g.cargo_count(id) > 0 {
                unload_in_port(g, id);
            }
            missions.remove(&id);
            match loading_port(g, &u, plan) {
                Some(other) if other != c => {
                    let p = g.city(other).pos();
                    if sail_toward(g, id, p, sink) == Progress::Arrived {
                        missions.insert(id, Mission::Loading { since: turn });
                        let _ = g.give_order(id, UnitOrder::Sentry);
                    }
                }
                _ => idle_ship(g, id, sink),
            }
            return;
        }
        let since = match mission {
            Some(Mission::Loading { since }) => since,
            _ => turn,
        };
        // cargo of the wrong kind (the war ended / started while loading) goes ashore
        let wrong = match wanted {
            Cargo::Colonists => troops_aboard(g, id) > 0,
            Cargo::Troops => settlers_aboard(g, id) > 0,
        };
        if wrong {
            unload_in_port(g, id);
        }
        let had_cargo = g.cargo_count(id) > 0;
        let boarded = load_in_port(g, id, c, plan);
        let since = if boarded && !had_cargo { turn } else { since };
        // the expedition is supported by this port if it can afford it
        rehome(g, id, c);
        for cargo in g.cargo_of(id) {
            rehome(g, cargo, c);
        }
        missions.insert(id, Mission::Loading { since });
        let _ = g.give_order(id, UnitOrder::Sentry);
        let settlers = settlers_aboard(g, id);
        let troops = troops_aboard(g, id);
        let escort = defender_aboard(g, id);
        let cap = g.unit(id).info().capacity as usize;
        let waited = turn.saturating_sub(since);
        let ready = match wanted {
            Cargo::Colonists => settlers > 0 && (escort || waited >= LOAD_WAIT || cap < 2),
            Cargo::Troops => troops >= 2 && (troops >= troop_quota(cap) || waited >= LOAD_WAIT / 2),
        };
        if !ready || g.unit(id).moves_left == 0 {
            return;
        }
        match plan_voyage(g, &u, wanted, plan) {
            Some(l) => {
                missions.insert(id, Mission::Sail { cargo: wanted, sea: l.sea, land: l.land, target: l.target, since: turn });
                let _ = g.give_order(id, UnitOrder::None);
                if sail_toward(g, id, l.sea, sink) == Progress::Arrived {
                    let site = if wanted == Cargo::Colonists { Some(l.target) } else { None };
                    disembark(g, id, l.land, site, sink);
                    if g.cargo_count(id) == 0 {
                        missions.remove(&id);
                    }
                }
            }
            None => {
                // nowhere to go from this port after all: cargo stays ashore
                unload_in_port(g, id);
            }
        }
        return;
    }

    // --- at sea without a mission
    if g.cargo_count(id) > 0 {
        // stray cargo (mission lost): make it a voyage of the kind the cargo suggests
        let cargo = if settlers_aboard(g, id) > 0 { Cargo::Colonists } else { Cargo::Troops };
        match plan_voyage(g, &u, cargo, plan) {
            Some(l) if plan.wants_transport => {
                missions.insert(id, Mission::Sail { cargo, sea: l.sea, land: l.land, target: l.target, since: turn });
            }
            _ => {
                if abort_voyage(g, id, sink) {
                    missions.remove(&id);
                }
            }
        }
        return;
    }
    missions.remove(&id);
    if plan.wants_transport {
        if let Some(c) = loading_port(g, &u, plan) {
            let p = g.city(c).pos();
            if sail_toward(g, id, p, sink) == Progress::Arrived {
                missions.insert(id, Mission::Loading { since: turn });
                let _ = g.give_order(id, UnitOrder::Sentry);
            }
            return;
        }
    }
    idle_ship(g, id, sink);
}

/// Drop bookkeeping for ships that no longer exist and fix land units that believe they are
/// aboard a ship which is gone or elsewhere.
fn tidy(g: &mut Game, slot: Slot, missions: &mut Missions) {
    missions.retain(|id, _| g.unit_opt(*id).map(|u| u.owner != slot || is_transport(u)).unwrap_or(false));
    let stale: Vec<UnitId> = g
        .all_units()
        .filter(|u| u.owner == slot && u.is_land())
        .filter(|u| match u.carried_by {
            Some(s) => g.unit_opt(s).map(|ship| ship.pos() != u.pos()).unwrap_or(true),
            None => false,
        })
        .map(|u| u.id)
        .collect();
    for id in stale {
        let u = g.unit_mut(id);
        u.carried_by = None;
        if u.order == UnitOrder::Sentry {
            u.order = UnitOrder::None;
        }
    }
}

/// Move every transport ship of the civilisation (called before the land units act so that
/// freshly landed units can still move this turn).
pub fn play_transports(g: &mut Game, slot: Slot, plan: &Plan, missions: &mut Missions, sink: &mut dyn FnMut(TurnStep)) {
    tidy(g, slot, missions);
    for id in transports_of(g, slot) {
        if g.unit_opt(id).map(|u| u.owner == slot).unwrap_or(false) {
            act_transport(g, id, plan, missions, sink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;
    use crate::rules::Terrain;

    /// Two islands in an ocean world; a wall of land across the map blocks the way.
    fn two_islands() -> Game {
        let mut g = Game::new(GameSetup { seed: 5, num_civs: 7, ..Default::default() }, None);
        let ids: Vec<_> = g.all_units().map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        for t in g.map.tiles.iter_mut() {
            t.terrain = Terrain::Ocean;
            t.flags = 0;
            t.city = None;
            t.special = false;
        }
        for (x, y) in [(10, 10), (11, 10), (10, 11), (11, 11)] {
            g.map.get_mut(x, y).unwrap().terrain = Terrain::Grassland;
        }
        for (x, y) in [(30, 30), (31, 30), (30, 31), (31, 31)] {
            g.map.get_mut(x, y).unwrap().terrain = Terrain::Plains;
        }
        g.map.compute_continents();
        g
    }

    #[test]
    fn water_reach_finds_the_other_island_and_respects_barriers() {
        let mut g = two_islands();
        let port = (10, 10);
        let c = g.add_city(1, "Port".into(), port.0, port.1);
        assert_eq!(g.city(c).owner, 1);
        let other = g.map.tile(30, 30).continent;
        assert_ne!(other, 0);
        assert_ne!(other, g.map.tile(10, 10).continent);
        let reach = water_reach(&g, 1, unit_ids::SAIL, port);
        assert!(reach[Map::idx(29, 29)], "ocean next to the far island is reachable");
        assert!(continent_reachable(&g, &reach, other));
        // a trireme must end every turn next to land: 20 tiles of open sea are out of reach
        let coastal = water_reach(&g, 1, unit_ids::TRIREME, port);
        assert!(!coastal[Map::idx(20, 20)]);
        assert!(!continent_reachable(&g, &coastal, other), "no coast to follow between the islands");
        // ... but a two-tile strait can be crossed in one turn (3 moves: open, open, coast)
        for (x, y) in [(14, 10), (15, 10)] {
            g.map.get_mut(x, y).unwrap().terrain = Terrain::Forest;
        }
        g.map.compute_continents();
        let strait = water_reach(&g, 1, unit_ids::TRIREME, (10, 10));
        assert!(strait[Map::idx(15, 11)], "coast of the islet across the strait");
        let t = g.add_unit(1, unit_ids::TRIREME, 10, 10);
        let path = sea_path(&g, g.unit(t), (16, 10)).expect("trireme path across the strait");
        // every third tile of the path (a turn end) is next to land
        for (i, p) in path.iter().enumerate().skip(1) {
            if i % 3 == 0 {
                assert!(near_land(&g, p.0, p.1), "turn end {p:?} not next to land");
            }
        }
        g.remove_unit(t);
        for (x, y) in [(14, 10), (15, 10)] {
            g.map.get_mut(x, y).unwrap().terrain = Terrain::Ocean;
        }
        g.map.compute_continents();
        // a land barrier across the whole map cuts the sail off too
        for x in 0..MAP_W as i32 {
            g.map.get_mut(x, 20).unwrap().terrain = Terrain::Hills;
        }
        g.map.compute_continents();
        let other = g.map.tile(30, 30).continent;
        let reach = water_reach(&g, 1, unit_ids::SAIL, port);
        assert!(!reach[Map::idx(29, 29)]);
        assert!(!continent_reachable(&g, &reach, other));
        // ... unless our own city on the barrier works as a canal
        g.add_city(1, "Canal".into(), 20, 20);
        let reach = water_reach(&g, 1, unit_ids::SAIL, port);
        assert!(continent_reachable(&g, &reach, other));
        // a path exists for a ship as well
        let s = g.add_unit(1, unit_ids::SAIL, 10, 10);
        let path = sea_path(&g, g.unit(s), (29, 29)).expect("sea path through the canal");
        assert!(path.contains(&(20, 20)));
    }

    /// Run `turns` AI turns for slot 1 only (ships, then land units), no world processing.
    fn run_slot1(g: &mut Game, missions: &mut Missions, turns: usize, plan: Option<Plan>) {
        for _ in 0..turns {
            g.reset_moves(1);
            let pl = plan.unwrap_or_else(|| super::plan(g, 1));
            let mut sink = |_s: TurnStep| {};
            play_transports(g, 1, &pl, missions, &mut sink);
            unit_ai::play_units(g, 1, &mut sink);
            g.turn += 1;
        }
    }

    /// A tiny home island (no room for a second city) and a fertile island 16+ tiles away.
    fn colony_world() -> Game {
        let mut g = two_islands();
        for (x, y) in [(10, 10), (11, 10), (10, 11), (11, 11)] {
            g.map.get_mut(x, y).unwrap().terrain = Terrain::Ocean;
        }
        for (x, y) in [(10, 10), (11, 10), (10, 11)] {
            g.map.get_mut(x, y).unwrap().terrain = Terrain::Hills;
        }
        for y in 26..33 {
            for x in 26..33 {
                g.map.get_mut(x, y).unwrap().terrain = Terrain::Grassland;
            }
        }
        g.map.compute_continents();
        g.turn = 40;
        g
    }

    #[test]
    fn colonists_are_shipped_to_the_other_island_and_found_a_city() {
        let mut g = colony_world();
        let home = g.map.tile(10, 10).continent;
        let far = g.map.tile(29, 29).continent;
        g.add_city(1, "Port".into(), 10, 10);
        g.add_unit(1, unit_ids::PHALANX, 10, 10);
        g.add_unit(1, unit_ids::SETTLERS, 10, 10);
        let ship = g.add_unit(1, unit_ids::SAIL, 10, 10);
        let plan = Plan { home_full: true, colonise: true, wants_transport: true, invasion: None };
        let mut missions = Missions::new();
        // turn 1: the Settlers board; the ship waits for an escort it will never get
        run_slot1(&mut g, &mut missions, 1, Some(plan));
        assert_eq!(settlers_aboard(&g, ship), 1);
        assert!(matches!(missions.get(&ship), Some(Mission::Loading { .. })));
        // ... then sails, lands the colonists and they found a city on the far island
        run_slot1(&mut g, &mut missions, 30, Some(plan));
        let colony = g.all_cities().find(|c| c.owner == 1 && g.map.tile(c.x, c.y).continent == far);
        assert!(colony.is_some(), "no colony founded on the far island: cities {:?} units {:?}", g.all_cities().map(|c| c.pos()).collect::<Vec<_>>(), g.all_units().map(|u| (u.info().name, u.pos(), u.carried_by)).collect::<Vec<_>>());
        assert_ne!(home, far);
        // the ship is empty again and heading home / in port
        assert_eq!(g.cargo_count(ship), 0);
        assert!(missions.get(&ship).map(|m| !matches!(m, Mission::Sail { .. })).unwrap_or(true));
    }

    #[test]
    fn idle_attackers_invade_an_enemy_across_the_sea() {
        let mut g = colony_world();
        let far = g.map.tile(29, 29).continent;
        g.player_mut(1).learn(crate::rules::advance_ids::MAP_MAKING);
        g.player_mut(1).learn(crate::rules::advance_ids::NAVIGATION);
        g.add_city(1, "Port".into(), 10, 10);
        g.add_city(2, "Target".into(), 28, 28);
        g.add_unit(2, unit_ids::MILITIA, 28, 28);
        g.make_contact(1, 2);
        g.declare_war(1, 2);
        g.add_unit(1, unit_ids::PHALANX, 10, 10);
        for _ in 0..4 {
            g.add_unit(1, unit_ids::LEGION, 10, 10);
        }
        let plan = super::plan(&g, 1);
        assert_eq!(plan.invasion, Some(2), "{plan:?}");
        assert!(plan.wants_transport);
        assert_eq!(plan.cargo(), Some(Cargo::Troops));
        let ship = g.add_unit(1, unit_ids::SAIL, 10, 10);
        let mut missions = Missions::new();
        run_slot1(&mut g, &mut missions, 1, None);
        assert!(troops_aboard(&g, ship) >= 2, "troops did not board: {:?}", g.cargo_of(ship));
        run_slot1(&mut g, &mut missions, 25, None);
        let landed = g.all_units().filter(|u| u.owner == 1 && u.is_land() && u.carried_by.is_none() && g.map.tile(u.x, u.y).continent == far).count();
        let captured = g.city_at(28, 28).map(|c| g.city(c).owner == 1).unwrap_or(false);
        assert!(landed > 0 || captured, "no landing on the enemy island: units {:?}", g.all_units().map(|u| (u.owner, u.info().name, u.pos(), u.carried_by)).collect::<Vec<_>>());
    }

    #[test]
    fn overseas_site_points_at_the_other_island() {
        let mut g = two_islands();
        // make the far island worth settling
        for y in 28..34 {
            for x in 28..34 {
                g.map.get_mut(x, y).unwrap().terrain = Terrain::Grassland;
            }
        }
        g.map.compute_continents();
        g.add_city(1, "Port".into(), 10, 10);
        let reach = water_reach(&g, 1, unit_ids::SAIL, (10, 10));
        let l = overseas_site(&g, 1, (10, 10), &reach).expect("a site overseas");
        assert_eq!(g.map.tile(l.land.0, l.land.1).continent, g.map.tile(30, 30).continent);
        assert!(g.map.tile(l.sea.0, l.sea.1).is_ocean());
        assert_eq!(Map::dist(l.sea, l.land), 1);
        assert!(Map::dist(l.land, l.target) <= 3);
    }
}
