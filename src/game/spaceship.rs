//! Spaceship construction, launch and flight (CIV.EXE overlay 18, o 0x17575..0x18c46).
//!
//! SS Structural/Component/Module "improvements" are not city improvements: completing one
//! puts a part on the owner's ship grid (`spaceship_part_built`, called from production
//! completion). The grid has `GRID_W` columns x `GRID_H` rows; parts only go on the cells of the
//! fixed placement list `PLACEMENT`. A part counts for the ship's statistics only when it is
//! attached: a structural always, any other part when a cell orthogonally next to its
//! footprint holds a structural (o 0x17964..0x179ed). The statistics (`ship_stats`, o 0x177ce):
//!
//! - population = 10,000 per habitation module; support = 100 * life / clamp(hab, 1, 99) %;
//!   energy = 200 * solar / clamp(hab + life, 1, 99) %; fuel = 100 * fuel / clamp(prop, 1, 99) %;
//! - mass = sum of part masses (parts table ds:0x4bd0) in units of 100 tons;
//! - flight time = 50 * mass / (min(10 * prop, 10 * fuel) + 1) tenths of years;
//! - success = 100, times clamp(support, 0, 100)% (only when the screen is drawn: the
//!   human's ship, or any ship with the Enemy Moves option on, o 0x177db), times
//!   clamp(energy, 0, 100)%, minus (flight - 150) / 10 when the flight takes over 15 years,
//!   clamped to 0..100.
//!
//! The ship lands in the year `launch year + flight / 10` without any roll: the success chance
//! only scales the human's score (o 0x1c3dd) and steers the AI's launch decision (0xe835).
//! Once a ship is in flight the calendar runs one year per turn from 1750 on (0x26ed,
//! `Game::next_year`), so a ship launched then lands on the turn whose year equals its arrival
//! year. CIV.EXE compares for equality (0x2a13): a ship launched before 1750, while the years
//! still go in steps of 5 or more, can miss its year and never land; civrs keeps that.

use super::events::{Decision, EndReason, GameEvent};
use super::map::{CityId, Slot};
use super::player::Spaceship;
use super::{Game, Production, RuleError};
use crate::rules::{self, improvement_ids as imp};

/// Grid columns (x, first byte of a placement entry); the ship screen draws column x at
/// pixel 16 * x - 16.
pub const GRID_W: usize = 15;
/// Grid rows (y, second byte); row y is drawn at pixel 16 * y + 8.
pub const GRID_H: usize = 12;
/// `grid[x][y]`: 0 = empty, else part type + 1 (es:0x16e0 + 0xb4 * civ + 12 * x + y, -1 = empty).
pub type ShipGrid = [[u8; GRID_H]; GRID_W];

/// Part types (index into `PARTS`); `kind_of` maps them to the three SS improvements.
pub mod part {
    pub const STRUCTURE1: u8 = 0;
    pub const STRUCTURE2: u8 = 1;
    pub const STRUCTURE3: u8 = 2;
    pub const PROPULSION: u8 = 3;
    pub const FUEL: u8 = 4;
    pub const HABITATION: u8 = 6;
    pub const LIFE_SUPPORT: u8 = 7;
    pub const SOLAR_PANEL: u8 = 8;
}

/// One record of the parts table ds:0x4bd0 (26 bytes: name, mass word at +0x10, DOCKER.PIC
/// sprite rect x, y, w, h).
pub struct ShipPart {
    pub name: &'static str,
    pub mass: i32,
    pub sprite: (i32, i32, i32, i32),
}

pub static PARTS: [ShipPart; 10] = [
    ShipPart { name: "Structure1", mass: 1, sprite: (0, 16, 16, 16) },
    ShipPart { name: "Structure2", mass: 1, sprite: (16, 16, 16, 16) },
    ShipPart { name: "Structure3", mass: 1, sprite: (32, 16, 16, 16) },
    ShipPart { name: "Propulsion", mass: 4, sprite: (256, 48, 32, 16) },
    ShipPart { name: "Fuel", mass: 4, sprite: (0, 80, 16, 16) },
    // the capsule drawn at (152, 88) once the mass reaches 80 (o 0x17a85); never placed
    ShipPart { name: "NONE", mass: 4, sprite: (192, 96, 32, 32) },
    ShipPart { name: "Habitation", mass: 16, sprite: (224, 96, 32, 32) },
    ShipPart { name: "Life Support", mass: 16, sprite: (288, 96, 32, 32) },
    ShipPart { name: "Solar Panel", mass: 4, sprite: (0, 96, 32, 32) },
    ShipPart { name: "Support", mass: 16, sprite: (256, 96, 32, 32) },
];

/// The fixed placement list (x, y, part type), far data segment 0x26d9 (ds:0x6440; file
/// 0x26d90 of the unpacked image), terminated by -1 after 67 entries. Entries before
/// `MODULE_SECTION` build the propulsion section, the rest the module section. Five structural
/// entries of the module section (33..36, 39) repeat cells of the first section, so those
/// cells serve both sections and count twice in the mass.
pub static PLACEMENT: [(u8, u8, u8); 67] = [
    (11, 5, 1), (11, 4, 2), (11, 6, 1), (11, 7, 2), (12, 4, 0), (13, 4, 0), (12, 5, 4), (13, 5, 3),
    (12, 7, 0), (13, 7, 0), (12, 6, 4), (13, 6, 3), (12, 8, 4), (13, 8, 3), (12, 3, 4), (13, 3, 3),
    (11, 9, 1), (11, 10, 2), (12, 10, 0), (13, 10, 0), (12, 9, 4), (13, 9, 3), (11, 3, 2), (11, 2, 1),
    (11, 1, 2), (12, 1, 0), (13, 1, 0), (12, 2, 4), (13, 2, 3), (12, 11, 4), (13, 11, 3), (12, 0, 4),
    (13, 0, 3), (11, 5, 1), (11, 4, 2), (11, 6, 1), (11, 7, 2), (9, 6, 6), (9, 4, 7), (11, 3, 2),
    (10, 3, 0), (9, 3, 0), (8, 1, 8), (11, 8, 2), (10, 8, 0), (9, 8, 0), (8, 8, 0), (7, 8, 0),
    (7, 6, 7), (8, 3, 0), (7, 3, 0), (8, 9, 8), (7, 4, 6), (6, 3, 0), (5, 3, 0), (5, 4, 7),
    (6, 8, 0), (5, 8, 0), (4, 9, 8), (5, 6, 6), (4, 3, 0), (3, 3, 0), (3, 6, 7), (4, 8, 0),
    (3, 8, 0), (4, 1, 8), (3, 4, 6),
];

/// First entry of the module section (0x21).
pub const MODULE_SECTION: usize = 0x21;

/// Distinct cells per kind on the placement list (structural, component, module).
pub const MAX_STRUCTURALS: u8 = 34;
pub const MAX_COMPONENTS: u8 = 16;
pub const MAX_MODULES: u8 = 12;

/// 0 structural, 1 component, 2 module (`type / 3`, o 0x1837e).
pub fn kind_of(part_type: u8) -> u8 {
    part_type / 3
}

/// Footprint in cells (w, h): modules 2x2, propulsion 2x1, the rest 1x1 (o 0x1792c..0x17951).
pub fn footprint(part_type: u8) -> (i32, i32) {
    let w = if part_type >= 6 || part_type == part::PROPULSION { 2 } else { 1 };
    let h = if part_type >= 6 { 2 } else { 1 };
    (w, h)
}

/// The values ship_draw (o 0x177ce) computes and shows in its stats column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShipStats {
    /// Attached parts per type, ds:0xdea2[type] (list entries, so shared cells count twice).
    pub counts: [i32; 10],
    /// Sum of part masses in 100 tons ("Mass: m/10,m%10 00 tons").
    pub mass: i32,
    /// Support (life support per habitation), percent.
    pub support: i32,
    /// Energy (solar panels per two other modules), percent.
    pub energy: i32,
    /// Fuel per propulsion, percent.
    pub fuel: i32,
    /// Flight time in tenths of years.
    pub flight: i32,
    /// Probability of success, percent.
    pub success: i32,
}

impl ShipStats {
    pub fn propulsion(&self) -> i32 {
        self.counts[part::PROPULSION as usize]
    }
    pub fn fuel_parts(&self) -> i32 {
        self.counts[part::FUEL as usize]
    }
    pub fn habitation(&self) -> i32 {
        self.counts[part::HABITATION as usize]
    }
    pub fn life_support(&self) -> i32 {
        self.counts[part::LIFE_SUPPORT as usize]
    }
    pub fn solar(&self) -> i32 {
        self.counts[part::SOLAR_PANEL as usize]
    }
    /// Colonists ("Population: <hab>0,000"); ds:0xdeec[civ] keeps the habitation count.
    pub fn population(&self) -> i32 {
        10_000 * self.habitation()
    }
}

/// Grid cell holds a structural (o 0x18186). CIV.EXE does not bounds-check and reads the
/// neighbouring column / civ for cells off the grid; no such cell can hold a structural for
/// the parts of `PLACEMENT`, so off-grid cells are simply not structural here.
fn is_structure(s: &Spaceship, x: i32, y: i32) -> bool {
    s.part_at(x, y).is_some_and(|t| t < 3)
}

/// The part of type `t` at (x, y) counts: a structural always, anything else when a cell
/// orthogonally next to its footprint holds a structural (o 0x17964..0x179ed; ship_draw
/// outlines the others in colour 12).
pub fn part_attached(s: &Spaceship, x: i32, y: i32, t: u8) -> bool {
    let (w, h) = footprint(t);
    let mut attached = t < 3;
    for dx in 0..w {
        for dy in 0..h {
            let (cx, cy) = (x + dx, y + dy);
            attached |= [(cx + 1, cy), (cx - 1, cy), (cx, cy + 1), (cx, cy - 1)].iter().any(|&(nx, ny)| is_structure(s, nx, ny));
        }
    }
    attached
}

/// Statistics of a ship (o 0x177ce). `drawn` is ship_draw's "screen visible" flag
/// (`Game::ship_drawn`: the human's ship, or any with Enemy Moves on): only then does a
/// life-support shortfall lower the success chance (the multiplication sits inside the
/// drawing branch, o 0x17c0a).
pub fn ship_stats(s: &Spaceship, drawn: bool) -> ShipStats {
    let mut st = ShipStats::default();
    for &(x, y, _) in PLACEMENT.iter() {
        let (x, y) = (x as i32, y as i32);
        let Some(t) = s.part_at(x, y) else { continue };
        if part_attached(s, x, y, t) {
            st.counts[t as usize] += 1;
        }
    }
    st.mass = (0..10).map(|t| PARTS[t].mass * st.counts[t]).sum();
    let (hab, life, solar) = (st.habitation(), st.life_support(), st.solar());
    let (prop, fuel) = (st.propulsion(), st.fuel_parts());
    st.support = 100 * life / hab.clamp(1, 99);
    st.energy = 200 * solar / (hab + life).clamp(1, 99);
    st.fuel = 100 * fuel / prop.clamp(1, 99);
    st.flight = 50 * st.mass / ((10 * prop).clamp(0, 10 * fuel) + 1);
    let mut success = 100;
    if drawn {
        success = st.support.clamp(0, 100) * success / 100;
    }
    success = st.energy.clamp(0, 100) * success / 100;
    if st.flight > 150 {
        success -= (st.flight - 150) / 10;
    }
    st.success = success.clamp(0, 100);
    st
}

fn cell_free(s: &Spaceship, entry: usize) -> bool {
    let (x, y, _) = PLACEMENT[entry];
    s.part_at(x as i32, y as i32).is_none()
}

/// Where a human's structural goes (o 0x185b9..0x186d9): the first free structural entry of
/// the section whose last placed component/module lies furthest beyond it (the module section
/// on a tie). None when that section has no free structural entry (CIV.EXE then writes through
/// entry 99, past the end of the list).
pub fn human_structural_slot(s: &Spaceship) -> Option<usize> {
    let scan = |range: std::ops::Range<usize>| {
        let mut first_free = 99i32;
        let mut last_other = -1i32;
        for i in range {
            let t = PLACEMENT[i].2;
            if cell_free(s, i) {
                if t < 3 && first_free == 99 {
                    first_free = i as i32;
                }
            } else if t >= 3 {
                last_other = i as i32;
            }
        }
        (first_free, last_other)
    };
    let (a_free, a_last) = scan(0..MODULE_SECTION);
    let (b_free, b_last) = scan(MODULE_SECTION..PLACEMENT.len());
    let pick = if b_last - b_free < a_last - a_free { a_free } else { b_free };
    (pick != 99).then_some(pick as usize)
}

/// Where a human's component/module of `part_type` goes (o 0x186de): the first free entry of
/// that type. None when there is none (the "No further X required." branch is unreachable in
/// CIV.EXE, which instead writes -1 through the terminator entry; nothing is placed here).
pub fn first_free_slot(s: &Spaceship, part_type: u8) -> Option<usize> {
    (0..PLACEMENT.len()).find(|i| PLACEMENT[*i].2 == part_type && cell_free(s, *i))
}

/// Outcome of the AI part placement scan (o 0x182c4 return value).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiPick {
    /// 0: place at this entry.
    Place(usize),
    /// 1: build the next kind instead (passed a free slot of a higher kind by 4+ entries and the
    /// next kind's advance is known, or the list ran out with such a slot still free).
    Refuse,
    /// 2: no slot of this kind and none of a higher kind left.
    Exhausted,
}

/// The AI's scan (o 0x18315..0x18428) for a part of `kind` starting at entry `start` (0 or
/// `MODULE_SECTION`); `knows_next` = the civ knows the prerequisite of the next kind's
/// improvement (ds:0xe68 + 0x1e * kind = prereq of improvement 23 + kind).
pub fn ai_pick(s: &Spaceship, kind: u8, start: usize, knows_next: bool) -> AiPick {
    let mut first_higher: Option<usize> = None;
    let mut i = start;
    loop {
        let t = PLACEMENT[i].2;
        if cell_free(s, i) {
            if first_higher.is_none() && t >= 3 * kind + 3 {
                first_higher = Some(i);
            }
            if kind_of(t) == kind {
                return AiPick::Place(i);
            }
        }
        i += 1;
        if i == PLACEMENT.len() {
            return if first_higher.is_none() { AiPick::Exhausted } else { AiPick::Refuse };
        }
        if first_higher.is_some_and(|f| i - f >= 4) && kind != 2 && knows_next {
            return AiPick::Refuse;
        }
    }
}

/// Put `PLACEMENT[entry]`'s part on the grid.
fn place(s: &mut Spaceship, entry: usize) {
    let (x, y, t) = PLACEMENT[entry];
    s.grid[x as usize][y as usize] = t + 1;
    s.recount();
    s.lost = false;
}

/// Rebuild a grid from the part counters of a pre-grid (version < 5) save: structurals where a
/// human's would go, components alternating propulsion/fuel, modules in the old rotation
/// habitation, life support, solar panel.
pub fn grid_from_counts(s: &mut Spaceship) {
    let (structurals, components, modules) = (s.structurals, s.components, s.modules);
    s.grid = ShipGrid::default();
    for n in 0..components {
        let t = if n % 2 == 0 { part::PROPULSION } else { part::FUEL };
        if let Some(i) = first_free_slot(s, t) {
            place(s, i);
        }
    }
    for n in 0..modules {
        let t = [part::HABITATION, part::LIFE_SUPPORT, part::SOLAR_PANEL][n as usize % 3];
        if let Some(i) = first_free_slot(s, t) {
            place(s, i);
        }
    }
    for _ in 0..structurals {
        if let Some(i) = human_structural_slot(s) {
            place(s, i);
        }
    }
    s.recount();
}

impl Game {
    /// Statistics of a civ's ship as ship_draw computes them.
    pub fn spaceship_stats(&self, slot: Slot) -> ShipStats {
        ship_stats(&self.player(slot).spaceship, self.ship_drawn(slot))
    }

    /// ship_draw shows (and counts the life support of) the human's ship, and every other
    /// ship when the Enemy Moves option (ds:0x19c0 bit 5) is on (o 0x177db..0x177f1).
    pub fn ship_drawn(&self, slot: Slot) -> bool {
        self.is_human(slot) || self.options.enemy_moves
    }

    /// Probability (percent) shown as "Prob. of Success".
    pub fn success_chance(&self, slot: Slot) -> i32 {
        self.spaceship_stats(slot).success
    }

    /// ship_draw's side effect: until launch the arrival year is re-projected from the current
    /// year (o 0x17ff3).
    fn project_arrival(&mut self, slot: Slot) {
        if self.player(slot).spaceship.launched() {
            return;
        }
        let flight = self.spaceship_stats(slot).flight;
        let year = self.year();
        self.player_mut(slot).spaceship.arrival_year = Some(year + flight / 10);
    }

    /// Completion of an SS improvement (`kind` 0 structural, 1 component, 2 module) in city
    /// `c` (0xe766..0xe916), after the shields were spent and an AI owner re-chose production.
    pub fn spaceship_part_built(&mut self, c: CityId, kind: u8) {
        let slot = self.city(c).owner;
        if self.player(slot).spaceship.launched() {
            return;
        }
        if self.is_human(slot) {
            if kind == 0 {
                self.place_structural(slot);
            } else {
                self.push_event(GameEvent::NeedsDecision(Decision::SpaceshipPart { slot, kind }));
            }
        } else {
            self.ai_spaceship_part(c, kind);
        }
    }

    fn place_structural(&mut self, slot: Slot) {
        if let Some(i) = human_structural_slot(&self.player(slot).spaceship) {
            place(&mut self.player_mut(slot).spaceship, i);
        }
        self.after_human_part(slot);
    }

    /// The human answered "Type?" for a component (`kind` 1: 0 Propulsion, 1 Fuel) or a module
    /// (`kind` 2: 0 Habitation, 1 Life Support, 2 Solar Panel) (o 0x1853f..0x185a8).
    pub fn place_spaceship_part(&mut self, slot: Slot, kind: u8, choice: usize) {
        let t = match kind {
            0 => return self.place_structural(slot),
            1 => part::PROPULSION + choice.min(1) as u8,
            _ => part::HABITATION + choice.min(2) as u8,
        };
        if let Some(i) = first_free_slot(&self.player(slot).spaceship, t) {
            place(&mut self.player_mut(slot).spaceship, i);
        }
        self.after_human_part(slot);
    }

    /// Redraw with the LAUNCH button (ship_draw(civ, -1, 1), o 0x18842): the human sees the
    /// ship after every part, with the button once it can be launched.
    fn after_human_part(&mut self, slot: Slot) {
        self.project_arrival(slot);
        self.push_event(GameEvent::NeedsDecision(Decision::SpaceshipLaunch { slot }));
    }

    /// The AI's slot scan for a part of `kind` (o 0x182c4 up to 0x18428).
    pub fn ai_spaceship_pick(&self, slot: Slot, kind: u8) -> AiPick {
        let s = self.player(slot).spaceship;
        let year = self.year();
        // near the end (projected arrival within 15 years and ahead of the human's) the
        // structurals and components go to the module section first (o 0x182de)
        let late = match (s.arrival_year, self.player(self.human_slot).spaceship.arrival_year) {
            (Some(a), h) => a - year < 15 && h.is_none_or(|h| a < h),
            (None, _) => false,
        };
        let start = if kind == 2 || late { MODULE_SECTION } else { 0 };
        let next_prereq = rules::IMPROVEMENTS[(imp::SS_COMPONENT + kind.min(1)) as usize].prereq;
        let knows_next = next_prereq.is_none_or(|a| self.player(slot).knows(a));
        ai_pick(&s, kind, start, knows_next)
    }

    /// SS improvements in the order an AI city should try them: the kinds the ship would take
    /// first, then the rest from modules down. CIV.EXE's own weighting of the three parts in
    /// choose_production (0xa9fd) is not decoded; this keeps the AI clear of refunded parts
    /// (structurals first, as before, while they still fit).
    pub fn ai_spaceship_preference(&self, slot: Slot) -> Vec<rules::ImprovementId> {
        let (fits, rest): (Vec<u8>, Vec<u8>) = (0..3u8).partition(|k| matches!(self.ai_spaceship_pick(slot, *k), AiPick::Place(_)));
        fits.into_iter().chain(rest.into_iter().rev()).map(|k| imp::SS_STRUCTURAL + k).collect()
    }

    /// AI part (o 0x182c4) and the launch decision that follows it (0xe7af..0xe916).
    fn ai_spaceship_part(&mut self, c: CityId, kind: u8) {
        let slot = self.city(c).owner;
        let mut ret = match self.ai_spaceship_pick(slot, kind) {
            AiPick::Place(i) => {
                place(&mut self.player_mut(slot).spaceship, i);
                self.project_arrival(slot);
                self.push_event(GameEvent::SpaceshipPartBuilt(slot));
                0
            }
            AiPick::Refuse => 1,
            AiPick::Exhausted => 2,
        };
        // ds:0x4cd6: cleared on entry, the success chance of the redrawn ship once placed
        let success = if ret == 0 { self.spaceship_stats(slot).success } else { 0 };
        // refused structural/component: the shields come back and a city still set to a
        // spaceship part switches to the next kind (0xe7d1..0xe831)
        if ret != 0 && kind < 2 {
            let part = imp::SS_STRUCTURAL + kind;
            let refund = self.production_cost(c, Production::Improvement(part));
            self.city_mut(c).shields += refund;
            if matches!(self.city(c).production, Production::Improvement(i) if (imp::SS_STRUCTURAL..=imp::SS_MODULE).contains(&i)) && self.can_build_improvement(c, part + 1) {
                self.city_mut(c).production = Production::Improvement(part + 1);
            }
            ret -= 1;
        }
        if self.ai_launch_wanted(slot, ret != 0, success) {
            let _ = self.launch_spaceship(slot);
        }
    }

    /// The AI launch rule (0xe835..0xe916) with the success chance ds:0x4cd6 of the AI's own
    /// ship (support-blind unless Enemy Moves had it drawn). `forced` = no slot was left for the part (after the
    /// refund step).
    fn ai_launch_wanted(&self, slot: Slot, forced: bool, success: i32) -> bool {
        let human = self.human_slot;
        if forced {
            return true;
        }
        if success < 40 || !self.player(human).knows(rules::advance_ids::SPACE_FLIGHT) {
            return false;
        }
        if success > 75 || self.power_rank(human) > self.power_rank(slot) {
            return true;
        }
        let hs = self.player(human).spaceship;
        if hs.launched() {
            let year = self.year();
            let mine = self.player(slot).spaceship.arrival_year.unwrap_or(-1) - year;
            let theirs = hs.arrival_year.unwrap_or(-1) - year;
            if mine <= theirs {
                return true;
            }
        }
        // the last rule (0xe8cb) launches when the human has started a ship and
        // gold[ds:0x6b8a] > 1000, indexing the gold table with the active-civ *mask*: that
        // reads unit counts or other globals past the table, so it is treated as never true
        false
    }

    /// ds:0x803c[civ]: 7 for the strongest of civs 1..7 down to 1 (power ranking 0x3121:
    /// gold / 32 + 8 * citizens + research count + sum of unit costs; ties go to the lower
    /// slot). CIV.EXE ranks once per turn; this ranks on the spot.
    pub(crate) fn power_rank(&self, slot: Slot) -> i32 {
        let strength = |s: Slot| self.power_strength(s);
        let mine = strength(slot);
        let above = (1..8u8).filter(|o| *o != slot).filter(|o| {
            let v = strength(*o);
            v > mine || (v == mine && *o < slot)
        });
        7 - above.count() as i32
    }

    /// The LAUNCH button: a habitation module on board and not launched (o 0x17fa1).
    pub fn spaceship_can_launch(&self, slot: Slot) -> bool {
        let s = self.player(slot).spaceship;
        self.apollo_built && !s.launched() && self.spaceship_stats(slot).habitation() > 0
    }

    /// Launch (o 0x18b38): sets the launched bit and stores the launch year ds:0xe8a8; the
    /// arrival year is the one ship_draw projected last. A ship whose arrival year is not after the launch year never lands
    /// (CIV.EXE compares the year for equality).
    pub fn launch_spaceship(&mut self, slot: Slot) -> Result<(), RuleError> {
        if self.player(slot).spaceship.launched() {
            return Err(RuleError::Other("The spaceship has already been launched."));
        }
        if self.is_human(slot) {
            self.project_arrival(slot);
        }
        let turn = self.turn;
        let year = self.year();
        let s = &mut self.player_mut(slot).spaceship;
        s.launched_turn = Some(turn);
        s.launch_year = Some(year);
        s.arrival_turn = None;
        s.lost = false;
        self.push_event(GameEvent::SpaceshipLaunched(slot));
        Ok(())
    }

    /// Year in which a launched ship arrives (for the F12 report).
    pub fn spaceship_arrival_year(&self, slot: Slot) -> Option<i32> {
        let s = self.player(slot).spaceship;
        s.launched_turn.and(s.arrival_year)
    }

    /// The capital was captured (0x14364): "<Adj> spaceship returns to Earth." /
    /// "construction cancelled." when a ship was started, and the ship is cleared (o5 0x5e8f).
    pub fn cancel_spaceship(&mut self, slot: Slot) {
        let s = self.player(slot).spaceship;
        if s.parts() == 0 && !s.launched() {
            return;
        }
        self.push_event(GameEvent::SpaceshipCancelled { slot, launched: s.launched() });
        self.player_mut(slot).spaceship = Spaceship { lost: true, ..Spaceship::default() };
    }

    /// Ships whose arrival year has come (0x29f8): the last civ in slot order whose launched
    /// ship's arrival year equals the year wins. Called once per turn from `process_world`
    /// before the calendar advances, so the coming turn's year is the one compared, as CIV.EXE
    /// does right after its year advance.
    pub fn spaceship_arrivals(&mut self) {
        let year = self.next_year();
        let mut winner = None;
        for slot in 1..8u8 {
            let s = self.player(slot).spaceship;
            if s.launched() && !s.landed && s.arrival_year == Some(year) {
                winner = Some(slot);
            }
        }
        let Some(slot) = winner else { return };
        self.player_mut(slot).spaceship.landed = true;
        self.push_event(GameEvent::SpaceshipArrived(slot));
        if self.ended.is_none() {
            self.ended = Some(EndReason::Spaceship(slot));
            self.push_event(GameEvent::GameEnded(EndReason::Spaceship(slot)));
        }
    }

    /// Spaceship score (o 0x1c3cc): success % * colonists / 2 (colonists = habitation modules)
    /// for the human's ship once it has landed. CIV.EXE adds the human's ship to every civ's
    /// score and tests "arrival year == current year"; civrs counts it for the human only.
    pub fn spaceship_score(&self, slot: Slot) -> i32 {
        let s = self.player(slot).spaceship;
        if !self.is_human(slot) || !s.landed {
            return 0;
        }
        let st = self.spaceship_stats(slot);
        (st.success * st.habitation() / 2).max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;
    use crate::rules::unit_ids;

    fn ship_with(parts: &[usize]) -> Spaceship {
        let mut s = Spaceship::default();
        for i in parts {
            place(&mut s, *i);
        }
        s
    }

    #[test]
    fn placement_list_shape() {
        let count = |k: u8| PLACEMENT.iter().filter(|e| kind_of(e.2) == k).count();
        assert_eq!((count(0), count(1), count(2)), (39, 16, 12));
        let mut cells: Vec<(u8, u8)> = PLACEMENT.iter().map(|e| (e.0, e.1)).collect();
        cells.sort();
        cells.dedup();
        assert_eq!(cells.len(), PLACEMENT.len() - 5);
        for &(x, y, t) in PLACEMENT.iter() {
            let (w, h) = footprint(t);
            assert!((x as i32 + w) as usize <= GRID_W && (y as i32 + h) as usize <= GRID_H);
        }
    }

    #[test]
    fn attachment_and_stats() {
        // fuel at (12,5) next to the structural at (11,5); propulsion at (13,5) next to nothing
        let s = ship_with(&[6, 7]);
        assert_eq!(ship_stats(&s, true).counts[part::FUEL as usize], 0);
        let s = ship_with(&[0, 6, 7]);
        let st = ship_stats(&s, true);
        assert_eq!(st.fuel_parts(), 1);
        assert_eq!(st.propulsion(), 0);
        // entries 0 and 33 share the cell: the structural counts twice
        assert_eq!(st.counts[part::STRUCTURE2 as usize], 2);
        assert_eq!(st.mass, 2 + 4);
        let s = ship_with(&[0, 4, 5, 6, 7]);
        let st = ship_stats(&s, true);
        assert_eq!((st.propulsion(), st.fuel_parts()), (1, 1));
        assert_eq!(st.mass, 2 + 1 + 1 + 4 + 4);
        assert_eq!(st.fuel, 100);
        assert_eq!(st.flight, 50 * 12 / 11);
    }

    #[test]
    fn success_formula() {
        // one habitation module attached at (9,6) through the structural (11,6) east of it
        let s = ship_with(&[2, 37]);
        let st = ship_stats(&s, true);
        assert_eq!(st.habitation(), 1);
        assert_eq!(st.support, 0);
        assert_eq!(st.energy, 0);
        assert_eq!(st.success, 0);
        // habitation + life support + solar: support 100%, energy 100%; no propulsion, so the
        // flight time is 50 * mass and the success drops by (flight - 150) / 10
        let mut s = ship_with(&[2, 37]);
        s.grid[9][4] = part::LIFE_SUPPORT + 1;
        s.grid[9][3] = part::STRUCTURE1 + 1;
        s.grid[8][1] = part::SOLAR_PANEL + 1;
        let st = ship_stats(&s, true);
        assert_eq!((st.life_support(), st.solar()), (1, 1));
        assert_eq!((st.support, st.energy), (100, 100));
        assert_eq!(st.mass, 2 + 1 + 16 + 16 + 4);
        assert_eq!(st.flight, 50 * 39);
        assert_eq!(st.success, 0);
        // the undrawn (AI) ship ignores life support: hab only, solar for two modules
        let mut s = ship_with(&[2, 37]);
        s.grid[8][1] = part::SOLAR_PANEL + 1;
        s.grid[9][3] = part::STRUCTURE1 + 1;
        for (x, y, t) in [(12, 6, part::FUEL), (13, 6, part::PROPULSION), (13, 7, part::STRUCTURE1)] {
            s.grid[x][y] = t + 1;
        }
        let st = ship_stats(&s, false);
        assert_eq!(st.mass, 2 + 1 + 1 + 16 + 4 + 4 + 4);
        assert_eq!((st.propulsion(), st.fuel_parts()), (1, 1));
        assert_eq!(st.energy, 200);
        assert_eq!(st.support, 0);
        assert_eq!(st.flight, 50 * st.mass / 11);
        assert_eq!(st.success, 100 - (st.flight - 150).max(0) / 10);
        assert_eq!(ship_stats(&s, true).success, 0);
    }

    #[test]
    fn human_structural_choice() {
        // empty ship: -1 - 0 against -1 - 33, the propulsion section comes first
        let s = Spaceship::default();
        assert_eq!(human_structural_slot(&s), Some(0));
        // a module placed in the module section pulls the structurals there
        let s = ship_with(&[37]);
        assert_eq!(human_structural_slot(&s), Some(33));
        let s = ship_with(&[7]);
        assert_eq!(human_structural_slot(&s), Some(0));
        assert_eq!(first_free_slot(&s, part::PROPULSION), Some(11));
        assert_eq!(first_free_slot(&s, part::HABITATION), Some(37));
    }

    #[test]
    fn ai_scan() {
        let s = Spaceship::default();
        assert_eq!(ai_pick(&s, 0, 0, true), AiPick::Place(0));
        assert_eq!(ai_pick(&s, 1, 0, true), AiPick::Place(6));
        assert_eq!(ai_pick(&s, 2, MODULE_SECTION, true), AiPick::Place(37));
        // structurals 0..5, 8, 9 placed: the next one (16) lies 10 entries past the free fuel
        // slot 6, so the AI builds a component instead when it knows Plastics
        let s = ship_with(&[0, 1, 2, 3, 4, 5, 8, 9]);
        assert_eq!(ai_pick(&s, 0, 0, true), AiPick::Refuse);
        assert_eq!(ai_pick(&s, 0, 0, false), AiPick::Place(16));
    }

    #[test]
    fn build_launch_arrive() {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        g.apollo_built = true;
        assert!(!g.spaceship_can_launch(slot));
        for _ in 0..3 {
            g.spaceship_part_built(c, 0);
        }
        assert_eq!(g.player(slot).spaceship.structurals, 3);
        assert!(!g.spaceship_can_launch(slot));
        g.place_spaceship_part(slot, 2, 0);
        assert_eq!(g.player(slot).spaceship.modules, 1);
        assert!(g.spaceship_can_launch(slot));
        for choice in [0, 1] {
            for _ in 0..2 {
                g.place_spaceship_part(slot, 1, choice);
            }
        }
        let st = g.spaceship_stats(slot);
        // 1770 AD: two years a turn until a ship is in flight, then one
        g.set_turn(360);
        assert_eq!((g.year(), g.next_year()), (1770, 1772));
        g.launch_spaceship(slot).unwrap();
        let s = g.player(slot).spaceship;
        assert_eq!((s.launch_year, s.launched_turn), (Some(1770), Some(360)));
        let arrival = 1770 + st.flight / 10;
        assert!(arrival > 1771);
        assert_eq!(s.arrival_year, Some(arrival));
        assert_eq!(g.spaceship_score(slot), 0);
        // the year the turn loop compares is the coming one (0x2a13 after 0x26ed)
        while g.next_year() < arrival {
            g.spaceship_arrivals();
            assert_eq!(g.ended, None);
            let y = g.year();
            g.advance_calendar();
            assert_eq!(g.year(), y + 1);
        }
        g.spaceship_arrivals();
        assert_eq!(g.ended, Some(EndReason::Spaceship(slot)));
        assert_eq!(g.spaceship_score(slot), st.success * st.habitation() / 2);
    }

    #[test]
    fn early_ship_misses_its_year() {
        // launched in 1600 AD the calendar still goes in steps of 5: an arrival year between
        // two steps never comes (CIV.EXE compares for equality)
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        g.apollo_built = true;
        g.set_turn(320);
        assert_eq!(g.year(), 1600);
        let s = &mut g.player_mut(slot).spaceship;
        *s = ship_with(&[2, 37]);
        s.recount();
        g.launch_spaceship(slot).unwrap();
        g.player_mut(slot).spaceship.arrival_year = Some(1612);
        for _ in 0..60 {
            g.spaceship_arrivals();
            g.advance_calendar();
        }
        assert_eq!(g.ended, None);
        assert!(!g.player(slot).spaceship.landed);
        assert_eq!(g.year(), 1750 + 30);
    }

    #[test]
    fn ai_builds_refunds_and_launches() {
        use crate::game::science::AdvanceSource;
        use crate::rules::advance_ids as adv;
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        let ai = (1..8u8).find(|s| *s != g.human_slot && g.player(*s).alive && !g.units_of(*s).is_empty()).unwrap();
        let settler = g.units_of(ai).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        g.apollo_built = true;
        for a in [adv::SPACE_FLIGHT, adv::PLASTICS, adv::ROBOTICS] {
            g.grant_advance(ai, a, AdvanceSource::Discovered);
        }
        // a structural refused because a component slot lies 4+ entries back: shields return
        // and the city switches to a component
        for i in [0, 1, 2, 3, 4, 5, 8, 9] {
            place(&mut g.player_mut(ai).spaceship, i);
        }
        g.city_mut(c).production = Production::Improvement(imp::SS_STRUCTURAL);
        g.city_mut(c).shields = 0;
        g.spaceship_part_built(c, 0);
        assert_eq!(g.city(c).shields, g.production_cost(c, Production::Improvement(imp::SS_STRUCTURAL)));
        assert_eq!(g.city(c).production, Production::Improvement(imp::SS_COMPONENT));
        assert_eq!(g.player(ai).spaceship.structurals, 8);
        // keep building what the ship takes: the AI launches at some point
        for _ in 0..200 {
            if g.player(ai).spaceship.launched() {
                break;
            }
            let part = g.ai_spaceship_preference(ai)[0];
            g.city_mut(c).production = Production::Improvement(part);
            g.spaceship_part_built(c, part - imp::SS_STRUCTURAL);
        }
        let s = g.player(ai).spaceship;
        assert!(s.launched());
        assert!(s.modules > 0 && s.components > 0);
        assert!(g.take_events().iter().any(|e| matches!(e, GameEvent::SpaceshipLaunched(x) if *x == ai)));
    }

    #[test]
    fn capital_loss_cancels_the_ship() {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        g.apollo_built = true;
        g.place_spaceship_part(slot, 0, 0);
        g.cancel_spaceship(slot);
        let s = g.player(slot).spaceship;
        assert_eq!((s.parts(), s.lost), (0, true));
        assert!(g.take_events().iter().any(|e| matches!(e, GameEvent::SpaceshipCancelled { launched: false, .. })));
    }

    #[test]
    fn old_counts_rebuild() {
        let mut s = Spaceship { structurals: 5, components: 2, modules: 3, ..Spaceship::default() };
        grid_from_counts(&mut s);
        assert_eq!((s.structurals, s.components, s.modules), (5, 2, 3));
        let st = ship_stats(&s, true);
        assert_eq!(st.habitation() + st.life_support() + st.solar() <= 3, true);
    }
}
