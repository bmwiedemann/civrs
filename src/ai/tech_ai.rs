//! Research choice for computer players: weighted by the advance table's AI interest/lean and
//! the civilisation's traits (militaristic civs favour military advances, civilized ones the
//! arts and sciences, expansionists seafaring).

use crate::game::{Game, Slot};
use crate::rules::{self, advance_ids as adv, AdvanceId, Govt};

const MILITARY: [AdvanceId; 12] = [
    adv::BRONZE_WORKING,
    adv::IRON_WORKING,
    adv::HORSEBACK_RIDING,
    adv::THE_WHEEL,
    adv::MATHEMATICS,
    adv::FEUDALISM,
    adv::CHIVALRY,
    adv::GUNPOWDER,
    adv::METALLURGY,
    adv::CONSCRIPTION,
    adv::AUTOMOBILE,
    adv::ROBOTICS,
];

const CULTURE: [AdvanceId; 10] = [
    adv::ALPHABET,
    adv::WRITING,
    adv::LITERACY,
    adv::CURRENCY,
    adv::TRADE,
    adv::PHILOSOPHY,
    adv::MYSTICISM,
    adv::UNIVERSITY,
    adv::BANKING,
    adv::THE_REPUBLIC,
];

/// Weight of researching advance `a` for the civilisation in `slot`.
pub fn weight(g: &Game, slot: Slot, a: AdvanceId) -> i32 {
    let p = g.player(slot);
    let civ = p.civ();
    let info = &rules::ADVANCES[a as usize];
    let mut w = info.ai_interest as i32 * 2;
    // lean > 0 suits civilized civs, lean < 0 militaristic ones
    let lean = info.ai_lean as i32;
    w += lean * civ.civilized as i32 * 2;
    w -= lean * civ.aggressive as i32 * 2;
    if MILITARY.contains(&a) {
        w += 2 * civ.aggressive as i32 + 1;
    }
    if CULTURE.contains(&a) {
        w += 2 * civ.civilized as i32 + 1;
    }
    if matches!(a, adv::MAP_MAKING | adv::NAVIGATION | adv::POTTERY) && civ.expansionist > 0 {
        w += 2;
    }
    // boxed in on the home landmass with a harbour: ships are the only way to grow
    if a == adv::MAP_MAKING && g.cities_of(slot).iter().any(|c| g.map.is_coastal(g.city(*c).x, g.city(*c).y)) && crate::ai::naval_ai::home_full(g, slot) {
        w += 12;
    }
    // governments: everyone wants out of Despotism; peaceful civs want Republic/Democracy
    if a == adv::MONARCHY && p.govt == Govt::Despotism {
        w += 6;
    }
    if (a == adv::THE_REPUBLIC || a == adv::DEMOCRACY) && civ.civilized > 0 && civ.aggressive <= 0 {
        w += 3;
    }
    if a == adv::COMMUNISM && civ.aggressive > 0 {
        w += 3;
    }
    // staples
    if matches!(a, adv::BRONZE_WORKING | adv::CEREMONIAL_BURIAL | adv::ALPHABET | adv::POTTERY | adv::MASONRY) {
        w += 2;
    }
    if a == adv::FUTURE_TECH {
        w = 1;
    }
    w.max(1)
}

/// Choose the next advance to research (weighted random; deterministic for a given game state).
pub fn choose(g: &Game, slot: Slot) -> Option<AdvanceId> {
    let opts = g.researchable(slot);
    if opts.is_empty() {
        return None;
    }
    let weights: Vec<i32> = opts.iter().map(|a| weight(g, slot, *a)).collect();
    let total: i32 = weights.iter().sum();
    let mut rng = g.rng.clone();
    let mut roll = rng.below(total.max(1) as u32) as i32;
    for (a, w) in opts.iter().zip(weights.iter()) {
        if roll < *w {
            return Some(*a);
        }
        roll -= *w;
    }
    opts.last().copied()
}
