//! Unit types (28 entries) as decoded from CIV.EXE.
//!
//! Sprite index in SP257: unit `i` is at `x = (i % 20) * 16`,
//! `y = 160 + (i / 20) * 16` (same order as this table).

use super::advance_ids as adv;
use super::{AdvanceId, UnitTypeId};

/// Movement domain of a unit.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Domain {
    Land = 0,
    Air = 1,
    Sea = 2,
}

impl Domain {
    pub const ALL: [Domain; 3] = [Domain::Land, Domain::Air, Domain::Sea];

    pub fn from_u8(v: u8) -> Option<Domain> {
        Self::ALL.get(v as usize).copied()
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn name(self) -> &'static str {
        match self {
            Domain::Land => "Land",
            Domain::Air => "Air",
            Domain::Sea => "Sea",
        }
    }
}

/// Role the AI assigns to a unit type when choosing production and orders.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AiRole {
    Settle = 0,
    Attack = 1,
    Defend = 2,
    SeaAttack = 3,
    Air = 4,
    Transport = 5,
    Civilian = 6,
}

impl AiRole {
    pub const ALL: [AiRole; 7] = [
        AiRole::Settle,
        AiRole::Attack,
        AiRole::Defend,
        AiRole::SeaAttack,
        AiRole::Air,
        AiRole::Transport,
        AiRole::Civilian,
    ];

    pub fn from_u8(v: u8) -> Option<AiRole> {
        Self::ALL.get(v as usize).copied()
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn name(self) -> &'static str {
        match self {
            AiRole::Settle => "Settle",
            AiRole::Attack => "Attack",
            AiRole::Defend => "Defend",
            AiRole::SeaAttack => "SeaAttack",
            AiRole::Air => "Air",
            AiRole::Transport => "Transport",
            AiRole::Civilian => "Civilian",
        }
    }
}

/// Static description of one unit type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct UnitType {
    pub name: &'static str,
    pub domain: Domain,
    /// Movement points per turn.
    pub moves: u8,
    /// Fuel range in turns for air units (0 for land/sea).
    pub range: u8,
    pub att: u8,
    pub def: u8,
    /// Production cost in rows of 10 shields.
    pub cost: u8,
    /// Sight flags as in CIV.EXE's table: bit 1 (value 2) = sees two squares far, for ships
    /// only over ocean (CIV.EXE 0x80a1); bit 0 is not used for sight.
    pub sight: u8,
    /// Number of units carried (transports / carriers).
    pub capacity: u8,
    pub role: AiRole,
    pub prereq: Option<AdvanceId>,
    pub obsolete_by: Option<AdvanceId>,
}

/// Index constants for every unit type in [`UNITS`].
pub mod unit_ids {
    pub const SETTLERS: u8 = 0;
    pub const MILITIA: u8 = 1;
    pub const PHALANX: u8 = 2;
    pub const LEGION: u8 = 3;
    pub const MUSKETEERS: u8 = 4;
    pub const RIFLEMEN: u8 = 5;
    pub const CAVALRY: u8 = 6;
    pub const KNIGHTS: u8 = 7;
    pub const CATAPULT: u8 = 8;
    pub const CANNON: u8 = 9;
    pub const CHARIOT: u8 = 10;
    pub const ARMOR: u8 = 11;
    pub const MECH_INF: u8 = 12;
    pub const ARTILLERY: u8 = 13;
    pub const FIGHTER: u8 = 14;
    pub const BOMBER: u8 = 15;
    pub const TRIREME: u8 = 16;
    pub const SAIL: u8 = 17;
    pub const FRIGATE: u8 = 18;
    pub const IRONCLAD: u8 = 19;
    pub const CRUISER: u8 = 20;
    pub const BATTLESHIP: u8 = 21;
    pub const SUBMARINE: u8 = 22;
    pub const CARRIER: u8 = 23;
    pub const TRANSPORT: u8 = 24;
    pub const NUCLEAR: u8 = 25;
    pub const DIPLOMAT: u8 = 26;
    pub const CARAVAN: u8 = 27;
}

#[allow(clippy::too_many_arguments)]
const fn unit(
    name: &'static str,
    domain: Domain,
    moves: u8,
    range: u8,
    att: u8,
    def: u8,
    cost: u8,
    sight: u8,
    capacity: u8,
    role: AiRole,
    prereq: Option<AdvanceId>,
    obsolete_by: Option<AdvanceId>,
) -> UnitType {
    UnitType {
        name,
        domain,
        moves,
        range,
        att,
        def,
        cost,
        sight,
        capacity,
        role,
        prereq,
        obsolete_by,
    }
}

use AiRole::{Air, Attack, Civilian, Defend, SeaAttack, Settle, Transport};
use Domain::{Air as DAir, Land, Sea};

/// The 28 unit types, in CIV.EXE order.
#[rustfmt::skip]
pub static UNITS: [UnitType; 28] = [
    //   name            domain moves range att  def cost sight cap  role       prereq                        obsolete_by
    unit("Settlers", Land, 1, 0, 0, 1, 4, 0, 0, Settle, None, None),
    unit("Militia", Land, 1, 0, 1, 1, 1, 0, 0, Defend, None, Some(adv::GUNPOWDER)),
    unit("Phalanx", Land, 1, 0, 1, 2, 2, 0, 0, Defend, Some(adv::BRONZE_WORKING), Some(adv::GUNPOWDER)),
    unit("Legion", Land, 1, 0, 3, 1, 2, 0, 0, Attack, Some(adv::IRON_WORKING), Some(adv::CONSCRIPTION)),
    unit("Musketeers", Land, 1, 0, 2, 3, 3, 0, 0, Defend, Some(adv::GUNPOWDER), Some(adv::CONSCRIPTION)),
    unit("Riflemen", Land, 1, 0, 3, 5, 3, 0, 0, Defend, Some(adv::CONSCRIPTION), None),
    unit("Cavalry", Land, 2, 0, 2, 1, 2, 0, 0, Attack, Some(adv::HORSEBACK_RIDING), Some(adv::CONSCRIPTION)),
    unit("Knights", Land, 2, 0, 4, 2, 4, 0, 0, Attack, Some(adv::CHIVALRY), Some(adv::AUTOMOBILE)),
    unit("Catapult", Land, 1, 0, 6, 1, 4, 0, 0, Attack, Some(adv::MATHEMATICS), Some(adv::METALLURGY)),
    unit("Cannon", Land, 1, 0, 8, 1, 4, 0, 0, Attack, Some(adv::METALLURGY), Some(adv::ROBOTICS)),
    unit("Chariot", Land, 2, 0, 4, 1, 4, 0, 0, Attack, Some(adv::THE_WHEEL), Some(adv::CHIVALRY)),
    unit("Armor", Land, 3, 0, 10, 5, 8, 0, 0, Attack, Some(adv::AUTOMOBILE), None),
    unit("Mech. Inf.", Land, 3, 0, 6, 6, 5, 0, 0, Defend, Some(adv::LABOR_UNION), None),
    unit("Artillery", Land, 2, 0, 12, 2, 6, 0, 0, Attack, Some(adv::ROBOTICS), None),
    unit("Fighter", DAir, 10, 1, 4, 2, 6, 2, 0, Air, Some(adv::FLIGHT), None),
    unit("Bomber", DAir, 8, 2, 12, 1, 12, 2, 0, Attack, Some(adv::ADVANCED_FLIGHT), None),
    unit("Trireme", Sea, 3, 0, 1, 0, 4, 0, 2, Transport, Some(adv::MAP_MAKING), Some(adv::NAVIGATION)),
    unit("Sail", Sea, 3, 0, 1, 1, 4, 0, 3, Transport, Some(adv::NAVIGATION), Some(adv::MAGNETISM)),
    unit("Frigate", Sea, 3, 0, 2, 2, 4, 0, 4, Transport, Some(adv::MAGNETISM), Some(adv::INDUSTRIALIZATION)),
    unit("Ironclad", Sea, 4, 0, 4, 4, 6, 0, 0, SeaAttack, Some(adv::STEAM_ENGINE), Some(adv::COMBUSTION)),
    unit("Cruiser", Sea, 6, 0, 6, 6, 8, 3, 0, SeaAttack, Some(adv::COMBUSTION), None),
    unit("Battleship", Sea, 4, 0, 18, 12, 16, 3, 0, SeaAttack, Some(adv::STEEL), None),
    unit("Submarine", Sea, 3, 0, 8, 2, 5, 3, 0, SeaAttack, Some(adv::MASS_PRODUCTION), None),
    // Raw table capacity is 0; the game code lets a Carrier carry 8 air units.
    unit("Carrier", Sea, 5, 0, 1, 12, 16, 3, 8, SeaAttack, Some(adv::ADVANCED_FLIGHT), None),
    unit("Transport", Sea, 4, 0, 0, 3, 5, 0, 8, Transport, Some(adv::INDUSTRIALIZATION), None),
    unit("Nuclear", DAir, 16, 1, 99, 0, 16, 0, 0, Attack, Some(adv::ROCKETRY), None),
    unit("Diplomat", Land, 2, 0, 0, 0, 3, 0, 0, Civilian, Some(adv::WRITING), None),
    unit("Caravan", Land, 1, 0, 0, 1, 5, 0, 0, Civilian, Some(adv::TRADE), None),
];

/// Look up a unit type by its exact name.
pub fn unit_by_name(name: &str) -> Option<UnitTypeId> {
    UNITS
        .iter()
        .position(|u| u.name == name)
        .map(|i| i as UnitTypeId)
}
