//! CIVILIZATION HALL OF FAME: cream page listing the five best games.

use crate::assets::Palette;
use crate::gfx::Surface;
use crate::platform::{Event, Key};
use crate::ui::context::UiContext;
use crate::ui::reports::{col, page, text, text_c, BLACK, BROWN, CREAM};
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
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        page(fb, ctx, CREAM);
        text_c(fb, ctx, 160, 16, "CIVILIZATION", BLACK);
        text_c(fb, ctx, 160, 24, "HALL OF FAME", BLACK);
        fb.hline(80, 240, 32, col(ctx, [255, 255, 150]));
        for i in 0..5 {
            let y = 40 + 26 * i as i32;
            match ctx.hof.entries.get(i) {
                Some(e) => {
                    let diff = crate::rules::Difficulty::from_u8(e.difficulty).map(|d| d.name()).unwrap_or("Chieftain");
                    let line = if e.title.is_empty() {
                        format!("{}. {} of the {} to {}.", i + 1, e.leader, e.civ_plural, crate::rules::year_text(e.year))
                    } else {
                        format!("{}. {} {} of the {} to {}.", i + 1, e.title, e.leader, e.civ_plural, crate::rules::year_text(e.year))
                    };
                    text(fb, ctx, 8, y, &line, BLACK);
                    text(fb, ctx, 20, y + 8, &format!("Score: {} ({diff})", e.score), BROWN);
                    text_c(fb, ctx, 139, y + 16, &format!("--- CIVILIZATION RATING: {}% ---", e.percent), [255, 85, 85]);
                }
                None => {
                    text(fb, ctx, 14, y + 3, "---", BLACK);
                }
            }
        }
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.game_palette()
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}
