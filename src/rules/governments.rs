//! Government types (6) and their rules table.

use super::advance_ids as adv;
use super::AdvanceId;

/// Government type; the discriminant is the in-game government index.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Govt {
    Anarchy = 0,
    Despotism = 1,
    Monarchy = 2,
    Communism = 3,
    Republic = 4,
    Democracy = 5,
}

impl Govt {
    pub const ALL: [Govt; 6] = [
        Govt::Anarchy,
        Govt::Despotism,
        Govt::Monarchy,
        Govt::Communism,
        Govt::Republic,
        Govt::Democracy,
    ];

    pub fn from_u8(v: u8) -> Option<Govt> {
        Self::ALL.get(v as usize).copied()
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn info(self) -> &'static Government {
        &GOVERNMENTS[self as usize]
    }

    pub fn name(self) -> &'static str {
        self.info().name
    }
}

/// Static description of one government type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Government {
    pub name: &'static str,
    pub adjective: &'static str,
    /// Leader title for a male leader.
    pub title_m: &'static str,
    /// Leader title for a female leader.
    pub title_f: &'static str,
    /// Maximum tax or science rate in percent (0 = luxuries only, Anarchy).
    pub max_rate: u8,
    /// Units up to the city's size need no shields (Anarchy, Despotism); otherwise every unit
    /// but Diplomats and Caravans costs one (CIV.EXE 0xfcdd).
    pub free_units_up_to_size: bool,
    /// Unhappy citizens caused by each aggressive unit outside a city.
    pub unhappy_per_aggressive_unit: u8,
    /// Tiles producing more than 2 of anything lose 1.
    pub tile_penalty: bool,
    /// Tiles with at least 1 trade gain 1.
    pub trade_bonus: bool,
    /// Corruption factor (0 = none, 4 = worst). Communism applies it flat,
    /// independent of the distance to the Palace.
    pub corruption: u8,
    /// Food eaten by each Settlers unit: 2 from Monarchy on (CIV.EXE 0xcc27, gov > Despotism).
    pub settler_food: u8,
    /// Maximum unhappy citizens pacified by martial law.
    pub martial_law_max: u8,
    pub prereq: Option<AdvanceId>,
}

/// The 6 governments, in in-game order.
pub static GOVERNMENTS: [Government; 6] = [
    Government {
        name: "Anarchy",
        adjective: "Anarchy",
        title_m: "Mr.",
        title_f: "Mr.", // no female form in the executable
        max_rate: 0,
        free_units_up_to_size: true,
        unhappy_per_aggressive_unit: 0,
        tile_penalty: true,
        trade_bonus: false,
        corruption: 4,
        settler_food: 1,
        martial_law_max: 3,
        prereq: None,
    },
    Government {
        name: "Despotism",
        adjective: "Despotism",
        title_m: "Emperor",
        title_f: "Empress",
        max_rate: 60,
        free_units_up_to_size: true,
        unhappy_per_aggressive_unit: 0,
        tile_penalty: true,
        trade_bonus: false,
        corruption: 3,
        settler_food: 1,
        martial_law_max: 3,
        prereq: None,
    },
    Government {
        name: "Monarchy",
        adjective: "Monarchy",
        title_m: "King",
        title_f: "Queen",
        max_rate: 70,
        free_units_up_to_size: false,
        unhappy_per_aggressive_unit: 0,
        tile_penalty: false,
        trade_bonus: false,
        corruption: 2,
        settler_food: 2,
        martial_law_max: 3,
        prereq: Some(adv::MONARCHY),
    },
    Government {
        name: "Communism",
        adjective: "Communist",
        title_m: "Comrade",
        title_f: "Comrade",
        max_rate: 80,
        free_units_up_to_size: false,
        unhappy_per_aggressive_unit: 0,
        tile_penalty: false,
        trade_bonus: false,
        corruption: 2, // flat: same in every city regardless of distance
        settler_food: 2,
        martial_law_max: 3,
        prereq: Some(adv::COMMUNISM),
    },
    Government {
        name: "Republic",
        adjective: "Republic",
        title_m: "President",
        title_f: "President",
        max_rate: 80,
        free_units_up_to_size: false,
        unhappy_per_aggressive_unit: 1,
        tile_penalty: false,
        trade_bonus: true,
        corruption: 1,
        settler_food: 2,
        martial_law_max: 0,
        prereq: Some(adv::THE_REPUBLIC),
    },
    Government {
        name: "Democracy",
        adjective: "Democratic",
        title_m: "President",
        title_f: "President",
        max_rate: 100,
        free_units_up_to_size: false,
        unhappy_per_aggressive_unit: 2,
        tile_penalty: false,
        trade_bonus: true,
        corruption: 0,
        settler_food: 2,
        martial_law_max: 0,
        prereq: Some(adv::DEMOCRACY),
    },
];
