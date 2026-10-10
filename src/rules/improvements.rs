//! City improvements (25 entries; 0 = none) as decoded from CIV.EXE.

use super::advance_ids as adv;
use super::{AdvanceId, ImprovementId};

/// Static description of one city improvement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Improvement {
    pub name: &'static str,
    /// Production cost in rows of 10 shields.
    pub cost: u8,
    /// Gold per turn.
    pub upkeep: u8,
    pub prereq: Option<AdvanceId>,
}

/// Index constants for every improvement in [`IMPROVEMENTS`].
pub mod improvement_ids {
    pub const NONE: u8 = 0;
    pub const PALACE: u8 = 1;
    pub const BARRACKS: u8 = 2;
    pub const GRANARY: u8 = 3;
    pub const TEMPLE: u8 = 4;
    pub const MARKETPLACE: u8 = 5;
    pub const LIBRARY: u8 = 6;
    pub const COURTHOUSE: u8 = 7;
    pub const CITY_WALLS: u8 = 8;
    pub const AQUEDUCT: u8 = 9;
    pub const BANK: u8 = 10;
    pub const CATHEDRAL: u8 = 11;
    pub const UNIVERSITY: u8 = 12;
    pub const MASS_TRANSIT: u8 = 13;
    pub const COLOSSEUM: u8 = 14;
    pub const FACTORY: u8 = 15;
    pub const MFG_PLANT: u8 = 16;
    pub const SDI_DEFENSE: u8 = 17;
    pub const RECYCLING_CNTR: u8 = 18;
    pub const POWER_PLANT: u8 = 19;
    pub const HYDRO_PLANT: u8 = 20;
    pub const NUCLEAR_PLANT: u8 = 21;
    pub const SS_STRUCTURAL: u8 = 22;
    pub const SS_COMPONENT: u8 = 23;
    pub const SS_MODULE: u8 = 24;
}

const fn imp(name: &'static str, cost: u8, upkeep: u8, prereq: Option<AdvanceId>) -> Improvement {
    Improvement {
        name,
        cost,
        upkeep,
        prereq,
    }
}

/// The 25 improvement slots, in CIV.EXE order (0 is the "nothing" entry).
pub static IMPROVEMENTS: [Improvement; 25] = [
    imp("NONE", 0, 0, None), // 0
    // Raw table upkeep is 5; the game charges no upkeep for the Palace.
    imp("Palace", 20, 0, Some(adv::MASONRY)), // 1
    // Raw table upkeep is 0; in-game (and per the manual) Barracks cost 1.
    imp("Barracks", 4, 1, None),                            // 2
    imp("Granary", 6, 1, Some(adv::POTTERY)),               // 3
    imp("Temple", 4, 1, Some(adv::CEREMONIAL_BURIAL)),      // 4
    imp("MarketPlace", 8, 1, Some(adv::CURRENCY)),          // 5
    imp("Library", 8, 1, Some(adv::WRITING)),               // 6
    imp("Courthouse", 8, 1, Some(adv::CODE_OF_LAWS)),       // 7
    imp("City Walls", 12, 2, Some(adv::MASONRY)),           // 8
    imp("Aqueduct", 12, 2, Some(adv::CONSTRUCTION)),        // 9
    imp("Bank", 12, 3, Some(adv::BANKING)),                 // 10
    imp("Cathedral", 16, 3, Some(adv::RELIGION)),           // 11
    imp("University", 16, 3, Some(adv::UNIVERSITY)),        // 12
    imp("Mass Transit", 16, 4, Some(adv::MASS_PRODUCTION)), // 13
    imp("Colosseum", 10, 4, Some(adv::CONSTRUCTION)),       // 14
    imp("Factory", 20, 4, Some(adv::INDUSTRIALIZATION)),    // 15
    imp("Mfg. Plant", 32, 6, Some(adv::ROBOTICS)),          // 16
    imp("SDI Defense", 20, 4, Some(adv::SUPERCONDUCTOR)),   // 17
    imp("Recycling Cntr.", 20, 2, Some(adv::RECYCLING)),    // 18
    imp("Power Plant", 16, 4, Some(adv::REFINING)),         // 19
    imp("Hydro Plant", 24, 4, Some(adv::ELECTRONICS)),      // 20
    imp("Nuclear Plant", 16, 2, Some(adv::NUCLEAR_POWER)),  // 21
    imp("SS Structural", 8, 0, Some(adv::SPACE_FLIGHT)),    // 22
    imp("SS Component", 16, 0, Some(adv::PLASTICS)),        // 23
    imp("SS Module", 32, 0, Some(adv::ROBOTICS)),           // 24
];

/// Look up an improvement by its exact name (the "NONE" slot never matches).
pub fn improvement_by_name(name: &str) -> Option<ImprovementId> {
    IMPROVEMENTS
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, i)| i.name == name)
        .map(|(i, _)| i as ImprovementId)
}
