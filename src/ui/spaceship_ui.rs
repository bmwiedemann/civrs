//! Spaceship arrival: PLANET1.PIC starfield with the rotating planet (PLANET2.PIC frames),
//! then the colony (SPACEST.PIC) with the KING.TXT SPACE (winner) / SPACE2 (rival) text.

use crate::assets::text::{paragraphs, substitute, Vars};
use crate::assets::Palette;
use crate::game::Slot;
use crate::gfx::text::draw_text;
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::Event;
use crate::ui::context::UiContext;
use crate::ui::eventpics::{caption_colours, draw_caption_centred, Dismiss};
use crate::ui::screen::{DialogResult, Screen, Transition};

/// PLANET2.PIC: 6x4 frames of 49x49 in 50-px cells.
fn planet_frame(i: i32) -> Rect {
    Rect::new((i % 6) * 50 + 1, (i / 6) * 50 + 1, 49, 49)
}

#[derive(PartialEq)]
enum Phase {
    Approach,
    Colony,
}

pub struct ArrivalScreen {
    slot: Slot,
    phase: Phase,
    t: u64,
    title: Vec<String>,
    story: Vec<String>,
    dismiss: Dismiss,
}

pub fn arrival_screen(ctx: &UiContext, slot: Slot) -> Box<dyn Screen> {
    let mut title = vec!["A spaceship reaches Alpha Centauri!".to_string()];
    let mut story = Vec::new();
    if let Some(g) = ctx.game.as_ref() {
        let me = g.human_slot;
        title = vec![format!("The {} spaceship reaches Alpha Centauri!", g.civ_name(slot))];
        let s = g.player(slot).spaceship;
        // launch year ds:0xe8a8, arrival year ds:0xdc10, colonists ds:0xdeec (overlay 2 0x174c)
        let launch = s.launched_turn.map(crate::rules::year_for_turn).unwrap_or_else(|| g.year());
        let arrive = s.arrival_year.unwrap_or_else(|| g.year());
        let habitation = g.spaceship_stats(slot).habitation();
        let v = Vars {
            rplc1: crate::rules::year_text(launch),
            rplc2: crate::rules::year_text(arrive),
            us: g.human().leader.clone(),
            them: g.player(slot).adjective().to_string(),
            bucks: habitation.max(1),
        };
        let section = if slot == me { "SPACE" } else { "SPACE2" };
        let lines = ctx.texts.king.section(section).map(|l| l.to_vec()).unwrap_or_default();
        story = paragraphs(&substitute(&lines, &v)).into_iter().filter(|l| !l.is_empty()).collect();
    }
    Box::new(ArrivalScreen { slot, phase: Phase::Approach, t: 0, title, story, dismiss: Dismiss::new() })
}

impl Screen for ArrivalScreen {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.t += 1;
        self.dismiss.tick();
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if !self.dismiss.closes(ev) {
            return Transition::None;
        }
        match self.phase {
            Phase::Approach => {
                self.phase = Phase::Colony;
                self.dismiss = Dismiss::new();
                Transition::None
            }
            Phase::Colony => Transition::Pop(DialogResult::Done),
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        match self.phase {
            Phase::Approach => {
                let bg = ctx.pic("PLANET1");
                fb.blit(&bg.surf, 0, 0);
                let pal = ctx.pic_pal("PLANET1");
                let planet = ctx.pic("PLANET2");
                let ppal = ctx.pic_pal("PLANET2");
                let mut map = [0u8; 256];
                for (i, m) in map.iter_mut().enumerate() {
                    *m = pal.nearest(ppal.rgb(i as u8));
                }
                let f = planet_frame(((self.t / 4) % 24) as i32);
                let key = planet.surf.get(f.x, f.y);
                // the planet grows as the ship approaches
                let scale = if self.t < 120 { 1 } else { 2 };
                crate::ui::wonderscreen::blit_scaled(fb, &planet.surf, f, 160 - 25 * scale, 95 - 25 * scale, scale, Some(key), &map);
                draw_caption_centred(fb, ctx, &pal, 170, 260, &self.title, 0);
            }
            Phase::Colony => {
                let bg = ctx.pic("SPACEST");
                fb.blit(&bg.surf, 0, 0);
                let pal = ctx.pic_pal("SPACEST");
                let cols = caption_colours(&pal);
                let lh = ctx.fonts.get(1).line_height().max(7);
                let h = 6 + self.story.len() as i32 * lh;
                let r = Rect::new(4, 4, 150, h.min(192));
                fb.fill_rect(r, cols.fill);
                fb.rect_outline(r, cols.border);
                let mut y = r.y + 3;
                for l in &self.story {
                    if y + lh > r.bottom() {
                        break;
                    }
                    draw_text(fb, &ctx.fonts, &TextStyle::new(1, cols.text), r.x + 4, y, &l.to_ascii_uppercase());
                    y += lh;
                }
                let _ = self.slot;
            }
        }
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        match self.phase {
            Phase::Approach => ctx.pic_pal("PLANET1"),
            Phase::Colony => ctx.pic_pal("SPACEST"),
        }
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}
