//! Saved games (own format) and the hall of fame.

use crate::game::Game;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAGIC: [u8; 4] = *b"CIVR";
/// 2: the game body is followed by `Game::hints_shown` (u32 LE); version 1 saves still load.
/// 3: then by each player's `research_count` (u16 LE); older saves derive it from the advances.
/// 4: then by each player's 72 `advance_source` bytes; older saves record no sources.
pub const VERSION: u16 = 4;
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
    for p in &g.players {
        data.extend_from_slice(&p.research_count.to_le_bytes());
    }
    for p in &g.players {
        data.extend((0..72).map(|a| p.advance_source(a) as u8));
    }
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
    let sources = 4 + 2 * g.players.len();
    for (i, p) in g.players.iter_mut().enumerate() {
        p.research_count = match rest.get(4 + 2 * i..6 + 2 * i) {
            Some(b) if ver >= 3 => u16::from_le_bytes([b[0], b[1]]),
            _ => 1 + p.advance_count() as u16 + p.future_techs as u16,
        };
        p.advance_source = match rest.get(sources + 72 * i..sources + 72 * (i + 1)) {
            Some(b) if ver >= 4 => b.iter().map(|v| *v as i8).collect(),
            _ => vec![-1; 72],
        };
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

    /// Insert keeping the list sorted by `(difficulty + 2) * score` (desc, CIV.EXE o 0x2c88),
    /// max 10 entries. Returns the rank.
    pub fn add(&mut self, e: FameEntry) -> usize {
        let weight = |x: &FameEntry| (x.difficulty as i32 + 2) * x.score;
        let pos = self.entries.iter().position(|x| weight(x) < weight(&e)).unwrap_or(self.entries.len());
        self.entries.insert(pos, e);
        self.entries.truncate(10);
        pos
    }
}

#[cfg(test)]
mod tests {
    use crate::game::science::AdvanceSource;
    use crate::game::{Game, GameSetup};
    use crate::rules::advance_ids as adv;

    #[test]
    fn research_count_and_advance_sources_survive_a_save() {
        let mut g = Game::new(GameSetup { seed: 3, ..Default::default() }, None);
        g.turn = 10;
        let h = g.human_slot;
        g.grant_advance(h, adv::CURRENCY, AdvanceSource::From(2));
        g.grant_advance(h, adv::POTTERY, AdvanceSource::GreatLibrary);
        let dir = std::env::temp_dir().join(format!("civrs-save-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let l = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        for s in 0..8 {
            assert_eq!(l.player(s).research_count, g.player(s).research_count);
            assert_eq!(l.player(s).advance_source, g.player(s).advance_source);
        }
        assert_eq!(l.human().advance_source(adv::CURRENCY), 2);
        assert_eq!(l.human().advance_source(adv::POTTERY), -2);
    }
}
