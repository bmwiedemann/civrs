//! Audience with a rival leader: throne room of the leader's government (BACKgX.PIC),
//! advisors from GOVTgX.PIC behind the king (count = size rank), the leader's bust from
//! KINGnn.PIC (palette merged on 64..143) and the KING.TXT text with `$` substitution in the
//! box at the bottom; `_` lines of the section become a menu.
//!
//! Engine-driven audiences (`Decision::Diplomacy`) pop `DialogResult::Choice(i)` (or `Done`
//! for texts without choices); `reply_for` turns that into `DecisionReply::Choice(i)` and the
//! engine answers with follow-up decisions. Human-initiated audiences (`human_audience`) drive
//! `diplo_ai::respond` themselves.

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
use crate::ui::widgets::YesNo;

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

/// Proposals that open with "An emissary ... wishes to speak with you" (not follow-ups).
fn opens_with_emissary(p: Proposal) -> bool {
    matches!(
        p,
        Proposal::Exchange | Proposal::Cancel | Proposal::Tribute | Proposal::TakeCiv | Proposal::Peace | Proposal::HelpMe | Proposal::Crusade | Proposal::GiveCiv | Proposal::GiveCash | Proposal::Grovel | Proposal::Provoke | Proposal::Rid
    )
}

pub struct AudienceScreen {
    state: Option<DiplomacyState>,
    ai: Slot,
    /// The screen itself calls `diplo_ai::respond` for follow-ups.
    self_driven: bool,
    /// Ask "Will you receive him?" over the map first.
    prompt: bool,
    prompt_shown: bool,
    header: String,
    body: Vec<String>,
    choices: Vec<String>,
    font: usize,
    sel: usize,
    dismiss: Dismiss,
}

/// Audience for an engine decision.
pub fn audience_screen(state: DiplomacyState) -> Box<dyn Screen> {
    let prompt = !state.human_initiated && opens_with_emissary(state.proposal);
    Box::new(AudienceScreen { ai: state.ai, state: Some(state), self_driven: false, prompt, prompt_shown: false, header: String::new(), body: Vec::new(), choices: Vec::new(), font: 0, sel: 0, dismiss: Dismiss::new() })
}

/// Audience requested by the human (Attitude advisor, diplomats): loops `respond` itself.
pub fn human_audience(ai_slot: u8) -> Box<dyn Screen> {
    Box::new(AudienceScreen { ai: ai_slot, state: None, self_driven: true, prompt: false, prompt_shown: false, header: String::new(), body: Vec::new(), choices: Vec::new(), font: 0, sel: 0, dismiss: Dismiss::new() })
}

impl AudienceScreen {
    fn layout(&mut self, ctx: &UiContext) {
        let Some(st) = &self.state else { return };
        let Some(g) = ctx.game.as_ref() else { return };
        let lines = ctx.texts.king.section(st.proposal.section()).map(|l| l.to_vec()).unwrap_or_default();
        let v = Vars { rplc1: st.rplc1.clone(), rplc2: st.rplc2.clone(), us: st.us(g).to_string(), them: st.them(g).to_string(), bucks: st.bucks };
        let m = split_menu(&substitute(&lines, &v));
        let p = g.player(st.ai);
        self.header = format!("{} {} of the {}:", p.title(), p.leader, g.civ_name(st.ai));
        self.choices = m.choices;
        self.sel = 0;
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
            my >= y && my < y + lh
        })
    }

    /// The human answered (or acknowledged) the current text.
    fn finish(&mut self, ctx: &mut UiContext, choice: Option<usize>) -> Transition {
        if !self.self_driven {
            return match choice {
                Some(i) => Transition::Pop(DialogResult::Choice(i)),
                None => Transition::Pop(DialogResult::Done),
            };
        }
        let Some(st) = self.state.clone() else { return Transition::Pop(DialogResult::Done) };
        let next = match choice {
            Some(i) => crate::ai::diplo_ai::respond(ctx.game_mut(), &st, i),
            None => None,
        };
        match next {
            Some(n) => {
                self.state = Some(n);
                self.dismiss = Dismiss::new();
                self.layout(ctx);
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
        if self.state.is_none() {
            let st = crate::ai::diplo_ai::human_audience(ctx.game_mut(), self.ai);
            self.state = Some(st);
        }
        self.layout(ctx);
    }

    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        self.dismiss.tick();
        if self.prompt && !self.prompt_shown {
            self.prompt_shown = true;
            let (civ, female) = ctx.game.as_ref().map(|g| (g.civ_name(self.ai).to_string(), g.player(self.ai).civ().female)).unwrap_or_default();
            let who = if female { "her" } else { "him" };
            let q = vec![format!("An emissary from the {civ} wishes to speak with you. Will you receive {who}?")];
            return Transition::Push(Box::new(YesNo::new(q, "Yes.", "No.").with_pos(210, 75)));
        }
        Transition::None
    }

    fn resume(&mut self, result: DialogResult, ctx: &mut UiContext) -> Transition {
        if self.prompt {
            self.prompt = false;
            self.dismiss = Dismiss::new();
            if result == DialogResult::No {
                // refusing the emissary counts as rejecting the proposal
                return self.finish(ctx, Some(0));
            }
        }
        Transition::None
    }

    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if self.prompt {
            return Transition::None;
        }
        if self.choices.is_empty() {
            return if self.dismiss.closes(ev) { self.finish(ctx, None) } else { Transition::None };
        }
        let n = self.choices.len();
        match ev {
            Event::Key { key, down: true, .. } => match key {
                Key::Up | Key::Kp(8) => self.sel = (self.sel + n - 1) % n,
                Key::Down | Key::Kp(2) => self.sel = (self.sel + 1) % n,
                Key::Enter | Key::Space => return self.finish(ctx, Some(self.sel)),
                _ => {}
            },
            Event::MouseMove { x, y } => {
                if let Some(i) = self.choice_at(ctx, *x, *y) {
                    self.sel = i;
                }
            }
            Event::MouseButton { button: MouseButton::Left, x, y, down: true } => {
                if let Some(i) = self.choice_at(ctx, *x, *y) {
                    return self.finish(ctx, Some(i));
                }
            }
            _ => {}
        }
        Transition::None
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        if self.prompt {
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
            draw_text(fb, &ctx.fonts, &TextStyle::new(self.font, ink), x + 2, cy, c);
        }
    }

    fn palette(&self, ctx: &UiContext) -> Palette {
        if self.prompt {
            return ctx.palette;
        }
        let (back, govt, king) = self.sheets(ctx);
        throne_palette(ctx, &back, &govt).merged(&ctx.pic_pal(&king), 64..=143)
    }

    fn draws_parent(&self) -> bool {
        self.prompt
    }
}
