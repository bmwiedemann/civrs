//! "ROME founded: 4000 BC." animation (CIV.EXE overlay 0x1e424): four settlers' wagons roll
//! over the meadow, one pixel every 10 ticks, then the city screen.

use crate::game::CityId;
use crate::gfx::text::draw_text_centered;
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::Event;
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};

pub struct CityFounded {
    city: CityId,
    t: u64,
    done: bool,
}

pub fn city_founded(c: CityId) -> Box<dyn Screen> {
    Box::new(CityFounded { city: c, t: 0, done: false })
}

/// The four wagon frames of SETTLERS.PIC, inside the boxes drawn around them on the sheet.
const FRAMES: [Rect; 4] = [Rect::new(1, 1, 47, 15), Rect::new(1, 17, 47, 15), Rect::new(1, 33, 47, 15), Rect::new(1, 49, 47, 15)];
/// The wagons advance one pixel (and one frame) every 10 ticks, from x 80 to x 119.
const TICKS_PER_STEP: u64 = 10;
const FIRST_STEP: i32 = 80;
const STEPS: u64 = 40;

impl CityFounded {
    /// The meadow, the wagons at time `t` and the "founded" line.
    fn render(fb: &mut Surface, ctx: &UiContext, city: CityId, t: u64) {
        let hill = ctx.pic("HILL");
        fb.blit(&hill.surf, 0, 0);
        let wag = ctx.pic("SETTLERS");
        let key = wag.surf.get(0, 70);
        let step = FIRST_STEP + (t / TICKS_PER_STEP).min(STEPS - 1) as i32;
        for i in 0..4 {
            let frame = FRAMES[((i + step) & 3) as usize];
            fb.blit_part(&wag.surf, frame, step + 13 * i, 120 + ((10 * i) & 15), Some(key));
        }
        let g = ctx.game();
        if let Some(c) = g.city_opt(city) {
            let text = format!("{} founded: {}.", c.name, crate::rules::year_text(crate::rules::year_for_turn(c.founded_turn)));
            let pal = ctx.pic_pal("HILL");
            let black = pal.nearest([0, 0, 0]);
            draw_text_centered(fb, &ctx.fonts, &TextStyle::new(3, black), 160, 2, &text);
        }
    }

    fn finish(&mut self) -> Transition {
        if self.done {
            return Transition::None;
        }
        self.done = true;
        Transition::Replace(crate::ui::cityscreen::city_screen(self.city))
    }
}

impl Screen for CityFounded {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.t += 1;
        if self.t >= STEPS * TICKS_PER_STEP {
            return self.finish();
        }
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } if self.t > 10 => self.finish(),
            _ => Transition::None,
        }
    }
    fn resume(&mut self, _r: DialogResult, _ctx: &mut UiContext) -> Transition {
        Transition::Pop(DialogResult::Done)
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        Self::render(fb, ctx, self.city, self.t);
    }
    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.pic_pal("HILL")
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}
