//! Leader introduction as the end of overlay 5's `new_game_setup` (o 0x47cc..0x4937) shows it:
//! a white page (colour 15) with the random SP299 page border (0x7e6:0x260a), the difficulty
//! portrait from DIFFS.PIC at (134, 20), and KING.TXT `*INIT` left-aligned at (88, 81) in font 1
//! (civrs 0), colour 0. The known advances follow in advance order, with a line break after the
//! second, then "and Roads.".

use crate::assets::text::{paragraphs, substitute, Vars};
use crate::gfx::text::draw_text;
use crate::gfx::{Surface, TextStyle};
use crate::platform::Event;
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};

const TEXT_X: i32 = 88;
const TEXT_Y: i32 = 81;
const LINE_H: i32 = 8;
const PAGE: u8 = 15;

struct LeaderIntro {
    lines: Vec<String>,
    border: u8,
    guard: u32,
}

pub fn leader_intro() -> Box<dyn Screen> {
    Box::new(LeaderIntro { lines: Vec::new(), border: 0, guard: 10 })
}

/// The text appended to `*INIT` (o 0x4826): every known advance in id order followed by ", ",
/// a line break after the second, then "and Roads.".
fn advance_lines(names: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for (i, n) in names.iter().enumerate() {
        line.push_str(n);
        line.push_str(", ");
        if i == 1 {
            out.push(std::mem::take(&mut line));
        }
    }
    out.push(format!("{line}and Roads."));
    out
}

impl Screen for LeaderIntro {
    fn on_exit(&mut self, ctx: &mut UiContext) {
        ctx.audio.fade_out();
    }
    fn on_enter(&mut self, ctx: &mut UiContext) {
        ctx.audio.national_theme(ctx.game().human().civ, true);
        // random(2) in the original; the frame counter keeps headless renders repeatable
        self.border = (ctx.frame.wrapping_mul(0x9e37_79b9) >> 13) as u8 & 1;
        let g = ctx.game();
        let p = g.human();
        let v = Vars { rplc1: p.leader.clone(), us: p.plural().to_string(), ..Default::default() };
        let lines = ctx.texts.king.section("INIT").map(|l| l.to_vec()).unwrap_or_default();
        let mut body = paragraphs(&substitute(&lines, &v));
        while body.last().is_some_and(|l| l.is_empty()) {
            body.pop();
        }
        let names: Vec<&str> = p.known_advances().iter().map(|a| crate::rules::ADVANCES[*a as usize].name).collect();
        body.extend(advance_lines(&names));
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
        fb.fill(PAGE);
        crate::ui::pedia_entry::draw_border(fb, ctx, self.border);
        let diffs = ctx.pic("DIFFS");
        fb.blit_part(&diffs.surf, crate::ui::setup::portrait(ctx.game().setup.difficulty as usize), 134, 20, None);
        let st = TextStyle::new(0, 0);
        for (i, l) in self.lines.iter().enumerate() {
            draw_text(fb, &ctx.fonts, &st, TEXT_X, TEXT_Y + LINE_H * i as i32, l);
        }
    }

    /// The palette DIFFS.PIC set for the difficulty screen.
    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.pic_pal("DIFFS")
    }

    fn draws_parent(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_list_breaks_after_the_second() {
        assert_eq!(advance_lines(&[]), vec!["and Roads."]);
        assert_eq!(advance_lines(&["Alphabet"]), vec!["Alphabet, and Roads."]);
        assert_eq!(advance_lines(&["Bronze Working", "Ceremonial Burial"]), vec!["Bronze Working, Ceremonial Burial, ", "and Roads."]);
        assert_eq!(advance_lines(&["Alphabet", "Bronze Working", "Pottery"]), vec!["Alphabet, Bronze Working, ", "Pottery, and Roads."]);
    }
}
