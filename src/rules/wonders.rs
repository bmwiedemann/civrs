//! Wonders of the World (22 slots; 0 unused) as decoded from CIV.EXE.
//!
//! BLURB1 has `*NAME` (history) and `*NAME2` (effect) sections per wonder.

use super::advance_ids as adv;
use super::{AdvanceId, WonderId};

/// Static description of one wonder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Wonder {
    pub name: &'static str,
    /// Production cost in rows of 10 shields.
    pub cost: u8,
    pub prereq: Option<AdvanceId>,
    /// Advance that cancels the wonder's effect once any civilisation learns it.
    pub obsolete_by: Option<AdvanceId>,
}

/// Index constants for every wonder in [`WONDERS`].
pub mod wonder_ids {
    pub const PYRAMIDS: u8 = 1;
    pub const HANGING_GARDENS: u8 = 2;
    pub const COLOSSUS: u8 = 3;
    pub const LIGHTHOUSE: u8 = 4;
    pub const GREAT_LIBRARY: u8 = 5;
    pub const ORACLE: u8 = 6;
    pub const GREAT_WALL: u8 = 7;
    pub const MAGELLANS_EXPEDITION: u8 = 8;
    pub const MICHELANGELOS_CHAPEL: u8 = 9;
    pub const COPERNICUS_OBSERVATORY: u8 = 10;
    pub const SHAKESPEARES_THEATRE: u8 = 11;
    pub const ISAAC_NEWTONS_COLLEGE: u8 = 12;
    pub const JS_BACHS_CATHEDRAL: u8 = 13;
    pub const DARWINS_VOYAGE: u8 = 14;
    pub const HOOVER_DAM: u8 = 15;
    pub const WOMENS_SUFFRAGE: u8 = 16;
    pub const MANHATTAN_PROJECT: u8 = 17;
    pub const UNITED_NATIONS: u8 = 18;
    pub const APOLLO_PROGRAM: u8 = 19;
    pub const SETI_PROGRAM: u8 = 20;
    pub const CURE_FOR_CANCER: u8 = 21;
}

const fn wonder(
    name: &'static str,
    cost: u8,
    prereq: Option<AdvanceId>,
    obsolete_by: Option<AdvanceId>,
) -> Wonder {
    Wonder {
        name,
        cost,
        prereq,
        obsolete_by,
    }
}

/// The 22 wonder slots, in CIV.EXE order (slot 0 is unused).
pub static WONDERS: [Wonder; 22] = [
    wonder("", 0, None, None),                                        // 0
    wonder("Pyramids", 30, Some(adv::MASONRY), Some(adv::COMMUNISM)), // 1
    wonder(
        "Hanging Gardens",
        30,
        Some(adv::POTTERY),
        Some(adv::INVENTION),
    ), // 2
    wonder(
        "Colossus",
        20,
        Some(adv::BRONZE_WORKING),
        Some(adv::ELECTRICITY),
    ), // 3
    wonder(
        "Lighthouse",
        20,
        Some(adv::MAP_MAKING),
        Some(adv::MAGNETISM),
    ), // 4
    wonder(
        "Great Library",
        30,
        Some(adv::LITERACY),
        Some(adv::UNIVERSITY),
    ), // 5
    wonder("Oracle", 30, Some(adv::MYSTICISM), Some(adv::RELIGION)),  // 6
    wonder("Great Wall", 30, Some(adv::MASONRY), Some(adv::GUNPOWDER)), // 7
    wonder("Magellan's Expedition", 40, Some(adv::NAVIGATION), None), // 8
    wonder(
        "Michelangelo's Chapel",
        30,
        Some(adv::RELIGION),
        Some(adv::COMMUNISM),
    ), // 9
    wonder("Copernicus' Observatory", 30, Some(adv::ASTRONOMY), None), // 10
    wonder(
        "Shakespeare's Theatre",
        40,
        Some(adv::MEDICINE),
        Some(adv::ELECTRONICS),
    ), // 11
    wonder(
        "Isaac Newton's College",
        40,
        Some(adv::THEORY_OF_GRAVITY),
        Some(adv::NUCLEAR_FISSION),
    ), // 12
    wonder("J.S.Bach's Cathedral", 40, Some(adv::RELIGION), None),    // 13
    wonder("Darwin's Voyage", 30, Some(adv::RAILROAD), None),         // 14
    wonder("Hoover Dam", 60, Some(adv::ELECTRONICS), None),           // 15
    wonder("Women's Suffrage", 60, Some(adv::INDUSTRIALIZATION), None), // 16
    wonder("Manhattan Project", 60, Some(adv::NUCLEAR_FISSION), None), // 17
    wonder("United Nations", 60, Some(adv::COMMUNISM), None),         // 18
    wonder("Apollo Program", 60, Some(adv::SPACE_FLIGHT), None),      // 19
    wonder("SETI Program", 60, Some(adv::COMPUTERS), None),           // 20
    wonder("Cure for Cancer", 60, Some(adv::GENETIC_ENGINEERING), None), // 21
];

/// Look up a wonder by its exact name (the unused slot 0 never matches).
pub fn wonder_by_name(name: &str) -> Option<WonderId> {
    if name.is_empty() {
        return None;
    }
    WONDERS
        .iter()
        .position(|w| w.name == name)
        .map(|i| i as WonderId)
}
