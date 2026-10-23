//! World generation, ported from CIV.EXE's world generator (overlay 7, `generate_world`
//! o 0x08d87) and the start of a new game (overlay 5: `civ_init` o 0x0493c, `start_report`
//! o 0x0515b).
//!
//! CIV.EXE builds the world as pixels of graphics page 1 (`ds:0xaa` page index 1) and copies
//! the whole page to the map page (page 2) when it is done (o 0x0933b). The map layers live in
//! bands of that page (see the catalogue's segment 0x1a68): terrain colour codes at (x, y),
//! continent / ocean numbers at (x, y+50), the improvement nibbles at (x, y+100) and
//! (x, y+150), the site-value layer at (x+80, y). This module keeps that page (`Page`) while
//! generating, so the order of the passes, their scratch areas and their random numbers
//! (`MsRand`, CIV.EXE's `random(n)`) are the original's, and then converts it into a `Map`.
//!
//! Unverified: CIV.EXE seeds `rand()` from the clock and draws random numbers in the intro
//! before the world is made; civrs starts the generator from the game seed (`MsRand::new`).
//! The pages are assumed linear (320 bytes per row, the 256-colour driver), which only matters
//! for the erosion pass stepping off the map (it reads 0 there and changes nothing).

use super::map::{tile_flags, Map, Slot, Tile, WorldParams};
use super::rng::MsRand;
use crate::assets::EarthMap;
use crate::rules::{Terrain, MAP_H, MAP_W};

const W: i32 = MAP_W as i32;
const H: i32 = MAP_H as i32;

/// The colour codes the generator paints, which are also MAP.PIC's codes and the codes of the
/// map page (`TerrainInfo::earth_code`; ds:0x2ba6 maps them to terrain ids). While the land
/// grows, a pixel counts how many land blobs covered it (0 = sea).
pub mod code {
    pub const OCEAN: u8 = 1;
    pub const FOREST: u8 = 2;
    pub const SWAMP: u8 = 3;
    pub const PLAINS: u8 = 6;
    pub const TUNDRA: u8 = 7;
    /// Marker under a river square at (x, y+50): the square uses the second set of river
    /// sprites (copied to the improvement band, `tile_flags::RIVER_ALT`).
    pub const RIVER_ALT: u8 = 8;
    pub const RIVER: u8 = 9;
    pub const GRASSLAND: u8 = 10;
    pub const JUNGLE: u8 = 11;
    pub const HILLS: u8 = 12;
    pub const MOUNTAINS: u8 = 13;
    pub const DESERT: u8 = 14;
    pub const ARCTIC: u8 = 15;
}

/// CIV.EXE's neighbour offsets ds:0x1882 (dx) / ds:0x18e4 (dy): 0 the square itself, 1..8 N,
/// NE, E, SE, S, SW, W, NW, 9..20 the rest of the 5x5 square without its corners, 21..24 the
/// corners, 25..48 the ring at distance 3.
#[rustfmt::skip]
pub const DX: [i32; 49] = [0, 0, 1, 1, 1, 0, -1, -1, -1, 0, 1, 2, 2, 2, 1, 0, -1, -2, -2, -2, -1, 2, 2, -2, -2,
    0, 1, 2, 3, 3, 3, 3, 3, 2, 1, 0, -1, -2, -3, -3, -3, -3, -3, -2, -1, 3, 3, -3, -3];
#[rustfmt::skip]
pub const DY: [i32; 49] = [0, -1, -1, 0, 1, 1, 1, 0, -1, -2, -2, -1, 0, 1, 2, 2, 2, 1, 0, -1, -2, 2, -2, -2, 2,
    -3, -3, -3, -2, -1, 0, 1, 2, 3, 3, 3, 3, 3, 2, 1, 0, -1, -2, -3, -3, 3, -3, 3, -3];

/// Site score of each terrain (0..11) and of its special-resource variant (12..23), as
/// `compute_site_values` (o 0x09c5f) builds it from the terrain table ds:0x28e (0x13 bytes per
/// terrain, specials following): 3 * food + trade, plus 2 * shields except for Grassland and
/// River, plus the bonus `-1 - v` of the mine effect (ds:0x44e) when that is a bonus (< 0), else
/// twice the bonus of the irrigation effect (ds:0x44a): the mine path (o 0x09d01) jumps past
/// the doubling at o 0x09ca1. Checked against the site layer of a CIV.EXE save (all 354 squares).
pub const SITE_SCORE: [i32; 24] = [3, 7, 8, 7, 6, 3, 3, 0, 3, 3, 5, 7, 12, 11, 8, 13, 10, 9, 9, 6, 11, 7, 11, 7];

/// One 320x200 graphics page (256-colour driver: one byte per pixel, rows of 320 bytes).
#[derive(Clone)]
pub struct Page {
    px: Vec<u8>,
}

impl Default for Page {
    fn default() -> Self {
        Page::new()
    }
}

impl Page {
    pub fn new() -> Page {
        Page { px: vec![0; 320 * 200] }
    }

    /// `get_pixel` (0x7e6:0x25f4). Off the page reads 0.
    pub fn get(&self, x: i32, y: i32) -> u8 {
        let i = y * 320 + x;
        if (0..64000).contains(&i) {
            self.px[i as usize]
        } else {
            0
        }
    }

    /// `set_pixel` (0x1a68:0x17d5 / 0:0x104f). Off the page writes nothing.
    pub fn set(&mut self, x: i32, y: i32, v: u8) {
        let i = y * 320 + x;
        if (0..64000).contains(&i) {
            self.px[i as usize] = v;
        }
    }

    /// Driver fill (0:0xbfa).
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, v: u8) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, v);
            }
        }
    }

    /// Driver copy (0:0x7fb) within the page; the generator only copies between areas that
    /// do not overlap.
    pub fn copy(&mut self, x: i32, y: i32, w: i32, h: i32, to_x: i32, to_y: i32) {
        for yy in 0..h {
            for xx in 0..w {
                let v = self.get(x + xx, y + yy);
                self.set(to_x + xx, to_y + yy, v);
            }
        }
    }

    /// Terrain id of the colour code at (x,y) (ds:0x2ba6; `gen_terrain_at` o 0x0a4a7).
    pub fn terrain(&self, x: i32, y: i32) -> Option<Terrain> {
        Terrain::from_earth_code(self.get(x, y))
    }

    fn is_ocean(&self, x: i32, y: i32) -> bool {
        self.terrain(x, y) == Some(Terrain::Ocean)
    }

    /// Continent / ocean number (`gen_plane_at` o 0x0a4c8: the pixel at (x, y+50)).
    pub fn plane(&self, x: i32, y: i32) -> u8 {
        self.get(x, y + 50)
    }

    /// A page holding `map`'s terrain codes, for the numbering passes on a finished map.
    pub fn from_map(map: &Map) -> Page {
        let mut p = Page::new();
        for y in 0..H {
            for x in 0..W {
                p.set(x, y, map.terrain(x, y).info().earth_code);
            }
        }
        p
    }
}

/// What the world generator leaves besides the map: its random number generator (the next
/// number goes to the new-game setup) and the tables of o 0x097a8 / o 0x09c5f.
#[derive(Clone)]
pub struct World {
    pub map: Map,
    pub rng: MsRand,
    /// ds:0xdcfe: squares of land continent 1..14 (index 15, all smaller ones, holds 1).
    pub land_sizes: [u16; 16],
    /// ds:0xdd7e: the same for the bodies of water.
    pub ocean_sizes: [u16; 16],
    /// ds:0xb1ee: per continent number, the squares that got a site value (Plains,
    /// Grassland and River in columns 2..77, rows 2..47).
    pub sites: [u16; 16],
    /// ds:0x7f38: sea connectivity of the 20x13 grid of 4x4 blocks (`sea_grid`).
    pub sea_grid: [u8; 260],
}

/// Generate a random world (`generate_world` o 0x08d87) from the game seed: the generator
/// starts at state `seed`, the special-resource and hut patterns use `seed & 0x7fff` (CIV.EXE
/// keeps the clock seed ds:0x6e80 for them).
pub fn generate(params: WorldParams, seed: u32) -> World {
    generate_from(params, MsRand::new(seed), (seed & 0x7fff) as u16)
}

/// `generate_world` with an explicit generator state and map seed (ds:0x6e80).
pub fn generate_from(params: WorldParams, mut rng: MsRand, map_seed: u16) -> World {
    let mut page = Page::new();
    let lm = params.land_mass as i32;
    // 0x8dcd: the page starts empty; 0x8de8: land blobs until enough squares were raised
    let mut raised = 0;
    loop {
        landmass_pass(&mut page, &mut rng, &mut raised);
        if 640 + 320 * lm <= raised {
            break;
        }
    }
    // 0x8e05: two rings of sea around the edge (outline 0x1c83:0xa0b)
    for (x0, y0, x1, y1) in [(0, 0, 79, 49), (1, 1, 78, 48)] {
        for x in x0..=x1 {
            page.set(x, y0, 0);
            page.set(x, y1, 0);
        }
        for y in y0..=y1 {
            page.set(x0, y, 0);
            page.set(x1, y, 0);
        }
    }
    join_diagonals(&mut page);
    climate_by_latitude(&mut page, &mut rng, params.temperature as i32);
    rainfall(&mut page, &mut rng, params.climate as i32);
    erosion(&mut page, &mut rng, params.age as i32);
    rivers(&mut page, &mut rng, lm, params.climate as i32);
    // 0x92ed: clear the right half (scratch of the passes above)
    page.fill(160, 0, 160, 200, 0);
    finish(page, rng, Map::empty(params, map_seed as u32))
}

/// `landmass_pass` (o 0x09633): one random blob drawn in the scratch area (x+160, y) and
/// added to the land: every covered square rises by one (ds:0x67fe counts the raises).
fn landmass_pass(page: &mut Page, rng: &mut MsRand, raised: &mut i32) {
    page.fill(160, 0, 80, 50, 0);
    let x = rng.random(72) + 4;
    let y = rng.random(34) + 8;
    landmass_blob(page, rng, x, y);
    for x in 0..80 {
        for y in 0..50 {
            if page.get(x + 160, y) != 0 {
                let v = page.get(x, y);
                page.set(x, y, v.wrapping_add(1));
                *raised += 1;
            }
        }
    }
}

/// `landmass_blob` (o 0x096fd): a random walk of 1..64 orthogonal steps, marking the square
/// and its east and south neighbours, that ends early outside columns 3..76 / rows 3..45.
fn landmass_blob(page: &mut Page, rng: &mut MsRand, mut x: i32, mut y: i32) {
    let mut steps = rng.random(64) + 1;
    loop {
        page.set(x + 160, y, 15);
        page.set(x + 161, y, 15);
        page.set(x + 160, y + 1, 15);
        let d = (rng.random(4) * 2 + 1) as usize;
        x += DX[d];
        y += DY[d];
        steps -= 1;
        if steps == 0 || x <= 2 || x >= 77 || y <= 2 || y >= 46 {
            break;
        }
    }
}

/// 0x8e34: land squares that only touch diagonally (2x2 patterns 6 and 9) are joined by
/// making the other three squares of the 2x2 plain land; the scan then steps back one column
/// and row (CIV.EXE moves its loop counters) to look at the new land again.
fn join_diagonals(page: &mut Page) {
    let mut x = 1;
    while x < 79 {
        let mut y = 1;
        while y < 49 {
            let mut m = 0;
            if page.get(x, y) != 0 {
                m |= 1;
            }
            if page.get(x + 1, y) != 0 {
                m |= 2;
            }
            if page.get(x, y + 1) != 0 {
                m |= 4;
            }
            if page.get(x + 1, y + 1) != 0 {
                m |= 8;
            }
            if m == 6 || m == 9 {
                page.set(x + 1, y, 1);
                page.set(x, y + 1, 1);
                page.set(x + 1, y + 1, 1);
                if x != 0 {
                    x -= 1;
                }
                if y != 0 {
                    y -= 1;
                }
            }
            y += 1;
        }
        x += 1;
    }
}

/// 0x8f26: sea (0) becomes Ocean, land raised twice Hills, more often Mountains; land raised
/// once gets its climate band from the latitude: |29 - y - random(8)| + 1 - temperature, in
/// steps of 6 from the equator: Desert, Plains, Plains, Tundra, Tundra, Arctic.
fn climate_by_latitude(page: &mut Page, rng: &mut MsRand, temperature: i32) {
    for x in 0..80 {
        for y in 0..50 {
            let c = match page.get(x, y) {
                0 => code::OCEAN,
                1 => {
                    let lat = (rng.random(8) + y - 29).abs() + 1 - temperature;
                    // jump table at o 0x8fe7 for lat / 6 + 1 = 0..7 (above 7: unchanged)
                    match lat / 6 + 1 {
                        0 | 1 => code::DESERT,
                        2 | 3 => code::PLAINS,
                        4 | 5 => code::TUNDRA,
                        6 | 7 => code::ARCTIC,
                        _ => continue,
                    }
                }
                2 => code::HILLS,
                _ => code::MOUNTAINS,
            };
            page.set(x, y, c);
        }
    }
}

/// 0x9019: rain. Each row is swept west to east and then east to west; over Ocean the rain
/// builds up to a limit (|12 - latitude| + 4 * climate going east, latitude / 2 + climate going
/// west, latitude = |25 - y|), over land each square uses random(7 - 2 * climate) of it and
/// changes while some is left: going east Plains -> Grassland, Tundra -> Arctic, Hills ->
/// Forest, Desert -> Plains, Mountains take 3 more; going west Swamp, Hills and Mountains (3
/// more) -> Forest, Plains -> Grassland, Desert -> Plains, Grassland -> Jungle within 10 rows
/// of the equator row 25 else Swamp (and the rain drops to -2).
fn rainfall(page: &mut Page, rng: &mut MsRand, climate: i32) {
    for y in 0..50i32 {
        let lat = (25 - y).abs();
        let mut rain = 0;
        for x in 0..80 {
            let t = page.get(x, y);
            if t == code::OCEAN {
                if (12 - lat).abs() + 4 * climate > rain {
                    rain += 1;
                }
            } else if rain > 0 {
                rain -= rng.random(7 - 2 * climate);
                match t {
                    code::PLAINS => page.set(x, y, code::GRASSLAND),
                    code::TUNDRA => page.set(x, y, code::ARCTIC),
                    code::HILLS => page.set(x, y, code::FOREST),
                    code::MOUNTAINS => rain -= 3,
                    code::DESERT => page.set(x, y, code::PLAINS),
                    _ => {}
                }
            }
        }
        let mut rain = 0;
        for x in (0..80).rev() {
            let t = page.get(x, y);
            if t == code::OCEAN {
                if lat / 2 + climate > rain {
                    rain += 1;
                }
            } else if rain > 0 {
                rain -= rng.random(7 - 2 * climate);
                match t {
                    code::SWAMP | code::HILLS => page.set(x, y, code::FOREST),
                    code::PLAINS => page.set(x, y, code::GRASSLAND),
                    code::GRASSLAND => {
                        page.set(x, y, if lat < 10 { code::JUNGLE } else { code::SWAMP });
                        rain = -2;
                    }
                    code::MOUNTAINS => {
                        rain -= 3;
                        page.set(x, y, code::FOREST);
                    }
                    code::DESERT => page.set(x, y, code::PLAINS),
                    _ => {}
                }
            }
        }
    }
}

/// 0x91e2: 800 * (age + 1) changes, every other one at a random square and the next one
/// beside it (random(8) + 1 direction, which may step off the map): Forest -> Jungle, Swamp
/// -> Grassland, Plains and Tundra -> Hills, Grassland -> Forest, Jungle -> Swamp, Hills and
/// Arctic -> Mountains, Desert -> Plains, and Mountains with no Ocean on any diagonal ->
/// Ocean (`smooth_tile` o 0x09349).
fn erosion(page: &mut Page, rng: &mut MsRand, age: i32) {
    let (mut x, mut y) = (0, 0);
    for i in 0..800 * age + 800 {
        if i & 1 != 0 {
            let d = (rng.random(8) + 1) as usize;
            x += DX[d];
            y += DY[d];
        } else {
            x = rng.random(80);
            y = rng.random(50);
        }
        let c = match page.get(x, y) {
            code::FOREST => code::JUNGLE,
            code::SWAMP => code::GRASSLAND,
            code::PLAINS | code::TUNDRA => code::HILLS,
            code::GRASSLAND => code::FOREST,
            code::JUNGLE => code::SWAMP,
            code::HILLS | code::ARCTIC => code::MOUNTAINS,
            code::DESERT => code::PLAINS,
            code::MOUNTAINS => {
                if ![(-1, -1), (-1, 1), (1, -1), (1, 1)].iter().any(|(dx, dy)| page.get(x + dx, y + dy) == code::OCEAN) {
                    page.set(x, y, code::OCEAN);
                }
                continue;
            }
            _ => continue,
        };
        page.set(x, y, c);
    }
}

/// `make_rivers` (o 0x093d1): up to 256 tries at (land mass + climate) * 2 + 6 rivers. A try
/// backs up the left half of rows 0..99 (to x+160), starts on a random Hills square and walks
/// orthogonally (the direction turns by random(2) - (length & 1) quarter turns per step),
/// painting River, until the painted square has Ocean beside it or the walk reaches Mountains,
/// Ocean or River. A river of 5 or more squares that ends at the sea or at another river
/// stays, and Forest near its source becomes Jungle; otherwise the backup is restored. A step
/// whose new direction is below the reverse of the old one marks the square at (x, y+50)
/// (`code::RIVER_ALT`). Finally the markers move to the improvement band (rows 150..199).
fn rivers(page: &mut Page, rng: &mut MsRand, land_mass: i32, climate: i32) {
    let mut made = 0;
    let mut tries = 0;
    loop {
        page.copy(0, 0, 160, 100, 160, 0);
        let mut len = 0;
        let (mut x, mut y) = loop {
            let x = rng.random(80);
            let y = rng.random(50);
            if page.get(x, y) == code::HILLS {
                break (x, y);
            }
        };
        let (x0, y0) = (x, y);
        let mut dir = rng.random(4) * 2;
        let (coast, t) = loop {
            page.set(x, y, code::RIVER);
            let coast = [1, 3, 5, 7].iter().any(|&k| page.get(x + DX[k], y + DY[k]) == code::OCEAN);
            let prev = dir;
            dir = (dir + (rng.random(2) - (len & 1)) * 2) & 7;
            if prev ^ 4 > dir {
                page.set(x, y + 50, code::RIVER_ALT);
            }
            x += DX[dir as usize + 1];
            y += DY[dir as usize + 1];
            let t = page.get(x, y);
            len += 1;
            if coast || t == code::MOUNTAINS || t == code::OCEAN || t == code::RIVER {
                break (coast, t);
            }
        };
        if (coast || t == code::RIVER) && len >= 5 {
            made += 1;
            for k in 1..=21 {
                if page.get(x0 + DX[k], y0 + DY[k]) == code::FOREST {
                    page.set(x0 + DX[k], y0 + DY[k], code::JUNGLE);
                }
            }
        } else {
            page.copy(160, 0, 160, 100, 0, 0);
        }
        tries += 1;
        if tries >= 256 || (land_mass + climate) * 2 + 6 <= made {
            break;
        }
    }
    page.copy(0, 50, 80, 50, 0, 150);
    page.fill(0, 50, 80, 50, 0);
}

/// The Earth map (main menu "EARTH", ds:0xd76a): `generate_world` loads MAP.PIC into the
/// generator page instead of growing land and then finishes it like a random world (o
/// 0x092ed), so the Earth also gets its continents numbered, site values and the random
/// Tundra dots on the polar rows. MAP.PIC only holds the 80x50 terrain (the rest of the page
/// is blank).
pub fn earth(em: &EarthMap, seed: u32) -> World {
    let mut page = Page::new();
    for y in 0..H {
        for x in 0..W {
            page.set(x, y, em.code(x as usize, y as usize));
        }
    }
    page.fill(160, 0, 160, 200, 0);
    let mut map = Map::empty(WorldParams::normal(), seed & 0x7fff);
    map.earth = true;
    finish(page, MsRand::new(seed), map)
}

/// `place_specials_and_huts` (o 0x097a8; the catalogue name is a guess: it numbers the
/// continents and oceans, runs the sea grid and the site values, draws the polar rows) and the
/// conversion of the finished page into `map`.
fn finish(mut page: Page, mut rng: MsRand, mut map: Map) -> World {
    let (land_sizes, ocean_sizes) = number_bodies(&mut page);
    // 0x9b70: the land grid (o 0x09edd) reads the map page (page 2) through the game's
    // accessors 0x1a68:0x1350 / 0x1a68:0x1948 while the new world is still on page 1, and
    // costs its links with the GoTo search; not ported (see `sea_grid`)
    let sea_grid = sea_grid(&page);
    let sites = site_values(&mut page, map.seed);
    // 0x9b8b: the polar rows become Arctic, then 20 times a Tundra square on each of rows 0,
    // 1, 48 and 49
    for x in 0..80 {
        page.set(x, 0, code::ARCTIC);
        page.set(x, 49, code::ARCTIC);
    }
    for _ in 0..20 {
        for y in [0, 1, 48, 49] {
            let x = rng.random(80);
            page.set(x, y, code::TUNDRA);
        }
    }
    for y in 0..H {
        for x in 0..W {
            let terrain = page.terrain(x, y).unwrap_or(Terrain::Ocean);
            let mut t = Tile { terrain, ..Default::default() };
            // the plane keeps the number of what the square was before the polar rows were
            // drawn: an Ocean number on the Arctic and Tundra squares made from sea there
            if t.is_land() {
                t.continent = page.plane(x, y);
            }
            t.special = Map::special_at(map.seed, x, y) && terrain.info().special.is_some();
            t.shield = Map::shield_at(x, y);
            t.set(tile_flags::HUT, terrain != Terrain::Ocean && Map::hut_at(map.seed, x, y));
            t.set(tile_flags::RIVER_ALT, page.get(x, y + 150) & code::RIVER_ALT != 0);
            map.tiles[Map::idx(x, y)] = t;
        }
    }
    map.site = (0..H).flat_map(|y| (0..W).map(move |x| (x, y))).map(|(x, y)| page.get(x + 80, y)).collect();
    World { map, rng, land_sizes, ocean_sizes, sites, sea_grid }
}

/// 0x097b1..0x09b65: number the land (first pass) and the water (second pass). A scan in rows
/// labels each run of squares of the kind, taking over the label of any of the three squares
/// above and merging two labels that meet (rows 0..y are relabelled, the sizes added); a new
/// run takes the lowest free label. The labels are then ranked by size (larger first, ties by
/// label): ranks 1..14, all others 15; the rank is written at (x, y+50). Returns the size
/// tables ds:0xdcfe (land) and ds:0xdd7e (water): the sizes of ranks 1..14, 1 for rank 15.
/// The scan does not wrap around the east-west edge. The size tables start empty (they are
/// BSS; a second game in one CIV.EXE session would start from the last game's tables).
pub fn number_bodies(page: &mut Page) -> ([u16; 16], [u16; 16]) {
    let mut tables = [[0u16; 16]; 2];
    for pass in 0..2 {
        // es=ds:0x6414:0x740 + 50x + y
        let mut label = [0u8; 80 * 50];
        let mut size = [0i32; 256];
        let mut free = 0usize;
        for y in 0..50i32 {
            let mut cur: i32 = -1;
            let mut run = false;
            for x in 0..80i32 {
                let ocean = page.is_ocean(x, y);
                if ocean != (pass == 1) {
                    run = false;
                    cur = -1;
                    continue;
                }
                if y > 0 {
                    let from = if x != 0 { -1 } else { 0 };
                    let to = if x < 79 { 1 } else { 0 };
                    for dx in from..=to {
                        let n = label[(50 * (x + dx) + y - 1) as usize] as i32;
                        if n == 0 {
                            continue;
                        }
                        if cur != -1 && cur != n {
                            for yy in 0..=y {
                                for xx in 0..80 {
                                    let l = &mut label[(50 * xx + yy) as usize];
                                    if *l as i32 == cur {
                                        *l = n as u8;
                                    }
                                }
                            }
                            size[n as usize] += size[cur as usize];
                            size[cur as usize] = 0;
                        }
                        cur = n;
                    }
                }
                if cur == -1 {
                    if !run {
                        free = 0;
                        loop {
                            free += 1;
                            if size[free] == 0 {
                                break;
                            }
                        }
                    }
                    cur = free as i32;
                }
                label[(50 * x + y) as usize] = cur as u8;
                size[cur as usize] += 1;
                run = true;
            }
        }
        // 0x99d3: insertion into a 15-entry list ordered by size
        let mut ord = [0usize; 16];
        let mut rank = [0u8; 256];
        for id in 1..64 {
            rank[id] = 15;
            for j in 1..15 {
                if size[ord[j]] < size[id] {
                    let mut k = 15;
                    while k > j {
                        rank[ord[k - 1]] = k as u8;
                        ord[k] = ord[k - 1];
                        k -= 1;
                    }
                    rank[id] = j as u8;
                    ord[j] = id;
                    break;
                }
            }
        }
        // 0x9a8c: the shifts above also ranked the empty entries (label 0)
        rank[0] = 0;
        for x in 0..80 {
            for y in 0..50 {
                let r = rank[label[(50 * x + y) as usize] as usize];
                if r != 0 {
                    page.set(x, y + 50, r);
                }
            }
        }
        let t = &mut tables[pass];
        for i in 1..15 {
            t[i] = size[ord[i]] as u16;
        }
        t[0] = 0;
        t[15] = 1;
    }
    (tables[0], tables[1])
}

/// `number_oceans` (o 0x0a17a; it builds the sea grid, the catalogue name is a guess): the
/// sea connectivity of the 20x13 blocks of 4x4 squares, ds:0x7f38 (byte 13 * bx + by, bit d-1
/// = sea route towards direction d = 1..8). Rows 0..11 are scanned: a block's sea is the
/// ocean number of the first Ocean square of its 2x2 at (4bx+1, 4by+1); a neighbour block
/// with the same ocean number is linked when a walk of 5 squares from the block's corner
/// towards it keeps at least two Ocean squares (on the map) in each 2x2 (in the bottom row the
/// eastward link is set even when the walk fails). The link is set both ways. A neighbour
/// without Ocean in its 2x2 keeps the previous neighbour's number (CIV.EXE does not reset it;
/// its very first value is whatever the land grid left on the stack, taken as none here).
/// Afterwards the west edge of column 0 and the east edge of column 19 are open (wrap), except
/// the outer corners.
pub fn sea_grid(page: &Page) -> [u8; 260] {
    let mut grid = [0u8; 260];
    let ocean_at = |x: i32, y: i32| -> Option<u8> {
        [(0, 0), (1, 0), (0, 1), (1, 1)].iter().find(|(dx, dy)| page.is_ocean(x + dx, y + dy)).map(|(dx, dy)| page.plane(x + dx, y + dy))
    };
    let mut other: Option<u8> = None;
    for bx in 0..20 {
        for by in 0..12 {
            let (x, y) = (4 * bx + 1, 4 * by + 1);
            let Some(id) = ocean_at(x, y) else { continue };
            for d in 1..=8usize {
                let (nx, ny) = (4 * DX[d] + x, 4 * DY[d] + y);
                if let Some(o) = ocean_at(nx, ny) {
                    other = Some(o);
                }
                if other != Some(id) {
                    continue;
                }
                let (mut px, mut py) = (x, y);
                let mut ok = true;
                for _ in 0..5 {
                    px += DX[d];
                    py += DY[d];
                    let wet = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().filter(|(dx, dy)| page.is_ocean(px + dx, py + dy) && Map::in_bounds_strict(px + dx, py + dy)).count();
                    if wet < 2 {
                        ok = false;
                        break;
                    }
                }
                if !ok && !(by == 11 && d == 3) {
                    continue;
                }
                let nby = DY[d] + by;
                if nby >= 12 {
                    continue;
                }
                grid[(13 * bx + by) as usize] |= 1 << (d - 1);
                let nbx = DX[d] + bx;
                if !(0..20).contains(&nbx) || !(0..=12).contains(&nby) {
                    continue;
                }
                grid[(13 * nbx + nby) as usize] |= 1 << ((d + 3) & 7);
            }
        }
    }
    for by in 0..12 {
        grid[by] |= 0xe0;
        grid[13 * 19 + by] |= 0x0e;
    }
    grid[0] &= 0x7f;
    grid[11] &= 0xdf;
    grid[13 * 19] &= 0xfd;
    grid[13 * 19 + 11] &= 0xf7;
    grid
}

/// `compute_site_values` (o 0x09c5f): the site-value layer (x+80, y) (also written at
/// (x+80, y+50)). Columns 2..77, rows 2..47: 0 unless the square is Plains, Grassland or River;
/// else S summed over the 21 squares of a city radius in CIV.EXE's order (ds:0x2498 /
/// ds:0x24c2, the centre last): `SITE_SCORE` of the terrain (its special variant where
/// `tile_has_special`), +2 for a Grassland/River shield, doubled for indices 0..8 and doubled
/// again for index 0 (the square north of the centre); minus 16 when a Grassland/River centre
/// has no shield; value 8 + clamp((S - 120) / 8, 1, 15) / 2. Counts the squares with a value
/// per continent number (ds:0xb1ee, returned).
fn site_values(page: &mut Page, seed: u32) -> [u16; 16] {
    let mut sites = [0u16; 16];
    for x in 2..78 {
        for y in 2..48 {
            let v = site_value_with(|x, y| page.terrain(x, y), seed, x, y);
            page.set(x + 80, y, v);
            page.set(x + 80, y + 50, v);
            if v != 0 {
                sites[page.plane(x, y) as usize & 15] += 1;
            }
        }
    }
    sites
}

/// The site value of (x,y) as `compute_site_values` (o 0x09c5f) computes it from the terrain
/// `at` gives (0 outside columns 2..77 / rows 2..47 and off Plains, Grassland and River).
pub fn site_value_with(at: impl Fn(i32, i32) -> Option<Terrain>, seed: u32, x: i32, y: i32) -> u8 {
    let centre = at(x, y);
    if !(2..78).contains(&x) || !(2..48).contains(&y) || !matches!(centre, Some(Terrain::Plains | Terrain::Grassland | Terrain::River)) {
        return 0;
    }
    let shield = |t: Option<Terrain>, x: i32, y: i32| matches!(t, Some(Terrain::Grassland | Terrain::River)) && Map::shield_at(x, y);
    let mut total = 0;
    for (i, (dx, dy)) in super::economy::EXE_RADIUS.iter().enumerate() {
        let (nx, ny) = (x + *dx as i32, y + *dy as i32);
        let t = at(nx, ny);
        let mut v = if shield(t, nx, ny) { 2 } else { 0 };
        let k = t.map(|t| t as usize).unwrap_or(0) + if Map::special_at(seed, nx, ny) { 12 } else { 0 };
        v += SITE_SCORE[k];
        if i < 9 {
            v *= 2;
        }
        if i == 0 {
            v *= 2;
        }
        total += v;
    }
    if centre != Some(Terrain::Plains) && !shield(centre, x, y) {
        total -= 16;
    }
    (((total - 120) / 8).clamp(1, 15) / 2 + 8) as u8
}

/// The site value of (x,y) on the map as it is now (for saves without the stored layer).
pub fn site_value(map: &Map, x: i32, y: i32) -> u8 {
    site_value_with(|x, y| Map::in_bounds_strict(x, y).then(|| map.terrain(x, y)), map.seed, x, y)
}

/// The start squares of CIV.EXE's Earth map by tribe (ds:0x35da, x/y bytes; 0 and 8 are the
/// barbarians).
pub const EARTH_STARTS: [(i32, i32); 16] = [
    (0, 0), (36, 19), (45, 22), (38, 15), (41, 24), (12, 18), (39, 18), (57, 24),
    (0, 0), (44, 12), (42, 42), (33, 16), (5, 23), (66, 19), (31, 14), (49, 19),
];

/// The start square search of `civ_init` (o 0x0493c) at turn 0: random squares
/// (random(64) + 8, random(34) + 8) until the k-th one is not Ocean, has a site value of at
/// least 12 - k/32, is at least 10 - k/64 (0x1d43:0x27f distance) from every square of
/// `others` (unit 0 of each civ with a lower number; (0,0) for the absent ones, whose records
/// are empty), lies on a continent with at least 32 - k/64 site squares (`World::sites`) and
/// has no hut. A computer civilization takes the 2001st square whatever it is, and CIV.EXE then
/// drops it (as after a 2000th try); `None` then. On the Earth map the search still runs (for
/// its random numbers) and the tribe's square of `EARTH_STARTS` is used.
pub fn start_square(map: &Map, sites: &[u16; 16], rng: &mut MsRand, others: &[(i32, i32)], human: bool, tribe: u8) -> Option<(i32, i32)> {
    let mut tries = 0;
    let found = loop {
        let x = rng.random(64) + 8;
        let y = rng.random(34) + 8;
        // no cities exist yet: 0x1d43:0xf8 finds none and the distance is 30
        let near = others.iter().map(|o| Map::civ_distance((x, y), *o)).fold(30, i32::min);
        let site = map.site_at(x, y) as i32;
        tries += 1;
        if tries > 2000 && !human {
            break (x, y);
        }
        if map.is_ocean(x, y) || 12 - tries / 32 > site || near < 10 - tries / 64 {
            continue;
        }
        if (sites[map.tile(x, y).continent as usize & 15] as i32) < 32 - tries / 64 {
            continue;
        }
        if map.tile(x, y).has(tile_flags::HUT) {
            continue;
        }
        break (x, y);
    };
    if map.earth {
        return Some(EARTH_STARTS[tribe as usize & 15]);
    }
    (human || tries < 2000).then_some(found)
}

/// What `start_report` (o 0x0515b) gives a civilization to even out the starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartBonus {
    /// The start's score: nearest other civ on the same continent at distance < 10: 0,
    /// < 20: 1, farther: 2, none: 4; +2 with more than one River among the 9 squares around
    /// the start (the square included), else +1 with at least 3 Grassland; continent size / 50
    /// of 2..3: +1, 4 or more: +2.
    pub score: i32,
    /// An extra Settlers when the score is more than 3 below the best (which then counts 3).
    pub settlers: bool,
    /// Free advances: the best score less this civ's (after the Settlers).
    pub advances: i32,
}

/// `start_report` (o 0x0515b) for the starts `starts` (slot, square): the score of each and
/// the Settlers and advances that make up for a worse one. `land_sizes` is ds:0xdcfe.
pub fn start_bonuses(map: &Map, land_sizes: &[u16; 16], starts: &[(Slot, (i32, i32))]) -> Vec<StartBonus> {
    let score = |i: usize| {
        let (_, p) = starts[i];
        let cont = map.tile(p.0, p.1).continent;
        let near = starts
            .iter()
            .enumerate()
            .filter(|(j, (_, q))| *j != i && map.tile(q.0, q.1).continent == cont)
            .map(|(_, (_, q))| Map::civ_distance(p, *q))
            .fold(999, i32::min);
        let mut s = match near {
            0..=9 => 0,
            10..=19 => 1,
            999 => 4,
            _ => 2,
        };
        let around = |t: Terrain| (0..9).filter(|&k| map.terrain(p.0 + DX[k], p.1 + DY[k]) == t).count();
        if around(Terrain::River) > 1 {
            s += 2;
        } else if around(Terrain::Grassland) >= 3 {
            s += 1;
        }
        s += match land_sizes[cont as usize & 15] / 50 {
            0 | 1 => 0,
            2 | 3 => 1,
            _ => 2,
        };
        s
    };
    let scores: Vec<i32> = (0..starts.len()).map(score).collect();
    let best = scores.iter().copied().max().unwrap_or(0);
    scores
        .iter()
        .map(|&s| {
            let settlers = best - s > 3;
            let s2 = if settlers { s + 3 } else { s };
            StartBonus { score: s, settlers, advances: best - s2 }
        })
        .collect()
}

/// Score a tile as a city site centre (sum of yields in the radius, with bonuses). civrs's own
/// measure (not CIV.EXE's), used by the computer players and the hints.
pub fn site_score(map: &Map, x: i32, y: i32) -> i32 {
    let t = map.tile(x, y);
    if !t.is_land() || t.terrain == Terrain::Arctic || t.terrain == Terrain::Mountains {
        return -1;
    }
    let mut score = 0;
    for (i, (dx, dy)) in crate::rules::CITY_RADIUS.iter().enumerate() {
        let (nx, ny) = (x + *dx as i32, y + *dy as i32);
        if !Map::in_bounds(nx, ny) {
            continue;
        }
        let nt = map.tile(nx, ny);
        let info = nt.terrain.info();
        let (f, s, tr) = match (&info.special, nt.special) {
            (Some(sp), true) => (sp.food as i32, sp.shields as i32, sp.trade as i32),
            _ => (info.food as i32, info.shields as i32, info.trade as i32),
        };
        let s = if matches!(nt.terrain, Terrain::Grassland | Terrain::River) { nt.shield as i32 } else { s };
        let w = if i == 0 { 3 } else if i < 9 { 2 } else { 1 };
        score += w * (f * 3 + s * 2 + tr);
        if nt.terrain == Terrain::River {
            score += 2;
        }
    }
    if map.is_coastal(x, y) {
        score += 6;
    }
    if t.terrain == Terrain::River {
        score += 4;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ms_rand_is_microsoft_c() {
        // srand(1): the first rand() values of the Microsoft C library
        let mut r = MsRand::new(1);
        let v: Vec<i32> = (0..5).map(|_| r.rand()).collect();
        assert_eq!(v, vec![41, 18467, 6334, 26500, 19169]);
        let mut r = MsRand::new(1);
        assert_eq!(r.random(100), 41 * 100 >> 15);
    }

    #[test]
    fn same_seed_same_world() {
        let a = generate(WorldParams::normal(), 1234);
        let b = generate(WorldParams::normal(), 1234);
        assert_eq!(a.map, b.map);
        assert_eq!(a.rng, b.rng);
        let c = generate(WorldParams::normal(), 1235);
        assert_ne!(a.map.tiles, c.map.tiles);
    }

    #[test]
    fn land_mass_option_grows_the_land() {
        let land = |lm: u8| -> usize {
            (0..8u32).map(|s| generate(WorldParams { land_mass: lm, ..WorldParams::normal() }, 77 + s).map.count_land()).sum::<usize>() / 8
        };
        let (small, normal, large) = (land(0), land(1), land(2));
        assert!(small < normal && normal < large, "{small} {normal} {large}");
    }

    #[test]
    fn edges_and_poles() {
        let w = generate(WorldParams::normal(), 99);
        for x in 0..W {
            // rows 0 and 49 are Arctic but for the Tundra dots
            for y in [0, 49] {
                assert!(matches!(w.map.terrain(x, y), Terrain::Arctic | Terrain::Tundra));
            }
            // land never reaches rows 1 and 48 except for the Tundra dots
            for y in [1, 48] {
                assert!(matches!(w.map.terrain(x, y), Terrain::Ocean | Terrain::Tundra));
            }
        }
        for y in 2..H - 2 {
            assert!(w.map.is_ocean(0, y) && w.map.is_ocean(79, y));
        }
    }

    #[test]
    fn numbering_ranks_by_size() {
        let w = generate(WorldParams::normal(), 5);
        let m = &w.map;
        // continent 1 is the largest landmass; ranks 1..14 have the sizes of the table
        let count = |c: u8| (2..H - 2).flat_map(|y| (0..W).map(move |x| (x, y))).filter(|&(x, y)| m.is_land(x, y) && m.tile(x, y).continent == c).count();
        for c in 1..15u8 {
            if w.land_sizes[c as usize] > 0 {
                assert_eq!(count(c), w.land_sizes[c as usize] as usize, "continent {c}");
            }
        }
        assert!(w.land_sizes[1] >= w.land_sizes[2]);
        assert_eq!(w.land_sizes[15], 1);
        // `Map::compute_continents` gives the same numbers once the polar rows are sea again
        let mut m2 = m.clone();
        for x in 0..W {
            for y in [0, 1, 48, 49] {
                m2.get_mut(x, y).unwrap().terrain = Terrain::Ocean;
            }
        }
        m2.compute_continents();
        for y in 2..H - 2 {
            for x in 0..W {
                assert_eq!(m2.tile(x, y).continent, m.tile(x, y).continent);
            }
        }
    }

    #[test]
    fn site_layer_matches_the_formula() {
        let w = generate(WorldParams::normal(), 31);
        let m = &w.map;
        for y in 4..H - 4 {
            for x in 0..W {
                assert_eq!(m.site_at(x, y), site_value(m, x, y), "({x},{y})");
            }
        }
        let total: u16 = w.sites.iter().sum();
        assert_eq!(total as usize, (0..W).flat_map(|x| (0..H).map(move |y| (x, y))).filter(|&(x, y)| m.site_at(x, y) != 0).count());
    }

    #[test]
    fn rivers_flow_to_the_sea_or_a_river() {
        let w = generate(WorldParams { climate: 2, ..WorldParams::normal() }, 8);
        let rivers = w.map.tiles.iter().filter(|t| t.terrain == Terrain::River).count();
        assert!(rivers >= 5 * 8, "{rivers} river squares");
        assert!(w.map.tiles.iter().all(|t| !t.has(tile_flags::RIVER_ALT) || t.terrain == Terrain::River));
    }

    #[test]
    fn huts_and_specials_follow_the_patterns() {
        let w = generate(WorldParams::normal(), 4242);
        let m = &w.map;
        for y in 0..H {
            for x in 0..W {
                let t = m.tile(x, y);
                assert_eq!(t.has(tile_flags::HUT), t.is_land() && Map::hut_at(m.seed, x, y));
                if t.special {
                    assert!((2..48).contains(&y));
                }
            }
        }
    }
    /// A world made by CIV.EXE in DOSBox (clock seed 0x5fa8, the normal world, 7 civilizations
    /// at Chieftain, Romans; saved after one turn as CIVIL0.MAP/.SVE): the terrain codes of the
    /// map page (rows of 80 hex digits), the continent sizes ds:0xdcfe and site squares ds:0xb1ee
    /// of the .SVE and the start of every civilization (the computer ones as their cities, slot
    /// 6's Settlers had moved one square west).
    const DOSBOX_5FA8: [&str; 50] = [
        "f7f7f7f7ff77ffffffff7fffff7ffffffff7ff7ff7ffffffffffffffffff7f7f7f7ff77ffffff7ff",
        "11177171771717111171111111717111111111111717111117111171717111111111111111111171",
        "11111111111111111111111111111111111111111111111111111111111111111111111111111111",
        "11111111111111111111111111111111111111111111111111111111111111111111111111111111",
        "11111111111111111111111111111111111111111111111111111111111111111fdd111111111111",
        "1111111111111111111111111111111111111111111111111111df11111111111fd1111111111111",
        "11111111111111111111111111111111111111111111111111fa3dff111111111f2d111111111111",
        "11111111d2a11111111111111111111111111111111111111fa2dfc2111111111fda111111111111",
        "1111111123f1111111111111111111111111111111111111da2fa321111111111da1111111111111",
        "11111111af1111111111111111111111111111111111111122266ab1111111111311111111111111",
        "111111111111111111111111111111111111111111111111fb311211111111111111111111111111",
        "1111111111111111111111111111111111111111111111112a1123f1111111111111111111111111",
        "1111111111111111111111111111111111111111111111111111aa22a1113aaa1111111111111111",
        "11111111111111111111111111111111111112aa221111111111a2aaa111baaa1111111111111111",
        "111111111111111111111111111111111111122a311111111111aa6a11111b311111111111111111",
        "1111111111111111111111111111111111aaaaa2b11111111111a331111ab9bb2111111111111111",
        "111ac66bba11111111111111111111111aa22aaaa111111111112aa1111ab99b1111111111111111",
        "111a2cccb111aa2111111111111111111a6a26ab1111111111111b111116bb9a1111111111111111",
        "11166b111111ac311116911ab111111a2aba111111111111111111111111999c1111111111111111",
        "111cb1111111addc1bd6966ea1111112aac1111111111111111111111111a2261111111111111111",
        "11111111111c9d1c1ddc96a61111111b2c11111111111111111111111111bbb31111111111111111",
        "11111111111c9bdcdcc996a1111111c6dc1111111111111111111111111126263111111111111111",
        "1111111111999bccccc9ec6111111cce61111111111111111111111111111ca21111111111111111",
        "11111111116662c6cdc9e6c11111126aa11116611111111111111111111111111111111111111111",
        "111111111111c2119999b6c11111112a111116a111111111111111111111111111116662a1111111",
        "11111111111111116622ca111111111111111aa1111111111111111111111111111c666c6ca16c11",
        "11111111111111111111111111111111111116666a1111111111111111111111111c1c6cc666a611",
        "11111111111111111111111111111111116c22d22a1111111111111111111111111cc2bb999ee611",
        "11111111111111111111111111111111dd6a2cddd61111111111111111116b11111ac2bbc6996611",
        "1111111111111111111111111111111ccabcddddea1111111111111111116dcccb12d2db1169c111",
        "1111111111111111111111111111111aa2d1dd111a11126a1111111111116bd99962eccdd1111111",
        "111111111111111abab111111111111aa2ddddddd6a11aaa111111111111c66bd99916dcc1111111",
        "111111111111111a6b3111111111111a22ddddddde2a16ec6db11111111111abcaab111ac9bb1111",
        "111111111111111ca6b111111111111111222dddcca11c6c62a111111111111222ac61ecd9cab111",
        "111111111111111a33a111111111111111222bd9c9111b2b1aa11111111111122c1cb16e99661111",
        "111111111111a2aba2b11111111111aaaaaabbd9996666c62ba111111111111a2c6cdc699aaaa111",
        "111111111111aa6caab1111111111b999c6cc1dccc666a22221111111111111aab6dc6996aaa1111",
        "111111111111112aa2a1111111111b9a996c6ddc66a116a22211111111111111111baaaaa2211111",
        "11111111111aaacc62a111111111119ab9dc6a2a211111aa22a1111111111111111aaa6abb111111",
        "11111111111aaaa11a1111111111111999baaa11111111222aa11111111111111111a31111111111",
        "1111111111aaf2a1111112311111111bb223a111111111a22a211111111111111111311111111111",
        "1111111111aaa17111113a2111111111d1111111111111ba3bc11111111111111111111111111111",
        "1111111111fab3c11111f27111111111111111111111113d1f111111111111111111111111111111",
        "1111111111affa111111fa111111111111111111111111a111111111111111111111111111111111",
        "1111111111faaf111111111111111111111111111111111111111111111111111111111111111111",
        "1111111111aaa1111111111111111111111111111111111111111111111111111111111111111111",
        "1111111111df11111111111111111111111111111111111111111111111111111111111111111111",
        "11111111111111111111111111111111111111111111111111111111111111111111111111111111",
        "11711717171111111171111117111111177711711111111111717711717171111711111171111717",
        "fffffff777fffffff7ffff77ffffffffff7fffffff77ffffff7fffff77ff7f7ffff7fffff7ff77ff",
    ];

    #[test]
    fn matches_a_world_made_by_civ_exe() {
        let w = generate(WorldParams::normal(), 0x5fa8);
        for (y, row) in DOSBOX_5FA8.iter().enumerate() {
            let ours: String = (0..W).map(|x| format!("{:x}", w.map.terrain(x, y as i32).info().earth_code)).collect();
            assert_eq!(&ours, row, "row {y}");
        }
        assert_eq!(w.land_sizes, [0, 221, 168, 86, 75, 59, 57, 46, 18, 14, 10, 8, 0, 0, 0, 1]);
        assert_eq!(w.sites, [0, 98, 85, 41, 43, 22, 30, 22, 7, 2, 2, 2, 0, 0, 0, 0]);
        let mut r = w.rng;
        r.random(50);
        let mut unit0 = [(0, 0); 8];
        let mut starts = Vec::new();
        for s in 1..8u8 {
            let tribe = if s == 1 {
                1
            } else {
                loop {
                    let t = r.random(16) as u8;
                    if t & 7 == s {
                        break t;
                    }
                }
            };
            assert_eq!(tribe, s, "slot {s} plays its first tribe");
            let p = start_square(&w.map, &w.sites, &mut r, &unit0[1..s as usize], s == 1, tribe).unwrap();
            unit0[s as usize] = p;
            starts.push((s, p));
        }
        let squares: Vec<(i32, i32)> = starts.iter().map(|(_, p)| *p).collect();
        assert_eq!(squares, vec![(70, 34), (10, 23), (41, 35), (32, 37), (15, 32), (50, 39), (34, 27)]);
        // the .SVE's known advances: one each but none for slot 2, two for 6, three for 7
        let bonus = start_bonuses(&w.map, &w.land_sizes, &starts);
        assert_eq!(bonus.iter().map(|b| b.advances).collect::<Vec<_>>(), vec![1, 0, 1, 1, 1, 2, 3]);
        assert!(bonus.iter().all(|b| !b.settlers));
    }
}
