//! Saved games (own format) and the hall of fame.

use crate::game::Game;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAGIC: [u8; 4] = *b"CIVR";
/// 2: the game body is followed by `Game::hints_shown` (u32 LE); version 1 saves still load.
pub const VERSION: u16 = 2;
pub const SLOTS: u8 = 10;

#[derive(Debug)]
pub enum SaveError {
    Io(std::io::Error),
    Format(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Io(e) => write!(f, "I/O error: {e}"),
            SaveError::Format(s) => write!(f, "bad save file: {s}"),
        }
    }
}

impl From<std::io::Error> for SaveError {
    fn from(e: std::io::Error) -> Self {
        SaveError::Io(e)
    }
}

pub fn default_save_dir() -> PathBuf {
    if let Ok(d) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(d).join("civrs");
    }
    if let Ok(h) = std::env::var("HOME") {
        return PathBuf::from(h).join(".local/share/civrs");
    }
    PathBuf::from("saves")
}

pub fn slot_path(dir: &Path, slot: u8) -> PathBuf {
    dir.join(format!("civ{slot}.sav"))
}

pub fn save_game(path: &Path, g: &Game) -> Result<(), SaveError> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let body = bincode::serialize(g).map_err(|e| SaveError::Format(e.to_string()))?;
    let mut data = Vec::with_capacity(body.len() + 8);
    data.extend_from_slice(&MAGIC);
    data.extend_from_slice(&VERSION.to_le_bytes());
    data.extend_from_slice(&body);
    data.extend_from_slice(&g.hints_shown.to_le_bytes());
    std::fs::write(path, data)?;
    Ok(())
}

pub fn load_game(path: &Path) -> Result<Game, SaveError> {
    let data = std::fs::read(path)?;
    if data.len() < 6 || data[..4] != MAGIC {
        return Err(SaveError::Format("not a civrs save".into()));
    }
    let ver = u16::from_le_bytes([data[4], data[5]]);
    if ver == 0 || ver > VERSION {
        return Err(SaveError::Format(format!("unsupported version {ver}")));
    }
    let mut rest = &data[6..];
    let mut g: Game = bincode::deserialize_from(&mut rest).map_err(|e| SaveError::Format(e.to_string()))?;
    if ver >= 2 {
        let bits: [u8; 4] = rest.get(..4).and_then(|b| b.try_into().ok()).ok_or_else(|| SaveError::Format("missing hint flags".into()))?;
        g.hints_shown = u32::from_le_bytes(bits);
    }
    Ok(g)
}

#[derive(Clone, Debug)]
pub struct SlotInfo {
    pub slot: u8,
    pub leader: String,
    pub civ: String,
    pub year: i32,
    pub difficulty: crate::rules::Difficulty,
}

pub fn list_slots(dir: &Path) -> Vec<Option<SlotInfo>> {
    (0..SLOTS)
        .map(|s| {
            let p = slot_path(dir, s);
            if !p.exists() {
                return None;
            }
            load_game(&p).ok().map(|g| SlotInfo {
                slot: s,
                leader: g.human().leader.clone(),
                civ: g.human().plural().to_string(),
                year: g.year(),
                difficulty: g.setup.difficulty,
            })
        })
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FameEntry {
    pub leader: String,
    pub civ_plural: String,
    pub score: i32,
    pub percent: i32,
    pub year: i32,
    pub difficulty: u8,
    /// Government title ("Emperor", "President", ...).
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub population: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HallOfFame {
    pub entries: Vec<FameEntry>,
}

impl HallOfFame {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("fame.dat")
    }

    pub fn load(dir: &Path) -> HallOfFame {
        std::fs::read(Self::path(dir)).ok().and_then(|d| bincode::deserialize(&d).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) {
        let _ = std::fs::create_dir_all(dir);
        if let Ok(d) = bincode::serialize(self) {
            let _ = std::fs::write(Self::path(dir), d);
        }
    }

    /// Insert keeping the list sorted by percent (desc), max 10 entries. Returns the rank.
    pub fn add(&mut self, e: FameEntry) -> usize {
        let pos = self.entries.iter().position(|x| x.percent < e.percent).unwrap_or(self.entries.len());
        self.entries.insert(pos, e);
        self.entries.truncate(10);
        pos
    }
}
