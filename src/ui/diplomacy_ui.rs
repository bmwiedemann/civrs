//! Audience with a rival leader (CIV.EXE overlay 6): the throne room of the leader's
//! government (BACKgX.PIC), advisors from GOVTgX.PIC behind the king (as many as his power
//! rank earns), the leader's bust from KINGnn.PIC and the KING.TXT texts in `leader_dialog`
//! (o 0x8892): the shared pop-up menu (`MenuDialog`) at (36, 0x8b) in the leader's font, rows
//! 8 apart, without a frame on the picture's own text box.
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
}

impl AudienceScreen {
    fn empty(ai: Slot) -> AudienceScreen {
        AudienceScreen { state: None, ai, start: None, sub_shown: false, has_items: false, anthem: false }
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
pub fn audience_dialog(ctx: &UiContext, st: &DiplomacyState) -> MenuDialog {
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
    d.with_pos(LEADER_DIALOG_X, LEADER_DIALOG_Y).with_font(leader_font(civ)).with_pitch(8)
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
        self.state = Some(st);
        self.sub_shown = false;
    }

    /// The human answered (or acknowledged) the current screen.
    fn answer(&mut self, ctx: &mut UiContext, choice: usize) -> Transition {
        let Some(st) = self.state.clone() else { return Transition::Pop(DialogResult::Done) };
        match crate::ai::diplo_ai::respond(ctx.game_mut(), &st, choice) {
            Some(n) => {
                self.enter_state(ctx, n);
                Transition::None
            }
            None => Transition::Pop(DialogResult::Done),
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
        let d = audience_dialog(ctx, &st);
        self.has_items = !d.items.is_empty();
        Transition::Push(Box::new(d))
    }

    fn resume(&mut self, result: DialogResult, ctx: &mut UiContext) -> Transition {
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
    fn king_text_lines_split_into_text_and_items() {
        let lines = vec!["'We note that your primitive civil- ".to_string(), "_'No, we do not need X.' ".into(), "_'OK, let's exchange knowledge.' ".into()];
        let (text, items) = split_items(lines);
        assert_eq!(text.len(), 1);
        assert_eq!(items[1], "_'OK, let's exchange knowledge.' ");
    }
}
