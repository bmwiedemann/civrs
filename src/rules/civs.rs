//! Civilizations (16 slots), player-slot colours and slot mapping, as
//! decoded from CIV.EXE.
//!
//! Index = civ id.  0 is the Barbarians, 8 is unused; player slot `s`
//! (0..7) is occupied by civ `s` or civ `s + 8`, which share a colour.

use super::city_names as names;

/// Static description of one civilization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Civ {
    pub leader: &'static str,
    pub adjective: &'static str,
    pub plural: &'static str,
    /// AI trait, -1..1.
    pub aggressive: i8,
    /// AI trait, -1..1.
    pub expansionist: i8,
    /// AI trait, -1..1.
    pub civilized: i8,
    pub cities: [&'static str; 16],
    /// Leader uses the female government titles.
    pub female: bool,
}

#[allow(clippy::too_many_arguments)]
const fn civ(
    leader: &'static str,
    adjective: &'static str,
    plural: &'static str,
    aggressive: i8,
    expansionist: i8,
    civilized: i8,
    cities: [&'static str; 16],
    female: bool,
) -> Civ {
    Civ {
        leader,
        adjective,
        plural,
        aggressive,
        expansionist,
        civilized,
        cities,
        female,
    }
}

/// The 16 civilization slots.
#[rustfmt::skip]
pub static CIVS: [Civ; 16] = [
    // Barbarians have no trait entry in the table; traits are unused for them.
    civ("Attila", "Barbarian", "Barbarians", 0, 0, 0, names::SPARE_FIRST, false), // 0
    civ("Caesar", "Roman", "Romans", 0, 1, 1, names::ROMAN, false), // 1
    civ("Hammurabi", "Babylonian", "Babylonians", -1, -1, 1, names::BABYLONIAN, false), // 2
    civ("Frederick", "German", "Germans", 1, -1, 1, names::GERMAN, false), // 3
    civ("Ramesses", "Egyptian", "Egyptians", 0, 0, 1, names::EGYPTIAN, false), // 4
    civ("Abe Lincoln", "American", "Americans", -1, 0, 1, names::AMERICAN, false), // 5
    civ("Alexander", "Greek", "Greeks", 0, 1, -1, names::GREEK, false), // 6
    civ("M.Gandhi", "Indian", "Indians", -1, -1, 0, names::INDIAN, false), // 7
    civ("", "", "", 0, 0, 0, names::SPARE_SECOND, false),           // 8 (unused)
    civ("Stalin", "Russian", "Russians", 1, 0, -1, names::RUSSIAN, false), // 9
    civ("Shaka", "Zulu", "Zulus", 1, 0, 0, names::ZULU, false),     // 10
    civ("Napoleon", "French", "French", 1, 1, 1, names::FRENCH, false), // 11
    civ("Montezuma", "Aztec", "Aztecs", 0, -1, 1, names::AZTEC, false), // 12
    civ("Mao Tse Tung", "Chinese", "Chinese", 0, 0, 1, names::CHINESE, false), // 13
    civ("Elizabeth I", "English", "English", 0, 1, 0, names::ENGLISH, true), // 14
    civ("Genghis Khan", "Mongol", "Mongols", 1, 1, -1, names::MONGOL, false), // 15
];

/// Palette indices used to recolour unit/city sprites for a player slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct SlotColour {
    /// Main colour (replaces placeholder index 10 in SP257).
    pub light: u8,
    /// Secondary / shadow colour (replaces placeholder index 2).
    pub dark: u8,
}

/// Colours per player slot: red (Barbarians), white, green, blue, yellow,
/// cyan, magenta, grey.
pub static SLOT_COLOURS: [SlotColour; 8] = [
    SlotColour { light: 12, dark: 4 }, // red (slot 0: Barbarians / unused civ 8)
    SlotColour { light: 15, dark: 7 }, // white
    SlotColour { light: 10, dark: 2 }, // green
    SlotColour { light: 9, dark: 1 },  // blue
    SlotColour {
        light: 14,
        dark: 10,
    }, // yellow
    SlotColour { light: 11, dark: 3 }, // cyan
    SlotColour { light: 13, dark: 4 }, // magenta
    SlotColour { light: 7, dark: 8 },  // grey
];

/// The two civilizations that can occupy player slot `slot` (0..7).
pub fn civs_for_slot(slot: u8) -> [u8; 2] {
    let s = slot & 7;
    [s, s + 8]
}

/// Player slot (0..7) of civilization `civ` (0..15).
pub fn slot_for_civ(civ: u8) -> u8 {
    civ & 7
}
