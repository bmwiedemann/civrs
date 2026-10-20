//! The `ASOUND.CVL` driver re-implemented: the per-voice bytecode interpreter and the
//! `PlayTune` handlers, producing the same OPL2 register writes as the original 8086 code,
//! tick for tick. Addresses in comments are offsets in the driver's code segment (`cs:`) and
//! data segment (`ds:`), checked against `objdump` of the shipped driver.
//!
//! Driver interface, as the game uses it: [`Driver::play`] = `PlayTune(id, level)`,
//! [`Driver::sound_worker`] at 60 Hz, [`Driver::fast_worker`] at 300 Hz while the last
//! non-zero `sound_worker` result was 1, [`Driver::timer`] = `SoundTimer`. Register writes
//! accumulate in [`Driver::writes`] for an OPL2 emulator.
//!
//! Format summary (DS bytes): a stream is `note, duration` pairs (note 0 = rest, duration 0 =
//! end) and commands `FF n` loop, `FE n` outer loop, `FD` restart, `FC n` instrument, `FB n`
//! release, `FA n` pitch slide, `F9 n` volume, `F8 p d` fade, `F7 n` detune, `F6 ...` random
//! poke, `F5 n` volume offset, `F4 n` pan, `F3 p d` pan slide.

use super::cvl::{Image, PATCH_SIZE, PATCH_TABLE};

pub const VOICES: usize = 9;
/// The two-byte "end" stream every voice is pointed at by the fade-out (ID 1).
const FADE_STREAM: u16 = 0x5eaa;
/// The seven war-drum patterns of ID 33 (cs:0x1539 table).
const ID33_STREAMS: [u16; 7] = [0x208, 0x27e, 0x50, 0x98, 0xf2, 0x138, 0x1a4];
/// Carrier level registers forced to silent by the war drums (cs:0x1547).
const ID33_MUTE_REGS: [u8; 9] = [0x43, 0x44, 0x45, 0x4b, 0x4c, 0x4d, 0x53, 0x54, 0x55];

/// Highest sound ID `PlayTune` accepts.
pub const MAX_ID: u8 = 44;

/// The 30-byte per-voice state block (ds:0x741C + 0x1E * voice, except that voices 7 and 8
/// sit at 0x74F0 / 0x7510).
#[derive(Clone, Copy, Default, Debug)]
struct Voice {
    dur: u8,
    slide: u8,
    fade_delta: u8,
    pan_delta: u8,
    note: u8,
    instr: u8,
    vol: u8,
    release: u8,
    gate: u8,
    fade_ctr: u8,
    fade_period: u8,
    pans_ctr: u8,
    pans_period: u8,
    pan: u8,
    start: u16,
    pos: u16,
    loop1: u16,
    loop2: u16,
    loop1_ctr: u16,
    loop2_ctr: u16,
    restart: u16,
    detune: u8,
    volofs: u8,
}

fn s8(v: u8) -> i32 {
    v as i8 as i32
}

pub struct Driver {
    im: Image,
    /// Shadow copy of the 256 OPL registers (ds:0x7536).
    shadow: [u8; 256],
    /// Register writes since the last [`Driver::take_writes`], in order.
    pub writes: Vec<(u8, u8)>,
    voices: [Voice; VOICES],
    rng: u16,
    lock: u16,
    paused: u8,
    saved_dur: [u8; 8],
    id33_index: u16,
    sweep_slot: usize,
    slot_voice: [usize; 2],
    slot_count: [u8; 2],
    slot_mask: [u16; 2],
    slot_value: [u16; 2],
    slot_inc: [u16; 2],
    /// Per voice: pitch-sweep (length, mask, start, step) of its current instrument.
    sweep: [(u8, u16, u16, u16); VOICES],
    fast_state: u16,
    ret: u16,
    timer: u16,
    /// The "volume changed" flag is an uninitialised stack local in the original
    /// (`[bp-4]` of cs:0x07F4); the same frame is reused for every voice, so it carries
    /// over from one voice tick to the next. Kept here to match the register stream exactly.
    volflag: bool,
    /// ds:0x75F3, rhythm bits of register 0xBD; never written by the driver.
    rhythm: u8,
}

impl Driver {
    pub fn new(im: Image) -> Driver {
        let rng = im.rng_seed;
        let id33_index = im.id33_index;
        let mut d = Driver {
            im,
            shadow: [0; 256],
            writes: Vec::new(),
            voices: [Voice::default(); VOICES],
            rng,
            lock: 0,
            paused: 0,
            saved_dur: [0; 8],
            id33_index,
            sweep_slot: 0,
            slot_voice: [0; 2],
            slot_count: [0; 2],
            slot_mask: [0; 2],
            slot_value: [0; 2],
            slot_inc: [0; 2],
            sweep: [(0, 0, 0, 0); VOICES],
            fast_state: 0,
            ret: 0,
            timer: 0,
            volflag: true,
            rhythm: 0,
        };
        // Init: silence and clear every register, enable waveform select.
        d.reset();
        d
    }

    /// Drain the register writes made since the last call.
    pub fn take_writes(&mut self) -> std::vec::Drain<'_, (u8, u8)> {
        self.writes.drain(..)
    }

    /// `SoundTimer`: ticks since the last `PlayTune`.
    pub fn timer(&self) -> u16 {
        self.timer
    }

    /// True while any voice or noise slot is active (what `PlayTune(2)` reports, plus the
    /// sweep slots).
    pub fn is_active(&self) -> bool {
        self.voices.iter().any(|v| v.dur != 0) || self.slot_count != [0, 0]
    }

    // ---- low level -------------------------------------------------------------------

    /// cs:0x114A
    fn rand(&mut self) -> u16 {
        let x = self.rng.wrapping_add(0x9248);
        self.rng = x.rotate_right(3);
        self.rng
    }

    /// cs:0x10DB
    fn write(&mut self, reg: u8, val: u8) {
        self.shadow[reg as usize] = val;
        self.writes.push((reg, val));
    }

    fn carrier_reg(&self, v: usize) -> u8 {
        0x40 + self.im.op_reg[self.im.voice_ops[v].1 as usize]
    }

    /// cs:0x0048: write the carrier's level register from volume + offset.
    fn update_volume(&mut self, v: usize) {
        let reg = self.carrier_reg(v);
        let ksl = self.shadow[reg as usize] & 0xc0;
        let vv = self.voices[v];
        let a = (vv.vol as i32 + s8(vv.volofs)).clamp(0, 0x3f) as u8;
        self.write(reg, (0x3f - a) | ksl);
    }

    /// cs:0x017C: A0/B0 from the current note.
    fn set_freq(&mut self, v: usize) {
        let vv = self.voices[v];
        let n = vv.note as usize;
        let f = (self.im.fnum[n % 12] as i32 + s8(vv.detune)) as u16;
        self.write(0xa0 + v as u8, (f & 0xff) as u8);
        let keyon = self.shadow[0xb0 + v] & 0x20;
        self.write(0xb0 + v as u8, keyon | (((n / 12) as u8) << 2) | (f >> 8) as u8);
    }

    /// cs:0x0230: per-tick pitch slide on the shadowed block:F-number.
    fn pitch_slide(&mut self, v: usize) {
        let slide = s8(self.voices[v].slide);
        let w = ((((self.shadow[0xb0 + v] & 0x1f) as i32) << 8) + self.shadow[0xa0 + v] as i32 + slide) as u16;
        self.write(0xa0 + v as u8, (w & 0xff) as u8);
        self.write(0xb0 + v as u8, (self.shadow[0xb0 + v] & 0x20) | (w >> 8) as u8);
    }

    /// cs:0x02BE
    fn key_off(&mut self, v: usize) {
        let b = self.shadow[0xb0 + v] & 0xdf;
        self.write(0xb0 + v as u8, b);
    }

    /// cs:0x1262
    fn request_fast(&mut self) {
        if self.fast_state != 1 {
            self.fast_state = 1;
            self.ret = 1;
        }
    }

    /// cs:0x02EC
    fn key_on(&mut self, v: usize) {
        self.update_volume(v);
        let sw = self.sweep[v];
        if sw.0 == 0 {
            self.set_freq(v);
            let b = self.shadow[0xb0 + v] | 0x20;
            self.write(0xb0 + v as u8, b);
            return;
        }
        // Noise / pitch-sweep instrument: the frequency is driven by the fast worker.
        if v == self.slot_voice[0] {
            self.sweep_slot = 0;
        }
        if v == self.slot_voice[1] {
            self.sweep_slot = 1;
        }
        let s = self.sweep_slot;
        self.sweep_slot ^= 1;
        // Original bug: both paths test slot A's counter (ds:0x74EE), so taking slot B while
        // slot A is busy keys off whatever voice slot B last held (voice 0 before first use).
        if self.slot_count[0] != 0 {
            let ov = self.slot_voice[s];
            let b = self.shadow[0xb0 + ov] & 0xdf;
            self.write(0xb0 + ov as u8, b);
        }
        self.slot_voice[s] = v;
        self.slot_count[s] = sw.0;
        self.slot_mask[s] = sw.1;
        self.slot_value[s] = sw.2;
        self.slot_inc[s] = sw.3;
        self.request_fast();
    }

    /// cs:0x04B4: program one operator from a 22-byte patch record.
    fn write_op(&mut self, v: usize, opreg: u8, p: [u8; 0x16]) {
        let (am, vib, csm) = self.im.flags;
        self.write(0x40 + opreg, 0x3f);
        self.write(0xbd, (self.rhythm & 0x3f) | if am != 0 { 0x80 } else { 0 } | if vib != 0 { 0x40 } else { 0 });
        self.write(0x08, if csm != 0 { 0x40 } else { 0 });
        self.write(0xc0 + v as u8, (p[0xa] << 1) | if p[0xd] != 0 { 0 } else { 1 });
        self.write(0x60 + opreg, (p[0] << 4) | (p[1] & 0xf));
        self.write(0x80 + opreg, (p[2] << 4) | (p[3] & 0xf));
        let mut r20 = p[9] & 0xf;
        if p[0xb] != 0 {
            r20 = r20.wrapping_add(0x80);
        }
        if p[0xc] != 0 {
            r20 = r20.wrapping_add(0x40);
        }
        if p[4] != 0 {
            r20 = r20.wrapping_add(0x20);
        }
        if p[5] != 0 {
            r20 = r20.wrapping_add(0x10);
        }
        self.write(0x20 + opreg, r20);
        self.write(0xe0 + opreg, p[8] & 3);
        self.write(0x40 + opreg, (0x3f - (p[6] & 0x3f)) | (p[7] << 6));
    }

    /// cs:0x06C8: load instrument `n` into both operators of voice `v`.
    fn load_patch(&mut self, v: usize, n: u8) {
        let b = self.shadow[0xb0 + v] & 0xdf;
        self.write(0xb0 + v as u8, b);
        let base = PATCH_TABLE + n as usize * PATCH_SIZE;
        let mut p1 = [0u8; 0x16];
        let mut p2 = [0u8; 0x16];
        p1.copy_from_slice(&self.im.ds[base..base + 0x16]);
        p2.copy_from_slice(&self.im.ds[base + 0x16..base + 0x2c]);
        let (m, c) = self.im.voice_ops[v];
        let (mreg, creg) = (self.im.op_reg[m as usize], self.im.op_reg[c as usize]);
        self.write_op(v, mreg, p1);
        let w = |o: usize| u16::from_le_bytes([p1[o], p1[o + 1]]);
        self.sweep[v] = (p1[0xe], w(0x10), w(0x12), w(0x14));
        self.write_op(v, creg, p2);
    }

    // ---- the per-voice tick, cs:0x07F4 ---------------------------------------------------

    fn tick_voice(&mut self, v: usize) {
        if self.voices[v].dur == 0 {
            return;
        }
        if self.voices[v].gate != 0 {
            self.voices[v].gate = self.voices[v].gate.wrapping_sub(1);
            if self.voices[v].gate == 0 {
                self.key_off(v);
            }
        }
        self.voices[v].dur = self.voices[v].dur.wrapping_sub(1);
        if self.voices[v].dur == 0 {
            loop {
                let pos = self.voices[v].pos as usize;
                let b = self.im.ds[pos];
                if b < 0xf3 {
                    break;
                }
                let arg = self.im.ds[(pos + 1) & 0xffff];
                self.volflag = false;
                let vv = &mut self.voices[v];
                match b {
                    0xff => {
                        // loop level 1
                        if vv.loop1_ctr == 0 {
                            let n = s8(arg);
                            if n == 0 {
                                vv.pos = vv.pos.wrapping_add(2);
                                vv.loop1 = vv.pos;
                                vv.loop1_ctr = 0;
                            } else {
                                vv.loop1_ctr = n as i16 as u16;
                                vv.pos = vv.loop1;
                            }
                        } else {
                            vv.loop1_ctr = vv.loop1_ctr.wrapping_sub(1);
                            if vv.loop1_ctr == 0 {
                                vv.pos = vv.pos.wrapping_add(2);
                                vv.loop1 = vv.pos;
                            } else {
                                vv.pos = vv.loop1;
                            }
                        }
                    }
                    0xfe => {
                        // loop level 2
                        if vv.loop2_ctr == 0 {
                            let n = s8(arg);
                            if n == 0 {
                                vv.pos = vv.pos.wrapping_add(2);
                                vv.loop2 = vv.pos;
                                vv.loop1 = vv.pos;
                                vv.loop1_ctr = 0;
                                vv.loop2_ctr = 0;
                            } else {
                                vv.loop2_ctr = n as i16 as u16;
                                vv.pos = vv.loop2;
                                vv.loop1 = vv.loop2;
                            }
                        } else {
                            vv.loop2_ctr = vv.loop2_ctr.wrapping_sub(1);
                            if vv.loop2_ctr == 0 {
                                vv.pos = vv.pos.wrapping_add(2);
                                vv.loop2 = vv.pos;
                                vv.loop1 = vv.pos;
                            } else {
                                vv.pos = vv.loop2;
                                vv.loop1 = vv.loop2;
                            }
                        }
                    }
                    0xfd => {
                        // restart from the top
                        vv.pos = vv.restart;
                        vv.loop1 = vv.restart;
                        vv.loop2 = vv.restart;
                        vv.slide = 0;
                        vv.fade_delta = 0;
                        vv.pan_delta = 0;
                        vv.vol = 0;
                        vv.release = 0;
                        vv.fade_ctr = 0;
                        vv.pans_ctr = 0;
                        vv.loop1_ctr = 0;
                        vv.loop2_ctr = 0;
                        vv.detune = 0;
                        vv.volofs = 0;
                        vv.pan = 0x40;
                    }
                    0xfc => {
                        vv.instr = arg;
                        vv.pos = vv.pos.wrapping_add(2);
                        self.load_patch(v, arg);
                    }
                    0xfb => {
                        vv.release = arg;
                        vv.pos = vv.pos.wrapping_add(2);
                    }
                    0xfa => {
                        vv.slide = arg;
                        vv.pos = vv.pos.wrapping_add(2);
                    }
                    0xf9 => {
                        vv.vol = (s8(arg) >> 1) as u8;
                        vv.pos = vv.pos.wrapping_add(2);
                        self.volflag = true;
                    }
                    0xf8 => {
                        vv.fade_period = arg;
                        vv.fade_delta = self.im.ds[(pos + 2) & 0xffff];
                        vv.fade_ctr = 1;
                        vv.pos = vv.pos.wrapping_add(3);
                    }
                    0xf7 => {
                        vv.detune = arg;
                        vv.pos = vv.pos.wrapping_add(2);
                    }
                    0xf6 => {
                        // random poke: pick one of n bytes and store it o bytes after the command
                        let n = s8(arg);
                        let si = pos as i32 + 2;
                        let r = self.rand() as i32;
                        let chosen = self.im.ds[(((n - 1) & r) + si) as usize & 0xffff];
                        let off = s8(self.im.ds[(si + n) as usize & 0xffff]);
                        let dst = (si + n + 1 + off) as usize & 0xffff;
                        self.im.ds[dst] = chosen;
                        let vv = &mut self.voices[v];
                        vv.pos = vv.pos.wrapping_add((n + 3) as u16);
                    }
                    0xf5 => {
                        vv.volofs = arg;
                        vv.pos = vv.pos.wrapping_add(2);
                        self.volflag = true;
                    }
                    0xf4 => {
                        vv.pan = arg;
                        vv.pos = vv.pos.wrapping_add(2);
                        self.volflag = true;
                    }
                    _ => {
                        // 0xf3 pan slide: period, delta
                        vv.pans_period = arg;
                        vv.pan_delta = self.im.ds[(pos + 2) & 0xffff];
                        vv.pans_ctr = 1;
                        vv.pos = vv.pos.wrapping_add(3);
                    }
                }
            }
            // a note (or rest / end marker)
            if self.volflag {
                self.update_volume(v);
            }
            let pos = self.voices[v].pos as usize;
            let note = self.im.ds[pos];
            let dur = self.im.ds[(pos + 1) & 0xffff];
            {
                let vv = &mut self.voices[v];
                vv.note = note;
                vv.dur = dur;
                vv.pos = vv.pos.wrapping_add(2);
            }
            if note == 0 || dur == 0 {
                self.key_off(v);
            } else {
                self.voices[v].gate = dur.wrapping_sub(self.voices[v].release);
                self.key_on(v);
            }
        }
        // per-tick effects (cs:0x0CA9)
        if self.voices[v].slide != 0 {
            self.pitch_slide(v);
        }
        self.volflag = false;
        if self.voices[v].fade_ctr == 0 && self.voices[v].pans_ctr == 0 {
            return;
        }
        {
            let vv = &mut self.voices[v];
            vv.fade_ctr = vv.fade_ctr.wrapping_sub(1);
            if vv.fade_ctr == 0 {
                vv.fade_ctr = vv.fade_period;
                if vv.fade_delta != 0 {
                    vv.volofs = vv.volofs.wrapping_add(vv.fade_delta);
                    self.volflag = true;
                    if s8(vv.volofs) <= 0 {
                        vv.fade_ctr = 0;
                        vv.volofs = 0;
                        vv.vol = 0;
                    }
                    if s8(vv.volofs) >= 0x3f {
                        vv.fade_ctr = 0;
                        vv.volofs = 0x3f;
                        vv.vol = 0;
                    }
                }
            }
            vv.pans_ctr = vv.pans_ctr.wrapping_sub(1);
            if vv.pans_ctr == 0 {
                vv.pans_ctr = vv.pans_period;
                if vv.pan_delta != 0 {
                    vv.pan = vv.pan.wrapping_add(vv.pan_delta);
                    self.volflag = true;
                }
            }
        }
        if self.volflag {
            self.update_volume(v);
        }
    }

    // ---- driver entry points ---------------------------------------------------------------

    /// `SoundWorker` (cs:0x115B), to be called at 60 Hz. Returns 1 to request
    /// `fast_worker` calls at 300 Hz, 0xFFFF to stop them, 0 for no change.
    pub fn sound_worker(&mut self) -> u16 {
        self.rand();
        if self.lock == 0xffff {
            return self.take_ret();
        }
        if self.paused != 0 && self.voices[8].dur == 0 {
            for i in 0..8 {
                self.voices[i].dur = self.saved_dur[i];
            }
            self.voices[8].pos = FADE_STREAM;
            self.paused = 0;
        }
        self.timer = self.timer.wrapping_add(1);
        for v in 0..VOICES {
            self.tick_voice(v);
        }
        if self.slot_count == [0, 0] {
            if self.fast_state != 0xffff {
                self.fast_state = 0xffff;
                self.ret = 0xffff;
            }
            return self.take_ret();
        }
        for s in 0..2 {
            if self.slot_count[s] != 0 {
                self.slot_value[s] = self.slot_value[s].wrapping_add(self.slot_inc[s]);
                self.slot_count[s] -= 1;
                if self.slot_count[s] == 0 {
                    let v = self.slot_voice[s];
                    let o = 1 - s;
                    if self.slot_count[o] == 0 || v != self.slot_voice[o] {
                        self.write(0xa0 + v as u8, 0);
                        self.write(0xb0 + v as u8, 0);
                    }
                }
            }
        }
        self.take_ret()
    }

    fn take_ret(&mut self) -> u16 {
        std::mem::take(&mut self.ret)
    }

    /// `FastSoundWorker` (cs:0x121F), ~300 Hz while requested: random F-numbers for the
    /// noise slots.
    pub fn fast_worker(&mut self) {
        self.rand();
        if self.slot_count[0] != 0 {
            let w = ((!self.rng) & self.slot_mask[0]).wrapping_add(self.slot_value[0]);
            self.raw_freq(self.slot_voice[0], w);
        }
        if self.slot_count[1] != 0 {
            let w = (self.rng & self.slot_mask[1]).wrapping_add(self.slot_value[1]);
            self.raw_freq(self.slot_voice[1], w);
        }
    }

    /// cs:0x1134
    fn raw_freq(&mut self, v: usize, w: u16) {
        self.write(0xa0 + v as u8, (w & 0xff) as u8);
        self.write(0xb0 + v as u8, ((w >> 8) & 0xff) as u8 | 0x20);
    }

    /// cs:0x0F27: silence and clear every register (`PlayTune(0)` and `Close`).
    pub fn reset(&mut self) {
        let saved = self.lock;
        self.lock = 0xffff;
        self.paused = 0;
        for vv in self.voices.iter_mut() {
            vv.dur = 0;
            vv.slide = 0;
            vv.fade_delta = 0;
            vv.pan_delta = 0;
        }
        self.slot_count = [0, 0];
        self.slot_mask = [0, 0];
        self.slot_value = [0, 0];
        self.slot_inc = [0, 0];
        for r in (0x40..=0x4f).rev() {
            self.write(r, 0x3f);
        }
        for r in (0x60..=0xff).rev() {
            self.write(r, 0);
        }
        for r in (1..=0x3f).rev() {
            self.write(r, 0);
        }
        self.write(1, 0x20);
        self.lock = saved;
    }

    /// cs:0x12B2: point voice `v` at `stream` and have it read on the next tick.
    fn start_voice(&mut self, v: usize, stream: u16) {
        let saved = self.lock;
        self.lock = 0xffff;
        let vv = &mut self.voices[v];
        vv.start = stream;
        vv.pos = stream;
        vv.loop1 = stream;
        vv.loop2 = stream;
        vv.restart = stream;
        vv.fade_period = 0xff;
        vv.slide = 0;
        vv.fade_delta = 0;
        vv.pan_delta = 0;
        vv.detune = 0;
        vv.volofs = 0;
        vv.vol = 0;
        vv.fade_ctr = 0;
        vv.pans_ctr = 0;
        vv.loop1_ctr = 0;
        vv.loop2_ctr = 0;
        vv.release = 0;
        vv.pan = 0x40;
        vv.dur = 1;
        self.lock = saved;
    }

    /// `PlayTune(id, level)` (cs:0x1D80). IDs above 44 are ignored. `level` 0-3 picks the
    /// arrangement of the long national themes (5-18); the game passes 3. Returns the
    /// handler's AX: non-zero for ID 2 while something is still playing.
    pub fn play(&mut self, id: u8, level: u8) -> u16 {
        if id > MAX_ID {
            return 0;
        }
        let r = self.handler(id, level);
        self.timer = 0;
        r
    }

    fn handler(&mut self, id: u8, level: u8) -> u16 {
        match id {
            0 => {
                self.reset();
                0
            }
            1 => {
                // fade everything out
                for vv in self.voices.iter_mut() {
                    vv.pos = FADE_STREAM;
                    vv.fade_delta = 0xff;
                    vv.fade_period = 1;
                    vv.fade_ctr = 1;
                }
                0
            }
            2 => self.voices.iter().fold(0u16, |r, v| r | v.dur as u16),
            33 => {
                // war drums: pause the music, mute the carriers, one of seven patterns on voice 8
                let saved = self.lock;
                self.lock = 0xffff;
                for i in 0..8 {
                    self.saved_dur[i] = self.voices[i].dur;
                    self.voices[i].dur = 0;
                }
                for op in ID33_MUTE_REGS {
                    let v = self.shadow[op as usize] | 0x3f;
                    self.write(op, v);
                }
                self.paused = 0xff;
                self.lock = saved;
                self.id33_index += 1;
                if self.id33_index > 6 {
                    self.id33_index = 0;
                }
                self.start_voice(8, ID33_STREAMS[self.id33_index as usize]);
                0
            }
            38 | 39 => {
                if id == 39 {
                    self.start_voice(0, 0x68a4);
                }
                let stream: usize = if id == 38 { 0x6898 } else { 0x688c };
                self.rand();
                let ax = (self.rng & 0x707).wrapping_add(0x140c);
                let (al, ah) = ((ax & 0xff) as u8, (ax >> 8) as u8);
                self.im.ds[stream + 5] = al;
                self.im.ds[stream + 7] = ah;
                self.im.ds[0x68a5] = al.wrapping_add(ah);
                self.start_voice(1, stream as u16);
                0
            }
            44 => {
                // four-part canon with random rests before voices 0-2
                let r = self.rand();
                self.im.ds[0x68fb] = (r & 7) as u8 + 0xe;
                self.start_voice(0, 0x68fa);
                let r = self.rand();
                self.im.ds[0x6901] = (r & 7) as u8 + 0x1c;
                self.start_voice(1, 0x6900);
                let r = self.rand();
                self.im.ds[0x6907] = (r & 7) as u8 + 0x2a;
                self.start_voice(2, 0x6906);
                for v in 0..3 {
                    self.voices[v].loop1 = 0x690c;
                }
                self.start_voice(3, 0x690c);
                0
            }
            _ => {
                for &(v, stream) in tune(id, level) {
                    self.start_voice(v as usize, stream);
                }
                0
            }
        }
    }
}

/// Voice -> stream assignments of the `PlayTune` handlers (cs:0x160A-0x1D0B, read from the
/// disassembly: `lea cx, ds:STREAM; call/jmp start_voice_N`). IDs 5-18 dispatch on `level`.
pub fn tune(id: u8, level: u8) -> &'static [(u8, u16)] {
    let l = (level & 3) as usize;
    match id {
        3 => &[(0, 0x4aa8), (1, 0x4bf8), (2, 0x4d1c), (3, 0x4dbe), (4, 0x4e78), (5, 0x4f30), (6, 0x4fba), (7, 0x5040)],
        4 => &[(0, 0x2da4), (1, 0x3014), (2, 0x3170), (3, 0x33f4), (4, 0x360e), (5, 0x38b6), (6, 0x3a2a), (7, 0x3cee)],
        5 => [
            &[(3, 0x2c76)][..],
            &[(2, 0x2afa), (3, 0x2c76)],
            &[(1, 0x2a5e), (2, 0x2afa), (3, 0x2c76)],
            &[(0, 0x297a), (1, 0x2a5e), (2, 0x2afa), (3, 0x2c76)],
        ][l],
        6 => [
            &[(3, 0x06a4)][..],
            &[(2, 0x066c), (3, 0x06a4)],
            &[(1, 0x05b0), (2, 0x066c), (3, 0x06a4)],
            &[(0, 0x0540), (1, 0x05b0), (2, 0x066c), (3, 0x06a4)],
        ][l],
        7 => [
            &[(6, 0x0928)][..],
            &[(1, 0x080c), (2, 0x0864), (3, 0x08be), (6, 0x0928)],
            &[(4, 0x0918), (0, 0x0742), (1, 0x080c), (2, 0x0864), (3, 0x08be), (6, 0x0928)],
            &[(4, 0x0918), (0, 0x0742), (1, 0x080c), (2, 0x0864), (3, 0x08be), (6, 0x0928)],
        ][l],
        8 => [
            &[(6, 0x0ba6)][..],
            &[(2, 0x0b1c), (6, 0x0ba6)],
            &[(1, 0x0a96), (2, 0x0b1c), (6, 0x0ba6)],
            &[(0, 0x0a02), (1, 0x0a96), (2, 0x0b1c), (6, 0x0ba6)],
        ][l],
        9 => [
            &[(7, 0x62f8), (4, 0x6090)][..],
            &[(6, 0x622c), (5, 0x615c), (7, 0x62f8), (4, 0x6090)],
            &[(2, 0x5f48), (3, 0x6036), (6, 0x622c), (5, 0x615c), (7, 0x62f8), (4, 0x6090)],
            &[(0, 0x5e0e), (1, 0x5eac), (2, 0x5f48), (3, 0x6036), (6, 0x622c), (5, 0x615c), (7, 0x62f8), (4, 0x6090)],
        ][l],
        10 => [
            &[(6, 0x0f4e), (2, 0x0e8e)][..],
            &[(3, 0x0fbc), (7, 0x1072), (4, 0x10ec), (6, 0x0f4e), (2, 0x0e8e)],
            &[(1, 0x0dcc), (5, 0x1168), (3, 0x0fbc), (7, 0x1072), (4, 0x10ec), (6, 0x0f4e), (2, 0x0e8e)],
            &[(0, 0x0d36), (1, 0x0dcc), (5, 0x1168), (3, 0x0fbc), (7, 0x1072), (4, 0x10ec), (6, 0x0f4e), (2, 0x0e8e)],
        ][l],
        11 => [
            &[(6, 0x4312), (7, 0x438a)][..],
            &[(0, 0x3fde), (1, 0x405c), (6, 0x4312), (7, 0x438a)],
            &[(2, 0x4100), (3, 0x41d0), (4, 0x424a), (5, 0x42ae), (0, 0x3fde), (1, 0x405c), (6, 0x4312), (7, 0x438a)],
            &[(2, 0x4100), (3, 0x41d0), (4, 0x424a), (5, 0x42ae), (0, 0x3fde), (1, 0x405c), (6, 0x4312), (7, 0x438a)],
        ][l],
        12 => [
            &[(3, 0x24e2), (4, 0x2562), (5, 0x25e2), (6, 0x268c)][..],
            &[(7, 0x26e4), (2, 0x232a), (3, 0x24e2), (4, 0x2562), (5, 0x25e2), (6, 0x268c)],
            &[(0, 0x1f6a), (1, 0x2148), (7, 0x26e4), (2, 0x232a), (3, 0x24e2), (4, 0x2562), (5, 0x25e2), (6, 0x268c)],
            &[(0, 0x1f6a), (1, 0x2148), (7, 0x26e4), (2, 0x232a), (3, 0x24e2), (4, 0x2562), (5, 0x25e2), (6, 0x268c)],
        ][l],
        13 => [
            &[(2, 0x5b96), (3, 0x5c16)][..],
            &[(1, 0x5a1a), (4, 0x5d14), (2, 0x5b96), (3, 0x5c16)],
            &[(0, 0x5886), (1, 0x5a1a), (4, 0x5d14), (2, 0x5b96), (3, 0x5c16)],
            &[(0, 0x5886), (1, 0x5a1a), (4, 0x5d14), (2, 0x5b96), (3, 0x5c16)],
        ][l],
        14 => [
            &[(2, 0x1534), (3, 0x1586), (4, 0x15d4), (5, 0x161e), (6, 0x166a)][..],
            &[(1, 0x143a), (2, 0x1534), (3, 0x1586), (4, 0x15d4), (5, 0x161e), (6, 0x166a)],
            &[(0, 0x133a), (1, 0x143a), (2, 0x1534), (3, 0x1586), (4, 0x15d4), (5, 0x161e), (6, 0x166a)],
            &[(0, 0x133a), (1, 0x143a), (2, 0x1534), (3, 0x1586), (4, 0x15d4), (5, 0x161e), (6, 0x166a)],
        ][l],
        15 => [
            &[(3, 0x043e), (4, 0x0468), (5, 0x0490)][..],
            &[(2, 0x03fa), (3, 0x043e), (4, 0x0468), (5, 0x0490)],
            &[(1, 0x0362), (6, 0x03ac), (2, 0x03fa), (3, 0x043e), (4, 0x0468), (5, 0x0490)],
            &[(0, 0x02d0), (1, 0x0362), (6, 0x03ac), (2, 0x03fa), (3, 0x043e), (4, 0x0468), (5, 0x0490)],
        ][l],
        16 => [
            &[(0, 0x187c), (5, 0x196e), (3, 0x1970), (7, 0x18c6), (1, 0x18c8)][..],
            &[(2, 0x1924), (0, 0x187c), (5, 0x196e), (3, 0x1970), (7, 0x18c6), (1, 0x18c8)],
            &[(4, 0x199e), (6, 0x19a0), (2, 0x1924), (0, 0x187c), (5, 0x196e), (3, 0x1970), (7, 0x18c6), (1, 0x18c8)],
            &[(4, 0x199e), (6, 0x19a0), (2, 0x1924), (0, 0x187c), (5, 0x196e), (3, 0x1970), (7, 0x18c6), (1, 0x18c8)],
        ][l],
        17 => [
            &[(6, 0x1e32), (7, 0x1e86)][..],
            &[(2, 0x1dc2), (3, 0x1de0), (4, 0x1dfc), (5, 0x1e18), (6, 0x1e32), (7, 0x1e86)],
            &[(1, 0x1c16), (2, 0x1dc2), (3, 0x1de0), (4, 0x1dfc), (5, 0x1e18), (6, 0x1e32), (7, 0x1e86)],
            &[(0, 0x1a68), (1, 0x1c16), (2, 0x1dc2), (3, 0x1de0), (4, 0x1dfc), (5, 0x1e18), (6, 0x1e32), (7, 0x1e86)],
        ][l],
        18 => [
            &[(2, 0x46f6), (3, 0x47f0)][..],
            &[(1, 0x4604), (4, 0x48d0), (2, 0x46f6), (3, 0x47f0)],
            &[(0, 0x4504), (1, 0x4604), (4, 0x48d0), (2, 0x46f6), (3, 0x47f0)],
            &[(5, 0x49a8), (0, 0x4504), (1, 0x4604), (4, 0x48d0), (2, 0x46f6), (3, 0x47f0)],
        ][l],
        19 => &[(0, 0x2d18), (1, 0x2d3e), (2, 0x2d56), (3, 0x2d88)],
        20 => &[(0, 0x06e6), (1, 0x0700), (2, 0x0716), (3, 0x0730)],
        21 => &[(0, 0x0988), (1, 0x09aa), (2, 0x09cc), (6, 0x09f0)],
        22 => &[(0, 0x0c84), (1, 0x0cb0), (2, 0x0cd8), (6, 0x0d00)],
        23 => &[(0, 0x632a), (1, 0x6346), (2, 0x6366), (3, 0x6382), (4, 0x639e), (5, 0x63ae)],
        24 => &[(0, 0x1222), (1, 0x1264), (2, 0x12aa), (3, 0x12e6), (4, 0x1310)],
        25 => &[(0, 0x4448), (1, 0x445a), (2, 0x446c), (3, 0x447e), (4, 0x4498), (5, 0x44b2), (6, 0x44cc), (7, 0x44e6)],
        26 => &[(0, 0x2870), (1, 0x2896), (2, 0x28bc), (3, 0x28e2), (4, 0x2908), (5, 0x292e), (6, 0x2954)],
        27 => &[(0, 0x5d92), (1, 0x5dac), (2, 0x5dc6), (3, 0x5dd6), (4, 0x5dee)],
        28 => &[(0, 0x17c2), (1, 0x17d2), (2, 0x17e2), (3, 0x1804), (4, 0x181c), (5, 0x1834), (6, 0x184c)],
        29 => &[(0, 0x04ba), (1, 0x04d8), (2, 0x04ec), (3, 0x04fe), (4, 0x050e), (5, 0x051c), (6, 0x052c)],
        30 => &[(0, 0x1a14), (1, 0x1a24), (2, 0x1a34), (3, 0x1a46), (4, 0x1a22), (5, 0x1a44), (6, 0x1a56), (7, 0x1a58)],
        31 => &[(0, 0x1eb6), (1, 0x1ec8), (2, 0x1eda), (3, 0x1eec), (4, 0x1f02), (5, 0x1f40)],
        32 => &[(0, 0x4a2e), (1, 0x4a4c), (2, 0x4a6a), (3, 0x4a88), (4, 0x4a98)],
        34 => &[(0, 0x5156), (1, 0x51e6), (2, 0x5276), (3, 0x5456), (4, 0x54d6), (5, 0x5556), (6, 0x5736), (7, 0x5830)],
        35 => &[(0, 0x63ce), (1, 0x6446), (2, 0x64a0), (3, 0x64f4), (4, 0x6542), (5, 0x6590), (6, 0x6604)],
        36 => &[(1, 0x32c2), (2, 0x34da), (3, 0x3804), (4, 0x3976), (5, 0x3bf6), (6, 0x3edc)],
        37 => &[(0, 0x68ee)],
        40 => &[(6, 0x6870)],
        41 => &[(2, 0x687c), (3, 0x6864)],
        42 => &[(4, 0x68c6), (5, 0x68b4)],
        43 => &[(0, 0x68d2), (1, 0x68e2)],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver() -> Option<Driver> {
        let dir = crate::assets::AssetDir::locate(None).ok()?;
        let bytes = dir.read("ASOUND.CVL").ok()?;
        Some(Driver::new(Image::parse(&bytes).expect("ASOUND.CVL parses")))
    }

    /// FNV-1a over the `(sub-tick, reg, val)` stream of one sound, clocked like the game
    /// (300 Hz sub-ticks, `sound_worker` on every fifth, `fast_worker` while requested).
    fn fingerprint(d: &mut Driver, id: u8, level: u8, ticks: u32) -> (u64, usize) {
        d.take_writes();
        d.play(id, level);
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut n = 0usize;
        let mut fast = false;
        for sub in 0..ticks * 5 {
            if fast {
                d.fast_worker();
            }
            if sub % 5 == 4 {
                match d.sound_worker() {
                    1 => fast = true,
                    0xffff => fast = false,
                    _ => {}
                }
            }
            for (r, v) in d.take_writes() {
                for b in [(sub & 0xff) as u8, (sub >> 8) as u8, r, v] {
                    h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
                }
                n += 1;
            }
        }
        (h, n)
    }

    /// Register streams verified byte for byte against the civ-clone reference simulator of
    /// the original driver (3000 ticks = 50 s). Write counts for 200 s match its published
    /// table too.
    #[test]
    fn register_streams_match_reference() {
        let Some(mut d) = driver() else { return };
        let expected: [(u8, u8, u64, usize); 8] = [
            (3, 3, 0xb63a33401a000fbb, 1981),
            (19, 0, 0xc44758ad0ed22287, 307),
            (33, 0, 0x805c7400377c8095, 159),
            (37, 0, 0x1de161bfa7675d8b, 30),
            (38, 0, 0x8e34b660078931d7, 35),
            (42, 0, 0x8341dceef6dd481c, 5215),
            (44, 0, 0xe014da7cccd2f37a, 993),
            (5, 0, 0x79c925e1b1ee24bd, 590),
        ];
        let mut bad = Vec::new();
        for (id, level, h, n) in expected {
            let mut fresh = driver().unwrap();
            let got = fingerprint(&mut fresh, id, level, 3000);
            if got != (h, n) {
                bad.push(format!("({id}, {level}, {:#018x}, {}),", got.0, got.1));
            }
        }
        assert!(bad.is_empty(), "fingerprints changed:\n{}", bad.join("\n"));
        // the same driver instance keeps its RNG state between sounds, as the original does
        let _ = fingerprint(&mut d, 38, 0, 100);
    }

    #[test]
    fn query_and_timer() {
        let Some(mut d) = driver() else { return };
        assert_eq!(d.play(2, 0), 0);
        d.play(37, 0);
        assert_ne!(d.play(2, 0), 0);
        for _ in 0..10 {
            d.sound_worker();
        }
        assert_eq!(d.timer(), 10);
        d.play(0, 0);
        assert_eq!(d.timer(), 0);
        assert!(!d.is_active());
    }
}
