//! Deterministic xorshift64* random number generator (serialisable so saves reproduce).

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let s = if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed };
        let mut r = Rng(s);
        for _ in 0..4 {
            r.next_u64();
        }
        r
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform integer in `0..n` (0 when n == 0).
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        ((self.next_u64() >> 33) % n as u64) as u32
    }

    /// Uniform integer in `lo..=hi`.
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + self.below((hi - lo + 1) as u32) as i32
    }

    /// True with probability `pct` percent.
    pub fn chance(&mut self, pct: u32) -> bool {
        self.below(100) < pct
    }

    /// Pick a random element.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            Some(&items[self.below(items.len() as u32) as usize])
        }
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u32 + 1) as usize;
            items.swap(i, j);
        }
    }
}

/// CIV.EXE's random numbers: the Microsoft C 5 `rand()` (0x1fc3:0x2af0: state = state *
/// 0x343fd + 0x269ec3, result bits 16..30 of the new state) seeded by `srand` (0x1fc3:0x2ade,
/// low word = seed, high word = 0), and `random(n)` (0x1d43:0x53) = rand() * n >> 15.
/// CIV.EXE seeds it once from the clock (0x1d43:0x38: time & 0x7fff); civrs starts the world
/// generator from the game seed instead, so the same seed always makes the same world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsRand {
    pub state: u32,
}

impl MsRand {
    /// A generator whose next `rand()` steps from `state` (a whole 32-bit state; `srand(s)`
    /// is `MsRand::new(s as u16 as u32)`).
    pub fn new(state: u32) -> MsRand {
        MsRand { state }
    }

    /// `rand()`: 0..=0x7fff.
    pub fn rand(&mut self) -> i32 {
        self.state = self.state.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        ((self.state >> 16) & 0x7fff) as i32
    }

    /// `random(n)` (0x1d43:0x53): 0..n for n > 0.
    pub fn random(&mut self, n: i32) -> i32 {
        (self.rand() * n) >> 15
    }
}
