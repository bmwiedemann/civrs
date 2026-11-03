//! Reusable dialog screens: pop-up menus, message boxes, text input, in the original styles.

use crate::gfx::text::{draw_paragraph, draw_text, text_width, wrap};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::{Event, Key, MouseButton};
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::theme;
use crate::ui::tilerender::{cell, sp};

pub const FONT: usize = 0;
pub const LINE_H: i32 = 8;
/// Bare-menu highlight bar width beyond the widest line (menu_select o 0x1ca94: max width + 5).
const BARE_BAR_EXTRA: i32 = 5;

/// Visual style of a panel/menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelStyle {
    /// Grey stone-noise panel with a light border (in-game menus and messages).
    Grey,
    /// White box with a double gold frame (setup screens).
    White,
    /// Brown box with a cream border (title screen main menu).
    Brown,
    /// Green "civilization note" box.
    Green,
    /// White box with a cyan border (name entry).
    Cyan,
    /// No panel, as CIV.EXE's `menu_draw` does for a menu at an odd y (o 0x1ce1a): the menu sits
    /// on the picture (DIFFS.PIC's white box, CUSTOM.PIC's teal side). Laid out from the menu
    /// origin: title at (+5, +5), rows from +13 with the text after a leading blank, items in
    /// colour 0. `menu_select` reads the background colour at the origin (o 0x1c917): the title
    /// is drawn in 3 on white (15), else in 15, and the highlight recolours that background to
    /// 11 from +3, as wide as the widest line + 5.
    Bare,
    /// `menu_draw`'s frame for a menu at an even y (`draw_frame` 0x1d1a2 with colour 7): fill
    /// (x+1, y+1)..(x+w, y+h) in 7, bevel from (x+1, y+1) with the top and right edges in 8 and
    /// the bottom and left in 15, black outline (x, y)..(x+w+2, y+h+2), where w is the widest
    /// line plus 8 and h = lines * pitch + 6. Inside, the layout and colours are `Bare`'s with
    /// background 7. (Before a game, ds:0x2f98 = 0: a plain fill, not the stone texture.)
    Framed,
}

pub struct StyleColours {
    pub text: u8,
    pub title: u8,
    pub hilite_bar: u8,
    pub hilite_text: u8,
    pub disabled: u8,
}

/// Draw a panel background + frame in the given style using the active palette.
pub fn draw_styled_panel(fb: &mut Surface, ctx: &UiContext, r: Rect, style: PanelStyle) -> StyleColours {
    match style {
        PanelStyle::Grey => {
            fb.fill_pattern(r, &ctx.sp.surf, cell(sp::PATTERN_STONE.0, sp::PATTERN_STONE.1));
            fb.hline(r.x, r.right() - 1, r.y, theme::DARK_GREY);
            fb.vline(r.right() - 1, r.y, r.bottom() - 1, theme::DARK_GREY);
            fb.vline(r.x, r.y, r.bottom() - 1, theme::WHITE);
            fb.hline(r.x, r.right() - 1, r.bottom() - 1, theme::WHITE);
            StyleColours { text: theme::BLACK, title: theme::CYAN, hilite_bar: theme::CYAN, hilite_text: theme::BLACK, disabled: theme::CYAN }
        }
        PanelStyle::Cyan => {
            let pal = &ctx.palette;
            let white = pal.nearest([255, 255, 255]);
            let border = pal.nearest([85, 255, 255]);
            let black = pal.nearest([0, 0, 0]);
            fb.fill_rect(r, white);
            fb.rect_outline(r, border);
            StyleColours { text: black, title: black, hilite_bar: pal.nearest([12, 227, 235]), hilite_text: black, disabled: pal.nearest([138, 138, 142]) }
        }
        PanelStyle::White => {
            let pal = &ctx.palette;
            let outer = pal.nearest([255, 182, 89]);
            let inner = pal.nearest([255, 219, 89]);
            let white = pal.nearest([255, 255, 255]);
            let black = pal.nearest([0, 0, 0]);
            fb.fill_rect(r, white);
            fb.rect_outline(r, black);
            fb.rect_outline(r.inset(1), outer);
            fb.rect_outline(r.inset(2), inner);
            fb.rect_outline(r.inset(3), outer);
            StyleColours { text: black, title: pal.nearest([0, 170, 170]), hilite_bar: pal.nearest([12, 227, 235]), hilite_text: black, disabled: pal.nearest([138, 138, 142]) }
        }
        PanelStyle::Brown => {
            let pal = &ctx.palette;
            let bg = pal.nearest([162, 125, 97]);
            let outer = pal.nearest([130, 77, 61]);
            let cream = pal.nearest([255, 255, 223]);
            let tan = pal.nearest([219, 206, 166]);
            let black = pal.nearest([0, 0, 0]);
            fb.fill_rect(r, bg);
            fb.rect_outline(r, outer);
            fb.rect_outline(r.inset(1), cream);
            StyleColours { text: tan, title: tan, hilite_bar: cream, hilite_text: black, disabled: bg }
        }
        PanelStyle::Bare | PanelStyle::Framed => {
            // the menu's own EGA colours; the title colour depends on the background (draw_bare)
            StyleColours { text: 0, title: 15, hilite_bar: 11, hilite_text: 0, disabled: 3 }
        }
        PanelStyle::Green => {
            fb.fill_rect(r, theme::GREEN);
            fb.rect_outline(r, theme::LIGHT_GREEN);
            StyleColours { text: theme::WHITE, title: theme::YELLOW, hilite_bar: theme::LIGHT_GREEN, hilite_text: theme::BLACK, disabled: theme::GREY }
        }
    }
}

/// Grey panel (default dialog background).
pub fn draw_panel(fb: &mut Surface, ctx: &UiContext, r: Rect) {
    draw_styled_panel(fb, ctx, r, PanelStyle::Grey);
}

pub fn draw_frame(fb: &mut Surface, r: Rect) {
    fb.rect_outline(r, theme::WHITE);
    fb.rect_outline(r.inset(1), theme::BLACK);
}

pub fn draw_bevel(fb: &mut Surface, r: Rect, light: u8, dark: u8) {
    fb.hline(r.x, r.right() - 1, r.y, light);
    fb.vline(r.x, r.y, r.bottom() - 1, light);
    fb.hline(r.x, r.right() - 1, r.bottom() - 1, dark);
    fb.vline(r.right() - 1, r.y, r.bottom() - 1, dark);
}

pub fn centred(w: i32, h: i32) -> Rect {
    Rect::new((320 - w) / 2, (200 - h) / 2, w, h)
}

/// `fill_texture(x, y, w, h)` (0x1d7f4): the stone texture, 32x16 tiles of SP299 from
/// (288, 120), each restarting at the area's corner.
pub fn fill_texture(fb: &mut Surface, ctx: &UiContext, r: Rect) {
    tile_fill(fb, ctx, r, Rect::new(288, 120, 32, 16));
}

/// Tiles the SP299 cell `src` over `r`, anchored at r's corner.
pub fn tile_fill(fb: &mut Surface, ctx: &UiContext, r: Rect, src: Rect) {
    let sheet = &ctx.pic("SP299").surf;
    let mut ty = 0;
    while ty < r.h {
        let mut tx = 0;
        while tx < r.w {
            fb.blit_part(sheet, Rect::new(src.x, src.y, (r.w - tx).min(src.w), (r.h - ty).min(src.h)), r.x + tx, r.y + ty, None);
            tx += src.w;
        }
        ty += src.h;
    }
}

/// `draw_bevel(x, y, w, h, a, b)` (0x1d29c): top and right lines in `b`, bottom and left in `a`.
pub fn bevel(fb: &mut Surface, x: i32, y: i32, w: i32, h: i32, a: u8, b: u8) {
    fb.hline(x, x + w, y, b);
    fb.hline(x, x + w, y + h, a);
    fb.vline(x + w, y, y + h, b);
    fb.vline(x, y + 1, y + h, a);
}

/// `draw_panel_frame(x, y, w, h)` (0x3f32): stone texture with the 15/8 bevel.
pub fn panel_frame(fb: &mut Surface, ctx: &UiContext, r: Rect) {
    fill_texture(fb, ctx, r);
    bevel(fb, r.x, r.y, r.w - 1, r.h - 1, 15, 8);
}

/// `edit_line(x, y, maxlen)` (overlay 23 +0x414): the line editor of the name prompts. The text
/// is padded to `max` with blanks and edited in place: the first typed character clears it,
/// typing overwrites and moves right, Left/Right move, Ins/Del/Backspace shift, Enter or Esc
/// (or a click) end it, trailing blanks are dropped. Drawn as a colour-15 field (x, y)
/// 7 * max x 11 inside a black outline (x, y - 1)..(x + 8 * max + 8, y + 12), the characters
/// from (x + 2, y + 2), the cursor cell recoloured 15 -> 11 for 10 ticks of every 20.
pub struct LineEdit {
    x: i32,
    y: i32,
    max: usize,
    font: usize,
    buf: Vec<char>,
    cursor: usize,
    first_key: bool,
    tick: u64,
}

/// How a `LineEdit` key ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineEnd {
    Enter,
    Esc,
}

impl LineEdit {
    pub fn new(x: i32, y: i32, max: usize, font: usize, initial: &str) -> LineEdit {
        let mut buf: Vec<char> = initial.chars().take(max).collect();
        buf.resize(max, ' ');
        LineEdit { x, y, max, font, buf, cursor: 0, first_key: true, tick: 0 }
    }

    /// The text without its trailing blanks (the first character always stays).
    pub fn text(&self) -> String {
        let mut end = self.buf.len();
        while end > 1 && self.buf[end - 1] == ' ' {
            end -= 1;
        }
        self.buf[..end].iter().collect()
    }

    pub fn tick(&mut self) {
        self.tick += 1;
    }

    /// Feed a key (`key`) or a typed character (`ch`); Some when the edit ends.
    pub fn key(&mut self, key: Option<Key>, ch: Option<char>) -> Option<LineEnd> {
        match key {
            Some(Key::Enter) => return Some(LineEnd::Enter),
            Some(Key::Esc) => return Some(LineEnd::Esc),
            Some(Key::Left) => self.cursor = self.cursor.saturating_sub(1),
            Some(Key::Right) => self.cursor = (self.cursor + 1).min(self.max - 1),
            Some(Key::Insert) => {
                self.buf.insert(self.cursor, ' ');
                self.buf.truncate(self.max);
            }
            // CIV.EXE (o 0x1ef53) also blanks the character that moves under the cursor, so
            // Del removes two; civrs removes one
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
            self.cursor = (self.cursor + 1).min(self.max - 1);
        }
        self.first_key = false;
        None
    }

    /// Route an event: Some when the edit ends (a click counts as Enter).
    pub fn handle(&mut self, ev: &Event) -> Option<LineEnd> {
        match ev {
            Event::Key { key: Key::Char(_) | Key::Space, down: true, .. } => None,
            Event::Key { key, down: true, .. } => self.key(Some(*key), None),
            Event::Text(c) => self.key(None, Some(*c)),
            Event::MouseButton { down: true, .. } => self.key(Some(Key::Enter), None),
            _ => None,
        }
    }

    pub fn draw(&self, fb: &mut Surface, ctx: &UiContext) {
        let (x, y) = (self.x, self.y);
        fb.fill_rect(Rect::new(x, y, 7 * self.max as i32, 11), 15);
        fb.rect_outline(Rect::new(x, y - 1, 8 * self.max as i32 + 9, 14), 0);
        let mut cx = x + 2;
        for (i, c) in self.buf.iter().enumerate() {
            let s = c.to_string();
            let w = text_width(&ctx.fonts, self.font, &s);
            fb.fill_rect(Rect::new(cx, y + 2, 8, 8), 15);
            draw_text(fb, &ctx.fonts, &TextStyle::new(self.font, 0), cx, y + 2, &s);
            if i == self.cursor && (self.tick / 10).is_multiple_of(2) {
                for py in y + 1..y + 11 {
                    for px in cx..cx + w {
                        if fb.get(px, py) == 15 {
                            fb.put(px, py, 11);
                        }
                    }
                }
            }
            cx += w;
        }
    }
}

/// CIV.EXE menu texts switch the text colour inline: a byte 0x80 + n draws what follows in
/// colour n (" No Orders \x8fspace" shows the hotkey in 15). The code itself takes no room.
fn is_colour_code(c: char) -> bool {
    ('\u{80}'..='\u{8f}').contains(&c)
}

/// Width of a menu line without its colour codes.
fn coded_width(ctx: &UiContext, font: usize, s: &str) -> i32 {
    let plain: String = s.chars().filter(|c| !is_colour_code(*c)).collect();
    text_width(&ctx.fonts, font, &plain)
}

/// Draw text that may switch colour with 0x80 + n codes (menu lines, report rows).
pub fn draw_coded_text(fb: &mut Surface, ctx: &UiContext, font: usize, x: i32, y: i32, s: &str, colour: u8) {
    draw_coded(fb, ctx, font, x, y, s, colour)
}

/// Draw a menu line, switching colour at each code.
fn draw_coded(fb: &mut Surface, ctx: &UiContext, font: usize, x: i32, y: i32, s: &str, colour: u8) {
    let mut colour = colour;
    let mut x = x;
    let mut run = String::new();
    for c in s.chars() {
        if is_colour_code(c) {
            x = draw_text(fb, &ctx.fonts, &TextStyle::new(font, colour), x, y, &run);
            run.clear();
            colour = c as u32 as u8 - 0x80;
        } else {
            run.push(c);
        }
    }
    draw_text(fb, &ctx.fonts, &TextStyle::new(font, colour), x, y, &run);
}

/// A pop-up menu: optional title lines, selectable items, keyboard and mouse driven.
#[derive(Clone)]
pub struct MenuDialog {
    pub title: Vec<String>,
    pub items: Vec<String>,
    pub sel: usize,
    pub rect: Rect,
    pub cancel_ok: bool,
    pub disabled: Vec<bool>,
    pub min_width: i32,
    pub min_height: i32,
    /// Position override (top-left); None = centred over the map area.
    pub pos: Option<(i32, i32)>,
    pub hotkeys: bool,
    pub style: PanelStyle,
    /// Extra blank gap (in rows) before item index.
    pub gaps: Vec<usize>,
    pub top_pad: i32,
    pub title_colour: Option<u8>,
    /// Font of title and items (`FONT` unless `with_font`), and its row pitch.
    pub font: usize,
    line_h: i32,
    /// menu_select's mouse loop (o 0x1c995..0x1ca0f): the button pressed in this menu, if
    /// any, and the row the pointer highlighted while a button was held.
    pressed: Option<MouseButton>,
    mouse_sel: Option<usize>,
    /// ds:0x2fa0: "(HELP AVAILABLE)" under the lines (Civilopedia help on the items).
    help: bool,
    /// The Civilopedia page of each item, for the callers that test ds:0x2f9c after the menu
    /// (production 0xba5b, research 0xc266): an item chosen with the right button shows its
    /// page and the menu is asked again.
    help_entries: Vec<Option<crate::ui::pedia_data::Entry>>,
    /// A help request closes the menu with `DialogResult::Help` and the caller shows the page
    /// and asks again: choose_production draws it bare (ds:0x3c62, 0xba6c -> o 0xb59f) and
    /// starts over from its listing (0xbaa6 -> 0xaa23).
    help_to_caller: bool,
    /// ds:0xd7f2: items whose leading blank menu_draw turns into a check mark as it draws
    /// them, after measuring (o 0x1d16f), so the box is as wide as without the marks.
    pub checked: Vec<bool>,
    /// ds:0x2fa4 forced to 8: menu_select/menu_draw use a pitch of 8 whatever the font while
    /// a leader's portrait is up (ds:0x3936 != -1, o 0x1c8de).
    pitch: Option<i32>,
    /// ds:0x2f9e: who speaks (0 Spies, 1 Diplomats, 2 Travelers, 3 Defense Minister, 4 Domestic
    /// Advisor, 5 Foreign Minister, 6 Science Advisor); menu_draw widens the frame for the
    /// speaker's picture and name (o 0x1ce49..0x1cf8e).
    speaker: Option<u8>,
    /// Boxes drawn once without input before this menu (popup_menu3 with ds:0xdb38 set) that
    /// stay under it, painted first (`with_under`).
    under: Vec<MenuDialog>,
}

/// The speakers of menu_draw (ds:0x2fa6); the first three get " report:".
pub const SPEAKERS: [&str; 7] = ["Spies", "Diplomats", "Travelers", "Defense Minister:", "Domestic Advisor:", "Foreign Minister:", "Science Advisor:"];
pub const SPEAKER_SPIES: u8 = 0;
pub const SPEAKER_DIPLOMATS: u8 = 1;
pub const SPEAKER_TRAVELERS: u8 = 2;
pub const SPEAKER_DEFENSE: u8 = 3;
pub const SPEAKER_DOMESTIC: u8 = 4;
pub const SPEAKER_FOREIGN: u8 = 5;
pub const SPEAKER_SCIENCE: u8 = 6;

/// The y that `leader_dialog` (o 0x8892) passes: menu_select keeps the background colour 7
/// instead of reading it and recolours 8 (not the stone's 0x16) to 3 in the highlight
/// (o 0x1c8f5..0x1c90c); menu_draw only tests bit 0, so no frame is drawn (o 0x1ce1a).
pub const LEADER_DIALOG_Y: i32 = 0x8b;

/// The name line of speaker `s` (o 0x1ce49..0x1ce73).
pub fn speaker_name(s: u8) -> String {
    let n = SPEAKERS[s as usize % 7];
    if s <= 2 {
        format!("{n} report:")
    } else {
        n.to_string()
    }
}

impl MenuDialog {
    pub fn new(title: Vec<String>, items: Vec<String>) -> MenuDialog {
        let n = items.len();
        MenuDialog {
            title,
            items,
            sel: 0,
            rect: Rect::default(),
            cancel_ok: true,
            disabled: vec![false; n],
            min_width: 0,
            min_height: 0,
            pos: None,
            hotkeys: false,
            style: PanelStyle::Grey,
            gaps: Vec::new(),
            top_pad: 4,
            title_colour: None,
            font: FONT,
            line_h: LINE_H,
            pressed: None,
            mouse_sel: None,
            help: false,
            help_entries: Vec::new(),
            help_to_caller: false,
            checked: Vec::new(),
            pitch: None,
            speaker: None,
            under: Vec::new(),
        }
    }

    /// A draw-only box (popup_menu3 with ds:0xdb38 = 1, which returns at once) left on the
    /// screen under this menu, as the science advice under the research menu (0xc0f2).
    pub fn with_under(mut self, m: MenuDialog) -> Self {
        self.under.push(m);
        self
    }

    /// Rows `p` apart whatever the font (ds:0x2fa4 = 8 with a leader on the screen).
    pub fn with_pitch(mut self, p: i32) -> Self {
        self.pitch = Some(p);
        self
    }

    /// The box of speaker `s` (ds:0x2f9e, see `SPEAKERS`).
    pub fn with_speaker(mut self, s: u8) -> Self {
        self.speaker = Some(s);
        self
    }

    /// menu_draw with ds:0x2fa0 set: the box grows by 2 and "(HELP AVAILABLE)" is written in
    /// the small font, colour 10, at (right - 74, bottom - 4) (o 0x1cf91).
    pub fn with_help(mut self) -> Self {
        self.help = true;
        self
    }

    /// `with_help` plus the Civilopedia page of each item for a right-button choice.
    pub fn with_help_entries(mut self, entries: Vec<Option<crate::ui::pedia_data::Entry>>) -> Self {
        self.help = true;
        self.help_entries = entries;
        self
    }

    /// Help requests go to the caller (`help_to_caller`).
    pub fn with_help_to_caller(mut self) -> Self {
        self.help_to_caller = true;
        self
    }

    /// A help request on item `i`: its Civilopedia page over the menu, or the caller's.
    fn help_page(&self, i: usize) -> Transition {
        if self.help_to_caller {
            return Transition::Pop(DialogResult::Help(i));
        }
        Transition::Push(crate::ui::civilopedia::entry_screen(self.help_entries[i].unwrap(), 0))
    }

    /// Draw the menu once without taking input, as `popup_menu3` does with ds:0xdb38 set
    /// (the advisors' notes under the production menu).
    /// menu_draw alone highlights nothing (the bar is menu_select's).
    pub fn paint(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        self.paint_with(fb, ctx, None);
    }

    /// The menu as it was left on the screen, with row `sel` highlighted.
    pub fn paint_with(&mut self, fb: &mut Surface, ctx: &mut UiContext, sel: Option<usize>) {
        self.layout(ctx);
        self.sel = sel.unwrap_or(usize::MAX);
        self.draw(fb, ctx);
    }

    pub fn with_title_colour(mut self, c: u8) -> Self {
        self.title_colour = Some(c);
        self
    }

    /// Use another font; rows are its line height apart.
    pub fn with_font(mut self, f: usize) -> Self {
        self.font = f;
        self
    }

    pub fn with_pos(mut self, x: i32, y: i32) -> Self {
        self.pos = Some((x, y));
        self
    }
    pub fn with_style(mut self, s: PanelStyle) -> Self {
        self.style = s;
        self
    }
    pub fn with_size(mut self, w: i32, h: i32) -> Self {
        self.min_width = w;
        self.min_height = h;
        self
    }
    pub fn no_cancel(mut self) -> Self {
        self.cancel_ok = false;
        self
    }
    pub fn with_selection(mut self, sel: usize) -> Self {
        self.sel = sel.min(self.items.len().saturating_sub(1));
        self
    }
    pub fn with_hotkeys(mut self) -> Self {
        self.hotkeys = true;
        self
    }
    pub fn with_disabled(mut self, d: Vec<bool>) -> Self {
        if d.len() == self.items.len() {
            self.disabled = d;
        }
        self
    }
    pub fn with_gaps(mut self, g: Vec<usize>) -> Self {
        self.gaps = g;
        self
    }

    fn row_y(&self, i: usize) -> i32 {
        let gaps_before = self.gaps.iter().filter(|g| **g <= i).count() as i32;
        self.items_top() + (i as i32 + gaps_before) * self.line_h
    }

    fn items_top(&self) -> i32 {
        self.rect.y + self.top_pad + self.title.len() as i32 * self.line_h
    }

    pub fn with_top_pad(mut self, p: i32) -> Self {
        self.top_pad = p;
        self
    }

    fn layout(&mut self, ctx: &UiContext) {
        if self.is_popup() {
            self.top_pad = 5;
            // menu_select o 0x1c8c3: the font's row count (0:0x7c3), 8 instead of 9
            let rows = ctx.fonts.get(self.font).line_height();
            self.line_h = self.pitch.unwrap_or(if rows == 9 { 8 } else { rows });
        } else if self.font != FONT {
            self.line_h = ctx.fonts.get(self.font).line_height();
        }
        let (w, h) = if self.is_popup() {
            let (_, w, h) = self.popup_size(ctx);
            // a speaker's name widens the box (ds:0xde0e, o 0x1ce85..0x1ce93)
            let w = match self.speaker {
                Some(s) => w.max(text_width(&ctx.fonts, self.font, &speaker_name(s)) + 8),
                None => w,
            };
            (w + 3, h + 3)
        } else {
            let mut w = self.min_width;
            for t in self.title.iter().chain(self.items.iter()) {
                w = w.max(text_width(&ctx.fonts, self.font, t) + 20);
            }
            let rows = self.items.len() as i32 + self.gaps.len() as i32 + self.title.len() as i32;
            (w, (self.top_pad + 4 + rows * self.line_h).max(self.min_height))
        };
        let (x, y) = match self.pos {
            Some(p) => p,
            None => (80 + (240 - w) / 2, 8 + (192 - h) / 2),
        };
        // a menu larger than the screen is kept at the top left rather than failing; the
        // leader's dialog stays where it is and loses its rows past y 192 (menu_draw clamps the
        // line count to (192 - y) / pitch, o 0x1cdb3)
        let x = x.clamp(0, (320 - w).max(0));
        let y = if y == LEADER_DIALOG_Y && self.is_popup() { y } else { y.clamp(0, (200 - h).max(0)) };
        self.rect = Rect::new(x, y, w, h);
    }

    pub(crate) fn item_at(&self, mx: i32, my: i32) -> Option<usize> {
        if mx < self.rect.x || mx >= self.rect.right() {
            return None;
        }
        (0..self.items.len()).find(|&i| {
            let y = self.row_y(i);
            my >= y - 1 && my < y + self.line_h - 1
        })
    }

    /// Whether this menu is drawn as CIV.EXE's `popup_menu3` (menu_draw 0x1ccab).
    fn is_popup(&self) -> bool {
        matches!(self.style, PanelStyle::Grey | PanelStyle::Framed | PanelStyle::Bare)
    }

    /// A menu line as the menu text holds it: items keep their marker (' ', '_' drawn as a
    /// diamond, '^' as a check mark), anything else gets the selectable blank in front.
    fn item_line(it: &str) -> String {
        if it.starts_with([' ', '_', '^']) {
            it.to_string()
        } else {
            format!(" {it}")
        }
    }

    /// Widest line (ds:0xd4c8) and the frame size (draw_frame's w, h).
    fn popup_size(&self, ctx: &UiContext) -> (i32, i32, i32) {
        let font = self.font;
        let mut widest = 0;
        for t in &self.title {
            widest = widest.max(coded_width(ctx, font, t));
        }
        for it in &self.items {
            widest = widest.max(coded_width(ctx, font, &Self::item_line(it)));
        }
        let rows = ((self.title.len() + self.items.len() + self.gaps.len()) as i32).min(self.max_rows());
        (widest, widest + 8, rows * self.line_h + 6 + if self.help { 2 } else { 0 })
    }

    /// menu_draw's line limit (192 - y) / pitch (o 0x1cdb3); only the leader's dialog, which
    /// is not moved up, reaches it.
    fn max_rows(&self) -> i32 {
        match self.pos {
            Some((_, y)) if y == LEADER_DIALOG_Y => ((192 - y) / self.line_h.max(1)).max(0),
            _ => i32::MAX,
        }
    }

    /// `popup_menu3` as menu_draw/menu_select draw it (see `PanelStyle::Bare` and `Framed`): a
    /// menu at an even y gets the frame, filled with colour 7 or, once a game runs
    /// (ds:0x2f98), with the stone texture of SP299 (`fill_texture` 0x1d7f4: 32x16 tiles from
    /// (288, 120), each restarting at the box corner).
    fn draw_popup(&self, fb: &mut Surface, ctx: &UiContext, cols: &StyleColours) {
        let (x, y) = (self.rect.x, self.rect.y);
        let font = self.font;
        let (widest, w, h) = self.popup_size(ctx);
        let stone = ctx.game.is_some();
        let framed = self.style != PanelStyle::Bare && y % 2 == 0;
        let leader = y == LEADER_DIALOG_Y && self.style != PanelStyle::Bare;
        let frame = |fb: &mut Surface, fx: i32, fy: i32, fw: i32, fh: i32| {
            if stone {
                fill_texture(fb, ctx, Rect::new(fx + 1, fy + 1, fw, fh));
            } else {
                fb.fill_rect(Rect::new(fx + 1, fy + 1, fw, fh), 7);
            }
            bevel(fb, fx + 1, fy + 1, fw, fh, 15, 8);
            fb.rect_outline(Rect::new(fx, fy, fw + 3, fh + 3), 0);
        };
        let bg = if framed {
            match self.speaker {
                Some(s) => self.draw_speaker(fb, ctx, s, w, h, frame),
                None => frame(fb, x, y, w, h),
            }
            7
        } else if leader {
            7
        } else {
            fb.get(x, y)
        };
        // a colour code in front of the title ("\x8cSelect Load File...") overrides it
        let title_colour = self.title_colour.unwrap_or(if bg == 15 { 3 } else { cols.title });
        // the lines are drawn as the menu text holds them, each ended by a line break, so no
        // glyph is the last of its string and all are cut to their advance (see draw_glyph_run)
        let max_rows = self.max_rows();
        for (i, t) in self.title.iter().enumerate().take(max_rows.max(0) as usize) {
            let ty = y + 5 + i as i32 * self.line_h;
            if self.line_h > 9 {
                draw_coded(fb, ctx, font, x + 5, ty + 1, &format!("{t} "), 0);
            }
            draw_coded(fb, ctx, font, x + 5, ty, &format!("{t} "), title_colour);
        }
        for (i, it) in self.items.iter().enumerate() {
            if (self.row_y(i) - self.items_top()) / self.line_h.max(1) + self.title.len() as i32 >= max_rows {
                break;
            }
            let colour = if self.disabled[i] { cols.disabled } else { cols.text };
            let mut line = Self::item_line(it);
            if self.checked.get(i) == Some(&true) {
                line.replace_range(..1, "^");
            }
            draw_coded(fb, ctx, font, x + 5, self.row_y(i), &format!("{line} "), colour);
        }
        if self.help {
            draw_text(fb, &ctx.fonts, &TextStyle::new(1, 10), x + w - 74, y + h - 4, "(HELP AVAILABLE)");
        }
        // the highlight recolours the row's background (o 0x1cab2), so text and the title's
        // descenders keep their colours; on the stone also its second colour 0x16 -> 3. A
        // greyed row the keys rest on is highlighted the same way (its text 3 then vanishes
        // in the bar's pattern, as in CIV.EXE's Diplomat menu at a city with a Palace)
        if let Some(i) = Some(self.sel).filter(|&i| i < self.items.len()) {
            let r = Rect::new(x + 3, self.row_y(i) - 1, widest + BARE_BAR_EXTRA, self.line_h);
            let second = if leader {
                Some(8)
            } else if framed && stone {
                Some(0x16)
            } else {
                None
            };
            for py in r.y..r.bottom() {
                for px in r.x..r.right() {
                    let v = fb.get(px, py);
                    if v == bg {
                        fb.put(px, py, cols.hilite_bar);
                    } else if Some(v) == second {
                        fb.put(px, py, 3);
                    }
                }
            }
        }
    }

    /// menu_draw's speaker box (o 0x1ce49..0x1cf8e): the frame starts 42 px left and 8 px above
    /// the menu, at least 61 high (box height + 8, clamp(.., 61, 999)), as wide as the menu or
    /// the speaker's name + 8. The picture goes 40 px left of the menu: the Spies' / Diplomats'
    /// / Travelers' card (ds:0xdf62, cut from SP299 at (160 + 42 i, 142), 41x58, o5 0x5626) at
    /// y - 5, an advisor's portrait (page 2 (40 s + 40, 140), the GOVT sheet's top strip) at the
    /// bottom of the frame, y - 6 + max(h - 53, 0). The name is drawn at (x + 5, y - 4) in 15
    /// and underlined in 11 at y + 3. Returns nothing: the box's background is 7.
    fn draw_speaker(&self, fb: &mut Surface, ctx: &UiContext, s: u8, w: i32, h: i32, frame: impl Fn(&mut Surface, i32, i32, i32, i32)) {
        let (x, y) = (self.rect.x, self.rect.y);
        let name = speaker_name(s);
        let name_w = text_width(&ctx.fonts, self.font, &name);
        let w = w.max(name_w + 8);
        frame(fb, x - 0x2a, y - 8, w + 0x2a, (h + 8).clamp(0x3d, 999));
        if s <= 2 {
            let sp = ctx.pic("SP299");
            let src_pal = ctx.pic_pal("SP299");
            let mut map = [0u8; 256];
            for (i, m) in map.iter_mut().enumerate() {
                *m = ctx.palette.nearest(src_pal.rgb(i as u8));
            }
            fb.blit_remap(&sp.surf, Rect::new(160 + 42 * s as i32, 142, 41, 58), x - 0x28, y - 5, None, &map);
        } else if ctx.game.is_some() {
            let pic = ctx.pic(&crate::ui::reports::advisor_pic_name(ctx));
            let off = if h - 0x34 > 0 { h - 0x34 - 1 } else { 0 };
            fb.blit_part(&pic.surf, Rect::new(40 * (s as i32 - 3), 0, 40, 60), x - 0x28, y - 6 + off, None);
        }
        draw_text(fb, &ctx.fonts, &TextStyle::new(self.font, 15), x + 5, y - 4, &name);
        fb.hline(x + 5, x + 5 + name_w, y + 3, 11);
    }

    /// menu_select's letter keys (o 0x1cc33..0x1cc16): the next item after the highlighted one
    /// whose hotkey (the line's second character, ds:0xdc4a) matches the key in its low five
    /// bits, so either case; only the highlight moves, and the search stops at the last item.
    fn letter_target(&self, c: char) -> Option<usize> {
        let k = c as u32 & 0x1f;
        (self.sel + 1..self.items.len().min(32)).find(|&i| Self::item_line(&self.items[i]).chars().nth(1).is_some_and(|h| h as u32 & 0x1f == k))
    }

    fn move_sel(&mut self, delta: i32) {
        let n = self.items.len() as i32;
        if n == 0 {
            return;
        }
        // menu_select (o 0x1cb9e, 0x1ca41): Up/Down stop at the ends and may rest on a greyed
        // item, which Enter then ignores
        if self.is_popup() {
            self.sel = (self.sel as i32 + delta).clamp(0, n - 1) as usize;
            return;
        }
        let mut s = self.sel as i32;
        for _ in 0..n {
            s = (s + delta).rem_euclid(n);
            if !self.disabled[s as usize] {
                break;
            }
        }
        self.sel = s as usize;
    }
}

impl Screen for MenuDialog {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        self.layout(ctx);
        if !self.is_popup() && self.disabled.get(self.sel).copied().unwrap_or(false) {
            self.move_sel(1);
        }
    }

    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        let t = self.handle_input(ev, ctx);
        // the chosen row stays up 20 ticks before the menu returns (o 0x1cc73), whoever
        // holds the menu (`ScreenStack::handle`)
        if let Transition::Pop(DialogResult::Choice(i)) = t {
            self.sel = i;
            ctx.menu_pause = MENU_CHOICE_TICKS;
        }
        t
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        for u in &mut self.under {
            u.paint(fb, ctx);
        }
        self.draw_menu(fb, ctx);
    }
}

/// Ticks a chosen menu row stays up (CIV.EXE 0x1cc73). In 16 colours menu_select first
/// turns the row's highlight (11) into the title colour ds:0x654c (0x1cc3c; colour 11 again,
/// so no change, with the help flag ds:0x2f9c, 0x1cb98); in 256 colours (ds:0xd762, as civrs
/// draws) it leaves the highlight as it is (0x1cb89).
pub const MENU_CHOICE_TICKS: u32 = 20;

impl MenuDialog {
    fn handle_input(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        match ev {
            // Alt-V flips the sound option in any menu (o 0x1cbd0)
            Event::Key { key: Key::Char('v'), mods, down: true, .. } if mods.alt => {
                if let Some(g) = ctx.game.as_mut() {
                    g.options.sound = !g.options.sound;
                }
            }
            // Alt-H (0x2300) asks for help on the highlighted item (o 0x1cbbc)
            Event::Key { key: Key::Char('h'), mods, down: true, .. } if mods.alt && self.help_entries.get(self.sel).is_some_and(Option::is_some) => {
                return self.help_page(self.sel);
            }
            Event::Key { key, down: true, .. } => match key {
                Key::Up | Key::Kp(8) => self.move_sel(-1),
                Key::Down | Key::Kp(2) => self.move_sel(1),
                Key::Home => self.sel = 0,
                Key::End => self.sel = self.items.len().saturating_sub(1),
                Key::Enter | Key::Space => {
                    if self.items.is_empty() {
                        return Transition::Pop(DialogResult::Cancelled);
                    }
                    if !self.disabled[self.sel] {
                        return Transition::Pop(DialogResult::Choice(self.sel));
                    }
                }
                Key::Esc => {
                    if self.cancel_ok {
                        return Transition::Pop(DialogResult::Cancelled);
                    }
                }
                Key::Char(c) => {
                    if self.hotkeys {
                        for (i, it) in self.items.iter().enumerate() {
                            let first = it.trim_start().chars().next().map(|f| f.to_ascii_lowercase());
                            if first == Some(*c) && !self.disabled[i] {
                                return Transition::Pop(DialogResult::Choice(i));
                            }
                        }
                    } else if let Some(i) = self.letter_target(*c) {
                        self.sel = i;
                    }
                }
                _ => {}
            },
            // while a button is held the highlight follows the pointer (a greyed row keeps
            // the last one) and the release picks the row under it, or -1 outside the menu;
            // the right button also asks for help (ds:0x2f9c). The press may come from the
            // opener (a press on the menu bar dragged onto an item); a release that neither
            // started here nor passed over a row is ignored.
            Event::MouseMove { x, y } if ctx.mouse_down || self.pressed.is_some() => {
                if let Some(i) = self.item_at(*x, *y) {
                    if !self.disabled[i] {
                        self.sel = i;
                        self.mouse_sel = Some(i);
                    }
                }
            }
            Event::MouseButton { button, x, y, down: true } => {
                self.pressed = Some(*button);
                if let Some(i) = self.item_at(*x, *y).filter(|&i| !self.disabled[i]) {
                    self.sel = i;
                    self.mouse_sel = Some(i);
                }
            }
            Event::MouseButton { button, x, y, down: false } => {
                let ours = self.pressed.take().is_some();
                if !ours && self.mouse_sel.is_none() {
                    return Transition::None;
                }
                let row = match self.item_at(*x, *y) {
                    Some(i) if self.disabled[i] => self.mouse_sel,
                    r => r,
                };
                self.mouse_sel = None;
                return match row {
                    Some(i) if *button == MouseButton::Right && self.help_entries.get(i).is_some_and(Option::is_some) => {
                        self.help_page(i)
                    }
                    Some(i) => Transition::Pop(DialogResult::Choice(i)),
                    None if self.cancel_ok => Transition::Pop(DialogResult::Cancelled),
                    None => Transition::None,
                };
            }
            _ => {}
        }
        Transition::None
    }

    fn draw_menu(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        if self.is_popup() {
            // the menu's own EGA colours; the title colour depends on the background
            let cols = StyleColours { text: 0, title: 15, hilite_bar: 11, hilite_text: 0, disabled: 3 };
            self.draw_popup(fb, ctx, &cols);
            return;
        }
        let cols = draw_styled_panel(fb, ctx, self.rect, self.style);
        let mut y = self.rect.y + self.top_pad;
        let title_colour = self.title_colour.unwrap_or(cols.title);
        for t in &self.title {
            draw_text(fb, &ctx.fonts, &TextStyle::new(self.font, title_colour), self.rect.x + 8, y, t);
            y += self.line_h;
        }
        for (i, it) in self.items.iter().enumerate() {
            let y = self.row_y(i);
            let selected = i == self.sel && !self.disabled[i];
            if selected {
                fb.fill_rect(Rect::new(self.rect.x + 2, y - 1, self.rect.w - 4, self.line_h), cols.hilite_bar);
            }
            let colour = if self.disabled[i] {
                cols.disabled
            } else if selected {
                cols.hilite_text
            } else {
                cols.text
            };
            draw_text(fb, &ctx.fonts, &TextStyle::new(self.font, colour), self.rect.x + 10, y, it);
        }
    }
}

/// A message box with wrapped text; any key or click dismisses it.
pub struct MessageBox {
    pub lines: Vec<String>,
    pub rect: Rect,
    pub pos: Option<(i32, i32)>,
    pub width: i32,
    pub style: PanelStyle,
    /// Optional title drawn in the title colour with an underline (news reports).
    pub title: Option<String>,
    pub text_colour: Option<u8>,
    pub font: usize,
    wrapped: Vec<String>,
    guard: u32,
}

impl MessageBox {
    pub fn new(lines: Vec<String>) -> MessageBox {
        MessageBox { lines, rect: Rect::default(), pos: None, width: 200, style: PanelStyle::Grey, title: None, text_colour: None, font: FONT, wrapped: Vec::new(), guard: 2 }
    }
    pub fn text(s: &str) -> MessageBox {
        MessageBox::new(s.split('\n').map(|l| l.to_string()).collect())
    }
    pub fn with_pos(mut self, x: i32, y: i32) -> Self {
        self.pos = Some((x, y));
        self
    }
    pub fn with_width(mut self, w: i32) -> Self {
        self.width = w;
        self
    }
    pub fn with_style(mut self, s: PanelStyle) -> Self {
        self.style = s;
        self
    }
    pub fn with_title(mut self, t: &str) -> Self {
        self.title = Some(t.to_string());
        self
    }
    pub fn with_text_colour(mut self, c: u8) -> Self {
        self.text_colour = Some(c);
        self
    }
    pub fn with_font(mut self, f: usize) -> Self {
        self.font = f;
        self
    }

    fn layout(&mut self, ctx: &UiContext) {
        self.wrapped.clear();
        for l in &self.lines {
            if l.is_empty() {
                self.wrapped.push(String::new());
            } else {
                self.wrapped.extend(wrap(&ctx.fonts, self.font, self.width - 16, l));
            }
        }
        let mut w = 0;
        for l in self.wrapped.iter().chain(self.title.iter()) {
            w = w.max(text_width(&ctx.fonts, self.font, l) + 16);
        }
        let w = w.min(self.width).max(60);
        let lh = ctx.fonts.get(self.font).line_height();
        let h = 8 + self.wrapped.len() as i32 * lh + if self.title.is_some() { lh + 2 } else { 0 };
        let (x, y) = match self.pos {
            Some(p) => p,
            None => (80 + (240 - w) / 2, 8 + (192 - h) / 2),
        };
        self.rect = Rect::new(x.clamp(0, 320 - w), y.clamp(0, 200 - h), w, h);
    }
}

impl Screen for MessageBox {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        self.layout(ctx);
    }
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        match ev {
            Event::Key { down: true, key, .. } => {
                if matches!(key, Key::Unknown) {
                    return Transition::None;
                }
                Transition::Pop(DialogResult::Done)
            }
            Event::MouseButton { down: true, .. } => Transition::Pop(DialogResult::Done),
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let cols = draw_styled_panel(fb, ctx, self.rect, self.style);
        let lh = ctx.fonts.get(self.font).line_height();
        let mut y = self.rect.y + 4;
        if let Some(t) = &self.title {
            let end = draw_text(fb, &ctx.fonts, &TextStyle::new(self.font, cols.title), self.rect.x + 8, y, t);
            fb.hline(self.rect.x + 8, end, y + lh - 1, cols.title);
            y += lh + 2;
        }
        let colour = self.text_colour.unwrap_or(cols.text);
        for l in &self.wrapped {
            draw_text(fb, &ctx.fonts, &TextStyle::new(self.font, colour), self.rect.x + 8, y, l);
            y += lh;
        }
    }
}

/// Text entry dialog (white box with a black-framed field).
pub struct TextInput {
    pub prompt: Vec<String>,
    pub value: String,
    pub max: usize,
    pub rect: Rect,
    pub pos: Option<(i32, i32)>,
    pub style: PanelStyle,
    guard: u32,
}

impl TextInput {
    pub fn new(prompt: Vec<String>, initial: &str, max: usize) -> TextInput {
        TextInput { prompt, value: initial.to_string(), max, rect: Rect::default(), pos: None, style: PanelStyle::Grey, guard: 1 }
    }
    pub fn with_pos(mut self, x: i32, y: i32) -> Self {
        self.pos = Some((x, y));
        self
    }
    pub fn with_style(mut self, s: PanelStyle) -> Self {
        self.style = s;
        self
    }
}

impl Screen for TextInput {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        let mut w = 120;
        for p in &self.prompt {
            w = w.max(text_width(&ctx.fonts, FONT, p) + 16);
        }
        let h = 8 + (self.prompt.len() as i32 + 1) * LINE_H + 4;
        let (x, y) = match self.pos {
            Some(p) => p,
            None => (80 + (240 - w) / 2, 8 + (192 - h) / 2),
        };
        self.rect = Rect::new(x, y, w, h);
    }
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        match ev {
            Event::Key { key: Key::Enter, down: true, .. } => Transition::Pop(DialogResult::Text(self.value.clone())),
            Event::Key { key: Key::Esc, down: true, .. } => Transition::Pop(DialogResult::Cancelled),
            Event::Key { key: Key::Backspace, down: true, .. } => {
                self.value.pop();
                Transition::None
            }
            Event::Text(c) => {
                if self.value.chars().count() < self.max && (' '..='~').contains(c) {
                    self.value.push(*c);
                }
                Transition::None
            }
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let cols = draw_styled_panel(fb, ctx, self.rect, self.style);
        let mut y = self.rect.y + 4;
        for p in &self.prompt {
            draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, cols.title), self.rect.x + 8, y, p);
            y += LINE_H;
        }
        y += 2;
        let field = Rect::new(self.rect.x + 6, y - 1, self.rect.w - 12, LINE_H + 2);
        fb.fill_rect(field, ctx.palette.nearest([255, 255, 255]));
        fb.rect_outline(field, ctx.palette.nearest([0, 0, 0]));
        let end = draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, ctx.palette.nearest([0, 0, 0])), field.x + 2, y, &self.value);
        if (ctx.frame / 20) % 2 == 0 {
            fb.fill_rect(Rect::new(end + 1, y, 4, LINE_H - 1), ctx.palette.nearest([0, 0, 0]));
        }
    }
}

/// Yes/No question (a two-item menu).
pub struct YesNo {
    menu: MenuDialog,
    /// Item index of the "yes" answer.
    yes: usize,
}

impl YesNo {
    pub fn new(question: Vec<String>, yes: &str, no: &str) -> YesNo {
        YesNo { menu: MenuDialog::new(question, vec![yes.to_string(), no.to_string()]), yes: 0 }
    }
    /// The "no" answer listed (and preselected) first.
    pub fn no_first(question: Vec<String>, no: &str, yes: &str) -> YesNo {
        YesNo { menu: MenuDialog::new(question, vec![no.to_string(), yes.to_string()]), yes: 1 }
    }
    pub fn with_pos(mut self, x: i32, y: i32) -> Self {
        self.menu.pos = Some((x, y));
        self
    }
    pub fn with_title_colour(mut self, c: u8) -> Self {
        self.menu.title_colour = Some(c);
        self
    }
}

impl Screen for YesNo {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        self.menu.on_enter(ctx);
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        match self.menu.handle(ev, ctx) {
            Transition::Pop(DialogResult::Choice(i)) if i == self.yes => Transition::Pop(DialogResult::Yes),
            Transition::Pop(DialogResult::Choice(_)) => Transition::Pop(DialogResult::No),
            t => t,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        self.menu.draw(fb, ctx);
    }
}

/// `ask_city_name` (overlay 23 +0x0, 0x1e975), for founding and renaming: a colour 15 box at
/// (80, 80) 160x32 outlined in 11, "City Name..." at (88, 82) and edit_line(88, 96, 12) on the
/// current name; Enter with an empty name edits again. Pops `Text` on Enter, `Cancelled` on Esc,
/// which keeps the name whatever was typed (o 0x1ea0f). CIV.EXE's loop on an empty name
/// (o 0x1ea06) never repeats, since edit_line keeps the first character, so it takes a blank
/// name as " "; civrs edits again instead.
pub struct CityNamePrompt {
    edit: LineEdit,
    /// Frames to ignore input for (the key that opened the prompt also arrives as text).
    guard: u32,
}

impl CityNamePrompt {
    pub fn new(name: &str) -> CityNamePrompt {
        CityNamePrompt { edit: LineEdit::new(88, 96, 12, FONT, name), guard: 2 }
    }
}

impl Screen for CityNamePrompt {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        self.edit.tick();
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        let text = self.edit.text().trim().to_string();
        match self.edit.handle(ev) {
            Some(LineEnd::Enter) if text.is_empty() => Transition::None,
            Some(LineEnd::Enter) => Transition::Pop(DialogResult::Text(text)),
            Some(LineEnd::Esc) => Transition::Pop(DialogResult::Cancelled),
            None => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        fb.fill_rect(Rect::new(80, 80, 160, 32), 15);
        fb.rect_outline(Rect::new(80, 80, 161, 33), 11);
        draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, 0), 88, 82, "City Name...");
        self.edit.draw(fb, ctx);
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

/// Green "CIVILIZATION NOTE" hint box (tiny font) at the top of the map area: (56,16)-(264,..),
/// black title, text lines as broken in HELP.TXT from x 64 on a 6-px pitch.
pub struct NoteBox {
    pub lines: Vec<String>,
    guard: u32,
    rect: Rect,
    wrapped: Vec<String>,
}

impl NoteBox {
    pub fn new(lines: Vec<String>) -> NoteBox {
        NoteBox { lines, guard: 2, rect: Rect::new(56, 16, 209, 41), wrapped: Vec::new() }
    }
}

impl Screen for NoteBox {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        (self.rect, self.wrapped) = note_layout(ctx, &self.lines);
    }
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => Transition::Pop(DialogResult::Done),
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let cols = draw_styled_panel(fb, ctx, self.rect, PanelStyle::Green);
        draw_note_text(fb, ctx, self.rect, &self.wrapped, cols.text);
    }
}

/// Box and text lines of a CIVILIZATION NOTE (green hint or red error) for HELP.TXT /
/// ERROR.TXT lines: the file's own line breaks are kept (it hyphenates by hand, and its
/// longest lines run up to the right border); only a line made longer by a substituted city
/// name is wrapped.
pub fn note_layout(ctx: &UiContext, lines: &[String]) -> (Rect, Vec<String>) {
    const W: i32 = 209;
    let mut wrapped: Vec<String> = lines.iter().flat_map(|l| wrap(&ctx.fonts, 1, W - 7, &l.to_ascii_uppercase())).collect();
    while wrapped.last().is_some_and(|l| l.is_empty()) {
        wrapped.pop();
    }
    let lh = ctx.fonts.get(1).line_height();
    (Rect::new(56, 16, W, 11 + wrapped.len() as i32 * lh), wrapped)
}

/// Black centred title, then the text lines from 8 px in, all on the tiny font's 6-px pitch.
pub fn draw_note_text(fb: &mut Surface, ctx: &UiContext, r: Rect, lines: &[String], colour: u8) {
    let lh = ctx.fonts.get(1).line_height();
    let mut y = r.y + 3;
    crate::gfx::text::draw_text_centered(fb, &ctx.fonts, &TextStyle::new(1, theme::BLACK), r.x + r.w / 2, y, "--- CIVILIZATION NOTE ---");
    y += lh;
    for l in lines {
        draw_text(fb, &ctx.fonts, &TextStyle::new(1, colour), r.x + 8, y, l);
        y += lh;
    }
}

/// Large text box (help/civilopedia-style) with a title.
pub struct TextBox {
    pub title: String,
    pub paras: Vec<String>,
    pub rect: Rect,
    pub font: usize,
    guard: u32,
}

impl TextBox {
    pub fn new(title: &str, paras: Vec<String>, rect: Rect) -> TextBox {
        TextBox { title: title.to_string(), paras, rect, font: FONT, guard: 2 }
    }
}

impl Screen for TextBox {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => Transition::Pop(DialogResult::Done),
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let cols = draw_styled_panel(fb, ctx, self.rect, PanelStyle::Grey);
        let mut y = self.rect.y + 4;
        if !self.title.is_empty() {
            draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, cols.title), self.rect.x + 8, y, &self.title);
            y += LINE_H + 2;
        }
        let body = Rect::new(self.rect.x + 8, y, self.rect.w - 16, self.rect.bottom() - y - 4);
        draw_paragraph(fb, &ctx.fonts, &TextStyle::new(self.font, cols.text), body, &self.paras, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Option<UiContext> {
        let dir = crate::assets::AssetDir::locate(None).ok()?;
        UiContext::new(dir, std::env::temp_dir().join(format!("civrs-menumouse-{}", std::process::id()))).ok()
    }

    fn button(button: MouseButton, x: i32, y: i32, down: bool) -> Event {
        Event::MouseButton { button, x, y, down }
    }

    fn menu(ctx: &mut UiContext) -> MenuDialog {
        let mut m = MenuDialog::new(vec!["Title".into()], vec!["One".into(), "Two".into(), "Three".into()]).with_pos(80, 40);
        m.on_enter(ctx);
        m
    }

    #[test]
    fn a_chosen_row_stays_up_twenty_ticks() {
        // menu_select: wait_ticks(20) after a choice (o 0x1cc73), no input meanwhile; none
        // after Esc
        struct Base;
        impl Screen for Base {
            fn handle(&mut self, _ev: &Event, _ctx: &mut UiContext) -> Transition {
                Transition::None
            }
            fn draw(&mut self, _fb: &mut Surface, _ctx: &mut UiContext) {}
        }
        let Some(mut ctx) = ctx() else { return };
        let key = |k| Event::Key { key: k, mods: Default::default(), down: true, repeat: false };
        let mut st = crate::ui::screen::ScreenStack::new();
        st.push(Box::new(Base), &mut ctx);
        st.push(Box::new(menu(&mut ctx)), &mut ctx);
        st.handle(&key(Key::Enter), &mut ctx);
        st.handle(&key(Key::Esc), &mut ctx);
        for _ in 1..MENU_CHOICE_TICKS {
            st.update(&mut ctx);
            assert_eq!(st.len(), 2);
        }
        st.update(&mut ctx);
        assert_eq!(st.len(), 1);
        let mut m = menu(&mut ctx);
        m.cancel_ok = true;
        st.push(Box::new(m), &mut ctx);
        st.handle(&key(Key::Esc), &mut ctx);
        assert_eq!(st.len(), 1);
    }

    #[test]
    fn a_menu_choice_is_made_on_release() {
        let Some(mut ctx) = ctx() else { return };
        let mut m = menu(&mut ctx);
        let (x, y1, y2) = (m.rect.x + 10, m.row_y(1) + 2, m.row_y(2) + 2);
        // pressed on one row, released on another: the second (menu_select 0x1ca03)
        assert!(matches!(m.handle(&button(MouseButton::Left, x, y1, true), &mut ctx), Transition::None));
        assert!(matches!(m.handle(&button(MouseButton::Left, x, y2, false), &mut ctx), Transition::Pop(DialogResult::Choice(2))));
        // released outside: -1
        let mut m = menu(&mut ctx);
        m.handle(&button(MouseButton::Left, x, y1, true), &mut ctx);
        assert!(matches!(m.handle(&button(MouseButton::Left, 2, 2, false), &mut ctx), Transition::Pop(DialogResult::Cancelled)));
        // the right button chooses too (and only asks for help in menus with pages)
        let mut m = menu(&mut ctx);
        m.handle(&button(MouseButton::Right, x, y1, true), &mut ctx);
        assert!(matches!(m.handle(&button(MouseButton::Right, x, y1, false), &mut ctx), Transition::Pop(DialogResult::Choice(1))));
    }

    #[test]
    fn a_press_from_the_menu_bar_can_be_dragged_onto_an_item() {
        let Some(mut ctx) = ctx() else { return };
        let mut m = menu(&mut ctx);
        let (x, y1) = (m.rect.x + 10, m.row_y(1) + 2);
        ctx.mouse_down = true;
        // a release where the press began (the bar) leaves the menu open
        assert!(matches!(m.handle(&button(MouseButton::Left, 20, 3, false), &mut ctx), Transition::None));
        m.handle(&Event::MouseMove { x, y: y1 }, &mut ctx);
        assert!(matches!(m.handle(&button(MouseButton::Left, x, y1, false), &mut ctx), Transition::Pop(DialogResult::Choice(1))));
    }

    #[test]
    fn a_right_choice_in_a_help_menu_shows_the_page_and_stays() {
        let Some(mut ctx) = ctx() else { return };
        let mut m = MenuDialog::new(vec!["Title".into()], vec!["One".into(), "Two".into()]).with_pos(80, 40).with_help_entries(vec![Some(crate::ui::pedia_data::Entry::Advance(1)), None]);
        m.on_enter(&mut ctx);
        let (x, y0, y1) = (m.rect.x + 10, m.row_y(0) + 2, m.row_y(1) + 2);
        m.handle(&button(MouseButton::Right, x, y0, true), &mut ctx);
        assert!(matches!(m.handle(&button(MouseButton::Right, x, y0, false), &mut ctx), Transition::Push(_)));
        m.handle(&button(MouseButton::Right, x, y1, true), &mut ctx);
        assert!(matches!(m.handle(&button(MouseButton::Right, x, y1, false), &mut ctx), Transition::Pop(DialogResult::Choice(1))));
    }

    #[test]
    fn letters_move_the_highlight_to_the_next_matching_item() {
        let mut m = MenuDialog::new(vec![], vec!["Keep Playing".into(), "Yes, Quit".into(), "Keep going".into()]);
        assert_eq!(m.letter_target('y'), Some(1));
        // the search starts after the highlighted item and does not wrap
        assert_eq!(m.letter_target('k'), Some(2));
        m.sel = 2;
        assert_eq!(m.letter_target('k'), None);
        assert_eq!(m.letter_target('y'), None);
        // upper and lower case match alike (low five bits)
        m.sel = 0;
        assert_eq!(m.letter_target('Y'), Some(1));
    }
}
