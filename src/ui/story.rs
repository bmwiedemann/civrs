//! "In the beginning..." evolution story, as overlay 7's `world_progress` (o 0xa509) shows it
//! while the world is generated. STORY.TXT is read one line per step; a `_` line fades to the
//! next BIRTHn picture (n = 1..8, palette from BIRTHn.PAL, with that picture's colour cycles),
//! every other line becomes the single caption at (160, 160). Line k is due at SoundTimer tick
//! `LINE_TICKS[k]` of the evolution music, but stays at least 45 ticks per two characters' weight
//! (90, or 135 for lines over 19 characters). A changing caption first dims to colour 3, is
//! erased, then appears in 3 and brightens to 11. After the last line the screen holds 180 ticks.
//! A key ends the story at once (the map is always finished here, `ds:0x67fc`).

use crate::assets::Palette;
use crate::gfx::text::draw_text_centered;
use crate::gfx::{Surface, TextStyle};
use crate::platform::Event;
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};

/// SoundTimer tick at which text line k is due (far table at seg 0x268f +0x83; 0xffff = none).
const LINE_TICKS: [u16; 45] = [
    0, 192, 336, 624, 768, 912, 1104, 1200, 1440, 1632, 1872, 2016, 2256, 2352, 2496, 2688, 2880, 3168, 3456, 3648, 3744, 3840, 3936, 4032, 4224, 4416, 4608, 4800, 4992,
    5184, 5376, 5568, 5760, 6000, 6192, 6336, 6528, 6720, 7008, 7296, 7584, 7872, 8064, 8512, 0xffff,
];

/// Colour cycles per picture (seg 0x268f +0): (first, last, speed); period = 300 / speed ticks.
const CYCLES: [&[(u8, u8, u8)]; 8] = [
    &[(146, 152, 21), (183, 187, 5), (188, 191, 17)],
    &[(48, 56, 11), (32, 47, 10), (141, 148, 13), (176, 183, 23), (211, 215, 3), (152, 155, 1), (57, 79, 15)],
    &[(80, 95, 17), (96, 111, 7), (112, 127, 30), (136, 140, 10), (184, 188, 11)],
    &[(134, 138, 5), (139, 143, 10), (240, 241, 23), (96, 101, 25), (242, 244, 30), (245, 250, 12), (251, 252, 11), (102, 105, 21)],
    &[(184, 187, 21), (180, 183, 16), (135, 140, 12), (214, 215, 21), (96, 102, 17)],
    &[(136, 138, 21), (129, 131, 8), (246, 249, 21), (250, 254, 12)],
    &[(246, 249, 21), (250, 251, 10), (208, 210, 11), (135, 138, 16)],
    &[(208, 210, 16), (216, 218, 11), (131, 134, 16), (48, 53, 25), (246, 249, 64)],
];

/// Caption: window font 7 (civrs id 6), centred on x 160 at y 160, dim colour 3, bright 11.
const CAPTION_FONT: usize = 6;
const CAPTION_Y: i32 = 160;
const DIM: u8 = 3;
const BRIGHT: u8 = 11;
/// Palette fades to black and in again around a picture change (`fade_to_colour(8, 0,0,0)`,
/// `fade_palette(8, pal)`).
const FADE_TICKS: u64 = 8;
/// Pause after the last line (o 0xa8aa).
const END_TICKS: u64 = 180;
/// Captions from this line count on are not dimmed before the next one (o 0xa605).
const NO_DIM_FROM: usize = 40;

/// Where `world_progress` is inside one call; waits split it into steps.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Start of a call: is the next line due?
    Entry,
    Read,
    ShowDim,
    ShowBright,
    /// Picture faded out; copy it to the screen and fade the new palette in.
    PictureIn,
    /// Palette faded in; start the picture's colour cycles.
    Cycles,
    End,
}

struct Fade {
    from: Palette,
    to: Palette,
    start: u64,
}

pub struct StoryScreen {
    step: Step,
    tick: u64,
    wait_until: u64,
    /// Tick at which the evolution music (and so SoundTimer) started.
    music_start: Option<u64>,
    next: usize,
    /// Current line (`ds:0xba06`).
    line: String,
    /// Text lines read (`ds:0x6802`).
    lines_read: usize,
    /// Due time of the next line, in weight units until `tail` turns it into ticks (`ds:0x6806`).
    due: i32,
    /// SoundTimer when the call began (`ds:0x6800`).
    now: i32,
    /// Next BIRTHn picture (`ds:0x3b64`).
    pic_no: usize,
    /// Picture on the screen.
    shown: Option<usize>,
    caption: Option<(String, u8)>,
    pal: Palette,
    fade: Option<Fade>,
    /// Running colour cycles: (first, last, period, start tick).
    cycles: Vec<(u8, u8, u64, u64)>,
    key: bool,
}

pub fn story_screen() -> Box<dyn Screen> {
    Box::new(StoryScreen {
        step: Step::Entry,
        tick: 0,
        wait_until: 0,
        music_start: None,
        next: 0,
        line: String::new(),
        lines_read: 0,
        due: 0,
        now: 0,
        pic_no: 1,
        shown: None,
        caption: None,
        pal: Palette::black(),
        fade: None,
        cycles: Vec::new(),
        key: false,
    })
}

impl StoryScreen {
    fn sound_timer(&self) -> i32 {
        self.music_start.map_or(0, |s| (self.tick - s) as i32)
    }

    fn wait(&mut self, ticks: u64, then: Step) {
        self.wait_until = self.tick + ticks;
        self.step = then;
    }

    fn fade_to(&mut self, ctx: &UiContext, to: Palette) {
        let from = self.palette(ctx);
        self.fade = Some(Fade { from, to, start: self.tick });
    }

    /// One step of `world_progress`; returns true when the story is over.
    fn run(&mut self, ctx: &mut UiContext) -> bool {
        match self.step {
            Step::Entry => {
                if self.lines_read != 0 {
                    self.now = self.sound_timer();
                }
                if self.now < self.due {
                    return false;
                }
                if self.lines_read < NO_DIM_FROM && !self.line.is_empty() {
                    self.caption = Some((self.line.clone(), DIM));
                }
                self.wait(5, Step::Read);
            }
            Step::Read => match ctx.texts.story.get(self.next) {
                Some(l) if !self.key => {
                    self.line = l.clone();
                    self.next += 1;
                    if self.pic_no > 1 {
                        self.caption = None;
                    }
                    self.wait(10, Step::ShowDim);
                }
                _ => {
                    // end of file: hold the last frame; a key ends at once (o 0xa8a4)
                    let hold = if self.key { 0 } else { END_TICKS };
                    self.wait(hold, Step::End);
                }
            },
            Step::ShowDim => {
                self.caption = Some((self.line.clone(), DIM));
                self.wait(5, Step::ShowBright);
            }
            Step::ShowBright => {
                self.caption = Some((self.line.clone(), BRIGHT));
                self.due = 0;
                if self.line.len() > 3 {
                    if self.lines_read == 0 {
                        ctx.audio.fade_out();
                        ctx.audio.play(crate::audio::Sound::Evolution);
                        self.music_start = Some(self.tick);
                    }
                    self.due += 2;
                    self.lines_read += 1;
                    self.tail();
                } else {
                    self.fade_to(ctx, Palette::black());
                    self.cycles.clear();
                    self.wait(FADE_TICKS, Step::PictureIn);
                }
            }
            Step::PictureIn => {
                let n = self.pic_no;
                self.pic_no += 1;
                self.shown = Some(n);
                self.caption = None;
                self.pal = ctx.pal(&format!("BIRTH{n}"));
                self.fade_to(ctx, self.pal);
                self.wait(FADE_TICKS, Step::Cycles);
            }
            Step::Cycles => {
                self.fade = None;
                let group = CYCLES.get(self.pic_no - 2).copied().unwrap_or(&[]);
                self.cycles = group.iter().map(|&(first, last, speed)| (first, last, 300 / speed as u64, self.tick)).collect();
                self.tail();
            }
            Step::End => {
                ctx.audio.fade_out();
                self.cycles.clear();
                return true;
            }
        }
        false
    }

    /// End of a call (o 0xa863): the next line's due time.
    fn tail(&mut self) {
        if self.line.len() > 19 {
            self.due += 1;
        }
        let at = LINE_TICKS.get(self.lines_read).map_or(-1, |&t| t as i16 as i32);
        let lo = self.now + 45 * self.due;
        self.due = at.clamp(lo, 0x7fff.max(lo));
        self.step = Step::Entry;
    }
}

impl Screen for StoryScreen {
    fn on_exit(&mut self, ctx: &mut UiContext) {
        ctx.audio.fade_out();
    }

    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        self.tick += 1;
        // the original polls back to back; an instant step continues in the same tick
        while self.tick >= self.wait_until {
            let step = self.step;
            if self.run(ctx) {
                return Transition::Pop(DialogResult::Done);
            }
            if step == Step::Entry && self.step == Step::Entry {
                break; // the next line is not due yet
            }
        }
        Transition::None
    }

    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if let Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } = ev {
            self.key = true;
        }
        Transition::None
    }

    fn resume(&mut self, _r: DialogResult, _ctx: &mut UiContext) -> Transition {
        Transition::None
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        fb.fill(0);
        if let Some(n) = self.shown {
            fb.blit(&ctx.pic(&format!("BIRTH{n}")).surf, 0, 0);
        }
        if let Some((line, colour)) = &self.caption {
            draw_text_centered(fb, &ctx.fonts, &TextStyle::new(CAPTION_FONT, *colour), 160, CAPTION_Y, line);
        }
    }

    fn palette(&self, _ctx: &UiContext) -> Palette {
        if let Some(f) = &self.fade {
            let t = ((self.tick - f.start).min(FADE_TICKS) * 255 / FADE_TICKS) as u8;
            return f.from.lerp(&f.to, t);
        }
        let mut pal = self.pal;
        for &(first, last, period, start) in &self.cycles {
            let n = (last - first) as usize + 1;
            let phase = ((self.tick - start) / period.max(1)) as usize % n;
            let ring: Vec<[u8; 3]> = (0..n).map(|i| self.pal.0[first as usize + i]).collect();
            for i in 0..n {
                pal.0[first as usize + i] = ring[(i + phase) % n];
            }
        }
        pal
    }

    fn draws_parent(&self) -> bool {
        false
    }

    fn wants_cursor(&self) -> bool {
        false
    }
}
