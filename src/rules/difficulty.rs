//! Difficulty levels (5).

/// Difficulty level; the discriminant is the in-game level index.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Difficulty {
    Chieftain = 0,
    Warlord = 1,
    Prince = 2,
    King = 3,
    Emperor = 4,
}

impl Difficulty {
    pub const ALL: [Difficulty; 5] = [
        Difficulty::Chieftain,
        Difficulty::Warlord,
        Difficulty::Prince,
        Difficulty::King,
        Difficulty::Emperor,
    ];

    pub fn from_u8(v: u8) -> Option<Difficulty> {
        Self::ALL.get(v as usize).copied()
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn info(self) -> &'static DifficultyInfo {
        &DIFFICULTY[self as usize]
    }

    pub fn name(self) -> &'static str {
        self.info().name
    }
}

/// Static description of one difficulty level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DifficultyInfo {
    pub name: &'static str,
    /// Short form used in the hall of fame / status displays.
    pub short: &'static str,
    /// Year at which the game ends (READ.ME).
    pub end_year: i32,
    /// Citizens born content in every city.
    pub content_base: u8,
    /// Settlers the human player starts with.
    pub human_start_settlers: u8,
    /// Free advances given to AI civilizations at the start.
    pub ai_start_techs: u8,
}

const fn level(
    name: &'static str,
    short: &'static str,
    end_year: i32,
    content_base: u8,
    human_start_settlers: u8,
    ai_start_techs: u8,
) -> DifficultyInfo {
    DifficultyInfo {
        name,
        short,
        end_year,
        content_base,
        human_start_settlers,
        ai_start_techs,
    }
}

/// The 5 difficulty levels, easiest first.
pub static DIFFICULTY: [DifficultyInfo; 5] = [
    level("Chieftain", "Chief", 2100, 4, 2, 0),
    level("Warlord", "Lord", 2080, 3, 1, 0),
    level("Prince", "Prince", 2060, 2, 1, 1),
    level("King", "King", 2040, 1, 1, 2),
    level("Emperor", "Emperor", 2020, 0, 1, 3),
];
