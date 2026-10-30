//! The game's random numbers: CIV.EXE's one `rand()` stream (`MsRand`), drawn with `random(n)`
//! at the same places and in the same order as the original, so a game seeded as CIV.EXE was
//! (reference/tools/civ_seed_patch.py) makes the same rolls.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;

/// The game's generator (`Game::rng`): the state of CIV.EXE's `rand()` (ds:0x5c38) in the low
/// 32 bits (a u64 keeps the save layout of the earlier generator).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rng(u64);

/// One draw for the trace (`trace_start`): the CIV.EXE address of the `random(n)` call, n and
/// the result, as the traced CIV.EXE logs them (civ_seed_patch.py --trace).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Draw {
    pub at: u32,
    pub n: i32,
    pub r: i32,
}

/// The trace address of a `random(n)` call in an overlay: its flat address in
/// civ.overlay.bin (the catalogue's "o 0x..."), marked with bit 20.
pub const fn ovl(flat: u32) -> u32 {
    flat | 0x10_0000
}

thread_local! {
    static TRACE: RefCell<Option<Vec<Draw>>> = const { RefCell::new(None) };
}

fn trace(at: u32, n: i32, r: i32) {
    TRACE.with(|t| {
        if let Some(v) = t.borrow_mut().as_mut() {
            v.push(Draw { at, n, r });
        }
    });
}

/// Record every draw of this thread's generators from now on (the world generator's
/// `MsRand` ones with address 0).
pub fn trace_start() {
    TRACE.with(|t| *t.borrow_mut() = Some(Vec::new()));
}

/// The number of draws recorded since `trace_start` (for the AI traces).
pub fn trace_len() -> usize {
    TRACE.with(|t| t.borrow().as_ref().map(|v| v.len()).unwrap_or(0))
}

/// The draws recorded since `trace_start` (recording goes on).
pub fn trace_take() -> Vec<Draw> {
    TRACE.with(|t| t.borrow_mut().as_mut().map(std::mem::take).unwrap_or_default())
}

impl Rng {
    /// `srand(seed)` for a seed below 0x10000; a larger one is folded to 32 bits.
    pub fn new(seed: u64) -> Rng {
        Rng(((seed as u32) ^ (seed >> 32) as u32) as u64)
    }

    /// The generator going on from a world generator's stream.
    pub fn from_ms(r: MsRand) -> Rng {
        Rng(r.state as u64)
    }

    pub fn state(&self) -> u32 {
        self.0 as u32
    }

    /// `random(n)` (0x1d43:0x53) called from CIV.EXE address `at`: rand() * n >> 15, so 0..n
    /// for n > 0 (0 for n == 0, which still draws, as CIV.EXE does).
    pub fn random(&mut self, at: u32, n: i32) -> i32 {
        let mut m = MsRand::new(self.0 as u32);
        let r = (m.rand() * n) >> 15;
        self.0 = m.state as u64;
        trace(at, n, r);
        r
    }

    /// `srand(seed)` (0x1fc3:0x2ade), as the city view does with its name (o 0x19193).
    pub fn srand(&mut self, seed: u16) {
        self.0 = seed as u64;
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
        let r = (self.rand() * n) >> 15;
        trace(0, n, r);
        r
    }
}
