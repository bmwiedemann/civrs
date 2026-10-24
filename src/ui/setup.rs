//! New-game setup flow as CIV.EXE runs it: the custom-world questions, then the world generator
//! with the evolution story (main_menu 0x1f91 calls overlay 7 before `start_game`), then
//! overlay 5's `new_game_setup` (o 0x4175): difficulty, number of civilisations, tribe and leader
//! name. Those menus are drawn without a panel over the white box of DIFFS.PIC at (160, 35) in
//! window font 7 (civrs 6); the left half shows all portraits, then the chosen one at (20, 100),
//! then one per civilization stacked from (20, 100) in steps of (2, 3), with the tribe's plural
//! centred on (46, 92) once it is picked.

use crate::game::{GameSetup, WorldParams, WorldSource};
use crate::gfx::text::{draw_text, draw_text_centered, text_width};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::{Event, Key};
use crate::rules::{self, Difficulty, CUSTOM_WORLD_OPTIONS};
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::{MenuDialog, PanelStyle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Start,
    Difficulty,
    NumCivs,
    Tribe,
    Name,
    Custom(usize),
    Story,
    Generate,
    Done,
}

/// Menu origin of the setup menus (`popup_menu3(text, 160, 35)`).
const MENU_X: i32 = 160;
const MENU_Y: i32 = 35;
/// Width of the menu's mouse area: the inside of the DIFFS.PIC box.
const MENU_W: i32 = 122;
/// Window font 7 of the setup screens.
const SETUP_FONT: usize = 6;
/// The left half (x < 150) is cleared once a level is chosen; the right half shows DIFFS.PIC.
const LEFT_W: i32 = 150;
/// Where the chosen portrait (and the front of the stack) goes.
const PORTRAIT_AT: (i32, i32) = (20, 100);

/// Portrait of a difficulty level in DIFFS.PIC as `new_game_setup` copies it: x 21 for even
/// levels, 80 for odd ones, y 6 + 35 * level, 53x47.
pub fn portrait(level: usize) -> Rect {
    let d = level as i32;
    Rect::new(if d & 1 == 1 { 80 } else { 21 }, 6 + 35 * d, 53, 47)
}

/// Tribes offered for `n` civilizations (o 0x449d): the first n of each set, ids 1..=n then
/// 9..=8+n.
fn tribes_for(n: u8) -> Vec<u8> {
    (1..=n).chain(9..=8 + n).collect()
}

pub struct SetupFlow {
    kind: usize,
    step: Step,
    setup: GameSetup,
    params: WorldParams,
    tribes: Vec<u8>,
    custom_choices: [Option<usize>; 4],
}

pub fn start_flow(kind: usize) -> Box<dyn Screen> {
    if kind == 1 {
        return crate::ui::reports::load_game_screen();
    }
    Box::new(SetupFlow {
        kind,
        step: Step::Start,
        setup: GameSetup { seed: seed_from_clock(), ..Default::default() },
        params: WorldParams::normal(),
        tribes: Vec::new(),
        custom_choices: [None; 4],
    })
}

/// The game seed from the clock as CIV.EXE takes it (`seed_random` 0x1d43:0x38: time(0) &
/// 0x7fff), so a new world is one of the original's (see `Game::new`).
fn seed_from_clock() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() & 0x7fff).unwrap_or(1)
}

impl SetupFlow {
    fn white_menu(title: &str, items: Vec<String>) -> MenuDialog {
        MenuDialog::new(vec![title.to_string()], items).with_pos(MENU_X, MENU_Y).with_style(PanelStyle::Bare).with_font(SETUP_FONT).with_top_pad(5).with_size(MENU_W, 0)
    }

    fn next(&mut self, ctx: &mut UiContext) -> Transition {
        match self.step {
            Step::Start => {
                if self.kind == 3 {
                    self.step = Step::Custom(0);
                    return self.custom_menu(0);
                }
                self.story()
            }
            Step::Custom(i) => {
                if i + 1 < 4 {
                    self.step = Step::Custom(i + 1);
                    return self.custom_menu(i + 1);
                }
                self.story()
            }
            Step::Story => self.difficulty_menu(),
            Step::Difficulty => {
                self.step = Step::NumCivs;
                let items: Vec<String> = (3..=7).rev().map(|n| format!("{n} Civilizations")).collect();
                Transition::Push(Box::new(Self::white_menu("Level of Competition...", items)))
            }
            Step::NumCivs => {
                self.step = Step::Tribe;
                self.tribes = tribes_for(self.setup.num_civs);
                let items: Vec<String> = self.tribes.iter().map(|c| rules::CIVS[*c as usize].adjective.to_string()).collect();
                Transition::Push(Box::new(Self::white_menu("Pick your tribe...", items)))
            }
            Step::Tribe => {
                self.step = Step::Name;
                let default = rules::CIVS[self.setup.human_civ as usize].leader;
                Transition::Push(Box::new(NameEntry::new(default)))
            }
            Step::Name => {
                self.step = Step::Generate;
                self.generate(ctx)
            }
            Step::Generate | Step::Done => Transition::None,
        }
    }

    fn story(&mut self) -> Transition {
        self.step = Step::Story;
        Transition::Push(crate::ui::story::story_screen())
    }

    fn difficulty_menu(&mut self) -> Transition {
        self.step = Step::Difficulty;
        let mut items: Vec<String> = rules::DIFFICULTY.iter().map(|d| d.name.to_string()).collect();
        items[0] = format!("{} (easiest)", items[0]);
        items[4] = format!("{} (toughest)", items[4]);
        Transition::Push(Box::new(Self::white_menu("Difficulty Level...", items)))
    }

    fn custom_menu(&self, i: usize) -> Transition {
        let (_, opts) = CUSTOM_WORLD_OPTIONS[i];
        let items: Vec<String> = opts.iter().map(|s| s.to_string()).collect();
        let y = 14 + i as i32 * 49;
        Transition::Push(Box::new(CustomMenu { inner: MenuDialog::new(vec![], items).with_pos(190, y).with_selection(1).no_cancel() }))
    }

    fn generate(&mut self, ctx: &mut UiContext) -> Transition {
        self.setup.world = if self.kind == 2 { WorldSource::Earth } else { WorldSource::Random(self.params) };
        let earth = if self.kind == 2 { ctx.assets.load_earth().ok() } else { None };
        let game = crate::game::Game::new(self.setup.clone(), earth.as_ref());
        ctx.game = Some(game);
        self.step = Step::Done;
        Transition::Replace(crate::ui::intro_message::leader_intro())
    }

    fn picture(&self) -> &'static str {
        if matches!(self.step, Step::Custom(_)) {
            "CUSTOM"
        } else {
            "DIFFS"
        }
    }

    /// One portrait per civilization, the back ones first (o 0x433b).
    fn draw_portrait_stack(&self, fb: &mut Surface, ctx: &UiContext, n: i32) {
        let diffs = ctx.pic("DIFFS");
        let r = portrait(self.setup.difficulty as usize);
        for i in (0..n).rev() {
            fb.blit_part(&diffs.surf, r, PORTRAIT_AT.0 + 2 * i, PORTRAIT_AT.1 + 3 * i, None);
        }
    }
}

/// `ask_leader_name` (overlay 23 +0xd6) with its line editor `edit_line(166, 104, 13)`
/// (ovl 23 +0x414): white box (158, 88) 161x33 with a colour-11 outline, "Your Name..." at
/// (166, 90), the field outline (166, 103) 113x14, characters from (168, 106). The editor
/// overwrites: the first typed character clears the field, Left/Right move, Ins/Del/Backspace
/// shift, and Enter, Esc or a click ends it (Esc keeps the text). The cursor cell blinks cyan.
struct NameEntry {
    buf: Vec<char>,
    cursor: usize,
    first_key: bool,
    tick: u64,
}

const NAME_MAX: usize = 13;
const NAME_BOX: Rect = Rect::new(158, 88, 161, 33);
const NAME_FIELD: Rect = Rect::new(166, 103, 113, 14);
const NAME_TEXT: (i32, i32) = (168, 106);

impl NameEntry {
    fn new(default: &str) -> NameEntry {
        let mut buf: Vec<char> = default.chars().take(NAME_MAX).collect();
        buf.resize(NAME_MAX, ' ');
        NameEntry { buf, cursor: 0, first_key: true, tick: 0 }
    }

    /// The text with trailing blanks removed (the first character always stays).
    fn text(&self) -> String {
        let mut end = self.buf.len();
        while end > 1 && self.buf[end - 1] == ' ' {
            end -= 1;
        }
        self.buf[..end].iter().collect()
    }

    fn key(&mut self, key: Option<Key>, ch: Option<char>) -> Option<Transition> {
        match key {
            Some(Key::Enter) | Some(Key::Esc) => return Some(Transition::Pop(DialogResult::Text(self.text()))),
            Some(Key::Left) => self.cursor = self.cursor.saturating_sub(1),
            Some(Key::Right) => self.cursor = (self.cursor + 1).min(NAME_MAX - 1),
            Some(Key::Insert) => {
                self.buf.insert(self.cursor, ' ');
                self.buf.truncate(NAME_MAX);
            }
            Some(Key::Delete) => {
                self.buf.remove(self.cursor);
                self.buf.push(' ');
            }
            Some(Key::Backspace) if self.cursor > 0 => {
                self.buf.remove(self.cursor - 1);
                self.buf.push(' ');
                self.cursor -= 1;
            }
            _ => {}
        }
        if let Some(c) = ch.filter(|c| (' '..='~').contains(c)) {
            if self.first_key {
                self.buf.iter_mut().for_each(|b| *b = ' ');
            }
            self.buf[self.cursor] = c;
            self.cursor = (self.cursor + 1).min(NAME_MAX - 1);
        }
        self.first_key = false;
        None
    }
}

impl Screen for NameEntry {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.tick += 1;
        Transition::None
    }

    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        let t = match ev {
            Event::Key { key: Key::Char(_) | Key::Space, down: true, .. } => None,
            Event::Key { key, down: true, .. } => self.key(Some(*key), None),
            Event::Text(c) => self.key(None, Some(*c)),
            Event::MouseButton { down: true, .. } => self.key(Some(Key::Enter), None),
            _ => None,
        };
        t.unwrap_or(Transition::None)
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let pal = &ctx.palette;
        let (white, black, cyan) = (pal.nearest([255, 255, 255]), pal.nearest([0, 0, 0]), pal.nearest([85, 255, 255]));
        fb.fill_rect(NAME_BOX, white);
        fb.rect_outline(NAME_BOX, cyan);
        draw_text(fb, &ctx.fonts, &TextStyle::new(SETUP_FONT, black), 166, 90, "Your Name...");
        fb.rect_outline(NAME_FIELD, black);
        let mut x = NAME_TEXT.0;
        for (i, c) in self.buf.iter().enumerate() {
            let s = c.to_string();
            let w = text_width(&ctx.fonts, SETUP_FONT, &s);
            // the cursor cell is recoloured 15 -> 11 for 10 ticks, then back for 10
            if i == self.cursor && (self.tick / 10).is_multiple_of(2) {
                fb.fill_rect(Rect::new(x, NAME_TEXT.1 - 1, w, 10), cyan);
            }
            draw_text(fb, &ctx.fonts, &TextStyle::new(SETUP_FONT, black), x, NAME_TEXT.1, &s);
            x += w;
        }
    }

    fn draws_parent(&self) -> bool {
        true
    }

    /// The mouse is hidden while the name is edited.
    fn wants_cursor(&self) -> bool {
        false
    }
}

/// Custom-world option menu drawn in the cyan area without a panel.
struct CustomMenu {
    inner: MenuDialog,
}

impl Screen for CustomMenu {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        self.inner.on_enter(ctx);
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        self.inner.handle(ev, ctx)
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let pal = ctx.pic_pal("CUSTOM");
        let bar = pal.nearest([12, 227, 235]);
        let black = pal.nearest([0, 0, 0]);
        let r = self.inner.rect;
        for (i, it) in self.inner.items.iter().enumerate() {
            let y = r.y + 4 + i as i32 * 8;
            if i == self.inner.sel {
                fb.fill_rect(Rect::new(r.x - 2, y - 1, 60, 8), bar);
            }
            draw_text(fb, &ctx.fonts, &TextStyle::new(0, black), r.x, y, it);
        }
    }
}

impl Screen for SetupFlow {
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        if self.step == Step::Start {
            return self.next(ctx);
        }
        Transition::None
    }

    fn handle(&mut self, _ev: &Event, _ctx: &mut UiContext) -> Transition {
        Transition::None
    }

    fn resume(&mut self, result: DialogResult, ctx: &mut UiContext) -> Transition {
        match (&self.step, result) {
            // Esc: Chieftain (o 0x4210), the competition question again (o 0x430b), a random
            // tribe (o 0x45eb; the original then asks for the tribe's name, which civrs has no
            // place for); in the custom-world questions it leaves the setup
            (Step::Difficulty, DialogResult::Cancelled) => self.resume(DialogResult::Choice(0), ctx),
            (Step::NumCivs, DialogResult::Cancelled) => {
                self.step = Step::Difficulty;
                self.next(ctx)
            }
            (Step::Tribe, DialogResult::Cancelled) => {
                let n = self.setup.num_civs.max(2) as u64 - 1;
                let pick = (ctx.frame.wrapping_mul(0x9e37_79b9) >> 13) % n;
                self.resume(DialogResult::Choice(pick as usize), ctx)
            }
            (_, DialogResult::Cancelled) => Transition::Pop(DialogResult::Done),
            (Step::Story, _) => self.next(ctx),
            (Step::Difficulty, DialogResult::Choice(i)) => {
                self.setup.difficulty = Difficulty::from_u8(i as u8).unwrap_or(Difficulty::Chieftain);
                self.next(ctx)
            }
            (Step::NumCivs, DialogResult::Choice(i)) => {
                self.setup.num_civs = 7 - i as u8;
                self.next(ctx)
            }
            (Step::Tribe, DialogResult::Choice(i)) => {
                self.setup.human_civ = self.tribes.get(i).copied().unwrap_or(1);
                self.next(ctx)
            }
            (Step::Name, DialogResult::Text(t)) => {
                self.setup.leader_name = t;
                self.next(ctx)
            }
            (Step::Custom(i), DialogResult::Choice(c)) => {
                let i = *i;
                self.custom_choices[i] = Some(c);
                match i {
                    0 => self.params.land_mass = c as u8,
                    1 => self.params.temperature = c as u8,
                    2 => self.params.climate = c as u8,
                    _ => self.params.age = c as u8,
                }
                self.next(ctx)
            }
            _ => Transition::None,
        }
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let p = ctx.pic(self.picture());
        let pal = ctx.pic_pal(self.picture());
        match self.step {
            Step::Custom(cur) => {
                fb.blit(&p.surf, 0, 0);
                let white = pal.nearest([255, 255, 255]);
                let black = pal.nearest([0, 0, 0]);
                let bar = pal.nearest([12, 227, 235]);
                for (i, (title, opts)) in CUSTOM_WORLD_OPTIONS.iter().enumerate() {
                    if i > cur {
                        break;
                    }
                    let y = 6 + i as i32 * 49;
                    draw_text(fb, &ctx.fonts, &TextStyle::new(0, white), 180, y, &format!("{}:", title.to_ascii_uppercase()));
                    if i < cur {
                        for (k, o) in opts.iter().enumerate() {
                            let yy = y + 8 + k as i32 * 8;
                            if self.custom_choices[i] == Some(k) {
                                fb.fill_rect(Rect::new(188, yy - 1, 60, 8), bar);
                            }
                            draw_text(fb, &ctx.fonts, &TextStyle::new(0, black), 190, yy, o);
                        }
                    }
                }
            }
            Step::Difficulty | Step::Start | Step::Story => {
                fb.blit(&p.surf, 0, 0);
            }
            _ => {
                let black = pal.nearest([0, 0, 0]);
                fb.fill(black);
                let n = match self.step {
                    Step::NumCivs => 1,
                    _ => self.setup.num_civs as i32,
                };
                self.draw_portrait_stack(fb, ctx, n);
                if matches!(self.step, Step::NumCivs | Step::Tribe) {
                    fb.blit_part(&p.surf, Rect::new(LEFT_W, 0, 320 - LEFT_W, 200), LEFT_W, 0, None);
                } else {
                    let white = pal.nearest([255, 255, 255]);
                    draw_text_centered(fb, &ctx.fonts, &TextStyle::new(SETUP_FONT, white), 46, 92, rules::CIVS[self.setup.human_civ as usize].plural);
                }
            }
        }
    }

    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.pic_pal(self.picture())
    }

    fn draws_parent(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portraits_follow_the_difficulty() {
        assert_eq!(portrait(0), Rect::new(21, 6, 53, 47));
        assert_eq!(portrait(3), Rect::new(80, 111, 53, 47));
    }

    #[test]
    fn tribes_follow_the_competition() {
        assert_eq!(tribes_for(3), vec![1, 2, 3, 9, 10, 11]);
        assert_eq!(tribes_for(7), vec![1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15]);
    }

    #[test]
    fn name_editor_overwrites() {
        let mut e = NameEntry::new("Caesar");
        e.key(None, Some('B'));
        assert_eq!(e.text(), "B");
        e.key(None, Some('o'));
        e.key(Some(Key::Left), None);
        e.key(Some(Key::Insert), None);
        assert_eq!(e.text(), "B o");
        e.key(Some(Key::Delete), None);
        e.key(Some(Key::Backspace), None);
        assert_eq!(e.text(), "o");
        let mut e = NameEntry::new("Caesar");
        e.key(Some(Key::Right), None);
        e.key(None, Some('x'));
        assert_eq!(e.text(), "Cxesar");
    }
}
