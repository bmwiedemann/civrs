//! Spaceship screens: the ship itself (CIV.EXE overlay 18: docker.pic parts on the grid, the
//! statistics column, the LAUNCH button, the WORLD menu's "Which nation?" viewer and the
//! launch / AI part notices), and the arrival (overlay 2 o 0x174c): the PLANET1.PIC starfield
//! sliding in with the rotating planet (PLANET2.PIC frames) and the KING.TXT SPACE (human) /
//! SPACE2 (rival) text, ending on the colony (SPACEST.PIC) or, for a rival's ship, the
//! human's palace in ruins (SAD.PIC).

use crate::assets::text::{substitute_line, Vars};
use crate::assets::Palette;
use crate::game::spaceship::{self, PARTS, PLACEMENT};
use crate::game::{Game, Slot};
use crate::gfx::text::{draw_text, draw_text_centered};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::{Event, Key};
use crate::rules;
use crate::ui::context::UiContext;
use crate::ui::eventpics::Dismiss;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::MenuDialog;

/// PLANET2.PIC: 24 frames of 49x49 in 50-px cells, six to a row (o 0x1781..0x17c9).
fn planet_frame(i: i32) -> Rect {
    Rect::new((i % 6) * 50 + 1, (i / 6) * 50 + 1, 49, 49)
}

/// Background colour of the approach on VGA (o 0x17d5..0x17e3; 0 on EGA).
const VOID: u8 = 0xec;
/// Ticks per scroll step (o 0x1c33).
const STEP_TICKS: u32 = 8;
/// Scroll steps: the scene slides in from the right one pixel per step (o 0x1c41).
const STEPS: i32 = 320;
/// A new line of the text every 22 steps (o 0x1cae).
const LINE_STEPS: i32 = 22;
/// The ending's lines: the marker pixel at (280, 32) is lit for 80 ticks, then dark for 80
/// (o 0x1f8d..0x1fd1).
const PAGE_TICKS: u32 = 80;
/// Colour cycles of the human's colony (o 0x1e16..0x1ea3, cycle_define(slot, period, first,
/// last)): (first, last, period in ticks).
const COLONY_CYCLES: [(u8, u8, u64); 6] = [(0x10, 0x1f, 15), (0x20, 0x22, 100), (0x23, 0x28, 42), (0x29, 0x2f, 300), (0x30, 0x35, 60), (0x36, 0x3f, 37)];

#[derive(PartialEq, Clone, Copy)]
enum Phase {
    /// The starfield and the planet slide in while the ship's dot crosses and the first lines
    /// of the text appear in the strip at y 32.
    Approach,
    /// The rest of the text, a line at a time, at the top of the ending picture.
    Lines,
    /// All shown: a key or a click ends it.
    Wait,
}

/// `spaceship_arrival(civ)` (overlay 2 o 0x174c), VGA path. The KING.TXT text (*SPACE for
/// the human's ship, *SPACE2 for a rival's) is cut at its '^' marks: during the approach a
/// new piece every 22 steps (the first is the empty line before the first '^'), the rest
/// on the ending picture: the human's colony SPACEST.PIC with six colour cycles, or for a
/// computer's ship the human's palace in ruins (palace_draw(1), o 0x16761: the palace on
/// SAD.PIC, in SAD.PIC's palette). Neither phase reads the keyboard; the final key wait
/// (0x13d7:0x91e) closes it.
pub struct ArrivalScreen {
    human_ship: bool,
    phase: Phase,
    /// The original's page 1: PLANET1.PIC moved up 24 rows, with its stars at rows 56..79
    /// copied to the strip at rows 176..199 that carries the text.
    page: Surface,
    screen: Surface,
    /// The text (msgbuf) and the scan position in it.
    text: Vec<u8>,
    pos: i32,
    start: i32,
    step: i32,
    ticks: u32,
    /// [bp-0x242]: x of the ship's dot plus the step.
    dot: i32,
    /// [bp-0x208]: planet frame.
    frame: i32,
    /// Ticks since the colony's cycles started.
    t: u64,
    dismiss: Dismiss,
}

pub fn arrival_screen(ctx: &UiContext, slot: Slot) -> Box<dyn Screen> {
    let mut text = String::new();
    let mut human_ship = true;
    if let Some(g) = ctx.game.as_ref() {
        human_ship = slot == g.human_slot;
        let s = g.player(slot).spaceship;
        // $US = the human's title and name (ds:0x19b2[govt], ds:0x19a2), $THEM the ship's
        // nation's adjective, $BUCKS its colonists (habitation modules, ds:0xdeec), $RPLC1 the
        // launch year ds:0xe8a8 and $RPLC2 this year ds:0xb220, as plain numbers (itoa)
        let v = Vars {
            rplc1: s.launch_year.unwrap_or_else(|| g.year()).to_string(),
            rplc2: g.year().to_string(),
            us: format!("{} {}", g.human().title().trim_end(), g.human().leader),
            them: g.player(slot).adjective().to_string(),
            bucks: g.spaceship_stats(slot).habitation(),
        };
        let section = if human_ship { "SPACE" } else { "SPACE2" };
        text = ctx.texts.king.entry_text(section).map(|t| substitute_line(&t, &v)).unwrap_or_default();
    }
    // CIV.EXE clears msgbuf's first byte after copying it (o 0x1a3f) but goes on scanning and
    // drawing msgbuf: that byte is the '^' of the text's empty first line, so its first piece
    // runs on to "In the year ..." and shows as an empty string, and that line is never seen.
    // civrs keeps the '^': an empty first piece, then "In the year ...".
    Box::new(ArrivalScreen::new(text.into_bytes(), human_ship))
}

impl ArrivalScreen {
    fn new(text: Vec<u8>, human_ship: bool) -> ArrivalScreen {
        let mut screen = Surface::new(320, 200);
        screen.fill(VOID);
        ArrivalScreen {
            human_ship,
            phase: Phase::Approach,
            page: Surface::new(320, 200),
            screen,
            text,
            pos: -1,
            start: -1,
            step: 0,
            ticks: 0,
            dot: -10,
            frame: 0,
            t: 0,
            dismiss: Dismiss::new(),
        }
    }
}

/// copy(src, sx, sy, w, h -> dst, dx, dy) (driver 0:0x7fb).
fn copy(dst: &mut Surface, src: &Surface, sx: i32, sy: i32, w: i32, h: i32, dx: i32, dy: i32) {
    dst.blit_part(src, Rect::new(sx, sy, w, h), dx, dy, None);
}

impl ArrivalScreen {
    /// The next '^'-terminated piece of the text (o 0x1cbc..0x1ce4, 0x1ef3..0x1f21): None
    /// once the scan runs off the end; the piece ends at the first 0 after its start.
    fn next_piece(&mut self) -> Option<String> {
        self.start = self.pos + 1;
        let len = self.text.len() as i32;
        loop {
            self.pos += 1;
            if self.pos >= len || self.text[self.pos as usize] == b'^' {
                break;
            }
        }
        if self.pos >= len {
            return None;
        }
        self.text[self.pos as usize] = 0;
        Some(self.piece())
    }

    fn piece(&self) -> String {
        let s = &self.text[(self.start.max(0) as usize).min(self.text.len())..];
        let end = s.iter().position(|b| *b == 0).unwrap_or(s.len());
        String::from_utf8_lossy(&s[..end]).into_owned()
    }

    /// One step of the approach (o 0x1c4c..0x1c3d).
    fn approach_step(&mut self, ctx: &UiContext) {
        let i = self.step;
        let snapshot = self.page.clone();
        self.page.fill_rect(Rect::new(0, 176, 320 - i, 24), VOID);
        if i != 0 {
            copy(&mut self.page, &snapshot, 0, 20, i, 24, 320 - i, 176);
        }
        if i % LINE_STEPS == 0 {
            // the piece shown stays while the scan has not run off the end
            let _ = self.next_piece();
        }
        if (self.pos as usize) < self.text.len() {
            let line = self.piece();
            draw_text_centered(&mut self.page, &ctx.fonts, &TextStyle::new(5, 0x38), 160, 184, &line);
            draw_text_centered(&mut self.page, &ctx.fonts, &TextStyle::new(5, 0x3c), 160, 183, &line);
        }
        self.frame += 1;
        if self.frame > 22 {
            self.frame = 0;
        }
        let planet = ctx.pic("PLANET2");
        self.page.blit_part(&planet.surf, planet_frame(self.frame), 135, 51, Some(0));
        let s = &mut self.screen;
        s.fill_rect(Rect::new(0, 12, 319, 20), VOID);
        if i != 0 {
            copy(s, &self.page, 0, 0, i, 20, 320 - i, 12);
        }
        copy(s, &self.page, 0, 176, 320, 24, 0, 32);
        s.fill_rect(Rect::new(0, 56, 320 - i, 132), VOID);
        if i != 0 {
            copy(s, &self.page, 0, 44, i, 132, 320 - i, 56);
        }
        // the ship: a dot at y 92 on odd steps, half a pixel further each step
        if i & 1 != 0 {
            self.dot += 1;
            s.put(self.dot - i, 92, 15);
        }
        self.dot += 1;
    }

    /// The end of the approach (o 0x1d14..0x1ef0): the rest of the text, and the ending
    /// picture on page 1 and the screen.
    fn ending(&mut self, ctx: &UiContext) {
        let rest = self.text.get((self.pos + 1).max(0) as usize..).unwrap_or(&[]);
        let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
        self.text = rest[..end].to_vec();
        self.pos = -1;
        self.page = Surface::new(320, 200);
        if self.human_ship {
            self.page.blit(&ctx.pic("SPACEST").surf, 0, 0);
            ctx.audio.play(crate::audio::Sound::Win);
        } else if let Some(g) = ctx.game.as_ref() {
            crate::ui::palace::draw_palace_on(&mut self.page, ctx, &g.human().palace(), "SAD");
        }
        self.screen = self.page.clone();
        self.phase = Phase::Lines;
        self.ticks = 0;
        self.next_line(ctx);
    }

    /// Show the next line of the ending (o 0x1f33..0x1fa2), or wait for the key.
    fn next_line(&mut self, ctx: &UiContext) {
        match self.next_piece() {
            Some(line) => {
                copy(&mut self.screen, &self.page, 0, 0, 320, 32, 0, 0);
                draw_text_centered(&mut self.screen, &ctx.fonts, &TextStyle::new(5, 0xb), 160, 5, &line);
                draw_text_centered(&mut self.screen, &ctx.fonts, &TextStyle::new(5, 0xf), 160, 4, &line);
                self.screen.put(280, 32, 15);
            }
            None => {
                self.phase = Phase::Wait;
                self.dismiss = Dismiss::new();
            }
        }
    }
}

impl Screen for ArrivalScreen {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        // PLANET1 moved up 24 rows, its rows 56..79 again at 176..199 (o 0x1800..0x1856)
        let p1 = ctx.pic("PLANET1");
        copy(&mut self.page, &p1.surf, 0, 24, 320, 176, 0, 0);
        copy(&mut self.page, &p1.surf, 0, 56, 320, 24, 0, 176);
        self.approach_step(ctx);
    }
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        self.dismiss.tick();
        self.ticks += 1;
        match self.phase {
            Phase::Approach if self.ticks >= STEP_TICKS => {
                self.ticks = 0;
                self.step += 1;
                if self.step < STEPS {
                    self.approach_step(ctx);
                } else {
                    self.ending(ctx);
                }
            }
            Phase::Lines => {
                self.t += 1;
                if self.ticks == PAGE_TICKS {
                    self.screen.put(280, 32, 0);
                } else if self.ticks >= 2 * PAGE_TICKS {
                    self.ticks = 0;
                    self.next_line(ctx);
                }
            }
            Phase::Wait => self.t += 1,
            _ => {}
        }
        Transition::None
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if self.phase == Phase::Wait && self.dismiss.closes(ev) {
            ctx.audio.play(crate::audio::Sound::FadeOut);
            Transition::Pop(DialogResult::Done)
        } else {
            Transition::None
        }
    }
    fn draw(&mut self, fb: &mut Surface, _ctx: &mut UiContext) {
        fb.blit(&self.screen, 0, 0);
    }
    /// PLANET1's palette for the approach (load_pic_pal 0x1f20:0x216; PLANET2's frames show in
    /// it unconverted), then SPACEST's with its cycles turning, or SAD's.
    fn palette(&self, ctx: &UiContext) -> Palette {
        match self.phase {
            Phase::Approach => ctx.pic_pal("PLANET1"),
            _ if !self.human_ship => ctx.pic_pal("SAD"),
            _ => {
                let base = ctx.pic_pal("SPACEST");
                let mut pal = base;
                for (first, last, period) in COLONY_CYCLES {
                    let n = (last - first + 1) as u64;
                    let k = (self.t / period) % n;
                    for i in 0..n {
                        pal.0[(first as u64 + i) as usize] = base.0[(first as u64 + (i + k) % n) as usize];
                    }
                }
                pal
            }
        }
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

// ---- the ship screen (overlay 18) ---------------------------------------------------------------

/// Palette cycles the ship screen starts (o 0x17645..0x176db, `cycle_define(slot, period,
/// first, last)` then `cycle_start(+slot)` on VGA): (first, last, period in ticks).
const CYCLES: [(u8, u8, u64); 5] = [(0xd0, 0xdf, 30), (0xc0, 0xcf, 11), (0xe0, 0xef, 20), (0x90, 0x9f, 8), (0x80, 0x81, 150)];

/// Stars of the ship screen (ds:0x683e x, ds:0x6852 y, ds:0x6866 drawn).
const STARS: usize = 20;

/// The statistics column (x 240..320) and the LAUNCH button, outlined (242,188)-(318,197)
/// (o 0x1801d..0x1804a).
const STATS_X: i32 = 242;
const LAUNCH_BOX: Rect = Rect { x: 242, y: 188, w: 77, h: 10 };

#[derive(Clone, Copy, PartialEq)]
enum Launch {
    /// No LAUNCH button (ship_draw's `allow_launch` = 0).
    No,
    /// The WORLD menu's viewer (o 0x18a9c): a confirmed launch happens here.
    Here,
    /// The prompt after a part (o 0x18842): a confirmed launch answers the decision with
    /// `Choice(1)`, which launches.
    Answer,
}

/// ship_screen_open + ship_draw (o 0x17575, 0x177ce): the docker.pic sprites of the ship's
/// parts on the grid, the statistics column and the drifting stars, until a key or a click
/// (`timeout` None, ship_draw's -1) or for `timeout` star frames. With a LAUNCH button a click
/// on it (x > 242, y >= 190) or the key 'l' asks "Confirm spaceship launch." (o 0x180d8).
pub struct ShipScreen {
    slot: Slot,
    timeout: Option<u32>,
    launch: Launch,
    /// Shown over the ship once its time is up (the "Type?" question of o 0x1853f).
    then: Option<Box<dyn Screen>>,
    waiting_then: bool,
    confirming: bool,
    page: Option<Surface>,
    stars: [(u8, u8, bool); STARS],
    /// ds:0x687a, bumped by every star step.
    frame: u32,
    t: u64,
    seed: u32,
}

impl ShipScreen {
    fn new(g: &Game, slot: Slot, timeout: Option<u32>, launch: Launch) -> ShipScreen {
        let mut s = ShipScreen {
            slot,
            timeout,
            launch,
            then: None,
            waiting_then: false,
            confirming: false,
            page: None,
            stars: [(0, 0, false); STARS],
            frame: 0,
            t: 0,
            seed: 0x2545_f491 ^ (g.turn << 8) ^ slot as u32,
        };
        // x = random(240), y = random(192) + 8 (o 0x17605); from a generator of the screen's
        // own, not the game's
        for i in 0..STARS {
            let x = s.random(240) as u8;
            let y = s.random(192) as u8 + 8;
            s.stars[i] = (x, y, false);
        }
        s
    }

    fn random(&mut self, n: u32) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed % n
    }

    /// The LAUNCH button is up: the human's own, unlaunched ship with a habitation module,
    /// waiting for input (`timeout` < 0) and `allow_launch` (o 0x17fa1, 0x18009..0x1801b).
    fn button(&self, g: &Game) -> bool {
        self.launch != Launch::No && self.timeout.is_none() && self.slot == g.human_slot && g.spaceship_can_launch(self.slot)
    }

    /// Draw the static picture (o 0x177fb..0x1804f).
    fn render(&self, ctx: &UiContext) -> Surface {
        let g = ctx.game();
        let mut fb = Surface::new(320, 200);
        fb.fill(0);
        if !g.ship_drawn(self.slot) {
            return fb;
        }
        let p = g.player(self.slot);
        let s = p.spaceship;
        let st = g.spaceship_stats(self.slot);
        let civ_col = rules::SLOT_COLOURS[(self.slot & 7) as usize].light;
        let fonts = &ctx.fonts;
        let text = |fb: &mut Surface, x: i32, y: i32, s: &str, c: u8| {
            draw_text(fb, fonts, &TextStyle::new(0, c), x, y, s);
        };
        // title bar in the civ's colour: "<Adjective> SpaceShip: X.S.S. <Leader>"
        fb.fill_rect(Rect::new(0, 0, 320, 8), civ_col);
        let adj = p.adjective();
        let initial = adj.chars().next().unwrap_or('X');
        let title = format!("{adj} SpaceShip: {initial}.S.S. {}", p.leader);
        draw_text_centered(&mut fb, fonts, &TextStyle::new(0, 0), 160, 1, &title);
        // the parts, unattached ones outlined in colour 12 (o 0x178b8..0x17a57)
        let docker = ctx.pic("DOCKER");
        for &(x, y, _) in PLACEMENT.iter() {
            let (x, y) = (x as i32, y as i32);
            let Some(t) = s.part_at(x, y) else { continue };
            let (px, py) = (16 * x - 16, 16 * y + 8);
            let (sx, sy, w, h) = PARTS[t as usize].sprite;
            fb.blit_part(&docker.surf, Rect::new(sx, sy, w, h), px, py, Some(0));
            if !spaceship::part_attached(&s, x, y, t) {
                let (fw, fh) = spaceship::footprint(t);
                fb.rect_outline(Rect::new(px, py, 16 * fw, 16 * fh), 12);
            }
        }
        // the capsule once the mass reaches 8,000 tons (o 0x17a85)
        if st.mass >= 80 {
            let (sx, sy, w, h) = PARTS[5].sprite;
            fb.blit_part(&docker.surf, Rect::new(sx, sy, w, h), 152, 88, Some(0));
        }
        fb.fill_rect(Rect::new(240, 8, 80, 192), civ_col);
        fb.vline(0, 8, 199, civ_col);
        fb.hline(0, 240, 199, civ_col);
        // the statistics column, red (4) below 100% (o 0x17b07..0x17f9d)
        let red = |v: i32| if v < 100 { 4 } else { 0 };
        let x = STATS_X;
        text(&mut fb, x, 16, "Population:", 0);
        text(&mut fb, x, 24, &format!("{}0,000", st.habitation()), 0);
        text(&mut fb, x, 32, &format!("Support: {}%", st.support), red(st.support));
        text(&mut fb, x, 40, &format!("Energy: {}%", st.energy), red(st.energy));
        text(&mut fb, x, 56, "Mass:", 0);
        text(&mut fb, x, 64, &format!("{},{}00 tons", st.mass / 10, st.mass % 10), 0);
        text(&mut fb, x, 72, &format!("Fuel: {}%", st.fuel), red(st.fuel));
        text(&mut fb, x, 80, "Flight Time:", 0);
        text(&mut fb, x, 88, &format!("{}.{} Years", st.flight / 10, st.flight % 10), 0);
        text(&mut fb, x, 104, "Prob. of Success", 0);
        text(&mut fb, x, 112, &format!("--- {}% ---", st.success), 0);
        if s.launched() {
            // "Landed!" once the arrival year has passed (o 0x17fd3)
            let landed = s.arrival_year.is_none_or(|a| a < g.year());
            text(&mut fb, x, 128, if landed { "Landed!" } else { "Launched!" }, 0);
        } else if self.button(g) {
            text(&mut fb, 256, 190, "LAUNCH", 0);
            fb.rect_outline(LAUNCH_BOX, 0);
        }
        fb
    }

    /// ship_animate_stars (o 0x181be): erase, move (every 4th star 2 px in colour 7, the next
    /// 1 px in colour 3, the other two 1 px every other frame in colour 8), wrap at x 240 to a
    /// new row, and draw where the picture is empty (colour 0).
    fn step_stars(&mut self) {
        let Some(mut page) = self.page.take() else { return };
        for i in 0..STARS {
            let (mut x, mut y, shown) = self.stars[i];
            if shown {
                page.put(x as i32, y as i32, 0);
            }
            let colour = match i & 3 {
                0 => {
                    x = x.wrapping_add(2);
                    7
                }
                1 => {
                    x = x.wrapping_add(1);
                    3
                }
                _ => {
                    if self.frame & 1 != 0 {
                        x = x.wrapping_add(1);
                    }
                    8
                }
            };
            if x >= 240 {
                x = 1;
                y = self.random(192) as u8 + 8;
            }
            let free = page.get(x as i32, y as i32) == 0;
            if free {
                page.put(x as i32, y as i32, colour);
            }
            self.stars[i] = (x, y, free);
        }
        self.frame += 1;
        self.page = Some(page);
    }

    /// The loop is over: show what follows, or close.
    fn finish(&mut self) -> Transition {
        match self.then.take() {
            Some(s) => {
                self.waiting_then = true;
                Transition::Push(s)
            }
            None => Transition::Pop(DialogResult::Done),
        }
    }
}

/// "Confirm spaceship launch. (NN% chance of success)" at (80,100) (o 0x1810d).
pub fn launch_confirm(g: &Game, slot: Slot) -> MenuDialog {
    let title = vec!["Confirm spaceship launch.".into(), format!("({}% chance of success)", g.success_chance(slot))];
    MenuDialog::new(title, vec!["No Launch".into(), "Launch CONFIRMED!".into()]).with_pos(80, 100)
}

impl Screen for ShipScreen {
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        if self.waiting_then || self.confirming {
            return Transition::None;
        }
        // nothing is drawn for another civ's ship without Enemy Moves: ship_draw returns at once
        if !ctx.game().ship_drawn(self.slot) {
            return self.finish();
        }
        if self.page.is_none() {
            self.page = Some(self.render(ctx));
        }
        self.t += 1;
        // one star step per wait_ticks(4), (2) for a launched ship (o 0x18076)
        let period = if ctx.game().player(self.slot).spaceship.launched() { 2 } else { 4 };
        if self.t.is_multiple_of(period) {
            self.step_stars();
            if let Some(n) = self.timeout.as_mut() {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    return self.finish();
                }
            }
        }
        Transition::None
    }

    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if self.waiting_then || self.confirming || self.page.is_none() {
            return Transition::None;
        }
        let ends = match ev {
            Event::Key { down: true, key, .. } => !matches!(key, Key::Unknown),
            Event::MouseButton { down: true, .. } => true,
            _ => false,
        };
        if !ends {
            return Transition::None;
        }
        let g = ctx.game();
        if self.button(g) {
            let on_button = matches!(ev, Event::MouseButton { x, y, .. } if *x > STATS_X && *y >= 190);
            let key_l = matches!(ev, Event::Key { key: Key::Char('l'), mods, .. } if !mods.shift);
            if on_button || key_l {
                self.confirming = true;
                return Transition::Push(Box::new(launch_confirm(g, self.slot)));
            }
        }
        self.finish()
    }

    fn resume(&mut self, result: DialogResult, ctx: &mut UiContext) -> Transition {
        if self.waiting_then {
            self.waiting_then = false;
            return Transition::Pop(result);
        }
        if !std::mem::take(&mut self.confirming) {
            return Transition::None;
        }
        if result != DialogResult::Choice(1) {
            return Transition::Pop(DialogResult::Done);
        }
        if self.launch == Launch::Here {
            // ship_launch (o 0x18b38); its screens follow from the SpaceshipLaunched event
            let _ = ctx.game_mut().launch_spaceship(self.slot);
        }
        Transition::Pop(DialogResult::Choice(1))
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        if self.page.is_none() && ctx.game().ship_drawn(self.slot) {
            self.page = Some(self.render(ctx));
        }
        match &self.page {
            Some(p) => fb.blit(p, 0, 0),
            None => fb.fill(0),
        }
    }

    /// docker.pic's palette with the five colour cycles turning: every `period` ticks each
    /// range moves on by one colour (cycle_step, 0x0:0x44a; the sense of the rotation on
    /// screen is not checked against a capture).
    fn palette(&self, ctx: &UiContext) -> Palette {
        let base = ctx.pic_pal("DOCKER");
        let mut pal = base;
        for (first, last, period) in CYCLES {
            let n = (last - first + 1) as u64;
            let k = (self.t / period) % n;
            for i in 0..n {
                pal.0[(first as u64 + i) as usize] = base.0[(first as u64 + (i + k) % n) as usize];
            }
        }
        pal
    }

    fn draws_parent(&self) -> bool {
        false
    }
}

/// The ship screen after a human's part (o 0x18842): waits with the LAUNCH button; its
/// result answers `Decision::SpaceshipLaunch` (`Choice(1)` = launch).
pub fn launch_prompt(ctx: &UiContext, slot: Slot) -> Box<dyn Screen> {
    Box::new(ShipScreen::new(ctx.game(), slot, None, Launch::Answer))
}

/// The ship shown for 15 star frames (ship_draw(civ, 15, 0), o 0x1851e) before `question`
/// ("Type?") opens over it; the question's result is passed on.
pub fn part_question(ctx: &UiContext, slot: Slot, question: Box<dyn Screen>) -> Box<dyn Screen> {
    let mut s = ShipScreen::new(ctx.game(), slot, Some(15), Launch::No);
    s.then = Some(question);
    Box::new(s)
}

/// The ship as ship_launch and the AI's part show it (ship_draw(civ, -1, 0)): no button, until
/// a key or a click; nothing for another civ's ship without Enemy Moves.
pub fn ship_view(ctx: &UiContext, slot: Slot) -> Box<dyn Screen> {
    Box::new(ShipScreen::new(ctx.game(), slot, None, Launch::No))
}

/// The viewer's ship screen for `slot` (ship_draw(civ, -1, 1)).
pub fn viewer_ship(ctx: &UiContext, slot: Slot) -> Box<dyn Screen> {
    Box::new(ShipScreen::new(ctx.game(), slot, None, Launch::Here))
}

/// The WORLD menu's SpaceShips viewer (o 0x18a9c, ship_view_any_nation; F12 here): when any civ
/// has started a ship, "Which nation?" at (100,80) lists the seven civs, those without a ship
/// greyed (ds:0xb276 = ~(ds:0x4cd8 >> 9)), and shows the chosen one's ship with the LAUNCH
/// button (ship_draw(civ, -1, 1)). Nothing happens when no ship has been started.
pub fn spaceship_viewer() -> Box<dyn Screen> {
    Box::new(Viewer { asked: false })
}

/// Any civ 1..7 has started a ship (ds:0x4cd8 & 0xfe00; the WORLD menu greys SpaceShips
/// otherwise, 0x1c710).
pub fn any_ship_started(g: &Game) -> bool {
    (1..8u8).any(|s| g.player(s).spaceship.parts() > 0)
}

/// "Which nation?" (o 0x18aad..0x18b0d).
pub fn nation_menu(g: &Game) -> MenuDialog {
    let items: Vec<String> = (1..8u8).map(|s| g.player(s).adjective().to_string()).collect();
    let greyed: Vec<bool> = (1..8u8).map(|s| g.player(s).spaceship.parts() == 0).collect();
    MenuDialog::new(vec!["Which nation?".into()], items).with_pos(100, 80).with_disabled(greyed)
}

struct Viewer {
    asked: bool,
}

impl Screen for Viewer {
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        if self.asked {
            return Transition::None;
        }
        if !any_ship_started(ctx.game()) {
            return Transition::Pop(DialogResult::Done);
        }
        self.asked = true;
        Transition::Push(Box::new(nation_menu(ctx.game())))
    }
    fn handle(&mut self, _ev: &Event, _ctx: &mut UiContext) -> Transition {
        Transition::None
    }
    fn resume(&mut self, result: DialogResult, ctx: &mut UiContext) -> Transition {
        match result {
            DialogResult::Choice(i) if i < 7 => Transition::Replace(viewer_ship(ctx, i as Slot + 1)),
            _ => Transition::Pop(DialogResult::Done),
        }
    }
    fn draw(&mut self, _fb: &mut Surface, _ctx: &mut UiContext) {}
}

/// Lines centred in colour 15 on a cleared screen until a key or a click: "<Plural> LAUNCH
/// space ship!" / "Estimate arrival in <year>." at y 92/100 (o 0x18b6d, shown with docker.pic's
/// palette) and "<Plural> build space ship." at y 96 (o 0x1842b, over the map's black).
pub struct Notice {
    lines: Vec<(i32, String)>,
    docker: bool,
    dismiss: Dismiss,
}

impl Screen for Notice {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.dismiss.tick();
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.dismiss.closes(ev) {
            return Transition::Pop(DialogResult::Done);
        }
        Transition::None
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        fb.fill(0);
        for (y, l) in &self.lines {
            draw_text_centered(fb, &ctx.fonts, &TextStyle::new(0, 15), 160, *y, l);
        }
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        if self.docker {
            ctx.pic_pal("DOCKER")
        } else {
            ctx.game_palette()
        }
    }
    fn draws_parent(&self) -> bool {
        false
    }
}

/// ship_launch's screens (o 0x18b38): the notice, then (for a ship ship_draw shows) the ship
/// with "Launched!". The arrival year is printed bare (itoa, no BC/AD).
pub fn launch_screens(ctx: &UiContext, slot: Slot) -> (Box<dyn Screen>, Option<Box<dyn Screen>>) {
    let g = ctx.game();
    let arrival = g.player(slot).spaceship.arrival_year.unwrap_or(g.year());
    let lines = vec![(92, format!("{} LAUNCH space ship!", g.player(slot).plural())), (100, format!("Estimate arrival in {arrival}."))];
    let notice: Box<dyn Screen> = Box::new(Notice { lines, docker: true, dismiss: Dismiss::new() });
    (notice, g.ship_drawn(slot).then(|| ship_view(ctx, slot)))
}

/// An AI civ's part (o 0x1842b..0x184ec): "<Plural> build space ship.", then its ship when
/// Enemy Moves is on. civrs shows them when the event comes up, so the ship may already
/// carry parts placed later in the same turn.
pub fn ai_part_screens(ctx: &UiContext, slot: Slot) -> (Box<dyn Screen>, Option<Box<dyn Screen>>) {
    let g = ctx.game();
    let lines = vec![(96, format!("{} build space ship.", g.player(slot).plural()))];
    let notice: Box<dyn Screen> = Box::new(Notice { lines, docker: false, dismiss: Dismiss::new() });
    (notice, g.ship_drawn(slot).then(|| ship_view(ctx, slot)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{GameEvent, GameSetup};
    use crate::platform::{Mods, MouseButton};
    use crate::ui::screen::ScreenStack;

    /// Stands under the viewer so that closing it leaves the stack non-empty.
    struct Base;
    impl Screen for Base {
        fn handle(&mut self, _ev: &Event, _ctx: &mut UiContext) -> Transition {
            Transition::None
        }
        fn draw(&mut self, _fb: &mut Surface, _ctx: &mut UiContext) {}
    }

    fn key(k: Key) -> Event {
        Event::Key { key: k, mods: Mods::default(), down: true, repeat: false }
    }

    /// A context with the human's ship ready to launch, and the viewer opened over `Base`.
    fn viewer() -> Option<(UiContext, ScreenStack)> {
        let dir = crate::assets::AssetDir::locate(None).ok()?;
        let mut ctx = UiContext::new(dir, std::env::temp_dir().join(format!("civrs-shipui-{}", std::process::id()))).ok()?;
        let mut g = Game::new(GameSetup { seed: 3, ..Default::default() }, None);
        let h = g.human_slot;
        g.apollo_built = true;
        g.set_turn(370);
        for _ in 0..3 {
            g.place_spaceship_part(h, 0, 0);
        }
        g.place_spaceship_part(h, 2, 0);
        let _ = g.take_events();
        ctx.game = Some(g);
        let mut st = ScreenStack::new();
        st.push(Box::new(Base), &mut ctx);
        st.push(spaceship_viewer(), &mut ctx);
        // "Which nation?": only the human's line is open, Enter takes it
        st.update(&mut ctx);
        st.handle(&key(Key::Enter), &mut ctx);
        st.update(&mut ctx);
        Some((ctx, st))
    }

    #[test]
    fn viewer_launches_with_l_and_confirmation() {
        let Some((mut ctx, mut st)) = viewer() else { return };
        let h = ctx.game().human_slot;
        assert_eq!(st.len(), 2, "the ship screen replaced the viewer");
        st.handle(&key(Key::Char('l')), &mut ctx);
        assert_eq!(st.len(), 3, "the confirmation is up");
        st.handle(&key(Key::Down), &mut ctx);
        st.handle(&key(Key::Enter), &mut ctx);
        assert_eq!(st.len(), 1);
        let g = ctx.game_mut();
        assert!(g.player(h).spaceship.launched());
        assert_eq!(g.player(h).spaceship.launch_year, Some(1790));
        assert!(g.take_events().iter().any(|e| *e == GameEvent::SpaceshipLaunched(h)));
    }

    #[test]
    fn viewer_closes_on_other_input_and_on_no_launch() {
        let Some((mut ctx, mut st)) = viewer() else { return };
        let h = ctx.game().human_slot;
        // a click away from the button closes the screen
        st.handle(&Event::MouseButton { button: MouseButton::Left, x: 100, y: 100, down: true }, &mut ctx);
        assert_eq!(st.len(), 1);
        let Some((mut ctx, mut st)) = viewer() else { return };
        // a click on the button asks; "No Launch" closes without launching
        st.handle(&Event::MouseButton { button: MouseButton::Left, x: 260, y: 192, down: true }, &mut ctx);
        assert_eq!(st.len(), 3);
        st.handle(&key(Key::Enter), &mut ctx);
        assert_eq!(st.len(), 1);
        assert!(!ctx.game().player(h).spaceship.launched());
    }

    #[test]
    fn arrival_text_pieces() {
        // the text as load_text_entry gives *SPACE: an empty first line, '^' + blank after each
        let mut a = ArrivalScreen::new(b"^ In the year 2010^ voyagers^ ".to_vec(), false);
        assert_eq!(a.next_piece().as_deref(), Some(""));
        assert_eq!(a.next_piece().as_deref(), Some(" In the year 2010"));
        assert_eq!(a.next_piece().as_deref(), Some(" voyagers"));
        assert_eq!(a.next_piece(), None);
    }

    #[test]
    fn viewer_does_nothing_without_ships() {
        let Some(dir) = crate::assets::AssetDir::locate(None).ok() else { return };
        let Ok(mut ctx) = UiContext::new(dir, std::env::temp_dir().join("civrs-shipui-none")) else { return };
        ctx.game = Some(Game::new(GameSetup { seed: 3, ..Default::default() }, None));
        let mut st = ScreenStack::new();
        st.push(Box::new(Base), &mut ctx);
        st.push(spaceship_viewer(), &mut ctx);
        st.update(&mut ctx);
        assert_eq!(st.len(), 1);
    }
}
