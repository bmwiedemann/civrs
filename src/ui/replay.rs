//! The end-of-game histories: the power graph (ovl 12 +0x573, o 0x106e8) and the replay
//! (overlay 9, o 0xcb75) of `end_game_scores` (0x2c20), which the GAME menu also offers once a
//! finished game goes on ("View Replay").

use crate::game::replay::{kind, tribe_names, Record};
use crate::game::{Game, Map, Slot};
use crate::gfx::text::{draw_text, TextStyle};
use crate::gfx::{Rect, Surface};
use crate::platform::{Event, Key};
use crate::rules::{self, SLOT_COLOURS};
use crate::ui::context::UiContext;
use crate::ui::reports::{fit_text, FONT};
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::{MenuDialog, MessageBox};

/// Window font 2 (the small digits of the year axis).
const SMALL: usize = 1;

/// A slot's colour (ds:0x1946).
fn colour(s: Slot) -> u8 {
    SLOT_COLOURS[s as usize & 7].light
}

/// "4000BC" / "1000AD": a year as the power graph's axis prints it (no blank).
fn axis_year(year: i32) -> String {
    format!("{}{}", year.abs(), if year < 0 { "BC" } else { "AD" })
}

// ---- power graph ---------------------------------------------------------------------------

/// `power_graph()` (o 0x106e8) on a colour-8 screen: a black frame (4, 9)..(315, 192) with a
/// black line every 50 turns up to the current turn (and turn 600), the year under every
/// second one in font 2; in font 1 the adjective of each civilization in the game (active, or
/// one of the first ds:0xd2f4 + 1) at (8, 8 * civ + 4) in its colour, "CIVILIZATION POWER"
/// shadowed at (100, 2), and each one's power history as a line 2 px per row, scaled so the
/// highest value of all (at least 50) is 180 px. Any key returns.
pub struct PowerGraph;

impl PowerGraph {
    pub fn render(fb: &mut Surface, ctx: &UiContext, g: &Game) {
        fb.fill(8);
        fb.rect_outline(Rect::new(4, 9, 312, 184), 0);
        let mut t = 0;
        while t <= g.turn && t <= 600 {
            let x = t as i32 / 2;
            fb.line(x + 4, 9, x + 4, 192, 0);
            if t % 100 == 0 {
                draw_text(fb, &ctx.fonts, &TextStyle::new(SMALL, 15), x, 194, &axis_year(rules::year_for_turn(t)));
            }
            t += 50;
        }
        let rows = (g.turn as usize / 4).min(crate::game::replay::POWER_ROWS);
        let civs: Vec<Slot> = (1..8u8).filter(|s| g.player(*s).in_game).collect();
        let mut max = 50u32;
        for &s in &civs {
            draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, colour(s)), 8, 8 * s as i32 + 4, g.player(s).adjective());
            for i in 0..rows {
                max = max.max(g.history.power_at(i, s) as u32);
            }
        }
        draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, 0), 100, 3, "CIVILIZATION POWER");
        draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, 15), 100, 2, "CIVILIZATION POWER");
        for &s in &civs {
            let (mut px, mut py) = (0, 0);
            for i in 0..rows {
                let x = 2 * i as i32;
                let v = (180 * g.history.power_at(i, s) as u32 / max) as i32;
                fb.line(px + 4, 192 - py, x + 4, 192 - v, colour(s));
                (px, py) = (x, v);
            }
        }
    }
}

impl Screen for PowerGraph {
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => Transition::Pop(DialogResult::Done),
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        if let Some(g) = ctx.game.as_ref() {
            PowerGraph::render(fb, ctx, g);
        }
    }
    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.game_palette()
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

// ---- replay --------------------------------------------------------------------------------

/// The modes of the "See Replay?" menu (ds:0x680a).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Quick = 1,
    Complete = 2,
    Disk = 3,
}

/// The replay's own picture of the world: the cities the records have founded (ds:0x6422, 128
/// slots of owner, x, y; owner -1 = free) and the tribe each slot goes by (ds:0xd7f4).
struct Playback {
    mode: Mode,
    records: Vec<Record>,
    next: usize,
    cities: Vec<(i32, i32, i32)>,
    tribes: [u8; 8],
    canvas: Surface,
    /// Ticks left of a quick replay's pause after a city record (wait_ticks(30)).
    pause: u32,
    /// A complete replay waits for a key after every record (getch).
    waiting: bool,
    /// The text of `replay.txt` (mode 3).
    file: String,
}

enum State {
    Menu(MenuDialog),
    Playing(Box<Playback>),
    Error(MessageBox),
}

/// `replay()` (o 0xcb75): "See Replay?" with No replay / Quick Replay / Complete Replay / Write
/// Replay to Disk at (100, 80), asked again after each replay until "No replay" (or after the
/// file is written). A replay paints the world 4 px a square (ocean colour 1, land 8, from
/// y 4, the polar rows left black) and plays the log: each record's line goes into a white
/// bar at the top (year, then the text; a line about the human in capitals) and each city
/// record repaints the land within 5 squares in the colour of its nearest replay city when
/// that is closer than 6 and on the same continent (the city square itself white, or 7 for
/// civ 1). A quick replay pauses 30 ticks after each city record, a complete one waits for a
/// key after every record (Esc stops it); writing to disk puts the map and every line,
/// rankings included, into `replay.txt`.
pub struct Replay {
    state: State,
}

impl Replay {
    pub fn new() -> Replay {
        Replay { state: State::Menu(Self::menu()) }
    }

    fn menu() -> MenuDialog {
        let items = ["No replay", "Quick Replay", "Complete Replay", "Write Replay to Disk"].iter().map(|s| s.to_string()).collect();
        MenuDialog::new(vec!["See Replay?".into()], items).with_pos(100, 80)
    }

    fn start(g: &Game, mode: Mode) -> Box<Playback> {
        let mut tribes = [0u8; 8];
        for (s, t) in tribes.iter_mut().enumerate() {
            *t = g.player(s as Slot).start_civ;
        }
        let mut canvas = Surface::screen();
        canvas.fill(0);
        for y in 1..49 {
            for x in 0..80 {
                let c = if g.map.is_ocean(x, y) { 1 } else { 8 };
                canvas.fill_rect(Rect::new(4 * x, 4 * y + 4, 4, 4), c);
            }
        }
        let file = if mode == Mode::Disk { map_text(g) } else { String::new() };
        Box::new(Playback { mode, records: g.history.records(), next: 0, cities: vec![(-1, 0, 0); 128], tribes, canvas, pause: 0, waiting: false, file })
    }
}

impl Replay {
    /// A quick replay played to its last record and held there (`--dump screen:replay`).
    pub fn played_out(ctx: &UiContext) -> Replay {
        let mut p = Self::start(ctx.game(), Mode::Quick);
        while !p.advance(ctx) {
            p.pause = 0;
        }
        p.pause = u32::MAX;
        Replay { state: State::Playing(p) }
    }
}

impl Default for Replay {
    fn default() -> Self {
        Replay::new()
    }
}

/// The map of `replay.txt` (o 0xd7a5): 50 rows of 80 characters ('.' land, ' ' ocean), each
/// city site the records name marked '*' while a city stands there ('+' else) and followed by
/// its name as far as the row goes; CR LF after each row.
fn map_text(g: &Game) -> String {
    let mut sites: Vec<(i32, i32)> = vec![(-1, -1); g.history.names.len()];
    for r in g.history.records() {
        if r.kind == kind::CITY && r.args[0] != 0xff {
            if let Some(s) = sites.get_mut(r.args[1] as usize) {
                *s = (r.args[2] as i32, r.args[3] as i32);
            }
        }
    }
    let mut out = String::new();
    for y in 0..50 {
        let mut row: Vec<u8> = (0..80).map(|x| if g.map.is_ocean(x, y) { b' ' } else { b'.' }).collect();
        for x in 0..80 {
            if g.map.is_ocean(x, y) {
                continue;
            }
            let Some(i) = sites.iter().position(|s| *s == (x, y)) else { continue };
            row[x as usize] = if g.city_at(x, y).is_some() { b'*' } else { b'+' };
            for (j, b) in g.history.names[i].bytes().enumerate() {
                let at = x as usize + j + 1;
                if at >= 80 {
                    break;
                }
                row[at] = b;
            }
        }
        out.push_str(&String::from_utf8_lossy(&row));
        out.push_str("\r\n");
    }
    out
}

impl Playback {
    fn plural<'a>(&self, g: &'a Game, s: u8) -> &'a str {
        if s == g.human_slot || s > 7 {
            return g.player(s.min(7)).plural();
        }
        tribe_names(self.tribes[s as usize]).0
    }
    fn adjective<'a>(&self, g: &'a Game, s: u8) -> &'a str {
        if s == g.human_slot || s > 7 {
            return g.player(s.min(7)).adjective();
        }
        tribe_names(self.tribes[s as usize]).1
    }

    /// One record (the jump table at o 0xd381): its line, if it prints one, with the civ the
    /// line is about (ds:0x6810).
    fn step(&mut self, g: &Game, r: &Record) -> Option<(String, Option<u8>)> {
        let year = rules::year_for_turn(r.turn);
        let mut line = format!("{}{}: ", year.abs(), if year < 0 { " BC" } else { " AD" });
        let a = &r.args;
        let about = match r.kind {
            kind::CITY | kind::CAPTURE => {
                let (civ, name, x, y) = (a[0], g.history.name(a[1]).to_string(), a[2] as i32, a[3] as i32);
                if civ == 0xff {
                    line += &format!("{name} destroyed");
                    if let Some(c) = self.cities.iter_mut().find(|c| (c.1, c.2) == (x, y)) {
                        c.0 = -1;
                    }
                } else if r.kind == kind::CITY {
                    line += &format!("{} found the city of {name}", self.plural(g, civ));
                    if let Some(c) = self.cities.iter_mut().find(|c| c.0 == -1) {
                        *c = (civ as i32, x, y);
                    }
                } else {
                    line += &format!("{} capture {name}", self.plural(g, civ));
                    if let Some(c) = self.cities.iter_mut().find(|c| (c.1, c.2) == (x, y)) {
                        c.0 = civ as i32;
                    }
                }
                self.territory(g, x, y);
                if self.mode == Mode::Quick {
                    self.pause = 30;
                }
                Some(civ)
            }
            kind::WAR | kind::PEACE => {
                let what = if r.kind == kind::WAR { "declare war on" } else { "make peace with" };
                line += &format!("{} {what} {}", self.plural(g, a[0] >> 4), self.plural(g, a[0] & 15));
                None
            }
            kind::DISCOVERY => {
                let adv = rules::ADVANCES.get(a[1] as usize).map(|v| v.name).unwrap_or("");
                line += &format!("{} discover {adv}", self.plural(g, a[0]));
                Some(a[0])
            }
            kind::FIRST_UNIT => {
                let unit = rules::UNITS.get(a[1] as usize).map(|u| u.name).unwrap_or("");
                line += &format!("{} produce first {unit}", self.plural(g, a[0]));
                Some(a[0])
            }
            kind::GOVERNMENT => {
                let govt = rules::GOVERNMENTS.get(a[1] as usize).map(|v| v.name).unwrap_or("");
                line += &format!("{} form {govt}", self.plural(g, a[0]));
                Some(a[0])
            }
            kind::WONDER => {
                let w = a[1] as usize;
                let name = rules::WONDERS.get(w.wrapping_sub(1)).map(|v| v.name).unwrap_or("");
                line += &format!("{} build {}{name}", self.plural(g, a[0]), if w <= 7 { "the " } else { "" });
                Some(a[0])
            }
            kind::CENSUS => {
                let h = g.human_slot;
                let pop = (a[1] as i32) << 8 | a[2] as i32;
                line += &format!("{}: {} cities; {} population", self.plural(g, h), a[0], crate::game::score::population_text(pop));
                Some(h)
            }
            kind::RANKING => {
                // only `replay.txt` gets the rankings, without the year
                line = "*** ".into();
                for (k, b) in a.iter().enumerate() {
                    if b >> 4 != 0 {
                        line += &format!("{}:{} ", 2 * k + 1, self.plural(g, b >> 4));
                    }
                    if b & 15 != 0 {
                        line += &format!("{}:{} ", 2 * k + 2, self.plural(g, b & 15));
                    }
                }
                if self.mode != Mode::Disk {
                    return None;
                }
                None
            }
            kind::DESTROYED => {
                let (civ, by) = (a[0], a[1]);
                line += &format!("{} civilization destroyed by {}", self.adjective(g, civ), self.plural(g, by));
                // the slot goes on as its other tribe (0xd368)
                if let Some(t) = self.tribes.get_mut(civ as usize) {
                    *t ^= 8;
                }
                Some(civ)
            }
            _ => return None,
        };
        Some((line, about))
    }

    /// `replay_print(text)` (o 0xd418): the line in black at (4, 1) on a white bar (0, 0)-(319, 7),
    /// fitted to 312 px, in capitals when it is about the human; `replay.txt` gets it whole.
    fn print(&mut self, ctx: &UiContext, line: String, about: Option<u8>) {
        let line = if about == Some(ctx.game().human_slot) { line.to_uppercase() } else { line };
        if self.mode == Mode::Disk {
            self.file.push_str(&line);
            self.file.push_str("\r\n");
        }
        let shown = fit_text(ctx, &line, 312);
        self.canvas.fill_rect(Rect::new(0, 0, 320, 8), 15);
        draw_text(&mut self.canvas, &ctx.fonts, &TextStyle::new(FONT, 0), 4, 1, &shown);
    }

    /// `replay_draw_territory(x, y)` (o 0xd63d): squares x-5..x+4, y-5..y+4 (inside x 0..79,
    /// y 2..48) of land.
    fn territory(&mut self, g: &Game, x: i32, y: i32) {
        let (x0, x1) = ((x - 5).clamp(0, 80), (x + 5).clamp(0, 80));
        let (y0, y1) = ((y - 5).clamp(2, 49), (y + 5).clamp(2, 49));
        for yy in y0..y1 {
            for xx in x0..x1 {
                if g.map.is_ocean(xx, yy) {
                    continue;
                }
                // replay_nearest_city (o 0xd5d9): the first of the nearest, 999 for none
                let mut best: Option<(i32, (i32, i32, i32))> = None;
                for c in self.cities.iter().filter(|c| c.0 != -1) {
                    let d = Map::civ_distance((c.1, c.2), (xx, yy));
                    if best.is_none_or(|(b, _)| d < b) {
                        best = Some((d, *c));
                    }
                }
                let col = match best {
                    Some((d, (owner, cx, cy))) if d < 6 && g.map.tile(cx, cy).continent == g.map.tile(xx, yy).continent => {
                        if d == 0 {
                            if owner == 1 {
                                7
                            } else {
                                15
                            }
                        } else {
                            colour(owner as Slot)
                        }
                    }
                    _ => 8,
                };
                self.canvas.fill_rect(Rect::new(4 * xx, 4 * yy + 4, 4, 4), col);
            }
        }
    }

    /// Plays records until a pause, a key wait or the end; true at the end.
    fn advance(&mut self, ctx: &UiContext) -> bool {
        while self.pause == 0 && !self.waiting {
            let Some(r) = self.records.get(self.next).cloned() else { return true };
            self.next += 1;
            if let Some((line, about)) = self.step(ctx.game(), &r) {
                self.print(ctx, line, about);
            }
            if self.mode == Mode::Complete {
                self.waiting = true;
            }
        }
        false
    }
}

impl Screen for Replay {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        if let State::Menu(m) = &mut self.state {
            m.on_enter(ctx);
        }
    }
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        let State::Playing(p) = &mut self.state else { return Transition::None };
        if p.pause > 0 {
            p.pause -= 1;
            if p.pause > 0 {
                return Transition::None;
            }
        }
        if !p.advance(ctx) {
            return Transition::None;
        }
        if p.mode == Mode::Disk {
            let path = ctx.save_dir.join("replay.txt");
            let ok = std::fs::create_dir_all(&ctx.save_dir).and_then(|_| std::fs::write(&path, p.file.as_bytes())).is_ok();
            if !ok {
                let mut b = MessageBox::text("File Error");
                b.on_enter(ctx);
                self.state = State::Error(b);
                return Transition::None;
            }
            return Transition::Pop(DialogResult::Done);
        }
        let mut m = Self::menu();
        m.on_enter(ctx);
        self.state = State::Menu(m);
        Transition::None
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        match &mut self.state {
            State::Menu(m) => match m.handle(ev, ctx) {
                Transition::Pop(DialogResult::Choice(i @ 1..=3)) => {
                    let mode = [Mode::Quick, Mode::Complete, Mode::Disk][i - 1];
                    self.state = State::Playing(Self::start(ctx.game(), mode));
                    Transition::None
                }
                Transition::Pop(_) => Transition::Pop(DialogResult::Done),
                t => t,
            },
            State::Playing(p) => {
                if p.waiting {
                    match ev {
                        Event::Key { key: Key::Esc, down: true, .. } => p.next = p.records.len(),
                        Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => {}
                        _ => return Transition::None,
                    }
                    p.waiting = false;
                }
                Transition::None
            }
            State::Error(b) => match b.handle(ev, ctx) {
                Transition::Pop(_) => Transition::Pop(DialogResult::Done),
                t => t,
            },
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        match &mut self.state {
            State::Menu(m) => m.draw(fb, ctx),
            State::Playing(p) => fb.blit(&p.canvas, 0, 0),
            State::Error(b) => b.draw(fb, ctx),
        }
    }
    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.game_palette()
    }
    fn draws_parent(&self) -> bool {
        !matches!(self.state, State::Playing(_))
    }
    fn wants_cursor(&self) -> bool {
        matches!(self.state, State::Menu(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::replay::History;
    use crate::game::GameSetup;

    fn lines(g: &Game, mode: Mode) -> Vec<String> {
        let mut p = Replay::start(g, mode);
        g.history.records().iter().filter_map(|r| p.step(g, r)).map(|(l, about)| if about == Some(g.human_slot) { l.to_uppercase() } else { l }).collect()
    }

    #[test]
    fn records_read_as_civ_exe_prints_them() {
        let mut g = Game::new(GameSetup { seed: 3, ..Default::default() }, None);
        g.history = History::default();
        let h = g.human_slot;
        let ai = if h == 2 { 3 } else { 2 };
        let (them, adj) = (g.player(ai).plural().to_string(), g.player(ai).adjective().to_string());
        g.set_turn(10);
        let c = g.add_city(ai, "Ur".into(), 30, 20);
        g.replay_log(kind::WONDER, &[ai, 1]);
        g.replay_log(kind::WONDER, &[ai, 8]);
        g.set_turn(250);
        g.replay_log(kind::GOVERNMENT, &[h, 2]);
        g.remove_city(c);
        g.replay_log(kind::DESTROYED, &[ai, h]);
        g.replay_log(kind::FIRST_UNIT, &[ai, 0]);
        g.replay_log(kind::RANKING, &[h << 4 | ai, 0, 0, 0]);
        let me = g.human().plural().to_uppercase();
        let other = tribe_names(g.player(ai).start_civ ^ 8).0;
        let quick = lines(&g, Mode::Quick);
        assert_eq!(quick[0], format!("3800 BC: {them} found the city of Ur"));
        assert_eq!(quick[1], format!("3800 BC: {them} build the {}", rules::WONDERS[0].name));
        assert_eq!(quick[2], format!("3800 BC: {them} build {}", rules::WONDERS[7].name), "\"the\" for wonders 1..7 only");
        assert_eq!(quick[3], format!("1000 AD: {me} FORM MONARCHY"));
        assert_eq!(quick[4], "1000 AD: Ur destroyed");
        assert_eq!(quick[5], format!("1000 AD: {adj} civilization destroyed by {}", g.human().plural()));
        assert_eq!(quick[6], format!("1000 AD: {other} produce first {}", rules::UNITS[0].name), "the slot goes on as its other tribe");
        assert_eq!(quick.len(), 7, "rankings only go to the file");
        let disk = lines(&g, Mode::Disk);
        assert_eq!(disk[7], format!("*** 1:{} 2:{other} ", g.human().plural()));
    }

    #[test]
    fn the_file_map_marks_logged_cities() {
        let mut g = Game::new(GameSetup { seed: 3, ..Default::default() }, None);
        g.history = History::default();
        let (x, y) = (0..80 * 50).map(|i| (i % 80, i / 80)).find(|&(x, y)| x < 60 && g.map.is_land(x, y) && g.city_at(x, y).is_none()).unwrap();
        let c = g.add_city(2, "Ur".into(), x, y);
        let text = map_text(&g);
        let rows: Vec<&str> = text.split("\r\n").collect();
        assert_eq!(rows.len(), 51);
        assert_eq!(&rows[y as usize][x as usize..x as usize + 3], "*Ur");
        g.remove_city(c);
        assert_eq!(&map_text(&g).split("\r\n").nth(y as usize).unwrap()[x as usize..x as usize + 3], "+Ur");
    }
}
