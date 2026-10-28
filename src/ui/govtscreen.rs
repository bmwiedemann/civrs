//! The government change (`government_changed`, CIV.EXE o 0x5d17): the top half of a
//! newspaper "ADJ government\nchanged to GOVT!" over a white lower half with "New Cabinet:"
//! and the four small advisor portraits of the new government's GOVTgX.PIC. The GOVT/BACK
//! sheet helpers are shared with the diplomacy screen's throne room.

use crate::assets::Palette;
use crate::gfx::text::draw_text_centered;
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::Event;
use crate::rules::Govt;
use crate::ui::context::UiContext;
use crate::ui::eventpics::{modern_era, Dismiss};
use crate::ui::newspaper::{front_page, palace_paper_city, Newspaper};
use crate::ui::screen::{DialogResult, Screen, Transition};

/// Picture group of a government: 0 despotism/anarchy, 1 monarchy, 2 republic/democracy,
/// 3 communism.
pub fn govt_group(g: Govt) -> u8 {
    match g {
        Govt::Anarchy | Govt::Despotism => 0,
        Govt::Monarchy => 1,
        Govt::Communism => 3,
        Govt::Republic | Govt::Democracy => 2,
    }
}

/// GOVTgX advisor sheet (communism has no modern variant).
pub fn govt_sheet(g: Govt, modern: bool) -> String {
    let grp = govt_group(g);
    let era = if modern && grp != 3 { 'M' } else { 'A' };
    format!("GOVT{grp}{era}")
}

/// BACKgX throne-room background.
pub fn back_sheet(g: Govt, modern: bool) -> String {
    let grp = govt_group(g);
    let era = if modern && grp != 3 { 'M' } else { 'A' };
    format!("BACK{grp}{era}")
}

/// Large advisor figure `i` (0..4) in a GOVT sheet (lower half, 79x99 cells).
pub fn advisor_rect(i: usize) -> Rect {
    Rect::new(i as i32 * 80 + 1, 101, 79, 99)
}

/// Text box of the throne-room backgrounds (inner area).
pub const THRONE_BOX: Rect = Rect::new(33, 138, 254, 58);

/// Draw advisor figures standing on the throne-room floor (feet at `floor_y`), outermost
/// first so the king can be drawn over the inner ones.
pub fn draw_advisors(fb: &mut Surface, ctx: &UiContext, sheet: &str, count: usize, floor_y: i32) {
    let pic = ctx.pic(sheet);
    const XS: [i32; 4] = [2, 239, 58, 183];
    for (i, x) in XS.iter().enumerate().take(count.min(4)) {
        let r = advisor_rect(i);
        let key = pic.surf.get(r.x, r.y);
        fb.blit_part(&pic.surf, r, *x, floor_y - r.h, Some(key));
    }
}

/// Throne-room palette: background with the advisor colours (16..63) of the GOVT sheet.
pub fn throne_palette(ctx: &UiContext, back: &str, govt_sheet: &str) -> Palette {
    ctx.pic_pal(back).merged(&ctx.pic_pal(govt_sheet), 16..=63)
}

/// Government names of the headline (ds:0x1966).
const HEADLINE_NAMES: [&str; 6] = ["Anarchy", "Despotism", "Monarchy", "Communist", "Republic", "Democratic"];

/// The cabinet's labels (ds:0x2fac), printed without their last character (the colon).
const CABINET: [&str; 4] = ["Defense Minister:", "Domestic Advisor:", "Foreign Minister:", "Science Advisor:"];

/// Small portrait `i` (0..4) of a GOVT sheet: the top-left 160x60 strip that
/// `load_govt_picture` (o 0x5c6b) keeps at (160, 140) of page 2, 40 px per advisor.
pub fn portrait_rect(i: usize) -> Rect {
    Rect::new(i as i32 * 40, 0, 40, 60)
}

pub struct GovtScreen {
    paper: Newspaper,
    sheet: String,
    dismiss: Dismiss,
}

/// government_changed (o 0x5d17), shown for the human's new government; nothing under Anarchy.
pub fn govt_screen(ctx: &UiContext, govt: Govt) -> Option<Box<dyn Screen>> {
    if govt == Govt::Anarchy {
        return None;
    }
    let (adj, city) = match ctx.game.as_ref() {
        Some(g) => (g.human().adjective().to_string(), palace_paper_city(g)),
        None => (String::new(), None),
    };
    let headline = vec![format!("{adj} government"), format!("changed to {}!", HEADLINE_NAMES[govt.as_u8() as usize])];
    Some(Box::new(GovtScreen { paper: front_page(headline, city), sheet: govt_sheet(govt, modern_era(ctx)), dismiss: Dismiss::new() }))
}

impl Screen for GovtScreen {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        self.paper.on_enter(ctx);
    }
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
    /// newspaper(-2) for the top half, then (o 0x5d96..0x5e72) the lower half filled with
    /// colour 15, "New Cabinet:" in font 6 centred at (160, 102), the portraits at
    /// (80 i + 20, 118) and their labels in font 2 centred at (80 i + 40, 180 or 186 for odd
    /// i), all in colour 0; then a key wait and the saved screen comes back.
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        self.paper.draw(fb, ctx);
        fb.fill_rect(Rect::new(0, 100, 320, 100), 15);
        draw_text_centered(fb, &ctx.fonts, &TextStyle::new(5, 0), 160, 102, "New Cabinet:");
        let pic = ctx.pic(&self.sheet);
        for (i, label) in CABINET.iter().enumerate() {
            let x = 80 * i as i32;
            fb.blit_part(&pic.surf, portrait_rect(i), x + 20, 118, None);
            draw_text_centered(fb, &ctx.fonts, &TextStyle::new(1, 0), x + 40, 180 + 6 * (i as i32 & 1), &label[..label.len() - 1]);
        }
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cabinet_headline_and_labels() {
        assert_eq!(HEADLINE_NAMES[Govt::Communism.as_u8() as usize], "Communist");
        assert_eq!(HEADLINE_NAMES[Govt::Democracy.as_u8() as usize], "Democratic");
        assert_eq!(portrait_rect(3), Rect::new(120, 0, 40, 60));
        assert_eq!(govt_sheet(Govt::Democracy, true), "GOVT2M");
        assert_eq!(govt_sheet(Govt::Communism, true), "GOVT3A");
    }
}
