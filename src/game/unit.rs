use super::map::{CityId, Slot, UnitId};
use crate::rules::{UnitTypeId, UNITS};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum UnitOrder {
    None,
    /// Fortifying (completes at start of next turn).
    Fortify,
    Fortified,
    Sentry,
    Goto,
    Road,
    Irrigate,
    Mine,
    Fortress,
    CleanPollution,
    Pillage,
    /// Skipped for this turn (space).
    Skip,
    /// An aircraft returning to the nearest base (unit +0 bit 0x02 on an air unit, set by
    /// 'h' at 0x4f94): the human's unit loop presses 'h' for it on every move (0x4556) until
    /// it lands (0x6c96, 0x6cea). No status letter (draw_unit 0x1b735 skips aircraft).
    /// (Appended last: bincode numbers the variants, so older saves still load.)
    Home,
}

impl UnitOrder {
    /// Orders that keep the unit from being offered as the active unit.
    pub fn is_standing(self) -> bool {
        !matches!(self, UnitOrder::None)
    }
    /// Short status letter shown in the unit's sprite (F, S, G, R, I, M, ...).
    pub fn letter(self) -> Option<char> {
        match self {
            UnitOrder::Fortified => Some('F'),
            UnitOrder::Fortify => Some('F'),
            UnitOrder::Sentry => Some('S'),
            UnitOrder::Goto => Some('G'),
            UnitOrder::Road => Some('R'),
            UnitOrder::Irrigate => Some('I'),
            UnitOrder::Mine => Some('M'),
            UnitOrder::Fortress => Some('F'),
            UnitOrder::CleanPollution => Some('P'),
            UnitOrder::Pillage => Some('P'),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Unit {
    pub id: UnitId,
    pub owner: Slot,
    pub kind: UnitTypeId,
    pub x: i32,
    pub y: i32,
    /// Movement points left this turn, in thirds.
    pub moves_left: u8,
    pub veteran: bool,
    pub order: UnitOrder,
    /// Settlers work counter (CIV.EXE unit +5): turns of work done on the current job,
    /// the order's own turn included; 0 when idle (see `orders.rs`).
    pub order_turns: u8,
    pub goto: Option<(i32, i32)>,
    pub home: Option<CityId>,
    /// Turns spent in the air since last landing (air units).
    pub fuel: u8,
    /// Unused, always `None`: kept for the layout of older saves. CIV.EXE links no unit to
    /// its ship; a ship's move takes along what its stack holds (`Game::take_cargo`).
    pub legacy_cargo: Option<UnitId>,
    /// Barbarian leader flag (uses the Diplomat sprite).
    pub leader: bool,
    /// Turn the unit was created (for AI bookkeeping / barbarian expiry).
    pub created_turn: u32,
    /// The civilizations that have seen the unit since its last step, a bit per slot (CIV.EXE
    /// unit +9): created with the owner's bit (create_unit 0x8bee), cleared by every step
    /// (0x67e4, cargo 0x69b4) and set again by the sightings of `Game::mark_sightings`
    /// (unit_look 0x8038). A foreign unit is drawn, and its moves and fights are shown, only
    /// to the civilizations whose bit it has. Saved in the version-12 trailer.
    #[serde(skip)]
    pub seen_by: u8,
    /// The unit's record in its owner's table (CIV.EXE ds:0x81d4 + 0x600 civ + 12 record):
    /// create_unit (0x8b51) takes the lowest free one, and every per-civ unit loop, the
    /// human's choice of the next unit among them (move_units 0x4149), runs in this order.
    /// Saved in the version-23 trailer.
    #[serde(skip)]
    pub record: u8,
    /// The next unit of the owner's on the square in the stack ring (CIV.EXE unit +0xa),
    /// `None` for a unit alone there (0xff); see `game::stack`. Saved in the version-28
    /// trailer.
    #[serde(skip)]
    pub next: Option<UnitId>,
    /// The order a sentried unit keeps under its sentry bit (CIV.EXE status bits 0xce besides
    /// bit 0): a ship's move ORs the sentry bit into its passengers' status (take_cargo
    /// `or status,1` 0x69e5), so a Settlers at work keeps its job (0x02 road, 0x40
    /// irrigation, 0x80 mine, 0xc0 fortress, 0x82 pollution) and a fortified unit its
    /// fortification; waking clears bit 0 only (0x85cd, 0x934a, 0x936e) and gives the order
    /// back (`Unit::wake_sentry`). Meaningful only while `order` is `Sentry`. Saved in the
    /// version-30 trailer.
    #[serde(skip)]
    pub sentry_job: Option<UnitOrder>,
}

impl Unit {
    pub fn new(id: UnitId, owner: Slot, kind: UnitTypeId, x: i32, y: i32, turn: u32) -> Unit {
        let t = &UNITS[kind as usize];
        Unit {
            id,
            owner,
            kind,
            x,
            y,
            moves_left: t.moves * 3,
            veteran: false,
            order: UnitOrder::None,
            order_turns: 0,
            goto: None,
            home: None,
            fuel: 0,
            legacy_cargo: None,
            leader: false,
            created_turn: turn,
            seen_by: 1 << owner,
            record: 0,
            next: None,
            sentry_job: None,
        }
    }

    /// Set the sentry bit (`or status,1`) over the unit's order: a Settlers job, and with
    /// `fortify` a fortification too, stays under it (`sentry_job`); a sentried unit keeps
    /// what it kept. Boarding drops the fortify bits first (`and status,0xf3`, 0x5e27).
    pub fn sentry_keeping_job(&mut self, fortify: bool) {
        let o = self.order;
        if o != UnitOrder::Sentry {
            let kept = o.is_work() || (fortify && matches!(o, UnitOrder::Fortify | UnitOrder::Fortified));
            self.sentry_job = kept.then_some(o);
        } else if !fortify && matches!(self.sentry_job, Some(UnitOrder::Fortify | UnitOrder::Fortified)) {
            self.sentry_job = None;
        }
        self.order = UnitOrder::Sentry;
    }

    /// Clear the sentry bit only (`and status,0xfe`, CIV.EXE 0x85cd, 0x934a, 0x936e): the
    /// order kept under it comes back, else a goto it kept (+6, which no wake touches; the
    /// loop steps it before any key, 0x44b3, 0x4bce), else the unit has none.
    pub fn wake_sentry(&mut self) {
        if self.order == UnitOrder::Sentry {
            let goto = if self.goto.is_some() { UnitOrder::Goto } else { UnitOrder::None };
            self.order = self.sentry_job.take().unwrap_or(goto);
        }
    }

    /// The order the status bits stand for but the sentry bit: the kept one of a sentried
    /// unit, else its order.
    pub fn order_under_sentry(&self) -> UnitOrder {
        match (self.order, self.sentry_job) {
            (UnitOrder::Sentry, Some(o)) => o,
            (o, _) => o,
        }
    }

    pub fn info(&self) -> &'static crate::rules::UnitType {
        &UNITS[self.kind as usize]
    }

    pub fn pos(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    pub fn is_land(&self) -> bool {
        self.info().domain == crate::rules::Domain::Land
    }
    pub fn is_sea(&self) -> bool {
        self.info().domain == crate::rules::Domain::Sea
    }
    /// A ship land units can board: one with a capacity (the Carrier has none, it takes air
    /// units).
    pub fn carries_land_units(&self) -> bool {
        self.is_sea() && self.info().capacity > 0
    }
    /// A ship with a capacity or the Carrier (CIV.EXE 0x4e1a, 0x68cb).
    pub fn carries_units(&self) -> bool {
        self.info().capacity > 0 || self.kind == crate::rules::unit_ids::CARRIER
    }
    pub fn is_air(&self) -> bool {
        self.info().domain == crate::rules::Domain::Air
    }
    pub fn is_military(&self) -> bool {
        self.info().att > 0 || (self.info().def > 0 && self.kind != crate::rules::unit_ids::SETTLERS && self.kind != crate::rules::unit_ids::CARAVAN)
    }
    pub fn can_move(&self) -> bool {
        self.moves_left > 0
    }
    pub fn full_moves(&self) -> u8 {
        self.info().moves * 3
    }
    /// Squares the unit sees around it: 2 with the table's sight flag 2 (Fighter, Bomber,
    /// Cruiser, Battleship, Submarine, Carrier), else 1.
    pub fn sight_range(&self) -> i32 {
        if self.info().sight & 2 != 0 {
            2
        } else {
            1
        }
    }
}
