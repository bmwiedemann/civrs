//! "ROME founded: 4000 BC." animation: wagons rolling over the meadow, then the city screen.

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

const FRAMES: [Rect; 4] = [Rect::new(0, 0, 49, 16), Rect::new(0, 16, 49, 16), Rect::new(0, 32, 49, 16), Rect::new(0, 48, 49, 16)];

impl CityFounded {
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
        if self.t > 150 {
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
        let hill = ctx.pic("HILL");
        fb.blit(&hill.surf, 0, 0);
        let wag = ctx.pic("SETTLERS");
        let key = wag.surf.get(0, 70);
        let frame = FRAMES[((self.t / 6) % 4) as usize];
        let x = 60 + (self.t as i32 * 2) % 260;
        for i in 0..3 {
            fb.blit_part(&wag.surf, frame, x - i * 40, 120 + i * 8, Some(key));
        }
        let g = ctx.game();
        if let Some(c) = g.city_opt(self.city) {
            let text = format!("{} founded: {}.", c.name, crate::rules::year_text(crate::rules::year_for_turn(c.founded_turn)));
            let pal = ctx.pic_pal("HILL");
            let black = pal.nearest([0, 0, 0]);
            draw_text_centered(fb, &ctx.fonts, &TextStyle::new(3, black), 160, 2, &text);
        }
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
