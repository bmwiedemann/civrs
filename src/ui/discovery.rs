//! "Wise men discover the secret of ..." picture (choose_research, CIV.EXE 0xbcc0..0xbf5a):
//! DISCOVR1.PIC (ancient scholar in front of a blank board), DISCOVR2.PIC (modern scientist)
//! once the human knows Electricity (0xbce3: advance 0x36). The text is popup_menu3(text, 96,
//! 1) in font 6 (the modern one at (96, 25) in font 1), drawn without a key (ds:0xdb38 = 1)
//! and, the y odd, without a frame (0xbe2b..0xbe51); the advance's Civilopedia icon (110x68
//! from ICONPGn.PIC) goes on the board centred at (174, 96), the modern one at (174, 87)
//! (pedia_draw_icon o 0xc66c, called at 0xbef7). No name is written on the board. The
//! picture dissolves in over the map with the driver's pixel dissolve.

use crate::assets::Palette;
use crate::gfx::{Rect, Surface};
use crate::platform::Event;
use crate::rules::{self, AdvanceId};
use crate::ui::context::UiContext;
use crate::ui::eventpics::Dismiss;
use crate::ui::reveal::Reveal;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::MenuDialog;

/// Driver duration of the dissolve (the value the original uses for the title and palace).
const DISSOLVE: u32 = 20;

pub struct DiscoveryScreen {
    adv: AdvanceId,
    /// First caption line ("Roman wise men").
    civ: String,
    modern: bool,
    dismiss: Dismiss,
    reveal: Reveal,
}

pub fn discovery_screen(ctx: &UiContext, adv: AdvanceId) -> Box<dyn Screen> {
    // scientists once the human knows Invention (0xbd70: knows(human, 0x14)), the modern
    // picture once he knows Electricity (0xbce3)
    let (civ, modern) = ctx
        .game
        .as_ref()
        .map(|g| {
            let p = g.human();
            let who = if p.knows(rules::advance_ids::INVENTION) { "scientists" } else { "wise men" };
            (format!("{} {who}", p.adjective()), p.knows(rules::advance_ids::ELECTRICITY))
        })
        .unwrap_or_default();
    Box::new(DiscoveryScreen { adv, civ, modern, dismiss: Dismiss::new(), reveal: Reveal::new(ctx, DISSOLVE) })
}

impl DiscoveryScreen {
    fn pic_name(&self) -> &'static str {
        if self.modern {
            "DISCOVR2"
        } else {
            "DISCOVR1"
        }
    }

    /// The finished screen: picture, text and the advance's icon on the board.
    fn render(&self, fb: &mut Surface, ctx: &mut UiContext) {
        let pic = ctx.pic(self.pic_name());
        fb.blit(&pic.surf, 0, 0);
        let advance = rules::ADVANCES[self.adv as usize].name;
        // "ADJ wise men|scientists\ndiscover the secret\nof ADV!" (CIV.EXE 0xbd52..0xbdfa)
        let lines = vec![self.civ.clone(), "discover the secret".into(), format!("of {advance}!")];
        let (y, font) = if self.modern { (25, crate::ui::widgets::FONT) } else { (1, 5) };
        MenuDialog::new(lines, Vec::new()).with_pos(96, y).with_font(font).paint(fb, ctx);
        // the icon keeps its colour indices: the screen's palette is the icon page's
        if let Some((sheet, r)) = advance_icon(self.adv) {
            let icons = ctx.pic(sheet);
            let cy = if self.modern { 87 } else { 96 };
            fb.blit_part(&icons.surf, r, 174 - r.w / 2, cy - r.h / 2, None);
        }
    }

    /// In VGA the screen fades to ICONPGn.PAL with its first 85 colours (255 bytes, 0xbf3e
    /// memcpy) taken from the picture's palette (0xbf06..0xbf52), so the picture keeps its
    /// colours and the icon shows in its own.
    fn screen_palette(&self, ctx: &UiContext) -> Palette {
        let base = ctx.pic_pal(self.pic_name());
        let Some((sheet, _)) = advance_icon(self.adv) else { return base };
        let mut p = ctx.pic_pal(sheet);
        p.0[..85].copy_from_slice(&base.0[..85]);
        p
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
        let pal = self.screen_palette(ctx);
        if self.reveal.done() {
            self.render(fb, ctx);
        } else {
            // `fb` still holds the map; it dissolves into the picture
            let mut content = Surface::screen();
            self.render(&mut content, ctx);
            let old = ctx.palette;
            self.reveal.draw(fb, &content, &old, &pal);
        }
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        self.screen_palette(ctx)
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}
