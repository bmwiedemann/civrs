//! Software rendering into a 320x200 8-bit indexed framebuffer.

pub mod surface;
pub mod text;

pub use surface::{Rect, Surface, SCREEN_H, SCREEN_W};
pub use text::{draw_text, draw_text_centered, draw_text_right, TextStyle};
