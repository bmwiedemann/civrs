//! The frame loop: poll events, update the top screen, draw, present at 60 Hz.

use crate::assets::Palette;
use crate::gfx::Surface;
use crate::platform::{Backend, Event};
use crate::ui::context::UiContext;
use crate::ui::{Screen, ScreenStack};

pub struct App {
    pub ctx: UiContext,
    pub screens: ScreenStack,
    pub fb: Surface,
    /// Dump the framebuffer to this path after `dump_after` frames, then quit.
    pub dump: Option<(std::path::PathBuf, u64)>,
}

/// The mouse pointer: TORCH.PIC (13x14) with its hot spot at the top-left corner, the colour
/// of that corner transparent (CIV.EXE 0x1b87 loads it, 0:0x1697 installs it at offset 0,0).
pub fn draw_cursor(fb: &mut Surface, ctx: &UiContext, x: i32, y: i32) {
    let torch = ctx.pic("TORCH");
    let key = torch.surf.get(0, 0);
    fb.blit_part(&torch.surf, torch.surf.bounds(), x, y, Some(key));
}

impl App {
    pub fn new(ctx: UiContext, first: Box<dyn Screen>) -> App {
        let mut app = App { ctx, screens: ScreenStack::new(), fb: Surface::screen(), dump: None };
        app.screens.push(first, &mut app.ctx);
        app
    }

    /// Run one frame. Returns false when the application should exit.
    pub fn frame(&mut self, backend: &mut dyn Backend) -> bool {
        for ev in backend.poll() {
            match &ev {
                Event::Quit => {
                    self.ctx.quit_requested = true;
                }
                Event::MouseMove { x, y } => self.ctx.mouse = (*x, *y),
                Event::MouseButton { x, y, down, button: crate::platform::MouseButton::Left } => {
                    self.ctx.mouse = (*x, *y);
                    self.ctx.mouse_down = *down;
                }
                _ => {}
            }
            self.screens.handle(&ev, &mut self.ctx);
        }
        self.screens.update(&mut self.ctx);
        if let Some((x, y)) = self.ctx.warp_mouse.take() {
            backend.warp_mouse(x, y);
            self.ctx.mouse = (x, y);
        }
        if let Some(g) = &self.ctx.game {
            // the in-game Sound option gates every driver call, as in CIV.EXE
            if self.ctx.audio.available() && self.ctx.audio.enabled() != g.options.sound {
                self.ctx.audio.set_enabled(g.options.sound);
            }
        }
        if self.screens.is_empty() || self.screens.quit_requested() {
            return false;
        }
        self.screens.draw(&mut self.fb, &mut self.ctx);
        let pal = self.screens.palette(&self.ctx);
        self.ctx.palette = pal;
        if self.screens.wants_cursor() {
            let (mx, my) = self.ctx.mouse;
            draw_cursor(&mut self.fb, &self.ctx, mx, my);
        }
        backend.present(&self.fb, &pal);
        self.ctx.frame += 1;
        if let Some((path, after)) = &self.dump {
            if self.ctx.frame >= *after {
                if let Err(e) = self.fb.write_ppm(&pal, path) {
                    eprintln!("dump failed: {e}");
                }
                return false;
            }
        }
        backend.wait_frame();
        true
    }

    pub fn run(&mut self, backend: &mut dyn Backend) {
        while self.frame(backend) {}
    }

    pub fn palette(&self) -> Palette {
        self.ctx.palette
    }
}
