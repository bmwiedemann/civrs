//! Goody huts and barbarian activity (sea landings, uprisings, expiry).

use super::map::{Map, Slot, UnitId};
use super::{Game, GameEvent, HutOutcome};
use crate::rules::{self, advance_ids as adv, unit_ids, UnitTypeId};

/// Turns a barbarian unit survives before it gives up and disappears.
pub const BARBARIAN_LIFETIME: u32 = 20;

impl Game {
    /// Technological era of the world as a whole (any civilisation knows the advance).
    /// 0 = ancient, 1 = Gunpowder, 2 = Conscription.
    pub fn world_era(&self) -> u8 {
        let knows = |a: u8| self.players.iter().any(|p| p.in_game && p.alive && p.slot != 0 && p.knows(a));
        if knows(adv::CONSCRIPTION) {
            2
        } else if knows(adv::GUNPOWDER) {
            1
        } else {
            0
        }
    }

    /// Unit types barbarians field in the current era: (defender/infantry, attacker).
    pub fn barbarian_unit_types(&self) -> (UnitTypeId, UnitTypeId) {
        match self.world_era() {
            0 => (unit_ids::LEGION, unit_ids::LEGION),
            1 => (unit_ids::MUSKETEERS, unit_ids::CANNON),
            _ => (unit_ids::RIFLEMEN, unit_ids::ARTILLERY),
        }
    }

    /// Unit `id` has just entered a hut tile.
    pub fn enter_hut(&mut self, id: UnitId) {
        let u = self.unit(id).clone();
        let slot = u.owner;
        let roll = self.rng.below(100);
        let near_city = self.all_cities().any(|c| c.owner == slot && Map::dist((c.x, c.y), (u.x, u.y)) <= 3);
        // weights: gold 50 (25:30 / 50:15 / 100:5), advance 15, mercenaries 15, barbarians 10,
        // settlers 5 (young civilisations), else empty
        let outcome = if roll < 30 {
            HutOutcome::Gold(25)
        } else if roll < 45 {
            HutOutcome::Gold(50)
        } else if roll < 50 {
            HutOutcome::Gold(100)
        } else if roll < 65 {
            let opts = self.researchable(slot);
            match self.rng.pick(&opts).copied() {
                Some(a) if a != adv::FUTURE_TECH => HutOutcome::Advance(a),
                _ => HutOutcome::Gold(50),
            }
        } else if roll < 80 {
            let p = self.player(slot);
            let pool: Vec<u8> = [unit_ids::MILITIA, unit_ids::PHALANX, unit_ids::LEGION, unit_ids::CAVALRY, unit_ids::CHARIOT, unit_ids::KNIGHTS, unit_ids::MUSKETEERS]
                .into_iter()
                .filter(|k| {
                    let info = &rules::UNITS[*k as usize];
                    info.prereq.map(|a| p.knows(a)).unwrap_or(true) && !info.obsolete_by.map(|a| p.knows(a)).unwrap_or(false)
                })
                .collect();
            HutOutcome::Mercenaries(*self.rng.pick(&pool).unwrap_or(&unit_ids::MILITIA))
        } else if roll < 90 {
            if u.kind == unit_ids::SETTLERS || near_city || slot == 0 {
                HutOutcome::Empty
            } else {
                HutOutcome::Barbarians
            }
        } else if roll < 95 && self.cities_of(slot).len() < 3 && slot != 0 {
            HutOutcome::Settlers
        } else {
            HutOutcome::Empty
        };
        match &outcome {
            HutOutcome::Gold(g) => self.player_mut(slot).gold += g,
            HutOutcome::Advance(a) => {
                self.grant_advance(slot, *a);
                self.push_event(GameEvent::AdvanceAcquired { slot, adv: *a, source: "hut".into() });
            }
            HutOutcome::Mercenaries(k) => {
                let m = self.add_unit(slot, *k, u.x, u.y);
                self.unit_mut(m).moves_left = 0;
            }
            HutOutcome::Settlers => {
                let s = self.add_unit(slot, unit_ids::SETTLERS, u.x, u.y);
                self.unit_mut(s).moves_left = 0;
            }
            HutOutcome::Barbarians => {
                let (inf, art) = self.barbarian_unit_types();
                let mut n = 0;
                let mut spots: Vec<(i32, i32)> = Map::neighbours8(u.x, u.y).collect();
                self.rng.shuffle(&mut spots);
                for (nx, ny) in spots {
                    if n >= 3 {
                        break;
                    }
                    if self.map.is_land(nx, ny) && self.units_at(nx, ny).is_empty() && self.city_at(nx, ny).is_none() {
                        let k = if n == 0 { art } else { inf };
                        self.add_unit(0, k, nx, ny);
                        n += 1;
                    }
                }
            }
            HutOutcome::Empty => {}
        }
        self.refresh_vision(slot);
        if matches!(outcome, HutOutcome::Barbarians) {
            self.refresh_vision(0);
        }
        self.push_event(GameEvent::HutEntered { unit: id, slot, outcome });
    }

    /// First turn barbarian raids may appear at this difficulty.
    pub fn barbarian_start_turn(&self) -> u32 {
        if self.setup.difficulty == rules::Difficulty::Chieftain {
            150
        } else {
            50
        }
    }

    /// Spawn barbarian raids when due (called once per turn before the barbarians move).
    ///
    /// Sea landings: from turn 50 (Chieftain 150) with (1 + difficulty)% chance per turn, a
    /// random city of a random civilisation is targeted and `2 + difficulty` era-appropriate
    /// units plus a leader land on a coastal tile 3..6 tiles away. Large empires also suffer
    /// occasional inland uprisings.
    pub fn barbarian_spawn(&mut self) {
        self.expire_barbarians();
        if self.turn < self.barbarian_start_turn() || self.ended.is_some() {
            return;
        }
        let diff = self.setup.difficulty as u32;
        if self.rng.chance(1 + diff) {
            let civs = self.alive_slots();
            if let Some(target) = self.rng.pick(&civs).copied() {
                let cities = self.cities_of(target);
                if let Some(city) = self.rng.pick(&cities).copied() {
                    let (cx, cy) = self.city(city).pos();
                    if let Some(at) = self.find_landing_site(cx, cy, true) {
                        self.spawn_barbarian_party(at, 2 + diff as usize, true);
                        self.push_event(GameEvent::BarbariansLanded { at, near_city: Some(city), target });
                    }
                }
            }
        }
        // uprisings near large empires (at most one per turn)
        if self.turn >= 100 && self.rng.chance(1) {
            let big: Vec<Slot> = self.alive_slots().into_iter().filter(|s| self.cities_of(*s).len() >= 8).collect();
            if let Some(target) = self.rng.pick(&big).copied() {
                let cities = self.cities_of(target);
                if let Some(city) = self.rng.pick(&cities).copied() {
                    let (cx, cy) = self.city(city).pos();
                    if let Some(at) = self.find_landing_site(cx, cy, false) {
                        self.spawn_barbarian_party(at, 2 + diff as usize, false);
                        self.push_event(GameEvent::BarbarianUprising { at, target });
                    }
                }
            }
        }
    }

    /// A land tile 3..6 tiles from (cx,cy) with no units, not adjacent to any city, coastal
    /// when `coastal` is set (sea landing) and on the same landmass as the city otherwise.
    fn find_landing_site(&mut self, cx: i32, cy: i32, coastal: bool) -> Option<(i32, i32)> {
        let continent = self.map.tile(cx, cy).continent;
        let mut cands: Vec<(i32, i32)> = Vec::new();
        for y in (cy - 6)..=(cy + 6) {
            for x in (cx - 6)..=(cx + 6) {
                let d = Map::dist((x, y), (cx, cy));
                if !(3..=6).contains(&d) || !Map::in_bounds(x, y) {
                    continue;
                }
                let t = self.map.tile(x, y);
                if !t.is_land() || t.city.is_some() {
                    continue;
                }
                if coastal {
                    if !self.map.is_coastal(x, y) {
                        continue;
                    }
                } else if t.continent != continent {
                    continue;
                }
                if Map::neighbours8(x, y).any(|(nx, ny)| self.city_at(nx, ny).is_some()) {
                    continue;
                }
                if !self.units_at(x, y).is_empty() {
                    continue;
                }
                cands.push((x, y));
            }
        }
        self.rng.pick(&cands).copied()
    }

    /// Create `n` barbarian military units (plus a leader for sea landings) on a tile.
    pub fn spawn_barbarian_party(&mut self, at: (i32, i32), n: usize, with_leader: bool) -> Vec<UnitId> {
        let (inf, art) = self.barbarian_unit_types();
        let mut ids = Vec::new();
        for i in 0..n {
            let k = if i % 2 == 0 { art } else { inf };
            let id = self.add_unit(0, k, at.0, at.1);
            self.unit_mut(id).veteran = self.rng.chance(25);
            self.unit_mut(id).moves_left = 0;
            ids.push(id);
        }
        if with_leader {
            let id = self.add_unit(0, unit_ids::DIPLOMAT, at.0, at.1);
            self.unit_mut(id).leader = true;
            self.unit_mut(id).moves_left = 0;
            ids.push(id);
        }
        self.players[0].alive = true;
        self.players[0].in_game = true;
        self.refresh_vision(0);
        ids
    }

    /// Barbarian units give up after `BARBARIAN_LIFETIME` turns.
    pub fn expire_barbarians(&mut self) {
        let doomed: Vec<UnitId> = self.all_units().filter(|u| u.owner == 0 && self.turn >= u.created_turn + BARBARIAN_LIFETIME).map(|u| u.id).collect();
        if doomed.is_empty() {
            return;
        }
        for id in doomed {
            let u = self.unit(id).clone();
            self.remove_unit(id);
            self.push_event(GameEvent::UnitExpired { slot: 0, kind: u.kind, at: (u.x, u.y) });
        }
        self.refresh_vision(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{tile_flags, GameSetup, MoveOutcome};
    use crate::rules::Terrain;

    #[test]
    fn two_hundred_huts_never_panic() {
        let mut g = Game::new(GameSetup { seed: 11, num_civs: 7, ..Default::default() }, None);
        for y in 5..30 {
            for x in 5..30 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Plains;
                t.flags = 0;
            }
        }
        let ids: Vec<UnitId> = g.all_units().filter(|u| (5..30).contains(&u.x) && (5..30).contains(&u.y)).map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        let mut outcomes = [0usize; 6];
        for i in 0..200 {
            let kind = if i % 7 == 0 { unit_ids::SETTLERS } else { unit_ids::LEGION };
            // clear the arena of leftovers (barbarians, mercenaries)
            let ids: Vec<UnitId> = g.all_units().filter(|u| (5..30).contains(&u.x) && (5..30).contains(&u.y)).map(|u| u.id).collect();
            for id in ids {
                g.remove_unit(id);
            }
            g.map.get_mut(11, 10).unwrap().set(tile_flags::HUT, true);
            let u = g.add_unit(1, kind, 10, 10);
            let out = g.try_move(u, crate::game::Dir8::E).unwrap();
            assert_eq!(out, MoveOutcome::EnteredHut);
            assert!(!g.map.tile(11, 10).has(tile_flags::HUT));
            for e in g.take_events() {
                if let GameEvent::HutEntered { outcome, .. } = e {
                    let k = match outcome {
                        HutOutcome::Gold(_) => 0,
                        HutOutcome::Advance(_) => 1,
                        HutOutcome::Mercenaries(_) => 2,
                        HutOutcome::Barbarians => 3,
                        HutOutcome::Empty => 4,
                        HutOutcome::Settlers => 5,
                    };
                    outcomes[k] += 1;
                }
            }
        }
        assert_eq!(outcomes.iter().sum::<usize>(), 200);
        assert!(outcomes[0] > 50, "gold is the most common outcome: {outcomes:?}");
        assert!(outcomes[3] > 0, "barbarians appear sometimes: {outcomes:?}");
    }

    #[test]
    fn barbarians_land_after_turn_150() {
        let mut g = Game::new(GameSetup { seed: 5, num_civs: 7, ..Default::default() }, None);
        // give the human a coastal city
        let site = (0..crate::rules::MAP_H as i32)
            .flat_map(|y| (0..crate::rules::MAP_W as i32).map(move |x| (x, y)))
            .find(|&(x, y)| g.map.is_land(x, y) && g.map.is_coastal(x, y) && g.map.tile(x, y).terrain != Terrain::Arctic && x > 8 && y > 8 && x < 70 && y < 40)
            .expect("coastal land");
        let c = g.add_city(1, "Port".into(), site.0, site.1);
        g.turn = 149;
        g.barbarian_spawn();
        assert!(g.units_of(0).is_empty(), "nothing before turn 150 on Chieftain");
        g.turn = 160;
        let mut landed = false;
        for _ in 0..2000 {
            g.barbarian_spawn();
            if g.events.iter().any(|e| matches!(e, GameEvent::BarbariansLanded { near_city: Some(cc), target: 1, .. } if *cc == c)) {
                landed = true;
                break;
            }
        }
        assert!(landed, "a landing happened within 2000 rolls");
        let barbs = g.units_of(0);
        assert!(barbs.len() >= 3, "2+difficulty units plus a leader: {}", barbs.len());
        assert!(barbs.iter().any(|b| g.unit(*b).leader));
        let at = g.unit(barbs[0]).pos();
        assert!((3..=6).contains(&Map::dist(at, site)));
        assert!(g.map.is_land(at.0, at.1) && g.map.is_coastal(at.0, at.1));
        // expiry
        g.turn += BARBARIAN_LIFETIME;
        g.expire_barbarians();
        assert!(g.units_of(0).is_empty());
    }
}
