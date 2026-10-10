//! Diplomatic judgement of computer players: attitude, what to propose to the human, how to
//! react to the human's menu choices (KING.TXT sections), and AI-AI relations.
//!
//! Menu choice indices follow the `_` lines of the KING.TXT sections:
//! EXCHANGE 0 no / 1 exchange; CANCEL 0 never / 1 cancel; TRIBUTE 0 laugh / 1 pay;
//! TAKECIV 0 ignore / 1 give; PEACE 0 reject / 1 accept; HELPME/CRUSADE 0 no / 1 yes;
//! GIVECIV/GIVECASH/GROVEL 0 reject / 1 make peace; MERCENARY 0 never mind / 1 pay;
//! THREAT 0 welcome peace / 1 military proposal / 2 demand tribute.

use crate::game::diplomacy::{DiplomacyState, Proposal};
use crate::game::{Game, Relation, Slot};
use crate::rules::{self, Govt};

/// Turns between audiences requested by the same AI.
const AUDIENCE_INTERVAL: u32 = 8;

/// Attitude of `a` towards `b` in -100..100 (traits, treaty, embassies, memory, relative
/// power, a slowly changing random component).
pub fn attitude(g: &Game, a: Slot, b: Slot) -> i32 {
    if a == b || a == 0 || b == 0 {
        return 0;
    }
    let pa = g.player(a);
    let civ = pa.civ();
    let mut s = -20 * civ.aggressive as i32 + 5 * civ.civilized as i32;
    s += pa.attitude[b as usize] as i32;
    s += match pa.relation(b) {
        Relation::Peace => 10,
        Relation::War => -30,
        Relation::NoContact => 0,
    };
    // border friction: their military next to our cities
    let friction = g.all_units().filter(|u| u.owner == b && u.is_military() && g.all_cities().any(|c| c.owner == a && crate::game::Map::dist(c.pos(), u.pos()) <= 2)).count() as i32;
    s -= (friction * 5).min(20);
    if g.player(b).has_embassy(a) {
        s += 10;
    }
    if pa.has_embassy(b) {
        s += 5;
    }
    let ratio = g.power_ratio(a, b);
    s += if ratio < 50 {
        15
    } else if ratio < 80 {
        5
    } else if ratio > 200 {
        -15
    } else if ratio > 130 {
        -5
    } else {
        0
    };
    // slow random drift: stable for 8 turns at a time
    let h = (g.turn / 8).wrapping_mul(2654435761).wrapping_add((a as u32) << 16).wrapping_add((b as u32) << 8).wrapping_add(g.setup.seed as u32);
    let h = h ^ (h >> 13);
    s += (h % 21) as i32 - 10;
    s.clamp(-100, 100)
}

fn remember(g: &mut Game, a: Slot, b: Slot, delta: i32) {
    let att = &mut g.players[a as usize].attitude[b as usize];
    *att = (*att as i32 + delta).clamp(-100, 100) as i8;
}

fn state(g: &Game, ai: Slot, human: Slot, p: Proposal) -> DiplomacyState {
    let mut st = DiplomacyState::new(ai, human, p);
    st.rplc1 = g.civ_name(ai).to_string();
    st
}

fn with_advance(mut st: DiplomacyState, a: u8) -> DiplomacyState {
    st.advance = Some(a);
    st.rplc1 = rules::ADVANCES[a as usize].name.to_string();
    st
}

fn round10(v: i32) -> i32 {
    (v / 10 * 10).max(10)
}

/// Treaty text after peace was agreed.
fn treaty(g: &Game, ai: Slot, human: Slot) -> DiplomacyState {
    state(g, ai, human, Proposal::Treaty)
}

/// Senate rule: under Republic/Democracy the human automatically accepts AI peace offers.
fn senate_accepts_peace(g: &Game, human: Slot) -> bool {
    matches!(g.player(human).govt, Govt::Republic | Govt::Democracy)
}

/// Build the AI's opening proposal for an audience with the human (None = nothing to say).
fn propose(g: &mut Game, ai: Slot, human: Slot) -> Option<DiplomacyState> {
    let rel = g.relation(ai, human);
    let att = attitude(g, ai, human);
    let ratio = g.power_ratio(ai, human);
    let ai_gold = g.player(ai).gold;
    let human_gold = g.player(human).gold;
    match rel {
        Relation::NoContact => None,
        Relation::War => {
            if ratio < 90 || att > 30 {
                // weaker (or friendly): sue for peace, sweetened by weakness
                let mut st = if ratio < 40 && ai_gold >= 50 {
                    let mut s = state(g, ai, human, Proposal::Grovel);
                    s.bucks = ai_gold;
                    s
                } else if ratio < 70 {
                    if let Some(a) = g.advance_to_share(ai, human) {
                        with_advance(state(g, ai, human, Proposal::GiveCiv), a)
                    } else if ai_gold >= 60 {
                        let mut s = state(g, ai, human, Proposal::GiveCash);
                        s.bucks = round10(ai_gold / 3);
                        s
                    } else {
                        state(g, ai, human, Proposal::Peace)
                    }
                } else {
                    state(g, ai, human, Proposal::Peace)
                };
                if senate_accepts_peace(g, human) {
                    // the senate signs at once
                    return respond(g, &st, 1).or_else(|| Some(treaty(g, ai, human)));
                }
                st.stage = 0;
                Some(st)
            } else if att > 0 {
                Some(state(g, ai, human, Proposal::Peace))
            } else if human_gold >= 50 && g.rng.chance(60) {
                let mut st = state(g, ai, human, Proposal::Tribute);
                st.bucks = round10((human_gold / 4).clamp(50, 500));
                Some(st)
            } else if let Some(a) = g.advance_to_share(human, ai) {
                Some(with_advance(state(g, ai, human, Proposal::TakeCiv), a))
            } else if att < -50 {
                None
            } else {
                Some(state(g, ai, human, Proposal::Peace))
            }
        }
        Relation::Peace => {
            // provocation: human military next to our cities
            let provoked = g.all_units().any(|u| u.owner == human && u.is_military() && g.all_cities().any(|c| c.owner == ai && crate::game::Map::dist(c.pos(), u.pos()) <= 1));
            if att < -60 && ratio > 150 && !g.senate_blocks_war(ai, human) {
                g.declare_war(ai, human);
                return Some(state(g, ai, human, Proposal::Rid));
            }
            if att < -40 && provoked && ratio > 100 {
                g.declare_war(ai, human);
                return Some(state(g, ai, human, Proposal::Provoke));
            }
            // third parties: our enemies the human is at peace with
            let third: Vec<Slot> = g.enemies_of(ai).into_iter().filter(|o| g.relation(human, *o) == Relation::Peace).collect();
            if let Some(o) = third.first().copied() {
                if att > 20 && ai_gold >= 50 && g.rng.chance(50) {
                    let mut st = state(g, ai, human, Proposal::HelpMe);
                    st.other = Some(o);
                    st.bucks = round10(ai_gold / 4).max(50).min(ai_gold);
                    return Some(st);
                }
                if att > 0 && g.rng.chance(50) {
                    let mut st = state(g, ai, human, Proposal::Crusade);
                    st.other = Some(o);
                    st.rplc1 = "honor our".to_string();
                    return Some(st);
                }
                if att < 0 {
                    let mut st = state(g, ai, human, Proposal::Cancel);
                    st.other = Some(o);
                    st.rplc1 = if att < -30 { "demand".to_string() } else { "insist".to_string() };
                    return Some(st);
                }
            }
            // knowledge exchange
            if att > -20 {
                let offer = g.advance_to_share(ai, human);
                let want = g.advance_to_share(human, ai);
                if let (Some(a), Some(_)) = (offer, want) {
                    return Some(with_advance(state(g, ai, human, Proposal::Exchange), a));
                }
            }
            if att < -20 && ratio > 120 && human_gold >= 50 {
                let mut st = state(g, ai, human, Proposal::Tribute);
                st.bucks = round10((human_gold / 5).clamp(50, 300));
                return Some(st);
            }
            if att < 0 {
                return Some(state(g, ai, human, Proposal::Attitude));
            }
            None
        }
    }
}

/// First contact: the AI opens with a peace treaty (or a demand when it feels superior).
pub fn on_contact(g: &mut Game, ai_slot: Slot, human_slot: Slot) -> Option<DiplomacyState> {
    if ai_slot == 0 || human_slot == 0 || ai_slot == human_slot || !g.player(ai_slot).alive || !g.player(human_slot).alive {
        return None;
    }
    g.player_mut(ai_slot).last_audience[human_slot as usize] = g.turn;
    let att = attitude(g, ai_slot, human_slot);
    let ratio = g.power_ratio(ai_slot, human_slot);
    let human_gold = g.player(human_slot).gold;
    if att < -30 && ratio > 120 && human_gold >= 50 {
        let mut st = state(g, ai_slot, human_slot, Proposal::Tribute);
        st.bucks = round10((human_gold / 4).clamp(50, 300));
        return Some(st);
    }
    if senate_accepts_peace(g, human_slot) {
        return Some(treaty(g, ai_slot, human_slot));
    }
    Some(state(g, ai_slot, human_slot, Proposal::Peace))
}

/// Per-turn diplomacy for an AI civilisation: AI-AI relations every 10 turns and, at most once
/// every 8 turns with a probability driven by attitude, an audience with the human.
pub fn turn(g: &mut Game, slot: Slot) -> Option<DiplomacyState> {
    if g.turn % 10 == 0 {
        ai_relations(g, slot);
    }
    let human = g.human_slot;
    if slot == human || !g.player(human).alive || g.relation(slot, human) == Relation::NoContact {
        return None;
    }
    let last = g.player(slot).last_audience[human as usize];
    if last != 0 && g.turn < last + AUDIENCE_INTERVAL {
        return None;
    }
    let att = attitude(g, slot, human);
    let p = match g.relation(slot, human) {
        Relation::War => 25,
        _ => 8 + if att < -30 { 12 } else if att > 30 { 6 } else { 0 },
    };
    if !g.rng.chance(p) {
        return None;
    }
    let st = propose(g, slot, human)?;
    g.player_mut(slot).last_audience[human as usize] = g.turn;
    Some(st)
}

/// Relations between computer players: wars break out between hostile neighbours, tired
/// enemies make peace; attitude memories decay.
fn ai_relations(g: &mut Game, slot: Slot) {
    let others: Vec<Slot> = g.alive_slots().into_iter().filter(|o| *o > slot && *o != g.human_slot).collect();
    for o in others {
        // decay memories towards neutral
        for (a, b) in [(slot, o), (o, slot)] {
            let m = g.players[a as usize].attitude[b as usize] as i32;
            remember(g, a, b, -m.signum() * 5);
        }
        match g.relation(slot, o) {
            Relation::NoContact => {}
            Relation::Peace => {
                let ab = attitude(g, slot, o);
                let ba = attitude(g, o, slot);
                let aggressive = g.player(slot).civ().aggressive as i32;
                if ab < -25 - 10 * aggressive.min(0) && ba < 10 && !g.senate_blocks_war(slot, o) && g.rng.chance(25) {
                    g.declare_war(slot, o);
                }
            }
            Relation::War => {
                let ab = attitude(g, slot, o);
                let ba = attitude(g, o, slot);
                if ab + ba > -40 && g.rng.chance(40) {
                    g.make_peace(slot, o);
                }
            }
        }
    }
}

/// Start a human-initiated audience with an AI civilisation.
pub fn human_audience(g: &mut Game, ai_slot: Slot) -> DiplomacyState {
    let human = g.human_slot;
    let mut st = state(g, ai_slot, human, Proposal::Threat);
    st.human_initiated = true;
    if ai_slot == 0 || ai_slot == human || !g.player(ai_slot).alive || g.relation(ai_slot, human) == Relation::NoContact {
        st.proposal = Proposal::NoContact;
        return st;
    }
    if g.relation(ai_slot, human) == Relation::War {
        let att = attitude(g, ai_slot, human);
        if att < -50 && g.power_ratio(ai_slot, human) > 150 && g.united_nations_owner() != Some(human) {
            st.proposal = Proposal::AtWar;
            return st;
        }
    }
    st
}

/// Apply the human's menu choice to the audience; returns the AI's follow-up, if any.
pub fn respond(g: &mut Game, st: &DiplomacyState, choice: usize) -> Option<DiplomacyState> {
    let (ai, human) = (st.ai, st.human);
    if ai == 0 || human == 0 || ai == human {
        return None;
    }
    let att = attitude(g, ai, human);
    let at_peace = g.relation(ai, human) == Relation::Peace;
    match st.proposal {
        Proposal::Exchange => {
            if choice == 1 {
                if let Some(a) = st.advance {
                    g.transfer_advance(ai, human, a, "exchange");
                }
                if let Some(b) = g.advance_to_share(human, ai) {
                    g.transfer_advance(human, ai, b, "exchange");
                }
                remember(g, ai, human, 5);
            } else {
                remember(g, ai, human, -5);
            }
            None
        }
        Proposal::Cancel => {
            if choice == 1 {
                if let Some(o) = st.other {
                    if !g.senate_blocks_war(human, o) {
                        g.declare_war(human, o);
                        remember(g, ai, human, 10);
                    }
                }
                None
            } else {
                remember(g, ai, human, -10);
                if at_peace && att < -40 && g.rng.chance(50) && !g.senate_blocks_war(ai, human) {
                    g.declare_war(ai, human);
                    Some(state(g, ai, human, Proposal::Reject))
                } else {
                    None
                }
            }
        }
        Proposal::Tribute | Proposal::TakeCiv => {
            if choice == 1 {
                let paid = if st.proposal == Proposal::Tribute {
                    if g.player(human).gold >= st.bucks {
                        g.transfer_gold(human, ai, st.bucks);
                        true
                    } else {
                        false
                    }
                } else if let Some(a) = st.advance {
                    g.transfer_advance(human, ai, a, "tribute");
                    true
                } else {
                    false
                };
                if paid {
                    remember(g, ai, human, 10);
                    if !at_peace && att > -20 {
                        g.make_peace(ai, human);
                        return Some(treaty(g, ai, human));
                    }
                    None
                } else {
                    if at_peace && g.rng.chance(50) && !g.senate_blocks_war(ai, human) {
                        g.declare_war(ai, human);
                    }
                    Some(state(g, ai, human, Proposal::Unfortunate))
                }
            } else {
                remember(g, ai, human, -10);
                if at_peace {
                    let war_chance = if att < -30 { 80 } else { 50 };
                    if g.rng.chance(war_chance) && !g.senate_blocks_war(ai, human) {
                        g.declare_war(ai, human);
                        Some(state(g, ai, human, Proposal::Reject))
                    } else {
                        Some(state(g, ai, human, Proposal::Attitude))
                    }
                } else {
                    None
                }
            }
        }
        Proposal::Peace => {
            if choice == 1 {
                g.make_peace(ai, human);
                remember(g, ai, human, 10);
                Some(treaty(g, ai, human))
            } else {
                remember(g, ai, human, -15);
                if at_peace {
                    // first-contact treaty refused: no peace, so war
                    g.declare_war(ai, human);
                }
                Some(state(g, ai, human, Proposal::Mobilize))
            }
        }
        Proposal::GiveCiv | Proposal::GiveCash | Proposal::Grovel => {
            if choice == 1 {
                match st.proposal {
                    Proposal::GiveCiv => {
                        if let Some(a) = st.advance {
                            g.transfer_advance(ai, human, a, "treaty");
                        }
                    }
                    Proposal::GiveCash => {
                        g.transfer_gold(ai, human, st.bucks);
                    }
                    _ => {
                        let all = g.player(ai).gold;
                        g.transfer_gold(ai, human, all);
                        let known: Vec<u8> = g.player(ai).known_advances().into_iter().filter(|a| !g.player(human).knows(*a)).collect();
                        for a in known {
                            g.transfer_advance(ai, human, a, "treaty");
                        }
                    }
                }
                g.make_peace(ai, human);
                remember(g, ai, human, 10);
                Some(treaty(g, ai, human))
            } else {
                remember(g, ai, human, -10);
                None
            }
        }
        Proposal::HelpMe | Proposal::Crusade => {
            if choice == 1 {
                if let Some(o) = st.other {
                    if g.senate_blocks_war(human, o) {
                        return None;
                    }
                    if st.proposal == Proposal::HelpMe {
                        g.transfer_gold(ai, human, st.bucks);
                    }
                    g.declare_war(human, o);
                    remember(g, ai, human, 15);
                }
            } else {
                remember(g, ai, human, -5);
            }
            None
        }
        Proposal::Mercenary => {
            if choice == 1 {
                if g.player(human).gold < st.bucks {
                    return Some(state(g, ai, human, Proposal::Unfortunate));
                }
                if let Some(o) = st.other {
                    g.transfer_gold(human, ai, st.bucks);
                    g.declare_war(ai, o);
                    remember(g, ai, human, 10);
                }
            }
            None
        }
        Proposal::Threat => respond_threat(g, st, choice, att),
        _ => None,
    }
}

/// The human's own audience menu: welcome peace / military proposal / demand tribute.
fn respond_threat(g: &mut Game, st: &DiplomacyState, choice: usize, att: i32) -> Option<DiplomacyState> {
    let (ai, human) = (st.ai, st.human);
    let at_war = g.relation(ai, human) == Relation::War;
    let ratio = g.power_ratio(ai, human);
    match choice {
        0 => {
            if at_war {
                let un = g.united_nations_owner() == Some(human);
                if un || ratio < 100 || att > 0 {
                    g.make_peace(ai, human);
                    remember(g, ai, human, 10);
                    Some(treaty(g, ai, human))
                } else {
                    Some(state(g, ai, human, Proposal::Mobilize))
                }
            } else if att >= 0 {
                Some(treaty(g, ai, human))
            } else {
                Some(state(g, ai, human, Proposal::Attitude))
            }
        }
        1 => {
            // our enemies they are at peace with: offer to fight them for money
            let enemies: Vec<Slot> = g.enemies_of(human).into_iter().filter(|o| *o != ai && g.relation(ai, *o) == Relation::Peace).collect();
            if let Some(o) = enemies.first().copied() {
                if att > -20 {
                    let mut s = state(g, ai, human, Proposal::Mercenary);
                    s.other = Some(o);
                    s.rplc1 = g.civ_name(o).to_string();
                    s.bucks = round10(50 + g.power(o).max(10) * 2);
                    return Some(s);
                }
            }
            // their enemies we are at peace with: invite us to a crusade
            let theirs: Vec<Slot> = g.enemies_of(ai).into_iter().filter(|o| *o != human && g.relation(human, *o) == Relation::Peace).collect();
            if let Some(o) = theirs.first().copied() {
                let mut s = state(g, ai, human, Proposal::Crusade);
                s.other = Some(o);
                s.rplc1 = "honor our".to_string();
                return Some(s);
            }
            Some(state(g, ai, human, Proposal::Attitude))
        }
        _ => {
            let ai_gold = g.player(ai).gold;
            if ratio < 70 && ai_gold >= 40 && att > -40 {
                let mut s = state(g, ai, human, Proposal::GiveMore);
                s.bucks = round10(ai_gold / 4).min(ai_gold);
                g.transfer_gold(ai, human, s.bucks);
                remember(g, ai, human, -5);
                return Some(s);
            }
            remember(g, ai, human, -15);
            if !at_war {
                if (att < 0 || g.rng.chance(50)) && !g.senate_blocks_war(ai, human) {
                    g.declare_war(ai, human);
                    return Some(state(g, ai, human, Proposal::Reject));
                }
                return Some(state(g, ai, human, Proposal::Attitude));
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{GameEvent, GameSetup};
    use crate::rules::{unit_ids, Terrain};

    /// Human (slot 1) and AI (slot 2) each with a city on a shared plain, units adjacent.
    fn meeting() -> (Game, Slot, Slot) {
        let mut g = Game::new(GameSetup { seed: 21, num_civs: 3, ..Default::default() }, None);
        g.players[2].in_game = true;
        g.players[2].alive = true;
        for y in 5..30 {
            for x in 5..30 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Grassland;
                t.flags = 0;
            }
        }
        let ids: Vec<_> = g.all_units().map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        g.add_city(1, "Rome".into(), 10, 10);
        g.add_city(2, "Babylon".into(), 20, 20);
        g.add_unit(1, unit_ids::PHALANX, 15, 15);
        g.add_unit(2, unit_ids::PHALANX, 16, 15);
        (g, 1, 2)
    }

    #[test]
    fn contact_opens_an_audience_and_peace_can_be_accepted_or_refused() {
        let (mut g, human, ai) = meeting();
        assert_eq!(g.relation(human, ai), Relation::NoContact);
        let pairs = g.update_contacts();
        assert_eq!(pairs, vec![(1, 2)]);
        assert_eq!(g.relation(human, ai), Relation::Peace);
        assert!(g.events.iter().any(|e| matches!(e, GameEvent::ContactMade { a: 1, b: 2 })));
        assert!(g.update_contacts().is_empty(), "contact is made once");
        let st = on_contact(&mut g, ai, human).expect("audience");
        assert!(matches!(st.proposal, Proposal::Peace | Proposal::Tribute));
        assert_eq!(st.choices(), 2);
        // accept -> treaty text, still at peace
        let mut g2 = g.clone();
        let follow = respond(&mut g2, &st, 1);
        if st.proposal == Proposal::Peace {
            assert_eq!(follow.map(|f| f.proposal), Some(Proposal::Treaty));
        }
        assert_eq!(g2.relation(human, ai), Relation::Peace);
        // refuse a peace treaty -> war
        let st_peace = DiplomacyState::new(ai, human, Proposal::Peace);
        let follow = respond(&mut g, &st_peace, 0).expect("follow-up");
        assert_eq!(follow.proposal, Proposal::Mobilize);
        assert_eq!(g.relation(human, ai), Relation::War);
        assert!(g.events.iter().any(|e| matches!(e, GameEvent::WarDeclared { a: 2, b: 1 })));
    }

    #[test]
    fn exchange_swaps_knowledge_and_tribute_moves_gold() {
        let (mut g, human, ai) = meeting();
        g.update_contacts();
        g.player_mut(ai).learn(crate::rules::advance_ids::ALPHABET);
        g.player_mut(human).learn(crate::rules::advance_ids::POTTERY);
        let mut st = DiplomacyState::new(ai, human, Proposal::Exchange);
        st.advance = Some(crate::rules::advance_ids::ALPHABET);
        let ai_before = g.player(ai).advance_count();
        assert!(respond(&mut g, &st, 1).is_none());
        assert!(g.player(human).knows(crate::rules::advance_ids::ALPHABET));
        assert_eq!(g.player(ai).advance_count(), ai_before + 1, "the AI takes one advance in return");
        // tribute paid
        g.player_mut(human).gold = 200;
        let mut t = DiplomacyState::new(ai, human, Proposal::Tribute);
        t.bucks = 50;
        let ai_gold = g.player(ai).gold;
        respond(&mut g, &t, 1);
        assert_eq!(g.player(human).gold, 150);
        assert_eq!(g.player(ai).gold, ai_gold + 50);
        assert_eq!(g.relation(human, ai), Relation::Peace);
        // tribute that cannot be paid
        t.bucks = 1000;
        let f = respond(&mut g, &t, 1).expect("unfortunate");
        assert_eq!(f.proposal, Proposal::Unfortunate);
    }

    #[test]
    fn human_audience_menu_and_weak_ai_pays_when_threatened() {
        let (mut g, human, ai) = meeting();
        let st = human_audience(&mut g, ai);
        assert_eq!(st.proposal, Proposal::NoContact);
        g.update_contacts();
        let st = human_audience(&mut g, ai);
        assert_eq!(st.proposal, Proposal::Threat);
        assert!(st.human_initiated);
        assert_eq!(st.choices(), 3);
        // make the human overwhelmingly strong and the AI rich
        for _ in 0..12 {
            g.add_unit(human, unit_ids::KNIGHTS, 10, 10);
        }
        g.player_mut(ai).gold = 400;
        assert!(g.power_ratio(ai, human) < 70);
        let f = respond(&mut g, &st, 2).expect("reply");
        assert_eq!(f.proposal, Proposal::GiveMore);
        assert!(f.bucks > 0);
        assert_eq!(g.player(ai).gold, 400 - f.bucks);
        // welcoming peace while at peace reaffirms or grumbles, never declares war
        let f = respond(&mut g, &st, 0).expect("reply");
        assert!(matches!(f.proposal, Proposal::Treaty | Proposal::Attitude));
        assert_eq!(g.relation(human, ai), Relation::Peace);
    }

    #[test]
    fn senate_signs_peace_automatically_and_blocks_wars() {
        let (mut g, human, ai) = meeting();
        g.update_contacts();
        g.declare_war(ai, human);
        g.player_mut(human).govt = Govt::Democracy;
        // the AI is far weaker: it sues for peace, which the senate accepts at once
        for _ in 0..12 {
            g.add_unit(human, unit_ids::KNIGHTS, 10, 10);
        }
        let st = propose(&mut g, ai, human).expect("proposal");
        assert_eq!(st.proposal, Proposal::Treaty);
        assert_eq!(g.relation(human, ai), Relation::Peace);
        // a Democracy cannot cancel a treaty
        g.players[3].in_game = true;
        g.players[3].alive = true;
        g.make_contact(human, 3);
        let mut c = DiplomacyState::new(ai, human, Proposal::Cancel);
        c.other = Some(3);
        respond(&mut g, &c, 1);
        assert_eq!(g.relation(human, 3), Relation::Peace);
    }

    #[test]
    fn attitude_is_bounded_and_reflects_war() {
        let (mut g, human, ai) = meeting();
        g.update_contacts();
        let peace = attitude(&g, ai, human);
        g.declare_war(ai, human);
        let war = attitude(&g, ai, human);
        assert!(war < peace);
        assert!((-100..=100).contains(&war) && (-100..=100).contains(&peace));
    }
}
