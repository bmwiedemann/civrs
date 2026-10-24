//! Research choice of the computer players and the value they put on an advance (CIV.EXE
//! segment 0xa5e: `choose_research` 0x0bb4a, `advance_ai_value` 0x0c8dd).

use crate::game::{Game, Slot};
use crate::rules::{self, AdvanceId, Difficulty};

/// `advance_ai_value` (CIV.EXE 0x0c8dd): the advance's interest byte (ds:0xb0a + 2a) plus its
/// lean byte (ds:0xb0b + 2a) times the leader's "civilized" trait (+1 civilized, -1
/// militaristic; leader record ds:0x14e2 + 0x3a * nation, word +0x34).
pub fn value(g: &Game, slot: Slot, a: AdvanceId) -> i32 {
    let info = &rules::ADVANCES[a as usize];
    info.ai_interest as i32 + info.ai_lean as i32 * g.player(slot).civ().civilized as i32
}

/// The advances `choose_research` considers: those the civ lacks with both prerequisites
/// known. The unused slots and Future Tech carry the prerequisite 0x7f, which nobody knows,
/// so Future Tech is never among them.
fn candidates(g: &Game, slot: Slot) -> Vec<AdvanceId> {
    let p = g.player(slot);
    (0..rules::advances::REAL_ADVANCE_COUNT as AdvanceId)
        .filter(|a| !p.knows(*a) && rules::ADVANCES[*a as usize].prereq.iter().all(|r| r.map(|r| p.knows(r)).unwrap_or(true)))
        .collect()
}

/// The weighted draw of `choose_research` (0x0bb5d..0x0bbfe): every candidate gets
/// random(4 * value) and the highest draw wins, the first one on ties. `skip` is left out
/// (the human's own current goal, ds:0x19be).
pub fn draw(g: &mut Game, slot: Slot, skip: Option<AdvanceId>) -> Option<AdvanceId> {
    let mut rng = g.rng.clone();
    let a = draw_by(g, slot, skip, |n| rng.below(n.max(0) as u32) as i32);
    g.rng = rng;
    a
}

/// `draw` with another random source: `random(n)` for each candidate in order.
pub fn draw_by(g: &Game, slot: Slot, skip: Option<AdvanceId>, mut random: impl FnMut(i32) -> i32) -> Option<AdvanceId> {
    let mut best: (i32, Option<AdvanceId>) = (-1, None);
    for a in candidates(g, slot) {
        let w = random(4 * value(g, slot, a));
        if w > best.0 && Some(a) != skip {
            best = (w, Some(a));
        }
    }
    best.1
}

/// The advance a computer civilisation gains when its beakers are full (`choose_research`,
/// non-human branch): computer players keep no research goal in CIV.EXE, the drawn advance is
/// granted at once. None for the barbarians, when nothing is left, and on Chieftain after
/// turn 0 half of the time when the human lacks the drawn advance (0x0bc0d).
pub fn choose(g: &mut Game, slot: Slot) -> Option<AdvanceId> {
    if slot == 0 {
        return None;
    }
    let a = draw(g, slot, None);
    let human = g.human_slot;
    if g.turn != 0 && g.setup.difficulty == Difficulty::Chieftain && !a.map(|a| g.player(human).knows(a)).unwrap_or(true) && g.rng.below(2) != 0 {
        return None;
    }
    a
}

/// The advance a computer captor takes from `victim` (`steal_advance` 0x14468): of those the
/// victim knows and the captor lacks, scanned from 71 down to 0, the first with the highest
/// value. Nothing when either side is the barbarians.
pub fn best_to_steal(g: &Game, captor: Slot, victim: Slot) -> Option<AdvanceId> {
    if captor == 0 || victim == 0 {
        return None;
    }
    let mut best: (i32, Option<AdvanceId>) = (-1, None);
    for a in stealable(g, captor, victim) {
        let v = value(g, captor, a);
        if v > best.0 {
            best = (v, Some(a));
        }
    }
    best.1
}

/// The advances `victim` knows and `captor` lacks, in `steal_advance`'s order (71 down to 0).
pub fn stealable(g: &Game, captor: Slot, victim: Slot) -> Vec<AdvanceId> {
    if captor == 0 || victim == 0 {
        return Vec::new();
    }
    (0..rules::advances::REAL_ADVANCE_COUNT as AdvanceId).rev().filter(|a| g.player(victim).knows(*a) && !g.player(captor).knows(*a)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;
    use crate::rules::advance_ids as adv;

    #[test]
    fn value_uses_the_civilized_trait() {
        let mut g = Game::new(GameSetup { seed: 1, num_civs: 7, ..Default::default() }, None);
        let s = g.alive_slots().into_iter().find(|s| *s != g.human_slot).unwrap();
        // Democracy: interest 3, lean 2 (ds:0xb0a/0xb0b + 8)
        for (civ, want) in [(1u8, 5), (6, 1), (7, 3)] {
            g.players[s as usize].civ = civ;
            assert_eq!(value(&g, s, adv::DEMOCRACY), want, "civ {civ}");
        }
        assert_eq!(rules::ADVANCES[adv::GUNPOWDER as usize].ai_interest, 8);
        assert_eq!(rules::ADVANCES[adv::GUNPOWDER as usize].ai_lean, -2);
    }

    #[test]
    fn choose_takes_a_reachable_advance_and_never_future_tech() {
        let mut g = Game::new(GameSetup { seed: 2, num_civs: 7, ..Default::default() }, None);
        g.setup.difficulty = Difficulty::King;
        g.turn = 10;
        let s = g.alive_slots().into_iter().find(|s| *s != g.human_slot).unwrap();
        for _ in 0..50 {
            let a = choose(&mut g, s).expect("something to learn");
            let info = &rules::ADVANCES[a as usize];
            assert!(!g.player(s).knows(a));
            assert!(info.prereq.iter().all(|r| r.map(|r| g.player(s).knows(r)).unwrap_or(true)));
        }
        for a in 0..67 {
            g.player_mut(s).learn(a);
        }
        assert_eq!(choose(&mut g, s), None);
        assert_eq!(choose(&mut g, 0), None);
    }

    #[test]
    fn steal_prefers_value_then_the_highest_index() {
        let mut g = Game::new(GameSetup { seed: 3, num_civs: 7, ..Default::default() }, None);
        let s: Vec<Slot> = g.alive_slots().into_iter().filter(|s| *s != g.human_slot).collect();
        let (a, b) = (s[0], s[1]);
        g.players[a as usize].civ = 7; // Gandhi: civilized 0, the values are the interests
        // without the advances the start gave them
        g.players[a as usize].advances = 0;
        g.players[b as usize].advances = 0;
        for x in [adv::ALPHABET, adv::POTTERY, adv::MONARCHY, adv::BRONZE_WORKING] {
            g.player_mut(b).learn(x);
        }
        for x in [adv::ALPHABET, adv::BRONZE_WORKING] {
            g.player_mut(a).learn(x);
        }
        // Pottery and Monarchy are both 6: Pottery (42) is found first scanning down
        assert_eq!(best_to_steal(&g, a, b), Some(adv::POTTERY));
        assert_eq!(best_to_steal(&g, 0, b), None);
        assert_eq!(best_to_steal(&g, a, 0), None);
    }
}
