//! Turns the driver's register writes into PCM: an OPL2 (emulated by `nuked-opl3`, which
//! is bit-exact with the YMF262 in its OPL2-compatible mode) clocked by a 300 Hz sub-tick,
//! with `sound_worker` on every fifth sub-tick (60 Hz) and `fast_worker` on every one while
//! the driver asks for it, which is how CIV.EXE drives the real driver from its PIT interrupt.

use super::driver::Driver;
use nuked_opl3::Opl3Chip;

/// Sub-ticks per second (the game's PIT rate); `sound_worker` runs on every fifth one.
const SUB_RATE: u32 = 300;

pub struct Engine {
    pub driver: Driver,
    chip: Opl3Chip,
    rate: u32,
    /// Fractional sample accumulator for the sub-tick clock, in 1/300 s units.
    acc: u32,
    /// Samples left until the next sub-tick.
    until_sub: usize,
    /// Sub-tick phase 0..4; `sound_worker` runs when it wraps.
    phase: u8,
    /// `fast_worker` requested by the last non-zero `sound_worker` result.
    fast: bool,
    /// Sound option off: `play` is ignored (CIV.EXE skips the driver call).
    pub enabled: bool,
    /// Output gain, 1.0 = the chip's own level.
    pub gain: f32,
}

impl Engine {
    pub fn new(driver: Driver, rate: u32) -> Engine {
        let mut chip = Opl3Chip::new(rate);
        let mut driver = driver;
        for (r, v) in driver.take_writes() {
            chip.write_register(r as u16, v);
        }
        Engine { driver, chip, rate, acc: 0, until_sub: 0, phase: 0, fast: false, enabled: true, gain: 1.0 }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// `PlayTune(id, level)`.
    pub fn play(&mut self, id: u8, level: u8) -> u16 {
        if !self.enabled {
            return 0;
        }
        self.driver.play(id, level)
    }

    /// Something still sounding (voices or noise slots)?
    pub fn is_active(&self) -> bool {
        self.driver.is_active()
    }

    fn sub_tick(&mut self) {
        if self.fast {
            self.driver.fast_worker();
        }
        self.phase = (self.phase + 1) % 5;
        if self.phase == 0 {
            match self.driver.sound_worker() {
                1 => self.fast = true,
                0xffff => self.fast = false,
                _ => {}
            }
        }
        for (r, v) in self.driver.take_writes() {
            self.chip.write_register(r as u16, v);
        }
    }

    /// Advance the clock by exactly one sub-tick and return the register writes it produced
    /// (for tests and the register dump).
    pub fn step_sub_tick(&mut self) -> Vec<(u8, u8)> {
        if self.fast {
            self.driver.fast_worker();
        }
        self.phase = (self.phase + 1) % 5;
        if self.phase == 0 {
            match self.driver.sound_worker() {
                1 => self.fast = true,
                0xffff => self.fast = false,
                _ => {}
            }
        }
        let w: Vec<(u8, u8)> = self.driver.take_writes().collect();
        for &(r, v) in &w {
            self.chip.write_register(r as u16, v);
        }
        w
    }

    /// Render interleaved stereo 16-bit samples.
    pub fn render(&mut self, out: &mut [i16]) {
        let frames = out.len() / 2;
        let mut i = 0;
        while i < frames {
            if self.until_sub == 0 {
                self.sub_tick();
                self.acc += self.rate;
                self.until_sub = (self.acc / SUB_RATE) as usize;
                self.acc %= SUB_RATE;
                if self.until_sub == 0 {
                    self.until_sub = 1;
                }
            }
            let n = self.until_sub.min(frames - i);
            let _ = self.chip.generate_stream(&mut out[2 * i..2 * (i + n)]);
            i += n;
            self.until_sub -= n;
        }
        if self.gain != 1.0 {
            for s in out.iter_mut() {
                *s = (*s as f32 * self.gain).clamp(-32768.0, 32767.0) as i16;
            }
        }
    }
}
