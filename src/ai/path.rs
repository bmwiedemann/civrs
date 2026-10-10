//! A* pathfinding over the map for a unit (used by goto and the AI).
//!
//! The search precomputes an occupancy grid (foreign units/cities, own transports) so that
//! each node expansion is O(1); the game's own queries (`units_at`, `owner_at`) are linear in
//! the number of units and far too slow to call thousands of times per unit per turn.

use crate::game::{Game, Map, Unit};
use crate::rules::{Domain, MAP_H, MAP_W};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Expansion budget for the human goto (long paths across the whole map).
pub const DEFAULT_LIMIT: usize = 6000;

/// Returns the path from the unit's position to `goal` (inclusive of both ends), or None.
pub fn astar(g: &Game, u: &Unit, goal: (i32, i32)) -> Option<Vec<(i32, i32)>> {
    astar_limited(g, u, goal, DEFAULT_LIMIT)
}

/// Per-tile occupancy used by the search.
struct Grid {
    /// Tile holds a foreign unit or city (relative to the moving unit's owner).
    foreign: Vec<bool>,
    /// Tile holds one of our own ships with cargo capacity (land units may board).
    own_transport: Vec<bool>,
}

fn build_grid(g: &Game, u: &Unit) -> Grid {
    let n = MAP_W * MAP_H;
    let mut grid = Grid { foreign: vec![false; n], own_transport: vec![false; n] };
    for other in g.all_units() {
        if !Map::in_bounds(other.x, other.y) {
            continue;
        }
        let i = Map::idx(other.x, other.y);
        if other.owner != u.owner {
            grid.foreign[i] = true;
        } else if other.is_sea() && other.info().capacity > 0 && other.id != u.id {
            grid.own_transport[i] = true;
        }
    }
    for c in g.all_cities() {
        if c.owner != u.owner {
            grid.foreign[Map::idx(c.x, c.y)] = true;
        }
    }
    grid
}

fn enterable(g: &Game, u: &Unit, grid: &Grid, x: i32, y: i32) -> bool {
    if !Map::in_bounds(x, y) {
        return false;
    }
    let t = &g.map.tiles[Map::idx(x, y)];
    match u.info().domain {
        Domain::Land => t.is_land() || grid.own_transport[Map::idx(x, y)],
        Domain::Sea => t.is_ocean() || t.city.map(|c| g.city(c).owner == u.owner).unwrap_or(false),
        Domain::Air => true,
    }
}

/// A* with an explicit expansion budget (the AI uses a small budget for short hops).
pub fn astar_limited(g: &Game, u: &Unit, goal: (i32, i32), limit: usize) -> Option<Vec<(i32, i32)>> {
    let start = (u.x, u.y);
    if start == goal {
        return Some(vec![start]);
    }
    if !Map::in_bounds(goal.0, goal.1) || !Map::in_bounds(start.0, start.1) {
        return None;
    }
    let grid = build_grid(g, u);
    let n = MAP_W * MAP_H;
    let h = |p: (i32, i32)| Map::dist(p, goal);
    let mut open: BinaryHeap<Reverse<(i32, i32, usize)>> = BinaryHeap::new();
    let mut best: Vec<i32> = vec![i32::MAX; n];
    let mut came: Vec<u16> = vec![u16::MAX; n];
    let si = Map::idx(start.0, start.1);
    let gi = Map::idx(goal.0, goal.1);
    best[si] = 0;
    open.push(Reverse((h(start) * 3, 0, si)));
    let mut expanded = 0;
    while let Some(Reverse((_, cost, cur))) = open.pop() {
        if cur == gi {
            let mut path = vec![goal];
            let mut p = cur;
            while came[p] != u16::MAX {
                p = came[p] as usize;
                path.push(((p % MAP_W) as i32, (p / MAP_W) as i32));
            }
            path.reverse();
            return Some(path);
        }
        if cost > best[cur] {
            continue;
        }
        expanded += 1;
        if expanded > limit {
            break;
        }
        let cp = ((cur % MAP_W) as i32, (cur / MAP_W) as i32);
        for (nx, ny) in Map::neighbours8(cp.0, cp.1) {
            let ni = Map::idx(nx, ny);
            let is_goal = ni == gi;
            if !is_goal && !enterable(g, u, &grid, nx, ny) {
                continue;
            }
            // avoid foreign units/cities except at the goal
            if !is_goal && grid.foreign[ni] {
                continue;
            }
            let step = g.move_cost(u, cp, (nx, ny)).max(1) as i32;
            let nc = cost + step;
            if nc < best[ni] {
                best[ni] = nc;
                came[ni] = cur as u16;
                open.push(Reverse((nc + h((nx, ny)) * 3, nc, ni)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;
    use crate::rules::{unit_ids, Terrain};

    #[test]
    fn astar_crosses_an_open_plain() {
        let mut g = Game::new(GameSetup { seed: 3, num_civs: 7, ..Default::default() }, None);
        for y in 2..20 {
            for x in 2..30 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Grassland;
                t.flags = 0;
            }
        }
        let ids: Vec<_> = g.all_units().map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        let u = g.add_unit(1, unit_ids::LEGION, 5, 5);
        let path = astar(&g, g.unit(u), (15, 9)).expect("path");
        assert_eq!(path.first(), Some(&(5, 5)));
        assert_eq!(path.last(), Some(&(15, 9)));
        assert_eq!(path.len(), 11, "diagonal moves make it a Chebyshev-distance path");
        // ocean is impassable for land units
        for x in 10..12 {
            for y in 0..crate::rules::MAP_H as i32 {
                g.map.get_mut(x, y).unwrap().terrain = Terrain::Ocean;
            }
        }
        assert!(astar(&g, g.unit(u), (15, 9)).is_none());
        // a ship goes around the land
        let s = g.add_unit(1, unit_ids::SAIL, 10, 1);
        assert!(astar(&g, g.unit(s), (11, 30)).is_some());
    }
}
