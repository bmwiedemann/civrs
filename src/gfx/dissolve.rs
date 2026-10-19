//! The MCGA driver's screen dissolve (MGRAPHIC.EXE entry 10; the original brings in the
//! title logo, the palace and the city-founding view with it). A 16-bit maximal linear
//! feedback shift register (taps 0xB400, period 65535) walks through every pixel offset
//! 0..63999 exactly once in a pseudo-random order; each step copies that pixel of the hidden
//! page to the screen. The caller passes a duration: the driver calibrates a delay loop so
//! that 8800 / duration pixels are copied per 70 Hz vertical retrace, i.e. a full screen takes
//! about duration * 0.104 s (20 = 2.1 s). Duration 0 runs at full speed.

use crate::gfx::{Surface, SCREEN_H, SCREEN_W};

/// Feedback taps of the register (x^16 + x^14 + x^13 + x^11 + 1).
pub const TAPS: u16 = 0xB400;
/// Pixels of a 320x200 page; register values above this are skipped.
pub const PIXELS: usize = (SCREEN_W * SCREEN_H) as usize;
/// The driver copies PIXELS_PER_TICK / duration pixels per retrace.
const PIXELS_PER_TICK: u32 = 8800;
/// Vertical retrace rate of MCGA mode 13h that the driver paces by.
const DRIVER_HZ: u32 = 70;
/// Our frame rate.
const FPS: u32 = 60;

/// The pixel order of the dissolve.
pub struct Dissolve {
    state: u16,
    copied: usize,
}

impl Default for Dissolve {
    fn default() -> Self {
        Self::new()
    }
}

impl Dissolve {
    pub fn new() -> Dissolve {
        Dissolve { state: 1, copied: 0 }
    }

    /// Offset of the next pixel to copy; None once every pixel has been copied.
    pub fn next_offset(&mut self) -> Option<usize> {
        if self.copied >= PIXELS {
            return None;
        }
        loop {
            let carry = self.state & 1 != 0;
            self.state >>= 1;
            if carry {
                self.state ^= TAPS;
            }
            if (self.state as usize) <= PIXELS {
                self.copied += 1;
                return Some(self.state as usize - 1);
            }
        }
    }

    pub fn finished(&self) -> bool {
        self.copied >= PIXELS
    }

    /// Copy the next `n` pixels of `src` into `dst` (both 320x200).
    pub fn copy(&mut self, dst: &mut Surface, src: &Surface, n: usize) {
        debug_assert!(dst.px.len() >= PIXELS && src.px.len() >= PIXELS);
        for _ in 0..n {
            let Some(o) = self.next_offset() else { return };
            dst.px[o] = src.px[o];
        }
    }
}

/// Pixels to copy per frame of ours for the driver duration `duration`.
pub fn pixels_per_frame(duration: u32) -> usize {
    if duration == 0 {
        return PIXELS;
    }
    let per_tick = (PIXELS_PER_TICK / duration).max(1);
    (per_tick * DRIVER_HZ).div_ceil(FPS) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pixel_once_in_the_drivers_order() {
        let mut d = Dissolve::new();
        // state 1 -> 0xB400 is the first value the driver copies, and 1 itself the last
        assert_eq!(d.next_offset(), Some(0xB400 - 1));
        assert_eq!(d.next_offset(), Some(0x5A00 - 1));
        let mut seen = vec![false; PIXELS];
        seen[0xB400 - 1] = true;
        seen[0x5A00 - 1] = true;
        let mut last = 0;
        while let Some(o) = d.next_offset() {
            assert!(!seen[o], "offset {o} twice");
            seen[o] = true;
            last = o;
        }
        assert!(seen.iter().all(|s| *s));
        assert_eq!(last, 0);
        assert!(d.finished());
    }

    #[test]
    fn duration_20_takes_about_two_seconds() {
        // 440 pixels per 70 Hz retrace -> 514 per 60 Hz frame -> 125 frames
        assert_eq!(pixels_per_frame(20), 514);
        assert_eq!(PIXELS.div_ceil(pixels_per_frame(20)), 125);
        assert_eq!(pixels_per_frame(0), PIXELS);
    }

    #[test]
    fn copy_moves_only_visited_pixels() {
        let mut d = Dissolve::new();
        let mut dst = Surface::screen();
        let mut src = Surface::screen();
        src.px.iter_mut().for_each(|p| *p = 7);
        d.copy(&mut dst, &src, 10);
        assert_eq!(dst.px.iter().filter(|p| **p == 7).count(), 10);
        d.copy(&mut dst, &src, PIXELS);
        assert!(dst.px.iter().all(|p| *p == 7));
    }
}
