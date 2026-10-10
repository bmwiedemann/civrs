//! The "VIEW" city landscape: the HILL meadow with the city's buildings from CITYPIX1-3
//! (ancient / medieval / modern) and the city name and year in the big serif font.

use crate::game::CityId;
use crate::gfx::text::draw_text_centered;
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::Event;
use crate::rules::advance_ids;
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};

pub struct CityView {
    city: CityId,
    guard: u32,
}

pub fn city_view(c: CityId) -> Box<dyn Screen> {
    Box::new(CityView { city: c, guard: 2 })
}

/// Which CITYPIX sheet fits the owner's technology.
fn era_pic(ctx: &UiContext, city: CityId) -> &'static str {
    let g = ctx.game();
    let p = g.player(g.city(city).owner);
    if p.knows(advance_ids::INDUSTRIALIZATION) {
        "CITYPIX3"
    } else if p.knows(advance_ids::FEUDALISM) || p.knows(advance_ids::MONARCHY) {
        "CITYPIX2"
    } else {
        "CITYPIX1"
    }
}

/// Isometric lattice positions around the centre, nearest first.
fn lattice(n: usize) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    for r in 0..6i32 {
        for gx in -r..=r {
            for gy in -r..=r {
                if gx.abs().max(gy.abs()) == r {
                    v.push((gx, gy));
                }
            }
        }
    }
    v.truncate(n);
    v
}

impl Screen for CityView {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }

    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => Transition::Pop(DialogResult::Done),
            _ => Transition::None,
        }
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let hill = ctx.pic("HILL");
        fb.blit(&hill.surf, 0, 0);
        let g = ctx.game();
        let Some(c) = g.city_opt(self.city) else { return };
        let pix = ctx.pic(era_pic(ctx, self.city));
        // buildings: one hut per citizen (at least three), bigger houses for later ones
        let n = (c.size as usize * 2).clamp(3, 30);
        let mut spots: Vec<(i32, i32, usize)> = lattice(n)
            .into_iter()
            .enumerate()
            .map(|(i, (gx, gy))| {
                let px = 160 + (gx - gy) * 18 - 16;
                let py = 118 + (gx + gy) * 9 - 16;
                let hash = (i as u32).wrapping_mul(2654435761) ^ (self.city as u32).wrapping_mul(40503);
                let variant = (hash >> 7) % 3;
                let row = if i % 7 == 6 { 1 } else { 0 };
                // columns 0..9 of the sheet are huts/tents/cabins first, bigger houses later
                let col = (variant + (i as u32 / 9).min(2) * 3).min(9) as usize;
                (px, py, col + row * 10)
            })
            .collect();
        spots.sort_by_key(|s| s.1);
        for (px, py, cell) in spots {
            let col = (cell % 10) as i32;
            let row = (cell / 10) as i32;
            let src = Rect::new(col * 32 + 1, 1 + row * 32, 31, 31);
            fb.blit_part(&pix.surf, src, px, py, Some(0));
        }
        // a few trees from the sheet's props row (black background)
        let trees = [Rect::new(51, 116, 9, 9), Rect::new(66, 116, 5, 9), Rect::new(7, 122, 9, 8)];
        for (i, t) in trees.iter().enumerate() {
            let x = 236 + i as i32 * 14;
            fb.blit_part(&pix.surf, *t, x, 128 + (i as i32 % 2) * 14, Some(0));
            fb.blit_part(&pix.surf, *t, x - 160, 140 - (i as i32 % 2) * 10, Some(0));
        }
        let pal = ctx.pic_pal("HILL");
        let white = pal.nearest([255, 255, 255]);
        let black = pal.nearest([0, 0, 0]);
        let st = TextStyle::shadowed(5, white, black);
        draw_text_centered(fb, &ctx.fonts, &st, 160, 1, &c.name);
        draw_text_centered(fb, &ctx.fonts, &st, 160, 16, &g.year_text());
    }

    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.pic_pal("HILL")
    }

    fn draws_parent(&self) -> bool {
        false
    }

    fn wants_cursor(&self) -> bool {
        false
    }
}
