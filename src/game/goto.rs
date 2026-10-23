//! GoTo, as CIV.EXE's segment 0x1db0 has it: the same code moves the human's units with a goto
//! order and every computer unit (`move_units` 0x4bdb asks `goto_direction` for the next key
//! whenever the unit's goto bytes +6/+7 are set).
//!
//! It works on two levels. The world is cut into a 20x13 grid of 4x4-square blocks whose links
//! to their 8 neighbours, by land and by sea, were worked out with the map (`BlockLinks`,
//! ds:0xdb44 / ds:0x7f38). A far target is first reduced to the centre of the next block on a
//! breadth-first block path (`block_search` 0x1e0dc); then a Dijkstra-like flood over the 16x16
//! squares around that waypoint (`local_search` 0x1e713) picks the step; if neither finds a
//! way, a greedy step that weighs the remaining distance, terrain and roads (0x1dcfe) is
//! taken. The search ignores units: a unit in the way is simply attacked or blocks the move.
//!
//! Faithful details kept: the cost grid of the last search is reused while the waypoint stays
//! the same, even by a unit of another kind; land blocks are not linked across the east-west
//! seam (overlay 7 0x9edd never computes them), sea blocks are (0xa477); the greedy step does
//! not wrap its distances; lakes of fewer than 5 squares are avoided by the greedy step.
//! Deviations: the links are computed from the map when first needed (CIV.EXE: with the world)
//! and never again; out-of-map squares, which CIV.EXE reads from neighbouring pixel planes,
//! count as ocean.

use super::goals::BlockLinks;
use super::map::{tile_flags, Map, Slot, UnitId};
use super::{Game, UnitOrder};
use crate::rules::{self, Domain, Terrain};

/// CIV.EXE's neighbour table ds:0x1882 / ds:0x18e4: index 0 is the square itself, 1..8 the
/// compass directions N, NE, E, SE, S, SW, W, NW, 9..20 the rest of the city radius and 21..44
/// the outer ring (`barbarians::CLAIM` is the same table).
pub const DX: [i32; 25] = [0, 0, 1, 1, 1, 0, -1, -1, -1, 0, 1, 2, 2, 2, 1, 0, -1, -2, -2, -2, -1, 2, 2, -2, -2];
pub const DY: [i32; 25] = [0, -1, -1, 0, 1, 1, 1, 0, -1, -2, -2, -1, 0, 1, 2, 2, 2, 1, 0, -1, -2, 2, -2, -2, 2];

const BW: i32 = 20;
const BH: i32 = 13;

/// The unit a local search plans for: the real one, or the scratch Militia / Trireme of
/// `block_cost` (unit 127 of civ 0 at ds:0x87c8).
#[derive(Clone, Copy, Debug)]
struct Probe {
    x: i32,
    y: i32,
    sea: bool,
    /// The unit type moves one square a turn (every step costs 3 off roads, 0x1e806).
    one_move: bool,
}

/// `dist_metric` 0x1d43:0x1fe: max(|dx|, |dy|) + min(|dx|, |dy|) / 2, no wrap.
pub fn dist_metric(dx: i32, dy: i32) -> i32 {
    let (a, b) = (dx.abs(), dy.abs());
    a.max(b) + a.min(b) / 2
}

/// A square is on CIV.EXE's map (`in_map` 0x1a68:0x132c): no wrap.
pub fn exe_in_map(x: i32, y: i32) -> bool {
    (0..rules::MAP_W as i32).contains(&x) && Map::in_bounds(x, y)
}

/// |a - b| with CIV.EXE's direction folding (0x181a3, 0x1e027): the angle in eighths, 0..4.
pub fn turn_angle(a: i32, b: i32) -> i32 {
    let d = (a - b).abs();
    if d > 4 {
        8 - d
    } else {
        d
    }
}

impl Game {
    /// Unit byte +8: the direction (1..8) of the unit's last step, 0xff for none.
    pub fn heading(&self, id: UnitId) -> u8 {
        let mem = if self.unit_opt(id).map(|u| u.owner) == Some(0) { &self.barbarian_memory.heading } else { &self.ai_memory.heading };
        mem.get(&id).copied().unwrap_or(0xff)
    }

    pub fn set_heading(&mut self, id: UnitId, d: u8) {
        let barbarian = self.unit_opt(id).map(|u| u.owner) == Some(0);
        let mem = if barbarian { &mut self.barbarian_memory.heading } else { &mut self.ai_memory.heading };
        if d == 0xff {
            mem.remove(&id);
        } else {
            mem.insert(id, d);
        }
    }

    /// Clear unit bytes +6/+7 (the goto).
    pub fn clear_goto(&mut self, id: UnitId) {
        if let Some(u) = self.unit_opt(id) {
            let goto_order = u.order == UnitOrder::Goto;
            let u = self.unit_mut(id);
            u.goto = None;
            if goto_order {
                u.order = UnitOrder::None;
            }
        }
    }

    /// Road bit 0x08 of a square; city squares have it (o 0x1b4f2 builds one).
    fn exe_road(&self, x: i32, y: i32) -> bool {
        let t = self.map.tile(x, y);
        t.has(tile_flags::ROAD) || t.has(tile_flags::RAILROAD) || t.city.is_some()
    }

    /// The owner of the units on a square (`unit_owner_at` 0x1a68:0x14e6, the catalogue's
    /// "city_owner_at"): the occupancy bit of map layer x+160.
    pub fn unit_owner_at(&self, x: i32, y: i32) -> Option<Slot> {
        let x = Map::wrap_x(x);
        self.all_units().find(|u| u.x == x && u.y == y).map(|u| u.owner)
    }

    /// `zoc_blocked` 0x7e6:0x1721: false in a city, else `enemy_adjacent` 0x7e6:0x174c — a
    /// unit of another civilization (treaty or not) on a neighbouring square of the same kind
    /// (land next to land, sea next to sea).
    pub fn zoc_blocked(&self, civ: Slot, x: i32, y: i32) -> bool {
        self.city_at(x, y).is_none() && self.enemy_adjacent(civ, x, y)
    }

    pub fn enemy_adjacent(&self, civ: Slot, x: i32, y: i32) -> bool {
        let land = self.map.is_land(x, y);
        Map::neighbours8(x, y).any(|(nx, ny)| self.map.is_land(nx, ny) == land && self.all_units().any(|u| u.x == nx && u.y == ny && u.owner != civ))
    }

    /// The block links of the map, worked out on first use.
    fn block_links(&mut self) -> &BlockLinks {
        if self.ai_memory.links.is_none() {
            let links = self.compute_block_links();
            self.ai_memory.links = Some(links);
        }
        self.ai_memory.links.as_ref().unwrap()
    }

    fn block_mask(&mut self, bx: i32, by: i32, sea: bool) -> u8 {
        if !(0..BW).contains(&bx) || !(0..BH).contains(&by) {
            return 0;
        }
        let l = self.block_links();
        let i = (bx * BH + by) as usize;
        if sea {
            l.sea[i]
        } else {
            l.land[i]
        }
    }

    /// Overlay 7 0x9edd (land) and 0xa17a (sea).
    ///
    /// Land: a block's representative square is the first land square of (1,1), (2,1), (1,2),
    /// (2,2) inside it; it is linked to its N, NE, E and SE neighbour blocks (and they back) when
    /// that block's representative is on the same continent and a local search from one to
    /// the other for a Militia costs less than 20 (0x1ec12 with limit 20).
    ///
    /// Sea: rows 0..11 only; the representative is the first ocean square of the same four;
    /// linked in all 8 directions to a block whose representative is in the same body of water
    /// when a 2x2 window slid five squares from this block's (1,1) square toward it keeps two or
    /// more ocean squares on the map (the eastward link from row 11 needs no window). Column 0
    /// is then linked west, column 19 east, except off the map's corners (0xa477..0xa49c).
    fn compute_block_links(&mut self) -> BlockLinks {
        let n = (BW * BH) as usize;
        let mut land = vec![0u8; n];
        let mut sea = vec![0u8; n];
        let ocean = |g: &Game, x: i32, y: i32| !exe_in_map(x, y) || g.map.is_ocean(x, y);
        let rep = |g: &Game, x: i32, y: i32, want_sea: bool| -> Option<(i32, i32)> {
            [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)].into_iter().find(|&(px, py)| exe_in_map(px, py) && ocean(g, px, py) == want_sea)
        };
        for bx in 0..BW {
            for by in 0..BH {
                let (x, y) = (4 * bx + 1, 4 * by + 1);
                let Some(r) = rep(self, x, y, false) else { continue };
                let cont = self.continent_exe(r.0, r.1);
                for k in 1..=4usize {
                    let Some(t) = rep(self, 4 * DX[k] + x, 4 * DY[k] + y, false) else { continue };
                    if self.continent_exe(t.0, t.1) != cont {
                        continue;
                    }
                    let probe = Probe { x: r.0, y: r.1, sea: false, one_move: true };
                    let Some(cost) = self.probe_cost(probe, t, 20) else { continue };
                    if cost >= 20 {
                        continue;
                    }
                    land[(bx * BH + by) as usize] |= 1 << (k - 1);
                    let (nx, ny) = (bx + DX[k], by + DY[k]);
                    if (0..BW).contains(&nx) && (0..BH).contains(&ny) {
                        land[(nx * BH + ny) as usize] |= 1 << ((k + 3) & 7);
                    }
                }
            }
        }
        for bx in 0..BW {
            for by in 0..12 {
                let (x, y) = (4 * bx + 1, 4 * by + 1);
                let Some(r) = rep(self, x, y, true) else { continue };
                let body = self.continent_exe(r.0, r.1);
                for k in 1..=8usize {
                    let Some(t) = rep(self, 4 * DX[k] + x, 4 * DY[k] + y, true) else { continue };
                    if self.continent_exe(t.0, t.1) != body {
                        continue;
                    }
                    let mut ok = true;
                    let (mut px, mut py) = (x, y);
                    for _ in 0..5 {
                        px += DX[k];
                        py += DY[k];
                        let water = [(px, py), (px + 1, py), (px, py + 1), (px + 1, py + 1)].iter().filter(|&&(qx, qy)| exe_in_map(qx, qy) && self.map.is_ocean(qx, qy)).count();
                        if water < 2 {
                            ok = false;
                            break;
                        }
                    }
                    if !ok && !(by == 11 && k == 3) {
                        continue;
                    }
                    let ny = by + DY[k];
                    if ny >= 12 {
                        continue;
                    }
                    sea[(bx * BH + by) as usize] |= 1 << (k - 1);
                    let nx = bx + DX[k];
                    if (0..BW).contains(&nx) && (0..=12).contains(&ny) {
                        sea[(nx * BH + ny) as usize] |= 1 << ((k + 3) & 7);
                    }
                }
            }
        }
        for by in 0..12 {
            sea[by as usize] |= 0xe0;
            sea[(19 * BH + by) as usize] |= 0x0e;
        }
        sea[0] &= 0x7f;
        sea[11] &= 0xdf;
        sea[(19 * BH) as usize] &= 0xfd;
        sea[(19 * BH + 11) as usize] &= 0xf7;
        BlockLinks { land, sea }
    }

    /// The next key for a unit with a goto (CIV.EXE 0x1db04 `goto_direction(civ, unit)`): 1..8,
    /// 0 for none, -1 when no way was found (the goto is then cleared). The human's units go
    /// straight to an adjacent target (and drop the goto), the rest as described in the module
    /// documentation; the chosen direction becomes the unit's heading.
    pub fn goto_direction(&mut self, id: UnitId) -> i32 {
        let u = self.unit(id).clone();
        let Some((gx, gy)) = u.goto else { return 0 };
        let civ = u.owner;
        if civ == self.human_slot {
            let (dx, dy) = (gx - u.x, gy - u.y);
            if dx.abs() <= 1 && dy.abs() <= 1 {
                let (sx, sy) = (dx.signum(), dy.signum());
                self.clear_goto(id);
                return (1..=8).find(|&k| DX[k] == sx && DY[k] == sy).map(|k| k as i32).unwrap_or(0);
            }
        }
        let mut waypoint = (gx, gy);
        if u.info().domain != Domain::Air {
            let mut failed_near = false;
            let adx = (gx - u.x).abs();
            if (adx < 7 || adx > 73) && (gy - u.y).abs() < 7 {
                let d = self.local_search(self.probe_of(id), waypoint, 999).0;
                if d != -1 {
                    return d;
                }
                failed_near = true;
            }
            let (found, w) = self.block_search(id);
            waypoint = w;
            if found || !failed_near {
                let (d, _) = self.local_search(self.probe_of(id), waypoint, 999);
                if d != -1 {
                    return d;
                }
                waypoint = (gx, gy);
            }
        }
        self.greedy_step(id, waypoint)
    }

    fn probe_of(&self, id: UnitId) -> Probe {
        let u = self.unit(id);
        Probe { x: u.x, y: u.y, sea: u.info().domain == Domain::Sea, one_move: u.info().moves == 1 }
    }

    /// `goto_block_cost` 0x1ec12: the cost of a local search from `from` to `to` for a
    /// scratch Militia (Trireme at sea), when the two are close enough (CIV.EXE tests |dx| <= 7
    /// OR |dy| <= 7).
    fn probe_cost(&mut self, probe: Probe, to: (i32, i32), limit: i32) -> Option<i32> {
        if (probe.x - to.0).abs() > 7 && (probe.y - to.1).abs() > 7 {
            return None;
        }
        let (d, cost) = self.local_search(probe, to, limit);
        (d != -1).then_some(cost)
    }

    /// `goto_local_search` 0x1e713: flood the 16x16 squares around `w` from `w` (cost 1; +1 on a
    /// road from a road, +3 for one-move units, else 3 x the terrain's move cost; land units
    /// keep to land and ships to the sea, cities let both in; at most 225 squares are taken
    /// from the queue), stopping where the cost passes the cheapest found for the probe's own
    /// square. Then the step to the neighbour of least cost below 99, ties going to fewer units
    /// there (4 each) plus distance to `w`. Returns (direction or -1, cost found).
    fn local_search(&mut self, p: Probe, w: (i32, i32), limit: i32) -> (i32, i32) {
        let (ox, oy) = (w.0 - 8, w.1 - 8);
        // the probe's square in window coordinates (CIV.EXE indexes it unwrapped)
        let near = |x: i32| -> i32 {
            let mut x = x;
            if (x - w.0).abs() >= 72 {
                x += if x > w.0 { -80 } else { 80 };
            }
            x
        };
        let cell = |x: i32, y: i32| -> Option<usize> {
            let (cx, cy) = (x - ox, y - oy);
            ((0..16).contains(&cx) && (0..16).contains(&cy)).then(|| (cx * 16 + cy) as usize)
        };
        let mut found = 0;
        let reuse = self.ai_memory.goto_cache.waypoint == Some(w)
            && self.ai_memory.goto_cache.grid.len() == 256
            && cell(near(p.x), p.y).map(|i| self.ai_memory.goto_cache.grid[i] != 0).unwrap_or(false);
        if !reuse {
            let mut grid = vec![0u8; 256];
            let mut qx = [0i32; 256];
            let mut qy = [0i32; 256];
            let (mut head, mut tail) = (0usize, 0usize);
            qx[tail] = w.0;
            qy[tail] = w.1;
            tail = 1;
            if let Some(i) = cell(w.0, w.1) {
                grid[i] = 1;
            }
            found = limit;
            loop {
                let (cx, cy) = (qx[head], qy[head]);
                head = (head + 1) & 0xff;
                let c = cell(cx, cy).map(|i| grid[i] as i32).unwrap_or(0);
                if c <= found {
                    if Map::wrap_x(cx) == p.x && cy == p.y {
                        found = c;
                    } else {
                        let road = self.exe_road(Map::wrap_x(cx), cy);
                        for k in 1..=8 {
                            let (nx, ny) = (cx + DX[k], cy + DY[k]);
                            if (nx - w.0).abs() >= 8 || (ny - w.1).abs() >= 8 || !Map::in_bounds(nx, ny) {
                                continue;
                            }
                            let wx = Map::wrap_x(nx);
                            let t = self.map.terrain(wx, ny);
                            if (t == Terrain::Ocean) != p.sea && self.city_at(wx, ny).is_none() {
                                continue;
                            }
                            let cost = if road && self.exe_road(wx, ny) {
                                c + 1
                            } else if p.one_move {
                                c + 3
                            } else {
                                c + 3 * t.info().move_cost as i32
                            };
                            let Some(i) = cell(nx, ny) else { continue };
                            if grid[i] != 0 && grid[i] as i32 <= cost {
                                continue;
                            }
                            grid[i] = cost.min(255) as u8;
                            qx[tail] = nx;
                            qy[tail] = ny;
                            tail = (tail + 1) & 0xff;
                        }
                    }
                }
                if head == tail || head >= 225 {
                    break;
                }
            }
            self.ai_memory.goto_cache = super::goals::GotoCache { waypoint: Some(w), grid };
        }
        if limit <= found {
            return (-1, found);
        }
        let mut best = 99;
        let mut dir = -1;
        let mut tie = 0;
        for k in 1..=8 {
            let nx = near(p.x + DX[k]);
            let ny = p.y + DY[k];
            if (nx - w.0).abs() >= 8 || (ny - w.1).abs() >= 8 || !Map::in_bounds(0, ny) {
                continue;
            }
            let wx = Map::wrap_x(nx);
            if (self.map.terrain(wx, ny) == Terrain::Ocean) != p.sea && self.city_at(wx, ny).is_none() {
                continue;
            }
            let Some(i) = cell(nx, ny) else { continue };
            let gv = self.ai_memory.goto_cache.grid[i] as i32;
            if gv == 0 {
                continue;
            }
            let crowd = |g: &Game| 4 * g.units_at(wx, ny).len() as i32 + Map::civ_distance(w, (wx, ny));
            if gv < best {
                best = gv;
                dir = k as i32;
                tie = crowd(self);
            } else if gv == best {
                let t = crowd(self);
                if t < tie {
                    dir = k as i32;
                    tie = t;
                }
            }
        }
        (dir, found)
    }

    /// `goto_block_entry` 0x1e522: the block of (x,y) when it has links of the right kind,
    /// else the neighbour block with links nearest to (x,y) that a scratch unit can reach (a
    /// local search of limit 18 from its first square of the right kind).
    fn block_entry(&mut self, x: i32, y: i32, sea: bool) -> Option<(i32, i32)> {
        let (bx, by) = (x / 4, y / 4);
        if self.block_mask(bx, by, sea) != 0 {
            return Some((bx, by));
        }
        let mut best = 99;
        let mut pick = None;
        for k in 1..=8 {
            let (nx, ny) = (bx + DX[k], by + DY[k]);
            if self.block_mask(nx, ny, sea) == 0 {
                continue;
            }
            let d = dist_metric(x - 4 * nx - 1, y - 4 * ny - 1);
            if d >= best {
                continue;
            }
            let (sx, sy) = (4 * nx + 1, 4 * ny + 1);
            let tile = [(sx, sy), (sx + 1, sy), (sx + 1, sy + 1), (sx, sy + 1)].into_iter().find(|&(tx, ty)| (!exe_in_map(tx, ty) || self.map.is_ocean(tx, ty)) == sea);
            let Some(t) = tile else { continue };
            let probe = Probe { x: t.0, y: t.1, sea, one_move: !sea };
            if self.probe_cost(probe, (x, y), 18).is_some() {
                best = d;
                pick = Some((nx, ny));
            }
        }
        pick
    }

    /// `goto_block_search` 0x1e0dc: breadth-first over the block links from the target's block
    /// to the unit's; the waypoint is the centre square (4b+1, moved one east and then one
    /// south when of the wrong kind) of the unit's neighbour block nearest the target (ties
    /// to the one closer to the goto square), or the goto square itself. Returns whether a
    /// block path exists and the waypoint.
    fn block_search(&mut self, id: UnitId) -> (bool, (i32, i32)) {
        let u = self.unit(id).clone();
        let goto = u.goto.unwrap_or(u.pos());
        let sea = u.info().domain == Domain::Sea;
        let Some(start) = self.block_entry(u.x, u.y, sea) else {
            // CIV.EXE returns the goto's y here, true unless it is row 0 (0x1e18a)
            return (goto.1 != 0, goto);
        };
        // a target without a block of its own leaves the start block as the target (0x1e19d)
        let target = self.block_entry(goto.0, goto.1, sea).unwrap_or(start);
        let mut dist = vec![0u8; (BW * BH) as usize];
        let mut qx = [0i32; 256];
        let mut qy = [0i32; 256];
        qx[0] = target.0;
        qy[0] = target.1;
        let (mut head, mut tail) = (0usize, 1usize);
        dist[(target.0 * BH + target.1) as usize] = 1;
        let mut found = false;
        loop {
            let (bx, by) = (qx[head], qy[head]);
            if (bx, by) == start {
                found = true;
                break;
            }
            let here = dist[(bx * BH + by) as usize];
            head = (head + 1) & 0xff;
            let mask = self.block_mask(bx, by, sea);
            for k in 1..=8 {
                if mask & (1 << (k - 1)) == 0 {
                    continue;
                }
                let mut nx = bx + DX[k];
                if nx == BW {
                    nx = 0;
                }
                if nx == -1 {
                    nx = BW - 1;
                }
                let ny = by + DY[k];
                if !(0..BH).contains(&ny) {
                    continue;
                }
                let i = (nx * BH + ny) as usize;
                if dist[i] == 0 {
                    dist[i] = here.saturating_add(1);
                    qx[tail] = nx;
                    qy[tail] = ny;
                    tail = (tail + 1) & 0xff;
                }
            }
            if head == tail {
                break;
            }
        }
        if !found {
            return (false, goto);
        }
        let mut best = 99;
        let mut pick: Option<usize> = None;
        let mut near = 0;
        let mask = self.block_mask(start.0, start.1, sea);
        for k in 1..=8 {
            if mask & (1 << (k - 1)) == 0 {
                continue;
            }
            let mut nx = start.0 + DX[k];
            if nx == BW {
                nx = 0;
            }
            if nx == -1 {
                nx = BW - 1;
            }
            let ny = start.1 + DY[k];
            if !(0..BH).contains(&ny) {
                continue;
            }
            let v = dist[(nx * BH + ny) as usize] as i32;
            if v == 0 {
                continue;
            }
            let d = Map::civ_distance(goto, (4 * nx + 1, 4 * ny + 1));
            if v < best {
                best = v;
                pick = Some(k);
                near = d;
            } else if v == best && d < near {
                pick = Some(k);
                near = d;
            }
        }
        let Some(k) = pick else { return (true, goto) };
        let mut w = (Map::wrap_x(4 * (start.0 + DX[k]) + 1), 4 * (start.1 + DY[k]) + 1);
        let wrong = |g: &Game, (x, y): (i32, i32)| (!exe_in_map(x, y) || g.map.is_ocean(x, y)) != sea;
        if wrong(self, w) {
            w.0 += 1;
            if wrong(self, w) {
                w.1 += 1;
            }
        }
        (true, w)
    }

    /// The greedy step of 0x1dcfe toward `w`: for each neighbour the unit may enter (empty or
    /// own units, of its kind unless it flies, not from one enemy-adjacent square to another;
    /// an own city always; no lake under 5 squares), 4 x the remaining distance |dx| + |dy| +
    /// max (unwrapped) + |dx| + |dy| + the step's cost (1 road to road, else 3 x terrain for
    /// units with more than one move, 3) + the turn away from the heading squared; the human's
    /// units never step farther away. A step back against the heading ends the unit's turn
    /// instead; no step drops the goto.
    fn greedy_step(&mut self, id: UnitId, w: (i32, i32)) -> i32 {
        let u = self.unit(id).clone();
        let civ = u.owner;
        let (dx, dy) = (w.0 - u.x, w.1 - u.y);
        let metric = |dx: i32, dy: i32| dx.abs().max(dy.abs()) + dx.abs() + dy.abs();
        let dist = metric(dx, dy);
        if dx == 0 && dy == 0 {
            self.set_heading(id, 0xff);
            self.clear_goto(id);
            self.unit_mut(id).moves_left = 0;
            return -1;
        }
        let sea = u.info().domain == Domain::Sea;
        let air = u.info().domain == Domain::Air;
        let road_here = self.exe_road(u.x, u.y);
        let zoc_here = self.zoc_blocked(civ, u.x, u.y);
        let heading = self.heading(id);
        let mut best = 9999;
        let mut pick = 0;
        for k in 1..=8 {
            let (rdx, rdy) = (dx - DX[k], dy - DY[k]);
            let m = metric(rdx, rdy);
            if civ == self.human_slot && m > dist {
                continue;
            }
            let (nx, ny) = (Map::wrap_x(u.x + DX[k]), u.y + DY[k]);
            if !Map::in_bounds(nx, ny) {
                continue;
            }
            let t = self.map.terrain(nx, ny);
            let owner = self.unit_owner_at(nx, ny);
            let own_city = self.city_at(nx, ny).is_some() && self.last_occupant(nx, ny) == civ;
            let ok = if owner.is_none() || owner == Some(civ) {
                let kind_ok = (t == Terrain::Ocean) == sea;
                if kind_ok && !(zoc_here && self.zoc_blocked(civ, nx, ny)) {
                    true
                } else {
                    air || own_city
                }
            } else {
                own_city
            };
            if !ok {
                continue;
            }
            if t == Terrain::Ocean && self.ocean_size(nx, ny) < 5 {
                continue;
            }
            let step = if road_here && self.exe_road(nx, ny) {
                1
            } else if u.info().moves > 1 {
                3 * t.info().move_cost as i32
            } else {
                3
            };
            let mut cost = step + 4 * m + rdx.abs() + rdy.abs();
            if heading != 0xff {
                let a = turn_angle(heading as i32, k as i32);
                cost += a * a;
            }
            if cost < best {
                best = cost;
                pick = k as i32;
            }
        }
        if heading != 0xff && (heading as i32 ^ 4) == pick {
            self.unit_mut(id).moves_left = 0;
            pick = 0;
        }
        if pick == 0 {
            self.set_heading(id, 0xff);
            self.clear_goto(id);
            return -1;
        }
        self.set_heading(id, pick as u8);
        pick
    }

    /// The step of a goto as a direction (`goto_direction`, with its side effects), `None` when
    /// there is none.
    pub fn goto_step(&mut self, id: UnitId) -> Option<super::Dir8> {
        let d = self.goto_direction(id);
        (1..=8).contains(&d).then(|| super::Dir8::ALL[(d - 1) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;
    use crate::rules::unit_ids;

    fn flat() -> Game {
        let mut g = Game::new(GameSetup { seed: 3, ..Default::default() }, None);
        for t in g.map.tiles.iter_mut() {
            t.terrain = Terrain::Ocean;
            t.flags = 0;
            t.city = None;
            t.continent = 0;
        }
        for y in 4..30 {
            for x in 4..40 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Grassland;
                t.continent = 1;
            }
        }
        let ids: Vec<_> = g.all_units().map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        g.cities.clear();
        g.barbarian_memory = Default::default();
        g.ai_memory = Default::default();
        g
    }

    #[test]
    fn a_near_goto_follows_the_local_search_and_turns_round_a_lake() {
        let mut g = flat();
        let slot = (1..8).find(|s| *s != g.human_slot).unwrap();
        // a wall of water between the unit and the target, open in the south
        for y in 4..20 {
            g.map.get_mut(12, y).unwrap().terrain = Terrain::Ocean;
        }
        let id = g.add_unit(slot, unit_ids::LEGION, 10, 10);
        g.set_goto(id, 14, 10);
        let mut pos = vec![g.unit(id).pos()];
        for _ in 0..40 {
            let d = g.goto_direction(id);
            if d <= 0 {
                break;
            }
            let (x, y) = g.unit(id).pos();
            let u = g.unit_mut(id);
            u.x = x + DX[d as usize];
            u.y = y + DY[d as usize];
            pos.push(g.unit(id).pos());
            if g.unit(id).pos() == (14, 10) {
                break;
            }
        }
        assert_eq!(*pos.last().unwrap(), (14, 10), "{pos:?}");
        assert!(pos.iter().any(|p| p.1 >= 20), "goes round the south end: {pos:?}");
        assert!(pos.iter().all(|p| g.map.is_land(p.0, p.1)));
    }

    #[test]
    fn far_gotos_go_through_block_waypoints() {
        let mut g = flat();
        let slot = (1..8).find(|s| *s != g.human_slot).unwrap();
        let id = g.add_unit(slot, unit_ids::LEGION, 5, 5);
        g.set_goto(id, 37, 27);
        let (found, w) = g.block_search(id);
        assert!(found);
        // the next block toward the target: (1,1)'s south-east neighbour, centre (9,9)
        assert_eq!(w, (9, 9));
        let d = g.goto_direction(id);
        assert_eq!(d, 4, "south-east");
    }

    #[test]
    fn the_humans_unit_steps_onto_an_adjacent_target_and_drops_the_goto() {
        let mut g = flat();
        let h = g.human_slot;
        let id = g.add_unit(h, unit_ids::LEGION, 10, 10);
        g.set_goto(id, 11, 9);
        assert_eq!(g.goto_direction(id), 2);
        assert_eq!(g.unit(id).goto, None);
    }

    #[test]
    fn the_greedy_step_refuses_to_reverse_its_heading() {
        let mut g = flat();
        let slot = (1..8).find(|s| *s != g.human_slot).unwrap();
        let id = g.add_unit(slot, unit_ids::LEGION, 10, 10);
        // only the straight way back is left toward the target
        g.map.get_mut(9, 9).unwrap().terrain = Terrain::Ocean;
        g.map.get_mut(9, 11).unwrap().terrain = Terrain::Ocean;
        g.set_goto(id, 8, 10);
        g.set_heading(id, 3);
        // west is the reverse of east (3 ^ 4 = 7)
        assert_eq!(g.greedy_step(id, (8, 10)), -1);
        assert_eq!(g.unit(id).moves_left, 0);
        assert_eq!(g.unit(id).goto, None);
    }

    #[test]
    fn land_blocks_link_along_a_continent_but_not_across_the_sea() {
        let mut g = flat();
        let links = g.compute_block_links();
        // block (2,2) (squares 8..11) links east to (3,2)
        assert_ne!(links.land[(2 * BH + 2) as usize] & 0x04, 0);
        // nothing links into the open ocean at block (15,10)
        assert_eq!(links.land[(15 * BH + 10) as usize], 0);
        assert_ne!(links.sea[(15 * BH + 10) as usize], 0);
        assert_eq!(dist_metric(-3, 2), 4);
        assert_eq!(turn_angle(1, 7), 2);
    }
}
