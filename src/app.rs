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

/// 8x11 arrow cursor, 1 = black outline, 2 = white fill.
const CURSOR: [&str; 11] = [
    "1.......", "11......", "121.....", "1221....", "12221...", "122221..", "1222221.", "12221111",
    "1211....", "11.1....", "1.......",
];

pub fn draw_cursor(fb: &mut Surface, x: i32, y: i32) {
    for (r, row) in CURSOR.iter().enumerate() {
        for (c, ch) in row.bytes().enumerate() {
            match ch {
                b'1' => fb.put(x + c as i32, y + r as i32, 0),
                b'2' => fb.put(x + c as i32, y + r as i32, 15),
                _ => {}
            }
        }
    }
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
        if self.screens.is_empty() || self.screens.quit_requested() {
            return false;
        }
        self.screens.draw(&mut self.fb, &mut self.ctx);
        let pal = self.screens.palette(&self.ctx);
        self.ctx.palette = pal;
        if self.screens.wants_cursor() {
            let (mx, my) = self.ctx.mouse;
            draw_cursor(&mut self.fb, mx, my);
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
