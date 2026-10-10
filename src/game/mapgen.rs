//! World generation: random worlds (land mass / temperature / climate / age) and the Earth map.

use super::map::{tile_flags, Map, Tile, WorldParams};
use super::rng::Rng;
use crate::assets::EarthMap;
use crate::rules::{Terrain, MAP_H, MAP_W};

const W: i32 = MAP_W as i32;
const H: i32 = MAP_H as i32;

/// Generate a random world.
pub fn generate(params: WorldParams, seed: u32) -> Map {
    let mut rng = Rng::new(seed as u64 ^ 0xC1_7A_11_E5);
    let mut map = Map::empty(params, seed);

    // 1. Land: random walks of "blobs".
    let mut height = vec![0i32; (W * H) as usize];
    let land_mass = params.land_mass as i32;
    let n_blobs = 11 + 5 * land_mass;
    let steps = 55 + 25 * land_mass;
    for _ in 0..n_blobs {
        let mut x = rng.range(4, W - 5);
        let mut y = rng.range(4, H - 5);
        for _ in 0..steps {
            // paint a small cross / 2x2 around (x,y)
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1), (-1, 0), (0, -1)] {
                let (px, py) = (x + dx, y + dy);
                if px >= 2 && px < W - 2 && py >= 2 && py < H - 2 {
                    height[(py * W + px) as usize] += 1;
                }
            }
            let d = rng.below(4);
            match d {
                0 => x += 1,
                1 => x -= 1,
                2 => y += 1,
                _ => y -= 1,
            }
            if x < 3 || x >= W - 3 || y < 3 || y >= H - 3 {
                x = rng.range(4, W - 5);
                y = rng.range(4, H - 5);
            }
        }
    }
    // land where height >= 1; remove single-tile specks for older/smaller worlds
    for i in 0..(W * H) as usize {
        map.tiles[i].terrain = if height[i] >= 1 { Terrain::Grassland } else { Terrain::Ocean };
    }
    // normalise elevation to 0..10 using the 97th percentile of land heights
    let mut hs: Vec<i32> = height.iter().copied().filter(|h| *h >= 1).collect();
    hs.sort_unstable();
    let p97 = hs.get(hs.len() * 97 / 100).copied().unwrap_or(1).max(1);
    for h in height.iter_mut() {
        if *h >= 1 {
            *h = (*h * 10 / p97).min(10);
        }
    }
    // remove tiny islands (< 3 tiles)
    remove_small_islands(&mut map, 3);

    // 2. Temperature by latitude (0 = polar .. 100 = equatorial) with noise.
    // 3. Moisture noise. 4. Elevation from `height`.
    let temp_shift = (params.temperature as i32 - 1) * 12;
    let wet_shift = (params.climate as i32 - 1) * 15;
    let mut moisture = vec![0i32; (W * H) as usize];
    for y in 0..H {
        let mut row = rng.range(30, 70);
        for x in 0..W {
            row += rng.range(-9, 9);
            row = row.clamp(0, 100);
            moisture[(y * W + x) as usize] = row;
        }
    }
    // smooth moisture vertically
    let mut sm = moisture.clone();
    for y in 1..H - 1 {
        for x in 0..W {
            let i = (y * W + x) as usize;
            sm[i] = (moisture[i] * 2 + moisture[i - W as usize] + moisture[i + W as usize]) / 4;
        }
    }
    let moisture = sm;

    for y in 0..H {
        let lat = ((y as f32 - (H as f32 - 1.0) / 2.0).abs() / ((H as f32 - 1.0) / 2.0) * 100.0) as i32; // 0 equator .. 100 pole
        for x in 0..W {
            let i = (y * W + x) as usize;
            if map.tiles[i].is_ocean() {
                continue;
            }
            let temp = (100 - lat + temp_shift + rng.range(-8, 8)).clamp(0, 100);
            let wet = (moisture[i] + wet_shift + rng.range(-5, 5)).clamp(0, 100);
            let elev = height[i];
            let t = classify(temp, wet, elev, &mut rng);
            map.tiles[i].terrain = t;
        }
    }

    // Age: smoothing passes (older world = more erosion = fewer isolated mountains/hills).
    let passes = (2 - params.age as i32).max(0) + 1;
    for _ in 0..passes {
        smooth_terrain(&mut map, &mut rng);
    }

    // Poles.
    for x in 0..W {
        for y in [0, H - 1] {
            let t = &mut map.tiles[(y * W + x) as usize];
            if rng.below(100) < 85 {
                t.terrain = Terrain::Arctic;
            }
        }
        for y in [1, H - 2] {
            let t = &mut map.tiles[(y * W + x) as usize];
            if t.is_land() && rng.below(100) < 60 {
                t.terrain = Terrain::Arctic;
            } else if t.is_ocean() && rng.below(100) < 25 {
                t.terrain = Terrain::Arctic;
            }
        }
    }

    // Rivers.
    let n_rivers = 14 + 5 * params.climate as i32;
    for _ in 0..n_rivers * 4 {
        if count_rivers(&map) >= n_rivers * 5 {
            break;
        }
        try_river(&mut map, &mut rng);
    }

    finish(&mut map, seed, &mut rng);
    map
}

fn classify(temp: i32, wet: i32, elev: i32, rng: &mut Rng) -> Terrain {
    // elevation first
    if elev >= 10 && rng.below(100) < 80 {
        return Terrain::Mountains;
    }
    if elev >= 8 && rng.below(100) < 45 {
        return if rng.below(100) < 40 { Terrain::Mountains } else { Terrain::Hills };
    }
    if elev >= 6 && rng.below(100) < 20 {
        return Terrain::Hills;
    }
    if rng.below(100) < 4 {
        return if rng.below(100) < 50 { Terrain::Hills } else { Terrain::Mountains };
    }
    if temp < 12 {
        return Terrain::Arctic;
    }
    if temp < 25 {
        return if wet > 55 && rng.below(100) < 30 { Terrain::Forest } else { Terrain::Tundra };
    }
    if temp < 55 {
        // temperate
        return if wet < 25 {
            Terrain::Plains
        } else if wet < 60 {
            if rng.below(100) < 25 { Terrain::Forest } else { Terrain::Grassland }
        } else if rng.below(100) < 45 {
            Terrain::Forest
        } else if rng.below(100) < 15 {
            Terrain::Swamp
        } else {
            Terrain::Grassland
        };
    }
    if temp < 80 {
        // warm
        return if wet < 30 {
            Terrain::Desert
        } else if wet < 55 {
            if rng.below(100) < 60 { Terrain::Plains } else { Terrain::Grassland }
        } else if rng.below(100) < 30 {
            Terrain::Forest
        } else {
            Terrain::Grassland
        };
    }
    // tropical
    if wet < 35 {
        Terrain::Desert
    } else if wet < 55 {
        if rng.below(100) < 50 { Terrain::Plains } else { Terrain::Grassland }
    } else if wet < 75 {
        if rng.below(100) < 50 { Terrain::Jungle } else { Terrain::Grassland }
    } else if rng.below(100) < 50 {
        Terrain::Swamp
    } else {
        Terrain::Jungle
    }
}

fn smooth_terrain(map: &mut Map, rng: &mut Rng) {
    let old = map.tiles.clone();
    for y in 0..H {
        for x in 0..W {
            let i = (y * W + x) as usize;
            if old[i].is_ocean() {
                continue;
            }
            let t = old[i].terrain;
            // count same-terrain orthogonal neighbours
            let mut same = 0;
            let mut counts = [0u8; 12];
            for (nx, ny) in Map::neighbours8(x, y) {
                let nt = old[(ny * W + nx) as usize].terrain;
                counts[nt as usize] += 1;
                if nt == t {
                    same += 1;
                }
            }
            if same == 0 && rng.below(100) < 60 {
                // isolated: adopt the most common land neighbour
                let mut best = t;
                let mut bc = 0;
                for (k, c) in counts.iter().enumerate() {
                    if k != Terrain::Ocean as usize && *c > bc {
                        bc = *c;
                        best = Terrain::from_u8(k as u8).unwrap_or(t);
                    }
                }
                map.tiles[i].terrain = best;
            }
        }
    }
}

fn remove_small_islands(map: &mut Map, min: usize) {
    map.compute_continents();
    // compute sizes
    let mut sizes = std::collections::HashMap::new();
    for t in &map.tiles {
        if t.is_land() {
            *sizes.entry(t.continent).or_insert(0usize) += 1;
        }
    }
    for t in map.tiles.iter_mut() {
        if t.is_land() && sizes.get(&t.continent).copied().unwrap_or(0) < min {
            t.terrain = Terrain::Ocean;
        }
    }
}

fn count_rivers(map: &Map) -> i32 {
    map.tiles.iter().filter(|t| t.terrain == Terrain::River).count() as i32
}

fn try_river(map: &mut Map, rng: &mut Rng) {
    // start on hills/mountains/forest/grassland inland, flow to ocean
    let sx = rng.range(2, W - 3);
    let sy = rng.range(3, H - 4);
    let start = map.terrain(sx, sy);
    if !matches!(start, Terrain::Hills | Terrain::Mountains | Terrain::Forest | Terrain::Grassland | Terrain::Plains) {
        return;
    }
    if map.is_coastal(sx, sy) {
        return;
    }
    let mut path = vec![(sx, sy)];
    let (mut x, mut y) = (sx, sy);
    let mut dir = rng.below(4);
    for _ in 0..30 {
        // prefer continuing direction, sometimes turn
        if rng.below(100) < 35 {
            dir = (dir + if rng.below(2) == 0 { 1 } else { 3 }) % 4;
        }
        let (dx, dy) = [(0, -1), (1, 0), (0, 1), (-1, 0)][dir as usize];
        let (nx, ny) = (x + dx, y + dy);
        if !Map::in_bounds(nx, ny) {
            return;
        }
        let t = map.terrain(nx, ny);
        if t == Terrain::Ocean {
            if path.len() >= 3 {
                for (px, py) in path {
                    map.get_mut(px, py).unwrap().terrain = Terrain::River;
                }
            }
            return;
        }
        if t == Terrain::River || path.contains(&(nx, ny)) {
            // join existing river
            if path.len() >= 3 && t == Terrain::River {
                for (px, py) in path {
                    map.get_mut(px, py).unwrap().terrain = Terrain::River;
                }
            }
            return;
        }
        if matches!(t, Terrain::Mountains | Terrain::Arctic) {
            dir = (dir + 2) % 4;
            continue;
        }
        path.push((nx, ny));
        x = nx;
        y = ny;
    }
}

/// Assign specials, grassland shields, huts and continents.
fn finish(map: &mut Map, seed: u32, rng: &mut Rng) {
    for y in 0..H {
        for x in 0..W {
            let i = (y * W + x) as usize;
            let t = &mut map.tiles[i];
            t.special = Map::special_at(seed, x, y) && t.terrain != Terrain::River && !(t.terrain == Terrain::Grassland);
            t.shield = Map::shield_at(x, y);
            if t.terrain == Terrain::Grassland && t.special {
                t.special = false;
            }
            // huts: land, not arctic, sparse
            if t.is_land() && t.terrain != Terrain::Arctic && y > 1 && y < H - 2 {
                let h = ((x / 4) as u32 * 7 + (y / 4) as u32 * 5 + seed / 3) % 16;
                if ((x % 4) * 4 + (y % 4)) as u32 == h && rng.below(100) < 60 {
                    t.set(tile_flags::HUT, true);
                }
            }
        }
    }
    map.compute_continents();
}

/// Build the Earth map from MAP.PIC.
pub fn earth(em: &EarthMap, seed: u32) -> Map {
    let mut rng = Rng::new(seed as u64 ^ 0xEA_47);
    let mut map = Map::empty(WorldParams::normal(), seed);
    map.earth = true;
    for y in 0..H {
        for x in 0..W {
            let code = em.code(x as usize, y as usize);
            let t = Terrain::from_earth_code(code).unwrap_or(Terrain::Ocean);
            map.tiles[(y * W + x) as usize] = Tile { terrain: t, ..Default::default() };
        }
    }
    finish(&mut map, seed, &mut rng);
    map
}

/// Score a tile as a city site centre (sum of yields in the radius, with bonuses).
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
            _ => (info.food as i32, info.shields as i32 + if nt.shield && nt.terrain == Terrain::Grassland { 1 } else { 0 }, info.trade as i32),
        };
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

/// Pick `n` start positions: good sites, far apart, preferring different continents.
pub fn start_positions(map: &Map, n: usize, rng: &mut Rng) -> Vec<(i32, i32)> {
    let mut candidates: Vec<((i32, i32), i32)> = Vec::new();
    for y in 2..H - 2 {
        for x in 1..W - 1 {
            let s = site_score(map, x, y);
            if s > 0 {
                candidates.push(((x, y), s + rng.range(0, 6)));
            }
        }
    }
    candidates.sort_by(|a, b| b.1.cmp(&a.1));
    let mut out: Vec<(i32, i32)> = Vec::new();
    let mut used_continents: Vec<u8> = Vec::new();
    for min_dist in [12, 9, 6, 3] {
        for pass in 0..2 {
            for (pos, _) in &candidates {
                if out.len() >= n {
                    return out;
                }
                if out.iter().any(|o| Map::dist(*o, *pos) < min_dist) {
                    continue;
                }
                let cont = map.tile(pos.0, pos.1).continent;
                if pass == 0 && used_continents.contains(&cont) {
                    continue;
                }
                out.push(*pos);
                used_continents.push(cont);
            }
        }
    }
    out
}

/// Historical start positions on the Earth map, by civ id.
pub fn earth_start(civ: u8) -> (i32, i32) {
    match civ {
        1 => (40, 18),  // Romans
        2 => (48, 21),  // Babylonians
        3 => (42, 13),  // Germans
        4 => (45, 24),  // Egyptians
        5 => (18, 17),  // Americans
        6 => (43, 19),  // Greeks
        7 => (57, 25),  // Indians
        9 => (48, 11),  // Russians
        10 => (46, 40), // Zulus
        11 => (38, 14), // French
        12 => (14, 25), // Aztecs
        13 => (66, 18), // Chinese
        14 => (37, 12), // English
        15 => (62, 14), // Mongols
        _ => (40, 25),
    }
}

/// Nearest acceptable land tile to `p`.
pub fn snap_to_land(map: &Map, p: (i32, i32)) -> (i32, i32) {
    let mut best = p;
    let mut bd = i32::MAX;
    for r in 0..6 {
        for y in (p.1 - r)..=(p.1 + r) {
            for x in (p.0 - r)..=(p.0 + r) {
                if site_score(map, x, y) > 0 {
                    let d = Map::dist(p, (x, y));
                    if d < bd {
                        bd = d;
                        best = (x, y);
                    }
                }
            }
        }
        if bd != i32::MAX {
            break;
        }
    }
    best
}
