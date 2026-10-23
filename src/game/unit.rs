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
    /// Ship carrying this unit (land units at sea / aircraft on carriers).
    pub carried_by: Option<UnitId>,
    /// Barbarian leader flag (uses the Diplomat sprite).
    pub leader: bool,
    /// Turn the unit was created (for AI bookkeeping / barbarian expiry).
    pub created_turn: u32,
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
            carried_by: None,
            leader: false,
            created_turn: turn,
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
    /// A ship land units can board (the Carrier only takes air units).
    pub fn carries_land_units(&self) -> bool {
        self.is_sea() && self.info().capacity > 0 && self.kind != crate::rules::unit_ids::CARRIER
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
