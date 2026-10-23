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
            hover: None,
        }
    }

    pub fn with_title_colour(mut self, c: u8) -> Self {
        self.title_colour = Some(c);
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
        self.items_top() + (i as i32 + gaps_before) * LINE_H
    }

    fn items_top(&self) -> i32 {
        self.rect.y + self.top_pad + self.title.len() as i32 * LINE_H
    }

    pub fn with_top_pad(mut self, p: i32) -> Self {
        self.top_pad = p;
        self
    }

    fn layout(&mut self, ctx: &UiContext) {
        let mut w = self.min_width;
        for t in self.title.iter().chain(self.items.iter()) {
            w = w.max(text_width(&ctx.fonts, FONT, t) + 20);
        }
        let rows = self.items.len() as i32 + self.gaps.len() as i32 + self.title.len() as i32;
        let h = (self.top_pad + 4 + rows * LINE_H).max(self.min_height);
        let (x, y) = match self.pos {
            Some(p) => p,
            None => (80 + (240 - w) / 2, 8 + (192 - h) / 2),
        };
        let x = x.clamp(0, 320 - w);
        let y = y.clamp(0, 200 - h);
        self.rect = Rect::new(x, y, w, h);
    }

    fn item_at(&self, mx: i32, my: i32) -> Option<usize> {
        if mx < self.rect.x || mx >= self.rect.right() {
            return None;
        }
        (0..self.items.len()).find(|&i| {
            let y = self.row_y(i);
            my >= y - 1 && my < y + LINE_H - 1
        })
    }

    fn move_sel(&mut self, delta: i32) {
        let n = self.items.len() as i32;
        if n == 0 {
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
        if self.disabled.get(self.sel).copied().unwrap_or(false) {
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
        let cols = draw_styled_panel(fb, ctx, self.rect, self.style);
        let mut y = self.rect.y + self.top_pad;
        let title_colour = self.title_colour.unwrap_or(cols.title);
        for t in &self.title {
            draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, title_colour), self.rect.x + 8, y, t);
            y += LINE_H;
        }
        for (i, it) in self.items.iter().enumerate() {
            let y = self.row_y(i);
            let selected = i == self.sel && !self.disabled[i];
            if selected {
                fb.fill_rect(Rect::new(self.rect.x + 2, y - 1, self.rect.w - 4, LINE_H), cols.hilite_bar);
            }
            let colour = if self.disabled[i] {
                cols.disabled
            } else if selected {
                cols.hilite_text
            } else {
                cols.text
            };
            draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, colour), self.rect.x + 10, y, it);
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
