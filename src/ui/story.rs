//! "In the beginning..." evolution sequence: one caption line at a time over the BIRTH pictures.

use crate::gfx::text::draw_text_centered;
use crate::gfx::{Surface, TextStyle};
use crate::platform::Event;
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};

pub struct StoryScreen {
    /// Flattened (picture index, caption) list.
    captions: Vec<(usize, String)>,
    idx: usize,
    t: u64,
    fade: u8,
}

pub fn story_screen() -> Box<dyn Screen> {
    Box::new(StoryScreen { captions: Vec::new(), idx: 0, t: 0, fade: 0 })
}

const CAPTION_FRAMES: u64 = 80;

impl StoryScreen {
    fn pic_name(&self) -> String {
        let p = self.captions.get(self.idx).map(|c| c.0).unwrap_or(8);
        format!("BIRTH{}", p.min(8))
    }

    fn finish(&self) -> Transition {
        Transition::Replace(crate::ui::intro_message::leader_intro())
    }
}

impl Screen for StoryScreen {
    fn on_exit(&mut self, ctx: &mut UiContext) {
        ctx.audio.fade_out();
    }
    fn on_enter(&mut self, ctx: &mut UiContext) {
        ctx.audio.fade_out();
        ctx.audio.play(crate::audio::Sound::Evolution);
        // 8 story screens over 9 pictures: screen k uses picture k, last caption uses BIRTH8
        let n = ctx.texts.story.len();
        for (k, screen) in ctx.texts.story.iter().enumerate() {
            for (j, line) in screen.iter().enumerate() {
                let pic = if k + 1 == n { 8 } else { k.min(7) };
                let _ = j;
                self.captions.push((pic, line.trim().to_string()));
            }
        }
        if self.captions.is_empty() {
            self.captions.push((0, "In the beginning,".into()));
        }
    }

    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.t += 1;
        if self.fade < 255 {
            self.fade = self.fade.saturating_add(8);
        }
        if self.t >= CAPTION_FRAMES {
            self.t = 0;
            let old_pic = self.captions[self.idx].0;
            self.idx += 1;
            if self.idx >= self.captions.len() {
                return self.finish();
            }
            if self.captions[self.idx].0 != old_pic {
                self.fade = 0;
            }
        }
        Transition::None
    }

    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => self.finish(),
            _ => Transition::None,
        }
    }

    fn resume(&mut self, _r: DialogResult, _ctx: &mut UiContext) -> Transition {
        self.finish()
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let name = self.pic_name();
        let p = ctx.pic(&name);
        fb.blit(&p.surf, 0, 0);
        if let Some((_, line)) = self.captions.get(self.idx) {
            let pal = ctx.pic_pal(&name);
            let cyan = pal.nearest([12, 227, 235]);
            draw_text_centered(fb, &ctx.fonts, &TextStyle::new(0, cyan), 160, 180, line);
        }
    }

    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.pic_pal(&self.pic_name()).dimmed(self.fade)
    }

    fn draws_parent(&self) -> bool {
        false
    }

    fn wants_cursor(&self) -> bool {
        false
    }
}
