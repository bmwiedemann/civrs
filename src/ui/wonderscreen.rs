//! "CIV completes WONDER in CITY." picture: the wonder's picture from WONDERS.PIC /
//! WONDERS2.PIC enlarged on the city-view meadow (CBACK.PIC) with a caption.

use crate::assets::Palette;
use crate::gfx::{Rect, Surface};
use crate::platform::Event;
use crate::rules::{self, WonderId};
use crate::ui::civilopedia::wonder_picture;
use crate::ui::context::UiContext;
use crate::ui::eventpics::{draw_caption_centred, Dismiss};
use crate::ui::screen::{DialogResult, Screen, Transition};

/// Blit `sr` of `src` enlarged `scale` times at (`x`, `y`), skipping `key` pixels.
#[allow(clippy::too_many_arguments)]
pub fn blit_scaled(fb: &mut Surface, src: &Surface, sr: Rect, x: i32, y: i32, scale: i32, key: Option<u8>, map: &[u8; 256]) {
    for sy in 0..sr.h {
        for sx in 0..sr.w {
            let c = src.get(sr.x + sx, sr.y + sy);
            if Some(c) == key {
                continue;
            }
            let c = map[c as usize];
            for dy in 0..scale {
                for dx in 0..scale {
                    fb.put(x + sx * scale + dx, y + sy * scale + dy, c);
                }
            }
        }
    }
}

/// A pyramid with its apex over `cx`, base line at `base`, `h` high (lit left face).
fn draw_pyramid(fb: &mut Surface, cx: i32, base: i32, h: i32, light: u8, dark: u8) {
    let half = h * 3 / 4;
    for dy in 0..h {
        let w = half * dy / h;
        let y = base - h + dy;
        fb.hline(cx - w, cx, y, light);
        fb.hline(cx + 1, cx + w * 3 / 4, y, dark);
    }
}

pub struct WonderScreen {
    wonder: WonderId,
    lines: Vec<String>,
    dismiss: Dismiss,
}

pub fn wonder_screen(ctx: &UiContext, wonder: WonderId, city: &str, owner: u8) -> Box<dyn Screen> {
    let name = rules::WONDERS[wonder as usize].name;
    let lines = match ctx.game.as_ref() {
        Some(g) => vec![format!("{} complete {name}", g.civ_name(owner)), format!("in {city}.")],
        None => vec![format!("{name} completed in {city}.")],
    };
    Box::new(WonderScreen { wonder, lines, dismiss: Dismiss::new() })
}

impl Screen for WonderScreen {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.dismiss.tick();
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.dismiss.closes(ev) {
            Transition::Pop(DialogResult::Done)
        } else {
            Transition::None
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let (sheet, r) = wonder_picture(self.wonder);
        let pal = ctx.pic_pal(sheet);
        // meadow backdrop (HILL shares most of the WONDERS palette) remapped into it
        let back = ctx.pic("HILL");
        let back_pal = ctx.pic_pal("HILL");
        let mut map = [0u8; 256];
        for (i, m) in map.iter_mut().enumerate() {
            *m = pal.nearest(back_pal.rgb(i as u8));
        }
        fb.blit_remap(&back.surf, back.surf.bounds(), 0, 0, None, &map);
        let pic = ctx.pic(sheet);
        let key = pic.surf.get(0, 100);
        let scale = if r.w * 2 <= 300 && r.h * 2 <= 150 { 2 } else { 1 };
        let x = (320 - r.w * scale) / 2;
        let y = (180 - r.h * scale).max(24);
        let ident: [u8; 256] = std::array::from_fn(|i| i as u8);
        blit_scaled(fb, &pic.surf, r, x, y, scale, Some(key), &ident);
        if self.wonder == rules::wonder_ids::PYRAMIDS {
            // WONDERS2 only holds the desert plateau: raise three pyramids on it with its
            // own light / shadow colours
            let light = pic.surf.get(r.x + r.w / 2, r.y + r.h / 2);
            let dark = pic.surf.get(r.right() - 3, r.y + r.h / 2);
            for (cx, base, h) in [(x + 60, y + 6, 70), (x + 150, y + 10, 95), (x + 125, y + 22, 40)] {
                draw_pyramid(fb, cx, base, h, light, dark);
            }
        }
        draw_caption_centred(fb, ctx, &pal, 4, 240, &self.lines, 0);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.pic_pal(wonder_picture(self.wonder).0)
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}
