//! The palace screen (CIV.EXE overlay 0x15f76..0x1700a): CBACK.PIC sky and road with the
//! palace built from CASTLE{level}.PIC pieces, the road in front from CBACKS{n}.PIC and the
//! gardens from CBRUSH{n}.PIC.
//!
//! A palace addition shows the KING.TXT PALACE text over the palace, then "Which section
//! shall we improve?" with the sections 1..7 (building, left to right) and A, B, C (left
//! garden, road, right garden) that can be raised; a building section then offers its three
//! styles (castle, classical, oriental) side by side. The improved palace dissolves in
//! (driver duration 20) and any key closes the screen.

use crate::game::palace::{Kind, Palace};
use crate::gfx::text::{draw_text, draw_text_centered};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::{Event, Key};
use crate::ui::context::UiContext;
use crate::ui::eventpics::{draw_caption, Dismiss};
use crate::ui::reveal::Reveal;
use crate::ui::screen::{DialogResult, Screen, Transition};

/// Driver duration of the dissolve (overlay call site 0x166f5: `push 20; push 1; call dissolve`).
const DISSOLVE: u32 = 20;
/// Top of the palace pieces.
const TOP: i32 = 36;
/// Piece widths used for the layout: centre, wall, left tower, right tower (ds:0x4b2c..).
const W_MIDDLE: i32 = 48;
const W_WALL: i32 = 24;
const W_TOWER_L: i32 = 25;
const W_TOWER_R: i32 = 25;
/// Vertical offset of piece type 0..4 per level and style (ds:0x4b36).
const OFFSET: [[[i32; 5]; 3]; 5] = [
    [[1, 1, 1, 1, 1], [0, 0, 0, 0, 0], [0, 0, 0, 0, 0]],
    [[1, 0, 0, 2, 2], [1, 0, 0, 1, 1], [1, 0, 0, 2, 2]],
    [[1, 0, 0, 2, 2], [1, 0, 0, 1, 1], [1, 0, 0, 2, 2]],
    [[1, 3, 3, 2, 2], [1, 0, 0, 1, 1], [1, 0, 0, 2, 2]],
    [[1, 3, 3, 2, 2], [1, 0, 0, 1, 1], [1, 0, 0, 2, 2]],
];
/// Palette indices the original uses on this screen.
const BLACK: u8 = 0;
const CHOOSER_BG: u8 = 9;
const STYLE_BG: u8 = 1;
const WHITE: u8 = 15;
const YELLOW: u8 = 14;

/// Piece `t` of `style` in a CASTLE sheet: each style has a 160x100 quadrant with the centre
/// (52 px), two walls (24 px) and the left and right towers; the classical style takes its
/// wider towers from the fourth quadrant.
pub fn piece_rect(style: u8, t: u8) -> Rect {
    let bx = (style as i32 & 1) * 160;
    let by = (style as i32 & 2) * 50 + 1;
    match (t, style) {
        (0, _) => Rect::new(bx, by, 52, 99),
        (1, _) => Rect::new(bx + 53, by, 24, 99),
        (2, _) => Rect::new(bx + 78, by, 24, 99),
        (3, 1) => Rect::new(160, 101, 35, 99),
        (3, _) => Rect::new(bx + 103, by, 28, 99),
        (4, 1) => Rect::new(196, 101, 35, 99),
        _ => Rect::new(bx + 132, by, 27, 99),
    }
}

fn blit_piece(fb: &mut Surface, ctx: &UiContext, level: i8, style: u8, t: u8, x: i32, y: i32) {
    let pic = ctx.pic(&format!("CASTLE{}", level.clamp(0, 4)));
    fb.blit_part(&pic.surf, piece_rect(style, t), x, y, Some(0));
}

/// Piece `k` of section `i` (0x16da4); `lvl` picks the vertical offset row.
fn draw_piece(fb: &mut Surface, ctx: &UiContext, p: &Palace, lvl: i8, i: usize, x: i32, y: i32, k: usize) {
    let Some(t) = p.pieces(i, true)[k] else { return };
    let s = p.style[i];
    let row = lvl.clamp(0, 4) as usize;
    let dy = lvl as i32 + OFFSET[row][s as usize % 3][t as usize];
    blit_piece(fb, ctx, p.level[i], s, t, x, y + dy);
}

/// The palace in the status panel (CIV.EXE 0x3823): each building section above level 0 as
/// a 7x15 piece of SP257, 7 px apart from `x`. A style row of SP257 (y 16 * style) holds four
/// levels of five pieces from x 160: left tower, left wall, centre, right wall, right tower.
pub fn draw_small_palace(fb: &mut Surface, ctx: &UiContext, palace: &Palace, x: i32, y: i32) {
    let lv = |i: usize| palace.level.get(i).copied().unwrap_or(-1);
    for s in 0..9usize {
        let level = lv(s);
        if level <= 0 {
            continue;
        }
        let piece = match s {
            0..=3 if lv(s.wrapping_sub(1)) == -1 || s == 1 => 0,
            0..=3 => 1,
            5..=8 if lv(s + 1) == -1 || s == 7 => 4,
            5..=8 => 3,
            _ => 2,
        };
        let idx = (level as i32 - 1).clamp(0, 3) * 5 + piece;
        let src = Rect::new(160 + 7 * idx, 16 * palace.style[s] as i32, 7, 15);
        fb.blit_part(&ctx.sp.surf, src, x + 7 * s as i32, y - 1, Some(0));
    }
}

/// The whole picture: sky, road, palace and gardens (0x16761).
pub fn draw_palace(fb: &mut Surface, ctx: &UiContext, p: &Palace) {
    fb.blit(&ctx.pic("CBACK").surf, 0, 0);
    if p.level[10] > 0 {
        fb.blit(&ctx.pic(&format!("CBACKS{}", p.level[10].min(3))).surf, 0, 135);
    }
    let mut x = 16;
    if p.level[2] == -1 {
        x -= 24;
    }
    if p.style[4] == 1 {
        x += 1;
    }
    let y = TOP;
    let right = (1..=7).rev().find(|i| p.level[*i] != -1).unwrap_or(5);
    let mut last_left = 3;
    for i in 1..=7usize {
        let lvl = p.level[i];
        if lvl == -1 {
            x += 48;
            continue;
        }
        match p.kind(i) {
            Kind::LeftEnd => {
                if i == 3 {
                    draw_piece(fb, ctx, p, lvl, i, x + W_TOWER_L, y, 1);
                }
                // a classical tower is drawn last, over its neighbour
                if p.style[i] != 1 || i == 3 {
                    draw_piece(fb, ctx, p, lvl, i, x, y, 0);
                }
                x += W_TOWER_L;
                if i == 3 {
                    x += W_WALL;
                }
                last_left = i;
            }
            Kind::Inner => {
                if last_left + 1 == i && p.style[i] == p.style[last_left] {
                    draw_piece(fb, ctx, p, lvl, last_left, x + W_WALL - 17, y - 2, 0);
                }
                if right == i + 1 && p.style[i] == p.style[right] {
                    draw_piece(fb, ctx, p, lvl, right, x + W_WALL - 17, y - 2, 1);
                }
                draw_piece(fb, ctx, p, lvl, i, x, y, 0);
                x += W_WALL;
                draw_piece(fb, ctx, p, lvl, i, x, y, 1);
                x += W_WALL;
            }
            Kind::Middle => {
                if p.style[4] == 1 {
                    x -= 1;
                }
                draw_piece(fb, ctx, p, lvl, i, x - 2, y, 0);
                x += W_MIDDLE;
                if p.style[4] == 1 {
                    x -= 1;
                }
            }
            Kind::RightEnd => {
                if i == 5 {
                    draw_piece(fb, ctx, p, lvl, i, x, y, 0);
                    x += W_WALL;
                }
                let step = p.level[i - 1] - lvl;
                if p.style[i] == 1 && step > 0 {
                    draw_piece(fb, ctx, p, lvl, i, x - 3, y + step as i32 + 1, 1);
                } else {
                    draw_piece(fb, ctx, p, lvl, i, x - if p.style[i] == 1 { 3 } else { 2 }, y, 1);
                }
                x += W_TOWER_R;
            }
        }
    }
    if p.style[last_left] == 1 && last_left != 3 {
        let lvl = p.level[last_left];
        let step = (p.level[last_left + 1] - lvl) as i32;
        draw_piece(fb, ctx, p, lvl, last_left, 48 * last_left as i32 - 36, y + step, 0);
    }
    for (part, x, odd) in [(9usize, 0, 0), (11usize, 184, 1)] {
        let l = p.level[part];
        if l > 0 {
            let brush = ctx.pic(&format!("CBRUSH{}", 2 * l.min(3) as i32 - 2 + odd));
            fb.blit_part(&brush.surf, brush.surf.bounds(), x, if l > 1 { 94 } else { 105 }, Some(0));
        }
    }
}

/// Text with the original's black shadow one pixel below (0x182:0x86).
fn label(fb: &mut Surface, ctx: &UiContext, x: i32, y: i32, s: &str, colour: u8) {
    draw_text(fb, &ctx.fonts, &TextStyle::new(0, BLACK), x, y + 1, s);
    draw_text(fb, &ctx.fonts, &TextStyle::new(0, colour), x, y, s);
}

/// The sections that can be raised, labelled under the palace.
fn draw_section_labels(fb: &mut Surface, ctx: &UiContext, p: &Palace) {
    for i in 1..=7usize {
        if p.offered(i) {
            label(fb, ctx, 48 * i as i32 - 36, 144, &i.to_string(), YELLOW);
        }
    }
    for j in 0..3usize {
        if p.offered(9 + j) {
            label(fb, ctx, 120 * j as i32 + 40, 160, &((b'A' + j as u8) as char).to_string(), YELLOW);
        }
    }
}

/// The three styles of section `part` at its next level (0x16354..0x16623).
fn draw_style_chooser(fb: &mut Surface, ctx: &UiContext, p: &Palace, part: usize) {
    fb.fill_rect(Rect::new(16, 2, 288, 102), CHOOSER_BG);
    draw_text_centered(fb, &ctx.fonts, &TextStyle::new(0, WHITE), 160, 3, "Select new section.");
    let lvl = p.next_level(part);
    let row = &OFFSET[lvl as usize];
    let types = p.pieces(part, false);
    for j in 1..=3i32 {
        let s = (j - 1) as u8;
        let off = |t: usize| row[s as usize][t] + 1;
        let x0 = 100 * j - 80;
        fb.fill_rect(Rect::new(x0, 10, 80, 92), STYLE_BG);
        let mut x = x0 + 16;
        let piece = |fb: &mut Surface, k: usize, x: i32, y: i32| {
            if let Some(t) = types[k] {
                blit_piece(fb, ctx, lvl, s, t, x, y);
            }
        };
        match p.kind(part) {
            Kind::LeftEnd => {
                if part != 1 {
                    piece(fb, 1, x + W_TOWER_L, off(1));
                }
                piece(fb, 0, x, off(3));
            }
            Kind::Inner => {
                piece(fb, 0, x, off(1));
                x += W_WALL;
                piece(fb, 1, x, off(2));
            }
            Kind::Middle => piece(fb, 0, x, off(0)),
            Kind::RightEnd => {
                if part != 7 {
                    piece(fb, 0, x, off(1));
                    x += W_WALL;
                }
                piece(fb, 1, x - if j == 2 { 3 } else { 2 }, off(4));
            }
        }
        label(fb, ctx, 100 * j - 44, 12, &j.to_string(), WHITE);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Just the palace (with an optional caption box).
    View,
    /// KING.TXT PALACE text over the palace; any key continues.
    Intro,
    /// "Which section shall we improve?"
    Section,
    /// Style chooser for a building section.
    Style(usize),
    /// The improved palace dissolves in; any key closes.
    Built,
}

pub struct PalaceScreen {
    palace: Palace,
    lines: Vec<String>,
    phase: Phase,
    dismiss: Dismiss,
    reveal: Option<Reveal>,
}

impl PalaceScreen {
    fn new(ctx: &UiContext, lines: Vec<String>, phase: Phase) -> PalaceScreen {
        let palace = ctx.game.as_ref().map(|g| g.human().palace()).unwrap_or_default();
        PalaceScreen { palace, lines, phase, dismiss: Dismiss::new(), reveal: None }
    }

    fn improve(&mut self, ctx: &mut UiContext, part: usize, style: u8) -> Transition {
        self.palace.improve(part, style);
        if let Some(g) = ctx.game.as_mut() {
            let p = self.palace;
            g.human_mut().set_palace(&p);
        }
        self.reveal = Some(Reveal::new(ctx, DISSOLVE));
        self.phase = Phase::Built;
        self.dismiss = Dismiss::new();
        Transition::None
    }

    /// A section key ('1'..'7', 'a'..'c') or a click on the label rows (0x161a6..0x16243).
    fn section_choice(ev: &Event) -> Option<usize> {
        match ev {
            Event::Key { key: Key::Char(c), down: true, .. } => match c.to_ascii_lowercase() {
                c @ '1'..='7' => Some(c as usize - '0' as usize),
                c @ 'a'..='c' => Some(9 + c as usize - 'a' as usize),
                _ => None,
            },
            Event::Key { key: Key::Kp(n), down: true, .. } if (1..=7).contains(n) => Some(*n as usize),
            Event::MouseButton { x, y, down: true, .. } if *y < 154 => Some(((x + 12) / 48 + 1).clamp(1, 7) as usize),
            Event::MouseButton { x, down: true, .. } => Some((x / 107).clamp(0, 2) as usize + 9),
            _ => None,
        }
    }

    /// A style key ('1'..'3') or a click on one of the three columns.
    fn style_choice(ev: &Event) -> Option<u8> {
        match ev {
            Event::Key { key: Key::Char(c @ '1'..='3'), down: true, .. } => Some(*c as u8 - b'1'),
            Event::Key { key: Key::Kp(n @ 1..=3), down: true, .. } => Some(n - 1),
            Event::MouseButton { x, down: true, .. } => Some((x / 107).clamp(0, 2) as u8),
            _ => None,
        }
    }
}

/// The palace of the human player with an optional caption.
pub fn palace_screen(ctx: &UiContext, lines: Vec<String>) -> Box<dyn Screen> {
    Box::new(PalaceScreen::new(ctx, lines, Phase::View))
}

/// A palace addition: KING.TXT PALACE text, then the section and style choice.
pub fn palace_addition(ctx: &UiContext) -> Box<dyn Screen> {
    let lines = ctx.texts.king.section("PALACE").map(crate::assets::text::paragraphs).unwrap_or_default();
    Box::new(PalaceScreen::new(ctx, lines, Phase::Intro))
}

/// Palace screen opened at a later step, for previews: "section" or "styleN".
pub fn palace_addition_at(ctx: &UiContext, step: &str) -> Box<dyn Screen> {
    let mut s = PalaceScreen::new(ctx, Vec::new(), Phase::Section);
    if let Some(part) = step.strip_prefix("style").and_then(|n| n.parse().ok()).and_then(|n| s.palace.pick(n)) {
        s.phase = Phase::Style(part);
    }
    Box::new(s)
}

impl Screen for PalaceScreen {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        if let Some(g) = ctx.game.as_ref() {
            ctx.audio.national_theme(g.human().civ, true);
        }
    }
    fn on_exit(&mut self, ctx: &mut UiContext) {
        ctx.audio.fade_out();
    }
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.dismiss.tick();
        Transition::None
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        match self.phase {
            Phase::View | Phase::Built => {
                if !self.dismiss.closes(ev) {
                    return Transition::None;
                }
                match self.reveal.as_mut() {
                    Some(r) if !r.done() => {
                        r.finish();
                        Transition::None
                    }
                    _ => Transition::Pop(DialogResult::Done),
                }
            }
            Phase::Intro => {
                if self.dismiss.closes(ev) {
                    if !self.palace.any_offered() {
                        return Transition::Pop(DialogResult::Done);
                    }
                    self.phase = Phase::Section;
                }
                Transition::None
            }
            Phase::Section => match Self::section_choice(ev).and_then(|n| self.palace.pick(n)) {
                Some(part) if Palace::is_garden(part) => self.improve(ctx, part, 0),
                Some(part) => {
                    self.phase = Phase::Style(part);
                    Transition::None
                }
                None => Transition::None,
            },
            Phase::Style(part) => match Self::style_choice(ev) {
                Some(style) => self.improve(ctx, part, style),
                None => Transition::None,
            },
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let pal = ctx.pic_pal("CBACK");
        let mut content = Surface::screen();
        draw_palace(&mut content, ctx, &self.palace);
        match self.phase {
            Phase::View if !self.lines.is_empty() => {
                draw_caption(&mut content, ctx, &pal, 20, 16, 280, &self.lines, 0);
            }
            Phase::Intro => {
                draw_caption(&mut content, ctx, &pal, 20, 16, 280, &self.lines, 0);
            }
            Phase::Section => {
                draw_caption(&mut content, ctx, &pal, 40, 16, 240, &["Which section shall we improve?".to_string()], 0);
                draw_section_labels(&mut content, ctx, &self.palace);
            }
            Phase::Style(part) => draw_style_chooser(&mut content, ctx, &self.palace, part),
            _ => {}
        }
        match self.reveal.as_mut() {
            Some(r) => r.draw(fb, &content, &pal, &pal),
            None => fb.blit(&content, 0, 0),
        }
    }
    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.pic_pal("CBACK")
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        !matches!(self.phase, Phase::View | Phase::Intro | Phase::Built)
    }
}
