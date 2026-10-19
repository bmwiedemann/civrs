//! A screen that dissolves in with the driver's pixel dissolve (see `gfx::dissolve`): the
//! frame that was on the screen when the new one appeared stays, and every frame a further
//! batch of its pixels is replaced by the new content in the shift-register order. The
//! content may animate meanwhile: revealed pixels always show its current state.

use crate::assets::Palette;
use crate::gfx::dissolve::{pixels_per_frame, Dissolve, PIXELS};
use crate::gfx::Surface;
use crate::ui::context::UiContext;

pub struct Reveal {
    order: Dissolve,
    per_frame: usize,
    /// The frame that was on the screen before; None before the first draw.
    old: Option<Surface>,
    /// Pixels already replaced by the new content.
    revealed: Vec<bool>,
}

impl Reveal {
    /// A dissolve taking the driver's `duration` (about 0.104 s per unit); instant when the
    /// context asks for no timed transitions (headless previews).
    pub fn new(ctx: &UiContext, duration: u32) -> Reveal {
        let per_frame = if ctx.instant { PIXELS } else { pixels_per_frame(duration) };
        Reveal { order: Dissolve::new(), per_frame, old: None, revealed: vec![false; PIXELS] }
    }

    pub fn done(&self) -> bool {
        self.order.finished()
    }

    /// Complete the dissolve on the next draw.
    pub fn finish(&mut self) {
        self.per_frame = PIXELS;
    }

    /// Draw `content` into `fb` through the dissolve. On the first call `fb` must still hold
    /// the previous frame, shown with palette `old`; `new` is the palette of the content (the
    /// old frame is remapped to it so its colours survive the palette switch).
    pub fn draw(&mut self, fb: &mut Surface, content: &Surface, old: &Palette, new: &Palette) {
        if self.done() {
            fb.blit(content, 0, 0);
            return;
        }
        let before = self.old.get_or_insert_with(|| {
            let mut s = fb.clone();
            if old != new {
                let mut map = [0u8; 256];
                for (i, m) in map.iter_mut().enumerate() {
                    *m = new.nearest(old.rgb(i as u8));
                }
                for p in s.px.iter_mut() {
                    *p = map[*p as usize];
                }
            }
            s
        });
        for _ in 0..self.per_frame {
            let Some(o) = self.order.next_offset() else { break };
            self.revealed[o] = true;
        }
        for (i, p) in fb.px.iter_mut().enumerate().take(PIXELS) {
            *p = if self.revealed[i] { content.px[i] } else { before.px[i] };
        }
    }
}
