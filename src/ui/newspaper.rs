//! The newspaper (`newspaper(city)`, overlay 21, CIV.EXE o 0x1cb75): the top half of the screen
//! becomes a front page over the map, with the headline in font 4 and a masthead named after a
//! city, then "Press any key to continue." in a box at (80, 128).
//!
//! Layout (o 0x1cb75..0x1cf9e): (0, 0, 320, 100) colour 15; lines (1,1)-(318,1), (0,35)-(319,35),
//! (1,1)-(1,35), (318,1)-(318,35) and (0,97)-(319,97) in colour 0 around a colour-15 masthead
//! band (8, 3, 304, 20); 16 torn-edge sprites (SP257 (176, 128) 20x9, ds:0xdf0c) at
//! (20 i, 100 - random(2)); the headline as a text block at (16, 40); the masthead in font 5
//! centred at (160, 11): random(4) picks "<City> Weekly", "<City> Today", "The <City> Times" or
//! "The <City> Tribune", "<City> News" when wider than 300 px, with ",-." at (8, 11) and
//! (268, 11) when narrower than 200; "FLASH" (random(2) == 0) or "EXTRA!" in font 3 at (6, 3)
//! and (272, 3); "January 1, <year>" at (8, 28) and "10 cents" at (272, 28) in font 1. The
//! KING.TXT section *NEWSA.. it then draws does not exist in the shipped file.

use crate::game::rng::MsRand;
use crate::game::CityId;
use crate::gfx::text::{draw_text, draw_text_centered, text_width};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::Event;
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::MessageBox;

/// civrs font indexes of CIV.EXE window fonts 1, 3, 4 and 5.
const FONT1: usize = 0;
const FONT3: usize = 2;
const FONT4: usize = 3;
const FONT5: usize = 4;

pub struct Newspaper {
    headline: Vec<String>,
    /// The city the paper is named after (`city` < 0: the human's, see `paper_city`).
    city: Option<CityId>,
    masthead: String,
    flanks: bool,
    badge: &'static str,
    edge: [i32; 16],
    date: String,
    /// "Press any key to continue." (left out by newspaper(-2), which only draws the page)
    press: Option<MessageBox>,
}

/// The newspaper with `headline` (one string per line), named after `city`, or after the
/// human's city when `None` (the -1 argument).
pub fn newspaper(headline: Vec<String>, city: Option<CityId>) -> Box<dyn Screen> {
    let mut n = front_page(headline, city);
    n.press = Some(MessageBox::new(vec!["Press any key to continue.".into()]).with_pos(80, 128));
    Box::new(n)
}

/// newspaper(-2) (o 0x1cbc7, 0x1cf5a): only the front page, drawn over what is on screen and
/// returning at once, without the screen save, the "Press any key" box and its wait. The
/// caller draws the rest and waits itself (government_changed o 0x5d17).
pub fn front_page(headline: Vec<String>, city: Option<CityId>) -> Newspaper {
    Newspaper { headline, city, masthead: String::new(), flanks: false, badge: "EXTRA!", edge: [100; 16], date: String::new(), press: None }
}

/// The city of a newspaper(-1) (o 0x1cb7d..0x1cbbb): the first of the human's cities in table
/// order, then the last with a Palace after it.
pub fn paper_city(g: &crate::game::Game) -> Option<CityId> {
    let me = g.human_slot;
    let mut found = None;
    for c in g.all_cities().filter(|c| c.owner == me) {
        if found.is_none() || c.has_improvement(crate::rules::improvement_ids::PALACE) {
            found = Some(c.id);
        }
    }
    found
}

/// The city of a newspaper(-2) (same loop, o 0x1cbac): only a city with a Palace names it, the
/// last of the human's in table order. Without one CIV.EXE goes on with city -2 and prints
/// whatever that record holds; civrs falls back to `paper_city`.
pub fn palace_paper_city(g: &crate::game::Game) -> Option<CityId> {
    let me = g.human_slot;
    g.all_cities().filter(|c| c.owner == me && c.has_improvement(crate::rules::improvement_ids::PALACE)).map(|c| c.id).last().or_else(|| paper_city(g))
}

impl Screen for Newspaper {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        // the original draws on the game's random(); civrs keeps its own stream for the page
        let mut rng = MsRand::new(ctx.frame as u32);
        for y in self.edge.iter_mut() {
            *y = 100 - rng.random(2);
        }
        let name = match ctx.game.as_ref() {
            // a city that has just starved away still names the paper in CIV.EXE (its record
            // keeps the name); civrs falls back to the human's city
            Some(g) => self.city.filter(|c| g.city_opt(*c).is_some()).or_else(|| paper_city(g)).and_then(|c| g.city_opt(c)).map(|c| c.name.clone()).unwrap_or_default(),
            None => String::new(),
        };
        self.masthead = match rng.random(4) {
            0 => format!("{name} Weekly"),
            1 => format!("{name} Today"),
            2 => format!("The {name} Times"),
            _ => format!("The {name} Tribune"),
        };
        let mut w = text_width(&ctx.fonts, FONT5, &self.masthead);
        if w > 300 {
            self.masthead = format!("{name} News");
            w = text_width(&ctx.fonts, FONT5, &self.masthead);
        }
        self.flanks = w < 200;
        self.badge = if rng.random(2) == 0 { "FLASH" } else { "EXTRA!" };
        self.date = format!("January 1, {}", ctx.game.as_ref().map(|g| g.year_text()).unwrap_or_default());
        if let Some(p) = self.press.as_mut() {
            p.on_enter(ctx);
        }
    }
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        match self.press.as_mut() {
            Some(p) => p.update(ctx),
            None => Transition::Pop(DialogResult::Done),
        }
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        match self.press.as_mut().map(|p| p.handle(ev, ctx)) {
            Some(Transition::Pop(_)) | None => Transition::Pop(DialogResult::Done),
            Some(t) => t,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        fb.fill_rect(Rect::new(0, 0, 320, 100), 15);
        fb.line(1, 1, 318, 1, 0);
        fb.fill_rect(Rect::new(8, 3, 304, 20), 15);
        fb.line(0, 35, 319, 35, 0);
        fb.line(1, 1, 1, 35, 0);
        fb.line(318, 1, 318, 35, 0);
        fb.line(0, 97, 319, 97, 0);
        let sp = ctx.pic("SP257");
        for (i, y) in self.edge.iter().enumerate() {
            fb.blit_part(&sp.surf, Rect::new(176, 128, 20, 9), 20 * i as i32, *y, Some(0));
        }
        let lh = ctx.fonts.get(FONT4).line_height();
        let mut y = 40;
        for l in &self.headline {
            draw_text(fb, &ctx.fonts, &TextStyle::new(FONT4, 0), 16, y, l);
            y += lh;
        }
        if self.flanks {
            draw_text(fb, &ctx.fonts, &TextStyle::new(FONT5, 0), 8, 11, ",-.");
            draw_text(fb, &ctx.fonts, &TextStyle::new(FONT5, 0), 268, 11, ",-.");
        }
        draw_text_centered(fb, &ctx.fonts, &TextStyle::new(FONT5, 0), 160, 11, &self.masthead);
        draw_text(fb, &ctx.fonts, &TextStyle::new(FONT3, 0), 6, 3, self.badge);
        draw_text(fb, &ctx.fonts, &TextStyle::new(FONT3, 0), 272, 3, self.badge);
        draw_text(fb, &ctx.fonts, &TextStyle::new(FONT1, 0), 272, 28, "10 cents");
        draw_text(fb, &ctx.fonts, &TextStyle::new(FONT1, 0), 8, 28, &self.date);
        if let Some(p) = self.press.as_mut() {
            p.draw(fb, ctx);
        }
    }
}
