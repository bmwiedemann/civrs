//! Unit behaviour for computer players and barbarians.
//!
//! Every move goes through `Game::try_move` (which resolves combat, capture and huts) and is
//! reported to the turn `sink` as `TurnStep::AiMove` / `TurnStep::Combat` with a `visible`
//! flag telling the UI whether the human can see either tile.

use super::naval_ai;
use super::path;
use crate::game::turn::TurnStep;
use crate::game::{tile_flags, CityId, Dir8, Game, Map, MoveOutcome, RuleError, Slot, Unit, UnitId, UnitOrder};
use crate::rules::{self, unit_ids, AiRole, Terraform, Terrain};

/// Expansion budget for AI pathfinding (short hops; a fresh path is computed every turn).
const AI_PATH_LIMIT: usize = 1500;
/// Upper bound on actions per unit per turn (guards against zero-cost loops on railroads).
const MAX_ACTIONS: usize = 24;

/// Result of trying to walk a unit towards a tile this turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// Standing on the target tile.
    Arrived,
    /// Out of moves (or path exhausted) before arriving.
    Partial,
    /// No path / blocked by something; the caller should pick another target.
    Blocked,
    /// The unit no longer exists.
    Dead,
    /// The last step produced a special outcome (attack, capture, foreign city, boarding).
    Outcome(MoveOutcome),
}

fn visible(g: &Game, a: (i32, i32), b: (i32, i32)) -> bool {
    let h = g.human_slot;
    g.visible_to(h, a.0, a.1) || g.visible_to(h, b.0, b.1)
}

/// One `try_move` with reporting.
pub fn step(g: &mut Game, id: UnitId, d: Dir8, sink: &mut dyn FnMut(TurnStep)) -> Result<MoveOutcome, RuleError> {
    let from = g.unit(id).pos();
    let attacker_kind = g.unit(id).kind;
    let (dx, dy) = d.delta();
    let to = (Map::wrap_x(from.0 + dx), from.1 + dy);
    let defender_owner = g.units_at(to.0, to.1).first().map(|u| g.unit(*u).owner).unwrap_or(0);
    // what happened before this move is shown before it, what the move brings (a contact and
    // its treaty, a hut) after it, as CIV.EXE shows its messages while the computer moves
    flush_events(g, sink);
    let r = g.try_move(id, d);
    if let Ok(out) = &r {
        match out {
            MoveOutcome::Moved | MoveOutcome::EnteredHut | MoveOutcome::Boarded(_) | MoveOutcome::CapturedCity(_) | MoveOutcome::DestroyedCity => {
                let vis = visible(g, from, to);
                sink(TurnStep::AiMove { unit: id, from, to, visible: vis });
            }
            MoveOutcome::Attacked { won, at, defender } => {
                let vis = visible(g, from, *at);
                sink(TurnStep::Combat { attacker_at: from, defender_at: *at, won: *won, visible: vis, attacker: attacker_kind, defender: *defender, defender_owner });
            }
            _ => {}
        }
    }
    flush_events(g, sink);
    r
}

/// Hand the game's pending events to the turn runner in the order they happened.
fn flush_events(g: &mut Game, sink: &mut dyn FnMut(TurnStep)) {
    for e in g.take_events() {
        sink(TurnStep::Event(e));
    }
}

/// Walk towards `target` along an A* path for as long as the unit has moves this turn.
pub fn move_toward(g: &mut Game, id: UnitId, target: (i32, i32), sink: &mut dyn FnMut(TurnStep)) -> Progress {
    let Some(u) = g.unit_opt(id).cloned() else { return Progress::Dead };
    if u.pos() == target {
        return Progress::Arrived;
    }
    if u.moves_left == 0 {
        return Progress::Partial;
    }
    let Some(path) = path::astar_limited(g, &u, target, AI_PATH_LIMIT) else { return Progress::Blocked };
    let trireme = u.kind == unit_ids::TRIREME;
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
        // a Trireme must end its turn next to land (an ocean tile is "coastal" by
        // `Map::is_coastal` whenever it has an ocean neighbour, so test for land directly)
        if trireme && !naval_ai::near_land(g, next.0, next.1) && g.city_at(next.0, next.1).is_none() {
            return Progress::Blocked;
        }
        let Some(d) = Dir8::from_delta(next.0 - cur.x, next.1 - cur.y) else { return Progress::Blocked };
        match step(g, id, d, sink) {
            Ok(MoveOutcome::Moved) | Ok(MoveOutcome::EnteredHut) => {
                if g.unit_opt(id).map(|u| u.pos() == target).unwrap_or(false) {
                    return Progress::Arrived;
                }
                // a hut may have spawned barbarians next to us: re-plan next turn
                if g.unit_opt(id).is_none() {
                    return Progress::Dead;
                }
            }
            Ok(MoveOutcome::Blocked) => return Progress::Blocked,
            Ok(other) => return Progress::Outcome(other),
            Err(_) => return Progress::Blocked,
        }
    }
    if g.unit_opt(id).map(|u| u.pos() == target).unwrap_or(false) {
        Progress::Arrived
    } else {
        Progress::Partial
    }
}

/// Is `other` an enemy of `slot`: anyone `slot`'s own diplomacy word has no treaty with
/// (`Game::hostile`; CIV.EXE's unit AI skips treaty partners, e.g. 0x181f8, 0x17721), but not
/// the human while an audience with him is queued (`Game::audience_pending`).
pub fn hostile(g: &Game, slot: Slot, other: Slot) -> bool {
    g.may_fight(slot, other)
}

/// Nearest enemy target (unit stack or city) for an attacker: returns (pos, win chance %).
fn find_target(g: &Game, u: &Unit, radius: i32, min_chance: i32) -> Option<(i32, i32)> {
    let slot = u.owner;
    let my_cont = g.map.tile(u.x, u.y).continent;
    let mut best: Option<((i32, i32), i32)> = None;
    let mut consider = |g: &Game, x: i32, y: i32, is_city: bool| {
        let d = Map::dist((x, y), u.pos());
        if d == 0 || d > radius {
            return;
        }
        let t = g.map.tile(x, y);
        if u.is_land() && (!t.is_land() || (t.continent != my_cont && d > 1)) {
            return;
        }
        if u.is_sea() && t.is_land() && !is_city {
            return;
        }
        if u.is_sea() && is_city && d > 1 {
            return;
        }
        let chance = if g.units_at(x, y).is_empty() {
            if is_city && u.is_land() && u.info().att > 0 {
                100
            } else {
                return;
            }
        } else {
            g.win_chance(u, x, y)
        };
        if chance < min_chance {
            return;
        }
        let mut score = chance - d * 6;
        if is_city {
            score += 15;
        }
        if best.map(|(_, s)| score > s).unwrap_or(true) {
            best = Some(((x, y), score));
        }
    };
    for c in g.all_cities() {
        if hostile(g, slot, c.owner) {
            consider(g, c.x, c.y, true);
        }
    }
    for o in g.all_units() {
        if hostile(g, slot, o.owner) && g.city_at(o.x, o.y).is_none() && o.carried_by.is_none() {
            consider(g, o.x, o.y, false);
        }
    }
    best.map(|(p, _)| p)
}

/// Nearest tile within `radius` not yet explored by the unit's owner that the unit could reach
/// (same landmass for land units, ocean for ships).
pub fn nearest_unexplored(g: &Game, u: &Unit, radius: i32) -> Option<(i32, i32)> {
    let slot = u.owner;
    let cont = g.map.tile(u.x, u.y).continent;
    let mut best: Option<((i32, i32), i32)> = None;
    for y in (u.y - radius)..=(u.y + radius) {
        for x in (u.x - radius)..=(u.x + radius) {
            let Some(t) = g.map.get(x, y) else { continue };
            if t.explored_by(slot) {
                continue;
            }
            let ok = match u.info().domain {
                rules::Domain::Land => t.is_land() && t.continent == cont,
                rules::Domain::Sea => t.is_ocean() && (u.kind != unit_ids::TRIREME || naval_ai::near_land(g, x, y)),
                rules::Domain::Air => false,
            };
            if !ok {
                continue;
            }
            let d = Map::dist((x, y), u.pos());
            if best.map(|(_, bd)| d < bd).unwrap_or(true) {
                best = Some(((x, y), d));
            }
        }
    }
    best.map(|(p, _)| p)
}

/// Attack an adjacent enemy when the odds are at least `min_chance` percent. Returns true if
/// an attack was made.
pub fn opportunistic_attack(g: &mut Game, id: UnitId, min_chance: i32, sink: &mut dyn FnMut(TurnStep)) -> bool {
    let Some(u) = g.unit_opt(id).cloned() else { return false };
    if u.info().att == 0 || u.moves_left == 0 {
        return false;
    }
    let mut best: Option<(Dir8, i32)> = None;
    for d in Dir8::ALL {
        let (dx, dy) = d.delta();
        let (x, y) = (Map::wrap_x(u.x + dx), u.y + dy);
        if !Map::in_bounds(x, y) || !g.is_enemy_at(u.owner, x, y) {
            continue;
        }
        let t = g.map.tile(x, y);
        if u.is_land() && t.is_ocean() {
            continue;
        }
        if u.is_sea() && t.is_land() {
            continue;
        }
        let units = g.units_at(x, y);
        if units.is_empty() {
            continue;
        }
        // only fighters may attack aircraft in the open
        if units.iter().all(|t| g.unit(*t).is_air()) && t.city.is_none() && u.kind != unit_ids::FIGHTER {
            continue;
        }
        let chance = g.win_chance(&u, x, y);
        if chance >= min_chance && best.map(|(_, c)| chance > c).unwrap_or(true) {
            best = Some((d, chance));
        }
    }
    if let Some((d, _)) = best {
        return step(g, id, d, sink).is_ok();
    }
    false
}

// ---- city sites ------------------------------------------------------------------------------

/// Minimum acceptable `site_score` for founding a city.
const MIN_SITE_SCORE: i32 = 80;
/// How far from a city the AI looks for new sites (the Civ1 AI knows the map).
pub const SITE_RADIUS: i32 = 12;

/// Can `slot` found a city on (x,y)? (land, 3+ tiles from any city, 2+ from foreign units,
/// decent yield)
pub fn site_ok(g: &Game, slot: Slot, p: (i32, i32)) -> bool {
    site_ok_scored(g, slot, p, MIN_SITE_SCORE)
}

/// `site_ok` with an explicit minimum score.
fn site_ok_scored(g: &Game, slot: Slot, p: (i32, i32), min_score: i32) -> bool {
    let Some(t) = g.map.get(p.0, p.1) else { return false };
    if !t.is_land() || t.city.is_some() {
        return false;
    }
    if g.all_cities().any(|c| Map::dist((c.x, c.y), p) < 3) {
        return false;
    }
    if g.all_units().any(|u| u.owner != slot && u.is_military() && Map::dist(u.pos(), p) < 2) {
        return false;
    }
    crate::game::mapgen::site_score(&g.map, p.0, p.1) >= min_score
}

/// Does the civilisation own a city on the unit's landmass? (false = a landed colonist /
/// expedition that has to make do with what it finds)
fn has_city_on_continent(g: &Game, slot: Slot, p: (i32, i32)) -> bool {
    let cont = g.map.tile(p.0, p.1).continent;
    g.all_cities().any(|c| c.owner == slot && g.map.tile(c.x, c.y).continent == cont)
}

/// Best city site within `radius` of `from` on the same landmass (score minus a distance
/// penalty), or None.
pub fn best_city_site(g: &Game, slot: Slot, from: (i32, i32), radius: i32) -> Option<(i32, i32)> {
    best_city_site_min(g, slot, from, radius, MIN_SITE_SCORE)
}

/// `best_city_site` with an explicit minimum `site_score` (colonists on a poor island settle
/// for less than the home rule).
pub fn best_city_site_min(g: &Game, slot: Slot, from: (i32, i32), radius: i32, min_score: i32) -> Option<(i32, i32)> {
    let cont = g.map.tile(from.0, from.1).continent;
    let mut best: Option<((i32, i32), i32)> = None;
    for y in (from.1 - radius)..=(from.1 + radius) {
        for x in (from.0 - radius)..=(from.0 + radius) {
            let Some(t) = g.map.get(x, y) else { continue };
            if !t.is_land() || (cont != 0 && t.continent != cont) {
                continue;
            }
            let score = crate::game::mapgen::site_score(&g.map, x, y);
            if score < min_score.min(MIN_SITE_SCORE) || !site_ok_scored(g, slot, (x, y), min_score) {
                continue;
            }
            let d = Map::dist((x, y), from);
            let s = score - d * 6;
            if best.map(|(_, bs)| s > bs).unwrap_or(true) {
                best = Some(((x, y), s));
            }
        }
    }
    best.map(|(p, _)| p)
}

/// Number of cities the civilisation wants (by expansionist trait).
pub fn city_target(g: &Game, slot: Slot) -> usize {
    let civ = g.player(slot).civ();
    let base = 10 + 4 * civ.expansionist as i32 + (g.turn as i32 / 60);
    base.max(4) as usize
}

// ---- terraforming ----------------------------------------------------------------------------

/// Terraforming order useful on tile (x,y) for the owner, if any.
fn useful_work(g: &Game, slot: Slot, x: i32, y: i32) -> Option<UnitOrder> {
    let t = g.map.tile(x, y);
    if !t.is_land() || t.city.is_some() {
        return None;
    }
    if t.has(tile_flags::POLLUTION) {
        return Some(UnitOrder::CleanPollution);
    }
    let p = g.player(slot);
    match t.terrain {
        Terrain::Grassland | Terrain::Plains | Terrain::River | Terrain::Desert => {
            if !t.has(tile_flags::IRRIGATION) && !t.has(tile_flags::MINE) && g.irrigation_water(x, y) {
                return Some(UnitOrder::Irrigate);
            }
            if !t.has(tile_flags::ROAD) && t.terrain != Terrain::River && matches!(t.terrain, Terrain::Grassland | Terrain::Plains | Terrain::Desert) {
                return Some(UnitOrder::Road);
            }
            if t.has(tile_flags::ROAD) && !t.has(tile_flags::RAILROAD) && p.knows(rules::advance_ids::RAILROAD) && t.terrain != Terrain::River {
                return Some(UnitOrder::Road);
            }
            None
        }
        Terrain::Hills => {
            if !t.has(tile_flags::MINE) && !t.has(tile_flags::IRRIGATION) {
                Some(UnitOrder::Mine)
            } else {
                None
            }
        }
        Terrain::Swamp | Terrain::Jungle => {
            if matches!(t.terrain.info().irrigate, Terraform::Becomes(_)) {
                Some(UnitOrder::Irrigate)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// A tile worked by one of the owner's cities (within 8 tiles of the unit) that needs work.
fn work_site(g: &Game, u: &Unit) -> Option<((i32, i32), UnitOrder)> {
    let slot = u.owner;
    let cont = g.map.tile(u.x, u.y).continent;
    let mut best: Option<((i32, i32), UnitOrder, i32)> = None;
    for c in g.all_cities().filter(|c| c.owner == slot) {
        if Map::dist(c.pos(), u.pos()) > 8 || g.map.tile(c.x, c.y).continent != cont {
            continue;
        }
        for idx in 1..21u8 {
            let Some((x, y)) = g.radius_tile(c.id, idx) else { continue };
            if g.map.tile(x, y).worked_by != Some(c.id) {
                continue;
            }
            if g.all_units().any(|o| o.owner == slot && o.id != u.id && o.pos() == (x, y) && o.order_turns > 0) {
                continue;
            }
            if let Some(w) = useful_work(g, slot, x, y) {
                let d = Map::dist((x, y), u.pos());
                let prio = match w {
                    UnitOrder::CleanPollution => 0,
                    UnitOrder::Irrigate => 1,
                    UnitOrder::Mine => 2,
                    _ => 3,
                };
                let score = d + prio * 3;
                if best.map(|(_, _, s)| score < s).unwrap_or(true) {
                    best = Some(((x, y), w, score));
                }
            }
        }
    }
    best.map(|(p, w, _)| (p, w))
}

// ---- roles -----------------------------------------------------------------------------------

fn act_settlers(g: &mut Game, id: UnitId, sink: &mut dyn FnMut(TurnStep)) {
    let u = g.unit(id).clone();
    let slot = u.owner;
    let n_cities = g.cities_of(slot).len();
    // first city: settle right away unless the start tile is poor and a clearly better site
    // lies a few tiles away (a starving capital never grows enough to build settlers)
    if n_cities == 0 && g.city_at(u.x, u.y).is_none() {
        let here = crate::game::mapgen::site_score(&g.map, u.x, u.y);
        let better = match u.goto {
            Some(t) if site_ok(g, slot, t) => Some(t),
            _ => best_city_site(g, slot, u.pos(), 4).filter(|t| crate::game::mapgen::site_score(&g.map, t.0, t.1) >= here + 40 && here < 150),
        };
        if let Some(t) = better {
            if t != u.pos() {
                g.set_goto(id, t.0, t.1);
                match move_toward(g, id, t, sink) {
                    Progress::Arrived => {
                        let _ = g.found_city(id);
                        return;
                    }
                    Progress::Dead | Progress::Partial => return,
                    _ => {}
                }
            }
        }
        if g.found_city(id).is_ok() {
            return;
        }
        // not allowed here (adjacent city / ocean): walk somewhere acceptable
        if let Some(t) = best_city_site(g, slot, u.pos(), 6) {
            g.set_goto(id, t.0, t.1);
            if move_toward(g, id, t, sink) == Progress::Arrived {
                let _ = g.found_city(id);
            }
            return;
        }
    }
    let settlers_out = g.all_units().filter(|o| o.owner == slot && o.kind == unit_ids::SETTLERS).count();
    // a colonist landed on a new continent always founds a city there
    let colonist = !has_city_on_continent(g, slot, u.pos());
    let want_expand = colonist || n_cities < city_target(g, slot) || settlers_out > n_cities / 2 + 1;
    if want_expand {
        let mut target = match u.goto {
            Some(t) if site_ok(g, slot, t) && g.map.tile(t.0, t.1).continent == g.map.tile(u.x, u.y).continent => Some(t),
            _ => best_city_site(g, slot, u.pos(), SITE_RADIUS + 2),
        };
        if colonist && target.is_none() {
            // poor island: take what there is, or build right here
            target = best_city_site_min(g, slot, u.pos(), 6, 40).or_else(|| if g.city_at(u.x, u.y).is_none() { Some(u.pos()) } else { None });
        }
        if let Some(t) = target {
            if u.pos() == t {
                if g.found_city(id).is_ok() {
                    return;
                }
            } else {
                g.set_goto(id, t.0, t.1);
                match move_toward(g, id, t, sink) {
                    Progress::Arrived => {
                        if g.found_city(id).is_ok() {
                            return;
                        }
                    }
                    Progress::Dead | Progress::Partial => return,
                    Progress::Blocked | Progress::Outcome(_) => {
                        if let Some(um) = g.units[id as usize].as_mut() {
                            um.goto = None;
                            um.order = UnitOrder::None;
                        }
                    }
                }
            }
        }
    }
    let Some(u) = g.unit_opt(id).cloned() else { return };
    if u.moves_left == 0 {
        return;
    }
    // no site left on this landmass but a ship is waiting in a harbour for colonists: go aboard
    if !colonist {
        if let Some(port) = naval_ai::colonist_port(g, &u) {
            if port == u.pos() {
                let _ = g.give_order(id, UnitOrder::Sentry);
                return;
            }
            g.set_goto(id, port.0, port.1);
            match move_toward(g, id, port, sink) {
                Progress::Arrived => {
                    let _ = g.give_order(id, UnitOrder::Sentry);
                    return;
                }
                Progress::Partial | Progress::Dead => return,
                _ => {}
            }
        }
    }
    // terraforming around own cities
    let Some(u) = g.unit_opt(id).cloned() else { return };
    if u.moves_left == 0 {
        return;
    }
    if let Some(w) = useful_work(g, slot, u.x, u.y) {
        if g.map.tile(u.x, u.y).worked_by.map(|c| g.city(c).owner == slot).unwrap_or(false) && g.give_order(id, w).is_ok() {
            return;
        }
    }
    if let Some((p, w)) = work_site(g, &u) {
        if p == u.pos() {
            let _ = g.give_order(id, w);
            return;
        }
        g.set_goto(id, p.0, p.1);
        if move_toward(g, id, p, sink) == Progress::Arrived {
            let _ = g.give_order(id, w);
        }
        return;
    }
    // nothing to do: join a small city we stand in, else wait in the nearest city
    if let Some(c) = g.city_at(u.x, u.y) {
        if g.city(c).owner == slot && g.city(c).size < 8 && n_cities >= city_target(g, slot) {
            let _ = g.found_city(id);
            return;
        }
        let _ = g.give_order(id, UnitOrder::Sentry);
        return;
    }
    if let Some(c) = nearest_own_city(g, slot, u.pos(), None) {
        let p = g.city(c).pos();
        g.set_goto(id, p.0, p.1);
        move_toward(g, id, p, sink);
    }
}

/// Nearest own city on the same landmass (optionally filtered).
fn nearest_own_city(g: &Game, slot: Slot, from: (i32, i32), filter: Option<&dyn Fn(&Game, CityId) -> bool>) -> Option<CityId> {
    let cont = g.map.tile(from.0, from.1).continent;
    g.all_cities()
        .filter(|c| c.owner == slot && (cont == 0 || g.map.tile(c.x, c.y).continent == cont))
        .filter(|c| filter.map(|f| f(g, c.id)).unwrap_or(true))
        .min_by_key(|c| Map::dist(c.pos(), from))
        .map(|c| c.id)
}

/// Defensive units (role Defend) of the owner standing in the city.
pub fn defenders_in(g: &Game, c: CityId) -> usize {
    let city = g.city(c);
    g.all_units().filter(|u| u.owner == city.owner && u.pos() == city.pos() && u.is_land() && u.info().role == AiRole::Defend).count()
}

/// Military land units of the owner standing in the city (any role).
pub fn garrison_in(g: &Game, c: CityId) -> usize {
    let city = g.city(c);
    g.all_units().filter(|u| u.owner == city.owner && u.pos() == city.pos() && u.is_land() && u.is_military()).count()
}

/// How many defenders a city wants: 2, 3 for the capital, +1 when enemies are near.
pub fn defenders_needed(g: &Game, c: CityId) -> usize {
    let city = g.city(c);
    let mut n = if g.player(city.owner).capital == Some(c) { 3 } else { 2 };
    if city.size <= 2 && n > 1 && g.turn < 40 {
        n = 1;
    }
    if enemy_near(g, city.owner, city.pos(), 4) {
        n += 1;
    }
    n
}

/// Is an enemy military unit within `r` tiles?
pub fn enemy_near(g: &Game, slot: Slot, p: (i32, i32), r: i32) -> bool {
    g.all_units().any(|u| hostile(g, slot, u.owner) && u.is_military() && Map::dist(u.pos(), p) <= r)
}

fn act_defender(g: &mut Game, id: UnitId, sink: &mut dyn FnMut(TurnStep)) {
    let u = g.unit(id).clone();
    let slot = u.owner;
    // adjacent enemies with good odds are attacked before settling in
    if opportunistic_attack(g, id, 65, sink) {
        return;
    }
    let Some(u) = g.unit_opt(id).cloned() else { return };
    if let Some(c) = g.city_at(u.x, u.y) {
        if g.city(c).owner == slot && defenders_in(g, c) <= defenders_needed(g, c) {
            if u.order != UnitOrder::Fortified {
                let _ = g.give_order(id, UnitOrder::Fortify);
            }
            return;
        }
    }
    if u.order == UnitOrder::Fortified && g.city_at(u.x, u.y).is_none() {
        // fortified in the field (fortress / stack guard): stay unless a city needs us
    }
    let needy = nearest_own_city(g, slot, u.pos(), Some(&|g: &Game, c: CityId| defenders_in(g, c) < defenders_needed(g, c)));
    let dest = needy.or_else(|| nearest_own_city(g, slot, u.pos(), None));
    match dest {
        Some(c) => {
            let p = g.city(c).pos();
            if p == u.pos() {
                let _ = g.give_order(id, UnitOrder::Fortify);
                return;
            }
            g.set_goto(id, p.0, p.1);
            match move_toward(g, id, p, sink) {
                Progress::Arrived => {
                    let _ = g.give_order(id, UnitOrder::Fortify);
                }
                Progress::Blocked => {
                    let _ = g.give_order(id, UnitOrder::Fortify);
                }
                _ => {}
            }
        }
        None => {
            let _ = g.give_order(id, UnitOrder::Fortify);
        }
    }
}

/// Is this attacker one of the civilisation's explorers (the first two by id)?
fn is_explorer(g: &Game, u: &Unit) -> bool {
    let mut n = 0;
    for o in g.all_units() {
        if o.owner == u.owner && o.is_land() && o.info().role == AiRole::Attack {
            if o.id == u.id {
                return n < 2;
            }
            n += 1;
            if n >= 2 {
                return false;
            }
        }
    }
    false
}

fn act_attacker(g: &mut Game, id: UnitId, sink: &mut dyn FnMut(TurnStep)) {
    let u = g.unit(id).clone();
    let slot = u.owner;
    if u.carried_by.is_some() {
        return;
    }
    // 1. a target worth attacking
    let mut attacks = 0;
    while attacks < 3 {
        let Some(cur) = g.unit_opt(id).cloned() else { return };
        if cur.moves_left == 0 {
            return;
        }
        let Some(t) = find_target(g, &cur, 10, 40) else { break };
        if Map::dist(t, cur.pos()) == 1 {
            let Some(d) = Dir8::from_delta(t.0 - cur.x, t.1 - cur.y) else { break };
            match step(g, id, d, sink) {
                Ok(MoveOutcome::Attacked { .. }) => attacks += 1,
                Ok(MoveOutcome::CapturedCity(_)) | Ok(MoveOutcome::DestroyedCity) => return,
                _ => break,
            }
        } else {
            g.set_goto(id, t.0, t.1);
            match move_toward(g, id, t, sink) {
                Progress::Blocked => break,
                _ => return,
            }
        }
    }
    let Some(u) = g.unit_opt(id).cloned() else { return };
    if u.moves_left == 0 {
        return;
    }
    // 2. explore
    if is_explorer(g, &u) {
        if let Some(t) = nearest_unexplored(g, &u, 12) {
            g.set_goto(id, t.0, t.1);
            if move_toward(g, id, t, sink) != Progress::Blocked {
                return;
            }
        }
    }
    // 3. rally in the own city closest to the enemy and wait
    let enemies: Vec<(i32, i32)> = g.all_units().filter(|o| hostile(g, slot, o.owner) && o.is_military()).map(|o| o.pos()).chain(g.all_cities().filter(|c| hostile(g, slot, c.owner)).map(|c| c.pos())).collect();
    let rally = if enemies.is_empty() {
        nearest_own_city(g, slot, u.pos(), None)
    } else {
        let cont = g.map.tile(u.x, u.y).continent;
        // enemy only overseas: gather in a harbour so a transport can pick the army up
        let overseas = !enemies.iter().any(|e| g.map.tile(e.0, e.1).continent == cont);
        let has_port = g.all_cities().any(|c| c.owner == slot && g.map.tile(c.x, c.y).continent == cont && g.map.is_coastal(c.x, c.y));
        g.all_cities()
            .filter(|c| c.owner == slot && g.map.tile(c.x, c.y).continent == cont && (!overseas || !has_port || g.map.is_coastal(c.x, c.y)))
            .min_by_key(|c| enemies.iter().map(|e| Map::dist(*e, c.pos())).min().unwrap_or(999))
            .map(|c| c.id)
    };
    match rally {
        Some(c) => {
            let p = g.city(c).pos();
            if p == u.pos() {
                let _ = g.give_order(id, UnitOrder::Sentry);
            } else {
                g.set_goto(id, p.0, p.1);
                if move_toward(g, id, p, sink) == Progress::Arrived {
                    let _ = g.give_order(id, UnitOrder::Sentry);
                }
            }
        }
        None => {
            let _ = g.give_order(id, UnitOrder::Fortify);
        }
    }
}

fn act_ship(g: &mut Game, id: UnitId, sink: &mut dyn FnMut(TurnStep)) {
    let u = g.unit(id).clone();
    let slot = u.owner;
    if u.info().att > 0 && opportunistic_attack(g, id, 55, sink) {
        return;
    }
    let Some(u) = g.unit_opt(id).cloned() else { return };
    if u.moves_left == 0 {
        return;
    }
    // carrying units: bring them to the nearest coast with a friendly or empty landing
    if g.cargo_count(id) > 0 {
        if let Some(c) = nearest_own_city(g, slot, u.pos(), None) {
            let p = g.city(c).pos();
            g.set_goto(id, p.0, p.1);
            if move_toward(g, id, p, sink) == Progress::Arrived {
                g.unload(id);
            }
            return;
        }
    }
    // explore the seas (triremes hug the coast)
    if let Some(t) = nearest_unexplored(g, &u, 14) {
        g.set_goto(id, t.0, t.1);
        if move_toward(g, id, t, sink) != Progress::Blocked {
            return;
        }
    }
    // nothing to see: wait in port
    if let Some(c) = g.city_at(u.x, u.y) {
        if g.city(c).owner == slot {
            let _ = g.give_order(id, UnitOrder::Sentry);
            return;
        }
    }
    let port = g.all_cities().filter(|c| c.owner == slot && g.map.is_coastal(c.x, c.y)).min_by_key(|c| Map::dist(c.pos(), u.pos())).map(|c| c.pos());
    if let Some(p) = port {
        g.set_goto(id, p.0, p.1);
        if move_toward(g, id, p, sink) == Progress::Arrived {
            let _ = g.give_order(id, UnitOrder::Sentry);
        }
    }
}

fn act_air(g: &mut Game, id: UnitId, sink: &mut dyn FnMut(TurnStep)) {
    let u = g.unit(id).clone();
    let slot = u.owner;
    let in_base = g.city_at(u.x, u.y).map(|c| g.city(c).owner == slot).unwrap_or(false) || u.carried_by.is_some();
    if u.kind == unit_ids::NUCLEAR {
        // only against a stronger enemy, on a sizeable city without SDI cover
        if !g.nukes_enabled || !in_base {
            return;
        }
        let target = g
            .all_cities()
            .filter(|c| hostile(g, slot, c.owner) && c.owner != 0 && c.size >= 5 && Map::dist(c.pos(), u.pos()) <= 12 && !g.sdi_protects(slot, c.x, c.y) && g.power_ratio(slot, c.owner) < 80)
            .min_by_key(|c| Map::dist(c.pos(), u.pos()))
            .map(|c| c.pos());
        if let Some(t) = target {
            match move_toward(g, id, t, sink) {
                Progress::Partial => {
                    // adjacent now? strike
                    if let Some(cur) = g.unit_opt(id).cloned() {
                        if Map::dist(cur.pos(), t) == 1 && cur.moves_left > 0 {
                            if let Some(d) = Dir8::from_delta(t.0 - cur.x, t.1 - cur.y) {
                                let _ = step(g, id, d, sink);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        return;
    }
    if in_base {
        // strike adjacent enemies with good odds, then stay home
        opportunistic_attack(g, id, 60, sink);
        if let Some(cur) = g.unit_opt(id) {
            if cur.order == UnitOrder::None {
                let _ = g.give_order(id, UnitOrder::Sentry);
            }
        }
        return;
    }
    // airborne: return to the nearest own city
    if let Some(c) = g.all_cities().filter(|c| c.owner == slot).min_by_key(|c| Map::dist(c.pos(), u.pos())).map(|c| c.id) {
        let p = g.city(c).pos();
        move_toward(g, id, p, sink);
    }
}

/// A computer Diplomat (CIV.EXE 0x16a39): one with a goto keeps going (a new Diplomat is
/// sent towards a human city when it is built, `Game::ai_diplomat_site`); otherwise it makes
/// for the nearest city (of anyone, 0x1d43:0xf8) when that is foreign and disbands when it is
/// its own. Coming into a foreign city it steals or sabotages (`Game::ai_diplomat_action`);
/// the computer never establishes embassies (only the human's Diplomats do, o 0x1d6c1).
fn act_diplomat(g: &mut Game, id: UnitId, sink: &mut dyn FnMut(TurnStep)) {
    let u = g.unit(id).clone();
    let slot = u.owner;
    // the goto of a Diplomat whose target is gone or became its own is dropped
    let goto = u.goto.filter(|(x, y)| g.city_at(*x, *y).is_some_and(|c| g.city(c).owner != slot));
    let target = match goto {
        Some(t) => t,
        None => {
            let Some(c) = g.all_cities().min_by_key(|c| Map::civ_distance(c.pos(), u.pos())).map(|c| c.id) else { return };
            if g.city(c).owner == slot {
                g.remove_unit(id);
                g.refresh_vision(slot);
                return;
            }
            let p = g.city(c).pos();
            g.set_goto(id, p.0, p.1);
            p
        }
    };
    match move_toward(g, id, target, sink) {
        Progress::Outcome(MoveOutcome::EnteredForeignCity(cid)) => {
            g.ai_diplomat_action(id, cid);
        }
        // no way there [approx: the original's goto handling for a blocked unit is not
        // decoded]: look for the nearest city again next turn
        Progress::Blocked => {
            if let Some(u) = g.unit_opt(id) {
                if u.goto.is_some() {
                    let um = g.unit_mut(id);
                    um.goto = None;
                    um.order = UnitOrder::None;
                }
            }
        }
        _ => {}
    }
}

fn act_caravan(g: &mut Game, id: UnitId, sink: &mut dyn FnMut(TurnStep)) {
    let u = g.unit(id).clone();
    let slot = u.owner;
    let cont = g.map.tile(u.x, u.y).continent;
    // help a wonder
    let wonder_city = g
        .all_cities()
        .filter(|c| c.owner == slot && matches!(c.production, crate::game::Production::Wonder(_)) && g.map.tile(c.x, c.y).continent == cont)
        .min_by_key(|c| Map::dist(c.pos(), u.pos()))
        .map(|c| c.id);
    if let Some(c) = wonder_city {
        let p = g.city(c).pos();
        g.set_goto(id, p.0, p.1);
        if p == u.pos() || move_toward(g, id, p, sink) == Progress::Arrived {
            let cm = g.city_mut(c);
            cm.shields += 50;
            cm.caravan_shields += 50;
            g.remove_unit(id);
            g.refresh_vision(slot);
        }
        return;
    }
    // trade route with the farthest own city (10+ tiles away)
    let home = u.home.filter(|h| g.city_opt(*h).is_some());
    if let Some(h) = home {
        let hp = g.city(h).pos();
        let routes = g.city(h).trade_routes.clone();
        if routes.len() < 3 {
            let dest = g
                .all_cities()
                .filter(|c| c.id != h && (c.owner == slot || g.treaty(slot, c.owner)) && g.map.tile(c.x, c.y).continent == cont && Map::dist(c.pos(), hp) >= 10 && !routes.contains(&c.id))
                .max_by_key(|c| Map::dist(c.pos(), hp) + if c.owner != slot { 6 } else { 0 })
                .map(|c| c.id);
            if let Some(d) = dest {
                let p = g.city(d).pos();
                g.set_goto(id, p.0, p.1);
                let arrived = p == u.pos() || matches!(move_toward(g, id, p, sink), Progress::Arrived | Progress::Outcome(MoveOutcome::EnteredForeignCity(_)));
                if arrived && g.unit_opt(id).is_some() {
                    let _ = g.caravan_action(id, crate::game::turn::CaravanAction::TradeRoute(d));
                }
                return;
            }
        }
    }
    // nothing useful: add shields to the city we are in, else wait
    if let Some(c) = g.city_at(u.x, u.y) {
        if g.city(c).owner == slot {
            g.disband(id);
            return;
        }
    }
    let _ = g.give_order(id, UnitOrder::Sentry);
}

/// Act with one unit of a computer civilisation.
pub fn act(g: &mut Game, id: UnitId, sink: &mut dyn FnMut(TurnStep)) {
    let Some(u) = g.unit_opt(id).cloned() else { return };
    if u.moves_left == 0 || u.order_turns > 0 {
        return;
    }
    // aboard a ship: the ship moves us
    if u.carried_by.is_some() && u.is_land() {
        return;
    }
    match u.kind {
        unit_ids::SETTLERS => act_settlers(g, id, sink),
        unit_ids::DIPLOMAT => act_diplomat(g, id, sink),
        unit_ids::CARAVAN => act_caravan(g, id, sink),
        _ => match u.info().role {
            AiRole::Defend => act_defender(g, id, sink),
            AiRole::Attack => {
                if u.is_air() {
                    act_air(g, id, sink)
                } else {
                    act_attacker(g, id, sink)
                }
            }
            AiRole::Air => act_air(g, id, sink),
            AiRole::SeaAttack => act_ship(g, id, sink),
            // transport ships are driven by `naval_ai::play_transports`
            AiRole::Transport => {}
            AiRole::Settle => act_settlers(g, id, sink),
            AiRole::Civilian => {
                let _ = g.give_order(id, UnitOrder::Sentry);
            }
        },
    }
}

/// Move every unit of a computer civilisation.
pub fn play_units(g: &mut Game, slot: Slot, sink: &mut dyn FnMut(TurnStep)) {
    // settlers first (they found cities others defend), then defenders, then the rest
    let mut ids = g.units_of(slot);
    ids.sort_by_key(|id| {
        let u = g.unit(*id);
        match u.kind {
            unit_ids::SETTLERS => 0,
            _ => match u.info().role {
                AiRole::Defend => 1,
                _ => 2,
            },
        }
    });
    for id in ids {
        if g.unit_opt(id).map(|u| u.owner == slot).unwrap_or(false) {
            act(g, id, sink);
        }
    }
}

/// Move the barbarians as CIV.EXE's `move_units` (0x403e) does for civ 0: units in index
/// order, sentried and fortified ones skipped, each asked for orders by the barbarian routine
/// (`Game::barbarian_command`, 0x18dc3) while it has moves, at most five times (0x4464).
pub fn play_barbarians(g: &mut Game, sink: &mut dyn FnMut(TurnStep)) {
    use crate::game::barbarians::BarbarianCommand as Cmd;
    let mut ids = g.units_of(0);
    ids.sort_unstable();
    for id in ids {
        for _ in 0..5 {
            let Some(u) = g.unit_opt(id) else { break };
            if u.moves_left == 0 || matches!(u.order, UnitOrder::Sentry | UnitOrder::Fortified) {
                break;
            }
            match g.barbarian_command(id) {
                Cmd::Step(d) => {
                    if g.unit_opt(id).is_some() {
                        g.unit_mut(id).goto = None;
                    }
                    if step(g, id, d, sink).is_err() {
                        break;
                    }
                }
                Cmd::Goto => {
                    let Some(d) = g.next_goto_step(id) else { break };
                    if !matches!(step(g, id, d, sink), Ok(MoveOutcome::Moved) | Ok(MoveOutcome::EnteredHut)) {
                        break;
                    }
                }
                Cmd::Skip => {
                    g.unit_mut(id).moves_left = 0;
                    break;
                }
                Cmd::Fortify => {
                    let um = g.unit_mut(id);
                    um.order = UnitOrder::Fortify;
                    um.moves_left = 0;
                    break;
                }
                // the passengers wake up; those after the ship in unit order move this turn
                Cmd::Unload => {
                    for c in g.cargo_of(id) {
                        g.unit_mut(c).order = UnitOrder::None;
                    }
                    break;
                }
                Cmd::Disband => break,
            }
        }
    }
    g.refresh_vision(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{GameEvent, GameSetup};
    use crate::rules::{unit_ids, Terrain};

    #[test]
    fn events_come_out_around_the_move_that_made_them() {
        let mut g = Game::new(GameSetup { seed: 21, num_civs: 4, ..Default::default() }, None);
        for s in 1..5 {
            g.players[s].in_game = true;
            g.players[s].alive = true;
        }
        for y in 5..30 {
            for x in 5..40 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Grassland;
                t.flags = 0;
                t.continent = 1;
            }
        }
        let ids: Vec<_> = g.all_units().map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        // the human hears of what civs 2 and 3 agree when they meet
        let h = g.human_slot;
        g.raise_flag(h, 2, crate::game::diplomacy::flags::EMBASSY);
        g.add_city(2, "A".into(), 10, 10);
        g.add_city(3, "B".into(), 30, 10);
        g.add_unit(2, unit_ids::MILITIA, 10, 10);
        let k = g.add_unit(3, unit_ids::KNIGHTS, 12, 10);
        let c = g.cities_of(2)[0];
        g.push_event(GameEvent::CityGrew(c));
        let mut steps = Vec::new();
        step(&mut g, k, Dir8::W, &mut |s| steps.push(s)).unwrap();
        assert!(matches!(steps[0], TurnStep::Event(GameEvent::CityGrew(_))), "{steps:?}");
        assert!(matches!(steps[1], TurnStep::AiMove { .. }), "{steps:?}");
        assert!(matches!(steps[2], TurnStep::Event(GameEvent::WarDeclared { .. } | GameEvent::PeaceTreaty { .. })), "{steps:?}");
        assert!(g.events.is_empty());
    }
}
