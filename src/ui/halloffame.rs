//! CIVILIZATION HALL OF FAME: cream page listing the five best games.

use crate::assets::Palette;
use crate::gfx::Surface;
use crate::platform::{Event, Key};
use crate::ui::context::UiContext;
use crate::gfx::text::{draw_text, draw_text_centered, text_width};
use crate::gfx::TextStyle;
use crate::ui::reports::{commas, FONT};
use crate::ui::screen::{DialogResult, Screen, Transition};

pub fn hall_of_fame() -> Box<dyn Screen> {
    Box::new(HallOfFame { guard: 2 })
}

pub struct HallOfFame {
    guard: u32,
}

impl Screen for HallOfFame {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        match ev {
            Event::Key { down: true, key, .. } if !matches!(key, Key::Unknown) => Transition::Pop(DialogResult::Done),
            Event::MouseButton { down: true, .. } => Transition::Pop(DialogResult::Done),
            _ => Transition::None,
        }
    }
    /// `fame_show(-1)` (ovl 3 +0xd7): a colour-15 page, "CIVILIZATION" / "HALL OF FAME" centred
    /// at y 16 and 24, a colour-14 rule (80, 32)..(240, 32), then five entries 26 px apart
    /// from y 40: "N. <leader>, <difficulty title> of the <civ> to <year> BC." at (8, y) in 0
    /// (cut after the civilization when it would pass x 320), "Population: N, Score: N
    /// (<level>)" at (20, y + 8) in 7 and "    --- CIVILIZATION RATING: N% ---" at (20, y + 16)
    /// in 12; an empty slot is " ---" at (8, y).
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        fb.fill(15);
        let st = |c: u8| TextStyle::new(FONT, c);
        for (y, t) in [(16, "CIVILIZATION"), (24, "HALL OF FAME")] {
            draw_text_centered(fb, &ctx.fonts, &st(0), 160, y, t);
        }
        fb.line(80, 32, 240, 32, 14);
        let mut y = 40;
        for i in 0..5 {
            match ctx.hof.entries.get(i) {
                Some(e) => {
                    let d = e.difficulty.min(4) as usize;
                    let era = if e.year < 0 { "BC" } else { "AD" };
                    let head = format!("{}. {}, {} of the {}", i + 1, e.leader, crate::rules::DIFFICULTY[d].short, e.civ_plural);
                    let full = format!("{head} to {} {era}.", e.year.abs());
                    let line = if text_width(&ctx.fonts, FONT, &full) + 8 >= 320 { head } else { full };
                    draw_text(fb, &ctx.fonts, &st(0), 8, y, &line);
                    let level = ["Chieftan", "Warlord", "Prince", "King", "Emperor"][d];
                    draw_text(fb, &ctx.fonts, &st(7), 20, y + 8, &format!("Population: {}, Score: {} ({level})", commas(e.population), e.score));
                    let rating = (d as i32 + 1) * e.score / 50;
                    draw_text(fb, &ctx.fonts, &st(12), 20, y + 16, &format!("    --- CIVILIZATION RATING: {rating}% ---"));
                    y += 26;
                }
                None => {
                    draw_text(fb, &ctx.fonts, &st(0), 8, y, " ---");
                    y += 26;
                }
            }
        }
    }
    /// From the title menu the screen keeps LOGO.PIC's palette (cream 15, brown 7, red 12).
    fn palette(&self, ctx: &UiContext) -> Palette {
        if ctx.game.is_some() {
            ctx.game_palette()
        } else {
            ctx.pic("LOGO").pal
        }
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}
