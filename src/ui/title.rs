//! Opening sequence as overlay 2's `startup_and_intro` (o 0xb75) runs it: BIRTH0 slides in
//! from the right, then BIRTH1 pushes it out to the left, while CREDITS.TXT is shown one line
//! at a time (starting with "A MICROPROSE PRESENTATION") in a band across the picture. Then the
//! logo wipes in over the picture, stays a while, and the picture dissolves away (the driver's
//! dissolve, duration 20) to leave the logo on black under the main menu.
//!
//! All pacing follows SoundTimer (60 Hz ticks since the title music started): scroll column x
//! is shown once the timer reaches 21*x/4, the second pass continues at 21*320/4 (o 0xfeb,
//! o 0x1345), the logo wipe starts at tick 0xd87 (o 0x158e).

use crate::assets::Palette;
use crate::gfx::text::draw_text_centered;
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::Event;
use crate::rules::MAIN_MENU;
use crate::ui::context::UiContext;
use crate::ui::reveal::Reveal;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::{MenuDialog, PanelStyle};

/// Logo sprite cut from LOGO.PIC (driver entry 2 with (0, 64, 320, 80)) and drawn at (0, 64).
const LOGO_SRC: Rect = Rect::new(0, 64, 320, 80);
const LOGO_Y: i32 = 64;
/// Driver duration of the dissolve from the picture to the logo on black (o 0x16f8).
const LOGO_DISSOLVE: u32 = 20;
/// The picture's rows 0..176 are shown from screen row 12 (o 0xf49..0xfaf).
const PIC_Y: i32 = 12;
const PIC_H: i32 = 176;
/// Text rows of the credit band (picture rows 64..88): highlight 0xf8 at 82, 0xfa at 83, shadow 7
/// at 84, drawn in font 5 centred on x 160 (o 0xec7..0xf1f).
const CREDIT_Y: i32 = 82;
const CREDIT_FONT: usize = 4;
/// Columns per scroll pass; the scroll runs two passes.
const PASS_W: i32 = 320;
/// SoundTimer tick at which the logo wipe starts.
const WIPE_TICK: u64 = 0xd87;
/// The wipe copies 4 columns per tick (o 0x162e, `wait_ticks(1)`).
const WIPE_STEP: i32 = 4;
/// The logo stays for 100 x `wait_ticks(5)` unless a key is pressed (o 0x1672).
const HOLD_TICKS: u64 = 500;

/// Credit-line events per pass at scroll column x: `true` reads the next CREDITS.TXT line,
/// `false` blanks the band (o 0x1097..0x116f and o 0x1414..0x1517).
const CREDIT_EVENTS: [&[(i32, bool)]; 2] = [
    &[(10, true), (50, false), (70, true), (120, true), (160, false), (170, true), (190, true), (210, true), (230, true), (250, true), (270, true), (290, true), (310, true)],
    &[(10, true), (30, true), (50, false), (64, true), (84, true), (116, false), (130, true), (150, true), (170, true), (190, true), (210, true), (230, true), (250, true), (270, true), (290, true), (310, false)],
];

/// Palette-cycle ranges: slot 1 (0x92..0x98, period 14) runs from the start (o 0xe59), slot 4
/// (0xe0..0xef, period 14) from the logo wipe (o 0x1605); all stop before the dissolve.
const CYCLE_STARS: (u8, u8) = (0x92, 0x98);
const CYCLE_LOGO: (u8, u8) = (0xe0, 0xef);
const CYCLE_PERIOD: u64 = 14;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// The two scroll passes with the credits.
    Scroll,
    /// Scroll finished, waiting for the wipe tick.
    Wait,
    /// The logo wipes in left to right.
    Wipe,
    Hold,
    /// The picture dissolves away around the logo.
    Dissolve,
    Menu,
}

pub struct TitleScreen {
    phase: Phase,
    /// Ticks since the sequence started (the music's SoundTimer).
    t: u64,
    /// Tick at which the current phase started.
    phase_t: u64,
    /// Last scroll column shown, 0..2*PASS_W over both passes; -1 before the first.
    col: i32,
    next_line: usize,
    line: String,
    /// Tick at which the logo wipe started (the logo colour cycle runs from there).
    wipe_t: Option<u64>,
    /// Tick at which the colour cycles were stopped; the palette keeps that rotation.
    cycles_stopped: Option<u64>,
    /// A key was pressed during the wait or the wipe; it ends the hold at once.
    key_pending: bool,
    menu_up: bool,
    /// Main-menu choice picked with N/L/E/C during the scroll (o 0x154c).
    shortcut: Option<usize>,
    reveal: Option<Reveal>,
}

impl Default for TitleScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl TitleScreen {
    pub fn new() -> TitleScreen {
        TitleScreen { phase: Phase::Scroll, t: 0, phase_t: 0, col: -1, next_line: 0, wipe_t: None, cycles_stopped: None, line: String::new(), key_pending: false, menu_up: false, shortcut: None, reveal: None }
    }

    pub fn at_menu() -> TitleScreen {
        TitleScreen { phase: Phase::Menu, ..Self::new() }
    }

    fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
        self.phase_t = self.t;
    }

    fn open_menu(&mut self) -> Transition {
        self.phase = Phase::Menu;
        self.menu_up = true;
        let items: Vec<String> = MAIN_MENU.iter().map(|s| s.to_string()).collect();
        Transition::Push(Box::new(MenuDialog::new(vec![], items).with_pos(86, 135).with_style(PanelStyle::Brown).with_size(154, 53).with_top_pad(7).no_cancel()))
    }

    fn start_dissolve(&mut self, ctx: &UiContext) {
        self.set_phase(Phase::Dissolve);
        self.cycles_stopped = Some(self.t);
        self.reveal = Some(Reveal::new(ctx, LOGO_DISSOLVE));
    }

    /// Advance the scroll to column `col` and apply its credit event.
    fn scroll_to(&mut self, col: i32, ctx: &UiContext) {
        self.col = col;
        let (pass, x) = ((col / PASS_W) as usize, col % PASS_W);
        if let Some(&(_, read)) = CREDIT_EVENTS[pass].iter().find(|e| e.0 == x) {
            self.line = if read { ctx.texts.credits.get(self.next_line).cloned().unwrap_or_default() } else { String::new() };
            if read {
                self.next_line += 1;
            }
        }
    }

    /// Picture and credit band at scroll column `col`: in pass 0 BIRTH0 comes in from the right
    /// over black, in pass 1 BIRTH1 pushes BIRTH0 out to the left.
    fn draw_scroll(&self, fb: &mut Surface, ctx: &UiContext, col: i32) {
        fb.fill(0);
        let (pass, x) = (col / PASS_W, col % PASS_W);
        let incoming = ctx.pic(if pass == 0 { "BIRTH0" } else { "BIRTH1" });
        if pass == 1 {
            let outgoing = ctx.pic("BIRTH0");
            fb.blit_part(&outgoing.surf, Rect::new(x, 0, PASS_W - x, PIC_H), 0, PIC_Y, None);
        }
        fb.blit_part(&incoming.surf, Rect::new(0, 0, x, PIC_H), PASS_W - x, PIC_Y, None);
        if !self.line.is_empty() {
            for (dy, c) in [(2, 7u8), (0, 0xf8), (1, 0xfa)] {
                draw_text_centered(fb, &ctx.fonts, &TextStyle::new(CREDIT_FONT, c), 160, CREDIT_Y + dy, &self.line);
            }
        }
    }

    /// LOGO.PIC's palette (loaded palette-only after the pictures, o 0xe2f) with the colour
    /// cycles, frozen where `cycle_stop` left them (o 0x169b).
    fn cycled_palette(&self, ctx: &UiContext) -> Palette {
        let mut pal = ctx.pic("LOGO").pal;
        let mut cycle = |(first, last): (u8, u8), ticks: u64| {
            let n = (last - first) as usize + 1;
            let phase = (ticks / CYCLE_PERIOD) as usize % n;
            let ring: Vec<[u8; 3]> = (0..n).map(|i| pal.0[first as usize + i]).collect();
            for i in 0..n {
                pal.0[first as usize + i] = ring[(i + phase) % n];
            }
        };
        if self.phase == Phase::Menu && self.cycles_stopped.is_none() {
            return pal;
        }
        let t = self.cycles_stopped.unwrap_or(self.t);
        cycle(CYCLE_STARS, t);
        if let Some(w) = self.wipe_t {
            cycle(CYCLE_LOGO, t - w);
        }
        pal
    }
}

impl Screen for TitleScreen {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        ctx.audio.play(crate::audio::Sound::MainTheme);
    }
    fn on_exit(&mut self, ctx: &mut UiContext) {
        ctx.audio.stop();
    }

    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        self.t += 1;
        match self.phase {
            Phase::Scroll => {
                while self.col + 1 < 2 * PASS_W && (21 * (self.col + 1) / 4) as u64 <= self.t {
                    self.scroll_to(self.col + 1, ctx);
                }
                if self.col + 1 == 2 * PASS_W && (21 * 2 * PASS_W / 4) as u64 <= self.t {
                    self.set_phase(Phase::Wait);
                }
            }
            Phase::Wait => {
                if self.t >= WIPE_TICK {
                    self.set_phase(Phase::Wipe);
                    self.wipe_t = Some(self.t);
                }
            }
            Phase::Wipe => {
                if (self.t - self.phase_t) as i32 * WIPE_STEP >= PASS_W {
                    self.set_phase(Phase::Hold);
                }
            }
            Phase::Hold => {
                if self.key_pending || self.t - self.phase_t >= HOLD_TICKS {
                    self.start_dissolve(ctx);
                }
            }
            Phase::Dissolve => {
                if self.reveal.as_ref().is_none_or(|r| r.done()) {
                    self.reveal = None;
                    if let Some(s) = self.shortcut.take() {
                        self.phase = Phase::Menu;
                        return Transition::Push(crate::ui::setup::start_flow(s));
                    }
                    return self.open_menu();
                }
            }
            Phase::Menu => {
                if !self.menu_up {
                    return self.open_menu();
                }
            }
        }
        Transition::None
    }

    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        let shortcut = match ev {
            Event::Key { key, down: true, .. } => {
                use crate::platform::Key;
                match key {
                    Key::Char('n') => Some(0),
                    Key::Char('l') => Some(1),
                    Key::Char('e') => Some(2),
                    Key::Char('c') => Some(3),
                    _ => None,
                }
            }
            Event::MouseButton { down: true, .. } => None,
            _ => return Transition::None,
        };
        match self.phase {
            // a key ends the scroll; N/L/E/C also pick the menu entry (o 0x1561)
            Phase::Scroll => {
                self.shortcut = shortcut;
                self.start_dissolve(ctx);
            }
            Phase::Wait | Phase::Wipe => self.key_pending = true,
            Phase::Hold => self.start_dissolve(ctx),
            Phase::Dissolve | Phase::Menu => {}
        }
        Transition::None
    }

    fn resume(&mut self, result: DialogResult, ctx: &mut UiContext) -> Transition {
        self.menu_up = false;
        if matches!(result, DialogResult::Choice(_)) {
            ctx.audio.fade_out();
        }
        match result {
            DialogResult::Choice(4) => Transition::Push(crate::ui::halloffame::hall_of_fame()),
            DialogResult::Choice(i) => Transition::Push(crate::ui::setup::start_flow(i)),
            _ => Transition::None,
        }
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let logo = ctx.pic("LOGO");
        match self.phase {
            Phase::Scroll | Phase::Wait => self.draw_scroll(fb, ctx, self.col.max(0)),
            Phase::Wipe | Phase::Hold => {
                self.draw_scroll(fb, ctx, 2 * PASS_W - 1);
                let w = if self.phase == Phase::Hold { PASS_W } else { ((self.t - self.phase_t) as i32 * WIPE_STEP).min(PASS_W) };
                fb.blit_part(&logo.surf, Rect::new(LOGO_SRC.x, LOGO_SRC.y, w, LOGO_SRC.h), 0, LOGO_Y, Some(0));
            }
            Phase::Dissolve => {
                // the last frame (picture and logo) stays in `fb` and dissolves into the logo on black
                let mut title = Surface::screen();
                title.blit_part(&logo.surf, LOGO_SRC, 0, LOGO_Y, Some(0));
                let pal = logo.pal;
                if let Some(r) = self.reveal.as_mut() {
                    r.draw(fb, &title, &pal, &pal);
                }
            }
            Phase::Menu => {
                fb.fill(0);
                fb.blit_part(&logo.surf, LOGO_SRC, 0, LOGO_Y, Some(0));
            }
        }
    }

    fn palette(&self, ctx: &UiContext) -> Palette {
        self.cycled_palette(ctx)
    }

    fn draws_parent(&self) -> bool {
        false
    }

    fn wants_cursor(&self) -> bool {
        self.phase == Phase::Menu
    }
}

/// Start-up entry: goes straight into the title sequence.
pub struct Splash;

impl Splash {
    pub fn new() -> TitleScreen {
        TitleScreen::new()
    }
}
