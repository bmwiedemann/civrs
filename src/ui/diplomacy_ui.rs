//! Audience with a rival leader: throne room of the leader's government (BACKgX.PIC),
//! advisors from GOVTgX.PIC behind the king (count = size rank), the leader's bust from
//! KINGnn.PIC (palette merged on 64..143) and the KING.TXT text with `$` substitution in the
//! box at the bottom; `_` lines of the section become a menu.
//!
//! The screen runs a whole audience (`diplo_ai`, CIV.EXE overlay 6): it shows each screen of
//! it, passes the answer to `diplo_ai::respond` and shows what follows, until the audience is
//! over. The emissary question and the texts of a refused emissary appear over the map
//! (`DiplomacyState::plain`), "Check Intelligence" opens the intelligence report. An audience
//! from the engine (`Decision::Diplomacy`) ends with `DialogResult::Done`, which `reply_for`
//! turns into `DecisionReply::Cancel`: the engine has nothing left to do.

use crate::assets::text::{split_menu, substitute, Vars};
use crate::assets::Palette;
use crate::game::diplomacy::{DiplomacyState, Proposal};
use crate::game::Slot;
use crate::gfx::text::{draw_text, wrap};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::{Event, Key, MouseButton};
use crate::rules::advance_ids as adv;
use crate::ui::context::UiContext;
use crate::ui::eventpics::Dismiss;
use crate::ui::govtscreen::{back_sheet, draw_advisors, govt_sheet, throne_palette, THRONE_BOX};
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::{MenuDialog, MessageBox};

/// KINGnn.PIC index of a civilisation's leader (identified on the sheets).
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

/// Leader bust in a KING sheet (bottom-right box).
pub const KING_BUST: Rect = Rect::new(181, 65, 139, 135);

/// Small portrait `i` (same numbering as KINGnn) in the SLAM2 face strip.
pub fn face_rect(i: u8) -> Rect {
    let c = (i % 7) as i32;
    let r = (i / 7) as i32;
    Rect::new(c * 28 + 1, r * 34, 27, 33)
}

/// Number of advisors behind a leader: by size rank among living civilisations (largest 4).
pub fn advisor_count(g: &crate::game::Game, slot: Slot) -> usize {
    let mut sizes: Vec<(usize, Slot)> = g.alive_slots().into_iter().filter(|s| *s != 0).map(|s| (g.cities_of(s).len(), s)).collect();
    sizes.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let rank = sizes.iter().position(|(_, s)| *s == slot).unwrap_or(3);
    (4 - rank.min(3)).max(1)
}

/// How the current screen is shown.
fn over_map(st: &DiplomacyState) -> bool {
    st.plain || matches!(st.proposal, Proposal::Emissary | Proposal::Intelligence)
}

pub struct AudienceScreen {
    state: Option<DiplomacyState>,
    ai: Slot,
    /// A Diplomat's audience to open on entering (`diplo_ai::human_audience`).
    start: Option<(i32, i32)>,
    /// The dialog over the map for the current screen was pushed.
    sub_shown: bool,
    /// The leader's anthem is playing (the throne room is up).
    anthem: bool,
    header: String,
    body: Vec<String>,
    choices: Vec<String>,
    disabled: Vec<bool>,
    font: usize,
    sel: usize,
    dismiss: Dismiss,
}

impl AudienceScreen {
    fn empty(ai: Slot) -> AudienceScreen {
        AudienceScreen {
            state: None,
            ai,
            start: None,
            sub_shown: false,
            anthem: false,
            header: String::new(),
            body: Vec::new(),
            choices: Vec::new(),
            disabled: Vec::new(),
            font: 0,
            sel: 0,
            dismiss: Dismiss::new(),
        }
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

impl AudienceScreen {
    /// The text and menu of the current screen.
    fn layout(&mut self, ctx: &UiContext) {
        let Some(st) = &self.state else { return };
        let Some(g) = ctx.game.as_ref() else { return };
        let lines = if st.proposal == Proposal::Message {
            st.text.iter().map(|l| format!("{l}^")).collect()
        } else {
            ctx.texts.king.section(st.proposal.section()).map(|l| l.to_vec()).unwrap_or_default()
        };
        let v = Vars { rplc1: st.rplc1.clone(), rplc2: st.rplc2.clone(), us: st.us(g), them: st.them(g), bucks: st.bucks };
        let m = split_menu(&substitute(&lines, &v));
        let p = g.player(st.ai);
        self.header = format!("{} {} of the {}:", p.title(), p.leader, g.civ_name(st.ai));
        self.choices = if st.menu.is_empty() { m.choices } else { st.menu.clone() };
        self.disabled = (0..self.choices.len()).map(|i| st.disabled.get(i).copied().unwrap_or(false)).collect();
        self.sel = (0..self.choices.len()).find(|i| !self.disabled[*i]).unwrap_or(0);
        // pick the font so that everything fits in the box
        let width = THRONE_BOX.w - 16;
        for font in [0usize, 1] {
            let lh = ctx.fonts.get(font).line_height();
            let mut body = Vec::new();
            for l in &m.body {
                if l.is_empty() {
                    body.push(String::new());
                } else {
                    body.extend(wrap(&ctx.fonts, font, width, l));
                }
            }
            let rows = 1 + body.len() + self.choices.len();
            self.font = font;
            self.body = body;
            if rows as i32 * lh <= THRONE_BOX.h - 6 {
                break;
            }
        }
    }

    /// The text of the current screen as plain lines (for the boxes over the map).
    fn plain_lines(&self, ctx: &UiContext) -> Vec<String> {
        let Some(st) = &self.state else { return Vec::new() };
        if st.proposal == Proposal::Message || st.proposal == Proposal::Emissary {
            return st.text.clone();
        }
        let Some(g) = ctx.game.as_ref() else { return Vec::new() };
        let lines = ctx.texts.king.section(st.proposal.section()).map(|l| l.to_vec()).unwrap_or_default();
        let v = Vars { rplc1: st.rplc1.clone(), rplc2: st.rplc2.clone(), us: st.us(g), them: st.them(g), bucks: st.bucks };
        split_menu(&substitute(&lines, &v)).body
    }

    fn line_h(&self, ctx: &UiContext) -> i32 {
        ctx.fonts.get(self.font).line_height()
    }

    fn choice_y(&self, ctx: &UiContext, i: usize) -> i32 {
        let lh = self.line_h(ctx);
        THRONE_BOX.y + 3 + (1 + self.body.len() as i32 + i as i32) * lh
    }

    fn choice_at(&self, ctx: &UiContext, mx: i32, my: i32) -> Option<usize> {
        if mx < THRONE_BOX.x || mx >= THRONE_BOX.right() {
            return None;
        }
        let lh = self.line_h(ctx);
        (0..self.choices.len()).find(|&i| {
            let y = self.choice_y(ctx, i);
            my >= y && my < y + lh && !self.disabled[i]
        })
    }

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
        self.dismiss = Dismiss::new();
        self.layout(ctx);
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

    fn step_sel(&mut self, delta: isize) {
        let n = self.choices.len();
        for k in 1..=n {
            let i = (self.sel as isize + delta * k as isize).rem_euclid(n as isize) as usize;
            if !self.disabled[i] {
                self.sel = i;
                return;
            }
        }
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
        self.dismiss.tick();
        let Some(st) = self.state.clone() else { return Transition::Pop(DialogResult::Done) };
        if !over_map(&st) || self.sub_shown {
            return Transition::None;
        }
        self.sub_shown = true;
        if st.proposal == Proposal::Intelligence {
            return Transition::Push(Box::new(crate::ui::report_advisors::Dossier::new(st.ai)));
        }
        let lines = self.plain_lines(ctx);
        if st.proposal == Proposal::Emissary {
            return Transition::Push(Box::new(MenuDialog::new(lines, st.menu.clone()).with_pos(100, 80)));
        }
        // a refused emissary: the declaration in a plain box (o 0x6f83 at (40, 100))
        Transition::Push(Box::new(MessageBox::new(lines).with_pos(40, 100)))
    }

    fn resume(&mut self, result: DialogResult, ctx: &mut UiContext) -> Transition {
        let Some(st) = self.state.as_ref() else { return Transition::Pop(DialogResult::Done) };
        if !over_map(st) {
            return Transition::None;
        }
        let choice = match (st.proposal, result) {
            (Proposal::Emissary, DialogResult::Choice(i)) => i,
            // Escape: "No."
            (Proposal::Emissary, _) => 1,
            _ => 0,
        };
        self.answer(ctx, choice)
    }

    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        let Some(st) = &self.state else { return Transition::None };
        if over_map(st) {
            return Transition::None;
        }
        if self.choices.is_empty() {
            return if self.dismiss.closes(ev) { self.answer(ctx, 0) } else { Transition::None };
        }
        match ev {
            Event::Key { key, down: true, .. } => match key {
                Key::Up | Key::Kp(8) => self.step_sel(-1),
                Key::Down | Key::Kp(2) => self.step_sel(1),
                Key::Enter | Key::Space => {
                    let s = self.sel;
                    return self.answer(ctx, s);
                }
                Key::Esc => return self.answer(ctx, usize::MAX),
                _ => {}
            },
            Event::MouseMove { x, y } => {
                if let Some(i) = self.choice_at(ctx, *x, *y) {
                    self.sel = i;
                }
            }
            Event::MouseButton { button: MouseButton::Left, x, y, down: true } => {
                if let Some(i) = self.choice_at(ctx, *x, *y) {
                    return self.answer(ctx, i);
                }
            }
            _ => {}
        }
        Transition::None
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        if self.state.as_ref().map(over_map).unwrap_or(true) {
            return;
        }
        let (back, govt, king) = self.sheets(ctx);
        let bg = ctx.pic(&back);
        fb.blit(&bg.surf, 0, 0);
        let count = ctx.game.as_ref().map(|g| advisor_count(g, self.ai)).unwrap_or(1);
        draw_advisors(fb, ctx, &govt, count, THRONE_BOX.y);
        let kp = ctx.pic(&king);
        let key = kp.surf.get(KING_BUST.x, KING_BUST.y);
        fb.blit_part(&kp.surf, KING_BUST, (320 - KING_BUST.w) / 2, THRONE_BOX.y - KING_BUST.h, Some(key));
        // text
        let pal = self.palette(ctx);
        let ink = pal.nearest([0, 0, 0]);
        let grey = pal.nearest([96, 96, 96]);
        let title = pal.nearest([130, 32, 20]);
        let hilite = pal.nearest([255, 255, 255]);
        let lh = self.line_h(ctx);
        let x = THRONE_BOX.x + 8;
        let mut y = THRONE_BOX.y + 3;
        draw_text(fb, &ctx.fonts, &TextStyle::new(self.font, title), x, y, &self.header);
        y += lh;
        for l in &self.body {
            draw_text(fb, &ctx.fonts, &TextStyle::new(self.font, ink), x, y, l);
            y += lh;
        }
        for (i, c) in self.choices.iter().enumerate() {
            let cy = self.choice_y(ctx, i);
            if i == self.sel {
                fb.fill_rect(Rect::new(x - 4, cy - 1, THRONE_BOX.w - 8, lh), hilite);
            }
            let colour = if self.disabled[i] { grey } else { ink };
            draw_text(fb, &ctx.fonts, &TextStyle::new(self.font, colour), x + 2, cy, c);
        }
    }

    fn palette(&self, ctx: &UiContext) -> Palette {
        if self.state.as_ref().map(over_map).unwrap_or(true) {
            return ctx.palette;
        }
        let (back, govt, king) = self.sheets(ctx);
        throne_palette(ctx, &back, &govt).merged(&ctx.pic_pal(&king), 64..=143)
    }

    fn draws_parent(&self) -> bool {
        self.state.as_ref().map(over_map).unwrap_or(true)
    }
}
