//! Audiences between a computer player and the human: CIV.EXE overlay 6 `diplomacy_meeting`
//! (o 0x6375..0x7a20) with `diplomacy_open` (o 0x7a42), the greeting (o 0x7be9), the treaty
//! (o 0x7db4, 0x7e0e), and the HELPME / CRUSADE offers of `civ_turn_politics` (0x14b75).
//!
//! An audience opens when units of the two meet (`Game::civs_meet`, mode 0: the computer's
//! emissary, who must be received first) or with a Diplomat's "Meet with King" (mode 1). It
//! weighs the human's threat to the leader's cities against the leader's military advantage
//! (per continent, from the census), nuclear weapons, vendettas and the human's lead, and then
//! runs its sections in a fixed order, each only when its condition holds: EXCHANGE (repeated
//! while it applies), CANCEL, TRIBUTE, TAKECIV, a declaration of war (PROVOKE / REJECT / RID),
//! PEACE (refused: the Senate, GIVECIV, GIVECASH, GROVEL, MOBILIZE), the leader's ATTITUDE or
//! welcome, and at peace the human's THREAT menu.
//!
//! `meeting` and `respond` run that program up to the next screen (`DiplomacyState`, which
//! carries the `Meeting`); the UI answers each screen with the index of the menu line chosen
//! (0 for a screen without choices).
//!
//! Menu lines of the KING.TXT sections: EXCHANGE 0 no / 1 exchange; CANCEL 0 never / 1
//! cancel; TRIBUTE 0 laugh / 1 pay; TAKECIV 0 ignore / 1 give; PEACE 0 reject / 1 accept;
//! HELPME / CRUSADE 0 no / 1 yes; GIVECIV / GIVECASH / GROVEL 0 reject / 1 make peace;
//! MERCENARY 0 never mind / 1 pay; THREAT 0 welcome peace / 1 military proposal / 2 demand
//! tribute.

use crate::game::diplomacy::{flags::*, Await, DiplomacyState, Meeting, Proposal, Step};
use crate::game::{Game, GameEvent, Slot};
use crate::rules::{self, Govt};

/// "a demand was made" ([bp-6] = 999, o 0x6d98).
const DEMANDED: i32 = 999;

fn human(g: &Game) -> Slot {
    g.human_slot
}

/// A screen of the audience with `m`.
fn screen(g: &Game, m: &Meeting, p: Proposal) -> DiplomacyState {
    let mut st = DiplomacyState::new(m.ai, human(g), p);
    st.human_initiated = m.mode == 1;
    st
}

/// A screen with a text of its own (lines separated by `\n` as in CIV.EXE).
fn message(g: &Game, m: &Meeting, text: &str) -> DiplomacyState {
    let mut st = screen(g, m, Proposal::Message);
    st.text = text.trim_end_matches('\n').split('\n').map(str::to_string).collect();
    st
}

fn wrap(mut m: Meeting, awaiting: Await, mut st: DiplomacyState) -> Option<DiplomacyState> {
    m.awaiting = awaiting;
    st.meeting = Some(Box::new(m));
    Some(st)
}

fn name(a: u8) -> String {
    rules::ADVANCES[a as usize].name.to_string()
}

fn round50(v: i32) -> i32 {
    v / 50 * 50
}

enum Open {
    Show(Await, DiplomacyState),
    Received,
    Refused,
}

/// CIV.EXE o 0x7a42 `diplomacy_open`: the first call of an audience asks "An emissary from the
/// <civ> wishes to speak with you. Will you receive him?" when the audience is the computer's
/// (ds:0x3934 == 0), with "Check Intelligence" when the human has an embassy; then the leader
/// greets the human. Later calls only repeat the answer. Received goes on with `then`,
/// refused with `refused` (ds:0x3934 = -1 for the rest of the audience).
fn open(g: &mut Game, m: &mut Meeting, then: Step, refused: Step) -> Open {
    if m.asked {
        return if g.diplo_latch == -1 { Open::Refused } else { Open::Received };
    }
    m.asked = true;
    match g.diplo_latch {
        0 => Open::Show(Await::Emissary { then, refused }, emissary(g, m)),
        -1 => Open::Refused,
        _ => Open::Show(Await::Ack(then), greeting(g, m)),
    }
}

/// The emissary question (o 0x7a82..0x7b15), shown over the map at (100, 80).
fn emissary(g: &Game, m: &Meeting) -> DiplomacyState {
    let mut st = screen(g, m, Proposal::Emissary);
    // "An emissary from the\n<Plural> wishes to\nspeak with you.\nWill you receive him?\n"
    st.text = vec!["An emissary from the".into(), format!("{} wishes to", g.civ_name(m.ai)), "speak with you.".into(), "Will you receive him?".into()];
    st.menu = vec!["Yes.".into(), "No.".into()];
    if g.player(human(g)).has_embassy(m.ai) {
        st.menu.push("Check Intelligence".into());
    }
    st.plain = true;
    st
}

/// CIV.EXE o 0x7be9 `build_greeting`: "O most untrustworthy leader of the infidels, hear now"
/// for a leader with a vendetta, else "Greetings from", then one of four honorifics
/// (random(4)), "of the <Plural>..." and "Our words are backed with NUCLEAR WEAPONS!" for a
/// civilization with Nuclear units.
fn greeting(g: &mut Game, m: &Meeting) -> DiplomacyState {
    let h = human(g);
    let p = g.player(m.ai);
    let (title, leader, plural) = (p.title(), p.leader.clone(), p.plural().to_string());
    let mut s = String::from(if g.diplo(m.ai, h) & VENDETTA != 0 { "O most untrustworthy leader of the\ninfidels, hear now " } else { "Greetings from " });
    match g.rng.below(4) {
        0 => s += &format!("the most exalted\n{title}: {leader} "),
        1 => s += &format!("{leader}, ruler\nand {title} "),
        2 => s += &format!("our most wise\n{title}: {leader} "),
        _ => s += &format!("he who makes mortals\ntremble: {leader} "),
    }
    s += &format!("of the {plural}...\n");
    if g.nuclear_units(m.ai) != 0 {
        s += "Our words are backed\nwith NUCLEAR WEAPONS!\n";
    }
    message(g, m, &s)
}

/// Open the audience of computer player `ai` with the human at `at`: `mode` 0 for the
/// computer's emissary (contact), 1 for the human's Diplomat. Returns the first screen, or None
/// when nothing is said (the effects are applied all the same).
pub fn meeting(g: &mut Game, ai: Slot, at: (i32, i32), mode: u8) -> Option<DiplomacyState> {
    let h = human(g);
    if ai == 0 || ai == h || !g.player(ai).alive || !g.player(h).alive {
        return None;
    }
    g.diplo_latch = mode as i8;
    let m = evaluate(g, ai, at, mode);
    run(g, m)
}

/// The human's audience with `ai` (a Diplomat's "Meet with King", CIV.EXE o 0x1db8b:
/// `diplomacy_meeting(owner, x, y, 1)`).
pub fn human_audience(g: &mut Game, ai: Slot, at: (i32, i32)) -> Option<DiplomacyState> {
    meeting(g, ai, at, 1)
}

/// The leader's judgement at the start of the audience (o 0x637d..0x69c4).
fn evaluate(g: &mut Game, ai: Slot, at: (i32, i32), mode: u8) -> Meeting {
    let h = human(g);
    let (a, hu) = (ai as usize, h as usize);
    let mut m = Meeting { ai, mode, at, last: g.player(ai).last_contact, mood: 1, ..Default::default() };
    if mode == 0 {
        g.player_mut(ai).last_contact = g.turn as i32;
    }
    let census = g.census();
    let chieftain = g.setup.difficulty as u8 == 0;
    let ai_cities = g.cities_of(ai).len() as i32;
    let h_cities = g.cities_of(h).len() as i32;
    // per continent 1..14: the human's attack where the leader has cities (threat, [bp-0x2e])
    // and the leader's military advantage ([bp-6]); both with units on one: war is an option
    let thr = if ai_cities > 1 { 1 } else { 0 };
    let (mut threat, mut adv, mut want) = (0i32, 0i32, false);
    for k in 1..15 {
        let (def, att_h) = (census.defence[a][k], census.attack[hu][k]);
        if census.cities[a][k] > thr && att_h > def {
            threat += if chieftain { 4 } else { 2 } * att_h / (def + 1);
            continue;
        }
        if census.defence[hu][k] != 0 && def != 0 {
            want = true;
        }
        adv += if census.cities[hu][k] != 0 {
            if census.cities[a][k] > 1 {
                census.attack[a][k] - att_h
            } else {
                census.attack[a][k]
            }
        } else if census.cities[a][k] != 0 {
            (def - att_h) / 2
        } else {
            (census.attack[a][k] - att_h) / 4
        };
    }
    let rank_h = g.power_rank(h);
    let rank_ai = g.power_rank(ai);
    let (nukes_ai, nukes_h) = (g.nuclear_units(ai), g.nuclear_units(h));
    let (str_ai, str_h) = (g.military_strength(ai), g.military_strength(h));
    // the human leads the world (the rule of 0x3121, o 0x656a)
    let mut human_leads = rank_h == 7 && h_cities > 4 && ai_cities > 1 && nukes_h == 0 && g.turn > 200;
    let vendetta = g.diplo(ai, h) & VENDETTA != 0;
    if (human_leads || (vendetta && 3 * str_ai > str_h)) && nukes_h == 0 {
        want = true;
        adv = adv.clamp(10 * rank_ai, 9999);
        threat = 0;
    }
    if nukes_ai != 0 {
        g.raise_flag(ai, h, NUCLEAR);
        if nukes_h == 0 {
            want = true;
            adv = adv.clamp(100, 9999);
        }
    }
    if nukes_h != 0 {
        if nukes_ai == 0 {
            threat = 4;
        } else {
            adv = (nukes_ai * (adv / 2)) as i16 as i32 / nukes_h;
        }
    }
    let mut mood = 1;
    if threat != 0 {
        want = false;
        mood = 0;
    }
    // the Great Wall and the United Nations (o 0x66d1)
    if g.offered_peace(h) {
        human_leads = false;
        want = false;
        if m.last == -2 {
            m.last = -1;
        }
    }
    if vendetta {
        adv *= 2;
    } else if m.last == -2 {
        adv /= 4;
    }
    // the tribute: advantage x (difficulty + 1) / 32, at most 20, in fifties (16-bit product)
    let t = (adv * (g.setup.difficulty as i32 + 1)) as i16 as i32 / 32;
    let mut tribute = t.clamp(0, 20) * 50;
    let gold_h = g.player(h).gold;
    if tribute > gold_h && tribute <= 2 * gold_h && gold_h >= 50 {
        tribute = round50(gold_h);
    }
    // (o 0x678c: a tribute cap for a leader with nuclear weapons that has not mentioned them
    // yet never applies: the NUCLEAR bit was set above)
    if tribute == 0 || 3 * str_ai < str_h {
        want = false;
    }
    if want {
        mood = 2;
    }
    if !want && g.diplo(ai, h) & ALLIANCE != 0 {
        mood = 0;
    }
    m.want_first = want;
    // other wars of the leader (o 0x682c): busy elsewhere, it keeps the peace
    let mut busy = if vendetta { -2 } else { 0 };
    for j in 1..8u8 {
        if j == h || g.diplo(ai, j) & 3 != CONTACT {
            continue;
        }
        let sj = g.military_strength(j);
        if str_ai < 4 * sj {
            busy += 1;
        }
        if sj > str_ai {
            busy += 1;
        }
        if g.treaty(h, j) {
            m.crusade = Some(j);
        }
    }
    busy -= g.player(ai).civ().aggressive as i32;
    if str_h < str_ai {
        busy -= 1;
    }
    let no_treaty = if g.treaty(ai, h) { 0 } else { 1 };
    if busy > no_treaty && !human_leads {
        want = false;
        if g.rng.below(2) != 0 {
            tribute = 0;
        }
    }
    // a Republic or Democracy wanting war: peace unless clearly stronger, else a coup
    // (set_government(ai, Monarchy), o 0x6973)
    if g.senate_overrules(ai) && want && !vendetta {
        if 3 * str_h > 2 * str_ai && !human_leads {
            want = false;
        } else {
            set_monarchy(g, ai);
        }
    }
    // a treaty holds it back when it is weaker, or when it has a vendetta (then it means to
    // break the treaty later, `BREAK`); stronger and without one it provokes a war (o 0x6994)
    let f = g.diplo(ai, h);
    if f & PEACE != 0 && want && (str_ai < str_h || f & VENDETTA != 0) {
        want = false;
        m.held_back = true;
        mood = 0;
    }
    m.want = want;
    m.mood = mood;
    m.threat = threat;
    m.tribute = tribute;
    m.human_leads = human_leads;
    m.step = Step::Exchange;
    m
}

/// CIV.EXE 0x14df7 `set_government(ai, 2)` for a computer player: no anarchy; the human with
/// an embassy hears of it ("<Adj> government changed to Monarchy.").
fn set_monarchy(g: &mut Game, ai: Slot) {
    if g.player(ai).govt == Govt::Monarchy {
        return;
    }
    g.player_mut(ai).govt = Govt::Monarchy;
    g.player_mut(ai).anarchy_turns = 0;
    g.ai_government_rates(ai);
    g.forget_vendettas(ai);
    g.push_event(GameEvent::GovernmentChanged { slot: ai, govt: Govt::Monarchy });
}

/// The advances of the exchange (o 0x69c4..0x6a87): counts, the highest the leader can offer
/// and the human's the leader values most.
fn advances(g: &Game, m: &mut Meeting) -> (i32, i32) {
    let h = human(g);
    let (mut hc, mut ac, mut best) = (0, 0, 0);
    m.offer = None;
    m.demand = None;
    for a in 0..72u8 {
        let (kh, ka) = (g.knows_advance(h, a), g.knows_advance(m.ai, a));
        hc += kh as i32;
        ac += ka as i32;
        if ka && !kh {
            m.offer = Some(a);
        }
        if kh && !ka {
            let v = crate::ai::tech_ai::value(g, m.ai, a);
            if v >= best {
                m.demand = Some(a);
                best = v;
            }
        }
    }
    (hc, ac)
}

/// Run the audience up to its next screen.
fn run(g: &mut Game, mut m: Meeting) -> Option<DiplomacyState> {
    let h = human(g);
    let ai = m.ai;
    macro_rules! open {
        ($then:expr, $refused:expr) => {
            match open(g, &mut m, $then, $refused) {
                Open::Show(aw, st) => return wrap(m, aw, st),
                Open::Received => m.step = $then,
                Open::Refused => m.step = $refused,
            }
        };
    }
    loop {
        match m.step {
            Step::Exchange => {
                let (hc, ac) = advances(g, &mut m);
                let busy = (g.diplo(ai, h) & FOUGHT != 0 || m.want) && ac > 6;
                if !busy && hc >= ac && m.offer.is_some() {
                    open!(Step::ExchangeShow, Step::AfterExchange);
                } else {
                    m.step = Step::AfterExchange;
                }
            }
            Step::ExchangeShow => {
                m.mood = 1;
                let mut st = screen(g, &m, Proposal::Exchange);
                st.advance = m.offer;
                st.rplc1 = m.offer.map(name).unwrap_or_default();
                return wrap(m, Await::Exchange, st);
            }
            // steal_advance(ai, human) (0x14468): the leader takes the human's advance it
            // values most, "<Plural> take <advance>."
            Step::ExchangeTake => {
                m.step = Step::ExchangePick;
                if let Some(a) = crate::ai::tech_ai::best_to_steal(g, ai, h) {
                    g.transfer_advance(h, ai, a, "exchange");
                    crate::ai::city_ai::rechoose(g, ai, None);
                    let st = message(g, &m, &format!("{} take\n{}.", g.civ_name(ai), name(a)));
                    return wrap(m, Await::Ack(Step::ExchangePick), st);
                }
            }
            // steal_advance(human, ai): "Select one..." of the leader's advances
            Step::ExchangePick => {
                let list = crate::ai::tech_ai::stealable(g, h, ai);
                if list.is_empty() {
                    m.want = false;
                    m.step = Step::Exchange;
                    continue;
                }
                let mut st = message(g, &m, "Select one...");
                st.menu = list.iter().map(|a| name(*a)).collect();
                return wrap(m, Await::Pick(list), st);
            }
            Step::AfterExchange => {
                // nothing to say to a treaty partner that feels threatened (o 0x6b97)
                if !m.want && m.mode == 0 && g.treaty(ai, h) && m.threat > 2 {
                    return finish(g, m, false);
                }
                m.step = Step::Cancel;
            }
            Step::Cancel => {
                if m.crusade.is_some() {
                    open!(Step::CancelShow, Step::Tribute);
                } else {
                    m.step = Step::Tribute;
                }
            }
            Step::CancelShow => {
                m.step = Step::Tribute;
                if m.mood != 0 {
                    let mut st = screen(g, &m, Proposal::Cancel);
                    st.other = m.crusade;
                    st.rplc1 = if m.want { "demand" } else { "request" }.into();
                    st.drums = m.want;
                    return wrap(m, Await::Cancel, st);
                }
            }
            Step::Tribute => {
                if m.tribute != 0 && m.mood == 2 && g.player(h).gold > m.tribute {
                    open!(Step::TributeShow, Step::TakeCiv);
                } else {
                    m.step = Step::TakeCiv;
                }
            }
            Step::TributeShow => {
                let mut st = screen(g, &m, Proposal::Tribute);
                st.bucks = m.tribute;
                st.drums = true;
                return wrap(m, Await::Tribute, st);
            }
            Step::TakeCiv => {
                if m.mood != 0 && m.demand.is_some() && m.tribute != DEMANDED {
                    open!(Step::TakeCivShow, Step::Hostile);
                } else {
                    m.step = Step::Hostile;
                }
            }
            Step::TakeCivShow => {
                let mut st = screen(g, &m, Proposal::TakeCiv);
                st.advance = m.demand;
                st.rplc1 = m.demand.map(name).unwrap_or_default();
                st.drums = true;
                return wrap(m, Await::TakeCiv, st);
            }
            // declarations of war (o 0x6e45..0x6f9b): PROVOKE breaks a treaty after a demand,
            // REJECT follows a refused demand, RID at first contact or a Diplomat's audience
            Step::Hostile => {
                if m.want && g.treaty(ai, h) && m.tribute > 50 {
                    open!(Step::ProvokeShow, Step::ProvokeRefused);
                } else if m.want && m.tribute == DEMANDED {
                    open!(Step::RejectShow, Step::AfterHostile);
                } else if m.want && (!g.player(ai).has_contact(h) || m.mode != 0) {
                    open!(Step::RidShow, Step::RidRefused);
                } else {
                    m.step = Step::AfterHostile;
                }
            }
            Step::ProvokeShow | Step::RejectShow | Step::RidShow | Step::ProvokeRefused | Step::RidRefused => {
                let p = match m.step {
                    Step::ProvokeShow | Step::ProvokeRefused => Proposal::Provoke,
                    Step::RejectShow => Proposal::Reject,
                    _ => Proposal::Rid,
                };
                let mut st = screen(g, &m, p);
                st.drums = true;
                st.plain = matches!(m.step, Step::ProvokeRefused | Step::RidRefused);
                g.clear_diplo_flag(h, ai, PEACE);
                m.step = Step::AfterHostile;
                return wrap(m, Await::Ack(Step::AfterHostile), st);
            }
            Step::AfterHostile => {
                if m.want || (g.treaty(ai, h) && m.mode == 0) {
                    m.step = Step::Attitude;
                } else {
                    open!(Step::PeaceShow, Step::Attitude);
                }
            }
            Step::PeaceShow => {
                m.mood = 0;
                let st = screen(g, &m, Proposal::Peace);
                return wrap(m, Await::Peace, st);
            }
            Step::GiveCiv => {
                m.step = Step::GiveCash;
                if m.threat >= 4 && m.offer.is_some() && g.diplo(ai, h) & (PEACE | VENDETTA) == 0 {
                    let mut st = screen(g, &m, Proposal::GiveCiv);
                    st.advance = m.offer;
                    st.rplc1 = m.offer.map(name).unwrap_or_default();
                    return wrap(m, Await::GiveCiv, st);
                }
            }
            Step::GiveCash => {
                m.step = Step::Grovel;
                if g.diplo(ai, h) & (PEACE | VENDETTA) == 0 {
                    let cash = (2 * m.threat - 4).clamp(0, (g.player(ai).gold / 50).max(0)) * 50;
                    if cash != 0 {
                        let mut st = screen(g, &m, Proposal::GiveCash);
                        st.bucks = cash;
                        return wrap(m, Await::GiveCash, st);
                    }
                }
            }
            Step::Grovel => {
                m.step = Step::Mobilize;
                if m.threat != 0 && g.cities_of(ai).len() <= 1 && g.diplo(ai, h) & (PEACE | VENDETTA) == 0 {
                    let mut st = screen(g, &m, Proposal::Grovel);
                    st.bucks = g.player(ai).gold;
                    return wrap(m, Await::Grovel, st);
                }
            }
            Step::Mobilize => {
                m.step = Step::Attitude;
                if !g.treaty(ai, h) {
                    let mut st = screen(g, &m, Proposal::Mobilize);
                    st.drums = true;
                    return wrap(m, Await::Ack(Step::Attitude), st);
                }
            }
            // o 0x740c: a treaty partner that wants no war says how it feels
            Step::Attitude => {
                if !m.want && g.treaty(ai, h) {
                    open!(Step::AttitudeShow, Step::Threat);
                } else {
                    m.step = Step::Threat;
                }
            }
            Step::AttitudeShow => {
                let st = if m.mood != 0 && m.tribute != DEMANDED {
                    screen(g, &m, Proposal::Attitude)
                } else {
                    let p = g.player(h);
                    let text = format!(
                        "We welcome the friendship of the\n{} people and their most\nwise and munificent leader:\n{} {}.",
                        p.adjective(),
                        p.title(),
                        p.leader
                    );
                    message(g, &m, &text)
                };
                return wrap(m, Await::Ack(Step::Threat), st);
            }
            Step::Threat => {
                if g.treaty(ai, h) {
                    open!(Step::ThreatShow, Step::Finish);
                } else {
                    m.step = Step::Finish;
                }
            }
            Step::ThreatShow => {
                let mut st = screen(g, &m, Proposal::Threat);
                st.rplc1 = g.civ_name(ai).into();
                // ds:0xb276: no tribute demand within 16 turns of the last talk or from a
                // Republic or Democracy; no military proposal when the two are alone in the
                // world (the active-civ mask also has the barbarians' bit: never)
                let mut disabled = vec![false; 3];
                disabled[2] = g.turn as i32 - m.last < 16 || g.senate_overrules(h);
                let alive: u16 = 1 | g.alive_slots().iter().fold(0u16, |b, s| b | 1 << s);
                disabled[1] = alive == (1 << ai) | (1 << h);
                st.disabled = disabled;
                return wrap(m, Await::Threat, st);
            }
            Step::Finish => return finish(g, m, true),
            Step::HelpMeShow => {
                let mut st = screen(g, &m, Proposal::HelpMe);
                st.other = m.crusade;
                st.bucks = m.tribute;
                let (other, bucks) = (m.crusade.unwrap_or(0), m.tribute);
                return wrap(m, Await::HelpMe { other, bucks }, st);
            }
            Step::CrusadeShow => {
                let mut st = screen(g, &m, Proposal::Crusade);
                st.other = m.crusade;
                st.rplc1 = if g.treaty(ai, h) { "extend our" } else { "agree to a" }.into();
                let other = m.crusade.unwrap_or(0);
                return wrap(m, Await::Crusade { other }, st);
            }
            Step::Close => {
                close(g, &m);
                return None;
            }
            Step::Done => {
                g.audience_pending &= !(1 << ai);
                return None;
            }
        }
    }
}

/// The end of the audience (o 0x7957..0x7a1b; `schism` false for the early end o 0x6bc0
/// that skips the bookkeeping): a leader held back by a treaty means to break it (`BREAK`),
/// the fighting is forgotten, and with advice on the human may read the SCHISM note about a
/// large and powerful civilization ranked at least two places above him.
fn finish(g: &mut Game, m: Meeting, schism: bool) -> Option<DiplomacyState> {
    let h = human(g);
    let ai = m.ai;
    if schism {
        if m.held_back {
            g.raise_flag(ai, h, BREAK);
        }
        g.drop_flag(ai, h, FOUGHT);
        g.drop_flag(h, ai, FOUGHT);
        if g.diplo_latch != -1 && g.cities_of(ai).len() > 4 && g.power_rank(h) + 1 < g.power_rank(ai) && !g.treaty(h, ai) && g.options.instant_advice {
            g.push_event(GameEvent::Schism(ai));
        }
    }
    g.diplo_latch = -1;
    g.audience_pending &= !(1 << ai);
    None
}

/// `diplomacy_close` (o 0x7a21) after an offer outside an audience: ds:0x3934 = 0.
fn close(g: &mut Game, m: &Meeting) {
    g.diplo_latch = 0;
    g.audience_pending &= !(1 << m.ai);
}

/// Apply the human's answer to the screen `st` (the index of the menu line, 0 for a screen
/// without choices); returns the next screen of the audience, if any.
pub fn respond(g: &mut Game, st: &DiplomacyState, choice: usize) -> Option<DiplomacyState> {
    let mut m = *st.meeting.clone()?;
    let h = human(g);
    let ai = m.ai;
    if !g.player(ai).alive || !g.player(h).alive {
        return finish(g, m, false);
    }
    let yes = choice == 1;
    match std::mem::take(&mut m.awaiting) {
        Await::Nothing => return None,
        Await::Ack(next) => m.step = next,
        Await::Emissary { then, refused } => match choice {
            0 => {
                g.diplo_latch = 0;
                return wrap(m.clone(), Await::Ack(then), greeting(g, &m));
            }
            2 => {
                let st = screen(g, &m, Proposal::Intelligence);
                return wrap(m, Await::Intelligence { then, refused }, st);
            }
            _ => {
                g.diplo_latch = -1;
                m.step = refused;
            }
        },
        Await::Intelligence { then, refused } => {
            let st = emissary(g, &m);
            return wrap(m, Await::Emissary { then, refused }, st);
        }
        Await::Exchange => {
            m.step = if yes { Step::ExchangeTake } else { Step::AfterExchange };
        }
        Await::Pick(list) => {
            if let Some(a) = list.get(choice).or(list.first()).copied() {
                g.transfer_advance(ai, h, a, "exchange");
            }
            m.want = false;
            m.mood = 1;
            m.step = Step::Exchange;
        }
        Await::Cancel => {
            if yes {
                m.want = false;
                if g.rng.below(2) != 0 {
                    m.tribute = 0;
                }
                if let Some(j) = m.crusade {
                    g.clear_diplo_flag(h, j, PEACE);
                    g.raise_flag(j, h, VENDETTA);
                }
            }
            m.step = Step::Tribute;
        }
        Await::Tribute => {
            if yes {
                g.player_mut(h).gold -= m.tribute;
                g.player_mut(ai).gold += m.tribute;
                m.want = false;
            }
            m.tribute = DEMANDED;
            m.step = Step::TakeCiv;
        }
        Await::TakeCiv => {
            if yes {
                if let Some(a) = m.demand {
                    g.transfer_advance(h, ai, a, "tribute");
                }
                m.want = false;
            }
            m.tribute = DEMANDED;
            m.step = Step::Hostile;
        }
        Await::Peace => {
            if yes {
                g.sign_treaty_with_human(ai);
                return treaty(g, m, Step::Attitude);
            }
            if g.senate_overrules(h) {
                g.sign_treaty_with_human(ai);
                let st = message(g, &m, "You are overruled by the Senate.\nTreaty signed.");
                return wrap(m, Await::Ack(Step::Attitude), st);
            }
            // refusing peace in a Diplomat's audience at peace ends the treaty
            if g.treaty(ai, h) {
                g.clear_diplo_flag(h, ai, PEACE);
            }
            m.step = Step::GiveCiv;
        }
        Await::GiveCiv => {
            if yes {
                if let Some(a) = m.offer {
                    g.transfer_advance(ai, h, a, "treaty");
                }
                g.sign_treaty_with_human(ai);
                m.last = g.turn as i32;
                return treaty(g, m, Step::GiveCash);
            }
            m.step = Step::GiveCash;
        }
        Await::GiveCash => {
            if yes {
                let cash = st.bucks;
                g.sign_treaty_with_human(ai);
                g.player_mut(h).gold += cash;
                g.player_mut(ai).gold -= cash;
                m.last = g.turn as i32;
                return treaty(g, m, Step::Grovel);
            }
            m.step = Step::Grovel;
        }
        Await::Grovel => {
            if yes {
                g.sign_treaty_with_human(ai);
                let gold = g.player(ai).gold;
                g.player_mut(h).gold += gold;
                g.player_mut(ai).gold = 0;
                for a in 0..72u8 {
                    if !g.knows_advance(h, a) && g.knows_advance(ai, a) {
                        g.transfer_advance(ai, h, a, "treaty");
                    }
                }
                m.last = g.turn as i32;
                return treaty(g, m, Step::Mobilize);
            }
            m.step = Step::Mobilize;
        }
        Await::Threat => match choice {
            1 => {
                let list: Vec<Slot> = g.alive_slots().into_iter().filter(|j| *j != h && *j != ai).collect();
                let mut st = message(g, &m, "You must attack the evil:");
                st.menu = list.iter().map(|j| g.civ_name(*j).to_string()).collect();
                return wrap(m, Await::Evil(list), st);
            }
            2 => {
                // "Demand tribute for our patience"
                let cash = m.threat.clamp(0, (g.player(ai).gold / 50).max(0)) * 50;
                let st = if cash != 0 {
                    g.player_mut(h).gold += cash;
                    g.player_mut(ai).gold -= cash;
                    g.player_mut(ai).last_contact = g.turn as i32;
                    let mut st = screen(g, &m, Proposal::GiveMore);
                    st.bucks = cash;
                    st
                } else if m.want_first {
                    g.clear_diplo_flag(h, ai, PEACE);
                    let mut st = screen(g, &m, Proposal::Provoke);
                    st.drums = true;
                    st
                } else {
                    message(g, &m, "We ignore your feeble threats.")
                };
                return wrap(m, Await::Ack(Step::Finish), st);
            }
            _ => m.step = Step::Finish,
        },
        Await::Evil(list) => {
            let Some(j) = list.get(choice).copied() else {
                m.step = Step::Finish;
                return run(g, m);
            };
            let mut st;
            if !g.player(ai).has_contact(j) {
                st = screen(g, &m, Proposal::NoContact);
                st.rplc1 = g.civ_name(j).into();
                return wrap(m, Await::Ack(Step::Finish), st);
            }
            if !g.treaty(ai, j) {
                st = screen(g, &m, Proposal::AtWar);
                st.rplc1 = g.civ_name(j).into();
                return wrap(m, Await::Ack(Step::ThreatShow), st);
            }
            // their price: the leader's gold / 100 x half the enemy's strength / 50, in
            // fifties, at least 100 (16-bit product, o 0x7836)
            let product = ((g.player(ai).gold / 100) * (g.military_strength(j) / 2)) as i16 as i32;
            let cost = (product / 50).clamp(2, 9999) * 50;
            st = screen(g, &m, Proposal::Mercenary);
            st.other = Some(j);
            st.rplc1 = g.civ_name(j).into();
            st.bucks = cost;
            return wrap(m, Await::Mercenary { other: j, cost }, st);
        }
        Await::Mercenary { other, cost } => {
            if !yes {
                m.step = Step::Finish;
            } else if g.player(h).gold >= cost {
                // CIV.EXE takes the money and says so, but changes no flag: the war is never
                // declared (o 0x78eb..0x7931)
                g.player_mut(ai).gold += cost;
                g.player_mut(h).gold -= cost;
                let text = format!("{} declare war\non the {}.", g.civ_name(ai), g.civ_name(other));
                let st = message(g, &m, &text);
                return wrap(m, Await::Ack(Step::Finish), st);
            } else {
                let st = screen(g, &m, Proposal::Unfortunate);
                return wrap(m, Await::Ack(Step::Finish), st);
            }
        }
        Await::HelpMe { other, bucks } => {
            if yes {
                // both treasuries gain the sum (0x14d3f adds it to the human's and the
                // leader's gold)
                g.player_mut(h).gold += bucks;
                g.player_mut(ai).gold += bucks;
                g.set_diplo_flag(ai, h, PEACE);
                g.clear_diplo_flag(other, h, PEACE);
                g.raise_flag(other, h, VENDETTA);
            }
            m.step = Step::Close;
        }
        Await::Crusade { other } => {
            if yes {
                g.set_diplo_flag(ai, h, PEACE);
                g.clear_diplo_flag(other, h, PEACE);
                g.raise_flag(other, h, VENDETTA);
                g.player_mut(ai).last_contact = g.turn as i32 + 16;
            }
            m.step = Step::Close;
        }
    }
    run(g, m)
}

/// `show_treaty` (o 0x7db4): "We affirm this treaty of eternal friendship and goodwill between
/// the people of the <Adj> and <Adj> civilizations."
fn treaty(g: &mut Game, m: Meeting, next: Step) -> Option<DiplomacyState> {
    let h = human(g);
    let mut st = screen(g, &m, Proposal::Treaty);
    st.rplc1 = g.player(m.ai).adjective().into();
    st.us = g.player(h).adjective().into();
    wrap(m, Await::Ack(next), st)
}

/// The 32-turn offers of `civ_turn_politics` (0x14b75): for every civilization j the computer
/// player `civ` has fought while the human is at peace with j and has not fought `civ`: a
/// much weaker `civ` (twice its strength below j's) asks for help (HELPME, clamp(gold / 100,
/// 1, 10) x 50 gold), a somewhat weaker one invites the human to a crusade (CRUSADE). Each needs
/// the emissary received, which `diplomacy_open` only asks when ds:0x3934 is 0 (after an
/// earlier offer was received); it is -1 at the start and after every audience, so in v474
/// these offers never appear. Then `civ` forgets fighting j.
pub fn ask_for_help(g: &mut Game, civ: Slot) {
    let h = human(g);
    for j in 1..8u8 {
        if g.diplo(civ, j) & FOUGHT != 0 && g.treaty(h, j) && g.diplo(civ, h) & FOUGHT == 0 {
            let (sc, sj) = (g.military_strength(civ), g.military_strength(j));
            if 2 * sc < sj {
                let bucks = (g.player(civ).gold / 100).clamp(1, 10) * 50;
                offer(g, Meeting { ai: civ, at: (-1, -1), crusade: Some(j), tribute: bucks, ..Default::default() }, Step::HelpMeShow);
            }
            if sj > sc && 2 * sc >= sj {
                offer(g, Meeting { ai: civ, at: (-1, -1), crusade: Some(j), ..Default::default() }, Step::CrusadeShow);
            }
        }
        g.drop_flag(civ, j, FOUGHT);
    }
}

/// Queue an offer of `ask_for_help` behind its emissary (refused: nothing, no
/// `diplomacy_close`).
fn offer(g: &mut Game, mut m: Meeting, show: Step) {
    let ai = m.ai;
    let first = match open(g, &mut m, show, Step::Done) {
        Open::Show(aw, st) => wrap(m, aw, st),
        Open::Received => {
            m.step = show;
            run(g, m)
        }
        Open::Refused => None,
    };
    if let Some(st) = first {
        g.audience_pending |= 1 << ai;
        g.push_event(GameEvent::NeedsDecision(crate::game::Decision::Diplomacy(st)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{GameSetup, Relation};
    use crate::rules::{unit_ids, Terrain};

    /// Human (slot 1) and AI (slot 2) each with a city on a shared plain, units adjacent.
    fn meeting_game() -> (Game, Slot, Slot) {
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
        g.map.compute_continents();
        let ids: Vec<_> = g.all_units().map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        g.add_city(1, "Rome".into(), 10, 10);
        g.add_city(2, "Babylon".into(), 20, 20);
        g.add_unit(1, unit_ids::PHALANX, 15, 15);
        g.add_unit(2, unit_ids::PHALANX, 16, 15);
        assert_eq!(g.human_slot, 1);
        (g, 1, 2)
    }

    fn decisions(g: &mut Game) -> Vec<DiplomacyState> {
        g.take_events()
            .into_iter()
            .filter_map(|e| match e {
                GameEvent::NeedsDecision(crate::game::Decision::Diplomacy(st)) => Some(st),
                _ => None,
            })
            .collect()
    }

    /// Answer every screen with `pick(state)` until the audience ends; returns the proposals.
    fn play(g: &mut Game, first: DiplomacyState, pick: &dyn Fn(&DiplomacyState) -> usize) -> Vec<Proposal> {
        let mut seen = vec![first.proposal];
        let mut st = first;
        for _ in 0..40 {
            let c = pick(&st);
            match respond(g, &st, c) {
                Some(n) => {
                    seen.push(n.proposal);
                    st = n;
                }
                None => return seen,
            }
        }
        panic!("the audience does not end: {seen:?}");
    }

    #[test]
    fn first_contact_brings_the_emissary_and_peace_can_be_accepted() {
        let (mut g, human, ai) = meeting_game();
        let id = g.units_of(human)[0];
        g.look_for_contacts(id);
        assert_ne!(g.diplo(human, ai) & CONTACT, 0, "contact is made");
        let ds = decisions(&mut g);
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].proposal, Proposal::Emissary);
        assert_eq!(g.player(ai).last_contact, g.turn as i32);
        // receive him, accept whatever peace is offered
        let seen = play(&mut g, ds[0].clone(), &|st| if st.proposal == Proposal::Peace { 1 } else { 0 });
        assert_eq!(seen[1], Proposal::Message, "the greeting");
        if seen.contains(&Proposal::Peace) {
            assert!(seen.contains(&Proposal::Treaty));
            assert!(g.treaty(human, ai) && g.treaty(ai, human));
            assert_eq!(g.player(ai).last_contact, g.turn as i32 + 16);
        }
        assert_eq!(g.diplo_latch, -1);
        assert_eq!(g.audience_pending, 0);
        // within 16 turns the leader does not come again
        g.look_for_contacts(id);
        assert!(decisions(&mut g).is_empty());
    }

    #[test]
    fn a_refused_emissary_says_nothing_unless_war_is_declared() {
        let (mut g, human, ai) = meeting_game();
        let st = meeting(&mut g, ai, (15, 15), 0).expect("emissary");
        let seen = play(&mut g, st, &|_| 1);
        assert!(seen.iter().skip(1).all(|p| matches!(p, Proposal::Provoke | Proposal::Rid)), "{seen:?}");
        assert!(!g.treaty(human, ai));
    }

    #[test]
    fn refusing_peace_under_the_senate_signs_it_anyway() {
        let (mut g, human, ai) = meeting_game();
        g.player_mut(human).govt = Govt::Democracy;
        // the human is far stronger: no war wanted
        for _ in 0..12 {
            g.add_unit(human, unit_ids::KNIGHTS, 10, 10);
        }
        let st = meeting(&mut g, ai, (15, 15), 0).expect("emissary");
        let seen = play(&mut g, st, &|st| if st.proposal == Proposal::Emissary { 0 } else { 0 });
        assert!(seen.contains(&Proposal::Peace), "{seen:?}");
        assert!(g.treaty(human, ai), "overruled by the Senate");
    }

    #[test]
    fn refusing_peace_without_senate_leaves_war_and_mobilize() {
        let (mut g, human, ai) = meeting_game();
        for _ in 0..12 {
            g.add_unit(human, unit_ids::KNIGHTS, 10, 10);
        }
        let st = meeting(&mut g, ai, (15, 15), 0).expect("emissary");
        let seen = play(&mut g, st, &|_| 0);
        assert!(seen.contains(&Proposal::Peace) && seen.contains(&Proposal::Mobilize), "{seen:?}");
        assert!(!g.treaty(human, ai));
    }

    #[test]
    fn a_strong_threatened_leader_pays_for_peace() {
        let (mut g, human, ai) = meeting_game();
        // the human's army next to the leader's only city: threat; the leader is rich
        for _ in 0..8 {
            g.add_unit(human, unit_ids::KNIGHTS, 21, 20);
        }
        g.player_mut(ai).gold = 400;
        let st = meeting(&mut g, ai, (15, 15), 0).expect("emissary");
        let m = st.meeting.clone().unwrap();
        assert!(m.threat >= 4, "threat {}", m.threat);
        assert!(!m.want);
        // reject the peace, take the gold
        let seen = play(&mut g, st, &|st| if st.proposal == Proposal::GiveCash { 1 } else { 0 });
        assert!(seen.contains(&Proposal::GiveCash), "{seen:?}");
        assert!(g.treaty(human, ai));
        assert!(g.player(ai).gold < 400);
    }

    #[test]
    fn a_diplomats_audience_at_peace_offers_peace_again() {
        let (mut g, human, ai) = meeting_game();
        g.set_relation(human, ai, Relation::Peace);
        g.player_mut(ai).last_contact = -1;
        let st = meeting(&mut g, ai, (15, 15), 1).expect("greeting");
        assert_eq!(st.proposal, Proposal::Message, "a Diplomat's audience has no emissary");
        // accepting keeps the treaty and leads to the human's own menu
        let mut g2 = g.clone();
        let seen = play(&mut g2, st.clone(), &|st| if st.proposal == Proposal::Peace { 1 } else { 0 });
        assert!(seen.contains(&Proposal::Peace) && seen.contains(&Proposal::Threat), "{seen:?}");
        assert!(g2.treaty(human, ai) && g2.treaty(ai, human));
        // rejecting it ends the treaty (o 0x7085)
        let seen = play(&mut g, st, &|_| 0);
        assert!(seen.contains(&Proposal::Peace), "{seen:?}");
        assert!(!g.treaty(human, ai) && !g.treaty(ai, human));
    }

    #[test]
    fn the_threat_menu_disables_tribute_after_a_recent_talk() {
        let (mut g, human, ai) = meeting_game();
        g.set_relation(human, ai, Relation::Peace);
        g.player_mut(ai).last_contact = g.turn as i32;
        let st = meeting(&mut g, ai, (15, 15), 1).expect("greeting");
        let mut menu = None;
        let mut st = Some(st);
        while let Some(s) = st {
            if s.proposal == Proposal::Threat {
                menu = Some(s.disabled.clone());
                break;
            }
            let c = if s.proposal == Proposal::Peace { 1 } else { 0 };
            st = respond(&mut g, &s, c);
        }
        assert_eq!(menu, Some(vec![false, false, true]));
    }

    #[test]
    fn the_great_wall_keeps_the_peace() {
        let (mut g, human, ai) = meeting_game();
        let c = g.cities_of(human)[0];
        g.wonders_built[crate::rules::wonder_ids::GREAT_WALL as usize] = Some(c);
        g.raise_flag(ai, human, VENDETTA);
        for _ in 0..12 {
            g.add_unit(ai, unit_ids::KNIGHTS, 20, 20);
        }
        let st = meeting(&mut g, ai, (15, 15), 0).expect("emissary");
        assert!(!st.meeting.as_ref().unwrap().want);
    }

    #[test]
    fn exchange_swaps_one_advance_each() {
        let (mut g, human, ai) = meeting_game();
        g.set_relation(human, ai, Relation::Peace);
        // whatever the start gave them: the leader knows only Alphabet of the three
        for a in [crate::rules::advance_ids::POTTERY, crate::rules::advance_ids::BRONZE_WORKING] {
            g.player_mut(ai).forget(a);
        }
        g.player_mut(ai).learn(crate::rules::advance_ids::ALPHABET);
        g.player_mut(human).learn(crate::rules::advance_ids::POTTERY);
        g.player_mut(human).learn(crate::rules::advance_ids::BRONZE_WORKING);
        let st = meeting(&mut g, ai, (15, 15), 1).expect("greeting");
        let seen = play(&mut g, st, &|st| if st.proposal == Proposal::Exchange { 1 } else { 0 });
        assert!(seen.contains(&Proposal::Exchange), "{seen:?}");
        assert!(g.player(human).knows(crate::rules::advance_ids::ALPHABET));
        let took = [crate::rules::advance_ids::POTTERY, crate::rules::advance_ids::BRONZE_WORKING].iter().filter(|a| g.player(ai).knows(**a)).count();
        assert_eq!(took, 1, "the leader takes one");
    }

    #[test]
    fn help_offers_need_an_open_latch() {
        let (mut g, human, ai) = meeting_game();
        g.players[3].in_game = true;
        g.players[3].alive = true;
        g.set_relation(human, 3, Relation::Peace);
        g.raise_flag(ai, 3, FOUGHT);
        for _ in 0..6 {
            g.add_unit(3, unit_ids::KNIGHTS, 25, 25);
        }
        ask_for_help(&mut g, ai);
        assert!(decisions(&mut g).is_empty(), "ds:0x3934 is -1: never received");
        assert_eq!(g.diplo(ai, 3) & FOUGHT, 0);
        g.raise_flag(ai, 3, FOUGHT);
        g.diplo_latch = 0;
        ask_for_help(&mut g, ai);
        let ds = decisions(&mut g);
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].proposal, Proposal::Emissary);
    }
}
