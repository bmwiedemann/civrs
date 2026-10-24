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
    /// the bottom and left in 15, black outline (x, y)..(x+w+2, y+h+2), where w = widest line
    /// + 8 and h = lines * pitch + 6. Inside, the layout and colours are `Bare`'s with
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
    let sheet = &ctx.pic("SP299").surf;
    let mut ty = 0;
    while ty < r.h {
        let mut tx = 0;
        while tx < r.w {
            fb.blit_part(sheet, Rect::new(288, 120, (r.w - tx).min(32), (r.h - ty).min(16)), r.x + tx, r.y + ty, None);
            tx += 32;
        }
        ty += 16;
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
    hover: Option<usize>,
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
            hover: None,
        }
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
            self.line_h = if rows == 9 { 8 } else { rows };
        } else if self.font != FONT {
            self.line_h = ctx.fonts.get(self.font).line_height();
        }
        let (w, h) = if self.is_popup() {
            let (_, w, h) = self.popup_size(ctx);
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
        // a menu larger than the screen is kept at the top left rather than failing
        let x = x.clamp(0, (320 - w).max(0));
        let y = y.clamp(0, (200 - h).max(0));
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
        let rows = (self.title.len() + self.items.len() + self.gaps.len()) as i32;
        (widest, widest + 8, rows * self.line_h + 6)
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
        let bg = if framed {
            if stone {
                fill_texture(fb, ctx, Rect::new(x + 1, y + 1, w, h));
            } else {
                fb.fill_rect(Rect::new(x + 1, y + 1, w, h), 7);
            }
            bevel(fb, x + 1, y + 1, w, h, 15, 8);
            fb.rect_outline(Rect::new(x, y, w + 3, h + 3), 0);
            7
        } else {
            fb.get(x, y)
        };
        // a colour code in front of the title ("\x8cSelect Load File...") overrides it
        let title_colour = self.title_colour.unwrap_or(if bg == 15 { 3 } else { cols.title });
        // the lines are drawn as the menu text holds them, each ended by a line break, so no
        // glyph is the last of its string and all are cut to their advance (see draw_glyph_run)
        for (i, t) in self.title.iter().enumerate() {
            let ty = y + 5 + i as i32 * self.line_h;
            if self.line_h > 9 {
                draw_coded(fb, ctx, font, x + 5, ty + 1, &format!("{t} "), 0);
            }
            draw_coded(fb, ctx, font, x + 5, ty, &format!("{t} "), title_colour);
        }
        for (i, it) in self.items.iter().enumerate() {
            let colour = if self.disabled[i] { cols.disabled } else { cols.text };
            draw_coded(fb, ctx, font, x + 5, self.row_y(i), &format!("{} ", Self::item_line(it)), colour);
        }
        // the highlight recolours the row's background (o 0x1cab2), so text and the title's
        // descenders keep their colours; on the stone also its second colour 0x16 -> 3
        if let Some(i) = Some(self.sel).filter(|&i| i < self.items.len() && !self.disabled[i]) {
            let r = Rect::new(x + 3, self.row_y(i) - 1, widest + BARE_BAR_EXTRA, self.line_h);
            let second = if framed && stone { Some(0x16) } else { None };
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

    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        match ev {
            Event::Key { key, down: true, .. } => match key {
                Key::Up | Key::Kp(8) => self.move_sel(-1),
                Key::Down | Key::Kp(2) => self.move_sel(1),
                Key::Home => self.sel = 0,
                Key::End => self.sel = self.items.len().saturating_sub(1),
                Key::Enter | Key::Space => {
                    if !self.items.is_empty() && !self.disabled[self.sel] {
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
                    }
                }
                _ => {}
            },
            Event::MouseMove { x, y } => {
                self.hover = self.item_at(*x, *y);
                if let Some(i) = self.hover {
                    if !self.disabled[i] {
                        self.sel = i;
                    }
                }
            }
            Event::MouseButton { button: MouseButton::Left, x, y, down: true } => {
                if let Some(i) = self.item_at(*x, *y) {
                    if !self.disabled[i] {
                        return Transition::Pop(DialogResult::Choice(i));
                    }
                } else if !self.rect.contains(*x, *y) && self.cancel_ok {
                    return Transition::Pop(DialogResult::Cancelled);
                }
            }
            Event::MouseButton { button: MouseButton::Right, down: true, .. } => {
                if self.cancel_ok {
                    return Transition::Pop(DialogResult::Cancelled);
                }
            }
            _ => {}
        }
        Transition::None
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
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
}

impl YesNo {
    pub fn new(question: Vec<String>, yes: &str, no: &str) -> YesNo {
        YesNo { menu: MenuDialog::new(question, vec![yes.to_string(), no.to_string()]) }
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
            Transition::Pop(DialogResult::Choice(0)) => Transition::Pop(DialogResult::Yes),
            Transition::Pop(DialogResult::Choice(_)) => Transition::Pop(DialogResult::No),
            t => t,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        self.menu.draw(fb, ctx);
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
