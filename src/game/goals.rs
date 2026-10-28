//! The computer players' goal lists (CIV.EXE segment 0x1579, 0x1876d..0x18b97) and the other
//! per-civilization bookkeeping of the AI turn that lives outside the unit records.
//!
//! Every civilization has 32 transient goals (type ds:0xe2c2, x ds:0xe598, y ds:0xe798,
//! priority ds:0xb2be, 32 bytes per civ) and 16 persistent ones (type ds:0xd76e, x ds:0xdc6c,
//! y ds:0xde22, priority ds:0xe518, 16 bytes per civ). A goal is a square, a type that matches
//! a unit role (0 settle, 1 attack, 2 defend, 3 sea attack, 4 air) and a priority; type 0xff
//! (-1) marks a free slot. Both lists are kept sorted by priority, highest first.
//!
//! The transient list is cleared at the start of every civilization's turn, after the AI turn
//! (`ai_civ_turn` 0x1579a) has used it, and refilled with the persistent goals of type <= 2
//! (0x1876d, called from the turn loop at 0x2541); the goals added while the civilization's
//! units move are used by its next AI turn. Only the persistent goals are in CIV.EXE's saved
//! game (SVE offset 0x6660); civrs stores them, with the continent policies, in the version-9
//! save trailer.

use super::map::{Map, Slot, UnitId};
use super::Game;
use std::collections::{HashMap, HashSet};

/// Transient goals per civilization.
pub const GOALS: usize = 32;
/// Persistent goals per civilization.
pub const PERSISTENT_GOALS: usize = 16;

/// Goal types (= the unit roles they call).
pub mod goal {
    pub const SETTLE: i8 = 0;
    pub const ATTACK: i8 = 1;
    pub const DEFEND: i8 = 2;
    pub const SEA: i8 = 3;
    pub const AIR: i8 = 4;
    /// A free slot (0xff).
    pub const FREE: i8 = -1;
}

/// One goal: CIV.EXE keeps every field as a signed byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Goal {
    pub x: i8,
    pub y: i8,
    pub kind: i8,
    pub priority: i8,
}

impl Goal {
    pub const FREE: Goal = Goal { x: 0, y: 0, kind: goal::FREE, priority: 0 };
    /// The uninitialised slots CIV.EXE starts a game with (zeroed BSS: type 0 at (0,0)).
    pub const ZERO: Goal = Goal { x: 0, y: 0, kind: 0, priority: 0 };

    pub fn pos(&self) -> (i32, i32) {
        (self.x as i32, self.y as i32)
    }

    pub fn is_free(&self) -> bool {
        self.kind == goal::FREE
    }
}

/// The page-2 style connectivity of the 20x13 grid of 4x4 squares the goto works with
/// (CIV.EXE ds:0xdb44 land, ds:0x7f38 sea; built by overlay 7 0x9edd / 0xa17a).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockLinks {
    pub land: Vec<u8>,
    pub sea: Vec<u8>,
}

/// The cost grid of the last local goto search (ds:0xb780, 16x16 around the waypoint cached in
/// ds:0x6796/0x6798), reused while the waypoint stays the same.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GotoCache {
    pub waypoint: Option<(i32, i32)>,
    pub grid: Vec<u8>,
}

/// AI state that CIV.EXE keeps in global tables and unit bytes civrs's records lack.
#[derive(Clone, Debug, PartialEq)]
pub struct AiMemory {
    pub goals: [[Goal; GOALS]; 8],
    pub persistent: [[Goal; PERSISTENT_GOALS]; 8],
    /// ds:0xe698[civ][continent]: the continent policies of each civilization's last AI turn
    /// (saved); `None` until the first AI turn of a game loaded from an older save.
    pub policies: [Option<[u8; 16]>; 8],
    /// The 20x13 coarse presence map of the last census (far ds:0x63f6 +0x2438): bit 0 a city
    /// of the civilization in the 4x4 block, bit (1 + role) one of its units.
    pub coarse: [[u8; 13]; 20],
    /// Unit flag 0x10 of the census: next to a foreign unit or a land unit at sea (skipped
    /// by the unit tasking).
    pub busy: HashSet<UnitId>,
    /// Unit byte +8, the direction of the unit's last step (1..8; absent = 0xff), read by
    /// the goto and the AI move scoring. Barbarians keep theirs in `BarbarianMemory`.
    pub heading: HashMap<UnitId, u8>,
    /// ds:0x6b90 while a civilization's units move (`move_units` 0x4046): the "focus" civ
    /// whose new type 3 and 4 goals redirect its units at once (0x18830).
    pub moving: Option<Slot>,
    pub goto_cache: GotoCache,
    pub links: Option<BlockLinks>,
    /// The census of the last AI turn (the tables ds:0x6cac, 0x807a, 0x6b96, ...), read by the
    /// unit routine while the civilization moves.
    pub census: super::production::Census,
}

impl Default for AiMemory {
    fn default() -> Self {
        AiMemory {
            goals: [[Goal::ZERO; GOALS]; 8],
            persistent: [[Goal::FREE; PERSISTENT_GOALS]; 8],
            policies: [None; 8],
            coarse: [[0; 13]; 20],
            busy: HashSet::new(),
            heading: HashMap::new(),
            moving: None,
            goto_cache: GotoCache::default(),
            links: None,
            census: Default::default(),
        }
    }
}

/// Insert `g` into a sorted goal list as 0x187e3 / 0x189e7 do: nothing when the same square
/// and type is already listed with a priority up to the new one; otherwise at the first slot
/// that is free or of lower priority, after shifting the occupied slots from there one down
/// (0x1896a / 0x18a95: only slots with a non-zero priority are copied, the last one falls off).
fn insert_goal(list: &mut [Goal], g: Goal) {
    if list.iter().any(|o| o.x == g.x && o.y == g.y && o.kind == g.kind && o.priority <= g.priority) {
        return;
    }
    let Some(at) = list.iter().position(|o| o.is_free() || o.priority < g.priority) else { return };
    for j in (at..list.len() - 1).rev() {
        if list[j].priority != 0 {
            list[j + 1] = list[j];
        }
    }
    list[at] = g;
}

impl Game {
    /// CIV.EXE 0x1876d `ai_reset_goals(civ)`: free the 32 transient goals, then add every
    /// persistent goal of type 0..2 to them.
    pub fn reset_goals(&mut self, civ: Slot) {
        let c = civ as usize;
        self.ai_memory.goals[c] = [Goal::FREE; GOALS];
        for p in self.ai_memory.persistent[c] {
            if (0..=2).contains(&p.kind) {
                self.add_goal(civ, p.x as i32, p.y as i32, p.kind, p.priority as i32);
            }
        }
    }

    /// CIV.EXE 0x187e3 `ai_add_goal(civ, x, y, type, priority)`. While `civ` moves its units
    /// (the focus civ ds:0x6b90) and is not the human, a new sea-attack (3) or air (4) goal
    /// first sends every unit of that role on the goal's continent that still has moves and
    /// is within half its move points (in thirds) of the square there at once (goto, AI flag
    /// 0x10, orders cleared). An existing goal with the same square and type and a priority
    /// up to the new one makes the call a no-op before that.
    pub fn add_goal(&mut self, civ: Slot, x: i32, y: i32, kind: i8, priority: i32) {
        let c = civ as usize;
        if c >= 8 {
            return;
        }
        let g = Goal { x: x as i8, y: y as i8, kind, priority: priority as i8 };
        if self.ai_memory.goals[c].iter().any(|o| o.x == g.x && o.y == g.y && o.kind == g.kind && o.priority <= g.priority) {
            return;
        }
        if self.ai_memory.moving == Some(civ) && civ != self.human_slot && (kind == goal::SEA || kind == goal::AIR) {
            let cont = self.continent_exe(x, y);
            let ids: Vec<UnitId> = self.units_of(civ);
            for id in ids {
                let u = self.unit(id).clone();
                if u.moves_left == 0 || u.info().role as i8 != kind {
                    continue;
                }
                if self.continent_exe(u.x, u.y) != cont {
                    continue;
                }
                if (u.moves_left as i32) < 2 * Map::civ_distance(u.pos(), (x, y)) {
                    continue;
                }
                self.ai_set_goto(id, (x, y));
            }
        }
        insert_goal(&mut self.ai_memory.goals[c], g);
    }

    /// A goto given by the AI (unit bytes +6/+7, flag 0x10 set and the low order bits
    /// 0x0f — sentry, fortifying, fortified — cleared, 0x16416 / 0x188fc).
    pub fn ai_set_goto(&mut self, id: UnitId, to: (i32, i32)) {
        self.ai_memory.busy.insert(id);
        let u = self.unit_mut(id);
        u.goto = Some(to);
        u.order = super::UnitOrder::Goto;
        u.order_turns = 0;
    }

    /// CIV.EXE 0x189b9 `ai_clear_persistent_goals(civ)` (a new game, overlay 5 0x4a74).
    pub fn clear_persistent_goals(&mut self, civ: Slot) {
        self.ai_memory.persistent[civ as usize] = [Goal::FREE; PERSISTENT_GOALS];
    }

    /// CIV.EXE 0x189e7 `ai_add_persistent_goal(civ, x, y, type, priority)` (same insertion,
    /// no redirect).
    pub fn add_persistent_goal(&mut self, civ: Slot, x: i32, y: i32, kind: i8, priority: i32) {
        let g = Goal { x: x as i8, y: y as i8, kind, priority: priority as i8 };
        insert_goal(&mut self.ai_memory.persistent[civ as usize], g);
    }

    /// CIV.EXE 0x18ae4 `ai_find_goal(civ, type, x, y, continent)`: the persistent goal of
    /// `kind` (on `continent` unless `None`) with the lowest distance / (priority + 1) and
    /// that distance (ds:0x6b88; 999 when none). No caller was found in the binary.
    pub fn find_goal(&mut self, civ: Slot, kind: i8, x: i32, y: i32, continent: Option<u8>) -> (Option<usize>, i32) {
        let mut best = (None, 999);
        let mut best_value = 999;
        for i in 0..PERSISTENT_GOALS {
            let p = self.ai_memory.persistent[civ as usize][i];
            if p.kind != kind {
                continue;
            }
            if let Some(k) = continent {
                if self.continent_exe(p.x as i32, p.y as i32) != k {
                    continue;
                }
            }
            let d = Map::civ_distance((x, y), p.pos());
            let v = d / (p.priority as i32 + 1);
            if v < best_value {
                best_value = v;
                best = (Some(i), d);
            }
        }
        best
    }

    /// CIV.EXE 0x18b97 `ai_remove_goals_near(civ, type, x, y, radius)`: persistent goals of
    /// `kind` within `radius` of (x,y) become free (their priority stays). Called for type 0
    /// (city sites) with radius 4 when a city is founded (o 0x1b38c) or captured (0x13faf),
    /// and with radius 0 by Settlers standing on a site (0x16ab1).
    pub fn remove_goals_near(&mut self, civ: Slot, kind: i8, x: i32, y: i32, radius: i32) {
        if civ as usize >= 8 {
            return;
        }
        for p in self.ai_memory.persistent[civ as usize].iter_mut() {
            if p.kind == kind && Map::civ_distance((x, y), p.pos()) <= radius {
                p.kind = goal::FREE;
            }
        }
    }
}

impl Game {
    /// The goal and goto side of `unit_look` (0x8038), run where civrs's unit looks around
    /// (after every move and at the start of its civilization's turn, with `look_for_contacts`).
    ///
    /// For each of the 8 neighbours n:
    /// - a city of another civ o next to a land unit (0x82c5) whose civ has no treaty with o
    ///   gets o's attack (1, priority 4) and defend (2, priority 2) goals on itself (0x82ea);
    /// - units of another civ o2 there: their sentried units wake but for land units at sea
    ///   (wake_if_not_at_sea 0x92fe, so ships wake too), the first one's goto
    ///   is dropped unless it is at sea and the looker on land; the looker's goto is dropped
    ///   unless it stands on land beside a ship (aircraft always). Between land units on land
    ///   without a treaty the looker's civ gets an attack goal on n (priority 3, or 1 against
    ///   barbarians, a Diplomat or a fortified defender) and, unless the looker is a Diplomat,
    ///   o2 a defend goal on the looker's square (priority 2, 1 when the looker is a barbarian);
    ///   a foreign ship (not barbarian, no treaty) is a sea goal (3, 2) of the looker's civ.
    ///
    /// For the 16 squares of the outer ring: a looker that sees two squares far (sight bit 2,
    /// ships over water only) makes foreign ships there sea goals (3, 2) of its civ; a foreign
    /// unit there that sees two squares far (a ship only with the looker at sea) wakes, drops
    /// its goto and, on water, gives its civ a sea goal (3, 2) on the looker (0x8546..0x8642),
    /// not for a looking Submarine.
    pub fn unit_look_goals(&mut self, id: UnitId) {
        use super::diplomacy::flags::PEACE;
        use crate::rules::{unit_ids, AiRole, Domain};
        let Some(u) = self.unit_opt(id).cloned() else { return };
        let civ = u.owner;
        let here_ocean = self.map.is_ocean(u.x, u.y);
        let land = u.info().domain == Domain::Land;
        let peace = |g: &Game, a: Slot, b: Slot| g.diplo(a, b) & PEACE != 0;
        for k in 1..=24usize {
            let (nx, ny) = (Map::wrap_x(u.x + super::goto::DX[k]), u.y + super::goto::DY[k]);
            if !Map::in_bounds(nx, ny) {
                continue;
            }
            let n_ocean = self.map.is_ocean(nx, ny);
            let first = self.units_at(nx, ny).into_iter().find(|o| self.unit(*o).owner != civ);
            if k <= 8 {
                if let Some(c) = self.city_at(nx, ny) {
                    let o = self.city(c).owner;
                    if o != civ && land && !peace(self, civ, o) {
                        self.add_goal(o, nx, ny, goal::ATTACK, 4);
                        self.add_goal(o, nx, ny, goal::DEFEND, 2);
                    }
                }
                let Some(f) = first else { continue };
                let fu = self.unit(f).clone();
                let o2 = fu.owner;
                for w in self.units_at(nx, ny) {
                    let wu = self.unit_mut(w);
                    if wu.order == super::UnitOrder::Sentry && !(n_ocean && wu.is_land()) {
                        wu.order = super::UnitOrder::None;
                    }
                }
                if !n_ocean || here_ocean {
                    self.clear_goto(f);
                }
                if !(n_ocean && !here_ocean && !u.is_air()) {
                    self.clear_goto(id);
                }
                if land && !here_ocean && !n_ocean && !fu.is_air() {
                    if !peace(self, civ, o2) {
                        let defender = fu.order == super::UnitOrder::Fortified && fu.info().role == AiRole::Defend;
                        let p = if o2 != 0 && fu.kind != unit_ids::DIPLOMAT && !defender { 3 } else { 1 };
                        self.add_goal(civ, nx, ny, goal::ATTACK, p);
                        if u.kind != unit_ids::DIPLOMAT {
                            self.add_goal(o2, u.x, u.y, goal::DEFEND, if civ == 0 { 1 } else { 2 });
                        }
                    }
                }
                if n_ocean && o2 != 0 && !peace(self, civ, o2) {
                    self.add_goal(civ, nx, ny, goal::SEA, 2);
                }
            } else {
                let Some(f) = first else { continue };
                let fu = self.unit(f).clone();
                let o2 = fu.owner;
                if u.kind == unit_ids::SUBMARINE {
                    continue;
                }
                let far = u.info().sight & 2 != 0 && (u.info().domain != Domain::Sea || n_ocean);
                if far && fu.kind != unit_ids::SUBMARINE && n_ocean && o2 != 0 && !peace(self, civ, o2) {
                    self.add_goal(civ, nx, ny, goal::SEA, 2);
                }
                if fu.info().sight & 2 != 0 && (fu.info().domain != Domain::Sea || here_ocean) {
                    let fm = self.unit_mut(f);
                    if fm.order == super::UnitOrder::Sentry {
                        fm.order = super::UnitOrder::None;
                    }
                    self.clear_goto(f);
                    if n_ocean && o2 != 0 && !peace(self, civ, o2) {
                        self.add_goal(o2, u.x, u.y, goal::SEA, 2);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;

    fn game() -> Game {
        Game::new(GameSetup { seed: 7, ..Default::default() }, None)
    }

    #[test]
    fn goals_are_sorted_by_priority_and_duplicates_with_lower_priority_block() {
        let mut g = game();
        g.reset_goals(2);
        g.add_goal(2, 10, 10, goal::DEFEND, 2);
        g.add_goal(2, 11, 10, goal::ATTACK, 5);
        g.add_goal(2, 12, 10, goal::ATTACK, 3);
        let l = &g.ai_memory.goals[2];
        assert_eq!([l[0].priority, l[1].priority, l[2].priority], [5, 3, 2]);
        assert!(l[3].is_free());
        // the same square and type with a lower or equal listed priority: ignored
        g.add_goal(2, 10, 10, goal::DEFEND, 4);
        assert_eq!(g.ai_memory.goals[2].iter().filter(|o| o.pos() == (10, 10)).count(), 1);
        // a listed duplicate of higher priority does not block a lower one
        g.add_goal(2, 11, 10, goal::ATTACK, 1);
        let l = &g.ai_memory.goals[2];
        assert_eq!(l.iter().filter(|o| o.pos() == (11, 10)).count(), 2);
        assert_eq!(l[3], Goal { x: 11, y: 10, kind: goal::ATTACK, priority: 1 });
    }

    #[test]
    fn a_full_list_drops_its_last_goal_and_refuses_lower_ones() {
        let mut g = game();
        g.reset_goals(3);
        for i in 0..GOALS as i32 {
            g.add_goal(3, i, 1, goal::ATTACK, 3);
        }
        assert!(g.ai_memory.goals[3].iter().all(|o| o.priority == 3));
        g.add_goal(3, 40, 40, goal::ATTACK, 2);
        assert!(!g.ai_memory.goals[3].iter().any(|o| o.pos() == (40, 40)));
        g.add_goal(3, 41, 41, goal::ATTACK, 4);
        assert_eq!(g.ai_memory.goals[3][0].pos(), (41, 41));
        // equal priorities insert after their equals: the first goal added is now second
        assert_eq!(g.ai_memory.goals[3][1].pos(), (0, 1));
        assert_eq!(g.ai_memory.goals[3][GOALS - 1].pos(), (30, 1));
    }

    #[test]
    fn reset_copies_persistent_goals_up_to_type_2() {
        let mut g = game();
        g.clear_persistent_goals(4);
        g.add_persistent_goal(4, 5, 6, goal::SETTLE, 2);
        g.add_persistent_goal(4, 7, 8, goal::SEA, 6);
        g.add_persistent_goal(4, 9, 9, goal::DEFEND, 3);
        g.reset_goals(4);
        let l = &g.ai_memory.goals[4];
        assert_eq!(l[0], Goal { x: 9, y: 9, kind: goal::DEFEND, priority: 3 });
        assert_eq!(l[1], Goal { x: 5, y: 6, kind: goal::SETTLE, priority: 2 });
        assert!(l[2].is_free());
        // city sites near a new city are forgotten
        g.remove_goals_near(4, goal::SETTLE, 7, 7, 4);
        assert!(g.ai_memory.persistent[4].iter().all(|p| p.kind != goal::SETTLE));
        assert_eq!(g.find_goal(4, goal::DEFEND, 9, 12, None), (Some(1), 3));
        assert_eq!(g.find_goal(4, goal::ATTACK, 9, 12, None), (None, 999));
    }
}
