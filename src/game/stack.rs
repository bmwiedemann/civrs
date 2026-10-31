//! The per-square unit stack ring (CIV.EXE unit byte +0xa).
//!
//! CIV.EXE links each civ's units on a square into a circular list through the "next unit"
//! byte of the unit record: 0xff for a unit alone, else the next unit of the ring. A unit
//! that joins a square goes in right after the square's first unit by record (`unit_at`
//! 0x1bade), so the ring order is a matter of the square's history. Every walk over a stack
//! follows it from some unit: the defender search, the unit shown on top, the stack box, the
//! unit panel, the martial law units, the AI's stack sums and the cargo a ship takes along.

use super::map::{Map, UnitId};
use super::Game;

impl Game {
    /// `unit_at` (CIV.EXE 0x1bade): the unit of the square's owner (map layer x+160) with
    /// the lowest record there, `None` on an empty square.
    pub fn unit_at(&self, x: i32, y: i32) -> Option<UnitId> {
        let x = Map::wrap_x(x);
        if self.exe_unit_bit_lost_at(x, y) {
            return None;
        }
        let here = || self.all_units().filter(move |u| u.x == x && u.y == y);
        let occupant = Map::in_bounds(x, y).then(|| self.barbarian_memory.occupant.get(Map::idx(x, y)).copied()).flatten();
        let owner = occupant.filter(|o| here().any(|u| u.owner == *o)).or_else(|| here().next().map(|u| u.owner))?;
        here().filter(|u| u.owner == owner).min_by_key(|u| u.record).map(|u| u.id)
    }

    /// The square's first unit of `id`'s owner by record, `id` left out: the unit a unit
    /// joining the square links up with (`unit_at` called while `id` is not yet there).
    fn stack_head_for(&self, id: UnitId, x: i32, y: i32) -> Option<UnitId> {
        let owner = self.unit(id).owner;
        let x = Map::wrap_x(x);
        self.all_units().filter(|u| u.id != id && u.owner == owner && u.x == x && u.y == y).min_by_key(|u| u.record).map(|u| u.id)
    }

    /// Link `id` into the stack ring of (x, y) (`link_to_square` 0x1a68:0x13d1 with
    /// `stack_link` 0x1972:0xb5c): right after the square's first unit by record; a lone unit
    /// there and `id` point at each other. `id` must be out of any ring.
    pub(crate) fn stack_link(&mut self, id: UnitId, x: i32, y: i32) {
        let Some(head) = self.stack_head_for(id, x, y).filter(|_| !self.exe_unit_bit_lost_at(x, y)) else {
            self.unit_mut(id).next = None;
            return;
        };
        let after = self.unit(head).next.unwrap_or(head);
        self.unit_mut(id).next = Some(after);
        self.unit_mut(head).next = Some(id);
    }

    /// Take `id` out of its stack ring (`stack_unlink` 0x1972:0xbbf, via 0x1a68:0x1418): its
    /// predecessor points past it, the last other unit of a ring of two is alone again.
    pub(crate) fn stack_unlink(&mut self, id: UnitId) {
        let Some(next) = self.unit(id).next else { return };
        if self.unit_opt(next).and_then(|n| n.next) == Some(id) {
            self.unit_mut(next).next = None;
        } else {
            // the walk gives up after 32 links (0x1a334)
            let mut p = next;
            for _ in 0..32 {
                match self.unit_opt(p).and_then(|u| u.next) {
                    Some(n) if n == id => {
                        self.unit_mut(p).next = Some(next);
                        break;
                    }
                    Some(n) => p = n,
                    None => break,
                }
            }
        }
        self.unit_mut(id).next = None;
    }

    /// Put unit `id` on (x, y) as a step does (move_units 0x67a2..0x67e0): out of the old
    /// square's ring, into the new one's, then the new position.
    pub fn relocate_unit(&mut self, id: UnitId, x: i32, y: i32) {
        let x = Map::wrap_x(x);
        self.stack_unlink(id);
        self.stack_link(id, x, y);
        let u = self.unit_mut(id);
        u.x = x;
        u.y = y;
    }

    /// The first `n` units of `first`'s stack ring, `first` included, as CIV.EXE follows the
    /// links (+0xa) from it: up to a unit alone (0xff) or back at `first`. A link that leads
    /// off the square or to another civ's unit ends the walk.
    pub fn stack_ring(&self, first: UnitId, n: usize) -> Vec<UnitId> {
        let u = self.unit(first);
        let (owner, pos) = (u.owner, u.pos());
        let mut v = vec![first];
        let mut cur = first;
        while v.len() < n {
            match self.unit(cur).next.and_then(|nx| self.unit_opt(nx)) {
                Some(nu) if nu.id != first && nu.owner == owner && nu.pos() == pos && !v.contains(&nu.id) => {
                    v.push(nu.id);
                    cur = nu.id;
                }
                _ => break,
            }
        }
        v
    }

    /// The stack of `first`'s square as CIV.EXE's stack walk (`for_each_in_stack` 0x92a7)
    /// visits it from `first`: at most ten units of the ring.
    pub fn stack_from(&self, first: UnitId) -> Vec<UnitId> {
        self.stack_ring(first, 10)
    }

    /// `best_defender_at(civ, unit)` (CIV.EXE 0x8ee5): for a unit in a stack the unit of
    /// highest positive `defence_value` among the first ten of the ring from the square's
    /// first unit, the first of equals; else `id` itself.
    pub fn best_defender_at(&self, id: UnitId) -> UnitId {
        if self.unit(id).next.is_none() {
            return id;
        }
        let (x, y) = self.unit(id).pos();
        let head = self.stack_head_for(id, x, y).filter(|h| self.unit(*h).record < self.unit(id).record).unwrap_or(id);
        let mut best = (id, 0);
        for o in self.stack_from(head) {
            if let Some(d) = self.defence_value(o) {
                if d > best.1 {
                    best = (o, d);
                }
            }
        }
        best.0
    }

    /// The whole stack ring of the square, from its first unit (`unit_at` 0x1bade).
    pub fn stack_at(&self, x: i32, y: i32) -> Vec<UnitId> {
        self.unit_at(x, y).map(|f| self.stack_ring(f, 128)).unwrap_or_default()
    }

    /// Link all units into rings as if each civ had created them in record order: for saves
    /// without the links (before version 28).
    pub fn rebuild_stacks(&mut self) {
        // each one in after the first: first, last, ..., second
        let mut order: Vec<(u8, i32, i32, u8, UnitId)> = self.all_units().map(|u| (u.owner, u.x, u.y, u.record, u.id)).collect();
        order.sort_unstable();
        for u in self.units.iter_mut().flatten() {
            u.next = None;
        }
        for group in order.chunk_by(|a, b| (a.0, a.1, a.2) == (b.0, b.1, b.2)) {
            if group.len() < 2 {
                continue;
            }
            let mut ring: Vec<UnitId> = vec![group[0].4];
            ring.extend(group[1..].iter().rev().map(|g| g.4));
            for (i, id) in ring.iter().enumerate() {
                self.unit_mut(*id).next = Some(ring[(i + 1) % ring.len()]);
            }
        }
    }

    /// Whether every unit is in the ring of its square with all of its owner's units there
    /// (for tests).
    pub fn stacks_consistent(&self) -> bool {
        self.all_units().all(|u| {
            let ring = self.stack_ring(u.id, 128);
            let here = self.all_units().filter(|o| o.owner == u.owner && o.pos() == u.pos()).count();
            ring.len() == here && (here > 1 || u.next.is_none())
        })
    }

    /// Whether every unit's link leads round a ring of its owner's units on its square back to
    /// it (or is 0xff). A square may hold several rings: a unit that steps onto a square whose
    /// layer lost its unit bit starts one of its own (`Game::exe_unit_bit_lost`).
    pub fn stacks_linked(&self) -> bool {
        self.all_units().all(|u| {
            let ring = self.stack_ring(u.id, 128);
            u.next.is_none() || (ring.len() > 1 && ring.last().and_then(|l| self.unit(*l).next) == Some(u.id))
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::game::{Game, GameSetup};
    use crate::rules::{unit_ids, Terrain};

    fn flat_game() -> Game {
        let mut g = Game::new(GameSetup { seed: 5, ..Default::default() }, None);
        for t in g.map.tiles.iter_mut() {
            t.terrain = Terrain::Grassland;
            t.flags = 0;
            t.city = None;
        }
        g.cities.clear();
        let ids: Vec<_> = g.all_units().map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        g
    }

    #[test]
    fn a_square_may_hold_two_closed_rings() {
        let mut g = flat_game();
        let s = g.human_slot;
        let a = g.add_unit(s, unit_ids::MILITIA, 10, 10);
        let b = g.add_unit(s, unit_ids::MILITIA, 10, 10);
        g.stack_unlink(b);
        assert!(g.stacks_linked());
        assert!(!g.stacks_consistent());
        g.unit_mut(a).next = Some(b);
        assert!(!g.stacks_linked(), "a link that leads nowhere back");
    }

    #[test]
    fn a_unit_joins_the_ring_after_the_squares_first_unit() {
        let mut g = flat_game();
        let s = g.human_slot;
        let a = g.add_unit(s, unit_ids::MILITIA, 10, 10);
        assert_eq!(g.unit(a).next, None, "alone: 0xff");
        let b = g.add_unit(s, unit_ids::MILITIA, 10, 10);
        assert_eq!((g.unit(a).next, g.unit(b).next), (Some(b), Some(a)));
        let c = g.add_unit(s, unit_ids::MILITIA, 10, 10);
        // c goes in after a (record 0): a -> c -> b -> a
        assert_eq!(g.stack_ring(a, 10), vec![a, c, b]);
        // b leaves and comes back: a -> b -> c
        g.relocate_unit(b, 11, 10);
        assert_eq!((g.stack_ring(a, 10), g.unit(b).next), (vec![a, c], None));
        g.relocate_unit(b, 10, 10);
        assert_eq!(g.stack_ring(a, 10), vec![a, b, c]);
        assert_eq!(g.stack_ring(c, 10), vec![c, a, b]);
        g.remove_unit(a);
        assert_eq!(g.stack_ring(b, 10), vec![b, c]);
        g.remove_unit(c);
        assert_eq!(g.unit(b).next, None, "a ring of two falls apart");
        assert!(g.stacks_consistent());
    }

    #[test]
    fn the_defender_of_equals_is_the_first_in_the_ring() {
        // best_defender_stack (0x8f7e) from unit_at (0x1bade): a Militia, then two Phalanxes
        // b and c, ring a -> c -> b: c defends, though b has the lower record
        let mut g = flat_game();
        let s = g.human_slot;
        let a = g.add_unit(s, unit_ids::MILITIA, 10, 10);
        let b = g.add_unit(s, unit_ids::PHALANX, 10, 10);
        let c = g.add_unit(s, unit_ids::PHALANX, 10, 10);
        assert!(g.unit(b).record < g.unit(c).record);
        assert_eq!(g.unit_at(10, 10), Some(a));
        assert_eq!(g.best_defender(10, 10, None), Some(c));
        assert_eq!(crate::ui::tilerender::top_unit(&g, 10, 10), Some(c));
        assert_eq!(g.best_defender_at(b), c, "the stack box's default (0x8ee5)");
        // b steps off and back: a -> b -> c
        g.relocate_unit(b, 11, 10);
        g.relocate_unit(b, 10, 10);
        assert_eq!(g.best_defender(10, 10, None), Some(b));
    }
}
