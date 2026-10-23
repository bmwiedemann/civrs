//! Saved games (own format) and the hall of fame.

use crate::game::Game;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAGIC: [u8; 4] = *b"CIVR";
/// 2: the game body is followed by `Game::hints_shown` (u32 LE); version 1 saves still load.
/// 3: then by each player's `research_count` (u16 LE); older saves derive it from the advances.
/// 4: then by each player's 72 `advance_source` bytes; older saves record no sources.
/// 5: then by each player's spaceship trailer (grid, arrival year, landed flag).
/// 6: then by the year (i32 LE) and each player's launch year (i32 LE, `i32::MIN` = none);
/// older saves take the launch-free calendar year of their turns.
/// 7: then by the number of city slots (u32 LE) and `CITY_TRAILER` bytes per slot (the stashed
/// garrison); older saves have none.
pub const VERSION: u16 = 7;
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
    for p in &g.players {
        data.extend_from_slice(&spaceship_trailer(&p.spaceship));
    }
    data.extend_from_slice(&g.year.to_le_bytes());
    for p in &g.players {
        data.extend_from_slice(&p.spaceship.launch_year.unwrap_or(i32::MIN).to_le_bytes());
    }
    data.extend_from_slice(&city_trailer(g));
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
    let ships = sources + 72 * g.players.len();
    for (i, p) in g.players.iter_mut().enumerate() {
        match rest.get(ships + SHIP_TRAILER * i..ships + SHIP_TRAILER * (i + 1)) {
            Some(b) if ver >= 5 => read_spaceship_trailer(&mut p.spaceship, b),
            _ => migrate_spaceship(&mut p.spaceship),
        }
    }
    let calendar = ships + SHIP_TRAILER * g.players.len();
    let word = |at: usize| rest.get(at..at + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    g.year = match word(calendar) {
        Some(y) if ver >= 6 => y,
        _ => crate::rules::calendar_year(g.turn),
    };
    for (i, p) in g.players.iter_mut().enumerate() {
        let s = &mut p.spaceship;
        s.launch_year = match word(calendar + 4 + 4 * i) {
            Some(y) if ver >= 6 => (y != i32::MIN).then_some(y),
            _ => s.launched_turn.map(crate::rules::calendar_year),
        };
    }
    let cities = calendar + 4 + 4 * g.players.len();
    if ver >= 7 {
        read_city_trailer(&mut g, rest.get(cities..).unwrap_or_default());
    }
    Ok(g)
}

/// Bytes per city slot of the version-7 city trailer: the two garrison slots and the
/// garrison-out flag.
const CITY_TRAILER: usize = 3;

fn city_trailer(g: &Game) -> Vec<u8> {
    let mut v = (g.cities.len() as u32).to_le_bytes().to_vec();
    for c in &g.cities {
        match c {
            Some(c) => v.extend_from_slice(&[c.garrison[0], c.garrison[1], c.garrison_out as u8]),
            None => v.extend_from_slice(&[crate::game::city::NO_GARRISON, crate::game::city::NO_GARRISON, 0]),
        }
    }
    v
}

fn read_city_trailer(g: &mut Game, b: &[u8]) {
    let n = b.get(..4).map(|n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize).unwrap_or(0);
    for (i, c) in g.cities.iter_mut().enumerate().take(n) {
        if let (Some(c), Some(r)) = (c.as_mut(), b.get(4 + CITY_TRAILER * i..4 + CITY_TRAILER * (i + 1))) {
            c.garrison = [r[0], r[1]];
            c.garrison_out = r[2] != 0;
        }
    }
}

/// Bytes per player of the version-5 spaceship trailer: the grid, the arrival year (i32,
/// `i32::MIN` = none) and the landed flag.
const SHIP_TRAILER: usize = crate::game::spaceship::GRID_W * crate::game::spaceship::GRID_H + 5;

fn spaceship_trailer(s: &crate::game::Spaceship) -> Vec<u8> {
    let mut v: Vec<u8> = s.grid.iter().flatten().copied().collect();
    v.extend_from_slice(&s.arrival_year.unwrap_or(i32::MIN).to_le_bytes());
    v.push(s.landed as u8);
    v
}

fn read_spaceship_trailer(s: &mut crate::game::Spaceship, b: &[u8]) {
    let h = crate::game::spaceship::GRID_H;
    for (i, c) in b[..SHIP_TRAILER - 5].iter().enumerate() {
        s.grid[i / h][i % h] = *c;
    }
    let year = i32::from_le_bytes([b[SHIP_TRAILER - 5], b[SHIP_TRAILER - 4], b[SHIP_TRAILER - 3], b[SHIP_TRAILER - 2]]);
    s.arrival_year = (year != i32::MIN).then_some(year);
    s.landed = b[SHIP_TRAILER - 1] != 0;
}

/// Saves before version 5 only counted parts: lay them out on a grid; a launched ship keeps
/// its arrival turn, and "launched, no arrival turn, not lost" meant it had landed.
fn migrate_spaceship(s: &mut crate::game::Spaceship) {
    crate::game::spaceship::grid_from_counts(s);
    if s.launched() {
        s.landed = s.arrival_turn.is_none() && !s.lost;
        s.arrival_year = s.arrival_turn.map(crate::rules::calendar_year);
    }
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

    #[test]
    fn stashed_garrison_survives_a_save_and_old_saves_have_none() {
        let mut g = Game::new(GameSetup { seed: 5, ..Default::default() }, None);
        let h = g.human_slot;
        let settler = g.units_of(h).into_iter().find(|id| g.unit(*id).kind == crate::rules::unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        g.city_mut(c).garrison = [crate::rules::unit_ids::PHALANX | crate::game::city::GARRISON_VETERAN, crate::rules::unit_ids::MILITIA];
        g.city_mut(c).garrison_out = true;
        let dir = std::env::temp_dir().join(format!("civrs-garrison-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let l = super::load_game(&path).unwrap();
        assert_eq!(l.city(c).garrison, g.city(c).garrison);
        assert!(l.city(c).garrison_out);
        // a version-5 file: same body, no city trailer
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&5u16.to_le_bytes());
        data.truncate(data.len() - 4 - g.cities.len() * super::CITY_TRAILER);
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(old.city(c).garrison, crate::game::city::no_garrison());
        assert!(!old.city(c).garrison_out);
        assert_eq!(old.player(h).spaceship, g.player(h).spaceship);
    }

    #[test]
    fn spaceship_grid_survives_a_save_and_old_saves_get_one() {
        let mut g = Game::new(GameSetup { seed: 4, ..Default::default() }, None);
        let h = g.human_slot;
        g.apollo_built = true;
        g.place_spaceship_part(h, 0, 0);
        g.place_spaceship_part(h, 2, 1);
        g.player_mut(h).spaceship.arrival_year = Some(-3000);
        let dir = std::env::temp_dir().join(format!("civrs-ship-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let l = super::load_game(&path).unwrap();
        assert_eq!(l.player(h).spaceship, g.player(h).spaceship);
        // a version-4 file: same body, no spaceship trailer
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&4u16.to_le_bytes());
        data.truncate(data.len() - g.players.len() * super::SHIP_TRAILER - 4 - 4 * g.players.len());
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        let s = old.player(h).spaceship;
        assert_eq!((s.structurals, s.components, s.modules), (1, 0, 1));
        assert_eq!(s.arrival_year, None);
    }

    #[test]
    fn year_and_launch_year_survive_a_save_and_old_saves_take_the_calendar() {
        let mut g = Game::new(GameSetup { seed: 5, ..Default::default() }, None);
        let h = g.human_slot;
        g.set_turn(380);
        g.player_mut(h).spaceship.launched_turn = Some(370);
        g.player_mut(h).spaceship.launch_year = Some(1790);
        // a calendar that ran one year a turn since the launch
        g.year = 1800;
        let dir = std::env::temp_dir().join(format!("civrs-year-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let l = super::load_game(&path).unwrap();
        assert_eq!(l.year(), 1800);
        assert_eq!(l.player(h).spaceship.launch_year, Some(1790));
        assert_eq!(l.player(2).spaceship.launch_year, None);
        // a version-5 file: no calendar trailer
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&5u16.to_le_bytes());
        data.truncate(data.len() - 4 - 4 * g.players.len());
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(old.year(), crate::rules::calendar_year(380));
        assert_eq!(old.player(h).spaceship.launch_year, Some(crate::rules::calendar_year(370)));
    }
}
