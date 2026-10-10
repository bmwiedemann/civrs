//! Civilopedia: the "ENCYCLOPAEDIA of CIVILIZATION" category lists and the entry pages.
//!
//! The picture-lookup tables (`advance_icon`, `unit_picture`, `terrain_picture`,
//! `improvement_picture`, `wonder_picture`) are re-exported here for the other screens.

use crate::gfx::{Rect, Surface};
use crate::platform::{Event, Key, MouseButton};
use crate::ui::context::UiContext;
use crate::ui::pedia_data::{self, Entry};
use crate::ui::pedia_entry::{draw_spaced, draw_spaced_centred, spaced_width, EntryScreen};
use crate::ui::screen::{DialogResult, Screen, Transition};

pub use crate::ui::pedia_data::{advance_icon, improvement_picture, terrain_picture, unit_picture, wonder_picture};

// SP257 palette indices of the list page colours.
const BLACK: u8 = 0;
const LIGHT_GREEN: u8 = 10;
const RED: u8 = 12;
const YELLOW: u8 = 14;
const PAGE: u8 = 15;
const BLUE: u8 = 1;

const FONT: usize = 0;
const SPACING: i32 = 1;
const ROWS: usize = 26;
const ROW_Y0: i32 = 16;
const ROW_PITCH: i32 = 7;
const EXIT_RECT: Rect = Rect::new(284, 2, 30, 10);
/// "MORE" (only on multi-page lists) sits at the left of the band, mirroring EXIT.
const MORE_RECT: Rect = Rect::new(8, 2, 32, 10);

/// The Civilopedia list for a category (0 Complete, 1 Advances, 2 Improvements, 3 Units,
/// 4 Terrain, 5 Miscellaneous), to be pushed over the map; pops with `DialogResult::Done`.
pub fn category(_ctx: &UiContext, c: usize) -> Box<dyn Screen> {
    Box::new(ListScreen::new(c))
}

/// An entry page screen (used by the lists and the headless dump).
pub fn entry_screen(entry: Entry, page: usize) -> Box<dyn Screen> {
    Box::new(EntryScreen::new(entry, page))
}

/// Entry for a (kind, index) pair: 0 advance, 1 improvement, 2 wonder, 3 unit, 4 terrain,
/// 5 concept. Used by the `--dump screen:pentry:KII` debugging hook.
pub fn entry_by_code(code: usize) -> Option<Entry> {
    let (kind, idx) = (code / 100, code % 100);
    match kind {
        0 => (idx < crate::rules::advances::REAL_ADVANCE_COUNT).then_some(Entry::Advance(idx as u8)),
        1 => (idx >= 1 && idx < crate::rules::IMPROVEMENTS.len()).then_some(Entry::Improvement(idx as u8)),
        2 => (idx >= 1 && idx < crate::rules::WONDERS.len()).then_some(Entry::Wonder(idx as u8)),
        3 => (idx < crate::rules::UNITS.len()).then_some(Entry::Unit(idx as u8)),
        4 => crate::rules::Terrain::from_u8(idx as u8).map(Entry::Terrain),
        5 => (idx < pedia_data::CONCEPTS.len()).then_some(Entry::Concept(idx)),
        _ => None,
    }
}

/// Category list: up to three columns of 26 names, MORE/EXIT at the top.
pub struct ListScreen {
    entries: Vec<Entry>,
    cols: usize,
    page: usize,
    /// Keyboard selection (shown only once the arrow keys have been used).
    sel: Option<usize>,
    guard: u32,
}

impl ListScreen {
    pub fn new(category: usize) -> ListScreen {
        let entries = pedia_data::entries(category);
        let cols = entries.len().div_ceil(ROWS).clamp(1, 3);
        ListScreen { entries, cols, page: 0, sel: None, guard: 2 }
    }

    fn per_page(&self) -> usize {
        ROWS * self.cols
    }

    fn pages(&self) -> usize {
        self.entries.len().div_ceil(self.per_page()).max(1)
    }

    fn col_x(&self, col: usize) -> i32 {
        10 + col as i32 * if self.cols == 3 { 100 } else { 150 }
    }

    fn col_w(&self) -> i32 {
        if self.cols == 3 {
            100
        } else {
            150
        }
    }

    /// Screen position of entry `i` (on the current page).
    fn pos(&self, i: usize) -> (i32, i32) {
        let k = i - self.page * self.per_page();
        (self.col_x(k / ROWS), ROW_Y0 + (k % ROWS) as i32 * ROW_PITCH)
    }

    fn entry_at(&self, mx: i32, my: i32) -> Option<usize> {
        let start = self.page * self.per_page();
        let end = (start + self.per_page()).min(self.entries.len());
        (start..end).find(|&i| {
            let (x, y) = self.pos(i);
            mx >= x - 2 && mx < x + self.col_w() && my >= y - 1 && my < y + ROW_PITCH - 1
        })
    }

    fn move_sel(&mut self, delta: i32) {
        let n = self.entries.len() as i32;
        if n == 0 {
            return;
        }
        let cur = self.sel.map(|s| s as i32).unwrap_or(if delta > 0 { -1 } else { 0 });
        let s = (cur + delta).rem_euclid(n) as usize;
        self.sel = Some(s);
        self.page = s / self.per_page();
    }

    fn open(&self, i: usize) -> Transition {
        Transition::Push(entry_screen(self.entries[i], 0))
    }
}

impl Screen for ListScreen {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }

    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        match ev {
            Event::Key { key, down: true, .. } => match key {
                Key::Esc => return Transition::Pop(DialogResult::Done),
                Key::Up | Key::Kp(8) => self.move_sel(-1),
                Key::Down | Key::Kp(2) => self.move_sel(1),
                Key::Left | Key::Kp(4) => self.move_sel(-(ROWS as i32)),
                Key::Right | Key::Kp(6) => self.move_sel(ROWS as i32),
                Key::Home => {
                    self.sel = Some(0);
                    self.page = 0;
                }
                Key::End => {
                    self.sel = Some(self.entries.len().saturating_sub(1));
                    self.page = self.pages() - 1;
                }
                Key::PageDown | Key::Space | Key::Char('m') => {
                    self.page = (self.page + 1) % self.pages();
                    self.sel = None;
                }
                Key::PageUp => {
                    self.page = (self.page + self.pages() - 1) % self.pages();
                    self.sel = None;
                }
                Key::Enter => {
                    if let Some(s) = self.sel {
                        return self.open(s);
                    }
                }
                _ => {}
            },
            Event::MouseButton { button: MouseButton::Left, x, y, down: true } => {
                if EXIT_RECT.contains(*x, *y) {
                    return Transition::Pop(DialogResult::Done);
                }
                if self.pages() > 1 && MORE_RECT.contains(*x, *y) {
                    self.page = (self.page + 1) % self.pages();
                    self.sel = None;
                } else if let Some(i) = self.entry_at(*x, *y) {
                    return self.open(i);
                }
            }
            Event::MouseButton { button: MouseButton::Right, down: true, .. } => return Transition::Pop(DialogResult::Done),
            _ => {}
        }
        Transition::None
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        fb.fill_rect(Rect::new(0, 0, 320, 200), YELLOW);
        fb.fill_rect(Rect::new(2, 14, 316, 184), PAGE);
        fb.fill_rect(Rect::new(60, 2, 200, 9), PAGE);
        // green title with a black drop shadow
        draw_spaced_centred(fb, &ctx.fonts, FONT, BLACK, 161, 4, "ENCYCLOPAEDIA of CIVILIZATION", SPACING);
        draw_spaced_centred(fb, &ctx.fonts, FONT, LIGHT_GREEN, 160, 3, "ENCYCLOPAEDIA of CIVILIZATION", SPACING);
        let exit_w = spaced_width(&ctx.fonts, FONT, "EXIT", SPACING);
        draw_spaced(fb, &ctx.fonts, FONT, RED, 312 - exit_w, 4, "EXIT", SPACING);
        if self.pages() > 1 {
            draw_spaced(fb, &ctx.fonts, FONT, RED, MORE_RECT.x + 2, 4, "MORE", SPACING);
        }
        let start = self.page * self.per_page();
        let end = (start + self.per_page()).min(self.entries.len());
        // names wider than the column are cut and end in a '.' ("Genetic Engineeri.")
        let max_w = self.col_w() - 3;
        for i in start..end {
            let (x, y) = self.pos(i);
            let colour = if self.sel == Some(i) { BLUE } else { BLACK };
            let mut name = self.entries[i].name();
            if spaced_width(&ctx.fonts, FONT, &name, SPACING) > max_w {
                while !name.is_empty() && spaced_width(&ctx.fonts, FONT, &format!("{name}."), SPACING) > max_w {
                    name.pop();
                }
                name.push('.');
            }
            draw_spaced(fb, &ctx.fonts, FONT, colour, x, y, &name, SPACING);
        }
    }

    fn draws_parent(&self) -> bool {
        false
    }
}
