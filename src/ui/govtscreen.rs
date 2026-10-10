//! Government change picture: the throne room of the new government (BACKgX.PIC) with the
//! four advisors of the GOVTgX.PIC sheet and the text "LEADER establishes a GOVERNMENT".
//! The sheet/background helpers are shared with the diplomacy screen.

use crate::assets::Palette;
use crate::gfx::{Rect, Surface};
use crate::platform::Event;
use crate::rules::Govt;
use crate::ui::context::UiContext;
use crate::ui::eventpics::{draw_caption_centred, modern_era, Dismiss};
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

pub struct GovtScreen {
    govt: Govt,
    modern: bool,
    lines: Vec<String>,
    dismiss: Dismiss,
}

pub fn govt_screen(ctx: &UiContext, govt: Govt) -> Box<dyn Screen> {
    let modern = modern_era(ctx);
    let leader = ctx.game.as_ref().map(|g| g.human().leader.clone()).unwrap_or_default();
    let name = govt.name().to_ascii_uppercase();
    let article = if name.starts_with('A') { "an" } else { "a" };
    let lines = vec![format!("{leader} establishes {article} {name}."), format!("Your government is now {article} {}.", govt.name())];
    Box::new(GovtScreen { govt, modern, lines, dismiss: Dismiss::new() })
}

impl Screen for GovtScreen {
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
        let back = back_sheet(self.govt, self.modern);
        let sheet = govt_sheet(self.govt, self.modern);
        let pic = ctx.pic(&back);
        fb.blit(&pic.surf, 0, 0);
        // the four advisors side by side in front of the throne-room box
        let adv = ctx.pic(&sheet);
        for i in 0..4 {
            let r = advisor_rect(i);
            let key = adv.surf.get(r.x, r.y);
            fb.blit_part(&adv.surf, r, 2 + i as i32 * 80, THRONE_BOX.y - r.h, Some(key));
        }
        let pal = self.palette(ctx);
        draw_caption_centred(fb, ctx, &pal, THRONE_BOX.y + 8, 240, &self.lines, 0);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        throne_palette(ctx, &back_sheet(self.govt, self.modern), &govt_sheet(self.govt, self.modern))
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}
