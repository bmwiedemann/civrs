//! The computer players' units, as CIV.EXE moves them.
//!
//! - `civ_turn` is the part of `ai_civ_turn` (0x1579a) that civrs's census (`Game::census`,
//!   `Game::continent_policy` in `game::production`) leaves: the unit census flags and the
//!   20x13 presence map, the reserve, the city goals, the continent policies with their
//!   production re-choice, and the unit tasking that sends free units to the goal with the best
//!   distance / priority ratio.
//! - `unit_command` is `ai_unit_command` (0x16433): it returns a keyboard command, which
//!   `move_units` (the computer's branch of 0x403e) carries out through the engine calls the
//!   human's keys use (`Game::try_move` with its zones of control, treaties and terrain costs;
//!   `Game::give_order`; `Game::found_city`; the goto of `game::goto`).
//! - Barbarians have their own routine (`Game::barbarian_command`, 0x18dc3), driven by
//!   `play_barbarians`.
//!
//! Every move is reported to the turn `sink` as `TurnStep::AiMove` / `TurnStep::Combat` with a
//! `visible` flag telling the UI whether the human can see either square.
//!
//! civrs keeps no unit "seen by" byte (+9): a unit counts as seen by another civilization while
//! that civilization can see its square now (unverified approximation).

use crate::game::goals::goal;
use crate::game::goto::{dist_metric, exe_in_map, turn_angle, DX, DY};
use crate::game::production::{continent_index, Census, POLICY_HOSTILE, POLICY_NONE, POLICY_OFFENSIVE, POLICY_THREATENED};
use crate::game::turn::TurnStep;
use crate::game::{tile_flags, CityId, Dir8, Game, Map, MoveOutcome, RuleError, Slot, UnitId, UnitOrder};
use crate::rules::{self, advance_ids as adv, improvement_ids as imp, unit_ids, AiRole, Domain, Govt, Terrain};

/// The shields byte (+0xf) of CIV.EXE's terrain table ds:0x28e, as the Settlers' exploration
/// reads it (0x17c7e; Grassland and River have 1 there, the city routine takes it off).
const EXE_SHIELDS: [i32; 12] = [1, 1, 1, 2, 0, 1, 0, 0, 0, 0, 0, 1];

/// Words 4 and 5 of the terrain's irrigation/mining row ds:0x44a (12 bytes a terrain): whether
/// the computer's Settlers improve the terrain without Monarchy (word 4) and with it (word 5,
/// 0x17790).
const AI_WORKS: [[i32; 2]; 12] = [[0, 0], [1, 1], [0, 1], [0, 0], [1, 1], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 1]];

/// The irrigation result (word 0) and mining result (word 2) of ds:0x44a: -2 needs water, -1
/// impossible, >= 0 changes the terrain, < -2 a bonus.
const EXE_IRRIGATION: [i32; 12] = [-2, -2, -2, 6, -2, -1, -1, -1, 10, 10, -1, -2];
const EXE_MINING: [i32; 12] = [-2, 2, 2, -1, -4, -2, -1, -1, 2, 2, -1, -1];

/// A keyboard command of `ai_unit_command` (see `move_units` 0x4bbe for their handlers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// 0: follow the goto, if any.
    Goto,
    /// 1..8: move or attack.
    Step(Dir8),
    /// ' ': done for this turn (moves = 0).
    Skip,
    /// 'b': found a city or join one.
    Build,
    /// 'f': fortify (land units).
    Fortify,
    /// 'h': home city here; aircraft fly toward a base (or are lost without one).
    Home,
    /// 'i', 'm', 'p', 'r': Settlers' work.
    Irrigate,
    Mine,
    Pollution,
    Road,
    /// 'u': wake the passengers.
    Unload,
    /// 'P'.
    Pillage,
    /// A key no handler takes (the Nuclear's `direction ^ 4` for north-west, 0x16a34).
    Invalid,
}

fn dir(d: usize) -> Command {
    if (1..=8).contains(&d) {
        Command::Step(Dir8::ALL[d - 1])
    } else {
        Command::Goto
    }
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

// ---- stacks -----------------------------------------------------------------------------

/// The stack of `first`'s square as CIV.EXE's stack walk (0x92a7) visits it from `first`: the
/// owner's units there, at most ten, in unit order round from `first`.
fn stack(g: &Game, first: UnitId) -> Vec<UnitId> {
    let u = g.unit(first);
    let here: Vec<UnitId> = g.units_at(u.x, u.y).into_iter().filter(|o| g.unit(*o).owner == u.owner).collect();
    let i = here.iter().position(|o| *o == first).unwrap_or(0);
    here.iter().cycle().skip(i).take(here.len().min(10)).copied().collect()
}

fn stack_count(g: &Game, first: UnitId) -> i32 {
    stack(g, first).len() as i32
}

/// `stack_sum` 0x90ad with `what` 0 cost, 1 defence, 3 attack.
fn stack_sum(g: &Game, first: UnitId, f: impl Fn(&rules::UnitType) -> u8) -> i32 {
    stack(g, first).iter().map(|o| f(g.unit(*o).info()) as i32).sum()
}

/// `stack_sum` what 4: Defend-role units of the stack with a lower index than `first`.
fn defenders_before(g: &Game, first: UnitId) -> i32 {
    stack(g, first).iter().filter(|o| **o < first && g.unit(**o).info().role == AiRole::Defend).count() as i32
}

/// `defence_value_cb` 0x8fc5: land units defence x (3 fortified, else 2) x terrain << 3, others
/// defence << 4, +50% veteran; land units at sea do not count (None).
fn defence_value(g: &Game, id: UnitId, fortified: bool) -> Option<i32> {
    let u = g.unit(id);
    let t = g.map.terrain(u.x, u.y);
    if t == Terrain::Ocean && u.info().domain != Domain::Sea {
        return None;
    }
    let mut v = if u.is_land() { u.info().def as i32 * if fortified { 3 } else { 2 } * t.info().defense as i32 * 8 } else { (u.info().def as i32) << 4 };
    if u.veteran {
        v += v / 2;
    }
    Some(v)
}

/// `best_defender_stack` 0x8f7e: the stack's unit of highest defence value (the first of
/// equals), `None` when nobody counts.
fn best_defender(g: &Game, first: UnitId) -> Option<UnitId> {
    let mut best: Option<(UnitId, i32)> = None;
    for o in stack(g, first) {
        let fortified = g.unit(o).order == UnitOrder::Fortified;
        if let Some(v) = defence_value(g, o, fortified) {
            if v > best.map(|b| b.1).unwrap_or(0) {
                best = Some((o, v));
            }
        }
    }
    best.map(|b| b.0)
}

// ---- the AI turn ------------------------------------------------------------------------

/// The policy of continent number `cont` in `slot`'s last AI turn (ds:0xe698).
fn policy(g: &Game, slot: Slot, cont: u8) -> u8 {
    g.ai_memory.policies[slot as usize].map(|p| p[continent_index(cont)]).unwrap_or(POLICY_NONE)
}

/// ds:0xe498[a][b] & 0x102 == 2: a peace treaty `a` does not mean to break.
fn keeps_peace(g: &Game, a: Slot, b: Slot) -> bool {
    use crate::game::diplomacy::flags::{BREAK, PEACE};
    g.diplo(a, b) & (PEACE | BREAK) == PEACE
}

fn peace(g: &Game, a: Slot, b: Slot) -> bool {
    g.diplo(a, b) & crate::game::diplomacy::flags::PEACE != 0
}

/// The rest of `ai_civ_turn` (0x1579a) for computer player `slot`; `ai_reset_goals` follows.
///
/// - Unit census (0x15801): every unit but the Nuclear marks its 4x4 block on the presence map
///   (bit 2 << role) and gets flag 0x10 (no tasking) next to a foreign unit or as a land unit
///   at sea; cities mark bit 1.
/// - The reserve (0x15986): clamp(units other than Settlers / 8, 3, 99).
/// - City goals (0x159be): a city of the civ with nobody in it (and no stashed garrison) asks
///   for a defender (type 2, priority 4); a foreign city of a civ it has no unbroken treaty with,
///   that it has seen (cities of computer players count as seen, and everything once turn x
///   difficulty passes 200), is an attack goal (type 1, or 3 with City Walls, priority 5) on
///   turns where (city + turn) & 3 != 0; an empty foreign city is also a type 2 goal of
///   priority 2.
/// - Policies (0x15cd1): a changed continent policy re-chooses production there (0x18bef).
/// - A civilization without Settlers and cities is destroyed (overlay 5 +0xe6c).
/// - Unit tasking (0x16057), see `task_units`.
pub fn civ_turn(g: &mut Game, slot: Slot) {
    let census = g.census();
    g.ai_memory.census = census.clone();
    census_units(g, slot);
    let reserve = (g.units_of(slot).iter().filter(|id| g.unit(**id).kind != unit_ids::NUCLEAR && g.unit(**id).kind != unit_ids::SETTLERS).count() as i32 / 8).clamp(3, 99);
    city_goals(g, slot);
    update_policies(g, &census, slot);
    if slot == 0 || g.is_human(slot) {
        return;
    }
    g.check_destroyed(slot);
    if !g.player(slot).alive {
        return;
    }
    task_units(g, slot, reserve);
}

fn census_units(g: &mut Game, slot: Slot) {
    let mut coarse = [[0u8; 13]; 20];
    let mut busy = std::collections::HashSet::new();
    for id in g.units_of(slot) {
        let u = g.unit(id);
        if u.kind == unit_ids::NUCLEAR {
            continue;
        }
        if g.enemy_adjacent(slot, u.x, u.y) || (g.map.is_ocean(u.x, u.y) && u.is_land()) {
            busy.insert(id);
        }
        coarse[(u.x / 4) as usize][(u.y / 4).min(12) as usize] |= 2 << (u.info().role as u8);
    }
    for c in g.all_cities().filter(|c| c.owner == slot) {
        coarse[(c.x / 4) as usize][(c.y / 4).min(12) as usize] |= 1;
    }
    g.ai_memory.coarse = coarse;
    g.ai_memory.busy = busy;
}

fn city_goals(g: &mut Game, slot: Slot) {
    let human = g.human_slot;
    let turn = g.turn as i32;
    let difficulty = g.setup.difficulty as i32;
    let cities: Vec<CityId> = g.all_cities().map(|c| c.id).collect();
    for c in cities {
        let city = g.city(c).clone();
        if city.owner != slot {
            let seen = turn * difficulty > 200 || g.explored(slot, city.x, city.y) || city.owner != human;
            if seen && !keeps_peace(g, slot, city.owner) && (c as i32 + turn) & 3 != 0 {
                let kind = if g.city_has_walls(&city) { goal::SEA } else { goal::ATTACK };
                g.add_goal(slot, city.x, city.y, kind, 5);
            }
        }
        if g.units_at(city.x, city.y).is_empty() && city.garrison[0] == crate::game::city::NO_GARRISON {
            let prio = if city.owner == slot { 4 } else { 2 };
            g.add_goal(slot, city.x, city.y, goal::DEFEND, prio);
        }
    }
}

fn update_policies(g: &mut Game, census: &Census, slot: Slot) {
    let mut now = [POLICY_NONE; 16];
    for (k, p) in now.iter_mut().enumerate() {
        *p = g.continent_policy(census, slot, k);
    }
    if let Some(old) = g.ai_memory.policies[slot as usize] {
        if !g.is_human(slot) && slot != 0 {
            for k in 0..16 {
                if old[k] != now[k] {
                    super::city_ai::rechoose(g, slot, Some(k));
                }
            }
        }
    }
    g.ai_memory.policies[slot as usize] = Some(now);
}

/// The unit tasking of 0x16057. Every unit but the Nuclear and those flagged 0x10:
/// - on turns where (unit + turn) & 15 == 0 an obsolete unit (its owner knows the advance that
///   makes it obsolete), or a Militia beyond Despotism or on an offensive continent, is
///   disbanded unless next to a foreign unit, the last unit in a city, at sea or on a
///   threatened continent;
/// - it takes the goal of its role on its continent (Bombers: any attack goal of odd priority
///   on alternate turns, or a walled city) with the lowest distance x units already sent /
///   (priority + 1) (Bombers 2 within reach, else 4 x distance, over priority + 1), when that
///   is at most 4 x priority x the reserve; fortified units only from outside a city, for
///   goals of priority 2 or more that are near enough (priority x reserve >= the score) or
///   untouched. The goal's count starts at the reserve and rises by one per unit sent.
fn task_units(g: &mut Game, slot: Slot, reserve: i32) {
    let mut assigned = [reserve; crate::game::goals::GOALS];
    let turn = g.turn as i32;
    for id in g.units_of(slot) {
        let Some(u) = g.unit_opt(id).cloned() else { continue };
        // civrs: units at work are left at it. CIV.EXE clears their order bits but keeps the
        // work counter (+5), and the unit routine gives the same work key again on the square.
        if g.ai_memory.busy.contains(&id) || u.kind == unit_ids::NUCLEAR || u.order_turns > 0 {
            continue;
        }
        let cont = g.continent_exe(u.x, u.y);
        let pol = policy(g, slot, cont);
        if (id as i32 + turn) & 0xf == 0 {
            let p = g.player(slot);
            let obsolete = u.info().obsolete_by.map(|a| p.knows(a)).unwrap_or(false);
            let militia = (p.govt as u8 > Govt::Despotism as u8 || pol == POLICY_OFFENSIVE) && u.kind == unit_ids::MILITIA;
            let alone_in_city = g.city_at(u.x, u.y).is_some() && g.units_at(u.x, u.y).len() == 1;
            if (obsolete || militia) && !g.enemy_adjacent(slot, u.x, u.y) && !alone_in_city && !g.map.is_ocean(u.x, u.y) && pol != POLICY_THREATENED {
                if trace() { eprintln!("TRACE t{} s{} #{} {} DISBAND obsolete={} militia={}", g.turn, slot, id, u.info().name, obsolete, militia); }
                g.remove_unit(id);
                continue;
            }
        }
        let role = u.info().role as i8;
        let fortified = matches!(u.order, UnitOrder::Fortify | UnitOrder::Fortified);
        let mut best = 9999;
        let mut pick = None;
        for i in 0..crate::game::goals::GOALS {
            let gl = g.ai_memory.goals[slot as usize][i];
            if gl.is_free() {
                continue;
            }
            let same = gl.kind == role && g.continent_exe(gl.x as i32, gl.y as i32) == cont;
            let bomber = u.kind == unit_ids::BOMBER && (gl.kind == role || gl.kind == goal::SEA);
            if !same && !bomber {
                continue;
            }
            let prio = gl.priority as i32;
            let d = Map::civ_distance(u.pos(), gl.pos());
            let score = if u.kind == unit_ids::BOMBER {
                if prio & 1 == 0 || (i as i32 + turn / 2) & 1 == 0 {
                    continue;
                }
                if u.info().moves as i32 >= d {
                    2 / (prio + 1)
                } else {
                    4 * d / (prio + 1)
                }
            } else {
                d * assigned[i] / (prio + 1)
            };
            if fortified {
                if prio < 2 || !(prio * reserve >= score || assigned[i] == reserve) || g.city_at(u.x, u.y).is_some() {
                    continue;
                }
            }
            if score < best && 4 * prio >= score / reserve {
                best = score;
                pick = Some(i);
            }
        }
        if let Some(i) = pick {
            let to = g.ai_memory.goals[slot as usize][i].pos();
            if trace() {
                let gl = g.ai_memory.goals[slot as usize][i];
                eprintln!("TRACE t{} s{} #{} {} at {:?} TASK goal {:?} score {}", g.turn, slot, id, u.info().name, u.pos(), gl, best);
            }
            g.ai_set_goto(id, to);
            assigned[i] += 1;
        }
    }
    g.refresh_vision(slot);
}

// ---- the unit command -------------------------------------------------------------------

/// What the routine works out first (0x166a9).
struct Ctx {
    slot: Slot,
    id: UnitId,
    x: i32,
    y: i32,
    kind: u8,
    role: AiRole,
    /// `zoc_blocked` of the unit's square.
    zoc: bool,
    /// The nearest city (any owner, 0x1d43:0xf8) and its distance.
    city: Option<CityId>,
    city_dist: i32,
    /// Distance to the nearest other unit of the civilization.
    unit_dist: i32,
    terrain: Terrain,
    cont: u8,
    policy: u8,
}

impl Ctx {
    fn city_owner(&self, g: &Game) -> Option<Slot> {
        self.city.map(|c| g.city(c).owner)
    }
    fn city_size(&self, g: &Game) -> i32 {
        self.city.map(|c| g.city(c).size as i32).unwrap_or(0)
    }
}

/// A goto written by the unit routine itself (bytes +6/+7 only).
fn set_goto(g: &mut Game, id: UnitId, to: (i32, i32)) {
    let u = g.unit_mut(id);
    u.goto = Some(to);
    if u.order == UnitOrder::None {
        u.order = UnitOrder::Goto;
    }
}

/// Fuel left in CIV.EXE's terms (unit byte +5: the range at a base, one less each turn
/// away; civrs counts the turns spent away).
fn fuel_left(u: &crate::game::Unit) -> i32 {
    u.info().range as i32 - 1 - u.fuel as i32
}

/// CIV.EXE 0x16433 `ai_unit_command(civ, unit)` for a unit of computer player `slot`.
pub fn unit_command(g: &mut Game, slot: Slot, id: UnitId) -> Command {
    let u = g.unit(id).clone();
    // Bombers out of fuel go home (0x16475)
    if u.kind == unit_ids::BOMBER && fuel_left(&u) <= 0 {
        return Command::Home;
    }
    if u.kind == unit_ids::FIGHTER && u.goto.is_none() {
        if let Some(c) = fighter(g, slot, id) {
            return c;
        }
    }
    let u = g.unit(id).clone();
    let (city, city_dist) = g.nearest_city_exe(u.x, u.y);
    let unit_dist = g.all_units().filter(|o| o.owner == slot && o.id != id).map(|o| Map::civ_distance(o.pos(), u.pos())).min().unwrap_or(999);
    let cont = g.continent_exe(u.x, u.y);
    let mut c = Ctx {
        slot,
        id,
        x: u.x,
        y: u.y,
        kind: u.kind,
        role: u.info().role,
        zoc: g.zoc_blocked(slot, u.x, u.y),
        city,
        city_dist,
        unit_dist,
        terrain: g.map.terrain(u.x, u.y),
        cont,
        policy: policy(g, slot, cont),
    };
    if u.kind == unit_ids::NUCLEAR {
        return nuclear(g, &c);
    }
    if u.kind == unit_ids::DIPLOMAT {
        if let Some(cmd) = diplomat(g, &c) {
            return cmd;
        }
    }
    if let Some(cmd) = city_site(g, &c) {
        return cmd;
    }
    if g.city_at(c.x, c.y).is_some() {
        if let Some(cmd) = in_city(g, &c) {
            return cmd;
        }
    }
    if matches!(c.role, AiRole::Transport | AiRole::SeaAttack) {
        if let Some(cmd) = transport(g, &c) {
            return cmd;
        }
    }
    if let Some(cmd) = pillage(g, &c) {
        return cmd;
    }
    if c.role == AiRole::Settle {
        if let Some(cmd) = settlers_work(g, &c) {
            return cmd;
        }
    }
    military(g, &mut c)
}

/// Fighters without a goto (0x16499): home when half their moves are spent; else after the
/// human's Bomber they see (when the last one listed is more than one square away), when
/// it is within their move rate; else to the own city within reach nearest that Bomber
/// (' ' when there is none).
fn fighter(g: &mut Game, slot: Slot, id: UnitId) -> Option<Command> {
    let u = g.unit(id).clone();
    let rate = u.info().moves as i32;
    if rate / 2 * 3 >= u.moves_left as i32 {
        return Some(Command::Home);
    }
    let h = g.human_slot;
    let mut nearest: Option<(UnitId, i32)> = None;
    let mut last = 0;
    for b in g.all_units().filter(|b| b.owner == h && b.kind == unit_ids::BOMBER && g.visible_to(slot, b.x, b.y)) {
        let d = Map::civ_distance(u.pos(), b.pos());
        last = d;
        if nearest.map(|n| d < n.1).unwrap_or(true) {
            nearest = Some((b.id, d));
        }
    }
    let (bomber, d) = nearest?;
    if last <= 1 {
        return None;
    }
    let bpos = g.unit(bomber).pos();
    if rate > d {
        set_goto(g, id, bpos);
        return None;
    }
    let base = g.all_cities().filter(|c| c.owner == slot && rate >= Map::civ_distance(u.pos(), c.pos())).min_by_key(|c| (Map::civ_distance(bpos, c.pos()), c.id)).map(|c| c.pos());
    match base {
        Some(p) => {
            set_goto(g, id, p);
            None
        }
        None => Some(Command::Skip),
    }
}

/// The Nuclear (0x1677e): the city of a civilization that has used nuclear weapons on this
/// one (diplomacy bit 0x80, no treaty) and is either clearly stronger, or a vendetta enemy,
/// or without Nuclear units and stronger by less than half, of size 5 or more, without SDI
/// Defense and within 16 of an own city, scored size / 2 + 1 per unit of such a civ around it,
/// -2 per own unit, -99 per unit of anyone else. With a score of 10 or more and more than one
/// Nuclear unit, the unit appears on the first free square next to the city and "moves" into
/// it (direction ^ 4, as CIV.EXE computes it: 0 for south-east, no key for north-west).
fn nuclear(g: &mut Game, c: &Ctx) -> Command {
    use crate::game::diplomacy::flags::{NUCLEAR, PEACE, VENDETTA};
    let slot = c.slot;
    let strength = |g: &Game, s: Slot| -> i32 { g.all_units().filter(|u| u.owner == s).map(|u| u.info().att as i32 + u.info().def as i32).sum() };
    let mine = strength(g, slot);
    let nukes = |g: &Game, s: Slot| g.all_units().filter(|u| u.owner == s && u.kind == unit_ids::NUCLEAR).count();
    let nuked_us = |g: &Game, o: Slot| g.diplo(slot, o) & (NUCLEAR | PEACE) == NUCLEAR;
    let mut best = (-1, None);
    let cities: Vec<CityId> = g.all_cities().map(|c| c.id).collect();
    for cid in cities {
        let city = g.city(cid).clone();
        let o = city.owner;
        if city.has_improvement(imp::SDI_DEFENSE) || o == slot {
            continue;
        }
        let theirs = 2 * strength(g, o);
        let candidate = 3 * mine < theirs || g.diplo(slot, o) & VENDETTA != 0 || (nukes(g, o) == 0 && mine < theirs);
        if !candidate || !nuked_us(g, o) || city.size <= 4 {
            continue;
        }
        let mut score = 0;
        for k in 1..=8 {
            let (nx, ny) = (city.x + DX[k], city.y + DY[k]);
            match g.unit_owner_at(nx, ny) {
                None => {}
                Some(n) if nuked_us(g, n) => score += 1,
                Some(n) if n == slot => score -= 2,
                Some(_) => score -= 99,
            }
        }
        score += city.size as i32 / 2;
        if score > best.0 && g.all_cities().any(|own| own.owner == slot && Map::civ_distance(own.pos(), city.pos()) <= 16) {
            best = (score, Some(cid));
        }
    }
    let (score, Some(target)) = best else { return Command::Skip };
    if score < 10 || nukes(g, slot) <= 1 {
        return Command::Skip;
    }
    let tp = g.city(target).pos();
    set_goto(g, c.id, tp);
    for k in 1..=8 {
        let (nx, ny) = (Map::wrap_x(tp.0 + DX[k]), tp.1 + DY[k]);
        if g.unit_owner_at(nx, ny).is_some() || !Map::in_bounds(nx, ny) {
            continue;
        }
        g.place_unit(c.id, nx, ny, 0);
        if g.city(target).owner == g.human_slot {
            // ds:0xd2e4[civ] = -2 (0x16a2b)
            g.player_mut(slot).last_contact = -2;
        }
        g.refresh_vision(slot);
        return match k ^ 4 {
            0 => Command::Goto,
            d if d <= 8 => dir(d),
            _ => Command::Invalid,
        };
    }
    Command::Skip
}

/// The Diplomat (0x16a39): a goto goes on; else toward the nearest city if foreign; in its own
/// city it is disbanded (and the routine runs on for the deleted unit: a defender is asked for
/// when no other unit is there, 0x16e25).
fn diplomat(g: &mut Game, c: &Ctx) -> Option<Command> {
    if g.unit(c.id).goto.is_some() {
        return Some(Command::Goto);
    }
    let city = c.city?;
    if g.city(city).owner != c.slot {
        let p = g.city(city).pos();
        set_goto(g, c.id, p);
        return Some(Command::Goto);
    }
    g.remove_unit(c.id);
    g.refresh_vision(c.slot);
    if g.city_at(c.x, c.y).is_some() && c.unit_dist > 0 {
        g.add_goal(c.slot, c.x, c.y, goal::DEFEND, 4);
    }
    Some(Command::Skip)
}

/// City sites (0x16a9d), for Settlers, attackers and defenders on a square of the site layer
/// (`Game::site_layer`) other than 0. Settlers first forget the city-site goals of their square.
/// The best of the square and its 8 neighbours counts (an attacker or defender next to a hut
/// takes it); on turn 0 of an Earth game the square is as good as can be. With a value of 8 or
/// more, at least 15 - the distance to the nearest city and no enemy adjacent, Settlers step to
/// the better neighbour or found the city; the others leave a persistent city-site goal (type 0,
/// priority 2) for Settlers. Then Settlers of a civ behind the human in research, near the
/// human's cities (distance 2 or more, difficulty above Chieftain) found a city on a square
/// whose original site value is 9 or more and at least 14 - that distance (0x16bca).
fn city_site(g: &mut Game, c: &Ctx) -> Option<Command> {
    if c.role == AiRole::Settle {
        g.remove_goals_near(c.slot, goal::SETTLE, c.x, c.y, 0);
    }
    let mut v = g.site_layer(c.x, c.y);
    if v != 0 && (c.role as u8) < 3 {
        let mut best_dir = 0;
        for k in 1..=8 {
            let (nx, ny) = (Map::wrap_x(c.x + DX[k]), c.y + DY[k]);
            let vn = g.site_layer(nx, ny);
            if vn > v {
                v = vn;
                best_dir = k;
            }
            if Map::in_bounds(nx, ny) && g.map.tile(nx, ny).has(tile_flags::HUT) && c.role != AiRole::Settle {
                return Some(dir(k));
            }
        }
        if g.turn == 0 && g.map.earth {
            v = 15;
            best_dir = 0;
        }
        if v >= 8 && 15 - c.city_dist <= v && !c.zoc {
            if c.role == AiRole::Settle {
                return Some(if best_dir != 0 { dir(best_dir) } else { Command::Build });
            }
            g.add_persistent_goal(c.slot, c.x, c.y, goal::SETTLE, 2);
        }
    }
    let h = g.human_slot;
    if c.role == AiRole::Settle && g.setup.difficulty as u8 != 0 && c.city_owner(g) == Some(h) && !c.zoc && c.city_dist > 1 && c.slot != h && g.player(c.slot).research_count < g.player(h).research_count {
        let v2 = g.site_value_pristine(c.x, c.y);
        if v2 >= 9 && 14 - c.city_dist <= v2 {
            return Some(Command::Build);
        }
    }
    None
}

/// A unit in a city (0x16c42). Defenders: alone, or better than the best other defender there,
/// they fortify; so do they while the city has no more than size / n + 1 of them (n = 3 with an
/// enemy adjacent, else 4; one less in the capital, one more on a continent without policy),
/// asking for another defender (type 2, priority 2) while it has fewer; an extra defender stays
/// fortified only if it would be the best defender and there are fewer Defend-role units before
/// it than that number; every 8th turn the others lose their fortification and go on. Other
/// units alone in a city stay (' ') and ask for a defender (type 2, priority 4).
fn in_city(g: &mut Game, c: &Ctx) -> Option<Command> {
    if c.role != AiRole::Defend {
        if c.unit_dist > 0 && c.role != AiRole::Settle {
            g.add_goal(c.slot, c.x, c.y, goal::DEFEND, 4);
            return Some(Command::Skip);
        }
        return None;
    }
    let others: Vec<UnitId> = stack(g, c.id).into_iter().filter(|o| *o != c.id).collect();
    if others.is_empty() {
        return Some(Command::Fortify);
    }
    // the best other defender, this unit counting as a Diplomat (defence 0, 0x16c99)
    let mut best: Option<(UnitId, i32)> = None;
    for o in &others {
        let fortified = g.unit(*o).order == UnitOrder::Fortified;
        if let Some(v) = defence_value(g, *o, fortified) {
            if v > best.map(|b| b.1).unwrap_or(0) {
                best = Some((*o, v));
            }
        }
    }
    let my_def = g.unit(c.id).info().def;
    if let Some((b, _)) = best {
        if g.unit(b).info().def < my_def {
            return Some(Command::Fortify);
        }
    }
    let mut n = if g.enemy_adjacent(c.slot, c.x, c.y) { 3 } else { 4 };
    if c.city.map(|ct| g.city(ct).has_improvement(imp::PALACE)).unwrap_or(false) {
        n -= 1;
    } else if c.policy == POLICY_NONE {
        n += 1;
    }
    let defenders = stack(g, c.id).iter().filter(|o| g.unit(**o).info().role == AiRole::Defend).count() as i32;
    let need = c.city_size(g) / n + 1;
    if need > defenders {
        g.add_goal(c.slot, c.x, c.y, goal::DEFEND, 2);
    }
    if need >= defenders {
        return Some(Command::Fortify);
    }
    let mine = defence_value(g, c.id, true).unwrap_or(0);
    let beaten = others.iter().any(|o| defence_value(g, *o, g.unit(*o).order == UnitOrder::Fortified).unwrap_or(0) > mine);
    if !beaten && need > defenders_before(g, c.id) {
        return Some(Command::Fortify);
    }
    if (c.id as i32 + g.turn as i32) & 7 == 0 {
        let u = g.unit_mut(c.id);
        if matches!(u.order, UnitOrder::Sentry | UnitOrder::Fortify | UnitOrder::Fortified) {
            u.order = UnitOrder::None;
        }
    }
    None
}

/// `ai_find_landing` 0x18cb7: of the unit's square and its ocean neighbours without units, the
/// one with the most empty land squares around it (squares of the human's territory not
/// counted while at peace with him); 0 for the square itself, -1 for none.
pub fn find_landing(g: &Game, slot: Slot, x: i32, y: i32) -> i32 {
    let h = g.human_slot;
    let peace_h = peace(g, slot, h);
    let mut best = (-1, -1);
    for d in 0..=8 {
        let (ex, ey) = (Map::wrap_x(x + DX[d]), y + DY[d]);
        if !Map::in_bounds(ex, ey) || !g.map.is_ocean(ex, ey) {
            continue;
        }
        if d != 0 && g.unit_owner_at(ex, ey).is_some() {
            continue;
        }
        let mut count = 0;
        for k in 1..=8 {
            let (nx, ny) = (Map::wrap_x(ex + DX[k]), ey + DY[k]);
            if !Map::in_bounds(nx, ny) || g.map.is_ocean(nx, ny) || g.unit_owner_at(nx, ny).is_some() {
                continue;
            }
            count += 1;
            if peace_h && g.site_layer(nx, ny) == h as i32 {
                count -= 1;
            }
        }
        if count > best.0 {
            best = (count, d as i32);
        }
    }
    best.1
}

/// Transports and sea attackers (0x16e02). The other units of the stack are counted: those not
/// fortified by role and as passengers when they are land units; on an offensive continent the
/// fortified ones beyond size / 5 (of the city there; CIV.EXE reads an uninitialised local)
/// lose their fortification and count. A transport with fewer passengers than it wants (1 at
/// sea, 3 in a port) or Settlers only aboard heads for the own coastal city where the civ's
/// defence is 16 or more, preferring offensive continents with much attack and ports without
/// another transport, and calls settlers, defenders and attackers there (goals of the roles
/// that are not the continent's policy, priority 4 offensive / 2), when the port is near enough.
/// A loaded transport unloads (' u', after a sea goal (3, 5) on hostile land) next to a fit
/// land square, or moves first to a better landing (`find_landing`) with moves to spare; on
/// its way it calls escorts to the middle of its voyage (3, 3). Without a goto it then sails
/// for the nearest enemy city (with attackers or without settlers aboard; only human cities
/// when the human ranks first), else for an own city 8 or more away on another, non-offensive
/// continent (twice as far when no passenger has its policy's role), else for random land
/// where the civ has no city. The rest of the routine follows (pillage, moves).
fn transport(g: &mut Game, c: &Ctx) -> Option<Command> {
    let slot = c.slot;
    let mut passengers = 0;
    let mut roles = 0u32;
    let mut released = 0;
    let size = g.city_at(c.x, c.y).map(|ct| g.city(ct).size as i32).unwrap_or(0);
    for o in stack(g, c.id).into_iter().filter(|o| *o != c.id) {
        let ou = g.unit(o).clone();
        if ou.order != UnitOrder::Fortified {
            roles |= 1 << (ou.info().role as u8);
            if ou.is_land() {
                passengers += 1;
            }
        } else if c.policy == POLICY_OFFENSIVE {
            let before = released;
            released += 1;
            if size / 5 < before {
                g.unit_mut(o).order = UnitOrder::None;
                passengers += 1;
            }
        }
    }
    let census = g.ai_memory.census.clone();
    let s = slot as usize;
    let want = if c.city_dist != 0 { 1 } else { 3 };
    if (want > passengers || roles == 1) && c.role == AiRole::Transport {
        let mut best = 999;
        let mut port_policy = None;
        let ports: Vec<CityId> = g.all_cities().filter(|ct| ct.owner == slot && g.map.is_coastal(ct.x, ct.y)).map(|ct| ct.id).collect();
        for p in ports {
            let pc = g.city(p).clone();
            let mut d = Map::civ_distance((c.x, c.y), pc.pos());
            let k = g.continent_exe(pc.x, pc.y);
            if census.defence[s][continent_index(k)] < 16 {
                continue;
            }
            if let Some(first) = g.units_at(pc.x, pc.y).first().copied() {
                let ships = stack(g, first).iter().filter(|o| g.unit(**o).info().role == AiRole::Transport).count() as i32;
                if ((d < 1) as i32) < ships {
                    d += 16;
                }
            }
            let pol = policy(g, slot, k);
            if pol != POLICY_OFFENSIVE {
                d += 16;
            } else {
                d -= census.attack[s][continent_index(k)].abs() / 4;
            }
            if d < best {
                best = d;
                set_goto(g, c.id, pc.pos());
                port_policy = Some(pol);
            }
        }
        if let Some(pol) = port_policy {
            let prio = if pol == POLICY_OFFENSIVE { 4 } else { 2 };
            if 3 * prio >= best {
                let to = g.unit(c.id).goto.unwrap_or((c.x, c.y));
                if pol != POLICY_NONE {
                    g.add_goal(slot, to.0, to.1, goal::SETTLE, prio);
                }
                if pol != POLICY_THREATENED {
                    g.add_goal(slot, to.0, to.1, goal::DEFEND, prio);
                }
                if pol != POLICY_HOSTILE {
                    g.add_goal(slot, to.0, to.1, goal::ATTACK, prio);
                }
            }
        }
        return None;
    }
    if c.role == AiRole::Transport {
        let mut broke = false;
        for k in 1..=8 {
            let (nx, ny) = (Map::wrap_x(c.x + DX[k]), c.y + DY[k]);
            if !Map::in_bounds(nx, ny) || g.map.is_ocean(nx, ny) {
                continue;
            }
            let owner = g.unit_owner_at(nx, ny);
            if owner.is_some() && owner != Some(slot) {
                continue;
            }
            if owner == Some(slot) && g.units_at(nx, ny).len().min(10) >= 2 {
                continue;
            }
            let h = g.human_slot;
            let t = g.map.tile(nx, ny);
            let developed = t.city.is_some() || t.flags & (tile_flags::IRRIGATION | tile_flags::MINE | tile_flags::ROAD | tile_flags::RAILROAD) != 0;
            if peace(g, slot, h) && g.last_occupant(nx, ny) == h && developed {
                continue;
            }
            let k_cont = g.continent_exe(nx, ny);
            let ki = continent_index(k_cont);
            let pol = policy(g, slot, k_cont);
            let mut ok = (census.defence[s][ki] < 16 && pol != POLICY_OFFENSIVE) || (1u32 << pol) & roles & 6 != 0;
            if let Some(gt) = g.unit(c.id).goto {
                if g.continent_exe(gt.0, gt.1) == k_cont {
                    ok = true;
                }
            }
            if !ok || census.sites[ki] < 5 || ny <= 1 || ny >= 48 {
                broke = true;
                break;
            }
            if g.unit(c.id).moves_left > 3 {
                let l = find_landing(g, slot, c.x, c.y);
                if l > 0 {
                    return Some(dir(l as usize));
                }
            }
            if pol == POLICY_HOSTILE {
                g.add_goal(slot, c.x, c.y, goal::SEA, 5);
            }
            g.unit_mut(c.id).moves_left = 0;
            g.clear_goto(c.id);
            return Some(Command::Unload);
        }
        if broke && g.turn & 3 == 0 {
            g.clear_goto(c.id);
        }
        let u = g.unit(c.id).clone();
        if u.moves_left <= 3 {
            if let Some((gx, gy)) = u.goto {
                let (mx, my) = ((c.x + gx) / 2, (c.y + gy) / 2);
                if g.map.is_ocean(mx, my) {
                    g.add_goal(slot, mx, my, goal::SEA, 3);
                }
            }
        }
    }
    // 0x173cd
    if c.zoc && c.role != AiRole::Transport {
        return None;
    }
    let trireme = c.kind == unit_ids::TRIREME;
    if g.unit(c.id).goto.is_none() && (roles & 2 != 0 || roles & 1 == 0) && !trireme {
        let h = g.human_slot;
        let top = g.power_rank(h) >= 7;
        let mut best = 999;
        let mut target = None;
        for ct in g.all_cities().filter(|ct| ct.id as u32 % 8 == g.turn % 8) {
            let o = ct.owner;
            if o == slot || keeps_peace(g, slot, o) || (top && o != h) {
                continue;
            }
            let d = Map::civ_distance((c.x, c.y), ct.pos());
            if d < best {
                best = d;
                target = Some(ct.pos());
            }
        }
        if let Some(t) = target {
            set_goto(g, c.id, t);
        }
    }
    if g.unit(c.id).goto.is_none() && !trireme {
        let here = (!g.map.is_ocean(c.x, c.y)).then_some(c.cont);
        let mut best = 999;
        let mut target = None;
        let cities: Vec<(CityId, (i32, i32))> = g.all_cities().filter(|ct| ct.owner == slot).map(|ct| (ct.id, ct.pos())).collect();
        for (_, p) in cities {
            let mut d = Map::civ_distance((c.x, c.y), p);
            if d < 8 {
                continue;
            }
            let k = g.continent_exe(p.0, p.1);
            if Some(k) == here {
                continue;
            }
            let pol = policy(g, slot, k);
            if pol == POLICY_OFFENSIVE {
                continue;
            }
            if (1u32 << pol) & roles & 7 == 0 {
                d *= 2;
            }
            if d < best {
                best = d;
                target = Some(p);
            }
        }
        if let Some(t) = target {
            set_goto(g, c.id, t);
        }
    }
    if g.unit(c.id).goto.is_none() {
        for r in 2..24 {
            let d = g.rng.below(9) as usize;
            let (tx, ty) = (Map::wrap_x(c.x + DX[d] * r), c.y + DY[d] * r);
            if ty <= 2 || ty >= 47 || g.map.is_ocean(tx, ty) {
                continue;
            }
            let k = g.continent_exe(tx, ty);
            if census.cities[s][continent_index(k)] != 0 {
                continue;
            }
            set_goto(g, c.id, (tx, ty));
            break;
        }
    }
    None
}

/// Pillage (0x176da): a slow unit (move rate below 2) within 3 of a human city on an
/// irrigated or mined square, while the human has no treaty with the civ.
fn pillage(g: &mut Game, c: &Ctx) -> Option<Command> {
    let h = g.human_slot;
    let t = g.map.tile(c.x, c.y);
    let rate = rules::UNITS[c.kind as usize].moves;
    if rate < 2 && c.city_dist < 4 && c.city_owner(g) == Some(h) && !peace(g, h, c.slot) && t.flags & (tile_flags::IRRIGATION | tile_flags::MINE) != 0 && Some(c.slot) != c.city_owner(g) {
        return Some(Command::Pillage);
    }
    None
}

/// `settler_irrigation_option` 0x7795: 0 on a city, irrigated or mined square; 2 where mining
/// gives a bonus of 2 or more (Hills); 1 where irrigation needs water and there is some; else 0.
fn irrigation_option(g: &Game, x: i32, y: i32) -> i32 {
    let t = g.map.tile(x, y);
    if t.city.is_some() || t.flags & (tile_flags::IRRIGATION | tile_flags::MINE) != 0 {
        return 0;
    }
    let k = t.terrain as usize;
    if EXE_MINING[k] <= -3 {
        return 2;
    }
    if EXE_IRRIGATION[k] == -2 && water_adjacent(g, x, y) {
        return 1;
    }
    0
}

/// `water_adjacent` 0x77fd: a River square, or an orthogonal neighbour that is no city and is
/// Ocean, River or irrigated.
fn water_adjacent(g: &Game, x: i32, y: i32) -> bool {
    if g.map.terrain(x, y) == Terrain::River {
        return true;
    }
    [1usize, 3, 5, 7].iter().any(|&k| {
        let (nx, ny) = (Map::wrap_x(x + DX[k]), y + DY[k]);
        let t = g.map.tile(nx, ny);
        Map::in_bounds(nx, ny) && t.city.is_none() && (matches!(t.terrain, Terrain::Ocean | Terrain::River) || t.has(tile_flags::IRRIGATION))
    })
}

/// Settlers at work (0x17748). At sea they explore. Away from other units they ask for an
/// escort (type 2, priority 2). On a square the civ improves (`AI_WORKS`, or anywhere on an
/// offensive continent) 1 or 2 from an own city (not bare Hills of a city below size 3) they
/// irrigate or mine (`irrigation_option`), build a road on irrigated or mined Desert, Plains or
/// Grassland, and with RailRoad a road or railroad where the square makes 2 (with a road 1)
/// shields; they clean pollution anywhere. With a goto they go on while the continent has room
/// ((expansion trait + 1) x sites > 16 x cities). Next to an own city they go to a neighbour
/// within 2 of it worth irrigating or mining with no Settlers on it; then roads: none on River
/// without Bridge Building or away from own cities; on Plains and Grassland 2 from an own city,
/// between two roads opposite each other where fewer than 4 neighbours have one, next to the
/// city on Desert to Grassland, and to extend a lone road on flat land. Else they explore
/// (0x17bb4): a random block of the 20 around theirs without own cities or Settlers, its square
/// 4 x the offset away when it is on the map, of shield-bearing terrain on their continent.
fn settlers_work(g: &mut Game, c: &Ctx) -> Option<Command> {
    let slot = c.slot;
    let id = c.id;
    if g.map.is_ocean(c.x, c.y) {
        return explore(g, c);
    }
    if c.unit_dist >= 2 {
        g.add_goal(slot, c.x, c.y, goal::DEFEND, 2);
    }
    let col = g.player(slot).knows(adv::MONARCHY) as usize;
    let t = g.map.tile(c.x, c.y);
    let k = c.terrain as usize;
    let own_city = c.city_owner(g) == Some(slot);
    let small = c.city_size(g) < 3;
    if (AI_WORKS[k][col] != 0 || c.policy == POLICY_OFFENSIVE) && (1..=2).contains(&c.city_dist) && own_city && !(small && c.terrain == Terrain::Hills && !t.special) {
        match irrigation_option(g, c.x, c.y) {
            1 => {
                g.clear_goto(id);
                return Some(Command::Irrigate);
            }
            2 => {
                g.clear_goto(id);
                return Some(Command::Mine);
            }
            _ => {
                let road = t.has(tile_flags::ROAD) || t.has(tile_flags::RAILROAD);
                if t.flags & (tile_flags::IRRIGATION | tile_flags::MINE) != 0 && !road && (c.terrain as u8) <= Terrain::Grassland as u8 {
                    g.clear_goto(id);
                    return Some(Command::Road);
                }
                if !(road && t.has(tile_flags::RAILROAD)) && g.player(slot).knows(adv::RAILROAD) {
                    let shields = g.tile_yield(slot, c.x, c.y, None).1;
                    if (if road { 1 } else { 2 }) <= shields {
                        g.clear_goto(id);
                        return Some(Command::Road);
                    }
                }
            }
        }
    }
    if t.has(tile_flags::POLLUTION) {
        return Some(Command::Pollution);
    }
    if g.unit(id).goto.is_some() {
        let census = &g.ai_memory.census;
        let ki = continent_index(c.cont);
        let trait_ = g.player(slot).civ().expansionist as i32;
        if (trait_ + 1) * census.sites[ki] > 16 * census.cities[slot as usize][ki] {
            return Some(Command::Goto);
        }
    }
    if c.city_dist <= 1 && own_city {
        let city = g.city(c.city.unwrap()).pos();
        for i in 1..=8 {
            let d = ((i + g.turn as usize) & 7) + 1;
            let (nx, ny) = (Map::wrap_x(c.x + DX[d]), c.y + DY[d]);
            if !Map::in_bounds(nx, ny) {
                continue;
            }
            if let Some(first) = g.units_at(nx, ny).first().copied() {
                if g.unit(first).owner != slot || stack(g, first).iter().any(|o| g.unit(*o).kind == unit_ids::SETTLERS) {
                    continue;
                }
            }
            let nt = g.map.tile(nx, ny);
            if AI_WORKS[nt.terrain as usize][col] == 0 || irrigation_option(g, nx, ny) == 0 {
                continue;
            }
            if small && nt.terrain == Terrain::Hills && !nt.special {
                continue;
            }
            if dist_metric(Map::delta_x(city.0, nx), ny - city.1) <= 2 {
                set_goto(g, id, (nx, ny));
                return Some(Command::Goto);
            }
        }
    }
    if t.has(tile_flags::ROAD) || t.has(tile_flags::RAILROAD) {
        return explore(g, c);
    }
    if c.terrain == Terrain::River && !g.player(slot).knows(adv::BRIDGE_BUILDING) {
        return explore(g, c);
    }
    if !own_city {
        return explore(g, c);
    }
    if c.city_dist <= 2 && matches!(c.terrain, Terrain::Plains | Terrain::Grassland) {
        return Some(Command::Road);
    }
    let road_at = |g: &Game, x: i32, y: i32| {
        let t = g.map.tile(x, y);
        t.has(tile_flags::ROAD) || t.has(tile_flags::RAILROAD) || t.city.is_some()
    };
    let (mut mask, mut count, mut far) = (0u32, 0, false);
    for k in 1..=8 {
        if road_at(g, c.x + DX[k], c.y + DY[k]) {
            mask |= 1 << (k - 1);
            count += 1;
            if road_at(g, c.x + 2 * DX[k], c.y + 2 * DY[k]) {
                far = true;
            }
        }
    }
    mask &= mask >> 4;
    if count < 4 {
        if mask != 0 {
            return Some(Command::Road);
        }
        if c.city_dist == 1 && (c.terrain as u8) <= Terrain::Grassland as u8 {
            return Some(Command::Road);
        }
    }
    if count == 1 && far && c.terrain.info().move_cost == 1 {
        return Some(Command::Road);
    }
    explore(g, c)
}

fn explore(g: &mut Game, c: &Ctx) -> Option<Command> {
    if g.unit(c.id).goto.is_some() {
        return Some(Command::Goto);
    }
    let r = g.rng.below(20) as usize + 1;
    let (bx, by) = (c.x / 4 + DX[r], c.y / 4 + DY[r]);
    let occupied = !(0..20).contains(&bx) || !(0..13).contains(&by) || g.ai_memory.coarse[bx as usize][by as usize] & 3 != 0;
    if !occupied {
        let to = (c.x + 4 * DX[r], c.y + 4 * DY[r]);
        if exe_in_map(to.0, to.1) && EXE_SHIELDS[g.map.terrain(to.0, to.1) as usize] != 0 && g.continent_exe(to.0, to.1) == c.cont {
            set_goto(g, c.id, to);
            return Some(Command::Goto);
        }
        g.clear_goto(c.id);
    }
    None
}

/// Bribe a unit without a Diplomat, as the move scoring does (overlay 22 +0x639 from 0x183f5 /
/// 0x184ef): not when more units share its square; the computer pays when half its treasury
/// covers the price (`Game::bribe_cost`).
fn try_bribe(g: &mut Game, slot: Slot, target: UnitId) {
    let t = g.unit(target).clone();
    if g.units_at(t.x, t.y).len() > 1 || g.player(slot).gold / 2 < g.bribe_cost(target) {
        return;
    }
    g.bribe_unit(slot, target);
}

/// The rest (0x17cb6..0x18766): a unit with a goto follows it (a sea attacker only while no
/// foreign unit is next to it); a lone defender near an own city, or one with an enemy next to
/// it and own units around, fortifies; Settlers on a full continent (more cities than sites / 8)
/// join the nearest own city below size 10. Then the 8 directions are scored and the best taken
/// (0 = none, also for an attack with less than a full move left); see `score_direction`.
fn military(g: &mut Game, c: &mut Ctx) -> Command {
    let slot = c.slot;
    if g.unit(c.id).goto.is_some() {
        if c.role != AiRole::SeaAttack || !g.foreign_unit_adjacent(slot, c.x, c.y) {
            return Command::Goto;
        }
    }
    let here_ocean = c.terrain == Terrain::Ocean;
    if c.role == AiRole::Defend && !here_ocean {
        if c.unit_dist > 1 && c.city_dist <= 3 && c.city_owner(g) == Some(slot) {
            return Command::Fortify;
        }
        if c.zoc && c.unit_dist != 0 {
            return Command::Fortify;
        }
    }
    if c.role == AiRole::Settle {
        let census = &g.ai_memory.census;
        let ki = continent_index(c.cont);
        if census.cities[slot as usize][ki] > census.sites[ki].abs() / 8 {
            if c.city_dist == 0 && c.city_size(g) < 10 {
                return Command::Build;
            }
            if c.city_owner(g) == Some(slot) && c.city_size(g) < 10 {
                let p = g.city(c.city.unwrap()).pos();
                set_goto(g, c.id, p);
                return Command::Goto;
            }
        }
    }
    let mut best = -999;
    let mut best_dir = 0;
    let mut best_enemy = false;
    let peaceful = if g.enemy_adjacent(slot, c.x, c.y) {
        g.set_heading(c.id, 0xff);
        false
    } else {
        true
    };
    if c.kind == unit_ids::MILITIA && c.policy == POLICY_HOSTILE {
        c.role = AiRole::Attack;
    }
    for k in 1..=8 {
        if let Some((v, enemy)) = score_direction(g, c, k, peaceful) {
            if v > best {
                best = v;
                best_dir = k;
                best_enemy = enemy;
            }
        }
    }
    if best_enemy && g.unit(c.id).moves_left < 3 {
        best_dir = 0;
    }
    g.set_heading(c.id, if best_dir == 0 { 0xff } else { best_dir as u8 });
    dir(best_dir)
}

/// The value of moving in direction `k` (0x17e1a..0x18727), `None` when the unit does not go
/// there, with whether an enemy is there.
///
/// Excluded: off the map; water for land units; for a land unit off a ship, an empty square
/// when it would break the zone of control (both squares next to foreign units, not into a
/// city, for units below the Diplomat), foreign units from a ship; for defenders a land square
/// unless an enemy is there and the defender is at sea (and not as a Submarine); own stacks of
/// two or more.
///
/// Base: Settlers 0 (and no square next to an enemy from a ship); a unit nobody else can see,
/// on land: random(3) - 2 x the terrain's move cost (attackers) or defence (others); else
/// random(5) plus, onto an own stack, 4 x its defence / (units + 1) (attackers) or 2 x its
/// attack / (defence + 1) (defenders), elsewhere 4 x the terrain defence; aircraft random(3).
/// Then: -2 x the turn from the heading squared; an empty enemy city 999 (empty square: a hut
/// +20); onto own units minus the unit's defence. An enemy: Settlers never; a treaty partner is
/// left alone (but see the bribes); otherwise the odds against the best defender (`combat_odds`)
/// x (the defenders' total cost + 1) / the unit's cost, x3 for a city, x3 for an attacker on a
/// hostile continent, x2 for an attacker in a stack of more attack than twice its own; 6 or
/// more adds 4 x the odds, less costs 999 (an attacker on a hostile continent then may bribe a
/// lone fortified or veteran unit on defensive terrain outside a city, 1 in 4). Aircraft attack
/// air units only as Fighters or in cities. With no enemy next to the unit: +8 when the block
/// four squares on has no own presence (land, on the map), and for the 8 squares around that
/// point +2 unseen land (sea for ships), -2 occupied, + the food of the terrain for Settlers.
/// An attack with less than a full move left is weighed by the stack's defence / attack.
fn score_direction(g: &mut Game, c: &Ctx, k: usize, peaceful: bool) -> Option<(i32, bool)> {
    let slot = c.slot;
    let u = g.unit(c.id).clone();
    let (nx, ny) = (Map::wrap_x(c.x + DX[k]), c.y + DY[k]);
    if !Map::in_bounds(nx, ny) {
        return None;
    }
    let owner = g.last_occupant(nx, ny);
    let t = g.map.terrain(nx, ny);
    let land_unit = u.is_land();
    let here_ocean = c.terrain == Terrain::Ocean;
    if t == Terrain::Ocean && land_unit {
        return None;
    }
    let mut s = g.units_at(nx, ny).first().copied();
    if let Some(first) = s {
        if g.unit(first).kind == unit_ids::DIPLOMAT {
            if let Some(o) = stack(g, first).into_iter().find(|o| g.unit(*o).kind != unit_ids::DIPLOMAT) {
                s = Some(o);
            }
        }
    }
    if c.zoc && land_unit && s.is_none() && g.zoc_blocked(slot, nx, ny) && c.kind < unit_ids::DIPLOMAT && !here_ocean {
        return None;
    }
    if here_ocean && land_unit && s.is_some() && owner != slot {
        return None;
    }
    if c.role == AiRole::Defend && t != Terrain::Ocean && (s.is_none() || owner == slot || c.kind == unit_ids::SUBMARINE || !here_ocean) {
        return None;
    }
    if let Some(first) = s {
        if owner == slot && stack_count(g, first) >= 2 {
            return None;
        }
    }
    let mut v = 0;
    if c.role == AiRole::Settle {
        if here_ocean && g.enemy_adjacent(slot, nx, ny) {
            return None;
        }
    } else {
        let seen = (1..8u8).any(|o| o != slot && g.player(o).alive && g.visible_to(o, c.x, c.y));
        if !seen && !here_ocean {
            let r = g.rng.below(3) as i32;
            let f = if c.role == AiRole::Attack { t.info().move_cost } else { t.info().defense } as i32;
            v = r - 2 * f;
        } else {
            v = g.rng.below(5) as i32;
            match s {
                Some(first) if owner == slot => match c.role {
                    AiRole::Attack => v += 4 * stack_sum(g, first, |t| t.def) / (stack_count(g, first) + 1),
                    AiRole::Defend => v += 2 * stack_sum(g, first, |t| t.att) / (stack_sum(g, first, |t| t.def) + 1),
                    _ => {}
                },
                _ => v += 4 * t.info().defense as i32,
            }
        }
        if u.is_air() {
            v = g.rng.below(3) as i32;
        }
    }
    let heading = g.heading(c.id);
    if heading != 0xff {
        let a = turn_angle(heading as i32, k as i32);
        v -= 2 * a * a;
    }
    let mut enemy = false;
    let city_there = g.city_at(nx, ny).is_some();
    match s {
        None => {
            if city_there && owner != slot {
                v = 999;
            }
            if g.map.tile(nx, ny).has(tile_flags::HUT) {
                v += 20;
            }
        }
        Some(_) if owner == slot => v -= u.info().def as i32,
        Some(first) => {
            enemy = true;
            if c.role == AiRole::Settle {
                return None;
            }
            let h = g.human_slot;
            if peace(g, slot, owner) {
                let fortified = g.unit(first).order == UnitOrder::Fortified;
                let war_h = g.diplo(slot, h) & 3 == 1;
                let their_war_h = g.diplo(owner, h) & 3 == 1;
                if c.role == AiRole::Attack && c.policy == POLICY_HOSTILE && land_unit && fortified && war_h && !their_war_h {
                    let census = &g.ai_memory.census;
                    if census.cities[h as usize][continent_index(c.cont)] != 0 && stack_count(g, first) < 2 && (g.rng.below(8) == 0 || c.city_owner(g) == Some(h)) && !city_there {
                        try_bribe(g, slot, first);
                    }
                }
                return None;
            }
            let def = best_defender(g, first).unwrap_or(first);
            if g.unit(def).is_air() && c.kind != unit_ids::FIGHTER && !city_there {
                return None;
            }
            let mut odds = g.combat_odds(c.id, def);
            odds = odds * (stack_sum(g, def, |t| t.cost) + 1) / (u.info().cost as i32).max(1);
            if city_there {
                odds *= 3;
            }
            if c.role == AiRole::Attack && c.policy == POLICY_HOSTILE {
                odds *= 3;
            }
            if c.role == AiRole::Attack && 2 * (u.info().att as i32) < stack_sum(g, c.id, |t| t.att) {
                odds *= 2;
            }
            if odds >= 6 {
                v += 4 * odds;
            } else {
                v -= 999;
                let du = g.unit(def).clone();
                let tough = du.order == UnitOrder::Fortified || du.veteran;
                if c.role == AiRole::Attack && c.policy == POLICY_HOSTILE && land_unit && tough && t.info().defense >= 4 && stack_count(g, def) < 2 && g.rng.below(4) == 0 && !city_there {
                    try_bribe(g, slot, def);
                }
            }
        }
    }
    if peaceful {
        let (fx, fy) = (c.x + 4 * DX[k], c.y + 4 * DY[k]);
        if exe_in_map(fx, fy) && g.ai_memory.coarse[(fx / 4) as usize][(fy / 4).min(12) as usize] == 0 && !g.map.is_ocean(fx, fy) {
            v += 8;
        }
        for e in 1..=8 {
            let (ex, ey) = (Map::wrap_x(fx + DX[e]), fy + DY[e]);
            if !exe_in_map(ex, ey) {
                continue;
            }
            if !g.explored(slot, ex, ey) && (g.map.is_land(ex, ey) || u.is_sea()) {
                v += 2;
            }
            if g.unit_owner_at(ex, ey).is_some() {
                v -= 2;
            }
            if c.role == AiRole::Settle {
                v += g.map.terrain(ex, ey).info().food as i32;
            }
        }
    }
    if enemy && u.moves_left < 3 {
        v = v * stack_sum(g, c.id, |t| t.def) / stack_sum(g, c.id, |t| t.att).clamp(1, 99);
    }
    Some((v, enemy))
}

// ---- carrying the commands out ----------------------------------------------------------

/// `dir_to_nearest_base` 0xa0c9: the direction that brings an aircraft nearest (2 x max +
/// min, wrapped) to the nearest own city or, if nearer, own Carrier; 0 for none.
fn dir_to_base(g: &Game, id: UnitId) -> usize {
    let u = g.unit(id);
    let cheb = |a: (i32, i32), b: (i32, i32)| Map::delta_x(a.0, b.0).abs().max((a.1 - b.1).abs());
    let mut base: Option<((i32, i32), i32)> = None;
    for p in g.all_cities().filter(|c| c.owner == u.owner).map(|c| c.pos()).chain(g.all_units().filter(|o| o.owner == u.owner && o.kind == unit_ids::CARRIER).map(|o| o.pos())) {
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
        if g.unit_owner_at(nx, ny).map(|o| o != u.owner).unwrap_or(false) {
            continue;
        }
        if g.city_at(nx, ny).is_some() && g.last_occupant(nx, ny) != u.owner {
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

/// A move key (0x5946 for the computer): a step back against the heading (heading ^ 4) by a
/// unit with at most one other unit beside it ends its turn and its goto (0x63cc); a land unit
/// (or an aircraft with fuel) does not join an own stack of four or more outside a city
/// (0x6437, the move is simply not made). Returns whether a move was tried.
fn do_step(g: &mut Game, id: UnitId, d: Dir8, sink: &mut dyn FnMut(TurnStep)) -> bool {
    let k = d as i32 + 1;
    let heading = g.heading(id);
    if heading != 0xff && (heading as i32 ^ 4) == k && stack_count(g, id) <= 2 {
        g.set_heading(id, 0xff);
        g.clear_goto(id);
        g.unit_mut(id).moves_left = 0;
        return false;
    }
    g.set_heading(id, k as u8);
    let u = g.unit(id).clone();
    let (dx, dy) = d.delta();
    let (nx, ny) = (Map::wrap_x(u.x + dx), u.y + dy);
    if let Some(first) = g.units_at(nx, ny).first().copied() {
        let counts = u.is_land() || (u.is_air() && fuel_left(&u) != 0);
        if g.unit(first).owner == u.owner && g.city_at(nx, ny).is_none() && counts && stack_count(g, first) >= 4 {
            return false;
        }
    }
    let slot = u.owner;
    match step(g, id, d, sink) {
        Ok(MoveOutcome::EnteredForeignCity(c)) => {
            if u.kind == unit_ids::DIPLOMAT {
                g.ai_diplomat_action(id, c);
            } else if u.kind == unit_ids::CARAVAN {
                // a foreign city: a trade route at once (0x5d15)
                let _ = g.caravan_action(id, crate::game::turn::CaravanAction::TradeRoute(c));
            }
        }
        Ok(MoveOutcome::Moved) if u.kind == unit_ids::CARAVAN => caravan_arrived(g, id, slot),
        _ => {}
    }
    true
}

/// A computer's Caravan entering one of its own cities (0x5c37): a wonder city within 10 of
/// its home on the same continent takes it in without a menu; elsewhere CIV.EXE shows the
/// human's "Will you?" menu (ds:0x1ee4). Unverified what the computer answers there: civrs
/// helps build the wonder when the city builds one, and keeps moving otherwise.
fn caravan_arrived(g: &mut Game, id: UnitId, slot: Slot) {
    let Some(u) = g.unit_opt(id).cloned() else { return };
    let Some(c) = g.city_at(u.x, u.y) else { return };
    let city = g.city(c).clone();
    if city.owner != slot || !matches!(city.production, crate::game::Production::Wonder(_)) {
        return;
    }
    if let Some(h) = u.home.and_then(|h| g.city_opt(h)) {
        if Map::civ_distance(h.pos(), city.pos()) < 10 && g.continent_exe(h.x, h.y) == g.continent_exe(city.x, city.y) {
            return;
        }
    }
    let cm = g.city_mut(c);
    cm.shields += crate::game::turn::CARAVAN_WONDER_SHIELDS;
    cm.caravan_shields += crate::game::turn::CARAVAN_WONDER_SHIELDS;
    g.remove_unit(id);
}

/// Carry out one command (the handlers of 0x4bbe). Returns true for 'u' that woke passengers.
fn execute(g: &mut Game, id: UnitId, cmd: Command, sink: &mut dyn FnMut(TurnStep)) -> bool {
    let Some(u) = g.unit_opt(id).cloned() else { return false };
    match cmd {
        Command::Goto | Command::Invalid => {}
        Command::Step(d) => {
            do_step(g, id, d, sink);
        }
        Command::Skip => g.unit_mut(id).moves_left = 0,
        Command::Build => {
            // Settlers on land in rows 2..47 (0x5753): join a city below size 10, else found one
            if u.kind == unit_ids::SETTLERS && g.map.is_land(u.x, u.y) && (2..48).contains(&u.y) {
                let joins = g.city_at(u.x, u.y).map(|c| g.city(c).size < 10);
                if joins != Some(false) {
                    let _ = g.found_city(id);
                    g.refresh_vision(u.owner);
                }
            }
        }
        Command::Fortify => {
            // land units only (0x5137); a computer's unit in a city makes it its home (0x51b7)
            if u.is_land() && u.kind != unit_ids::SETTLERS {
                let um = g.unit_mut(id);
                um.order = UnitOrder::Fortify;
                um.goto = None;
                um.moves_left = 0;
                if let Some(c) = g.city_at(u.x, u.y) {
                    g.unit_mut(id).home = Some(c);
                }
            }
        }
        Command::Home => {
            let (near, d) = g.nearest_city_exe(u.x, u.y);
            if d == 0 {
                g.unit_mut(id).home = near;
            } else if u.is_air() {
                let k = dir_to_base(g, id);
                if k != 0 {
                    do_step(g, id, Dir8::ALL[k - 1], sink);
                } else {
                    // no base: the computer's aircraft is deleted (0x502d -> 0x4c5a)
                    g.remove_unit(id);
                    g.refresh_vision(u.owner);
                }
            }
        }
        Command::Irrigate => {
            if let Err(e) = g.give_order(id, UnitOrder::Irrigate) {
                if trace() {
                    eprintln!("TRACE ORDERFAIL Irrigate {e:?} at {:?} {:?}", u.pos(), g.map.tile(u.x, u.y));
                }
            }
        }
        Command::Mine => {
            if let Err(e) = g.give_order(id, UnitOrder::Mine) {
                if trace() {
                    eprintln!("TRACE ORDERFAIL Mine {e:?} at {:?} {:?}", u.pos(), g.map.tile(u.x, u.y));
                }
            }
        }
        Command::Road => {
            if let Err(e) = g.give_order(id, UnitOrder::Road) {
                if trace() {
                    eprintln!("TRACE ORDERFAIL Road {e:?} at {:?} {:?}", u.pos(), g.map.tile(u.x, u.y));
                }
            }
        }
        Command::Pollution => {
            if let Err(e) = g.give_order(id, UnitOrder::CleanPollution) {
                if trace() {
                    eprintln!("TRACE ORDERFAIL CleanPollution {e:?} at {:?} {:?}", u.pos(), g.map.tile(u.x, u.y));
                }
            }
        }
        Command::Pillage => {
            if let Err(e) = g.give_order(id, UnitOrder::Pillage) {
                if trace() {
                    eprintln!("TRACE ORDERFAIL Pillage {e:?} at {:?} {:?}", u.pos(), g.map.tile(u.x, u.y));
                }
            }
        }
        Command::Unload => {
            // 0x4dfc: a ship with room for units wakes its stack and its passengers move next
            if u.info().capacity > 0 {
                let woken: Vec<UnitId> = g.cargo_of(id);
                for c in &woken {
                    g.unit_mut(*c).order = UnitOrder::None;
                }
                return !woken.is_empty();
            }
        }
    }
    false
}

/// The computer's units move (0x403e for a computer civ): in unit order, skipping sentried
/// and fortified units and those without moves; each is asked for commands while it has moves,
/// five times at most, the fifth answer being ' ' (0x4464, dropping goto and heading). Any
/// command but 0 drops the goto first; with a goto the key comes from `goto_direction` (an
/// unreachable goto is dropped there), without one the heading is forgotten before the key is
/// carried out. After 'u' the woken passengers move at once and the units are gone through
/// again.
pub fn move_units(g: &mut Game, slot: Slot, sink: &mut dyn FnMut(TurnStep)) {
    g.ai_memory.moving = Some(slot);
    let mut passes = 0;
    loop {
        passes += 1;
        let mut again = false;
        let mut queue: std::collections::VecDeque<UnitId> = g.units_of(slot).into();
        while let Some(id) = queue.pop_front() {
            if g.unit_opt(id).map(|u| u.owner != slot).unwrap_or(true) {
                continue;
            }
            if let Some(cargo) = run_unit(g, slot, id, sink) {
                // the passengers next (0x4e64), then everything again (0x4e5a)
                for c in cargo.into_iter().rev() {
                    queue.push_front(c);
                }
                again = true;
            }
        }
        if !again || passes >= 3 {
            break;
        }
    }
    g.ai_memory.moving = None;
    g.refresh_vision(slot);
}

fn trace() -> bool {
    static T: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *T.get_or_init(|| std::env::var("CIVRS_AI_TRACE").is_ok())
}

/// One unit's turn in `move_units`; returns the passengers a 'u' woke.
fn run_unit(g: &mut Game, slot: Slot, id: UnitId, sink: &mut dyn FnMut(TurnStep)) -> Option<Vec<UnitId>> {
    let u = g.unit(id);
    // sentried (0x01) and fortified (0x08) units are passed over (0x41fb), as are those that
    // fortify (0x04 becomes 0x08 with no moves, 0x4239) and, in civrs, those at work
    if matches!(u.order, UnitOrder::Sentry | UnitOrder::Fortified | UnitOrder::Fortify) || u.moves_left == 0 || u.order_turns > 0 {
        return None;
    }
    for call in 1..=5 {
        let Some(u) = g.unit_opt(id) else { break };
        if u.moves_left == 0 || u.owner != slot {
            break;
        }
        let mut cmd = unit_command(g, slot, id);
        if trace() {
            if let Some(u) = g.unit_opt(id) {
                eprintln!("TRACE t{} s{} #{} {} at {:?} cmd {:?} goto {:?} order {:?} moves {} site {}", g.turn, slot, id, u.info().name, u.pos(), cmd, u.goto, u.order, u.moves_left, g.site_layer(u.x, u.y));
            }
        }
        if g.unit_opt(id).is_none() {
            break;
        }
        if cmd != Command::Goto {
            g.clear_goto(id);
        }
        if call == 5 {
            cmd = Command::Skip;
            g.set_heading(id, 0xff);
            g.clear_goto(id);
        }
        if g.unit(id).goto.is_some() {
            let d = g.goto_direction(id);
            cmd = if (1..=8).contains(&d) { dir(d as usize) } else { Command::Goto };
        } else {
            g.set_heading(id, 0xff);
        }
        if execute(g, id, cmd, sink) {
            return Some(g.cargo_of(id));
        }
    }
    None
}

/// Give one computer unit its turn (tests and tools): `run_unit` with the civilization as
/// the moving one.
pub fn act(g: &mut Game, id: UnitId, sink: &mut dyn FnMut(TurnStep)) {
    let Some(slot) = g.unit_opt(id).map(|u| u.owner) else { return };
    g.ai_memory.moving = Some(slot);
    run_unit(g, slot, id, sink);
    g.ai_memory.moving = None;
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

    /// Grassland from (5,5) to (39,29) in an ocean, no units, civs 1..4 in the game.
    fn arena() -> Game {
        let mut g = Game::new(GameSetup { seed: 21, num_civs: 4, ..Default::default() }, None);
        for s in 1..5 {
            g.players[s].in_game = true;
            g.players[s].alive = true;
        }
        for t in g.map.tiles.iter_mut() {
            t.terrain = Terrain::Ocean;
            t.flags = 0;
            t.city = None;
            t.continent = 0;
            t.special = false;
        }
        for y in 5..30 {
            for x in 5..40 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Grassland;
                t.continent = 1;
            }
        }
        let ids: Vec<_> = g.all_units().map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        g.cities.clear();
        g.barbarian_memory = Default::default();
        // the generator's stored site layer describes the old world: compute from the arena
        g.map.site.clear();
        g.ai_memory = Default::default();
        g
    }

    fn ai(g: &Game) -> Slot {
        (2..5).find(|s| *s != g.human_slot).unwrap()
    }

    #[test]
    fn settlers_found_a_city_away_from_others_and_step_to_a_better_square() {
        let mut g = arena();
        let s = ai(&g);
        let id = g.add_unit(s, unit_ids::SETTLERS, 20, 20);
        assert_eq!(unit_command(&mut g, s, id), Command::Build);
        // three squares from a city the square is claimed (value 8) and the unclaimed one to
        // the east (four away) is better
        g.add_city(s, "Old".into(), 17, 20);
        let v = g.site_layer(20, 20);
        assert_eq!(v, 8);
        let best = (1..=8).map(|k| (g.site_layer(20 + DX[k], 20 + DY[k]), k)).fold((v, 0), |b, n| if n.0 > b.0 { n } else { b });
        assert!(best.0 > 8 && best.1 != 0);
        assert_eq!(unit_command(&mut g, s, id), dir(best.1));
        // next to the city the square belongs to it: no city site at all
        g.unit_mut(id).x = 18;
        assert_eq!(g.site_layer(18, 20), s as i32);
        assert_ne!(unit_command(&mut g, s, id), Command::Build);
    }

    #[test]
    fn defenders_fortify_until_the_city_has_size_over_n_plus_one() {
        let mut g = arena();
        let s = ai(&g);
        let c = g.add_city(s, "Fort".into(), 20, 20);
        g.city_mut(c).size = 8;
        let a = g.add_unit(s, unit_ids::PHALANX, 20, 20);
        // alone: fortify
        assert_eq!(unit_command(&mut g, s, a), Command::Fortify);
        // two of three wanted (8 / 3 + 1 in the capital): fortify and call for a third
        let b = g.add_unit(s, unit_ids::PHALANX, 20, 20);
        g.reset_goals(s);
        assert_eq!(unit_command(&mut g, s, b), Command::Fortify);
        assert!(g.ai_memory.goals[s as usize].iter().any(|o| o.kind == goal::DEFEND && o.pos() == (20, 20) && o.priority == 2));
        // a better defender than the best other one fortifies at once
        let m = g.add_unit(s, unit_ids::MUSKETEERS, 20, 20);
        assert_eq!(unit_command(&mut g, s, m), Command::Fortify);
    }

    #[test]
    fn attackers_take_an_empty_enemy_city_and_avoid_bad_odds() {
        let mut g = arena();
        let s = ai(&g);
        let e = (2..5).find(|o| *o != s && *o != g.human_slot).unwrap();
        g.set_relation(s, e, crate::game::Relation::War);
        g.add_city(e, "Prize".into(), 21, 20);
        let id = g.add_unit(s, unit_ids::LEGION, 20, 20);
        assert_eq!(unit_command(&mut g, s, id), Command::Step(Dir8::E));
        // a fortified Musketeers on hills outside a city: odds 3*8*8 / (3*4*12/2 + 1) = 2, x
        // (cost 3 + 1) / cost 2 = 4, below 6
        let w = g.add_unit(s, unit_ids::LEGION, 30, 20);
        g.map.get_mut(31, 20).unwrap().terrain = Terrain::Hills;
        let m = g.add_unit(e, unit_ids::MUSKETEERS, 31, 20);
        g.unit_mut(m).order = UnitOrder::Fortified;
        assert_eq!(g.combat_odds(w, m), 2);
        assert_ne!(unit_command(&mut g, s, w), Command::Step(Dir8::E));
        // in a city the odds count three times: 12 > 6, it attacks
        g.add_city(e, "Hill".into(), 31, 20);
        assert_eq!(unit_command(&mut g, s, w), Command::Step(Dir8::E));
    }

    #[test]
    fn the_tasking_sends_a_free_defender_to_an_empty_city() {
        let mut g = arena();
        let s = ai(&g);
        g.add_city(s, "Home".into(), 10, 10);
        let empty = g.add_city(s, "Empty".into(), 20, 10);
        g.add_unit(s, unit_ids::PHALANX, 10, 10);
        let free = g.add_unit(s, unit_ids::PHALANX, 12, 12);
        // a Legion has no goal of its role here
        let legion = g.add_unit(s, unit_ids::LEGION, 12, 13);
        g.reset_goals(s);
        civ_turn(&mut g, s);
        assert_eq!(g.unit(free).goto, Some(g.city(empty).pos()));
        assert_eq!(g.unit(legion).goto, None);
    }

    #[test]
    fn a_transport_lands_where_most_empty_land_is() {
        let mut g = arena();
        for y in 0..40 {
            for x in 0..50 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Ocean;
                t.continent = 0;
            }
        }
        for y in 8..13 {
            g.map.get_mut(14, y).unwrap().terrain = Terrain::Grassland;
        }
        g.map.get_mut(13, 10).unwrap().terrain = Terrain::Grassland;
        let s = ai(&g);
        // the ship's square sees one land square, the one to the north-east four
        assert_eq!(find_landing(&g, s, 12, 10), 2);
        // a unit there takes it out of the choice; south-east has as many
        g.add_unit(s, unit_ids::TRIREME, 13, 9);
        assert_eq!(find_landing(&g, s, 12, 10), 4);
    }

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
