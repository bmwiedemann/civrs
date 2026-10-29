//! Saved games (own format) and the hall of fame.

pub mod actions;
pub mod sve;

use crate::game::{Game, UnitOrder};
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
/// 8: then by each player's diplomacy flags (8 x u16 LE, `Player::diplo`) and last contact turn
/// (i32 LE); older saves derive the flags from their relations and embassies.
/// 9: then by each civilization's `AI_TRAILER` bytes: the 16 persistent AI goals (x, y, type,
/// priority as signed bytes, CIV.EXE ds:0xd76e..) and the continent policies (a flag byte, 1
/// when known, and 16 policies, ds:0xe698); older saves start with no goals and no policies.
/// 10: then by the map's site-value layer (`Map::site`, 80 x 50 bytes, or none when the map
/// has no layer: a u32 LE byte count first); older saves compute it from the terrain.
/// 11: then by the global warming counter (u16 LE, `Game::warming_counter`); older saves start
/// it at 0.
/// 12: then by the number of unit slots (u32 LE) and each slot's `Unit::seen_by` byte (0 for an
/// empty slot); older saves take the owner and the civilizations that see the square now.
/// 13: then by each player's 28 unit counters (`Player::unit_counter`, i16 LE) and 28 losses
/// (`Player::units_lost_by_type`); older saves count the units they have and no losses.
/// 14: then by each player's tribe names (`Player::tribe_names`): plural and adjective, each a
/// u8 length and the bytes, 0/0 for none; then each player's starting tribe (`start_civ`, one
/// byte; older files take the current tribe).
/// 15: then by the 72 first-discoverer bytes (`Game::first_discoverer`); older saves credit
/// each known advance to the first civilization that researched it, else to the first that
/// knows it.
/// 16: then by the tile-owner layer (`BarbarianMemory::occupant`, a u32 LE byte count and 80 x 50
/// bytes or none); older saves start without it.
/// 17: then by the historian's turn (`Game::historian_timer`, u32 LE); older saves take 20
/// turns after their turn, 80 at the earliest.
/// 18: then by the replay log and power history (`Game::history`, a u32 LE byte count and its
/// bincode) and the research catch-up bonus (`Game::catch_up`, i32 LE, set every 50 turns);
/// older saves start with empty histories and compute the bonus for their turn.
/// 19: then by the number of city slots (u32 LE) and each slot's `City::known_size` byte (0 for
/// an empty slot); older saves let the human know the size of every city whose square he has
/// explored.
/// 20: then by the census trailer (`CENSUS_TRAILER` bytes per civilization: the continent
/// defence, attack and cities rows of `Census`, 16 i32 LE each, its strength and citizens
/// (i32 LE) and its power rank (u8), then the ds:0x2a6e and ds:0x8078 bytes); older saves take the census of
/// every civilization and the power ranking as things stand.
/// 21: then by the number of city slots (u32 LE) and each slot's `City::coastal` byte (0/1, 0
/// for an empty slot); older saves give each city the bit its owner would get founding it now.
/// 22: then by the number of city slots (u32 LE) and each slot's `City::hydro_site` byte (0/1,
/// 0 for an empty slot); older saves derive it from the squares around the city.
/// 23: then by the number of city slots (u32 LE) and each slot's `City::name_index` byte (0xff
/// for an empty slot); older saves take the table entry of the city's name, none for a renamed
/// city.
/// 24: then by the number of unit slots (u32 LE) and each slot's `Unit::record` byte (0 for an
/// empty slot); older saves number each civ's units in slot order.
/// 25: then by the population milestone (`Game::pop_milestone`, u16 LE); older saves take the
/// one their population reaches from the start (`Game::derive_pop_milestone`).
/// 26: then by the length of the human's view of the improvements (`Map::seen`, u32 LE, 0
/// before he has looked at any square) and its words (u16 LE, `tile_flags::SEEN` bits); older
/// saves show the improvements as they are.
/// 27: then by the number of city slots (u32 LE) and each slot's `City::trade` (i16 LE, 0 for an
/// empty slot); older saves take the trade each city has now (`Game::visible_trade`).
/// 28: then by the number of unit slots (u32 LE) and each slot's `Unit::next` (u16 LE unit id,
/// 0xffff for none or an empty slot); older saves link each square's units up in record order
/// (`Game::rebuild_stacks`).
/// 29: then by the length of the site/owner layer (`Map::claims`, u32 LE, 0 while no city has
/// claimed anything) and its bytes; older saves rebuild it from the cities
/// (`Game::rebuild_claims`).
/// 30: then by the number of unit slots (u32 LE) and each slot's `Unit::sentry_job` as its
/// CIV.EXE status bits (u8: 0x02 road, 0x40 irrigation, 0x80 mine, 0xc0 fortress, 0x82
/// pollution, 0x04 fortifying, 0x08 fortified, 0 for none or an empty slot); older saves keep
/// no order under a sentry.
pub const VERSION: u16 = 30;
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

impl std::error::Error for SaveError {}

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
    for p in &g.players {
        for f in p.diplo {
            data.extend_from_slice(&f.to_le_bytes());
        }
        data.extend_from_slice(&p.last_contact.to_le_bytes());
    }
    data.extend_from_slice(&ai_trailer(g));
    data.extend_from_slice(&(g.map.site.len() as u32).to_le_bytes());
    data.extend_from_slice(&g.map.site);
    data.extend_from_slice(&g.warming_counter.to_le_bytes());
    data.extend_from_slice(&(g.units.len() as u32).to_le_bytes());
    data.extend(g.units.iter().map(|u| u.as_ref().map(|u| u.seen_by).unwrap_or(0)));
    for p in &g.players {
        for c in p.unit_counter.iter().chain(p.units_lost_by_type.iter()) {
            data.extend_from_slice(&c.to_le_bytes());
        }
    }
    for p in &g.players {
        let (plural, adj) = p.tribe_names.clone().unwrap_or_default();
        for name in [plural, adj] {
            let b = &name.as_bytes()[..name.len().min(255)];
            data.push(b.len() as u8);
            data.extend_from_slice(b);
        }
    }
    data.extend(g.players.iter().map(|p| p.start_civ));
    data.extend((0..72).map(|a| g.first_discoverer(a)));
    data.extend_from_slice(&(g.barbarian_memory.occupant.len() as u32).to_le_bytes());
    data.extend_from_slice(&g.barbarian_memory.occupant);
    data.extend_from_slice(&g.historian_timer.to_le_bytes());
    let history = bincode::serialize(&g.history).map_err(|e| SaveError::Format(e.to_string()))?;
    data.extend_from_slice(&(history.len() as u32).to_le_bytes());
    data.extend_from_slice(&history);
    data.extend_from_slice(&g.catch_up.to_le_bytes());
    data.extend_from_slice(&(g.cities.len() as u32).to_le_bytes());
    data.extend(g.cities.iter().map(|c| c.as_ref().map(|c| c.known_size).unwrap_or(0)));
    data.extend_from_slice(&census_trailer(g));
    data.extend_from_slice(&(g.cities.len() as u32).to_le_bytes());
    data.extend(g.cities.iter().map(|c| c.as_ref().map(|c| c.coastal as u8).unwrap_or(0)));
    data.extend_from_slice(&(g.cities.len() as u32).to_le_bytes());
    data.extend(g.cities.iter().map(|c| c.as_ref().map(|c| c.hydro_site as u8).unwrap_or(0)));
    data.extend_from_slice(&(g.cities.len() as u32).to_le_bytes());
    data.extend(g.cities.iter().map(|c| c.as_ref().map_or(crate::game::city::NO_NAME_INDEX, |c| c.name_index)));
    data.extend_from_slice(&(g.units.len() as u32).to_le_bytes());
    data.extend(g.units.iter().map(|u| u.as_ref().map(|u| u.record).unwrap_or(0)));
    data.extend_from_slice(&g.pop_milestone.to_le_bytes());
    data.extend_from_slice(&(g.map.seen.len() as u32).to_le_bytes());
    for f in &g.map.seen {
        data.extend_from_slice(&f.to_le_bytes());
    }
    data.extend_from_slice(&(g.cities.len() as u32).to_le_bytes());
    for c in &g.cities {
        let t = c.as_ref().map_or(0, |c| c.trade.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
        data.extend_from_slice(&t.to_le_bytes());
    }
    data.extend_from_slice(&(g.units.len() as u32).to_le_bytes());
    for u in &g.units {
        data.extend_from_slice(&u.as_ref().and_then(|u| u.next).unwrap_or(0xffff).to_le_bytes());
    }
    data.extend_from_slice(&(g.map.claims.len() as u32).to_le_bytes());
    data.extend_from_slice(&g.map.claims);
    data.extend_from_slice(&(g.units.len() as u32).to_le_bytes());
    data.extend(g.units.iter().map(|u| u.as_ref().filter(|u| u.order == UnitOrder::Sentry).and_then(|u| u.sentry_job).map_or(0, job_bits)));
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
    let city_slots = rest.get(cities..cities + 4).map(|n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize).unwrap_or(0);
    let diplo = cities + 4 + CITY_TRAILER * city_slots;
    for (i, p) in g.players.iter_mut().enumerate() {
        match rest.get(diplo + DIPLO_TRAILER * i..diplo + DIPLO_TRAILER * (i + 1)) {
            Some(b) if ver >= 8 => {
                for (k, f) in p.diplo.iter_mut().enumerate() {
                    *f = u16::from_le_bytes([b[2 * k], b[2 * k + 1]]);
                }
                p.last_contact = i32::from_le_bytes([b[16], b[17], b[18], b[19]]);
            }
            _ => migrate_relations(p),
        }
    }
    let ai = diplo + DIPLO_TRAILER * g.players.len();
    if ver >= 9 {
        if let Some(b) = rest.get(ai..ai + 8 * AI_TRAILER) {
            read_ai_trailer(&mut g, b);
        }
    }
    let site = ai + 8 * AI_TRAILER;
    if ver >= 10 {
        let n = rest.get(site..site + 4).map(|n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize).unwrap_or(0);
        g.map.site = rest.get(site + 4..site + 4 + n).map(|b| b.to_vec()).unwrap_or_default();
    }
    let warming = site + 4 + rest.get(site..site + 4).map(|n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize).unwrap_or(0);
    if let Some(b) = rest.get(warming..warming + 2).filter(|_| ver >= 11) {
        g.warming_counter = u16::from_le_bytes([b[0], b[1]]);
    }
    let seen = warming + 2;
    let slots = rest.get(seen..seen + 4).filter(|_| ver >= 12).map(|n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize);
    let counters = seen + 4 + slots.unwrap_or(0);
    let unit_kinds: Vec<(u8, usize)> = g.all_units().map(|u| (u.owner, u.kind as usize % 28)).collect();
    for (i, p) in g.players.iter_mut().enumerate() {
        match rest.get(counters + 112 * i..counters + 112 * (i + 1)).filter(|_| ver >= 13) {
            Some(b) => {
                for (k, c) in p.unit_counter.iter_mut().chain(p.units_lost_by_type.iter_mut()).enumerate() {
                    *c = i16::from_le_bytes([b[2 * k], b[2 * k + 1]]);
                }
            }
            None => {
                p.unit_counter = [0; 28];
                for (owner, kind) in &unit_kinds {
                    if *owner as usize == i {
                        p.unit_counter[*kind] += 1;
                    }
                }
            }
        }
    }
    let mut catch_up = None;
    let mut known_sizes: Option<Vec<u8>> = None;
    let mut census: Option<&[u8]> = None;
    let mut coastal: Option<Vec<u8>> = None;
    let mut hydro: Option<Vec<u8>> = None;
    let mut name_index: Option<Vec<u8>> = None;
    let mut records: Option<Vec<u8>> = None;
    let mut milestone: Option<u16> = None;
    let mut seen_layer: Option<Vec<u16>> = None;
    let mut stored_trade: Option<Vec<i16>> = None;
    let mut links: Option<Vec<u16>> = None;
    let mut claims: Option<Vec<u8>> = None;
    let mut sentry_jobs: Option<Vec<u8>> = None;
    // version 14: the tribe names, after the counters
    if ver >= 14 {
        let mut at = counters + 112 * g.players.len();
        let mut take = |rest: &[u8]| -> Option<String> {
            let n = *rest.get(at)? as usize;
            let b = rest.get(at + 1..at + 1 + n)?;
            at += 1 + n;
            Some(String::from_utf8_lossy(b).into_owned())
        };
        let mut names_ok = true;
        for p in g.players.iter_mut() {
            let (Some(plural), Some(adj)) = (take(rest), take(rest)) else {
                names_ok = false;
                break;
            };
            p.tribe_names = (!plural.is_empty()).then_some((plural, adj));
        }
        let starts = names_ok.then(|| rest.get(at..at + g.players.len())).flatten();
        for (i, p) in g.players.iter_mut().enumerate() {
            p.start_civ = starts.map_or(p.civ, |b| b[i]);
        }
        at += g.players.len();
        g.first_discoverer = match names_ok.then(|| rest.get(at..at + 72)).flatten() {
            Some(b) if ver >= 15 => b.to_vec(),
            _ => migrate_first_discoverers(&g),
        };
        at += 72;
        if let Some(n) = rest.get(at..at + 4).filter(|_| names_ok && ver >= 16) {
            let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
            g.barbarian_memory.occupant = rest.get(at + 4..at + 4 + n).map(|b| b.to_vec()).unwrap_or_default();
            at += 4 + n;
        }
        g.historian_timer = match rest.get(at..at + 4).filter(|_| names_ok && ver >= 17) {
            Some(b) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            None => (g.turn + 20).max(80),
        };
        at += 4;
        if let Some(n) = rest.get(at..at + 4).filter(|_| names_ok && ver >= 18) {
            let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
            g.history = rest.get(at + 4..at + 4 + n).and_then(|b| bincode::deserialize(b).ok()).unwrap_or_default();
            at += 4 + n;
            catch_up = rest.get(at..at + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            at += 4;
            if let Some(n) = rest.get(at..at + 4).filter(|_| ver >= 19) {
                let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
                known_sizes = rest.get(at + 4..at + 4 + n).map(|b| b.to_vec());
                at += 4 + n;
                census = rest.get(at..at + 8 * CENSUS_TRAILER + 2).filter(|_| ver >= 20);
                at += 8 * CENSUS_TRAILER + 2;
                if let Some(n) = rest.get(at..at + 4).filter(|_| ver >= 21) {
                    let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
                    coastal = rest.get(at + 4..at + 4 + n).map(|b| b.to_vec());
                    at += 4 + n;
                    if let Some(n) = rest.get(at..at + 4).filter(|_| ver >= 22) {
                        let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
                        hydro = rest.get(at + 4..at + 4 + n).map(|b| b.to_vec());
                        at += 4 + n;
                        if let Some(n) = rest.get(at..at + 4).filter(|_| ver >= 23) {
                            let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
                            name_index = rest.get(at + 4..at + 4 + n).map(|b| b.to_vec());
                            at += 4 + n;
                            if let Some(n) = rest.get(at..at + 4).filter(|_| ver >= 24) {
                                let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
                                records = rest.get(at + 4..at + 4 + n).map(|b| b.to_vec());
                                at += 4 + n;
                                milestone = rest.get(at..at + 2).filter(|_| ver >= 25).map(|b| u16::from_le_bytes([b[0], b[1]]));
                                at += 2;
                                if let Some(n) = rest.get(at..at + 4).filter(|_| ver >= 26) {
                                    let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
                                    seen_layer = rest.get(at + 4..at + 4 + 2 * n).map(|b| b.chunks(2).map(|w| u16::from_le_bytes([w[0], w[1]])).collect());
                                    at += 4 + 2 * n;
                                    if let Some(n) = rest.get(at..at + 4).filter(|_| ver >= 27) {
                                        let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
                                        stored_trade = rest.get(at + 4..at + 4 + 2 * n).map(|b| b.chunks(2).map(|w| i16::from_le_bytes([w[0], w[1]])).collect());
                                        at += 4 + 2 * n;
                                        if let Some(n) = rest.get(at..at + 4).filter(|_| ver >= 28) {
                                            let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
                                            links = rest.get(at + 4..at + 4 + 2 * n).map(|b| b.chunks(2).map(|w| u16::from_le_bytes([w[0], w[1]])).collect());
                                            at += 4 + 2 * n;
                                            if let Some(n) = rest.get(at..at + 4).filter(|_| ver >= 29) {
                                                let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
                                                claims = rest.get(at + 4..at + 4 + n).map(|b| b.to_vec());
                                                at += 4 + n;
                                                if let Some(n) = rest.get(at..at + 4).filter(|_| ver >= 30) {
                                                    let n = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
                                                    sentry_jobs = rest.get(at + 4..at + 4 + n).map(|b| b.to_vec());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    } else {
        for p in g.players.iter_mut() {
            p.start_civ = p.civ;
        }
        g.historian_timer = (g.turn + 20).max(80);
        g.first_discoverer = migrate_first_discoverers(&g);
    }
    match slots.and_then(|n| rest.get(seen + 4..seen + 4 + n)) {
        Some(b) => {
            for (u, v) in g.units.iter_mut().zip(b) {
                if let Some(u) = u {
                    u.seen_by = *v;
                }
            }
        }
        None => {
            let vision = g.vision.clone();
            for u in g.units.iter_mut().flatten() {
                let here = if crate::game::Map::in_bounds(u.x, u.y) { vision.get(crate::game::Map::idx(u.x, u.y)).copied().unwrap_or(0) } else { 0 };
                u.seen_by = 1 << u.owner | here;
            }
        }
    }
    match records {
        Some(b) => {
            for (u, r) in g.units.iter_mut().zip(b) {
                if let Some(u) = u {
                    u.record = r;
                }
            }
        }
        None => g.number_unit_records(),
    }
    if let Some(b) = sentry_jobs {
        for (u, j) in g.units.iter_mut().zip(b) {
            if let Some(u) = u.as_mut().filter(|u| u.order == UnitOrder::Sentry) {
                u.sentry_job = bits_job(j);
            }
        }
    }
    match links {
        Some(b) => {
            for (u, n) in g.units.iter_mut().zip(b) {
                if let Some(u) = u {
                    u.next = (n != 0xffff).then_some(n);
                }
            }
        }
        None => g.rebuild_stacks(),
    }
    // older saves link cargo to its ship; the stack ring carries it now
    for u in g.units.iter_mut().flatten() {
        u.legacy_cargo = None;
    }
    match catch_up {
        Some(v) => g.catch_up = v,
        None => g.update_research_catch_up(),
    }
    let human = g.human_slot;
    match known_sizes {
        Some(b) => {
            for (c, k) in g.cities.iter_mut().zip(b) {
                if let Some(c) = c {
                    c.known_size = k;
                }
            }
        }
        None => {
            for i in 0..g.cities.len() {
                let Some((x, y, size)) = g.cities[i].as_ref().map(|c| (c.x, c.y, c.size)) else { continue };
                if g.explored(human, x, y) {
                    g.cities[i].as_mut().unwrap().known_size = size;
                }
            }
        }
    }
    match census {
        Some(b) => read_census_trailer(&mut g, b),
        None => {
            g.take_census_all();
            g.rank_powers();
        }
    }
    match coastal {
        Some(b) => {
            for (c, k) in g.cities.iter_mut().zip(b) {
                if let Some(c) = c {
                    c.coastal = k != 0;
                }
            }
        }
        None => {
            for i in 0..g.cities.len() {
                let Some((owner, x, y)) = g.cities[i].as_ref().map(|c| (c.owner, c.x, c.y)) else { continue };
                let bit = g.founding_coastal(owner, x, y);
                g.cities[i].as_mut().unwrap().coastal = bit;
            }
        }
    }
    for i in 0..g.cities.len() {
        let Some((x, y)) = g.cities[i].as_ref().map(|c| (c.x, c.y)) else { continue };
        let bit = match &hydro {
            Some(b) => b.get(i).is_some_and(|k| *k != 0),
            None => g.founding_hydro_site(x, y),
        };
        g.cities[i].as_mut().unwrap().hydro_site = bit;
    }
    for (i, c) in g.cities.iter_mut().enumerate() {
        let Some(c) = c else { continue };
        c.name_index = match &name_index {
            Some(b) => b.get(i).copied().unwrap_or(crate::game::city::NO_NAME_INDEX),
            None => crate::game::city::name_table_index(&c.name),
        };
    }
    match milestone {
        Some(t) => g.pop_milestone = t,
        None => g.derive_pop_milestone(),
    }
    // the load routine (o 0xf0ce): the next two turn changes pass the milestone in silence
    g.milestone_quiet = -3;
    // older saves: the human sees every square's improvements as they are
    g.map.seen = seen_layer.filter(|s| s.len() == g.map.tiles.len()).unwrap_or_default();
    // older saves: every city's stored trade is its trade as things stand
    for i in 0..g.cities.len() as crate::game::CityId {
        if g.city_opt(i).is_none() {
            continue;
        }
        let t = match &stored_trade {
            Some(b) => b.get(i as usize).map_or(0, |t| *t as i32),
            None => g.visible_trade(i),
        };
        g.city_mut(i).trade = t;
    }
    match claims.filter(|c| c.is_empty() || c.len() == g.map.tiles.len()) {
        Some(c) => g.map.claims = c,
        None => g.rebuild_claims(),
    }
    Ok(g)
}

/// The status bits (CIV.EXE unit +0) of an order kept under a sentry (version-30 trailer).
fn job_bits(o: UnitOrder) -> u8 {
    match o {
        UnitOrder::Road => 0x02,
        UnitOrder::Irrigate => 0x40,
        UnitOrder::Mine => 0x80,
        UnitOrder::Fortress => 0xc0,
        UnitOrder::CleanPollution => 0x82,
        UnitOrder::Fortify => 0x04,
        UnitOrder::Fortified => 0x08,
        _ => 0,
    }
}

fn bits_job(b: u8) -> Option<UnitOrder> {
    Some(match b {
        0x02 => UnitOrder::Road,
        0x40 => UnitOrder::Irrigate,
        0x80 => UnitOrder::Mine,
        0xc0 => UnitOrder::Fortress,
        0x82 => UnitOrder::CleanPollution,
        0x04 => UnitOrder::Fortify,
        0x08 => UnitOrder::Fortified,
        _ => return None,
    })
}

/// Bytes per civilization of the version-20 census trailer: three rows of 16 i32, the
/// strength and citizens (i32) and the power rank.
const CENSUS_TRAILER: usize = 3 * 16 * 4 + 4 + 4 + 1;

fn census_trailer(g: &Game) -> Vec<u8> {
    let m = &g.ai_memory;
    let c = &m.census;
    let mut v = Vec::with_capacity(8 * CENSUS_TRAILER + 2);
    for s in 0..8 {
        for x in c.defence[s].iter().chain(&c.attack[s]).chain(&c.cities[s]) {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(&c.strength[s].to_le_bytes());
        v.extend_from_slice(&c.citizens[s].to_le_bytes());
        v.push(m.ranks[s]);
    }
    v.push(c.faces_human);
    v.push(m.gang_up as u8);
    v
}

fn read_census_trailer(g: &mut Game, b: &[u8]) {
    let live = g.live_census();
    let m = &mut g.ai_memory;
    let word = |at: usize| i32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    for s in 0..8 {
        let r = s * CENSUS_TRAILER;
        for k in 0..16 {
            m.census.defence[s][k] = word(r + 4 * k);
            m.census.attack[s][k] = word(r + 64 + 4 * k);
            m.census.cities[s][k] = word(r + 128 + 4 * k);
        }
        m.census.strength[s] = word(r + 192);
        m.census.citizens[s] = word(r + 196);
        m.ranks[s] = b[r + 200];
    }
    m.census.faces_human = b[8 * CENSUS_TRAILER];
    m.gang_up = b[8 * CENSUS_TRAILER + 1] != 0;
    m.census.squares = live.squares;
    m.census.sites = live.sites;
}

/// First discoverers for a save from before version 15: the first civilization 1..7 that
/// researched the advance itself (source 0), else the first that knows it.
fn migrate_first_discoverers(g: &Game) -> Vec<u8> {
    (0..72u8)
        .map(|a| {
            let knows = |s: &u8| g.player(*s).knows(a);
            (1..8u8).filter(knows).find(|s| g.player(*s).advance_source(a) == 0).or_else(|| (1..8u8).find(knows)).unwrap_or(0)
        })
        .collect()
}

/// Bytes per civilization of the version-9 AI trailer: 16 persistent goals of 4 bytes, the
/// policy flag and 16 policies.
const AI_TRAILER: usize = crate::game::goals::PERSISTENT_GOALS * 4 + 1 + 16;

fn ai_trailer(g: &Game) -> Vec<u8> {
    let m = &g.ai_memory;
    let mut v = Vec::with_capacity(8 * AI_TRAILER);
    for c in 0..8 {
        for p in &m.persistent[c] {
            v.extend_from_slice(&[p.x as u8, p.y as u8, p.kind as u8, p.priority as u8]);
        }
        match m.policies[c] {
            Some(pol) => {
                v.push(1);
                v.extend_from_slice(&pol);
            }
            None => v.extend_from_slice(&[0; 17]),
        }
    }
    v
}

fn read_ai_trailer(g: &mut Game, b: &[u8]) {
    use crate::game::goals::{Goal, PERSISTENT_GOALS};
    for c in 0..8 {
        let r = &b[c * AI_TRAILER..(c + 1) * AI_TRAILER];
        for i in 0..PERSISTENT_GOALS {
            let q = &r[4 * i..4 * i + 4];
            g.ai_memory.persistent[c][i] = Goal { x: q[0] as i8, y: q[1] as i8, kind: q[2] as i8, priority: q[3] as i8 };
        }
        let at = 4 * PERSISTENT_GOALS;
        g.ai_memory.policies[c] = (r[at] != 0).then(|| r[at + 1..at + 17].try_into().unwrap());
    }
}

/// Bytes per player of the version-8 diplomacy trailer: the flags towards each slot and the
/// last contact turn.
const DIPLO_TRAILER: usize = 8 * 2 + 4;

/// Saves before version 8 kept a relation and an embassy bit per slot: a treaty becomes
/// contact and peace, a war contact without peace.
fn migrate_relations(p: &mut crate::game::Player) {
    use crate::game::diplomacy::flags::*;
    use crate::game::Relation;
    for s in 0..8 {
        let mut f = match p.legacy_relations[s] {
            Relation::NoContact => 0,
            Relation::Peace => CONTACT | PEACE,
            Relation::War => CONTACT,
        };
        if p.legacy_embassies & (1 << s) != 0 {
            f |= EMBASSY;
        }
        p.diplo[s] = f;
    }
    p.last_contact = -1;
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
    /// The final `civ_score`'s achievement (wonder) and citizen points, ds:0xd764 / ds:0xd766
    /// (record +8 / +0xa, o 0x2d45); FAME.DTA keeps them, the list does not show them.
    #[serde(default)]
    pub achievements: i32,
    #[serde(default)]
    pub citizens: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HallOfFame {
    pub entries: Vec<FameEntry>,
}

/// Bytes per Hall-of-Fame record of FAME.DTA (ovl 3: 6 records in far memory at 0x37d2):
/// +0 score, +2 difficulty, +4 ds:0xdc6a, +6 year, +8/+0xa achievement/citizen points,
/// +0xc population in 10,000s (i16 LE), +0xe leader name (16 B), +0x1e civilization (16 B).
pub const FAME_RECORD: usize = 0x2e;
/// Records in FAME.DTA (the screen lists five; a sixth shows only when it is the new one).
pub const FAME_RECORDS: usize = 6;

impl HallOfFame {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("FAME.DTA")
    }

    /// The Hall of Fame of `dir`, else the game's own FAME.DTA (`original`), else an older
    /// civrs `fame.dat`.
    pub fn load(dir: &Path, original: Option<&[u8]>) -> HallOfFame {
        if let Ok(d) = std::fs::read(Self::path(dir)) {
            return Self::from_dta(&d);
        }
        if let Some(d) = original {
            return Self::from_dta(d);
        }
        std::fs::read(dir.join("fame.dat")).ok().and_then(|d| bincode::deserialize(&d).ok()).unwrap_or_default()
    }

    /// Parse FAME.DTA; records with a score of 0 or less are empty.
    pub fn from_dta(d: &[u8]) -> HallOfFame {
        let word = |r: &[u8; FAME_RECORD], o: usize| i16::from_le_bytes([r[o], r[o + 1]]) as i32;
        let name = |b: &[u8]| String::from_utf8_lossy(b.split(|c| *c == 0).next().unwrap_or_default()).trim_end().to_string();
        let entries = d
            .as_chunks::<FAME_RECORD>()
            .0
            .iter()
            .take(FAME_RECORDS)
            .filter(|r| word(r, 0) > 0)
            .map(|r| {
                let (score, difficulty) = (word(r, 0), word(r, 2).clamp(0, 4) as u8);
                FameEntry {
                    leader: name(&r[0xe..0x1e]),
                    civ_plural: name(&r[0x1e..0x2e]),
                    score,
                    percent: (difficulty as i32 + 1) * score / 50,
                    year: word(r, 6),
                    difficulty,
                    title: String::new(),
                    population: word(r, 0xc) as i64 * 10_000,
                    achievements: word(r, 8),
                    citizens: word(r, 0xa),
                }
            })
            .collect();
        HallOfFame { entries }
    }

    /// FAME.DTA bytes: the entries, then empty records (score -1) up to six.
    pub fn to_dta(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(FAME_RECORD * FAME_RECORDS);
        for i in 0..FAME_RECORDS {
            let mut r = [0u8; FAME_RECORD];
            match self.entries.get(i) {
                Some(e) => {
                    r[0..2].copy_from_slice(&(e.score as i16).to_le_bytes());
                    r[2..4].copy_from_slice(&(e.difficulty as i16).to_le_bytes());
                    r[6..8].copy_from_slice(&(e.year as i16).to_le_bytes());
                    r[8..10].copy_from_slice(&(e.achievements as i16).to_le_bytes());
                    r[0xa..0xc].copy_from_slice(&(e.citizens as i16).to_le_bytes());
                    r[0xc..0xe].copy_from_slice(&((e.population / 10_000).min(i16::MAX as i64) as i16).to_le_bytes());
                    for (dst, src) in [(0xe, &e.leader), (0x1e, &e.civ_plural)] {
                        let b = src.as_bytes();
                        let n = b.len().min(15);
                        r[dst..dst + n].copy_from_slice(&b[..n]);
                    }
                }
                None => r[0..2].copy_from_slice(&(-1i16).to_le_bytes()),
            }
            out.extend_from_slice(&r);
        }
        out
    }

    pub fn save(&self, dir: &Path) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(Self::path(dir), self.to_dta());
    }

    /// Insert keeping the list sorted by `(difficulty + 2) * score` (desc, CIV.EXE o 0x2c88),
    /// at most six entries. A game below all six takes the sixth place (the search stops at
    /// index 5). Returns the rank.
    pub fn add(&mut self, e: FameEntry) -> usize {
        let weight = |x: &FameEntry| (x.difficulty as i32 + 2) * x.score;
        let pos = self.entries.iter().position(|x| weight(x) < weight(&e)).unwrap_or(self.entries.len()).min(FAME_RECORDS - 1);
        self.entries.insert(pos, e);
        self.entries.truncate(FAME_RECORDS);
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
    fn first_discoverers_tile_owners_and_histories_survive_a_save() {
        let mut g = Game::new(GameSetup { seed: 6, ..Default::default() }, None);
        let h = g.human_slot;
        let other = if h == 2 { 3 } else { 2 };
        g.grant_advance(h, adv::CURRENCY, AdvanceSource::Discovered);
        assert_eq!(g.first_discoverer(adv::CURRENCY), 0, "advances at turn 0 are not recorded");
        g.turn = 10;
        g.grant_advance(other, adv::POTTERY, AdvanceSource::Discovered);
        g.grant_advance(h, adv::POTTERY, AdvanceSource::From(other));
        assert_eq!(g.first_discoverer(adv::POTTERY), other);
        g.note_occupant(other, 10, 10);
        g.historian_timer = 123;
        g.record_power();
        g.replay_log(crate::game::replay::kind::WONDER, &[other, 3]);
        g.catch_up = 4;
        let dir = std::env::temp_dir().join(format!("civrs-first-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let l = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(l.first_discoverer, g.first_discoverer);
        assert_eq!(l.barbarian_memory.occupant, g.barbarian_memory.occupant);
        assert_eq!(l.historian_timer, 123);
        assert_eq!(l.history, g.history);
        assert_eq!(l.catch_up, 4, "the bonus of the last 50-turn mark");
    }

    #[test]
    fn the_coastal_bit_survives_a_save_and_old_saves_derive_it() {
        let mut g = Game::new(GameSetup { seed: 6, ..Default::default() }, None);
        let h = g.human_slot;
        let settler = g.units_of(h).into_iter().find(|id| g.unit(*id).kind == crate::rules::unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        let bit = g.city(c).coastal;
        g.city_mut(c).coastal = !bit;
        let dir = std::env::temp_dir().join(format!("civrs-coastal-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        assert_eq!(super::load_game(&path).unwrap().city(c).coastal, !bit);
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&19u16.to_le_bytes());
        data.truncate(data.len() - 4 - g.cities.len());
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(old.city(c).coastal, bit, "derived as at founding");
    }

    #[test]
    fn the_hydro_site_bit_survives_a_save_and_old_saves_derive_it() {
        let mut g = Game::new(GameSetup { seed: 6, ..Default::default() }, None);
        let h = g.human_slot;
        let settler = g.units_of(h).into_iter().find(|id| g.unit(*id).kind == crate::rules::unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        let bit = g.city(c).hydro_site;
        assert_eq!(bit, g.founding_hydro_site(g.city(c).x, g.city(c).y));
        g.city_mut(c).hydro_site = !bit;
        let dir = std::env::temp_dir().join(format!("civrs-hydro-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        assert_eq!(super::load_game(&path).unwrap().city(c).hydro_site, !bit);
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&21u16.to_le_bytes());
        data.truncate(data.len() - 4 - g.cities.len());
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(old.city(c).hydro_site, bit, "derived as at founding");
    }

    #[test]
    fn the_name_index_survives_a_save_and_old_saves_derive_it() {
        let mut g = Game::new(GameSetup { seed: 6, ..Default::default() }, None);
        let h = g.human_slot;
        let settler = g.units_of(h).into_iter().find(|id| g.unit(*id).kind == crate::rules::unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        let idx = g.city(c).name_index;
        assert_eq!(idx, crate::game::city::name_table_index(&g.city(c).name));
        assert_ne!(idx, crate::game::city::NO_NAME_INDEX);
        g.rename_city(c, "Elsewhere").unwrap();
        let dir = std::env::temp_dir().join(format!("civrs-nameidx-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        assert_eq!(super::load_game(&path).unwrap().city(c).name_index, idx);
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&22u16.to_le_bytes());
        data.truncate(data.len() - 4 - g.cities.len());
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(old.city(c).name_index, crate::game::city::NO_NAME_INDEX, "a renamed city's name is not in the table");
    }

    #[test]
    fn the_population_milestone_survives_a_save_and_old_saves_derive_it() {
        let mut g = Game::new(GameSetup { seed: 6, ..Default::default() }, None);
        let h = g.human_slot;
        let settler = g.units_of(h).into_iter().find(|id| g.unit(*id).kind == crate::rules::unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        g.city_mut(c).size = 7; // 28 x 10,000
        g.pop_milestone = 5;
        let dir = std::env::temp_dir().join(format!("civrs-milestone-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let l = super::load_game(&path).unwrap();
        assert_eq!((l.pop_milestone, l.milestone_quiet), (5, -3));
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&24u16.to_le_bytes());
        data.truncate(data.len() - 2);
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(old.pop_milestone, 2, "from 1, as CIV.EXE after loading");
    }

    #[test]
    fn the_human_view_of_improvements_survives_a_save() {
        use crate::game::tile_flags;
        let mut g = Game::new(GameSetup { seed: 6, ..Default::default() }, None);
        let h = g.human_slot;
        let ai = (1..8u8).find(|s| *s != h && g.players[*s as usize].alive).unwrap();
        let (x, y) = (0..80).flat_map(|x| (1..49).map(move |y| (x, y))).find(|&(x, y)| g.map.is_land(x, y) && !g.explored(h, x, y)).unwrap();
        g.map.update_seen(x, y);
        g.map.get_mut(x, y).unwrap().set(tile_flags::ROAD, true);
        assert_eq!(g.map.seen_flags(x, y), 0, "a road the human has not seen yet");
        // a computer's unit there shows the human nothing; his own one does
        let u = g.add_unit(ai, crate::rules::unit_ids::MILITIA, x, y);
        g.refresh_vision(ai);
        assert_eq!(g.map.seen_flags(x, y), 0);
        let dir = std::env::temp_dir().join(format!("civrs-seen-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let mut l = super::load_game(&path).unwrap();
        assert_eq!(l.map.seen, g.map.seen);
        l.remove_unit(u);
        l.add_unit(h, crate::rules::unit_ids::MILITIA, x, y);
        l.refresh_vision(h);
        assert_eq!(l.map.seen_flags(x, y), tile_flags::ROAD);
        // older saves see everything as it is
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&25u16.to_le_bytes());
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert!(old.map.seen.is_empty());
        assert_eq!(old.map.seen_flags(x, y), tile_flags::ROAD);
    }

    #[test]
    fn the_stored_trade_survives_a_save_and_old_saves_derive_it() {
        let mut g = Game::new(GameSetup { seed: 6, ..Default::default() }, None);
        let h = g.human_slot;
        let settler = g.units_of(h).into_iter().find(|id| g.unit(*id).kind == crate::rules::unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        let live = g.visible_trade(c);
        g.city_mut(c).trade = live + 9;
        let dir = std::env::temp_dir().join(format!("civrs-trade-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        assert_eq!(super::load_game(&path).unwrap().city(c).trade, live + 9);
        // older saves: the trade as things stand
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&26u16.to_le_bytes());
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(old.city(c).trade, live);
    }

    #[test]
    fn the_stack_rings_survive_a_save_and_old_saves_link_in_record_order() {
        let mut g = Game::new(GameSetup { seed: 6, ..Default::default() }, None);
        let h = g.human_slot;
        let a = g.add_unit(h, crate::rules::unit_ids::MILITIA, 30, 30);
        let b = g.add_unit(h, crate::rules::unit_ids::MILITIA, 30, 30);
        let c = g.add_unit(h, crate::rules::unit_ids::MILITIA, 30, 30);
        // a -> c -> b; b steps off and back: a -> b -> c
        g.relocate_unit(b, 31, 30);
        g.relocate_unit(b, 30, 30);
        assert_eq!(g.stack_ring(a, 10), vec![a, b, c]);
        let dir = std::env::temp_dir().join(format!("civrs-rings-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        assert_eq!(super::load_game(&path).unwrap().stack_ring(a, 10), vec![a, b, c]);
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&27u16.to_le_bytes());
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        // as if created in record order: a, then b after a, then c after a
        assert_eq!(old.stack_ring(a, 10), vec![a, c, b]);
        assert!(old.stacks_consistent());
    }

    #[test]
    fn the_site_owner_layer_survives_a_save_and_old_saves_rebuild_it() {
        let mut g = Game::new(GameSetup { seed: 6, ..Default::default() }, None);
        let h = g.human_slot;
        let settler = g.units_of(h).into_iter().find(|id| g.unit(*id).kind == crate::rules::unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        let (x, y) = g.city(c).pos();
        g.map.set_claim(x + 1, y, 0);
        let dir = std::env::temp_dir().join(format!("civrs-claims-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let l = super::load_game(&path).unwrap();
        assert_eq!(l.map.claims, g.map.claims);
        assert_eq!(l.map.claim_at(x + 1, y), 0);
        // older saves: the founding claims again
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&27u16.to_le_bytes());
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(old.map.claim_at(x + 1, y), h);
    }

    #[test]
    fn the_site_layer_survives_a_save() {
        let g = Game::new(GameSetup { seed: 5, ..Default::default() }, None);
        assert_eq!(g.map.site.len(), 80 * 50);
        let dir = std::env::temp_dir().join(format!("civrs-save-site-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let l = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(l.map.site, g.map.site);
        assert_eq!(l.map, g.map);
    }

    #[test]
    fn unit_sightings_survive_a_save_and_old_saves_take_the_owner_and_vision() {
        let mut g = Game::new(GameSetup { seed: 4, ..Default::default() }, None);
        let id = g.all_units().next().unwrap().id;
        let owner = g.unit(id).owner;
        let other = if owner == 2 { 3 } else { 2 };
        g.unit_mut(id).seen_by = 1 << owner | 1 << other;
        let dir = std::env::temp_dir().join(format!("civrs-sightings-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        assert_eq!(super::load_game(&path).unwrap().unit(id).seen_by, 1 << owner | 1 << other);
        // a version-11 file: no sightings; the owner, and who sees the square now
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&11u16.to_le_bytes());
        data.truncate(data.len() - 4 - g.units.len());
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        let u = old.unit(id);
        assert_eq!(old.unit(id).seen_by, 1 << owner | old.vision[crate::game::Map::idx(u.x, u.y)]);
    }

    #[test]
    fn warming_counter_survives_a_save_and_old_saves_start_at_zero() {
        let mut g = Game::new(GameSetup { seed: 4, ..Default::default() }, None);
        g.warming_counter = 13;
        let dir = std::env::temp_dir().join(format!("civrs-warming-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        assert_eq!(super::load_game(&path).unwrap().warming_counter, 13);
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&10u16.to_le_bytes());
        data.truncate(data.len() - 2 - 4 - g.units.len());
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(old.warming_counter, 0);
        assert_eq!(old.player(1).diplo, g.player(1).diplo);
    }

    #[test]
    fn diplomacy_flags_survive_a_save_and_old_saves_derive_them() {
        use crate::game::diplomacy::flags::*;
        use crate::game::Relation;
        let mut g = Game::new(GameSetup { seed: 6, ..Default::default() }, None);
        g.set_relation(1, 2, Relation::Peace);
        g.set_relation(1, 3, Relation::War);
        g.raise_flag(2, 1, VENDETTA | BREAK);
        g.raise_flag(1, 2, EMBASSY);
        g.player_mut(2).last_contact = 17;
        let dir = std::env::temp_dir().join(format!("civrs-diplo-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let l = super::load_game(&path).unwrap();
        for s in 0..8 {
            assert_eq!(l.player(s).diplo, g.player(s).diplo);
        }
        assert_eq!(l.player(2).last_contact, 17);
        // a version-7 file: no diplomacy trailer; relations and embassies of the body count
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&7u16.to_le_bytes());
        data.truncate(data.len() - 8 * super::DIPLO_TRAILER - 8 * super::AI_TRAILER - 4 - g.map.site.len() - 2 - 4 - g.units.len());
        std::fs::write(&path, &data).unwrap();
        let mut old = super::load_game(&path).unwrap();
        // what a version-7 save stored
        old.players[1].legacy_relations[2] = Relation::Peace;
        old.players[1].legacy_relations[3] = Relation::War;
        old.players[1].legacy_embassies = 1 << 2;
        super::migrate_relations(&mut old.players[1]);
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(old.diplo(1, 2), CONTACT | PEACE | EMBASSY);
        assert_eq!(old.diplo(1, 3), CONTACT);
        assert_eq!(old.diplo(1, 4), 0);
        assert_eq!(old.player(2).last_contact, -1);
    }

    #[test]
    fn the_census_tables_and_power_ranks_survive_a_save() {
        let mut g = Game::new(GameSetup { seed: 8, ..Default::default() }, None);
        g.take_census_all();
        g.ai_memory.census.attack[3][2] = 77;
        g.ai_memory.census.strength[5] = -4;
        g.ai_memory.census.faces_human = 0x24;
        g.ai_memory.ranks = [0, 7, 6, 5, 4, 3, 2, 1];
        g.ai_memory.gang_up = true;
        let dir = std::env::temp_dir().join(format!("civrs-census-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let l = super::load_game(&path).unwrap();
        assert_eq!(l.ai_memory.census, g.ai_memory.census);
        assert_eq!((l.ai_memory.ranks, l.ai_memory.gang_up), (g.ai_memory.ranks, true));
        // a version-19 file takes the census and ranking of the moment
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&19u16.to_le_bytes());
        data.truncate(data.len() - (8 * super::CENSUS_TRAILER + 2));
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        let mut now = g.clone();
        now.ai_memory.census = Default::default();
        now.take_census_all();
        assert_eq!(old.ai_memory.census, now.ai_memory.census);
        assert!(!old.ai_memory.gang_up);
        assert_eq!(old.ai_memory.ranks.iter().filter(|r| **r == 7).count(), 1);
    }

    #[test]
    fn persistent_ai_goals_and_policies_survive_a_save() {
        use crate::game::goals::goal;
        let mut g = Game::new(GameSetup { seed: 8, ..Default::default() }, None);
        g.add_persistent_goal(3, 12, 34, goal::SETTLE, 2);
        g.add_persistent_goal(5, 70, 1, goal::DEFEND, 7);
        g.ai_memory.policies[3] = Some([5; 16]);
        g.add_goal(3, 1, 1, goal::ATTACK, 4);
        let dir = std::env::temp_dir().join(format!("civrs-goals-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let l = super::load_game(&path).unwrap();
        assert_eq!(l.ai_memory.persistent, g.ai_memory.persistent);
        assert_eq!(l.ai_memory.policies[3], Some([5; 16]));
        assert_eq!(l.ai_memory.policies[2], None);
        // the transient goals are not saved, as in CIV.EXE
        assert!(!l.ai_memory.goals[3].iter().any(|o| o.pos() == (1, 1)));
        // a version-8 file: no AI trailer
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&8u16.to_le_bytes());
        data.truncate(data.len() - 8 * super::AI_TRAILER);
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert!(old.ai_memory.persistent.iter().flatten().all(|p| p.is_free()));
        assert_eq!(old.player(2).diplo, g.player(2).diplo);
    }

    #[test]
    fn a_game_below_six_entries_takes_the_sixth_place() {
        let e = |score| super::FameEntry { leader: String::new(), civ_plural: String::new(), score, percent: 0, year: 0, difficulty: 0, title: String::new(), population: 0, achievements: 0, citizens: 0 };
        let mut h = super::HallOfFame::default();
        for s in [600, 500, 400, 300, 200, 100] {
            h.add(e(s));
        }
        // fame_insert (o 0x2c88) stops its search at index 5
        assert_eq!(h.add(e(50)), 5);
        assert_eq!(h.entries.len(), 6);
        assert_eq!(h.entries[5].score, 50);
        assert_eq!(h.add(e(450)), 2);
        assert_eq!(h.entries.iter().map(|x| x.score).collect::<Vec<_>>(), [600, 500, 450, 400, 300, 200]);
    }

    #[test]
    fn fame_dta_round_trips_and_reads_the_original_layout() {
        let mut rec = vec![0u8; super::FAME_RECORD];
        rec[0..2].copy_from_slice(&12i16.to_le_bytes());
        rec[2..4].copy_from_slice(&4i16.to_le_bytes());
        rec[6..8].copy_from_slice(&(-1700i16).to_le_bytes());
        rec[0xc..0xe].copy_from_slice(&26i16.to_le_bytes());
        rec[8..10].copy_from_slice(&40i16.to_le_bytes());
        rec[0xa..0xc].copy_from_slice(&33i16.to_le_bytes());
        rec[0xe..0xe + 11].copy_from_slice(b"Ray Maloney");
        rec[0x1e..0x1e + 9].copy_from_slice(b"Egyptians");
        let mut empty = vec![0u8; super::FAME_RECORD];
        empty[0..2].copy_from_slice(&(-1i16).to_le_bytes());
        let data: Vec<u8> = [rec, empty.clone(), empty.clone(), empty.clone(), empty.clone(), empty].concat();
        let h = super::HallOfFame::from_dta(&data);
        assert_eq!(h.entries.len(), 1);
        let e = &h.entries[0];
        assert_eq!((e.leader.as_str(), e.civ_plural.as_str(), e.score, e.difficulty, e.year, e.population, e.percent), ("Ray Maloney", "Egyptians", 12, 4, -1700, 260_000, 1));
        assert_eq!((e.achievements, e.citizens), (40, 33));
        assert_eq!(super::HallOfFame::from_dta(&h.to_dta()), h);
    }

    #[test]
    fn unit_counters_survive_a_save_and_old_saves_count_their_units() {
        let mut g = Game::new(GameSetup { seed: 6, ..Default::default() }, None);
        let h = g.human_slot;
        // civ_init leaves the start units uncounted
        assert_eq!(g.player(h).unit_counter[crate::rules::unit_ids::MILITIA as usize], 0);
        let u = g.add_unit(h, crate::rules::unit_ids::MILITIA, 10, 10);
        g.player_mut(h).units_lost_by_type[crate::rules::unit_ids::PHALANX as usize] = 3;
        let dir = std::env::temp_dir().join(format!("civrs-counters-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let l = super::load_game(&path).unwrap();
        assert_eq!(l.player(h).unit_counter, g.player(h).unit_counter);
        assert_eq!(l.player(h).units_lost_by_type, g.player(h).units_lost_by_type);
        // a version-12 file: the units it has are counted, no losses
        let mut data = std::fs::read(&path).unwrap();
        data[4..6].copy_from_slice(&12u16.to_le_bytes());
        data.truncate(data.len() - 112 * g.players.len());
        std::fs::write(&path, &data).unwrap();
        let old = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        let militia = old.units_of(h).into_iter().filter(|id| old.unit(*id).kind == crate::rules::unit_ids::MILITIA).count() as i16;
        assert_eq!(old.player(h).unit_counter[crate::rules::unit_ids::MILITIA as usize], militia);
        assert_eq!(old.player(h).units_lost_by_type, [0; 28]);
        let _ = u;
    }

    #[test]
    fn tribe_names_survive_a_save() {
        let g = Game::new(GameSetup { seed: 6, tribe_name: Some("Vandals".into()), ..Default::default() }, None);
        let h = g.human_slot;
        assert_eq!((g.player(h).plural(), g.player(h).adjective()), ("Vandals", "Vandal"));
        let dir = std::env::temp_dir().join(format!("civrs-tribe-{}", std::process::id()));
        let path = dir.join("s.sav");
        super::save_game(&path, &g).unwrap();
        let l = super::load_game(&path).unwrap();
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(l.player(h).tribe_names, g.player(h).tribe_names);
        assert!((1..8).filter(|s| *s != h).all(|s| l.player(s).tribe_names.is_none()));
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
        data.truncate(data.len() - 8 * super::AI_TRAILER - 4 - g.map.site.len() - 2 - 4 - g.units.len() - 8 * super::DIPLO_TRAILER - 4 - g.cities.len() * super::CITY_TRAILER);
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
