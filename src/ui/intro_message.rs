//! Leader introduction (KING.TXT INIT) on a white meander-bordered screen, then the map view.

use crate::assets::text::{paragraphs, substitute, Vars};
use crate::gfx::text::{draw_text, text_width};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::Event;
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::tilerender::cell;

struct LeaderIntro {
    lines: Vec<String>,
    guard: u32,
}

pub fn leader_intro() -> Box<dyn Screen> {
    Box::new(LeaderIntro { lines: Vec::new(), guard: 10 })
}

/// Draw the Greek-key border used by the leader intro and other "white" screens.
pub fn draw_meander_border(fb: &mut Surface, ctx: &UiContext) {
    let sp = &ctx.sp.surf;
    let h = cell(9, 8);
    let v = cell(10, 8);
    for x in (0..320).step_by(16) {
        fb.blit_part(sp, Rect::new(h.x, h.y, 16, 8), x, 0, None);
        fb.blit_part(sp, Rect::new(h.x, h.y + 8, 16, 8), x, 192, None);
    }
    for y in (0..200).step_by(16) {
        fb.blit_part(sp, Rect::new(v.x, v.y, 8, 16), 0, y, None);
        fb.blit_part(sp, Rect::new(v.x + 8, v.y, 8, 16), 312, y, None);
    }
}

impl Screen for LeaderIntro {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        let g = ctx.game();
        let p = g.human();
        let mut known: Vec<&str> = p.known_advances().iter().map(|a| crate::rules::ADVANCES[*a as usize].name).collect();
        known.sort();
        let v = Vars { rplc1: p.leader.clone(), us: p.plural().to_string(), ..Default::default() };
        let lines = ctx.texts.king.section("INIT").map(|l| l.to_vec()).unwrap_or_default();
        let mut body = paragraphs(&substitute(&lines, &v));
        let mut tail = known.join(", ");
        if !tail.is_empty() {
            tail.push_str(", ");
        }
        body.push(format!("{tail}and Roads."));
        self.lines = body;
    }

    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }

    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => {
                let mut view = crate::ui::mapview::MapView::new();
                view.first_turn_hint = true;
                Transition::Replace(Box::new(view))
            }
            _ => Transition::None,
        }
    }

    fn resume(&mut self, _r: DialogResult, _ctx: &mut UiContext) -> Transition {
        Transition::None
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        fb.fill(16);
        draw_meander_border(fb, ctx);
        let diffs = ctx.pic("DIFFS");
        let r = crate::ui::setup::PORTRAITS[ctx.game().setup.difficulty as usize];
        fb.blit_part(&diffs.surf, r, 134, 20, None);
        let st = TextStyle::new(0, 0);
        let maxw = self.lines.iter().map(|l| text_width(&ctx.fonts, 0, l)).max().unwrap_or(0);
        let x = (320 - maxw) / 2;
        let mut y = 82;
        for l in &self.lines {
            draw_text(fb, &ctx.fonts, &st, x, y, l);
            y += 8;
        }
    }

    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        let mut p = ctx.pic_pal("DIFFS");
        p.0[16] = [255, 255, 255];
        p.0[0] = [0, 0, 0];
        // meander border colours come from the sprite sheet palette
        let sp = &ctx.sp.surf;
        for c in [cell(9, 8), cell(10, 8)] {
            for y in c.y..c.bottom() {
                for x in c.x..c.right() {
                    let v = sp.get(x, y) as usize;
                    p.0[v] = ctx.sp.pal.0[v];
                }
            }
        }
        p
    }

    fn draws_parent(&self) -> bool {
        false
    }
}
