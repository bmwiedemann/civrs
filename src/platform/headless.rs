//! Backend without a window, used for tests and screen dumps.

use super::{Backend, Event};
use crate::assets::Palette;
use crate::gfx::Surface;

pub struct HeadlessBackend {
    pub last_frame: Option<(Surface, Palette)>,
    pub queued: Vec<Event>,
    frames: u64,
}

impl Default for HeadlessBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadlessBackend {
    pub fn new() -> Self {
        HeadlessBackend { last_frame: None, queued: Vec::new(), frames: 0 }
    }
    pub fn push(&mut self, e: Event) {
        self.queued.push(e);
    }
    pub fn frames(&self) -> u64 {
        self.frames
    }
}

impl Backend for HeadlessBackend {
    fn poll(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.queued)
    }
    fn present(&mut self, fb: &Surface, pal: &Palette) {
        self.last_frame = Some((fb.clone(), *pal));
        self.frames += 1;
    }
    fn wait_frame(&mut self) {}
    fn now_ms(&self) -> u64 {
        self.frames * 1000 / 60
    }
}
