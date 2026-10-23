//! Goody huts (CIV.EXE 0x0978d `hut_outcome`), barbarian raids and uprisings (0x02d00
//! `barbarians()`, called at the end of every turn from 0x2620) and the orders of barbarian
//! units (0x18dc3, the civ-0 branch of the AI unit routine 0x16433).

use std::collections::HashMap;

#[cfg(test)]
use super::economy::EXE_RADIUS;
use super::map::{tile_flags, Dir8, Map, Slot, UnitId};
use super::{CityId, Game, GameEvent, HutOutcome, UnitOrder};
use crate::rules::{self, advance_ids as adv, unit_ids, AiRole, Domain, UnitTypeId, MAP_H, MAP_W};

/// CIV.EXE's direction offsets 1..8 (ds:0x1882 dx / ds:0x18e4 dy): N, NE, E, SE, S, SW, W, NW,
/// the same order as `Dir8::ALL`.
const DIR_DX: [i32; 9] = [0, 0, 1, 1, 1, 0, -1, -1, -1];
const DIR_DY: [i32; 9] = [0, -1, -1, 0, 1, 1, 1, 0, -1];

/// The 45 squares a new city claims on the site-value layer (ds:0x1882 / ds:0x18e4 indices
/// 0..0x2c, found_city o 0x1b375).
#[rustfmt::skip]
const CLAIM: [(i32, i32); 45] = [
    (0, 0), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1),
    (0, -2), (1, -2), (2, -1), (2, 0), (2, 1), (1, 2), (0, 2), (-1, 2), (-2, 1), (-2, 0), (-2, -1), (-1, -2),
    (2, 2), (2, -2), (-2, -2), (-2, 2),
    (0, -3), (1, -3), (2, -3), (3, -2), (3, -1), (3, 0), (3, 1), (3, 2), (2, 3), (1, 3), (0, 3), (-1, 3), (-2, 3),
    (-3, 2), (-3, 1), (-3, 0), (-3, -1), (-3, -2), (-2, -3), (-1, -3),
];

/// Bookkeeping CIV.EXE keeps in its unit records and map layers but the civrs save does not
/// carry; it starts empty after a load, as the original's layers start on a new map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BarbarianMemory {
    /// Unit byte +8, the barbarian's last direction (1..8; 0xff from `create_unit` 0x8b51).
    pub heading: HashMap<UnitId, u8>,
    /// Map layer (x+160, y) low bits: the civ whose units were last on the square (0 at the
    /// start, which is also the barbarians), written by `set_tile_owner` 0x1a68:0x1392.
    pub occupant: Vec<u8>,
    /// Ocean body of each sea square (1 = the largest) and the size of each body, the
    /// counterpart of the continent numbers (lazily built).
    oceans: Option<(Vec<u8>, Vec<u16>)>,
}

/// One order of the barbarian unit routine (0x18dc3), which returns keyboard commands like
/// every AI unit (see `move_units` 0x4440).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarbarianCommand {
    /// Keys 1..8: move or attack.
    Step(Dir8),
    /// 0: follow the goto that was just set.
    Goto,
    /// ' ': done for this turn.
    Skip,
    /// 'f'.
    Fortify,
    /// 'u': a ship next to an empty shore wakes its passengers.
    Unload,
    /// The unit was deleted (0x7e6:0xf0c).
    Disband,
}

impl Game {
    /// Has any civilisation (not the barbarians) discovered the advance? CIV.EXE keeps the
    /// first discoverer at far 0x26f1:[0x4b0 + 2 * adv] & 7 (set by grant_advance 0xc756),
    /// which survives the discoverer's destruction; known advances of destroyed civs count
    /// here the same way.
    pub fn discovered_by_anyone(&self, a: u8) -> bool {
        self.players.iter().skip(1).any(|p| p.knows(a))
    }

    /// The city nearest to (x,y) and its distance (0x1d43:0xf8 with the distance of 0x1d6af):
    /// the first city slot at the smallest distance, any owner; 999 without cities.
    pub fn nearest_city_exe(&self, x: i32, y: i32) -> (Option<CityId>, i32) {
        let mut best = (None, 999);
        for c in self.all_cities() {
            let d = Map::civ_distance((x, y), (c.x, c.y));
            if d < best.1 {
                best = (Some(c.id), d);
            }
        }
        best
    }

    /// Continent number of a square as 0x1a68:0x1948 reads it: the landmass for land, the
    /// ocean body for sea (both numbered by size, 1 = largest).
    pub fn continent_exe(&mut self, x: i32, y: i32) -> u8 {
        let t = self.map.tile(x, y);
        if t.is_land() {
            return t.continent;
        }
        self.ocean_bodies();
        self.barbarian_memory.oceans.as_ref().map(|(id, _)| id[Map::idx(Map::wrap_x(x), y)]).unwrap_or(0)
    }

    /// Size of the body of water a sea square belongs to (0x1a68:0x1963, ds:0xdd7e[id]).
    pub fn ocean_size(&mut self, x: i32, y: i32) -> i32 {
        let id = self.continent_exe(x, y) as usize;
        self.barbarian_memory.oceans.as_ref().and_then(|(_, sizes)| sizes.get(id)).copied().unwrap_or(0) as i32
    }

    /// Number the bodies of water (8-connected, like the landmasses) by size. Unverified:
    /// CIV.EXE numbers them in world generation (o 0x097a8) with a run-merging scan whose
    /// cap on the number of ids was not decoded; here every body gets its own number.
    fn ocean_bodies(&mut self) {
        if self.barbarian_memory.oceans.is_some() {
            return;
        }
        let n = MAP_W * MAP_H;
        let mut label = vec![0u16; n];
        let mut sizes: Vec<(u16, u16)> = Vec::new();
        let mut stack = Vec::new();
        for i in 0..n {
            if label[i] != 0 || !self.map.tiles[i].is_ocean() {
                continue;
            }
            let lab = sizes.len() as u16 + 1;
            let mut size = 0u16;
            label[i] = lab;
            stack.push(i);
            while let Some(j) = stack.pop() {
                size += 1;
                let (x, y) = ((j % MAP_W) as i32, (j / MAP_W) as i32);
                for (nx, ny) in Map::neighbours8(x, y) {
                    let k = Map::idx(nx, ny);
                    if label[k] == 0 && self.map.tiles[k].is_ocean() {
                        label[k] = lab;
                        stack.push(k);
                    }
                }
            }
            sizes.push((lab, size));
        }
        let mut ranked = sizes.clone();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        let mut rank = vec![0u8; sizes.len() + 1];
        let mut by_rank = vec![0u16; ranked.len().min(254) + 1];
        for (r, (lab, size)) in ranked.iter().enumerate() {
            let r = (r + 1).min(255);
            rank[*lab as usize] = r as u8;
            if r < by_rank.len() {
                by_rank[r] = *size;
            }
        }
        let ids = label.iter().map(|l| if *l == 0 { 0 } else { rank[*l as usize] }).collect();
        self.barbarian_memory.oceans = Some((ids, by_rank));
    }

    /// The civ last on a square (map layer x+160, see `BarbarianMemory::occupant`).
    pub fn last_occupant(&self, x: i32, y: i32) -> Slot {
        if let Some(c) = self.city_at(x, y) {
            return self.city(c).owner;
        }
        if let Some(o) = self.units_at(x, y).first().map(|u| self.unit(*u).owner) {
            return o;
        }
        self.barbarian_memory.occupant.get(Map::idx(Map::wrap_x(x), y)).copied().unwrap_or(0)
    }

    /// Record the owner of a unit placed on a square (`set_tile_owner` 0x1a68:0x1392).
    pub fn note_occupant(&mut self, slot: Slot, x: i32, y: i32) {
        if !Map::in_bounds(x, y) {
            return;
        }
        let m = &mut self.barbarian_memory.occupant;
        if m.len() != MAP_W * MAP_H {
            *m = vec![0; MAP_W * MAP_H];
        }
        m[Map::idx(Map::wrap_x(x), y)] = slot;
    }

    /// The site-value layer (page 2, x+80) at a square: the value the world generator left
    /// there (`Map::site_at`, `mapgen::site_value_with`, o 0x09c5f), then the claims of the
    /// cities in founding order (found_city o 0x1b702): the 21 squares of a city radius take
    /// the founder's number, the other 24 squares within 3 lower values >= 8 to 8.
    /// Unverified: CIV.EXE also writes the owner over the squares a city works (0x13233),
    /// clears another civ's number where a unit steps (0x683a) and restores the generator's
    /// values (kept at (x+80, y+50)) when a city is destroyed (0xa8f3); none of that is modelled.
    pub fn hut_site_value(&self, x: i32, y: i32) -> i32 {
        let x = Map::wrap_x(x);
        let mut value = self.map.site_at(x, y) as i32;
        for c in self.all_cities() {
            match CLAIM.iter().position(|(dx, dy)| (Map::wrap_x(c.x + dx), c.y + dy) == (x, y)) {
                Some(0..=20) => value = c.owner as i32,
                Some(_) if value >= 8 => value = 8,
                _ => {}
            }
        }
        value
    }

    /// The site value as `compute_site_values` (o 0x09c5f) wrote it, before any city claimed
    /// the square: the copy at page 2 (x+80, y+50), which nothing changes later (read by the
    /// computer's Settlers near the human's border, 0x16c24).
    pub fn site_value_pristine(&self, x: i32, y: i32) -> i32 {
        self.map.site_at(Map::wrap_x(x), y) as i32
    }

    /// The square's byte of map layer (x+80, y) as the computer's units read it (0x16ac5): the
    /// owner of the city that works the square (written by the city routine, 0x13233; civrs
    /// takes `territory_owner`), else `hut_site_value` (the generator's value with the city
    /// claims). Values of 8 and more are city sites, lower ones owners.
    pub fn site_layer(&self, x: i32, y: i32) -> i32 {
        if !Map::in_bounds(x, y) {
            return 0;
        }
        match self.territory_owner(Map::wrap_x(x), y) {
            Some(o) => o as i32,
            None => self.hut_site_value(x, y),
        }
    }

    /// Unit `id` has just entered a hut tile (`hut_outcome` 0x0978d): barbarians and air
    /// units get nothing. With d = distance to the nearest city, random(4) picks:
    /// 0: d >= 4 on a site-value square whose low 3 bits exceed 4 -> an advanced tribe founds
    ///    a city for the civ; else (d >= 4) 50$ of metal; d < 4 -> mercenaries;
    /// 1: on turn 0 or after 1000 AD the metal, else scrolls of ancient wisdom: the first
    ///    advance from random(72) upwards (wrapping) the civ lacks and has both prerequisites of;
    /// 2: the metal;
    /// 3: with cities and d >= 4 a horde of barbarians, else mercenaries.
    pub fn enter_hut(&mut self, id: UnitId) {
        let u = self.unit(id).clone();
        let slot = u.owner;
        if slot == 0 || u.info().domain == Domain::Air {
            return;
        }
        // without any city CIV.EXE reads the coordinates of the record before the city table
        // (ds:0x70d4/5); taken as far away here (unverified)
        let (_, d) = self.nearest_city_exe(u.x, u.y);
        let cities = self.cities_of(slot).len();
        let roll = self.rng.below(4);
        let outcome = match roll {
            0 if d < 4 => self.hut_mercenaries(),
            0 if self.hut_site_value(u.x, u.y) & 7 > 4 => HutOutcome::AdvancedTribe(None),
            1 if self.turn != 0 && self.year() <= 1000 => match self.hut_scroll(slot) {
                Some(a) => HutOutcome::Advance(a),
                None => HutOutcome::EmptyScrolls,
            },
            0..=2 => HutOutcome::Gold(50),
            _ if d >= 4 && cities > 0 => HutOutcome::Barbarians,
            _ => self.hut_mercenaries(),
        };
        // the message comes first, then the effect (CIV.EXE shows the box before acting)
        let mut outcome = outcome;
        let event_at = self.events.len();
        match &outcome {
            HutOutcome::Gold(g) => self.player_mut(slot).gold += g,
            HutOutcome::Advance(a) => {
                self.grant_advance(slot, *a, super::science::AdvanceSource::Discovered);
                self.push_event(GameEvent::AdvanceAcquired { slot, adv: *a, source: "hut".into() });
            }
            HutOutcome::Mercenaries(k) => {
                // create_unit (0x8b51) does not set the moves byte; none are given here
                let m = self.add_unit(slot, *k, u.x, u.y);
                self.unit_mut(m).moves_left = 0;
            }
            HutOutcome::AdvancedTribe(_) => {
                if let Some(c) = self.hut_city(slot, u.x, u.y) {
                    outcome = HutOutcome::AdvancedTribe(Some(c));
                }
            }
            HutOutcome::Barbarians => {
                // neighbours 1, 1+s, 1+2s, ... up to 8 with s = clamp(4 - cities, 1, 4): empty,
                // city-free land gets Legion on squares costing more than 2 (Mountains) and
                // Cavalry elsewhere (0x9a87..0x9b81)
                let step = (4 - cities as i32).clamp(1, 4) as usize;
                let mut i = 1;
                while i <= 8 {
                    let (nx, ny) = (Map::wrap_x(u.x + DIR_DX[i]), u.y + DIR_DY[i]);
                    if Map::in_bounds(nx, ny) && self.owner_at(nx, ny).is_none() && self.map.is_land(nx, ny) {
                        let kind = if self.map.terrain(nx, ny).info().move_cost > 2 { unit_ids::LEGION } else { unit_ids::CAVALRY };
                        self.add_barbarian(kind, nx, ny);
                    }
                    i += step;
                }
                self.refresh_vision(0);
            }
            HutOutcome::EmptyScrolls | HutOutcome::Empty | HutOutcome::Settlers => {}
        }
        self.refresh_vision(slot);
        let effects = self.events.split_off(event_at.min(self.events.len()));
        self.push_event(GameEvent::HutEntered { unit: id, slot, outcome });
        self.events.extend(effects);
    }

    /// "A friendly tribe of skilled mercenaries": Legion or Cavalry (random(2), 0x9a46).
    fn hut_mercenaries(&mut self) -> HutOutcome {
        HutOutcome::Mercenaries(if self.rng.below(2) != 0 { unit_ids::LEGION } else { unit_ids::CAVALRY })
    }

    /// The advance in the scrolls (0x994f..0x99e1): from random(72) upwards, wrapping, the
    /// first one the civ lacks with both prerequisites known, within 999 tries. The four unused
    /// slots and Future Tech have the prerequisite 0x7f, which nobody knows.
    fn hut_scroll(&mut self, slot: Slot) -> Option<u8> {
        let mut a = self.rng.below(72) as usize;
        let p = self.player(slot);
        for _ in 0..999 {
            let info = &rules::ADVANCES[a];
            if a < 67 && !p.knows(a as u8) && info.prereq.iter().all(|r| r.map(|r| p.knows(r)).unwrap_or(true)) {
                return Some(a as u8);
            }
            a = (a + 1) % 72;
        }
        None
    }

    /// "An advanced tribe" founds a size-1 city on the hut square (overlay 20 +0, found_city
    /// o 0x1b375); none past the 128-city limit.
    fn hut_city(&mut self, slot: Slot, x: i32, y: i32) -> Option<CityId> {
        if self.all_cities().count() >= 128 || self.city_at(x, y).is_some() {
            return None;
        }
        let name = self.next_city_name(slot);
        let c = self.add_city(slot, name, x, y);
        // found_city o 0x1b375 drops the city-site goals within 4 (0x1b38c)
        self.remove_goals_near(slot, super::goals::goal::SETTLE, x, y, 4);
        if !self.player(slot).human {
            let p = self.ai_production(c);
            self.city_mut(c).production = p;
        }
        self.push_event(GameEvent::CityFounded { city: c });
        Some(c)
    }

    /// A barbarian unit as `create_unit` (0x8b51) makes it: no veteran flag, no heading.
    fn add_barbarian(&mut self, kind: UnitTypeId, x: i32, y: i32) -> UnitId {
        let id = self.add_unit(0, kind, x, y);
        self.barbarian_memory.heading.remove(&id);
        self.players[0].alive = true;
        self.players[0].in_game = true;
        id
    }

    /// First turn of barbarian activity: (5 - difficulty) * 30 (CIV.EXE 0x2d08), 150 on
    /// Chieftain down to 30 on Emperor.
    pub fn barbarian_start_turn(&self) -> u32 {
        (5 - self.setup.difficulty as u32) * 30
    }

    /// `barbarians()` (CIV.EXE 0x02d00), once at the end of every turn.
    ///
    /// (a) When (turn + 1) is a multiple of 8 and nobody has discovered Automobile: a raid from
    /// the sea. A random ocean square (random(80), 3 + random(44)) without units on a body of
    /// at least 16 squares gets a Sail, or a Frigate from turn 300 on, carrying clamp(turn / 150
    /// + 1, 1, 3) Legions (Knights once Explosives is known) and a leader (Diplomat), all
    /// sentried aboard.
    ///
    /// (b) A random city slot (random(128)) that holds a city of the human revolts with
    /// probability plunder value % (0x13d7:0x68d): up to 98 squares are tried, each within +-6
    /// of the city nearest to the previous try, for one at least 6 from its nearest city, on
    /// the same continent, land, without units or enemies next to it; there clamp(random(size
    /// / 2), 2, 99) units plus a leader appear: Cavalry with every fifth to eighth a Chariot,
    /// or Musketeers and Cannon once Industrialization is known.
    pub fn barbarian_spawn(&mut self) {
        if self.turn < self.barbarian_start_turn() || self.ended.is_some() {
            return;
        }
        if (self.turn + 1) & 7 == 0 && !self.discovered_by_anyone(adv::AUTOMOBILE) {
            self.barbarian_sea_raid();
        }
        self.barbarian_uprising();
    }

    fn barbarian_sea_raid(&mut self) {
        // CIV.EXE loops until it finds a square; bounded here
        let mut at = None;
        for _ in 0..100_000 {
            let x = self.rng.below(80) as i32;
            let y = self.rng.below(44) as i32 + 3;
            if self.map.is_ocean(x, y) && self.units_at(x, y).is_empty() && self.ocean_size(x, y) >= 16 {
                at = Some((x, y));
                break;
            }
        }
        let Some((x, y)) = at else { return };
        let count = (self.turn as i32 / 150 + 1).clamp(1, 3);
        let ship = self.add_barbarian(if count < 3 { unit_ids::SAIL } else { unit_ids::FRIGATE }, x, y);
        let troops = if self.discovered_by_anyone(adv::EXPLOSIVES) { unit_ids::KNIGHTS } else { unit_ids::LEGION };
        for i in 0..=count {
            let kind = if i == count { unit_ids::DIPLOMAT } else { troops };
            let u = self.add_barbarian(kind, x, y);
            let um = self.unit_mut(u);
            um.carried_by = Some(ship);
            um.order = UnitOrder::Sentry;
            um.leader = kind == unit_ids::DIPLOMAT;
        }
        self.refresh_vision(0);
    }

    fn barbarian_uprising(&mut self) {
        let slot = self.rng.below(128) as usize;
        let Some(c) = self.cities.get(slot).and_then(|c| c.as_ref()).map(|c| c.id) else { return };
        if self.city(c).owner != self.human_slot {
            return;
        }
        let chance = self.plunder_value(c);
        if self.rng.below(100) as i32 >= chance {
            return;
        }
        let mut around = c;
        let mut tries = 0;
        let found = loop {
            let (cx, cy) = self.city(around).pos();
            let x = Map::wrap_x(cx + self.rng.below(13) as i32 - 6);
            let y = cy + self.rng.below(13) as i32 - 6;
            let (near, d) = self.nearest_city_exe(x, y);
            if let Some(n) = near {
                around = n;
            }
            tries += 1;
            if tries >= 99 {
                break None;
            }
            // squares off the top or bottom are read from other map layers in CIV.EXE; they
            // are never taken here
            if d < 6 || !Map::in_bounds(x, y) {
                continue;
            }
            // the civ passed to enemy_adjacent (0x2f00) is an uninitialised local; any unit of
            // a civilisation next to the square is taken to count (unverified)
            if Map::neighbours8(x, y).any(|(nx, ny)| self.map.is_land(nx, ny) && self.owner_at(nx, ny).map(|o| o != 0).unwrap_or(false)) {
                continue;
            }
            if !self.map.is_land(x, y) || self.owner_at(x, y).is_some() {
                continue;
            }
            let n = self.city(around).pos();
            if self.continent_exe(n.0, n.1) != self.continent_exe(x, y) {
                continue;
            }
            break Some((x, y));
        };
        let Some((x, y)) = found else { return };
        let near = self.city(around).clone();
        if near.owner == self.human_slot && self.map.tile(x, y).explored_by(self.human_slot) {
            self.push_event(GameEvent::BarbarianUprising { at: (x, y), target: self.human_slot });
        }
        let count = (self.rng.below((near.size / 2) as u32) as i32).clamp(2, 99);
        let modern = self.discovered_by_anyone(adv::INDUSTRIALIZATION);
        for i in 0..=count {
            let kind = match (i == count, modern, i & 4 != 0) {
                (true, _, _) => unit_ids::DIPLOMAT,
                (_, true, true) => unit_ids::CANNON,
                (_, true, false) => unit_ids::MUSKETEERS,
                (_, false, true) => unit_ids::CHARIOT,
                (_, false, false) => unit_ids::CAVALRY,
            };
            let u = self.add_barbarian(kind, x, y);
            self.unit_mut(u).leader = kind == unit_ids::DIPLOMAT;
        }
        self.refresh_vision(0);
    }

    /// Delete a barbarian unit silently (0x7e6:0xf0c), its passengers with it.
    fn drop_barbarian(&mut self, id: UnitId) -> BarbarianCommand {
        self.remove_unit(id);
        self.barbarian_memory.heading.remove(&id);
        BarbarianCommand::Disband
    }

    /// The other unit after `id` in its square's stack (unit byte +0xa), if any.
    fn stack_next(&self, id: UnitId) -> Option<UnitId> {
        let u = self.unit(id);
        let here = self.units_at(u.x, u.y);
        let i = here.iter().position(|o| *o == id)?;
        (here.len() > 1).then(|| here[(i + 1) % here.len()])
    }

    /// `zoc_blocked` (0x09581) for the barbarians: a unit of a civilisation on a neighbouring
    /// square of the same kind (land or sea); never in a city.
    fn barbarian_zoc(&self, x: i32, y: i32) -> bool {
        if self.city_at(x, y).is_some() {
            return false;
        }
        let land = self.map.is_land(x, y);
        Map::neighbours8(x, y).any(|(nx, ny)| self.map.is_land(nx, ny) == land && self.owner_at(nx, ny).map(|o| o != 0).unwrap_or(false))
    }

    /// The next order of barbarian unit `id` (CIV.EXE 0x18dc3); deletions and the gotos it
    /// sets happen here, the moves in the caller (ai::unit_ai::play_barbarians).
    pub fn barbarian_command(&mut self, id: UnitId) -> BarbarianCommand {
        let u = self.unit(id).clone();
        if u.y < 2 || u.y >= 48 {
            return self.drop_barbarian(id);
        }
        let zoc_here = self.barbarian_zoc(u.x, u.y);
        if u.info().role == AiRole::Transport {
            return self.barbarian_ship(id);
        }
        let (near, d) = self.nearest_city_exe(u.x, u.y);
        if d == 0 && (self.stack_next(id).is_none() || u.info().role == AiRole::Defend) {
            return BarbarianCommand::Fortify;
        }
        // every fourth turn a unit away from any city, or whose nearest city is on another
        // continent, gives up (0x1913f)
        let here_cont = self.continent_exe(u.x, u.y);
        if (id as u32 + self.turn) & 3 == 0 {
            let lost = match near {
                Some(c) => {
                    let (cx, cy) = self.city(c).pos();
                    self.continent_exe(cx, cy) != here_cont || d > 8
                }
                None => true,
            };
            if lost {
                return self.drop_barbarian(id);
            }
        }
        // the direction of a human city on this continent (raw signs, no wrap), unless its owner
        // is down to one city and less than 100$ (0x19187..0x19225)
        let mut sign = (-2, -2);
        if let Some(c) = near {
            let city = self.city(c).clone();
            let o = city.owner;
            if self.continent_exe(city.x, city.y) == here_cont && self.player(o).human {
                sign = ((city.x - u.x).signum(), (city.y - u.y).signum());
            }
            if o == 0 || (self.cities_of(o).len() < 2 && self.player(o).gold < 100) {
                sign = (-2, -2);
            }
            if sign.0 != -2 {
                // goto the city for the pathfinder's step (0x1db0:0x4, with its side effects),
                // kept as the heading (0x19261; -1 becomes 0xff, none)
                self.unit_mut(id).goto = Some(city.pos());
                let d = self.goto_direction(id);
                self.set_heading(id, d as u8);
            }
        }
        self.unit_mut(id).goto = None;
        let leader = u.kind == unit_ids::DIPLOMAT;
        let at_sea = self.map.is_ocean(u.x, u.y);
        if leader {
            // a leader with an escort stays with it on land (0x19286)
            if let Some(next) = self.stack_next(id) {
                if !at_sea && self.unit(next).kind != unit_ids::DIPLOMAT {
                    return BarbarianCommand::Skip;
                }
            }
            if at_sea {
                if d < 3 {
                    return self.drop_barbarian(id);
                }
            } else {
                // follow another barbarian within 3, else give up far from cities
                let other = self.all_units().filter(|o| o.owner == 0 && o.id != id).map(|o| (Map::civ_distance(o.pos(), u.pos()), o.id)).min();
                if let Some((od, o)) = other {
                    if od <= 3 && self.unit(o).kind != unit_ids::DIPLOMAT {
                        self.unit_mut(id).goto = Some(self.unit(o).pos());
                        return BarbarianCommand::Goto;
                    }
                }
                if (self.turn as i32 & 7) + 4 <= d {
                    return self.drop_barbarian(id);
                }
            }
        }
        let sea_unit = u.info().domain == Domain::Sea;
        let heading = self.barbarian_memory.heading.get(&id).copied().unwrap_or(0xff);
        let mut best = (-999, 0usize);
        for i in 1..=8usize {
            let (nx, ny) = (Map::wrap_x(u.x + DIR_DX[i]), u.y + DIR_DY[i]);
            if !Map::in_bounds(nx, ny) || self.map.is_ocean(nx, ny) != sea_unit {
                continue;
            }
            let there = self.units_at(nx, ny).first().copied();
            let score = if leader {
                let mut s = match there {
                    Some(o) if self.unit(o).owner == 0 => 99,
                    Some(_) => -99,
                    None => 0,
                };
                let (_, cd) = self.nearest_city_exe(nx, ny);
                s += self.map.terrain(nx, ny).info().move_cost as i32 + self.rng.below(4) as i32 + 4 * cd;
                s
            } else {
                if let Some(c) = self.city_at(nx, ny) {
                    if self.city(c).owner != 0 {
                        if at_sea {
                            continue;
                        }
                        return BarbarianCommand::Step(Dir8::ALL[i - 1]);
                    }
                }
                let t = self.map.tile(nx, ny);
                let mut s = self.rng.below(6) as i32;
                if t.has(tile_flags::IRRIGATION) || t.has(tile_flags::MINE) {
                    s += 6;
                }
                if self.last_occupant(nx, ny) != 0 {
                    s += 4;
                }
                if let Some(o) = there {
                    let ou = self.unit(o);
                    if ou.owner == 0 {
                        if !at_sea {
                            continue;
                        }
                        s -= 20;
                    } else {
                        if at_sea {
                            continue;
                        }
                        if ou.kind != unit_ids::DIPLOMAT {
                            s += 99;
                        }
                    }
                }
                if zoc_here && !sea_unit && there.is_none() && self.barbarian_zoc(nx, ny) {
                    continue;
                }
                if heading as usize == i {
                    s += 6;
                }
                if DIR_DX[i] == sign.0 {
                    s += 2;
                }
                if DIR_DY[i] == sign.1 {
                    s += 2;
                }
                s
            };
            if score > best.0 {
                best = (score, i);
            }
        }
        // a lone unit in a city stays there unless it can attack (0x195e0)
        if self.city_at(u.x, u.y).is_some() && self.stack_next(id).is_none() && best.0 < 99 {
            return BarbarianCommand::Skip;
        }
        if best.1 == 0 {
            self.barbarian_memory.heading.insert(id, 0);
            return BarbarianCommand::Skip;
        }
        self.barbarian_memory.heading.insert(id, best.1 as u8);
        BarbarianCommand::Step(Dir8::ALL[best.1 - 1])
    }

    /// The ship branch of 0x18dc3 (unit role 5).
    fn barbarian_ship(&mut self, id: UnitId) -> BarbarianCommand {
        let u = self.unit(id).clone();
        if self.stack_next(id).is_none() {
            return self.drop_barbarian(id);
        }
        let (near, d) = self.nearest_city_exe(u.x, u.y);
        for i in 1..=8usize {
            let (nx, ny) = (Map::wrap_x(u.x + DIR_DX[i]), u.y + DIR_DY[i]);
            let occupant = self.owner_at(nx, ny);
            if d <= 8 && Map::in_bounds(nx, ny) && self.map.is_land(nx, ny) && occupant.is_none() && ny > 1 && ny < 48 {
                // the human is told when the landing is near one of his cities on that
                // continent, on a square he knows (0x18f1d)
                let human = self.human_slot;
                if let Some(c) = near {
                    let city = self.city(c).clone();
                    if self.map.tile(nx, ny).explored_by(human) && city.owner == human && self.continent_exe(city.x, city.y) == self.continent_exe(nx, ny) {
                        let near_city = self.nearest_city_exe(nx, ny).0;
                        self.push_event(GameEvent::BarbariansLanded { at: (nx, ny), near_city, target: human });
                    }
                }
                self.unit_mut(id).moves_left = 0;
                return BarbarianCommand::Unload;
            }
            if occupant.map(|o| o > 0).unwrap_or(false) {
                return BarbarianCommand::Step(Dir8::ALL[i - 1]);
            }
        }
        if u.goto.is_none() {
            // the city with the most (plunder + 50) / (distance + 1), 0x19052
            let mut best = (0, None);
            for c in self.all_cities() {
                let v = (self.plunder_value(c.id) + 50) / (Map::civ_distance(u.pos(), c.pos()) + 1);
                if v > best.0 {
                    best = (v, Some(c.pos()));
                }
            }
            match best.1 {
                Some(p) => self.unit_mut(id).goto = Some(p),
                None => return self.drop_barbarian(id),
            }
        }
        BarbarianCommand::Goto
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{tile_flags, GameSetup, MoveOutcome};
    use crate::rules::{Difficulty, Terrain};

    fn plain_world(seed: u64) -> Game {
        let mut g = Game::new(GameSetup { seed, num_civs: 7, ..Default::default() }, None);
        for t in g.map.tiles.iter_mut() {
            t.terrain = Terrain::Ocean;
            t.flags = 0;
            t.city = None;
        }
        for y in 5..30 {
            for x in 5..30 {
                g.map.get_mut(x, y).unwrap().terrain = Terrain::Plains;
            }
        }
        g.map.compute_continents();
        // the generator's site layer no longer fits the terrain: compute it from the terrain
        g.map.site.clear();
        let ids: Vec<UnitId> = g.all_units().map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        let cities: Vec<CityId> = g.all_cities().map(|c| c.id).collect();
        for c in cities {
            g.cities[c as usize] = None;
        }
        g
    }

    fn hut_outcome(g: &mut Game, kind: UnitTypeId, at: (i32, i32)) -> HutOutcome {
        g.map.get_mut(at.0 + 1, at.1).unwrap().set(tile_flags::HUT, true);
        let u = g.add_unit(1, kind, at.0, at.1);
        g.take_events();
        assert_eq!(g.try_move(u, Dir8::E).unwrap(), MoveOutcome::EnteredHut);
        g.take_events()
            .into_iter()
            .find_map(|e| match e {
                GameEvent::HutEntered { outcome, .. } => Some(outcome),
                _ => None,
            })
            .expect("hut message")
    }

    #[test]
    fn site_value_matches_the_world_generator_formula() {
        let mut g = plain_world(3);
        // an all-Plains radius without specials: 21 squares of 7, 8 of them doubled, index 0
        // doubled again: 7 * (21 + 8 + 2) = 217 -> (217 - 120) / 8 = 12 -> 12 / 2 + 8 = 14
        let plain = |g: &Game, x: i32, y: i32| !EXE_RADIUS.iter().any(|(dx, dy)| Map::special_at(g.map.seed, x + *dx as i32, y + *dy as i32));
        let (x, y) = (8..27).flat_map(|y| (8..27).map(move |x| (x, y))).find(|(x, y)| plain(&g, *x, *y)).expect("a radius without specials");
        assert_eq!(g.hut_site_value(x, y), 14);
        // a Horses special (Plains special, score 11) on the doubled index 0 square, north of
        // the centre: +4 * 4 = 16 -> (233 - 120) / 8 = 14 -> 15
        let (hx, hy) = (8..27).flat_map(|y| (8..27).map(move |x| (x, y))).find(|(x, y)| {
            Map::special_at(g.map.seed, *x, *y - 1) && EXE_RADIUS.iter().skip(1).all(|(dx, dy)| !Map::special_at(g.map.seed, x + *dx as i32, y + *dy as i32))
        }).expect("a lone special north");
        assert_eq!(g.hut_site_value(hx, hy), 15);
        // not a Plains/Grassland/River centre, or at the edge: 0
        g.map.get_mut(x, y).unwrap().terrain = Terrain::Hills;
        assert_eq!(g.hut_site_value(x, y), 0);
        assert_eq!(g.hut_site_value(1, 15), 0);
        // a city's claim lowers it to 8
        g.map.get_mut(x, y).unwrap().terrain = Terrain::Plains;
        g.add_city(2, "Claim".into(), x + 3, y + 2);
        assert_eq!(g.hut_site_value(x, y), 8);
    }

    #[test]
    fn hut_outcomes_follow_hut_outcome() {
        let mut g = plain_world(11);
        let mut seen = [0usize; 5];
        for i in 0..400 {
            let ids: Vec<UnitId> = g.all_units().map(|u| u.id).collect();
            for id in ids {
                g.remove_unit(id);
            }
            let cities: Vec<CityId> = g.all_cities().map(|c| c.id).collect();
            for c in cities {
                g.remove_city(c);
            }
            g.turn = 10;
            g.player_mut(1).advances = 0;
            // half the time a city of the finder 2 squares away (distance < 4)
            let near = i % 2 == 0;
            if near {
                g.add_city(1, "Near".into(), 12, 10);
            }
            let gold = g.player(1).gold;
            match hut_outcome(&mut g, unit_ids::LEGION, (10, 10)) {
                HutOutcome::Gold(50) => {
                    assert_eq!(g.player(1).gold, gold + 50);
                    seen[0] += 1;
                }
                HutOutcome::Advance(a) => {
                    assert!(g.player(1).knows(a));
                    seen[1] += 1;
                }
                HutOutcome::Mercenaries(k) => {
                    assert!(k == unit_ids::LEGION || k == unit_ids::CAVALRY);
                    assert_eq!(g.units_at(11, 10).len(), 2);
                    seen[2] += 1;
                }
                HutOutcome::Barbarians => {
                    assert!(!near, "no horde next to a city");
                    seen[3] += 1;
                }
                HutOutcome::AdvancedTribe(c) => {
                    assert!(!near);
                    assert_eq!(g.city_at(11, 10), c);
                    seen[4] += 1;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(seen.iter().take(3).all(|n| *n > 0), "{seen:?}");
        // without cities of the finder there is never a horde: plain_world gives the human
        // no city, the horde needs one; with the city near, never
        assert_eq!(seen[3], 0, "{seen:?}");
    }

    #[test]
    fn horde_needs_a_city_and_distance_and_grows_with_cities() {
        let mut g = plain_world(5);
        g.turn = 10;
        let home = g.add_city(1, "Home".into(), 25, 25);
        let mut hordes = 0;
        for _ in 0..200 {
            let ids: Vec<UnitId> = g.all_units().map(|u| u.id).collect();
            for id in ids {
                g.remove_unit(id);
            }
            let tribes: Vec<CityId> = g.all_cities().filter(|c| c.id != home).map(|c| c.id).collect();
            for c in tribes {
                g.remove_city(c);
            }
            if hut_outcome(&mut g, unit_ids::LEGION, (10, 10)) == HutOutcome::Barbarians {
                hordes += 1;
                // one city: neighbours 1, 4, 7 (N, SE, W) of the hut square (11,10)
                let mut barbs: Vec<(i32, i32)> = g.units_of(0).iter().map(|u| g.unit(*u).pos()).collect();
                barbs.sort();
                assert_eq!(barbs, vec![(10, 10), (11, 9), (12, 11)]);
                assert!(g.units_of(0).iter().all(|u| g.unit(*u).kind == unit_ids::CAVALRY));
            }
        }
        assert!(hordes > 0);
    }

    #[test]
    fn scrolls_only_until_1000_ad() {
        let mut g = plain_world(7);
        g.turn = 300;
        g.year = 1500;
        for _ in 0..100 {
            let ids: Vec<UnitId> = g.all_units().map(|u| u.id).collect();
            for id in ids {
                g.remove_unit(id);
            }
            let out = hut_outcome(&mut g, unit_ids::LEGION, (10, 10));
            assert!(!matches!(out, HutOutcome::Advance(_) | HutOutcome::EmptyScrolls), "{out:?}");
        }
    }

    #[test]
    fn barbarians_and_aircraft_find_nothing() {
        let mut g = plain_world(9);
        g.map.get_mut(11, 10).unwrap().set(tile_flags::HUT, true);
        let b = g.add_unit(0, unit_ids::LEGION, 10, 10);
        g.take_events();
        g.try_move(b, Dir8::E).unwrap();
        assert!(!g.take_events().iter().any(|e| matches!(e, GameEvent::HutEntered { .. })));
        assert!(!g.map.tile(11, 10).has(tile_flags::HUT));
    }

    #[test]
    fn start_turn_by_difficulty() {
        let mut g = plain_world(1);
        for (d, t) in [(Difficulty::Chieftain, 150), (Difficulty::Prince, 90), (Difficulty::Emperor, 30)] {
            g.setup.difficulty = d;
            assert_eq!(g.barbarian_start_turn(), t);
        }
    }

    #[test]
    fn sea_raid_every_eighth_turn_until_automobile() {
        let mut g = plain_world(13);
        // a big ocean everywhere else
        g.turn = 151;
        g.barbarian_spawn();
        let ships: Vec<UnitId> = g.units_of(0).into_iter().filter(|u| g.unit(*u).is_sea()).collect();
        assert_eq!(ships.len(), 1);
        let ship = g.unit(ships[0]).clone();
        assert_eq!(ship.kind, unit_ids::SAIL);
        let cargo = g.cargo_of(ship.id);
        // clamp(151 / 150 + 1, 1, 3) = 2 Legions and the leader
        assert_eq!(cargo.len(), 3);
        assert_eq!(cargo.iter().filter(|c| g.unit(**c).kind == unit_ids::LEGION).count(), 2);
        assert!(cargo.iter().any(|c| g.unit(*c).leader && g.unit(*c).kind == unit_ids::DIPLOMAT));
        assert!(cargo.iter().all(|c| g.unit(*c).order == UnitOrder::Sentry));
        // not on other turns, nor after Automobile
        let ids = g.units_of(0);
        for id in ids {
            g.remove_unit(id);
        }
        g.turn = 152;
        g.barbarian_spawn();
        assert!(g.units_of(0).is_empty());
        g.player_mut(3).learn(adv::AUTOMOBILE);
        g.turn = 159;
        g.barbarian_spawn();
        assert!(g.units_of(0).iter().all(|u| !g.unit(*u).is_sea()));
        // before the start turn nothing at all
        g.player_mut(3).advances = Default::default();
        g.turn = 143;
        g.barbarian_spawn();
        assert!(g.units_of(0).is_empty());
    }

    #[test]
    fn ship_lands_its_troops_and_announces_it() {
        let mut g = plain_world(17);
        let human = g.human_slot;
        let city = g.add_city(human, "Port".into(), 8, 6);
        for y in 0..crate::rules::MAP_H as i32 {
            for x in 0..crate::rules::MAP_W as i32 {
                g.map.get_mut(x, y).unwrap().explored |= 1 << human;
            }
        }
        // the ship one square west of the coast at (4, 10)
        let ship = g.add_barbarian(unit_ids::SAIL, 4, 10);
        let l = g.add_barbarian(unit_ids::LEGION, 4, 10);
        g.unit_mut(l).carried_by = Some(ship);
        g.unit_mut(l).order = UnitOrder::Sentry;
        g.take_events();
        assert_eq!(g.barbarian_command(ship), BarbarianCommand::Unload);
        assert_eq!(g.unit(ship).moves_left, 0);
        let ev = g.take_events();
        assert!(ev.iter().any(|e| matches!(e, GameEvent::BarbariansLanded { near_city: Some(c), .. } if *c == city)), "{ev:?}");
        // an empty ship is deleted
        g.remove_unit(l);
        assert_eq!(g.barbarian_command(ship), BarbarianCommand::Disband);
        assert!(g.unit_opt(ship).is_none());
    }

    #[test]
    fn raiders_go_ashore_after_their_ship_unloads() {
        let mut g = plain_world(23);
        let human = g.human_slot;
        g.add_city(human, "Port".into(), 9, 9);
        let ship = g.add_barbarian(unit_ids::SAIL, 4, 10);
        let mut troops = Vec::new();
        for kind in [unit_ids::LEGION, unit_ids::LEGION, unit_ids::DIPLOMAT] {
            let u = g.add_barbarian(kind, 4, 10);
            let um = g.unit_mut(u);
            um.carried_by = Some(ship);
            um.order = UnitOrder::Sentry;
            um.leader = kind == unit_ids::DIPLOMAT;
            troops.push(u);
        }
        assert!(troops.iter().all(|t| *t > ship));
        g.turn = 1;
        crate::ai::unit_ai::play_barbarians(&mut g, &mut |_| {});
        // the Legions woke up and stepped onto the shore; the leader follows them
        for t in &troops[..2] {
            let u = g.unit(*t);
            assert!(g.map.is_land(u.x, u.y), "{:?}", u.pos());
            assert!(u.carried_by.is_none());
        }
    }

    #[test]
    fn a_lone_barbarian_holds_its_city() {
        let mut g = plain_world(29);
        let c = g.add_city(0, "Camp".into(), 15, 15);
        let b = g.add_barbarian(unit_ids::LEGION, 15, 15);
        assert_eq!(g.barbarian_command(b), BarbarianCommand::Fortify);
        // with company the Legion (an attacker) moves on, the Phalanx (a defender) stays
        let p = g.add_barbarian(unit_ids::PHALANX, 15, 15);
        assert!(matches!(g.barbarian_command(b), BarbarianCommand::Step(_)));
        assert_eq!(g.barbarian_command(p), BarbarianCommand::Fortify);
        assert_eq!(g.city(c).owner, 0);
    }

    #[test]
    fn land_units_go_for_cities_and_give_up_far_away() {
        let mut g = plain_world(19);
        let human = g.human_slot;
        let c = g.add_city(human, "Target".into(), 20, 10);
        g.add_city(human, "Second".into(), 20, 20);
        let b = g.add_barbarian(unit_ids::LEGION, 19, 10);
        // next to an enemy city: straight in (direction E)
        assert_eq!(g.barbarian_command(b), BarbarianCommand::Step(Dir8::E));
        assert_eq!(g.city(c).pos(), (20, 10));
        // far away (> 8) on a turn where (unit + turn) & 3 == 0: deleted
        g.unit_mut(b).x = 6;
        g.unit_mut(b).y = 28;
        g.turn = (4 - (b as u32 & 3)) & 3;
        assert_eq!(g.barbarian_command(b), BarbarianCommand::Disband);
    }
}
