//! The world map: 80x50 tiles, no horizontal wrap.

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
        Map { tiles: vec![Tile::default(); MAP_W * MAP_H], params, earth: false, seed }
    }

    #[inline]
    pub fn in_bounds(x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < MAP_W && (y as usize) < MAP_H
    }

    #[inline]
    pub fn idx(x: i32, y: i32) -> usize {
        y as usize * MAP_W + x as usize
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
            (x + dx, y + dy)
        }).filter(|&(nx, ny)| Map::in_bounds(nx, ny))
    }

    pub fn neighbours4(x: i32, y: i32) -> impl Iterator<Item = (i32, i32)> {
        [(0, -1), (1, 0), (0, 1), (-1, 0)].into_iter().map(move |(dx, dy)| (x + dx, y + dy)).filter(|&(nx, ny)| Map::in_bounds(nx, ny))
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

    /// Chebyshev distance.
    pub fn dist(a: (i32, i32), b: (i32, i32)) -> i32 {
        (a.0 - b.0).abs().max((a.1 - b.1).abs())
    }

    /// Does the tile carry the special resource pattern? (Civ1 formula.)
    pub fn special_at(seed: u32, x: i32, y: i32) -> bool {
        ((x % 4) * 4 + (y % 4)) as u32 == ((x / 4) as u32 * 13 + (y / 4) as u32 * 11 + seed) % 16
    }

    /// Grassland shield pattern.
    pub fn shield_at(x: i32, y: i32) -> bool {
        (x * 3 + y * 5 + (x / 4) + (y / 4)).rem_euclid(4) < 2
    }

    pub fn count_land(&self) -> usize {
        self.tiles.iter().filter(|t| t.is_land()).count()
    }

    /// Flood-fill landmasses (8-connected), numbering them by size (1 = largest).
    pub fn compute_continents(&mut self) {
        let n = MAP_W * MAP_H;
        let mut label = vec![0u16; n];
        let mut sizes: Vec<(u16, usize)> = Vec::new();
        let mut next = 1u16;
        let mut stack = Vec::new();
        for i in 0..n {
            if label[i] != 0 || self.tiles[i].is_ocean() {
                continue;
            }
            let mut size = 0;
            stack.push(i);
            label[i] = next;
            while let Some(j) = stack.pop() {
                size += 1;
                let (x, y) = ((j % MAP_W) as i32, (j / MAP_W) as i32);
                for (nx, ny) in Map::neighbours8(x, y) {
                    let k = Map::idx(nx, ny);
                    if label[k] == 0 && self.tiles[k].is_land() {
                        label[k] = next;
                        stack.push(k);
                    }
                }
            }
            sizes.push((next, size));
            next += 1;
        }
        sizes.sort_by(|a, b| b.1.cmp(&a.1));
        let mut rank = vec![0u8; next as usize];
        for (r, (lab, _)) in sizes.iter().enumerate() {
            rank[*lab as usize] = (r + 1).min(255) as u8;
        }
        for i in 0..n {
            self.tiles[i].continent = if label[i] == 0 { 0 } else { rank[label[i] as usize] };
        }
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
