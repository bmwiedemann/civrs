//! Civilization advances (72 entries) as decoded from CIV.EXE.
//!
//! Indices 67..=70 are unused placeholders (empty name) and 71 is
//! "Future Tech.".  Prerequisite indices refer back into this table.

use super::AdvanceId;

/// Static description of one advance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Advance {
    pub name: &'static str,
    /// Up to two prerequisite advances.
    pub prereq: [Option<AdvanceId>; 2],
    /// AI research interest (2..8 in the original table).
    pub ai_interest: u8,
    /// AI lean by "civilized" trait (-2..2 in the original table).
    pub ai_lean: i8,
}

/// Index constants for the 67 real advances plus `FUTURE_TECH`.
pub mod advance_ids {
    pub const ALPHABET: u8 = 0;
    pub const CODE_OF_LAWS: u8 = 1;
    pub const CURRENCY: u8 = 2;
    pub const ATOMIC_THEORY: u8 = 3;
    pub const DEMOCRACY: u8 = 4;
    pub const MONARCHY: u8 = 5;
    pub const ASTRONOMY: u8 = 6;
    pub const MAP_MAKING: u8 = 7;
    pub const NAVIGATION: u8 = 8;
    pub const MATHEMATICS: u8 = 9;
    pub const MEDICINE: u8 = 10;
    pub const PHYSICS: u8 = 11;
    pub const ENGINEERING: u8 = 12;
    pub const UNIVERSITY: u8 = 13;
    pub const MAGNETISM: u8 = 14;
    pub const ELECTRONICS: u8 = 15;
    pub const MASONRY: u8 = 16;
    pub const BRONZE_WORKING: u8 = 17;
    pub const IRON_WORKING: u8 = 18;
    pub const BRIDGE_BUILDING: u8 = 19;
    pub const INVENTION: u8 = 20;
    pub const COMPUTERS: u8 = 21;
    pub const WRITING: u8 = 22;
    pub const STEAM_ENGINE: u8 = 23;
    pub const TRADE: u8 = 24;
    pub const CEREMONIAL_BURIAL: u8 = 25;
    pub const MYSTICISM: u8 = 26;
    pub const NUCLEAR_FISSION: u8 = 27;
    pub const PHILOSOPHY: u8 = 28;
    pub const RELIGION: u8 = 29;
    pub const LITERACY: u8 = 30;
    pub const HORSEBACK_RIDING: u8 = 31;
    pub const FEUDALISM: u8 = 32;
    pub const THE_WHEEL: u8 = 33;
    pub const GUNPOWDER: u8 = 34;
    pub const INDUSTRIALIZATION: u8 = 35;
    pub const CHEMISTRY: u8 = 36;
    pub const COMBUSTION: u8 = 37;
    pub const FLIGHT: u8 = 38;
    pub const ADVANCED_FLIGHT: u8 = 39;
    pub const SPACE_FLIGHT: u8 = 40;
    pub const MASS_PRODUCTION: u8 = 41;
    pub const POTTERY: u8 = 42;
    pub const COMMUNISM: u8 = 43;
    pub const THE_REPUBLIC: u8 = 44;
    pub const CONSTRUCTION: u8 = 45;
    pub const ROCKETRY: u8 = 46;
    pub const THE_CORPORATION: u8 = 47;
    pub const METALLURGY: u8 = 48;
    pub const RAILROAD: u8 = 49;
    pub const NUCLEAR_POWER: u8 = 50;
    pub const THEORY_OF_GRAVITY: u8 = 51;
    pub const STEEL: u8 = 52;
    pub const BANKING: u8 = 53;
    pub const ELECTRICITY: u8 = 54;
    pub const REFINING: u8 = 55;
    pub const EXPLOSIVES: u8 = 56;
    pub const SUPERCONDUCTOR: u8 = 57;
    pub const AUTOMOBILE: u8 = 58;
    pub const GENETIC_ENGINEERING: u8 = 59;
    pub const PLASTICS: u8 = 60;
    pub const RECYCLING: u8 = 61;
    pub const CHIVALRY: u8 = 62;
    pub const ROBOTICS: u8 = 63;
    pub const CONSCRIPTION: u8 = 64;
    pub const LABOR_UNION: u8 = 65;
    pub const FUSION_POWER: u8 = 66;
    pub const FUTURE_TECH: u8 = 71;
}

/// Number of entries in [`ADVANCES`].
pub const ADVANCE_COUNT: usize = 72;
/// Number of real (researchable, named) advances excluding Future Tech.
pub const REAL_ADVANCE_COUNT: usize = 67;

// The per-tech AI weights (interest 2..8, civilized lean -2..2) were not
// transcribed from the executable; neutral defaults are used until they are.
const DEFAULT_INTEREST: u8 = 4;
const DEFAULT_LEAN: i8 = 0;

const fn adv(name: &'static str, p1: Option<AdvanceId>, p2: Option<AdvanceId>) -> Advance {
    Advance {
        name,
        prereq: [p1, p2],
        ai_interest: DEFAULT_INTEREST,
        ai_lean: DEFAULT_LEAN,
    }
}

const fn placeholder() -> Advance {
    Advance {
        name: "",
        prereq: [None, None],
        ai_interest: 0,
        ai_lean: 0,
    }
}

/// The 72 advances, in CIV.EXE order.
pub static ADVANCES: [Advance; ADVANCE_COUNT] = [
    adv("Alphabet", None, None),                    // 0
    adv("Code of Laws", Some(0), None),             // 1
    adv("Currency", Some(17), None),                // 2
    adv("Atomic Theory", Some(51), Some(11)),       // 3
    adv("Democracy", Some(28), Some(30)),           // 4
    adv("Monarchy", Some(25), Some(1)),             // 5
    adv("Astronomy", Some(26), Some(9)),            // 6
    adv("MapMaking", Some(0), None),                // 7
    adv("Navigation", Some(7), Some(6)),            // 8
    adv("Mathematics", Some(0), Some(16)),          // 9
    adv("Medicine", Some(28), Some(24)),            // 10
    adv("Physics", Some(9), Some(8)),               // 11
    adv("Engineering", Some(33), Some(45)),         // 12
    adv("University", Some(9), Some(28)),           // 13
    adv("Magnetism", Some(8), Some(11)),            // 14
    adv("Electronics", Some(12), Some(54)),         // 15
    adv("Masonry", None, None),                     // 16
    adv("Bronze Working", None, None),              // 17
    adv("Iron Working", Some(17), None),            // 18
    adv("Bridge Building", Some(18), Some(45)),     // 19
    adv("Invention", Some(12), Some(30)),           // 20
    adv("Computers", Some(9), Some(15)),            // 21
    adv("Writing", Some(0), None),                  // 22
    adv("Steam Engine", Some(11), Some(20)),        // 23
    adv("Trade", Some(2), Some(1)),                 // 24
    adv("Ceremonial Burial", None, None),           // 25
    adv("Mysticism", Some(25), None),               // 26
    adv("Nuclear Fission", Some(41), Some(3)),      // 27
    adv("Philosophy", Some(26), Some(30)),          // 28
    adv("Religion", Some(28), Some(22)),            // 29
    adv("Literacy", Some(22), Some(1)),             // 30
    adv("Horseback Riding", None, None),            // 31
    adv("Feudalism", Some(16), Some(5)),            // 32
    adv("The Wheel", None, None),                   // 33
    adv("Gunpowder", Some(20), Some(18)),           // 34
    adv("Industrialization", Some(49), Some(53)),   // 35
    adv("Chemistry", Some(13), Some(10)),           // 36
    adv("Combustion", Some(55), Some(56)),          // 37
    adv("Flight", Some(37), Some(11)),              // 38
    adv("Advanced Flight", Some(38), Some(54)),     // 39
    adv("Space Flight", Some(21), Some(46)),        // 40
    adv("Mass Production", Some(58), Some(47)),     // 41
    adv("Pottery", None, None),                     // 42
    adv("Communism", Some(28), Some(35)),           // 43
    adv("The Republic", Some(1), Some(30)),         // 44
    adv("Construction", Some(16), Some(2)),         // 45
    adv("Rocketry", Some(39), Some(15)),            // 46
    adv("The Corporation", Some(53), Some(35)),     // 47
    adv("Metallurgy", Some(34), Some(13)),          // 48
    adv("RailRoad", Some(23), Some(19)),            // 49
    adv("Nuclear Power", Some(27), Some(15)),       // 50
    adv("Theory of Gravity", Some(6), Some(13)),    // 51
    adv("Steel", Some(48), Some(35)),               // 52
    adv("Banking", Some(24), Some(44)),             // 53
    adv("Electricity", Some(48), Some(14)),         // 54
    adv("Refining", Some(36), Some(47)),            // 55
    adv("Explosives", Some(34), Some(36)),          // 56
    adv("SuperConductor", Some(60), Some(41)),      // 57
    adv("Automobile", Some(37), Some(52)),          // 58
    adv("Genetic Engineering", Some(10), Some(47)), // 59
    adv("Plastics", Some(55), Some(40)),            // 60
    adv("Recycling", Some(41), Some(4)),            // 61
    adv("Chivalry", Some(32), Some(31)),            // 62
    adv("Robotics", Some(60), Some(21)),            // 63
    adv("Conscription", Some(44), Some(56)),        // 64
    adv("Labor Union", Some(41), Some(43)),         // 65
    adv("Fusion Power", Some(50), Some(57)),        // 66
    placeholder(),                                  // 67
    placeholder(),                                  // 68
    placeholder(),                                  // 69
    placeholder(),                                  // 70
    adv("Future Tech.", None, None),                // 71
];

/// True for the 67 named advances and Future Tech; false for the four
/// placeholder slots (67..=70) and any out-of-range id.
pub fn is_real(id: AdvanceId) -> bool {
    let i = id as usize;
    i < REAL_ADVANCE_COUNT || i == advance_ids::FUTURE_TECH as usize
}

/// Look up an advance by its exact name (placeholders never match).
pub fn advance_by_name(name: &str) -> Option<AdvanceId> {
    if name.is_empty() {
        return None;
    }
    ADVANCES
        .iter()
        .position(|a| a.name == name)
        .map(|i| i as AdvanceId)
}

/// Civilopedia section key for an advance: the upper-cased name as used by
/// the `*NAME` section headers in BLURB0 (e.g. "CODE OF LAWS").
pub fn advance_pedia_key(id: AdvanceId) -> String {
    ADVANCES
        .get(id as usize)
        .map(|a| a.name.to_ascii_uppercase())
        .unwrap_or_default()
}
