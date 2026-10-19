//! "Wise men discover the secret of ..." picture: DISCOVR1.PIC (ancient scholar in front of a
//! blank board) before Invention, DISCOVR2.PIC (modern scientist) afterwards. The advance's
//! name (and its Civilopedia icon when available) is drawn inside the board. The picture
//! dissolves in over the map with the driver's pixel dissolve.

use crate::assets::Palette;
use crate::gfx::text::{draw_text_centered, text_width};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::Event;
use crate::rules::{self, AdvanceId};
use crate::ui::context::UiContext;
use crate::ui::eventpics::{draw_caption_centred, modern_era, Dismiss};
use crate::ui::reveal::Reveal;
use crate::ui::screen::{DialogResult, Screen, Transition};

/// Blank board rectangles measured in the pictures.
const BOARD_ANCIENT: Rect = Rect::new(119, 61, 110, 68);
const BOARD_MODERN: Rect = Rect::new(83, 29, 174, 92);
/// Driver duration of the dissolve (the value the original uses for the title and palace).
const DISSOLVE: u32 = 20;

pub struct DiscoveryScreen {
    adv: AdvanceId,
    /// Civilisation adjective ("Roman").
    civ: String,
    modern: bool,
    dismiss: Dismiss,
    reveal: Reveal,
}

pub fn discovery_screen(ctx: &UiContext, adv: AdvanceId) -> Box<dyn Screen> {
    let civ = ctx.game.as_ref().map(|g| g.human().adjective().to_string()).unwrap_or_default();
    Box::new(DiscoveryScreen { adv, civ, modern: modern_era(ctx), dismiss: Dismiss::new(), reveal: Reveal::new(ctx, DISSOLVE) })
}

impl DiscoveryScreen {
    fn pic_name(&self) -> &'static str {
        if self.modern {
            "DISCOVR2"
        } else {
            "DISCOVR1"
        }
    }

    /// The finished screen: picture, icon, advance name and caption.
    fn render(&self, fb: &mut Surface, ctx: &UiContext, pal: &Palette) {
        let pic = ctx.pic(self.pic_name());
        fb.blit(&pic.surf, 0, 0);
        let board = if self.modern { BOARD_MODERN } else { BOARD_ANCIENT };
        let advance = rules::ADVANCES[self.adv as usize].name;
        let ink = pal.nearest([0, 0, 0]);
        let mut text_y = board.y + (board.h - 10) / 2;
        if let Some((sheet, r)) = advance_icon(self.adv) {
            let icons = ctx.pic(sheet);
            let icon_pal = ctx.pic_pal(sheet);
            let mut map = [0u8; 256];
            for (i, m) in map.iter_mut().enumerate() {
                *m = pal.nearest(icon_pal.rgb(i as u8));
            }
            let avail_h = board.h - 14;
            let r = Rect::new(r.x, r.y, r.w.min(board.w - 4), r.h.min(avail_h));
            let ix = board.x + (board.w - r.w) / 2;
            let iy = board.y + 2;
            fb.blit_remap(&icons.surf, r, ix, iy, None, &map);
            text_y = iy + r.h + 2;
        }
        // advance name in the board (bold serif if it fits, else the small font)
        let font = if text_width(&ctx.fonts, 5, advance) <= board.w - 8 { 5 } else { 0 };
        draw_text_centered(fb, &ctx.fonts, &TextStyle::new(font, ink), board.x + board.w / 2, text_y, advance);
        let lines = vec![format!("{} wise men discover", self.civ), format!("the secret of {advance}.")];
        let y = if self.modern { 130 } else { 2 };
        draw_caption_centred(fb, ctx, pal, y, 240, &lines, 0);
    }
}

/// Civilopedia icon of an advance (sheet name + rectangle), when the Civilopedia provides one.
fn advance_icon(adv: AdvanceId) -> Option<(&'static str, Rect)> {
    crate::ui::civilopedia::advance_icon(adv)
}

impl Screen for DiscoveryScreen {
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
            // a key during the dissolve completes it instead of closing the screen
            self.reveal.finish();
            Transition::None
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let pal = ctx.pic_pal(self.pic_name());
        if self.reveal.done() {
            self.render(fb, ctx, &pal);
        } else {
            // `fb` still holds the map; it dissolves into the picture
            let mut content = Surface::screen();
            self.render(&mut content, ctx, &pal);
            let old = ctx.palette;
            self.reveal.draw(fb, &content, &old, &pal);
        }
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.pic_pal(self.pic_name())
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}
