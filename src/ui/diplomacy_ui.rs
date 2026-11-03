//! Audience with a rival leader (CIV.EXE overlay 6): the throne room of the leader's
//! government (BACKgX.PIC), advisors from GOVTgX.PIC behind the king (as many as his power
//! rank earns), the leader's bust from KINGnn.PIC and the KING.TXT texts in `leader_dialog`
//! (o 0x8892): the shared pop-up menu (`MenuDialog`) at (36, 0x8b) in the leader's font, rows
//! 8 apart, without a frame on the picture's own text box, while the leader's face moves
//! with his mood (`animate_leader` o 0x7ea8).
//!
//! The screen runs a whole audience (`diplo_ai`, CIV.EXE overlay 6): it shows each screen of
//! it, passes the answer to `diplo_ai::respond` and shows what follows, until the audience is
//! over. The emissary question and the texts of a refused emissary appear over the map
//! (`DiplomacyState::plain`), "Check Intelligence" opens the intelligence report. An audience
//! from the engine (`Decision::Diplomacy`) ends with `DialogResult::Done`, which `reply_for`
//! turns into `DecisionReply::Cancel`: the engine has nothing left to do.

use crate::assets::text::{substitute_line, wrap_text, Vars};
use crate::assets::Palette;
use crate::game::diplomacy::{Await, DiplomacyState, Proposal};
use crate::game::Slot;
use crate::gfx::{Rect, Surface};
use crate::platform::Event;
use crate::rules::advance_ids as adv;
use crate::ui::context::UiContext;
use crate::ui::govtscreen::{back_sheet, govt_sheet};
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::{MenuDialog, LEADER_DIALOG_Y};

/// KINGnn.PIC index of a civilisation's leader (ds:0x3938, by leader).
pub fn king_index(civ: u8) -> u8 {
    match civ {
        1 => 10,  // Caesar
        2 => 7,   // Hammurabi
        3 => 12,  // Frederick
        4 => 1,   // Ramesses
        5 => 4,   // Lincoln
        6 => 13,  // Alexander
        7 => 2,   // Gandhi
        9 => 8,   // Stalin
        10 => 3,  // Shaka
        11 => 11, // Napoleon
        12 => 9,  // Montezuma
        13 => 6,  // Mao Tse Tung
        14 => 0,  // Elizabeth I
        15 => 5,  // Genghis Khan
        _ => 10,
    }
}

/// Leader bust in a KING sheet: `load_leader_portrait` cuts (181, 67) 139x133 (o 0x8205) and
/// `draw_leader_background` puts it at (90, 0) (o 0x85b3).
pub const KING_BUST: Rect = Rect::new(181, 67, 139, 133);
const KING_AT: (i32, i32) = (90, 0);

/// Small portrait `i` (same numbering as KINGnn) in the SLAM2 face strip.
pub fn face_rect(i: u8) -> Rect {
    let c = (i % 7) as i32;
    let r = (i / 7) as i32;
    Rect::new(c * 28 + 1, r * 34, 27, 33)
}

/// Advisor figure `i` (0..4) of a GOVT sheet: (80 i + 1, 101), 78x99 (o 0x8167).
fn advisor_rect(i: usize) -> Rect {
    Rect::new(i as i32 * 80 + 1, 101, 78, 99)
}

/// Where the advisors stand (ds:0x3afe), their tops at y 34 (o 0x857b).
const ADVISOR_X: [i32; 4] = [41, 199, 1, 239];

/// Number of advisors behind a leader: (power rank + 1) / 2, the rank ds:0x803c being 7 for
/// the strongest civilization (o 0x85a6).
pub fn advisor_count(g: &crate::game::Game, slot: Slot) -> usize {
    ((g.power_rank(slot) + 1) / 2).clamp(0, 4) as usize
}

/// The leader's font for the whole audience: a byte per leader in the far table at segment
/// 0x2679 plus 8, 10 meaning 3 (o 0x7b84..0x7ba8; CIV.EXE font n is civrs font n - 1).
pub fn leader_font(civ: u8) -> usize {
    const STYLE: [u8; 16] = [0, 0, 1, 2, 1, 2, 0, 1, 0, 2, 0, 2, 1, 0, 2, 1];
    let f = STYLE[civ as usize & 15] + 8;
    if f == 10 {
        2
    } else {
        f as usize - 1
    }
}

/// The heads' picture group (o 0x86f4..0x8724): (government / 2) % 3, 3 more in the modern
/// era, 6 for Communism.
fn head_group(govt: crate::rules::Govt, modern: bool) -> usize {
    let g = govt.as_u8() as usize;
    if g == 3 {
        6
    } else {
        (g / 2) % 3 + if modern { 3 } else { 0 }
    }
}

/// leader_dialog (o 0x8892): popup_menu3(text, 0x14, 0x8b), the x 0x14 made 0x24.
const LEADER_DIALOG_X: i32 = 0x24;

/// How the current screen is shown.
fn over_map(st: &DiplomacyState) -> bool {
    st.plain || matches!(st.proposal, Proposal::Emissary | Proposal::Intelligence)
}

/// The text of a KING.TXT section as `load_king_text` (0x1f105) leaves it in ds:0xba06: the
/// entry's lines (load_text_entry), the variables substituted, wrapped at 80 columns; the
/// declarations of war are wrapped again at 40 (o 0x6e98, 0x6f6b). One string per line.
pub fn king_lines(ctx: &UiContext, st: &DiplomacyState) -> Vec<String> {
    let Some(g) = ctx.game.as_ref() else { return Vec::new() };
    let v = Vars { rplc1: st.rplc1.clone(), rplc2: st.rplc2.clone(), us: st.us(g), them: st.them(g), bucks: st.bucks };
    let raw = ctx.texts.king.entry_text(st.proposal.section()).unwrap_or_default();
    let mut text = wrap_text(&substitute_line(&raw, &v), 80);
    if matches!(st.proposal, Proposal::Provoke | Proposal::Reject | Proposal::Rid) {
        text = wrap_text(&text, 40);
    }
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// menu_draw's lines: one starting with ' ' or '_' is an item, the others are text.
fn split_items(lines: Vec<String>) -> (Vec<String>, Vec<String>) {
    let (items, text): (Vec<String>, Vec<String>) = lines.into_iter().partition(|l| l.starts_with([' ', '_']));
    (text, items)
}

/// What a screen sets of the leader's face: the mood ds:0x3936 (0 pleased, 1 calm, 2 angry;
/// None: -1, no animation and the font's own pitch), ds:0x6b8e (the angry or pleased face
/// variant) and the talk counter ds:0xd76c; a None field keeps the last value (the calls of
/// `leader_dialog` in o 0x6375..0x7a20 and 0x7bb1, 0x7de2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cue {
    mood: Option<Option<i8>>,
    mad: Option<bool>,
    talk: Option<i32>,
}

fn cue(st: &DiplomacyState) -> Cue {
    let c = |mood: Option<Option<i8>>, mad: Option<bool>, talk: Option<i32>| Cue { mood, mad, talk };
    let m = |v: i8| Some(Some(v));
    let want = st.meeting.as_ref().is_some_and(|m| m.want);
    match st.proposal {
        Proposal::Exchange => c(m(1), None, Some(12)),
        Proposal::Cancel => c(m(if want { 2 } else { 1 }), None, Some(12)),
        Proposal::Tribute | Proposal::TakeCiv => c(m(2), Some(false), Some(12)),
        Proposal::Provoke | Proposal::Reject | Proposal::Rid => c(m(2), Some(true), Some(12)),
        Proposal::Peace => c(m(0), Some(false), Some(12)),
        Proposal::GiveCiv | Proposal::GiveCash | Proposal::Grovel | Proposal::GiveMore => c(m(0), None, Some(12)),
        Proposal::Mobilize => c(m(2), Some(true), Some(4)),
        Proposal::Attitude => c(m(1), None, Some(4)),
        // ds:0x6b8e = 1 after the attitude (o 0x754b)
        Proposal::Threat => c(None, Some(true), None),
        Proposal::NoContact | Proposal::AtWar | Proposal::Mercenary => c(m(1), None, Some(4)),
        Proposal::Treaty => c(None, None, Some(12)),
        Proposal::Message => {
            let first = st.text.first().map(String::as_str).unwrap_or("");
            match st.meeting.as_deref().map(|m| &m.awaiting) {
                // steal_advance runs with ds:0x3936 = -1 (o 0x6b31)
                Some(Await::Pick(_)) | Some(Await::Ack(crate::game::diplomacy::Step::ExchangePick)) => c(Some(None), None, None),
                _ if first.starts_with("Greetings") || first.starts_with("O most") => c(m(1), None, Some(12)),
                _ if first.starts_with("We welcome") => c(m(0), None, Some(12)),
                _ if first.starts_with("We ignore") => c(m(1), None, Some(4)),
                _ => c(None, None, None),
            }
        }
        _ => c(None, None, None),
    }
}

/// Eyes (ds:0x3a94), mouth (ds:0x3aa6) and next step (ds:0x3ab8: 0 rest, 1 go on, -1 / -2 / -3
/// back to the rest state 0 / 6 / 14) of the 18 face states: 0 calm, 1..2 blink, 3..5 smiling,
/// 6 pleased, 7..9 back to calm, 10..11 pleased blink, 12..13 frowning, 14 angry, 15..16 back,
/// 17 angry blink.
const FACE_EYES: [i8; 18] = [8, 0, 1, 2, 2, 4, 4, 4, 2, 2, 12, 13, 9, 10, 10, 10, 9, 11];
const FACE_MOUTH: [i8; 18] = [0, 0, 0, 2, 3, 4, 5, 4, 3, 2, 5, 5, 7, 10, 11, 10, 7, 11];
const FACE_NEXT: [i8; 18] = [0, 1, -1, 1, 1, 1, 0, 1, 1, -1, 1, -2, 1, 1, 0, 1, -1, -3];

/// Where the face parts go for each KINGnn (the far table at segment 0x2679 +0x10 and +0x48,
/// o 0x8800..0x8837): the eyes at (x, y - 2), the mouth at (x, y + 18).
const FACE_AT: [(i32, i32); 14] = [(133, 31), (139, 31), (133, 35), (131, 31), (128, 22), (130, 48), (127, 34), (128, 39), (130, 28), (124, 37), (130, 24), (124, 44), (128, 24), (131, 21)];

/// The advisors' heads relative to the figure, by picture group and figure (the far table's
/// +0x80/+0x82 and +0xf0/+0xf2, o 0x8768..0x87af).
const HEAD_AT: [[(i32, i32); 4]; 7] = [
    [(32, 17), (17, 12), (15, 13), (30, 7)],
    [(29, 6), (23, 10), (26, 3), (27, 9)],
    [(26, 15), (29, 8), (24, 4), (25, 6)],
    [(25, 14), (22, 8), (30, 6), (27, 2)],
    [(23, 8), (32, 5), (21, 13), (22, 8)],
    [(26, 6), (19, 5), (33, 6), (22, 6)],
    [(19, 7), (23, 10), (25, 7), (28, 12)],
];

/// The face sprite `k` (0..13) of a KING sheet (o 0x821e..0x8279): cells of 60x50 in rows of
/// five, cells 8, 9, 12 and 13 skipped; the eyes are the top 59x20, the mouth the 59x29 below.
fn face_cell(k: i8) -> (i32, i32) {
    let k = k as i32;
    let n = k + if k >= 8 { 2 } else { 0 } + if k >= 11 { 2 } else { 0 };
    ((n % 5) * 60 + 1, (n / 5) * 50 + 1)
}

/// The leader's animation, `animate_leader` (o 0x7ea8): one step per `wait_ticks(10)` while a
/// dialog waits; the parts drawn last stay on the screen.
#[derive(Clone, Debug)]
struct Face {
    mood: Option<i8>,
    mad: bool,
    talk: i32,
    /// ds:0x67f4, ds:0x67f6.
    state: usize,
    hold: i32,
    eyes: [i8; 18],
    mouth: [i8; 18],
    /// The eyes and mouth drawn last, and the advisors' expression (ds:0x67e8).
    shown: (i8, i8),
    heads: i8,
    rng: u32,
    last_step: Option<u64>,
}

impl Face {
    fn new(seed: u64) -> Face {
        Face { mood: None, mad: false, talk: 0, state: 0, hold: 0, eyes: FACE_EYES, mouth: FACE_MOUTH, shown: (-1, -1), heads: -1, rng: seed as u32 | 1, last_step: None }
    }

    fn apply(&mut self, c: Cue) {
        if let Some(m) = c.mood {
            self.mood = m;
        }
        if let Some(b) = c.mad {
            self.mad = b;
        }
        if let Some(t) = c.talk {
            self.talk = t;
        }
    }

    /// random(n) of the animation (CIV.EXE draws it from the game's generator; civrs keeps the
    /// game's stream out of it).
    fn random(&mut self, n: u32) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng % n
    }

    fn step(&mut self) {
        let Some(mood) = self.mood else { return };
        let mut adv = -1i8;
        let (eyes, mouth);
        if self.talk != 0 && self.state == 0 {
            // talking: the mouth moves, the eyes stay (o 0x7ec4..0x7f0b)
            eyes = -1;
            let mut m = if self.random(2) == 0 { 0 } else { 9 };
            if self.talk & 3 == 0 {
                m = 8;
            }
            mouth = m;
            self.talk -= 1;
            self.hold = 4;
        } else {
            let t = FACE_NEXT[self.state];
            if t == 0 {
                // at rest: the mood picks where to go (o 0x7f1e..0x7fb5)
                let mut next = -1i32;
                match mood {
                    0 => {
                        self.eyes[6] = if self.mad { 4 } else { 2 };
                        if self.state != 6 {
                            next = 3;
                        }
                    }
                    1 => {
                        if self.state != 0 {
                            next = 0;
                        }
                    }
                    2 => {
                        let v = if self.mad { 11 } else { 7 };
                        self.mouth[14] = v;
                        self.mouth[17] = v;
                        if self.state != 14 {
                            next = 12;
                        }
                    }
                    _ => {}
                }
                if self.talk != 0 && self.state != 0 {
                    next = 0;
                }
                if next == -1 {
                    adv = match self.state {
                        14 => 1,
                        6 => 2,
                        _ => 0,
                    };
                } else {
                    if self.state == 0 {
                        self.state = next as usize;
                    }
                    if self.state == 6 || self.state == 14 {
                        self.state += 1;
                    }
                }
            } else if t < 0 {
                self.state = match t {
                    -3 => 14,
                    -2 => 6,
                    _ => 0,
                };
            } else {
                self.state += 1;
            }
            // a blink now and then (random(12), o 0x8011)
            if self.hold == 0 && self.random(12) == 0 && FACE_NEXT[self.state] == 0 {
                self.state = match self.state {
                    0 => 1,
                    6 => 10,
                    14 => 17,
                    s => s,
                };
            }
            eyes = self.eyes[self.state];
            mouth = self.mouth[self.state];
        }
        if self.hold != 0 {
            self.hold -= 1;
        }
        if adv != -1 {
            self.heads = adv;
        }
        if eyes != -1 {
            self.shown.0 = eyes;
        }
        if mouth != -1 {
            self.shown.1 = mouth;
        }
    }

    /// The steps since the last frame, one per 10 ticks (60 Hz frames).
    fn advance(&mut self, frame: u64) {
        let mut last = self.last_step.unwrap_or(frame).min(frame);
        while frame >= last + 10 {
            last += 10;
            self.step();
        }
        self.last_step = Some(last);
    }
}

pub struct AudienceScreen {
    state: Option<DiplomacyState>,
    ai: Slot,
    /// A Diplomat's audience to open on entering (`diplo_ai::human_audience`).
    start: Option<(i32, i32)>,
    /// The dialog of the current screen was pushed.
    sub_shown: bool,
    /// The dialog has menu lines: Esc answers -1, a text without them is acknowledged.
    has_items: bool,
    /// The leader's anthem is playing (the throne room is up).
    anthem: bool,
    face: Face,
    /// The Civilopedia page of an advance the human got in the audience is up (an exchange,
    /// an offer for peace): grant_advance shows it at once (0xc5d9), then the next screen.
    page_shown: bool,
    /// The audience is over once that page is read.
    end_after_page: bool,
}

impl AudienceScreen {
    fn empty(ai: Slot) -> AudienceScreen {
        AudienceScreen { state: None, ai, start: None, sub_shown: false, has_items: false, anthem: false, face: Face::new(0x2545_f491 ^ ai as u64), page_shown: false, end_after_page: false }
    }
}

/// The audience of an engine decision, from its first screen.
pub fn audience_screen(state: DiplomacyState) -> Box<dyn Screen> {
    let mut s = AudienceScreen::empty(state.ai);
    s.state = Some(state);
    Box::new(s)
}

/// The human's audience with `ai_slot` (a Diplomat's "Meet with King" next to (x, y)).
pub fn human_audience(ai_slot: u8, at: (i32, i32)) -> Box<dyn Screen> {
    let mut s = AudienceScreen::empty(ai_slot);
    s.start = Some(at);
    Box::new(s)
}

/// The pop-up of a screen of the audience: `leader_dialog` in the throne room, popup_menu
/// over the map for the emissary (100, 80, o 0x7b15) and for a refused emissary's declaration
/// (40, 100, o 0x6f83).
pub fn audience_dialog(ctx: &UiContext, st: &DiplomacyState, mood_set: bool) -> MenuDialog {
    let (text, items) = match st.proposal {
        Proposal::Message | Proposal::Emissary => (st.text.clone(), st.menu.clone()),
        _ => split_items(king_lines(ctx, st)),
    };
    let n = items.len();
    let mut d = MenuDialog::new(text, items);
    if st.disabled.iter().any(|b| *b) {
        d = d.with_disabled((0..n).map(|i| st.disabled.get(i).copied().unwrap_or(false)).collect());
    }
    if st.proposal == Proposal::Emissary {
        return d.with_pos(100, 80);
    }
    if st.plain {
        return d.with_pos(40, 100);
    }
    // "Select one..." is asked again after an Esc (0x1457b)
    if matches!(st.meeting.as_deref().map(|m| &m.awaiting), Some(Await::Pick(_))) {
        d = d.no_cancel();
    }
    let civ = ctx.game.as_ref().map(|g| g.player(st.ai).civ).unwrap_or(1);
    d = d.with_pos(LEADER_DIALOG_X, LEADER_DIALOG_Y).with_font(leader_font(civ));
    // the pitch is 8 while the leader's mood is set (o 0x1c8de)
    if mood_set {
        d = d.with_pitch(8);
    }
    d
}

impl AudienceScreen {
    /// Show `st` (music, text).
    fn enter_state(&mut self, ctx: &mut UiContext, st: DiplomacyState) {
        let civ = ctx.game().player(st.ai).civ;
        let throne = !over_map(&st);
        if st.proposal == Proposal::Emissary {
            // the anthem plays while the emissary waits (o 0x7afc, tune ds:0x1518)
            ctx.audio.national_theme(civ, false);
        } else if throne && !self.anthem {
            self.anthem = true;
            ctx.audio.national_theme(civ, true);
        }
        // CIV.EXE plays the war drums (tune 33) for demands, threats and declarations of war
        if st.drums {
            ctx.audio.play(crate::audio::Sound::WarDrums);
        }
        if throne {
            self.face.apply(cue(&st));
        }
        self.state = Some(st);
        self.sub_shown = false;
    }

    /// The human answered (or acknowledged) the current screen.
    fn answer(&mut self, ctx: &mut UiContext, choice: usize) -> Transition {
        let Some(st) = self.state.clone() else { return Transition::Pop(DialogResult::Done) };
        let next = crate::ai::diplo_ai::respond(ctx.game_mut(), &st, choice);
        // an advance the human got: its Civilopedia page comes before the next screen
        let page = take_acquired(ctx).and_then(crate::ui::dialogs::audience_advance_page);
        match next {
            Some(n) => {
                self.enter_state(ctx, n);
                match page {
                    Some(p) => {
                        self.sub_shown = true;
                        self.page_shown = true;
                        Transition::Push(p)
                    }
                    None => Transition::None,
                }
            }
            // the page is drawn over the leader's palette, so the audience stays below it
            // until it is read
            None => match page {
                Some(p) => {
                    self.page_shown = true;
                    self.end_after_page = true;
                    Transition::Push(p)
                }
                None => Transition::Pop(DialogResult::Done),
            },
        }
    }

    fn era(&self, ctx: &UiContext) -> bool {
        ctx.game.as_ref().map(|g| g.player(self.ai).knows(adv::INVENTION)).unwrap_or(false)
    }

    fn sheets(&self, ctx: &UiContext) -> (String, String, String) {
        let modern = self.era(ctx);
        let (govt, civ) = ctx.game.as_ref().map(|g| (g.player(self.ai).govt, g.player(self.ai).civ)).unwrap_or((crate::rules::Govt::Despotism, 1));
        (back_sheet(govt, modern), govt_sheet(govt, modern), format!("KING{:02}", king_index(civ)))
    }
}

/// The human's advance an answer has just granted (`transfer_advance` queues it), taken out
/// of the game's events so the map view does not show it again after the audience.
fn take_acquired(ctx: &mut UiContext) -> Option<crate::rules::AdvanceId> {
    let g = ctx.game_mut();
    let me = g.human_slot;
    let i = g.events.iter().position(|e| matches!(e, crate::game::GameEvent::AdvanceAcquired { slot, .. } if *slot == me))?;
    match g.events.remove(i) {
        Some(crate::game::GameEvent::AdvanceAcquired { adv, .. }) => Some(adv),
        _ => None,
    }
}

impl Screen for AudienceScreen {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        if let Some(at) = self.start.take() {
            if let Some(st) = crate::ai::diplo_ai::human_audience(ctx.game_mut(), self.ai, at) {
                self.enter_state(ctx, st);
            }
            return;
        }
        if let Some(st) = self.state.take() {
            self.enter_state(ctx, st);
        }
    }

    fn on_exit(&mut self, ctx: &mut UiContext) {
        ctx.audio.fade_out();
    }

    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        let Some(st) = self.state.clone() else { return Transition::Pop(DialogResult::Done) };
        if self.sub_shown {
            return Transition::None;
        }
        self.sub_shown = true;
        if st.proposal == Proposal::Intelligence {
            return Transition::Push(Box::new(crate::ui::report_advisors::Dossier::new(st.ai)));
        }
        let d = audience_dialog(ctx, &st, self.face.mood.is_some());
        self.has_items = !d.items.is_empty();
        Transition::Push(Box::new(d))
    }

    fn resume(&mut self, result: DialogResult, ctx: &mut UiContext) -> Transition {
        if std::mem::take(&mut self.page_shown) {
            if std::mem::take(&mut self.end_after_page) {
                return Transition::Pop(DialogResult::Done);
            }
            self.sub_shown = false;
            return Transition::None;
        }
        let Some(st) = self.state.as_ref() else { return Transition::Pop(DialogResult::Done) };
        let choice = match (st.proposal, result) {
            (_, DialogResult::Choice(i)) => i,
            // Esc to the emissary: "No." (CIV.EXE negates the answer into ds:0x3934, o 0x7b1d,
            // so its -1 becomes 1 and the emissary is received; civrs keeps Esc a refusal)
            (Proposal::Emissary, _) => 1,
            // Esc answers -1 to a menu; a text is acknowledged
            _ if self.has_items => usize::MAX,
            _ => 0,
        };
        self.answer(ctx, choice)
    }

    fn handle(&mut self, _ev: &Event, _ctx: &mut UiContext) -> Transition {
        Transition::None
    }

    /// draw_leader_background (o 0x8521): the throne room BACKgX, the first (rank + 1) / 2
    /// advisors of the GOVT sheet at x 41, 199, 1, 239 (y 34), advisor (i + civ) & 3 at place
    /// i, then the leader's bust at (90, 0).
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        if self.state.as_ref().map(over_map).unwrap_or(true) {
            return;
        }
        let (back, govt, king) = self.sheets(ctx);
        let bg = ctx.pic(&back);
        fb.blit(&bg.surf, 0, 0);
        let count = ctx.game.as_ref().map(|g| advisor_count(g, self.ai)).unwrap_or(1);
        let sheet = ctx.pic(&govt);
        for (i, x) in ADVISOR_X.iter().enumerate().take(count) {
            let r = advisor_rect((i + self.ai as usize) & 3);
            let key = sheet.surf.get(r.x, r.y);
            fb.blit_part(&sheet.surf, r, *x, 34, Some(key));
        }
        let kp = ctx.pic(&king);
        let key = kp.surf.get(KING_BUST.x, KING_BUST.y);
        fb.blit_part(&kp.surf, KING_BUST, KING_AT.0, KING_AT.1, Some(key));
        // animate_leader's parts over the bust and the heads (draw_leader_face o 0x86d7)
        self.face.advance(ctx.frame);
        if self.face.heads >= 0 {
            let grp = ctx.game.as_ref().map(|g| head_group(g.player(self.ai).govt, self.era(ctx))).unwrap_or(0);
            let hkey = sheet.surf.get(161, 1);
            for (i, x) in ADVISOR_X.iter().enumerate().take(count) {
                let idx = (i + self.ai as usize) & 3;
                let (dx, dy) = HEAD_AT[grp][idx];
                let r = Rect::new(161 + 29 * self.face.heads as i32, 1 + 25 * idx as i32, 28, 24);
                fb.blit_part(&sheet.surf, r, x + dx, 34 + dy, Some(hkey));
            }
        }
        let k = king_index(ctx.game.as_ref().map(|g| g.player(self.ai).civ).unwrap_or(1)) as usize;
        let (fx, fy) = FACE_AT[k.min(13)];
        if self.face.shown.0 >= 0 {
            let (cx, cy) = face_cell(self.face.shown.0);
            fb.blit_part(&kp.surf, Rect::new(cx, cy, 59, 20), fx, fy - 2, Some(key));
        }
        if self.face.shown.1 >= 0 {
            let (cx, cy) = face_cell(self.face.shown.1);
            fb.blit_part(&kp.surf, Rect::new(cx, cy + 20, 59, 29), fx, fy + 18, Some(key));
        }
    }

    /// BACKgX.PAL with the leader's colours 64..143 from KINGnn.PAL (o 0x85f3..0x8637); the
    /// pictures themselves are loaded without their palettes.
    fn palette(&self, ctx: &UiContext) -> Palette {
        if self.state.as_ref().map(over_map).unwrap_or(true) {
            return ctx.palette;
        }
        let (back, _, king) = self.sheets(ctx);
        ctx.pic_pal(&back).merged(&ctx.pic_pal(&king), 64..=143)
    }

    fn draws_parent(&self) -> bool {
        self.state.as_ref().map(over_map).unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leader_fonts_and_advisor_places() {
        // the table of segment 0x2679: Caesar 0 -> font 8, Hammurabi 1 -> 9, Frederick 2 -> 3
        assert_eq!((leader_font(1), leader_font(2), leader_font(3)), (7, 8, 2));
        assert_eq!(advisor_rect(3), Rect::new(241, 101, 78, 99));
    }

    #[test]
    fn the_face_settles_on_its_mood() {
        // animate_leader: talking first (12 steps of mouths 0 / 9 / 8), then to the rest state
        // of the mood: angry 14 with the advisors' expression 1, pleased 6 with 2
        let mut f = Face::new(7);
        f.apply(Cue { mood: Some(Some(2)), mad: Some(true), talk: Some(12) });
        for _ in 0..12 {
            f.step();
            assert!([0, 8, 9].contains(&f.shown.1));
        }
        assert_eq!(f.talk, 0);
        for _ in 0..40 {
            f.step();
        }
        assert!(f.state == 14 || f.state == 17, "{}", f.state);
        assert_eq!((f.heads, f.mouth[14]), (1, 11));
        f.apply(Cue { mood: Some(Some(0)), mad: Some(false), talk: None });
        for _ in 0..40 {
            f.step();
        }
        assert!(f.state == 6 || f.state == 10 || f.state == 11, "{}", f.state);
        assert_eq!((f.heads, f.eyes[6]), (2, 2));
        assert_eq!(face_cell(8), (1, 101));
    }

    #[test]
    fn king_text_lines_split_into_text_and_items() {
        let lines = vec!["'We note that your primitive civil- ".to_string(), "_'No, we do not need X.' ".into(), "_'OK, let's exchange knowledge.' ".into()];
        let (text, items) = split_items(lines);
        assert_eq!(text.len(), 1);
        assert_eq!(items[1], "_'OK, let's exchange knowledge.' ");
    }
}
