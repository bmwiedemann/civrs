//! The world-conquest ending, `conquest_victory()` (ovl 9 +0xdde, o 0xd953): on a black
//! screen the replay runs silently (mode 4) and stops at each civilization the human destroyed
//! for its "slam" (`civ_destroyed_slam` o 0xdaee): the leader's bust on the SLAM1 wall of
//! trophies, the plaque "<year>: <Us> destroy / <Adjective> civilization!", and a head added to
//! the trophies. Then "The entire world hails / <leader> the CONQUEROR!" on the plaque.

use crate::assets::Palette;
use crate::game::replay::{kind, tribe_names, Record};
use crate::game::{Game, Slot};
use crate::gfx::dissolve::{pixels_per_frame, Dissolve};
use crate::gfx::text::{draw_text_centered, TextStyle};
use crate::gfx::{Rect, Surface};
use crate::platform::Event;
use crate::rules;
use crate::ui::context::UiContext;
use crate::ui::diplomacy_ui::{face_rect, king_index};
use crate::ui::screen::{DialogResult, Screen, Transition};

/// Window font 6, set before the replay (0xd9a4).
const FONT: usize = 5;
/// The bust cut from KINGnn.PIC (grab_sprite(1, 181, 67, 139, 133)), drawn at (90, 0).
const BUST: Rect = Rect::new(181, 67, 139, 133);
/// The two faces of the sheet's left column: before (1, 51) and after (1, 151) the slam.
const FACE_BEFORE: Rect = Rect::new(1, 51, 59, 49);
const FACE_AFTER: Rect = Rect::new(1, 151, 59, 49);
/// Where SLAM2.PIC's heads go in the work page (load at row 133).
const HEADS_Y: i32 = 133;
/// Offsets of the face in each bust (far 0x2679:0x10 + 4 * face: x, dx; 0x2679:0x48: y, dy).
const FACE_X: [(i32, i32); 14] = [(250, 27), (242, 13), (253, 30), (244, 23), (241, 23), (245, 25), (232, 15), (231, 13), (234, 14), (230, 16), (240, 20), (241, 27), (226, 8), (242, 21)];
const FACE_Y: [(i32, i32); 14] = [(101, 6), (96, 1), (114, 15), (109, 14), (102, 16), (123, 11), (107, 9), (114, 11), (102, 10), (115, 14), (102, 14), (120, 12), (99, 11), (101, 16)];
/// Ticks of a slam: the text at 240 (tick_get >= 0xf0), the dissolve 60 ticks later.
const TEXT_AT: u32 = 240;
const DISSOLVE_AT: u32 = TEXT_AT + 60;
/// Driver duration of the dissolve into the wall with the new trophy (0:0x7b5(1, 30)).
const DISSOLVE: u32 = 30;
/// VGA: the old picture fades to black (fade_to_colour(5, 0, 0, 0)), then the slam fades in
/// from black (fade_palette(8, ...)) before its ticks start.
const FADE_OUT: u32 = 5;
const FADE_IN: u32 = 8;

/// Where trophy slot `i` hangs: two rows of seven, the first at y 49, the second above it.
fn slot_pos(i: usize) -> (i32, i32) {
    (46 * (i as i32 % 7) + 8, 49 - 42 * (i as i32 / 7))
}

/// The slot the n-th trophy takes (0xddc9): from the ends of a row to its middle (0, 6, 1, 5,
/// 2, 4, 3), the lower row first.
fn trophy_slot(n: u8) -> usize {
    let k = n as usize % 7;
    let k = if k % 2 == 1 { 6 - k / 2 } else { k / 2 };
    if n >= 7 {
        k + 7
    } else {
        k
    }
}

/// One slam under way.
struct Slam {
    t: u32,
    /// Fade ticks left: out of the old picture (`from`), then into the slam palette.
    fade_out: u32,
    fade_in: u32,
    from: Palette,
    to: Palette,
    /// The visible screen; `page` is what it dissolves into (page 1: the wall with every
    /// trophy, then the plaque text).
    screen: Surface,
    page: Surface,
    face_at: (i32, i32),
    king: String,
    lines: [String; 2],
    dissolve: Option<Dissolve>,
}

enum Phase {
    Replaying,
    Slam(Box<Slam>),
    Hail,
}

pub struct Conquest {
    records: Vec<Record>,
    next: usize,
    /// The tribe each slot goes by in the replay (ds:0xd7f4).
    tribes: [u8; 8],
    /// The faces on the wall (far 0x642c:0x37c4, 14 slots) and how many slams there were
    /// (ds:0xb1d4).
    trophies: [Option<u8>; 14],
    count: u8,
    phase: Phase,
    screen: Surface,
    pal: Option<Palette>,
    per_frame: usize,
}

impl Conquest {
    pub fn new(ctx: &UiContext) -> Conquest {
        let g = ctx.game();
        let mut tribes = [0u8; 8];
        for (s, t) in tribes.iter_mut().enumerate() {
            *t = g.player(s as Slot).start_civ;
        }
        let mut screen = Surface::screen();
        screen.fill(0);
        let per_frame = if ctx.instant { crate::gfx::dissolve::PIXELS } else { pixels_per_frame(DISSOLVE) };
        Conquest { records: g.history.records(), next: 0, tribes, trophies: [None; 14], count: 0, phase: Phase::Replaying, screen, pal: None, per_frame }
    }

    fn adjective<'a>(&self, g: &'a Game, s: u8) -> &'a str {
        if s == g.human_slot || s > 7 {
            return g.player(s.min(7)).adjective();
        }
        tribe_names(self.tribes[s as usize]).1
    }

    /// The wall with the trophies (copied from SLAM2's heads, 27x33 cells on a 28x34 grid).
    fn wall(&self, ctx: &UiContext) -> Surface {
        let mut s = ctx.pic("SLAM1").surf.clone();
        let heads = ctx.pic("SLAM2");
        for (i, t) in self.trophies.iter().enumerate() {
            if let Some(f) = t {
                let (x, y) = slot_pos(i);
                s.blit_part(&heads.surf, face_rect(*f), x, y, None);
            }
        }
        s
    }

    /// `civ_destroyed_slam(civ, year)` (o 0xdaee) up to its first wait.
    fn start_slam(&mut self, ctx: &UiContext, civ: u8, year: i32) -> Slam {
        let g = ctx.game();
        let tribe = self.tribes[civ as usize & 7];
        let face = king_index(tribe);
        let king = format!("KING{face:02}");
        let sheet = ctx.pic(&king);
        let mut screen = self.wall(ctx);
        screen.blit_part(&sheet.surf, BUST, 90, 0, Some(0));
        let (fx, fy) = (FACE_X[face as usize], FACE_Y[face as usize]);
        let face_at = (90 + fx.0 - 180 - fx.1, fy.0 - 64 - fy.1 - 2);
        screen.blit_part(&sheet.surf, FACE_BEFORE, face_at.0, face_at.1, Some(0));
        ctx.audio.national_theme(tribe, true);
        // the slam palette: SLAM1.PAL with the king's colours 64..143
        let from = self.palette(ctx);
        let to = ctx.pic_pal("SLAM1").merged(&ctx.pic_pal(&king), 64..=143);
        self.pal = Some(to);
        let (fade_out, fade_in) = if ctx.instant { (0, 0) } else { (FADE_OUT, FADE_IN) };
        let slot = trophy_slot(self.count);
        self.count = self.count.wrapping_add(1);
        if let Some(t) = self.trophies.get_mut(slot) {
            *t = Some(face);
        }
        let page = self.wall(ctx);
        let me = g.human_slot;
        let lines = [
            format!("{}{}: {} destroy", year.abs(), if year < 0 { " BC" } else { " AD" }, g.player(me).plural()),
            // "the " is copied first and then overwritten (strcpy, 0xdfa4)
            format!("{} civilization!", self.adjective(g, civ)),
        ];
        Slam { t: 0, fade_out, fade_in, from, to, screen, page, face_at, king, lines, dissolve: None }
    }

    /// Runs the silent replay to the next slam (mode 4: only the tribe changes and the
    /// civilizations the human destroyed count); false at the end of the log.
    fn replay_to_slam(&mut self, ctx: &UiContext) -> bool {
        let me = ctx.game().human_slot;
        while let Some(r) = self.records.get(self.next).cloned() {
            self.next += 1;
            if r.kind != kind::DESTROYED {
                continue;
            }
            let (civ, by) = (r.args[0], r.args[1]);
            let slam = (by == me).then(|| self.start_slam(ctx, civ, rules::year_for_turn(r.turn)));
            // the slot goes on as its other tribe (0xd368)
            if let Some(t) = self.tribes.get_mut(civ as usize) {
                *t ^= 8;
            }
            if let Some(s) = slam {
                // the old picture stays up while it fades out
                if s.fade_out == 0 {
                    self.screen = s.screen.clone();
                }
                self.phase = Phase::Slam(Box::new(s));
                return true;
            }
        }
        false
    }

    /// The plaque lines, shadowed in colour 20 one row below the line in 23 (VGA colours).
    fn plaque(fb: &mut Surface, ctx: &UiContext, lines: &[String; 2]) {
        for (i, l) in lines.iter().enumerate() {
            let y = 151 + 16 * i as i32;
            draw_text_centered(fb, &ctx.fonts, &TextStyle::new(FONT, 20), 160, y + 1, l);
            draw_text_centered(fb, &ctx.fonts, &TextStyle::new(FONT, 23), 160, y, l);
        }
    }

    fn hail(&mut self, ctx: &UiContext) {
        let g = ctx.game();
        ctx.audio.national_theme(g.human().civ, true);
        self.screen.fill_rect(Rect::new(18, 150, 284, 32), 34);
        let lines = ["The entire world hails".to_string(), format!("{} the CONQUEROR!", g.human().leader)];
        Self::plaque(&mut self.screen, ctx, &lines);
        self.phase = Phase::Hail;
    }
}

impl Screen for Conquest {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        if matches!(self.phase, Phase::Replaying) && !self.replay_to_slam(ctx) {
            self.hail(ctx);
        }
    }
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        let Phase::Slam(s) = &mut self.phase else { return Transition::None };
        if s.fade_out > 0 {
            s.fade_out -= 1;
            if s.fade_out == 0 {
                self.screen = s.screen.clone();
            }
            return Transition::None;
        }
        if s.fade_in > 0 {
            s.fade_in -= 1;
            return Transition::None;
        }
        s.t += 1;
        if s.t == TEXT_AT || (ctx.instant && s.t == 1) {
            Self::plaque(&mut s.screen, ctx, &s.lines);
            // the text strip (rows 133..199) goes into the page that is dissolved in
            let strip = Rect::new(0, HEADS_Y, 320, 200 - HEADS_Y);
            let shown = s.screen.clone();
            s.page.blit_part(&shown, strip, 0, HEADS_Y, None);
            let sheet = ctx.pic(&s.king);
            s.screen.blit_part(&sheet.surf, FACE_AFTER, s.face_at.0, s.face_at.1, Some(0));
            ctx.audio.fade_out();
        }
        if s.t == DISSOLVE_AT || (ctx.instant && s.t == 1) {
            ctx.audio.national_theme(ctx.game().human().civ, true);
            s.dissolve = Some(Dissolve::new());
        }
        if let Some(d) = &mut s.dissolve {
            d.copy(&mut s.screen, &s.page, self.per_frame);
            if d.finished() {
                ctx.audio.fade_out();
                self.screen = s.page.clone();
                self.phase = Phase::Replaying;
                if !self.replay_to_slam(ctx) {
                    self.hail(ctx);
                }
                return Transition::None;
            }
        }
        if let Phase::Slam(s) = &self.phase {
            self.screen = s.screen.clone();
        }
        Transition::None
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        match (&self.phase, ev) {
            (Phase::Hail, Event::Key { down: true, .. } | Event::MouseButton { down: true, .. }) => {
                ctx.audio.fade_out();
                Transition::Pop(DialogResult::Done)
            }
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, _ctx: &mut UiContext) {
        fb.blit(&self.screen, 0, 0);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        match &self.phase {
            Phase::Slam(s) if s.fade_out > 0 => s.from.dimmed((255 * s.fade_out / FADE_OUT) as u8),
            Phase::Slam(s) if s.fade_in > 0 => s.to.dimmed((255 * (FADE_IN - s.fade_in) / FADE_IN) as u8),
            _ => self.pal.unwrap_or_else(|| ctx.game_palette()),
        }
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trophies_fill_each_row_from_the_ends() {
        let order: Vec<usize> = (0..14).map(trophy_slot).collect();
        assert_eq!(order, vec![0, 6, 1, 5, 2, 4, 3, 7, 13, 8, 12, 9, 11, 10]);
        assert_eq!(slot_pos(0), (8, 49));
        assert_eq!(slot_pos(13), (284, 7));
    }
}
