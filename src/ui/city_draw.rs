//! Drawing primitives of the city screen: the blue stipple panels, buttons, header bands,
//! the 8x8 resource icons, citizen figures, unit boxes and improvement icons, all measured
//! from captures of the original (SP257 palette indices).

use crate::game::{Slot, Specialist};
use crate::assets::fonts::GLYPH_SPACING;
use crate::gfx::text::{draw_text, ink_width};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::rules::SLOT_COLOURS;
use crate::ui::context::UiContext;
use crate::ui::tilerender::{blit_sprite_remap, civ_remap, sp};

/// Light stipple colour (121,142,255).
pub const LIGHT: u8 = 9;
/// Dark stipple colour (117,117,239).
pub const DARK: u8 = 57;
/// Panel border, header bands and button labels (49,77,178).
pub const BORDER: u8 = 1;
pub const WHITE: u8 = 15;
pub const GREY: u8 = 7;
pub const CYAN: u8 = 11;
pub const RED: u8 = 12;
pub const DARK_RED: u8 = 4;
pub const BLACK: u8 = 0;
/// The small-caps font used for every label on the screen.
pub const SMALL: usize = 1;

/// `fill_background` (0x13a13): tiles the 16x16 texture at SP299 (208, 100) over `r`, anchored
/// at r's corner.
pub fn fill_stipple(fb: &mut Surface, ctx: &UiContext, r: Rect) {
    crate::ui::widgets::tile_fill(fb, ctx, r, Rect::new(208, 100, 16, 16));
}

/// `draw_panel(x1, y1, x2, y2)` (0x139d1): the texture over the rectangle, then a colour 1
/// outline.
pub fn panel(fb: &mut Surface, ctx: &UiContext, r: Rect) {
    fill_stipple(fb, ctx, r);
    fb.rect_outline(r, BORDER);
}

/// Small-caps text; returns the x after the last glyph.
pub fn small_text(fb: &mut Surface, ctx: &UiContext, x: i32, y: i32, colour: u8, s: &str) -> i32 {
    draw_text(fb, &ctx.fonts, &TextStyle::new(SMALL, colour), x, y, s) - GLYPH_SPACING
}

pub fn small_text_width(ctx: &UiContext, s: &str) -> i32 {
    ink_width(&ctx.fonts, SMALL, s)
}

/// `draw_text_centred` (0x18d3) in the small font: centred on the full advance width.
pub fn small_text_centered(fb: &mut Surface, ctx: &UiContext, cx: i32, y: i32, colour: u8, s: &str) -> i32 {
    let w = crate::gfx::text::text_width(&ctx.fonts, SMALL, s);
    small_text(fb, ctx, cx - w / 2, y, colour, s)
}

/// Solid header band with a white small-caps title (text 2 px below the band top, 6 px in).
pub fn header(fb: &mut Surface, ctx: &UiContext, r: Rect, text: &str) {
    fb.fill_rect(r, BORDER);
    small_text(fb, ctx, r.x + 6, r.y + 2, WHITE, text);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Colour 9: light face, grey top/left and dark right/bottom edges (CHANGE, BUY, RENAME,
    /// unselected tabs).
    Plain,
    /// White face (the selected tab).
    Selected,
    /// Red face with dark red edges (EXIT).
    Exit,
}

pub fn button(fb: &mut Surface, ctx: &UiContext, r: Rect, label: &str, style: ButtonStyle) {
    if style == ButtonStyle::Selected {
        fb.fill_rect(r, WHITE);
        fb.hline(r.x, r.right() - 1, r.y, GREY);
        fb.vline(r.x, r.y, r.bottom() - 1, GREY);
        small_text_centered(fb, ctx, (2 * r.x + r.w - 1) / 2 + 1, r.y + 2, BORDER, label);
        return;
    }
    // draw_button(x1, y1, x2, y2, col) (0x13ac5), r covering x1..=x2, y1..=y2: fill up to
    // x2-1/y2-1, top and left lines in col+8 (7 from col 8 up), bottom and right in 8
    // (col-8), the label in col^8 centred on (x1 + x2) / 2 + 1
    let col = if style == ButtonStyle::Exit { RED } else { LIGHT };
    let (light, dark) = if col < 8 { (col + 8, 8) } else { (GREY, col - 8) };
    let (x2, y2) = (r.right() - 1, r.bottom() - 1);
    fb.fill_rect(Rect::new(r.x, r.y, r.w - 1, r.h - 1), col);
    fb.hline(r.x, x2, r.y, light);
    fb.vline(r.x, r.y, y2, light);
    fb.hline(r.x + 1, x2, y2, dark);
    fb.vline(x2, r.y, y2, dark);
    small_text_centered(fb, ctx, (r.x + x2) / 2 + 1, (r.y + y2) / 2 - 2, col ^ 8, label);
}

/// 8x8 resource icons in SP257 (top-left of the cell; colours 0 and 3 are transparent).
pub const ICON_FOOD: (i32, i32) = (128, 32);
pub const ICON_SHIELD: (i32, i32) = (136, 32);
pub const ICON_TRADE: (i32, i32) = (144, 32);
pub const ICON_LUX: (i32, i32) = (144, 40);
pub const ICON_TAX: (i32, i32) = (152, 32);
pub const ICON_SCI: (i32, i32) = (128, 40);
pub const ICON_UNHAPPY: (i32, i32) = (136, 40);

/// Blit an 8x8 icon. The black outline (colour 5) can be recoloured for dark backgrounds.
pub fn icon(fb: &mut Surface, ctx: &UiContext, src: (i32, i32), x: i32, y: i32, outline: Option<u8>) {
    let sheet = &ctx.sp.surf;
    for yy in 0..8 {
        for xx in 0..8 {
            let v = sheet.get(src.0 + xx, src.1 + yy);
            if v == 0 || v == 3 {
                continue;
            }
            let v = if v == 5 { outline.unwrap_or(5) } else { v };
            fb.put(x + xx, y + yy, v);
        }
    }
}

/// A row of `n` icons at 8 px pitch, with a 4 px gap after the first `split` icons.
/// The pitch shrinks when the row would not fit in `max_w` pixels.
pub fn icon_row(fb: &mut Surface, ctx: &UiContext, src: (i32, i32), x: i32, y: i32, n: i32, split: i32, max_w: i32) {
    if n <= 0 {
        return;
    }
    let gap = if split > 0 && split < n { 4 } else { 0 };
    let pitch = if n * 8 + gap <= max_w { 8 } else { ((max_w - gap) / n).max(1) };
    for i in 0..n {
        let g = if i >= split { gap } else { 0 };
        icon(fb, ctx, src, x + i * pitch + g, y, None);
    }
}

/// Citizen figure index in the SP257 row at y=128 (8x16 each).
pub fn citizen_index(happy: bool, content: bool, unhappy: bool, pos: usize) -> u8 {
    let female = (pos % 2) as u8;
    if happy {
        female
    } else if content {
        2 + female
    } else if unhappy {
        4 + female
    } else {
        2 + female
    }
}

pub fn specialist_index(s: Specialist) -> u8 {
    match s {
        Specialist::Taxman => 6,
        Specialist::Scientist => 7,
        Specialist::Entertainer => 8,
    }
}

pub fn citizen(fb: &mut Surface, ctx: &UiContext, idx: u8, x: i32, y: i32) {
    fb.blit_part(&ctx.sp.surf, sp::citizen(idx), x, y, Some(0));
}

/// Replace colour `from` by `to` inside `r` (MGRAPHIC entry 3, `recolour(page, x, y, w, h,
/// from, to)` via CIV.EXE 0:0x784).
pub fn recolour(fb: &mut Surface, r: Rect, from: u8, to: u8) {
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            if fb.get(x, y) == from {
                fb.put(x, y, to);
            }
        }
    }
}

/// Colour map used for unit sprites on the city screen: civ colours for the box, the white
/// frame line of the cell turns cyan (as in the original), the guide colour takes the dark shade.
pub fn unit_remap(slot: Slot) -> [u8; 256] {
    let mut m = civ_remap(slot);
    let c = SLOT_COLOURS[(slot & 7) as usize];
    m[15] = CYAN;
    m[3] = c.dark;
    m
}

/// The 15x15 unit box (cyan left/bottom, dark top/right, civ-coloured face) with the unit
/// sprite, as drawn in the INFO view and the production frame. `(x,y)` is the box corner.
pub fn unit_box(fb: &mut Surface, ctx: &UiContext, kind: u8, owner: Slot, x: i32, y: i32) {
    let map = unit_remap(owner);
    blit_sprite_remap(fb, &ctx.sp.surf, sp::unit(kind), x - 1, y - 1, &map);
}

/// Empty 15x15 frame in the unit-box style (for improvement icons in the production frame).
pub fn item_frame(fb: &mut Surface, x: i32, y: i32, owner: Slot) {
    let c = SLOT_COLOURS[(owner & 7) as usize];
    fb.fill_rect(Rect::new(x, y, 15, 15), c.light);
    fb.hline(x + 1, x + 14, y, c.dark);
    fb.vline(x + 14, y + 1, y + 13, c.dark);
    fb.vline(x, y, y + 14, CYAN);
    fb.hline(x, x + 14, y + 14, CYAN);
}

/// SP299 holds the improvement / wonder pictograms, 18x10 cells 19 px apart, five to a column
/// (CIV.EXE 0xc91:0x703b); `cell` 40 and 41 sit side by side below the grid.
pub fn imp_icon(fb: &mut Surface, ctx: &UiContext, cell: usize, x: i32, y: i32) {
    let pic = ctx.pic("SP299");
    let cell = cell as i32;
    let src = if cell < 40 { Rect::new(161 + 19 * (cell / 5), 50 + 10 * (cell % 5), 18, 10) } else { Rect::new(161 + 19 * (cell & 1), 100, 18, 10) };
    fb.blit_part(&pic.surf, src, x, y, None);
}

/// Pictogram cell for an improvement id; the spaceship parts have none.
pub fn improvement_cell(id: u8) -> Option<usize> {
    (1..=21).contains(&id).then(|| id as usize - 1)
}

/// Pictogram cell for a wonder id (after the 21 improvements).
pub fn wonder_cell(id: u8) -> usize {
    id as usize + 20
}

/// Cut `s` down to `max` pixels of small text the way CIV.EXE 0x1f1ad does: drop the last
/// character and end the rest with a '.' unless that would follow a space.
pub fn fit_small_text(ctx: &UiContext, s: &str, max: i32) -> String {
    let mut b: Vec<u8> = s.bytes().collect();
    while b.len() > 1 && small_text_width(ctx, std::str::from_utf8(&b).unwrap_or("")) > max {
        let n = b.len();
        if n >= 3 && b[n - 3] != b' ' {
            b[n - 2] = b'.';
        }
        b.pop();
    }
    String::from_utf8(b).unwrap_or_default()
}

/// The red "sell" dot: 7x7, black outline, white ring, red centre.
pub fn sell_dot(fb: &mut Surface, x: i32, y: i32) {
    const DOT: [&str; 7] = ["..###..", ".#WWW#.", "#WRRRW#", "#WRRRW#", "#WRRRW#", ".#WWW#.", "..###.."];
    for (yy, row) in DOT.iter().enumerate() {
        for (xx, ch) in row.chars().enumerate() {
            let c = match ch {
                '#' => BLACK,
                'W' => WHITE,
                'R' => RED,
                _ => continue,
            };
            fb.put(x + xx as i32, y + yy as i32, c);
        }
    }
}
