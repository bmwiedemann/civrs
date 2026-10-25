//! End-of-game screens: SAD.PIC when the civilisation is destroyed, the SLAM1.PIC score wall
//! with the leader's portrait (score, rating, Hall of Fame entry) for retirement / the time
//! limit / conquest, ARCH.PIC with the KING.TXT ARCH text afterwards, and the spaceship
//! arrival (see `spaceship_ui`). `end_game_scores` (0x2c20) opens with the power graph and,
//! unless the human retired (ds:0xdc48 = 2), the replay (see `replay`).

use crate::assets::text::{paragraphs, substitute, Vars};
use crate::assets::Palette;
use crate::game::EndReason;
use crate::gfx::text::{draw_text, draw_text_centered, text_width};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::Event;
use crate::rules;
use crate::ui::context::UiContext;
use crate::ui::diplomacy_ui::{face_rect, king_index};
use crate::ui::eventpics::{Dismiss, PicScreen};
use crate::ui::replay::{PowerGraph, Replay};
use crate::ui::reveal::Reveal;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::wonderscreen::blit_scaled;

/// Inner frame of the SLAM1 wall plaque.
const PLAQUE: Rect = Rect::new(17, 148, 288, 37);
/// Driver duration of the dissolve into the score wall (overlay call site, after the
/// "... destroy the ... civilization!" plaque line: `push 30; push 1; call dissolve`).
const DISSOLVE: u32 = 30;

/// The screen (chain) for a game end reason.
pub fn end_screen(ctx: &UiContext, reason: EndReason) -> Box<dyn Screen> {
    let g = ctx.game.as_ref();
    let (leader, civ) = g.map(|g| (g.human().leader.clone(), g.civ_name(g.human_slot).to_string())).unwrap_or_default();
    match reason {
        EndReason::Destroyed => {
            let lines = vec![format!("The {civ} civilization has been destroyed!"), format!("{leader}, your reign is over.")];
            Box::new(Chain { screen: Box::new(PicScreen::new("SAD", lines).with_caption_pos(20, 20).with_dissolve(ctx, DISSOLVE)), next: Some(arch_screen(ctx)) })
        }
        EndReason::Spaceship(slot) => chain(vec![crate::ui::spaceship_ui::arrival_screen(ctx, slot), Box::new(PowerGraph), Box::new(Replay::new())]),
        EndReason::Conquest => {
            let wall = ScoreWall::new(ctx, vec![format!("{leader}, you have conquered the world!")]);
            chain(vec![Box::new(PowerGraph), Box::new(Replay::new()), Box::new(wall)])
        }
        EndReason::TimeLimit => {
            let wall = ScoreWall::new(ctx, vec!["Your reign has reached the end of its allotted time.".to_string()]);
            chain(vec![Box::new(PowerGraph), Box::new(Replay::new()), Box::new(wall), arch_screen(ctx)])
        }
        EndReason::Retired => {
            let wall = ScoreWall::new(ctx, vec![format!("{leader} retires from the throne of the {civ}.")]);
            chain(vec![Box::new(PowerGraph), Box::new(wall), arch_screen(ctx)])
        }
    }
}

/// The screens one after the other.
fn chain(screens: Vec<Box<dyn Screen>>) -> Box<dyn Screen> {
    let mut next: Option<Box<dyn Screen>> = None;
    for screen in screens.into_iter().rev() {
        next = Some(match next {
            None => screen,
            Some(n) => Box::new(Chain { screen, next: Some(n) }),
        });
    }
    next.expect("an empty chain")
}

/// ARCH.PIC with the "Centuries later, archeologists discover..." text.
pub fn arch_screen(ctx: &UiContext) -> Box<dyn Screen> {
    let g = ctx.game.as_ref();
    let evidence = g
        .and_then(|g| {
            let me = g.human_slot;
            g.wonders_built.iter().enumerate().find(|(_, c)| c.and_then(|c| g.city_opt(c)).map(|c| c.owner == me).unwrap_or(false)).map(|(w, _)| format!("the {}", rules::WONDERS[w].name))
        })
        .unwrap_or_else(|| "temples".to_string());
    let v = Vars { rplc1: evidence, us: g.map(|g| g.civ_name(g.human_slot).to_ascii_uppercase()).unwrap_or_default(), ..Default::default() };
    let lines = ctx.texts.king.section("ARCH").map(|l| l.to_vec()).unwrap_or_default();
    let text = paragraphs(&substitute(&lines, &v));
    Box::new(PicScreen::new("ARCH", text).with_caption_pos(6, 4).with_caption_width(200))
}

/// Shows `screen`, then `next` (if any) in its place.
struct Chain {
    screen: Box<dyn Screen>,
    next: Option<Box<dyn Screen>>,
}

impl Screen for Chain {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        self.screen.on_enter(ctx);
    }
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        let t = self.screen.update(ctx);
        self.forward(t, ctx)
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        let t = self.screen.handle(ev, ctx);
        self.forward(t, ctx)
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        self.screen.draw(fb, ctx);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        self.screen.palette(ctx)
    }
    fn draws_parent(&self) -> bool {
        self.screen.draws_parent()
    }
    fn wants_cursor(&self) -> bool {
        self.screen.wants_cursor()
    }
}

impl Chain {
    fn forward(&mut self, t: Transition, ctx: &mut UiContext) -> Transition {
        match t {
            Transition::Pop(_) => match self.next.take() {
                Some(n) => Transition::Replace(n),
                None => Transition::Pop(DialogResult::Done),
            },
            // a screen of the chain that hands over to another keeps the rest of the chain
            Transition::Replace(mut s) => {
                s.on_enter(ctx);
                self.screen = s;
                Transition::None
            }
            other => other,
        }
    }
}

/// SLAM1 score wall: the leader's portrait on the panelled wall, score and rating on the
/// plaque; records a Hall of Fame entry on entry.
pub struct ScoreWall {
    lines: Vec<String>,
    king: u8,
    dismiss: Dismiss,
    recorded: bool,
    reveal: Reveal,
}

impl ScoreWall {
    pub fn new(ctx: &UiContext, mut lines: Vec<String>) -> ScoreWall {
        let mut king = 10;
        if let Some(g) = ctx.game.as_ref() {
            let me = g.human_slot;
            king = king_index(g.player(me).civ);
            let s = g.score(me);
            lines.push(format!("Civilization score: {} ({}%)", s.total, s.percent));
            lines.push(format!("Your rating: {}.", g.rating_name(me)));
            lines.push("Your final score has been entered into the Hall of Fame.".into());
        }
        ScoreWall { lines, king, dismiss: Dismiss::new(), recorded: false, reveal: Reveal::new(ctx, DISSOLVE) }
    }
}

impl Screen for ScoreWall {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        if self.recorded {
            return;
        }
        self.recorded = true;
        if let Some(g) = ctx.game.as_ref() {
            ctx.audio.national_theme(g.human().civ, true);
        }
        if let Some(g) = ctx.game.as_ref() {
            let me = g.human_slot;
            let s = g.score(me);
            let e = crate::save::FameEntry { leader: g.human().leader.clone(), civ_plural: g.civ_name(me).to_string(), score: s.total, percent: s.percent, year: g.year(), difficulty: g.setup.difficulty.as_u8(), title: g.human().title().trim().to_string(), population: g.total_population(me) };
            ctx.hof.add(e);
            ctx.hof.save(&ctx.save_dir);
        }
    }
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.dismiss.tick();
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if !self.dismiss.closes(ev) {
            Transition::None
        } else if self.reveal.done() {
            Transition::Pop(DialogResult::Done)
        } else {
            self.reveal.finish();
            Transition::None
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let pal = ctx.pic_pal("SLAM1");
        let mut content = Surface::screen();
        self.render(&mut content, ctx, &pal);
        let old = ctx.palette;
        self.reveal.draw(fb, &content, &old, &pal);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.pic_pal("SLAM1")
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

impl ScoreWall {
    /// The wall with the portrait, name plate and plaque text.
    fn render(&self, fb: &mut Surface, ctx: &UiContext, pal: &Palette) {
        let wall = ctx.pic("SLAM1");
        fb.blit(&wall.surf, 0, 0);
        // framed portrait (SLAM2 face strip, enlarged 2x) hanging on the wall
        let faces = ctx.pic("SLAM2");
        let face_pal = ctx.pic_pal("SLAM2");
        let mut map = [0u8; 256];
        for (i, m) in map.iter_mut().enumerate() {
            *m = pal.nearest(face_pal.rgb(i as u8));
        }
        let r = face_rect(self.king);
        let (fw, fh) = (r.w * 2, r.h * 2);
        let (fx, fy) = ((320 - fw) / 2, 36);
        let frame = Rect::new(fx - 4, fy - 4, fw + 8, fh + 8);
        fb.fill_rect(frame, pal.nearest([130, 32, 20]));
        fb.rect_outline(frame, pal.nearest([255, 219, 89]));
        fb.rect_outline(frame.inset(1), pal.nearest([0, 0, 0]));
        blit_scaled(fb, &faces.surf, r, fx, fy, 2, None, &map);
        // name plate under the portrait
        if let Some(g) = ctx.game.as_ref() {
            let name = format!("{} of the {}", g.human().leader, g.civ_name(g.human_slot));
            let w = text_width(&ctx.fonts, 0, &name) + 8;
            let plate = Rect::new(160 - w / 2, fy + fh + 8, w, 11);
            fb.fill_rect(plate, pal.nearest([255, 219, 89]));
            fb.rect_outline(plate, pal.nearest([0, 0, 0]));
            draw_text_centered(fb, &ctx.fonts, &TextStyle::new(0, pal.nearest([0, 0, 0])), 160, plate.y + 2, &name);
        }
        // plaque text
        let ink = pal.nearest([0, 0, 0]);
        let mut y = PLAQUE.y + 3;
        for l in &self.lines {
            for part in crate::gfx::text::wrap(&ctx.fonts, 0, PLAQUE.w - 12, l) {
                draw_text(fb, &ctx.fonts, &TextStyle::new(0, ink), PLAQUE.x + 6, y, &part);
                y += 8;
            }
        }
    }
}
