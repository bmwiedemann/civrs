//! Import of the original game's saved games: `CIVILn.SVE` and `CIVILn.MAP` as CIV.EXE's
//! overlay 11 writes them (`load_files` o 0xf5b0, `save_files` o 0xf66b).
//!
//! The .SVE is the field dump of `serialize_game` (o 0xf75b): 37856 bytes of game tables in a
//! fixed order (the offsets below; see the catalogue's overlay 11 table). The .MAP is the map
//! page (graphics page 2) as a PIC picture (`pic_load_page0` 0x1f20:0x24a reads it back): the
//! terrain codes at (x, y), the continent and ocean numbers at (x, y+50), the improvement
//! nibbles at (x, y+100) / (x, y+150), the site-value layer at (x+80, y), the human's view of
//! the improvements at (x+80, y+100/150) and the tile owners at (x+160, y).
//!
//! The import builds a `Game` in memory; civrs keeps saving in its own format. Specials and
//! huts are patterns of the map seed (ds:0x6e80); a hut is gone once bit 0 of the square's
//! visibility byte is set (`tile_has_hut` 0x1a68:0x189a).
//!
//! Not imported (civrs has no such state or computes it): the per-type production counts
//! ds:0xd308, the per-civ and per-continent AI statistics (ds:0x6cac, 0x807a, 0x6b96,
//! 0xdcfe, 0xe16a, 0x8068, 0xd2f8, 0x6e82, 0x803c, 0xddfe, 0xb1ee), the coarse block tables
//! (far 0x1f88, ds:0xdb44), the unit type table copy, the kills matrix ds:0xe418, the
//! historian's scores ds:0x6c9c, the colonist counts ds:0xdeec, the human's view of the tile
//! improvements, the city trade byte +0xa (recomputed), the unit flag 0x10, the world
//! parameters and the founding turn of cities (not saved by CIV.EXE), the human-player mask
//! ds:0x7f36 (civrs has one human: offset 2).

use super::{SaveError, SlotInfo};
use crate::game::city::{City, Production, Specialist};
use crate::game::goals::Goal;
use crate::game::map::{tile_flags, Map, Tile};
use crate::game::{Game, GameSetup, Player, Unit, UnitOrder, WONDER_LOST};
use crate::rules::{self, Difficulty, Govt, Terrain, MAP_H, MAP_W};
use std::path::{Path, PathBuf};

/// Size of a CIVILn.SVE (0x93e0).
pub const SVE_LEN: usize = 0x93e0;

/// File offsets of the fields of `serialize_game` (o 0xf75b).
mod off {
    pub const TURN: usize = 0x0000;
    pub const HUMAN: usize = 0x0002;
    pub const SEED: usize = 0x0006;
    pub const YEAR: usize = 0x0008;
    pub const DIFFICULTY: usize = 0x000a;
    pub const ACTIVE: usize = 0x000c;
    pub const RESEARCH: usize = 0x000e;
    pub const LEADERS: usize = 0x0010;
    pub const PLURALS: usize = 0x0080;
    pub const ADJECTIVES: usize = 0x00e0;
    pub const GOLD: usize = 0x0138;
    pub const BEAKERS: usize = 0x0148;
    pub const UNIT_COUNTS: usize = 0x0158;
    pub const RESEARCH_COUNT: usize = 0x04d8;
    pub const ADVANCES: usize = 0x04e8;
    pub const GOVERNMENT: usize = 0x0538;
    pub const POLICIES: usize = 0x0548;
    pub const DIPLOMACY: usize = 0x0648;
    pub const CITIZENS: usize = 0x0708;
    pub const STRENGTH: usize = 0x0718;
    pub const RANKS: usize = 0x0728;
    pub const TAX: usize = 0x0738;
    pub const LAST_CONTACT: usize = 0x0758;
    pub const TRIBE: usize = 0x0778;
    pub const CENSUS: usize = 0x0788;
    pub const POWER: usize = 0x0ba8;
    pub const CITIES: usize = 0x1508;
    pub const UNITS: usize = 0x26c0;
    pub const VISIBILITY: usize = 0x56c0;
    pub const GOAL_TYPE: usize = 0x6660;
    pub const GOAL_PRIORITY: usize = 0x66e0;
    pub const GOAL_X: usize = 0x6760;
    pub const GOAL_Y: usize = 0x67e0;
    pub const FIRST: usize = 0x6860;
    pub const CITY_NAMES: usize = 0x6970;
    pub const REPLAY_LEN: usize = 0x7670;
    pub const REPLAY: usize = 0x7672;
    pub const WONDERS: usize = 0x8672;
    pub const LOSSES: usize = 0x869e;
    pub const SOURCES: usize = 0x885e;
    pub const POLLUTED: usize = 0x8a9e;
    pub const WARMING_COUNTER: usize = 0x8aa0;
    pub const WARMINGS: usize = 0x8aa2;
    pub const OPTIONS: usize = 0x8aa4;
    pub const LAND_LINKS: usize = 0x8aa6;
    pub const FUTURE_TECHS: usize = 0x8bac;
    pub const SCIENCE: usize = 0x8bb0;
    pub const HISTORIAN: usize = 0x8bc0;
    pub const SHIPS: usize = 0x8bd2;
    pub const SHIP_FLAGS: usize = 0x9172;
    pub const ARRIVAL: usize = 0x9178;
    pub const PALACE_LEVELS: usize = 0x9188;
    pub const PALACE_STYLES: usize = 0x91a0;
    pub const NAME_USED: usize = 0x91b8;
    pub const PALACE_LEVEL: usize = 0x93b8;
    pub const PEACE: usize = 0x93ba;
    pub const CIVS_LESS_ONE: usize = 0x93bc;
    pub const LAUNCH: usize = 0x93ce;
    pub const START_ALT: usize = 0x93de;
}

struct Sve<'a>(&'a [u8]);

impl Sve<'_> {
    fn b(&self, o: usize) -> u8 {
        self.0[o]
    }
    fn w(&self, o: usize) -> i16 {
        i16::from_le_bytes([self.0[o], self.0[o + 1]])
    }
    /// Word `i` of a table of words per civilization.
    fn civ_w(&self, o: usize, civ: usize) -> i16 {
        self.w(o + 2 * civ)
    }
    /// A NUL-terminated, blank-padded string of at most `n` bytes.
    fn text(&self, o: usize, n: usize) -> String {
        let raw = &self.0[o..o + n];
        let end = raw.iter().position(|c| *c == 0).unwrap_or(n);
        String::from_utf8_lossy(&raw[..end]).trim_end().to_string()
    }
}

/// The `CIVILn.SVE` of slot `n` in `dir`, in either case.
pub fn slot_file(dir: &Path, n: u8) -> Option<PathBuf> {
    [format!("CIVIL{n}.SVE"), format!("civil{n}.sve")].into_iter().map(|f| dir.join(f)).find(|p| p.is_file())
}

/// The .MAP next to a .SVE (same name, the extension's case following the file's).
fn map_file(sve: &Path) -> PathBuf {
    let upper = sve.extension().is_some_and(|e| e.to_string_lossy().chars().all(|c| !c.is_ascii_lowercase()));
    let p = sve.with_extension(if upper { "MAP" } else { "map" });
    if p.is_file() {
        p
    } else {
        sve.with_extension(if upper { "map" } else { "MAP" })
    }
}

/// The load menu's line for an original save (`read_save_header` o 0xee78, header_only): the
/// human civ (offset 2), year (8), difficulty (10), leader name and civ name.
pub fn read_header(path: &Path) -> Option<SlotInfo> {
    let data = std::fs::read(path).ok()?;
    if data.len() < SVE_LEN {
        return None;
    }
    let s = Sve(&data);
    let civ = (s.w(off::HUMAN) & 7) as usize;
    Some(SlotInfo {
        slot: 0,
        leader: s.text(off::LEADERS + 14 * civ, 14),
        civ: s.text(off::PLURALS + 12 * civ, 12),
        year: s.w(off::YEAR) as i32,
        difficulty: Difficulty::from_u8(s.w(off::DIFFICULTY) as u8).unwrap_or(Difficulty::Chieftain),
    })
}

/// Import `CIVILn.SVE` and the `CIVILn.MAP` next to it.
pub fn import(sve: &Path) -> Result<Game, SaveError> {
    let data = std::fs::read(sve)?;
    let map = std::fs::read(map_file(sve))?;
    let mut g = import_bytes(&data, &map)?;
    // load_game sets ds:0xdf60 = 2 for the slots from 4 on (o 0xf0bc, slot ds:0xe168), and
    // then move_units (0x40c0) gives the human's units their full moves again, as at the start
    // of a turn; slots 0..3 (ds:0xdf60 = 1) keep the saved moves
    if slot_of(sve).is_some_and(|n| n >= 4) {
        let extra = g.extra_sea_move(g.human_slot);
        let human = g.human_slot;
        for u in g.units.iter_mut().flatten().filter(|u| u.owner == human) {
            u.moves_left = u.full_moves() + if u.is_sea() && extra { 3 } else { 0 };
        }
        g.active_unit = None;
        g.active_unit = g.next_active_unit();
    }
    Ok(g)
}

/// The slot n of a CIVILn.SVE file name.
fn slot_of(sve: &Path) -> Option<u8> {
    let name = sve.file_name()?.to_str()?.to_ascii_uppercase();
    let n = name.strip_prefix("CIVIL")?.strip_suffix(".SVE")?;
    (n.len() == 1).then(|| n.parse().ok()).flatten()
}

/// The order a unit's flag byte (+0) and goto (+6) stand for (the order keys of `move_units`
/// 0x403e: 0x01 sentry, 0x04 fortify ('f' 0x5160), 0x08 fortified (0x4239), the Settlers jobs
/// in bits 0xc2: 0x02 road, 0x40 irrigation, 0x80 mine, 0xc0 fortress, 0x82 pollution).
fn unit_order(flags: u8, goto: bool, air: bool) -> UnitOrder {
    // bit 0x02 on an aircraft is its return to base ('h', 0x4f94), shown with no letter
    if air && flags & 0x02 != 0 {
        return UnitOrder::Home;
    }
    match flags & 0xc2 {
        0x02 => return UnitOrder::Road,
        0x40 => return UnitOrder::Irrigate,
        0x80 => return UnitOrder::Mine,
        0xc0 => return UnitOrder::Fortress,
        0x82 | 0xc2 => return UnitOrder::CleanPollution,
        _ => {}
    }
    if flags & 0x08 != 0 {
        UnitOrder::Fortified
    } else if flags & 0x04 != 0 {
        UnitOrder::Fortify
    } else if flags & 0x01 != 0 {
        UnitOrder::Sentry
    } else if goto {
        UnitOrder::Goto
    } else {
        UnitOrder::None
    }
}

/// A city's production byte +9: a unit type, or minus an improvement (1..24) or minus 24 plus
/// a wonder (process_city 0xe52d..0xe5ed).
fn production(p: i8) -> Production {
    match p {
        0.. => Production::Unit(p as u8),
        -24..=-1 => Production::Improvement((-p) as u8),
        _ => Production::Wonder((-(p as i32) - 24).clamp(1, 21) as u8),
    }
}

/// The map-page improvement bits (0x1a68:0x158b: 0x02 irrigation, 0x04 mine, 0x08 road, 0x10
/// railroad, 0x20 fortress, 0x40 pollution (0x12e8f), 0x80 the second river sprites) as civrs
/// tile flags; 0x01 (city) comes from the city records.
fn tile_bits(v: u8) -> u16 {
    let mut f = 0;
    for (bit, flag) in [
        (0x02, tile_flags::IRRIGATION),
        (0x04, tile_flags::MINE),
        (0x08, tile_flags::ROAD),
        (0x10, tile_flags::RAILROAD),
        (0x20, tile_flags::FORTRESS),
        (0x40, tile_flags::POLLUTION),
        (0x80, tile_flags::RIVER_ALT),
    ] {
        if v & bit != 0 {
            f |= flag;
        }
    }
    f
}

pub fn import_bytes(data: &[u8], map_pic: &[u8]) -> Result<Game, SaveError> {
    if data.len() < SVE_LEN {
        return Err(SaveError::Format(format!("a CIVILn.SVE has {SVE_LEN} bytes, this one {}", data.len())));
    }
    let page = crate::assets::pic::decode_pic(map_pic).map_err(|e| SaveError::Format(format!("map file: {e}")))?;
    if page.width < 320 || page.height < 200 {
        return Err(SaveError::Format(format!("map page is {}x{}", page.width, page.height)));
    }
    let px = |x: i32, y: i32| page.get(x as usize, y as usize);
    let s = Sve(data);
    let turn = s.w(off::TURN).max(0) as u32;
    let human = (s.w(off::HUMAN) & 7) as u8;
    let seed = s.w(off::SEED) as u16 as u32;
    let difficulty = Difficulty::from_u8(s.w(off::DIFFICULTY).clamp(0, 4) as u8).unwrap_or(Difficulty::Chieftain);
    let active = s.w(off::ACTIVE) as u16 as u8;
    let tribe = |c: usize| match s.civ_w(off::TRIBE, c) {
        t @ 0..=15 if t as usize & 7 == c && t != 8 => t as u8,
        _ => c as u8,
    };

    let setup = GameSetup {
        difficulty,
        num_civs: (s.w(off::CIVS_LESS_ONE) + 1).clamp(3, 7) as u8,
        human_civ: tribe(human as usize),
        leader_name: s.text(off::LEADERS + 14 * human as usize, 14),
        seed: seed as u64,
        ..Default::default()
    };
    let mut g = Game::new(setup, None);
    g.events.clear();
    g.ai_memory = Default::default();
    g.barbarian_memory = Default::default();
    g.history = Default::default();
    g.turn = turn;
    g.year = s.w(off::YEAR) as i32;
    g.human_slot = human;
    g.opening = false;
    g.hints_shown = 0;

    // the map page
    let vis = |x: i32, y: i32| s.b(off::VISIBILITY + 50 * x as usize + y as usize);
    let mut map = Map::empty(crate::game::WorldParams::normal(), seed);
    for y in 0..MAP_H as i32 {
        for x in 0..MAP_W as i32 {
            let terrain = Terrain::from_earth_code(px(x, y)).unwrap_or(Terrain::Ocean);
            let mut t = Tile { terrain, ..Default::default() };
            if t.is_land() {
                t.continent = px(x, y + 50);
            }
            t.special = Map::special_at(seed, x, y) && terrain.info().special.is_some();
            t.shield = Map::shield_at(x, y);
            t.flags = tile_bits(px(x, y + 100) & 15 | (px(x, y + 150) & 15) << 4);
            // the hut pattern, unless the hut is taken (bit 0 of the visibility byte)
            t.set(tile_flags::HUT, t.is_land() && Map::hut_at(seed, x, y) && vis(x, y) & 1 == 0);
            t.explored = vis(x, y) & 0xfe;
            map.tiles[Map::idx(x, y)] = t;
        }
    }
    map.site = (0..MAP_H as i32).flat_map(|y| (0..MAP_W as i32).map(move |x| (x, y))).map(|(x, y)| px(x + 80, y)).collect();
    g.barbarian_memory.occupant = (0..MAP_H as i32).flat_map(|y| (0..MAP_W as i32).map(move |x| (x, y))).map(|(x, y)| px(x + 160, y) & 7).collect();
    g.map = map;

    // civilizations
    let names: Vec<String> = (0..256).map(|i| s.text(off::CITY_NAMES + 13 * i, 13)).collect();
    let replay_len = (s.w(off::REPLAY_LEN).max(0) as usize).min(crate::game::replay::LOG_LIMIT);
    let mut log = data[off::REPLAY..off::REPLAY + replay_len].to_vec();
    let destroyed = destroyed_turns(&log);
    let mut players = Vec::with_capacity(8);
    for c in 0..8usize {
        let slot = c as u8;
        let civ = if c == 0 { 0 } else { tribe(c) };
        let mut p = Player::new(slot, civ, slot == human);
        p.alive = active & (1 << c) != 0 || c == 0;
        p.in_game = p.alive || destroyed[c].is_some();
        p.destroyed_turn = if p.alive { None } else { destroyed[c] };
        if p.in_game {
            p.leader = s.text(off::LEADERS + 14 * c, 14);
            let (plural, adj) = (s.text(off::PLURALS + 12 * c, 12), s.text(off::ADJECTIVES + 11 * c, 11));
            let tribe_info = &rules::CIVS[civ as usize];
            if c != 0 && (plural != tribe_info.plural || adj != tribe_info.adjective) && !plural.is_empty() {
                p.tribe_names = Some((plural, adj));
            }
        }
        p.start_civ = if c != 0 && s.w(off::START_ALT) & (1 << c) != 0 { slot + 8 } else { slot };
        p.govt = Govt::from_u8(s.civ_w(off::GOVERNMENT, c).clamp(0, 5) as u8).unwrap_or(Govt::Despotism);
        p.gold = s.civ_w(off::GOLD, c) as i32;
        p.beakers = s.civ_w(off::BEAKERS, c) as i32;
        let tax = (s.civ_w(off::TAX, c).clamp(0, 10) * 10) as u8;
        let sci = (s.civ_w(off::SCIENCE, c).clamp(0, 10) * 10) as u8;
        p.tax = tax;
        p.lux = 100u8.saturating_sub(tax).saturating_sub(sci);
        let mut adv = [0u8; 16];
        adv[..10].copy_from_slice(&data[off::ADVANCES + 10 * c..off::ADVANCES + 10 * c + 10]);
        p.advances = u128::from_le_bytes(adv) & ((1u128 << 72) - 1);
        p.research_count = s.civ_w(off::RESEARCH_COUNT, c).max(0) as u16;
        p.advance_source = data[off::SOURCES + 72 * c..off::SOURCES + 72 * (c + 1)].iter().map(|v| *v as i8).collect();
        for o in 0..8 {
            p.diplo[o] = s.w(off::DIPLOMACY + 16 * c + 2 * o) as u16;
        }
        // the revolution flag ds:0xe498[civ][0] & 4 (`Player::anarchy_turns`)
        p.anarchy_turns = (p.diplo[0] & 4 != 0) as u8;
        p.last_contact = s.civ_w(off::LAST_CONTACT, c) as i32;
        for k in 0..28 {
            p.unit_counter[k] = s.w(off::UNIT_COUNTS + 0x38 * c + 2 * k);
            p.units_lost_by_type[k] = s.w(off::LOSSES + 0x38 * c + 2 * k);
        }
        // the spaceship: grid es:0x16e0[civ], launched bit ds:0x4cd8, years ds:0xde10 / 0xe8a8
        let ship = &mut p.spaceship;
        for x in 0..crate::game::spaceship::GRID_W {
            for y in 0..crate::game::spaceship::GRID_H {
                let v = s.b(off::SHIPS + 0xb4 * c + 12 * x + y);
                ship.grid[x][y] = if v == 0xff { 0 } else { v.wrapping_add(1) };
            }
        }
        ship.recount();
        let arrival = s.civ_w(off::ARRIVAL, c);
        ship.arrival_year = (arrival != -1).then_some(arrival as i32);
        if s.w(off::SHIP_FLAGS) as u16 & (1 << c) != 0 {
            let launch = s.civ_w(off::LAUNCH, c) as i32;
            ship.launch_year = Some(launch);
            // one year a turn after the launch (0x26ed)
            ship.launched_turn = Some(turn.saturating_sub((g.year - launch).max(0) as u32));
        }
        if p.human {
            let r = s.w(off::RESEARCH);
            p.researching = (0..72).contains(&r).then_some(r as u8);
            p.future_techs = s.w(off::FUTURE_TECHS).clamp(0, 255) as u8;
            p.palace_level = s.w(off::PALACE_LEVEL).clamp(0, 255) as u8;
            let levels = (0..12).map(|i| s.w(off::PALACE_LEVELS + 2 * i) as i8 as u8);
            let styles = (0..12).map(|i| s.w(off::PALACE_STYLES + 2 * i) as u8);
            p.palace_parts = levels.chain(styles).collect();
            p.peace_turns = s.w(off::PEACE).max(0) as u32;
        }
        players.push(p);
    }
    g.players = players;

    // cities: the record index is the city id
    let mut cities: Vec<Option<City>> = Vec::with_capacity(128);
    for i in 0..128usize {
        let r = &data[off::CITIES + 0x1c * i..off::CITIES + 0x1c * (i + 1)];
        if r[6] == 0xff {
            cities.push(None);
            continue;
        }
        let owner = r[0xb] & 7;
        let imp = u32::from_le_bytes([r[0], r[1], r[2], r[3]]);
        let worked_exe = u32::from_le_bytes([r[0x10], r[0x11], r[0x12], r[0x13]]);
        let codes = u16::from_le_bytes([r[0x14], r[0x15]]);
        // worked bit i = radius square i of CIV.EXE's order (ds:0x2498 / 0x24c2), 20 the centre
        let mut worked = 0u32;
        for (b, d) in crate::game::economy::EXE_RADIUS[..20].iter().enumerate() {
            if worked_exe & (1 << b) != 0 {
                let idx = rules::CITY_RADIUS.iter().position(|q| q == d).unwrap();
                worked |= 1 << idx;
            }
        }
        let size = r[7];
        let spare = (size as i32 - worked.count_ones() as i32).max(0) as usize;
        // 2-bit codes at +0x14 (0xc91:0x6d97: 1 taxman, 2 scientist, 3 entertainer; past the
        // eighth always 1)
        let specialists = (0..spare)
            .map(|k| match if k < 8 { codes >> (2 * k) & 3 } else { 1 } {
                1 => Specialist::Taxman,
                2 => Specialist::Scientist,
                _ => Specialist::Entertainer,
            })
            .collect();
        let st = r[6];
        let city = City {
            id: i as u16,
            owner,
            founder: owner,
            name: names[r[0x16] as usize].clone(),
            x: r[4] as i32,
            y: r[5] as i32,
            size,
            food: i16::from_le_bytes([r[0xc], r[0xd]]) as i32,
            shields: i16::from_le_bytes([r[0xe], r[0xf]]) as i32,
            production: production(r[9] as i8),
            // bit (id - 1) of the record, bit id here; the spaceship parts have no bits
            improvements: (imp & 0x1f_ffff) << 1,
            wonders: 0,
            worked,
            specialists,
            trade_routes: r[0x17..0x1a].iter().filter(|c| **c < 128).map(|c| *c as u16).collect(),
            disorder: st & 0x01 != 0,
            disorder_turns: 0,
            celebrating: st & 0x40 != 0,
            bought: false,
            tech_stolen: st & 0x20 != 0,
            auto_build: st & 0x10 != 0,
            founded_turn: 0,
            pollution_pending: 0,
            caravan_shields: 0,
            garrison: [r[0x1a], r[0x1b]],
            garrison_out: st & 0x04 != 0,
            known_size: r[8],
            // +6 bit 0x2: set at founding next to a big ocean (o 0x1b6b9)
            coastal: st & 0x02 != 0,
            // +6 bit 0x8: set at founding next to River or Mountains (o 0x1b6d2)
            hydro_site: st & 0x08 != 0,
            name_index: r[0x16],
        };
        if st & 0x80 != 0 {
            g.players[owner as usize].sold_cities.push(i as u16);
        }
        cities.push(Some(city));
    }
    while cities.last().is_some_and(|c| c.is_none()) {
        cities.pop();
    }
    g.cities = cities;
    for i in 0..g.cities.len() {
        let Some((x, y, owner, worked, palace)) = g.cities[i].as_ref().map(|c| (c.x, c.y, c.owner, c.worked, c.has_improvement(rules::improvement_ids::PALACE))) else { continue };
        if let Some(t) = g.map.get_mut(x, y) {
            t.city = Some(i as u16);
            t.set(tile_flags::HUT, false);
        }
        for idx in 1..21u8 {
            if worked & (1 << idx) != 0 {
                if let Some((tx, ty)) = g.radius_tile(i as u16, idx) {
                    g.map.get_mut(tx, ty).unwrap().worked_by = Some(i as u16);
                }
            }
        }
        if palace {
            g.players[owner as usize].capital = Some(i as u16);
        }
    }
    // wonder cities ds:0x6ea8[w]: -1 not built, 0x80 destroyed
    for w in 1..22usize {
        g.wonders_built[w] = match s.w(off::WONDERS + 2 * w) {
            c @ 0..=127 if g.city_opt(c as u16).is_some() => {
                g.city_mut(c as u16).add_wonder(w as u8);
                Some(c as u16)
            }
            -1 => None,
            _ => Some(WONDER_LOST),
        };
    }
    g.wonders_built[0] = None;
    g.nukes_enabled = g.wonders_built[rules::wonder_ids::MANHATTAN_PROJECT as usize].is_some();
    g.apollo_built = g.wonders_built[rules::wonder_ids::APOLLO_PROGRAM as usize].is_some();

    // units, civ by civ in record order
    let mut units: Vec<Option<Unit>> = Vec::new();
    for c in 0..8usize {
        for i in 0..128usize {
            let r = &data[off::UNITS + 0x600 * c + 12 * i..off::UNITS + 0x600 * c + 12 * (i + 1)];
            let kind = r[3];
            if kind == 0xff || kind as usize >= rules::UNITS.len() {
                continue;
            }
            let id = units.len() as u16;
            let mut u = Unit::new(id, c as u8, kind, r[1] as i32, r[2] as i32, turn);
            u.record = i as u8;
            u.moves_left = r[4];
            u.veteran = r[0] & 0x20 != 0;
            u.goto = (r[6] != 0xff).then_some((r[6] as i32, r[7] as i32));
            u.order = unit_order(r[0], u.goto.is_some(), u.is_air());
            // +5: the turns of fuel left of an aircraft, else the Settlers' work counter; the turn
            // end refuels to the range and counts the turn at once (0x7597, 0x75ab), so a full
            // tank is saved as range - 1 and civrs's turns flown are range - 1 - fuel
            if u.is_air() {
                u.fuel = u.info().range.saturating_sub(1).saturating_sub(r[5]);
            } else {
                u.order_turns = r[5];
            }
            u.home = (r[0xb] < 128).then_some(r[0xb] as u16).filter(|h| g.city_opt(*h).is_some());
            u.seen_by = r[9];
            u.leader = c == 0 && kind == rules::unit_ids::DIPLOMAT;
            if r[8] != 0xff {
                let heading = if c == 0 { &mut g.barbarian_memory.heading } else { &mut g.ai_memory.heading };
                heading.insert(id, r[8]);
            }
            units.push(Some(u));
        }
    }
    // cargo: land units at sea on a ship, aircraft at sea on a Carrier, in record order
    let mut room: Vec<i32> = units.iter().map(|u| u.as_ref().map_or(0, |u| u.info().capacity as i32)).collect();
    for i in 0..units.len() {
        let Some(u) = units[i].clone() else { continue };
        if !(u.is_land() || u.is_air()) || !g.map.is_ocean(u.x, u.y) {
            continue;
        }
        let ship = (0..units.len()).find(|&j| {
            units[j].as_ref().is_some_and(|s| s.owner == u.owner && s.pos() == u.pos() && room[j] > 0 && if u.is_land() { s.carries_land_units() } else { s.kind == rules::unit_ids::CARRIER })
        });
        if let Some(j) = ship {
            room[j] -= 1;
            units[i].as_mut().unwrap().carried_by = Some(j as u16);
        }
    }
    g.units = units;

    // AI goals ds:0xd76e.. and continent policies ds:0xe698
    for c in 0..8usize {
        for i in 0..crate::game::goals::PERSISTENT_GOALS {
            let at = 16 * c + i;
            g.ai_memory.persistent[c][i] = Goal {
                x: data[off::GOAL_X + at] as i8,
                y: data[off::GOAL_Y + at] as i8,
                kind: data[off::GOAL_TYPE + at] as i8,
                priority: data[off::GOAL_PRIORITY + at] as i8,
            };
        }
        if g.players[c].alive && !g.players[c].human {
            let mut pol = [0u8; 16];
            for (k, v) in pol.iter_mut().enumerate() {
                *v = s.w(off::POLICIES + 32 * c + 2 * k) as u8;
            }
            g.ai_memory.policies[c] = Some(pol);
        }
    }
    // the census of each civ's last AI turn (SVE 0x0788: ds:0x6cac defence, 0x807a attack,
    // 0x6b96 cities per civ and continent; 0x0708 ds:0xd2f8 citizens, 0x0718 ds:0x6e82
    // strength, 0x0728 ds:0x803c power rank); land squares and sites are counted from the map
    let live = g.live_census();
    {
        let m = &mut g.ai_memory;
        for c in 0..8usize {
            for k in 0..16 {
                let at = 32 * c + 2 * k;
                m.census.defence[c][k] = s.w(off::CENSUS + at) as i32;
                m.census.attack[c][k] = s.w(off::CENSUS + 0x100 + at) as i32;
                m.census.cities[c][k] = s.w(off::CENSUS + 0x200 + at) as i32;
            }
            m.census.citizens[c] = s.civ_w(off::CITIZENS, c) as i32;
            m.census.strength[c] = s.civ_w(off::STRENGTH, c) as i32;
            m.ranks[c] = s.civ_w(off::RANKS, c).clamp(0, 7) as u8;
        }
        m.census.squares = live.squares;
        m.census.sites = live.sites;
        let h = g.human_slot as usize;
        m.census.faces_human = (0..8).filter(|&c| h < 8 && (1..15).any(|k| m.census.defence[c][k] != 0 && m.census.cities[h][k] > 1)).fold(0, |f, c| f | 1 << c);
    }

    // global tables
    g.first_discoverer = (0..72).map(|a| s.w(off::FIRST + 2 * a) as u8 & 7).collect();
    g.polluted_tiles = s.w(off::POLLUTED).max(0) as u16;
    g.warming_counter = s.w(off::WARMING_COUNTER).clamp(0, 99) as u16;
    g.global_warming_count = s.w(off::WARMINGS).clamp(0, 255) as u8;
    g.historian_timer = s.w(off::HISTORIAN).max(0) as u32;
    let o = s.w(off::OPTIONS) as u8;
    let bit = |n: u8| o & (1 << n) != 0;
    g.options = crate::game::Options {
        instant_advice: bit(0),
        autosave: bit(1),
        end_turn_confirm: bit(2),
        animations: bit(3),
        sound: bit(4),
        enemy_moves: bit(5),
        civilopedia_text: bit(6),
        palace: bit(7),
    };

    // the replay log (city names by index into the 256-name table, remapped to civrs's list,
    // which also holds every name given out: far 0x540[name] != 0xff) and the power history
    let mut logged: Vec<String> = Vec::new();
    let mut at = 0;
    while at + 1 < log.len() {
        let k = log[at] >> 4;
        let n = crate::game::replay::arg_count(k);
        if at + 2 + n > log.len() {
            log.truncate(at);
            break;
        }
        if matches!(k, crate::game::replay::kind::CITY | crate::game::replay::kind::CAPTURE) {
            let name = &names[log[at + 3] as usize];
            let idx = logged.iter().position(|l| l == name).unwrap_or_else(|| {
                logged.push(name.clone());
                logged.len() - 1
            });
            log[at + 3] = idx as u8;
        }
        at += 2 + n;
    }
    // a renamed city's entry holds its new name; its first name (civrs's fixed table) stays
    // given out as well (`Game::set_city_name`)
    let table = crate::rules::city_names::name_table();
    for (i, name) in names.iter().enumerate() {
        if s.b(off::NAME_USED + i) == 0xff {
            continue;
        }
        for n in [name.as_str(), table.get(i).copied().unwrap_or("")] {
            if !n.is_empty() && !logged.iter().any(|l| l == n) && logged.len() < 255 {
                logged.push(n.to_string());
            }
        }
    }
    g.history.log = log;
    g.history.names = logged;
    let power = &data[off::POWER..off::POWER + 8 * crate::game::replay::POWER_ROWS];
    if power.iter().any(|v| *v != 0) {
        g.history.power = power.to_vec();
    }
    // unit types built: bit 3 of the first-discoverer words (0xdf46)
    g.history.units_built = (0..28).filter(|t| s.w(off::FIRST + 2 * t) & 8 != 0).fold(0, |m, t| m | 1 << t);

    g.refresh_all_vision();
    g.update_research_catch_up();
    g.active_unit = None;
    g.active_unit = g.next_active_unit();
    Ok(g)
}

/// CIV.EXE's goto land links of a save (ds:0xdb44, `Game::use_exe_land_links`).
pub fn exe_land_links(sve: &Path) -> Result<Vec<u8>, SaveError> {
    let data = std::fs::read(sve)?;
    data.get(off::LAND_LINKS..off::LAND_LINKS + 260).map(|l| l.to_vec()).ok_or_else(|| SaveError::Format("short save".into()))
}

/// The stored game state as `key value` lines, for the differential test against CIV.EXE
/// (`civrs --load-sve F --dump state:TURNS`, reference/tools/sve_diff.py): the same lines
/// from civrs's turns and from the import of CIV.EXE's own save after those turns diff field
/// by field. Units are keyed by owner, type and their order of position, cities by record id.
pub fn state_text(g: &Game) -> String {
    use std::fmt::Write;
    let mut o = String::new();
    let _ = writeln!(o, "turn {}\nyear {}", g.turn, g.year);
    for p in g.players.iter().filter(|p| p.alive) {
        let k = format!("civ{}", p.slot);
        let _ = writeln!(o, "{k}.gold {}\n{k}.beakers {}\n{k}.rates tax{} lux{} sci{}", p.gold, p.beakers, p.tax, p.lux, p.sci());
        let _ = writeln!(o, "{k}.govt {:?} revolution_flag {}\n{k}.advances {:018x}\n{k}.research_count {}", p.govt, p.anarchy_turns, p.advances, p.research_count);
        if p.human {
            let _ = writeln!(o, "{k}.researching {:?}", p.researching);
        }
        let _ = writeln!(o, "{k}.units_per_type {:?}", &p.unit_counter[..]);
        let _ = writeln!(o, "{k}.cities {}", g.all_cities().filter(|c| c.owner == p.slot).count());
    }
    for c in g.all_cities() {
        let k = format!("city{}", c.id);
        let y = g.city_yield(c.id);
        let _ = writeln!(o, "{k}.name {} owner {} at {},{}", c.name, c.owner, c.x, c.y);
        let _ = writeln!(o, "{k}.size {}\n{k}.food {}\n{k}.shields {}\n{k}.production {:?}", c.size, c.food, c.shields, c.production);
        let _ = writeln!(o, "{k}.improvements {:08x} wonders {:08x}", c.improvements, c.wonders);
        let _ = writeln!(o, "{k}.worked {:08x} specialists {:?}", c.worked, c.specialists);
        let sold = g.player(c.owner).sold_cities.contains(&c.id);
        let _ = writeln!(o, "{k}.flags disorder {} celebrating {} known_size {} auto {} sold {sold}", c.disorder, c.celebrating, c.known_size, c.auto_build);
        let _ = writeln!(o, "{k}.yield food {}-{} shields {}-{} trade {}-{} tax {} lux {} sci {}", y.food, y.food_upkeep, y.shields, y.shield_upkeep, y.trade, y.corruption, y.tax, y.lux, y.sci);
    }
    let mut units: Vec<&Unit> = g.all_units().collect();
    units.sort_by_key(|u| (u.owner, u.kind, u.x, u.y, u.id));
    let mut last = (u8::MAX, u8::MAX);
    let mut n = 0;
    for u in units {
        n = if (u.owner, u.kind) == last { n + 1 } else { 0 };
        last = (u.owner, u.kind);
        let _ = writeln!(
            o,
            "unit{}.{}.{} at {},{} moves {} order {:?} turns {} fuel {} vet {} home {:?} goto {:?} heading {}",
            u.owner,
            u.info().name,
            n,
            u.x,
            u.y,
            u.moves_left,
            u.order,
            u.order_turns,
            u.fuel,
            u.veteran,
            u.home,
            u.goto,
            g.heading(u.id)
        );
    }
    // the terrain, a letter per square (Terrain order: Desert .. River)
    for y in 0..MAP_H as i32 {
        let row: String = (0..MAP_W as i32).map(|x| b"dpgfhmtasjor"[g.map.tile(x, y).terrain as usize] as char).collect();
        let _ = writeln!(o, "terrain.y{y} {row}");
    }
    for y in 0..MAP_H as i32 {
        for x in 0..MAP_W as i32 {
            let t = g.map.tile(x, y);
            let f = t.flags & !tile_flags::RIVER_ALT;
            if f != 0 {
                let _ = writeln!(o, "tile{x},{y} {f:04x}");
            }
        }
    }
    let _ = writeln!(o, "polluted {}\nwarming {}", g.polluted_tiles, g.warming_counter);
    o
}

/// The turn of each slot's DESTROYED record in a replay log.
fn destroyed_turns(log: &[u8]) -> [Option<u32>; 8] {
    let mut out = [None; 8];
    let mut at = 0;
    while at + 1 < log.len() {
        let k = log[at] >> 4;
        let n = crate::game::replay::arg_count(k);
        if k == crate::game::replay::kind::DESTROYED {
            if let Some(&civ) = log.get(at + 2) {
                out[civ as usize & 7] = Some(((log[at] as u32 & 15) << 8) | log[at + 1] as u32);
            }
        }
        at += 2 + n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::turn::TurnStep;
    use crate::game::DecisionReply;

    #[test]
    fn bit_2_is_an_aircraft_s_return_to_base_and_a_settlers_road() {
        assert_eq!(unit_order(0x02, false, true), UnitOrder::Home);
        assert_eq!(unit_order(0x02, false, false), UnitOrder::Road);
        assert_eq!(unit_order(0x00, true, true), UnitOrder::Goto);
    }

    fn sample(n: u8) -> Option<Game> {
        let dir = crate::assets::AssetDir::locate(None).ok()?;
        let path = slot_file(dir.root(), n)?;
        Some(import(&path).expect("import"))
    }

    #[test]
    fn imports_an_original_save() {
        let Some(g) = sample(0) else { return };
        assert_eq!((g.turn, g.year, g.year_text()), (35, -3300, "3300 BC".to_string()));
        assert_eq!(g.difficulty(), Difficulty::Chieftain);
        assert_eq!(g.human_slot, 1);
        assert_eq!(g.map.seed, 8578);
        let plurals: Vec<&str> = g.alive_slots().iter().map(|s| g.player(*s).plural()).collect();
        assert_eq!(plurals, vec!["Romans", "Babylonians", "Germans", "Aztecs"]);
        assert_eq!(g.player(4).civ, 12);
        assert_eq!(g.human().leader, "Caesar");
        assert_eq!(g.human().gold, 85);
        assert_eq!((g.human().tax, g.human().lux, g.human().sci()), (50, 0, 50));
        assert_eq!(g.all_cities().count(), 4);
        let rome = g.city(0);
        assert_eq!((rome.name.as_str(), rome.pos(), rome.size, rome.shields, rome.owner), ("Rome", (12, 29), 2, 15, 1));
        assert_eq!(rome.production, Production::Improvement(rules::improvement_ids::GRANARY));
        assert!(rome.has_improvement(rules::improvement_ids::PALACE));
        assert_eq!(g.human().capital, Some(0));
        assert_eq!(rome.workers(), 2);
        assert_eq!(g.city(1).name, "Berlin");
        assert_eq!(g.city(1).owner, 3);
        assert_eq!(g.units.iter().flatten().count(), 5);
        let settlers = g.all_units().find(|u| u.owner == 1 && u.kind == rules::unit_ids::SETTLERS).unwrap();
        assert_eq!(settlers.pos(), (10, 25));
        // the map: cities on land, the human sees his surroundings, the replay starts with Rome
        assert!(g.all_cities().all(|c| g.map.is_land(c.x, c.y) && g.map.tile(c.x, c.y).city == Some(c.id)));
        assert!(g.explored(1, 12, 29));
        assert!(g.map.count_land() > 400);
        let first = &g.history.records()[0];
        assert_eq!((first.turn, first.kind, first.args[0]), (0, crate::game::replay::kind::CITY, 1));
        assert_eq!(g.history.name(first.args[1]), "Rome");
    }

    #[test]
    fn imports_the_census_of_each_civs_last_ai_turn() {
        // ai_civ_turn runs after the civ's cities and the game is saved in the human's moves,
        // so the snapshot holds the human's cities as they were then (CIVIL1 has a city
        // founded since: the save's own city count, SVE 0x06c8, does not have it either)
        for n in [0u8, 1, 3, 4] {
            let Some(g) = sample(n) else { continue };
            let h = g.human_slot;
            let sizes: i32 = g.all_cities().filter(|c| c.owner == h).map(|c| c.size as i32).sum();
            let c = &g.ai_memory.census;
            assert!(c.citizens[h as usize] <= sizes && (n != 0 || c.citizens[h as usize] == sizes), "CIVIL{n}");
            assert!(c.strength[h as usize] > 0, "CIVIL{n}");
            let on_land: i32 = c.cities[h as usize].iter().sum();
            assert!(on_land >= 1 && on_land <= g.all_cities().filter(|x| x.owner == h).count() as i32, "CIVIL{n}");
        }
    }

    #[test]
    fn the_coastal_bits_civ_exe_saved_follow_civrs_founding_rule() {
        for n in [0u8, 1, 3, 4] {
            let Some(mut g) = sample(n) else { continue };
            let cities: Vec<_> = g.all_cities().map(|c| (c.name.clone(), c.owner, c.x, c.y, c.coastal)).collect();
            for (name, owner, x, y, coastal) in cities {
                assert_eq!(coastal, g.founding_coastal(owner, x, y), "CIVIL{n} {name}");
            }
        }
    }

    #[test]
    fn the_hydro_site_bits_civ_exe_saved_follow_civrs_founding_rule() {
        let mut seen = [0; 2];
        for n in [0u8, 1, 3, 4] {
            let Some(g) = sample(n) else { continue };
            for c in g.all_cities() {
                assert_eq!(c.hydro_site, g.founding_hydro_site(c.x, c.y), "CIVIL{n} {}", c.name);
                seen[c.hydro_site as usize] += 1;
            }
        }
        assert!(seen[0] + seen[1] == 0 || seen.iter().all(|n| *n > 0), "both kinds among the samples: {seen:?}");
    }

    #[test]
    fn a_full_aircraft_is_saved_with_range_minus_one_turns_of_fuel() {
        // the turn end sets +5 to the range and counts the turn at once (0x7597, 0x75ab): a
        // Bomber saved with 1 has flown no turn, one saved with 0 has flown one
        let Ok(dir) = crate::assets::AssetDir::locate(None) else { return };
        let Some(path) = slot_file(dir.root(), 0) else { return };
        let (mut data, map) = (std::fs::read(&path).unwrap(), std::fs::read(map_file(&path)).unwrap());
        let (c, i) = (0..8).flat_map(|c| (0..128).map(move |i| (c, i))).find(|(c, i)| data[off::UNITS + 0x600 * c + 12 * i + 3] != 0xff).unwrap();
        let at = off::UNITS + 0x600 * c + 12 * i;
        data[at + 3] = rules::unit_ids::BOMBER;
        for (saved, flown) in [(1u8, 0u8), (0, 1)] {
            data[at + 5] = saved;
            let g = import_bytes(&data, &map).unwrap();
            let u = g.all_units().find(|u| u.owner as usize == c && u.record as usize == i).unwrap();
            assert_eq!((u.kind, u.fuel), (rules::unit_ids::BOMBER, flown));
        }
    }

    #[test]
    fn a_load_from_slot_4_on_gives_the_human_units_their_moves() {
        // CIVIL4's Militia was saved with no moves left; ds:0xdf60 = 2 restores them, and the
        // records keep their numbers (the human's Militia is record 0)
        let Some(g) = sample(4) else { return };
        let h = g.human_slot;
        let m = g.units_of(h)[0];
        assert_eq!((g.unit(m).kind, g.unit(m).moves_left, g.unit(m).record), (rules::unit_ids::MILITIA, 3, 0));
        assert_eq!(g.active_unit, Some(m));
        let Some(g0) = sample(0) else { return };
        let s = g0.all_units().find(|u| u.owner == g0.human_slot && u.kind == rules::unit_ids::SETTLERS).unwrap();
        assert_eq!(s.moves_left, 0, "slots 0..3 keep the saved moves");
    }

    #[test]
    fn a_city_that_builds_settlers_pays_taxes_on_the_squares_it_worked() {
        // CIVIL4's Caesarea (size 2): 0xdfe3 lowers the size but the trade totals of the turn stay
        // (ds:0x70de), so the taxes are those of both squares; the city screen the human is
        // shown then puts the workers in order
        let Some(mut g) = sample(4) else { return };
        let caesarea = 4;
        g.city_mut(caesarea).production = Production::Unit(rules::unit_ids::SETTLERS);
        g.city_mut(caesarea).shields = 39;
        let tax = g.city_yield_raw(caesarea).tax;
        let gold = g.human().gold;
        g.process_city(caesarea);
        assert_eq!(g.city(caesarea).size, 1);
        assert_eq!(g.human().gold - gold, tax);
        assert_eq!(g.city(caesarea).workers(), 1);
        assert!(g.city_yield_raw(caesarea).tax < tax, "one square makes less");
    }

    #[test]
    fn the_state_dump_lists_the_saved_fields() {
        let Some(mut g) = sample(3) else { return };
        let text = state_text(&g);
        assert!(text.starts_with("turn 73\nyear -2540\n"));
        assert!(text.contains("\ncity0.name Rome owner 1 at 12,29\n"));
        assert!(text.contains("\nciv1.gold 147\n"));
        let dir = crate::assets::AssetDir::locate(None).unwrap();
        let land = exe_land_links(&slot_file(dir.root(), 3).unwrap()).unwrap();
        g.use_exe_land_links(&land);
        assert_eq!(g.ai_memory.links.as_ref().unwrap().land, land);
    }

    #[test]
   fn reads_the_load_menu_line() {
        let Ok(dir) = crate::assets::AssetDir::locate(None) else { return };
        let Some(path) = slot_file(dir.root(), 3) else { return };
        let h = read_header(&path).unwrap();
        assert_eq!(crate::ui::loadgame::slot_line(&Some(h)), "Chief Caesar, Romans/2540 BC");
    }

    #[test]
    fn plays_on_from_an_imported_save() {
        for n in [0, 1, 3, 4] {
            let Some(mut g) = sample(n) else { return };
            let mut ai = crate::ai::Ai::new(1);
            let start = g.turn;
            for _ in 0..30 {
                let mut r = g.end_turn();
                let mut guard = 0;
                loop {
                    match r.step(&mut g, &mut ai) {
                        TurnStep::Done => break,
                        TurnStep::Decision(_) => r.resolve(&mut g, DecisionReply::Choice(0)),
                        _ => {}
                    }
                    guard += 1;
                    assert!(guard < 100_000);
                }
                let _ = g.take_events();
            }
            assert_eq!(g.turn, start + 30);
            // and it saves in civrs's format
            let path = std::env::temp_dir().join(format!("civrs-sve-import-{n}.sav"));
            crate::save::save_game(&path, &g).unwrap();
            let back = crate::save::load_game(&path).unwrap();
            assert_eq!(back.turn, g.turn);
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn the_map_page_is_the_world_civ_exe_made() {
        // CIVIL0 is a normal world of clock seed 8578: terrain, continents, specials and river
        // sprites are the generator's, one hut is taken, the cities claimed site squares
        let Some(g) = sample(0) else { return };
        let w = crate::game::mapgen::generate(crate::game::WorldParams::normal(), 8578);
        let squares = || (0..MAP_H as i32).flat_map(|y| (0..MAP_W as i32).map(move |x| (x, y)));
        for (x, y) in squares() {
            let (a, b) = (g.map.tile(x, y), w.map.tile(x, y));
            assert_eq!((a.terrain, a.continent, a.special, a.has(tile_flags::RIVER_ALT)), (b.terrain, b.continent, b.special, b.has(tile_flags::RIVER_ALT)), "({x},{y})");
        }
        let huts = |m: &Map| m.tiles.iter().filter(|t| t.has(tile_flags::HUT)).count();
        assert_eq!((huts(&g.map), huts(&w.map)), (27, 28));
        assert!(squares().filter(|&(x, y)| g.map.site_at(x, y) != w.map.site_at(x, y)).all(|(x, y)| g.all_cities().any(|c| Map::dist(c.pos(), (x, y)) <= 3)));
    }
}
