//! The world map: 80x50 tiles, wrapping east-west.

use crate::rules::{Terrain, MAP_H, MAP_W};
use serde::{Deserialize, Serialize};

pub type CityId = u16;
pub type UnitId = u16;
pub type Slot = u8;

/// Tile improvement / state flags.
pub mod tile_flags {
    pub const ROAD: u16 = 1 << 0;
    pub const RAILROAD: u16 = 1 << 1;
    pub const IRRIGATION: u16 = 1 << 2;
    pub const MINE: u16 = 1 << 3;
    pub const FORTRESS: u16 = 1 << 4;
    pub const POLLUTION: u16 = 1 << 5;
    pub const HUT: u16 = 1 << 6;
    /// A River square drawn with the second set of river sprites (bit 0x80 of the improvement
    /// nibbles, set by the world generator's river walk, o 0x09497; read by 0x1aedd).
    pub const RIVER_ALT: u16 = 1 << 7;
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tile {
    pub terrain: Terrain,
    /// Special resource present (Oasis, Horses, Fish, ...).
    pub special: bool,
    /// Grassland with the extra shield.
    pub shield: bool,
    pub flags: u16,
    /// One bit per player slot: tile has been explored (terrain known).
    pub explored: u8,
    /// Landmass id (0 = ocean); numbered by size, 1 = largest.
    pub continent: u8,
    pub city: Option<CityId>,
    /// City currently working this tile.
    pub worked_by: Option<CityId>,
}

impl Default for Tile {
    fn default() -> Self {
        Tile { terrain: Terrain::Ocean, special: false, shield: false, flags: 0, explored: 0, continent: 0, city: None, worked_by: None }
    }
}

impl Tile {
    pub fn has(&self, f: u16) -> bool {
        self.flags & f != 0
    }
    pub fn set(&mut self, f: u16, on: bool) {
        if on {
            self.flags |= f;
        } else {
            self.flags &= !f;
        }
    }
    pub fn is_ocean(&self) -> bool {
        self.terrain == Terrain::Ocean
    }
    pub fn is_land(&self) -> bool {
        self.terrain != Terrain::Ocean
    }
    pub fn explored_by(&self, slot: Slot) -> bool {
        self.explored & (1 << slot) != 0
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorldParams {
    /// 0 small, 1 normal, 2 large
    pub land_mass: u8,
    /// 0 cool, 1 temperate, 2 warm
    pub temperature: u8,
    /// 0 arid, 1 normal, 2 wet
    pub climate: u8,
    /// 0 3 billion, 1 4 billion, 2 5 billion years
    pub age: u8,
}

impl WorldParams {
    pub fn normal() -> Self {
        WorldParams { land_mass: 1, temperature: 1, climate: 1, age: 1 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Map {
    pub tiles: Vec<Tile>,
    pub params: WorldParams,
    pub earth: bool,
    pub seed: u32,
    /// The site-value layer of the map page (x+80, y), 80x50 row-major, as the world generator
    /// left it (`mapgen::site_value_with`, o 0x09c5f); not part of the bincode body (the save
    /// appends it), empty for maps made otherwise.
    #[serde(skip)]
    pub site: Vec<u8>,
}

/// 8 compass directions in the order used for keypad moves and sprite sheets.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Dir8 {
    N = 0,
    NE = 1,
    E = 2,
    SE = 3,
    S = 4,
    SW = 5,
    W = 6,
    NW = 7,
}

impl Dir8 {
    pub const ALL: [Dir8; 8] = [Dir8::N, Dir8::NE, Dir8::E, Dir8::SE, Dir8::S, Dir8::SW, Dir8::W, Dir8::NW];

    pub fn delta(self) -> (i32, i32) {
        match self {
            Dir8::N => (0, -1),
            Dir8::NE => (1, -1),
            Dir8::E => (1, 0),
            Dir8::SE => (1, 1),
            Dir8::S => (0, 1),
            Dir8::SW => (-1, 1),
            Dir8::W => (-1, 0),
            Dir8::NW => (-1, -1),
        }
    }

    /// Keypad digit (1..9 except 5) to direction.
    pub fn from_keypad(n: u8) -> Option<Dir8> {
        match n {
            8 => Some(Dir8::N),
            9 => Some(Dir8::NE),
            6 => Some(Dir8::E),
            3 => Some(Dir8::SE),
            2 => Some(Dir8::S),
            1 => Some(Dir8::SW),
            4 => Some(Dir8::W),
            7 => Some(Dir8::NW),
            _ => None,
        }
    }

    pub fn from_delta(dx: i32, dy: i32) -> Option<Dir8> {
        // a step across the wrapping east-west edge
        let dx = Map::delta_x(0, dx);
        Dir8::ALL.iter().copied().find(|d| d.delta() == (dx.signum(), dy.signum()) && dx.abs() <= 1 && dy.abs() <= 1 && (dx, dy) != (0, 0))
    }

    pub fn opposite(self) -> Dir8 {
        Dir8::ALL[(self as usize + 4) % 8]
    }

    pub fn is_diagonal(self) -> bool {
        matches!(self, Dir8::NE | Dir8::SE | Dir8::SW | Dir8::NW)
    }
}

impl Map {
    pub fn empty(params: WorldParams, seed: u32) -> Map {
        Map { tiles: vec![Tile::default(); MAP_W * MAP_H], params, earth: false, seed, site: Vec::new() }
    }

    /// 0 <= x < 80 and 0 <= y < 50 (`in_map` 0x1a68:0x132c), for the passes of the world
    /// generator, which do not wrap.
    #[inline]
    pub fn in_bounds_strict(x: i32, y: i32) -> bool {
        (0..MAP_W as i32).contains(&x) && (0..MAP_H as i32).contains(&y)
    }

    #[inline]
    /// The map wraps around east-west (CIV.EXE 0x1ec91): every x is on the map, only y can
    /// fall off it.
    pub fn in_bounds(_x: i32, y: i32) -> bool {
        y >= 0 && (y as usize) < MAP_H
    }

    /// The column `x` stands for on the wrapping map (0..MAP_W).
    #[inline]
    pub fn wrap_x(x: i32) -> i32 {
        x.rem_euclid(MAP_W as i32)
    }

    /// Shortest signed horizontal step from column `a` to column `b`, around either way.
    #[inline]
    pub fn delta_x(a: i32, b: i32) -> i32 {
        let w = MAP_W as i32;
        let d = (b - a).rem_euclid(w);
        if d > w / 2 {
            d - w
        } else {
            d
        }
    }

    #[inline]
    pub fn idx(x: i32, y: i32) -> usize {
        y as usize * MAP_W + Map::wrap_x(x) as usize
    }

    pub fn get(&self, x: i32, y: i32) -> Option<&Tile> {
        if Map::in_bounds(x, y) {
            Some(&self.tiles[Map::idx(x, y)])
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, x: i32, y: i32) -> Option<&mut Tile> {
        if Map::in_bounds(x, y) {
            Some(&mut self.tiles[Map::idx(x, y)])
        } else {
            None
        }
    }

    /// Tile reference; out-of-bounds coordinates yield an ocean tile.
    pub fn tile(&self, x: i32, y: i32) -> Tile {
        self.get(x, y).copied().unwrap_or_default()
    }

    pub fn terrain(&self, x: i32, y: i32) -> Terrain {
        self.get(x, y).map(|t| t.terrain).unwrap_or(Terrain::Ocean)
    }

    pub fn is_land(&self, x: i32, y: i32) -> bool {
        self.get(x, y).map(|t| t.is_land()).unwrap_or(false)
    }

    pub fn is_ocean(&self, x: i32, y: i32) -> bool {
        self.get(x, y).map(|t| t.is_ocean()).unwrap_or(true)
    }

    pub fn neighbours8(x: i32, y: i32) -> impl Iterator<Item = (i32, i32)> {
        Dir8::ALL.iter().map(move |d| {
            let (dx, dy) = d.delta();
            (Map::wrap_x(x + dx), y + dy)
        }).filter(|&(nx, ny)| Map::in_bounds(nx, ny))
    }

    pub fn neighbours4(x: i32, y: i32) -> impl Iterator<Item = (i32, i32)> {
        [(0, -1), (1, 0), (0, 1), (-1, 0)].into_iter().map(move |(dx, dy)| (Map::wrap_x(x + dx), y + dy)).filter(|&(nx, ny)| Map::in_bounds(nx, ny))
    }

    /// Bitmask of orthogonal neighbours with the same terrain: N=1, E=2, S=4, W=8.
    /// Off-map neighbours count as "same" so edges join cleanly.
    pub fn same_terrain_mask(&self, x: i32, y: i32) -> u8 {
        let t = self.terrain(x, y);
        let mut m = 0;
        for (bit, (dx, dy)) in [(1u8, (0, -1)), (2, (1, 0)), (4, (0, 1)), (8, (-1, 0))] {
            let (nx, ny) = (x + dx, y + dy);
            if !Map::in_bounds(nx, ny) || self.terrain(nx, ny) == t {
                m |= bit;
            }
        }
        m
    }

    /// Bitmask of orthogonal neighbours that are land (for coast pieces): N=1, E=2, S=4, W=8.
    pub fn land_mask4(&self, x: i32, y: i32) -> u8 {
        let mut m = 0;
        for (bit, (dx, dy)) in [(1u8, (0, -1)), (2, (1, 0)), (4, (0, 1)), (8, (-1, 0))] {
            if self.is_land(x + dx, y + dy) {
                m |= bit;
            }
        }
        m
    }

    /// Bitmask of the 8 neighbours that are land, bit i = Dir8::ALL[i].
    pub fn land_mask8(&self, x: i32, y: i32) -> u8 {
        let mut m = 0;
        for (i, d) in Dir8::ALL.iter().enumerate() {
            let (dx, dy) = d.delta();
            if self.is_land(x + dx, y + dy) {
                m |= 1 << i;
            }
        }
        m
    }

    /// Bitmask of orthogonal neighbours that are river or ocean (rivers flow into them).
    pub fn river_mask(&self, x: i32, y: i32) -> u8 {
        let mut m = 0;
        for (bit, (dx, dy)) in [(1u8, (0, -1)), (2, (1, 0)), (4, (0, 1)), (8, (-1, 0))] {
            let t = self.terrain(x + dx, y + dy);
            if Map::in_bounds(x + dx, y + dy) && (t == Terrain::River || t == Terrain::Ocean) {
                m |= bit;
            }
        }
        m
    }

    /// Bitmask of orthogonal neighbours with a road, railroad or city: N=1,E=2,S=4,W=8 plus
    /// diagonals NE=16, SE=32, SW=64, NW=128 (bit order of Dir8 is used by `road_mask8`).
    pub fn road_mask8(&self, x: i32, y: i32, rail: bool) -> u8 {
        let mut m = 0;
        for (i, d) in Dir8::ALL.iter().enumerate() {
            let (dx, dy) = d.delta();
            if let Some(t) = self.get(x + dx, y + dy) {
                let has = if rail { t.has(tile_flags::RAILROAD) } else { t.has(tile_flags::ROAD) || t.has(tile_flags::RAILROAD) };
                if has || t.city.is_some() {
                    m |= 1 << i;
                }
            }
        }
        m
    }

    pub fn is_coastal(&self, x: i32, y: i32) -> bool {
        Map::neighbours8(x, y).any(|(nx, ny)| self.is_ocean(nx, ny))
    }

    /// Chebyshev distance, around the east-west edge where that is shorter.
    pub fn dist(a: (i32, i32), b: (i32, i32)) -> i32 {
        Map::delta_x(a.0, b.0).abs().max((a.1 - b.1).abs())
    }

    /// CIV.EXE's travel distance (0x1d6af, 0x1d62e): the longer axis plus half the shorter
    /// one, the horizontal one taken around the edge where that is shorter.
    pub fn civ_distance(a: (i32, i32), b: (i32, i32)) -> i32 {
        let (dx, dy) = (Map::delta_x(a.0, b.0).abs(), (a.1 - b.1).abs());
        dx.max(dy) + dx.min(dy) / 2
    }

    /// The special-resource pattern (`tile_has_special` 0x1a68:0x183c): rows 2..47 only, and
    /// ((x >> 2) * 13 + (y >> 2) * 11 + seed) & 15 == (x & 3) * 4 + (y & 3), seed = the map
    /// seed ds:0x6e80. CIV.EXE tests it on every draw; civrs stores it in `Tile::special`.
    pub fn special_at(seed: u32, x: i32, y: i32) -> bool {
        let x = Map::wrap_x(x);
        (2..48).contains(&y) && ((x >> 2) * 13 + (y >> 2) * 11 + seed as i32) & 15 == (x & 3) * 4 + (y & 3)
    }

    /// The hut pattern (`tile_has_hut` 0x1a68:0x189a, which also needs land, no city and the
    /// "hut taken" bit 0 of the tile's visibility byte clear): rows 2..47 and
    /// ((x >> 2) * 13 + (y >> 2) * 11 + seed + 8) & 31 == (x & 3) * 4 + (y & 3), so a hut in half
    /// of the 4x4 blocks. civrs stores it as `tile_flags::HUT`.
    pub fn hut_at(seed: u32, x: i32, y: i32) -> bool {
        let x = Map::wrap_x(x);
        (2..48).contains(&y) && ((x >> 2) * 13 + (y >> 2) * 11 + seed as i32 + 8) & 31 == (x & 3) * 4 + (y & 3)
    }

    /// The site-value layer at (x,y) (see `Map::site`); computed from the terrain as it is now
    /// when the map has no stored layer.
    pub fn site_at(&self, x: i32, y: i32) -> u8 {
        if !Map::in_bounds(x, y) {
            return 0;
        }
        match self.site.get(Map::idx(x, y)) {
            Some(v) => *v,
            None => super::mapgen::site_value(self, Map::wrap_x(x), y),
        }
    }

    /// Grassland and River squares that give a shield (CIV.EXE 0x134b0): bit 1 of the low
    /// byte of 7x + 11y is clear.
    pub fn shield_at(x: i32, y: i32) -> bool {
        (7 * x + 11 * y) & 2 == 0
    }

    pub fn count_land(&self) -> usize {
        self.tiles.iter().filter(|t| t.is_land()).count()
    }

    /// Number the landmasses as the world generator does (o 0x097b1, `mapgen::number_bodies`):
    /// by size, 1 = largest, 1..14, and 15 for all smaller ones; 8-connected, not around the
    /// east-west edge. Ocean squares get 0.
    pub fn compute_continents(&mut self) {
        let mut page = super::mapgen::Page::from_map(self);
        super::mapgen::number_bodies(&mut page);
        for y in 0..MAP_H as i32 {
            for x in 0..MAP_W as i32 {
                let t = &mut self.tiles[Map::idx(x, y)];
                t.continent = if t.is_land() { page.plane(x, y) } else { 0 };
            }
        }
    }

    /// The numbers of the bodies of water (the water pass of o 0x097b1: by size, 1 = largest,
    /// 15 for all beyond the 14th; 0 on land) and their sizes (ds:0xdd7e: ranks 1..14, 1 for
    /// rank 15), from the terrain as it is now.
    pub fn ocean_numbers(&self) -> (Vec<u8>, [u16; 16]) {
        let mut page = super::mapgen::Page::from_map(self);
        let (_, sizes) = super::mapgen::number_bodies(&mut page);
        let ids = (0..MAP_H as i32)
            .flat_map(|y| (0..MAP_W as i32).map(move |x| (x, y)))
            .map(|(x, y)| if self.is_ocean(x, y) { page.plane(x, y) } else { 0 })
            .collect();
        (ids, sizes)
    }

    pub fn explore(&mut self, slot: Slot, x: i32, y: i32, radius: i32) {
        for yy in (y - radius)..=(y + radius) {
            for xx in (x - radius)..=(x + radius) {
                if let Some(t) = self.get_mut(xx, yy) {
                    t.explored |= 1 << slot;
                }
            }
        }
    }

    pub fn explore_all(&mut self, slot: Slot) {
        for t in self.tiles.iter_mut() {
            t.explored |= 1 << slot;
        }
    }
}
