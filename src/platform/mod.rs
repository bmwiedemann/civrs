//! Window, input and timing abstraction. The game renders into a 320x200 indexed surface;
//! the backend converts it to RGBA and presents it scaled by an integer factor with
//! nearest-neighbour filtering.

pub mod headless;
pub mod sdl;

use crate::assets::Palette;
use crate::gfx::Surface;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    /// Numeric keypad digit (0-9), reported regardless of NumLock.
    Kp(u8),
    Enter,
    Esc,
    Space,
    Backspace,
    Tab,
    Delete,
    Insert,
    /// Function key F1..F12.
    F(u8),
    /// A printable character (lower-case for letters; Shift is reported via `Mods`).
    Char(char),
    Plus,
    Minus,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Key { key: Key, mods: Mods, down: bool, repeat: bool },
    /// Typed text (already shifted/localised), one char at a time.
    Text(char),
    /// Mouse position in 320x200 screen space (may be outside when in the letterbox).
    MouseMove { x: i32, y: i32 },
    MouseButton { button: MouseButton, x: i32, y: i32, down: bool },
    Quit,
}

impl Event {
    /// Convenience: a key press (not release, not repeat) of `k`.
    pub fn is_press(&self, k: Key) -> bool {
        matches!(self, Event::Key { key, down: true, .. } if *key == k)
    }
    /// A lower-case character key press (ignores shift state).
    pub fn char_press(&self) -> Option<(char, Mods)> {
        match self {
            Event::Key { key: Key::Char(c), mods, down: true, .. } => Some((*c, *mods)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PlatformOptions {
    pub scale: Option<u32>,
    pub fullscreen: bool,
    pub title: String,
}

impl Default for PlatformOptions {
    fn default() -> Self {
        PlatformOptions { scale: None, fullscreen: false, title: "Civilization".to_string() }
    }
}

pub trait Backend {
    fn poll(&mut self) -> Vec<Event>;
    fn present(&mut self, fb: &Surface, pal: &Palette);
    /// Sleep until the next 60 Hz frame boundary (no-op when headless).
    fn wait_frame(&mut self);
    fn now_ms(&self) -> u64;
    fn scale(&self) -> u32 {
        1
    }
    fn set_fullscreen(&mut self, _on: bool) {}
    fn is_fullscreen(&self) -> bool {
        false
    }
}

/// Largest integer scale so that the 320x200 image fits comfortably in a display of the
/// given size (leaving room for window decorations).
pub fn pick_scale(display_w: u32, display_h: u32) -> u32 {
    let kw = (display_w * 9 / 10) / 320;
    let kh = (display_h * 9 / 10) / 200;
    kw.min(kh).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scale_pick() {
        assert_eq!(pick_scale(1920, 1080), 4);
        assert_eq!(pick_scale(1280, 800), 3);
        assert_eq!(pick_scale(640, 480), 1);
        assert_eq!(pick_scale(3840, 2160), 9);
    }
}
