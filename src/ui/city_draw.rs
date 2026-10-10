//! Drawing primitives of the city screen: the blue stipple panels, buttons, header bands,
//! the 8x8 resource icons, citizen figures, unit boxes and improvement icons, all measured
//! from captures of the original (SP257 palette indices).

use crate::game::{Slot, Specialist};
use crate::gfx::text::{draw_text, text_width};
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

/// 16x16 panel stipple of the original (bit set = light), tiled from the screen origin.
const STIPPLE: [u16; 16] = [
    0x8b3a, 0x595d, 0xe342, 0xbc5b, 0xe984, 0x93da, 0xcb63, 0x88ef, 0xb648, 0xb258, 0x8995, 0xd11a, 0xac26, 0xe84a, 0xa52d, 0x75df,
];

pub fn fill_stipple(fb: &mut Surface, r: Rect) {
    for y in r.y..r.bottom() {
        let row = STIPPLE[(y & 15) as usize];
        for x in r.x..r.right() {
            let light = row & (0x8000 >> (x & 15)) != 0;
            fb.put(x, y, if light { LIGHT } else { DARK });
        }
    }
}

/// A bordered stipple panel.
pub fn panel(fb: &mut Surface, r: Rect) {
    fill_stipple(fb, r.inset(1));
    fb.rect_outline(r, BORDER);
}

/// Small-caps text as the original spaces it: one extra pixel between letters.
pub fn small_text(fb: &mut Surface, ctx: &UiContext, x: i32, y: i32, colour: u8, s: &str) -> i32 {
    let st = TextStyle::new(SMALL, colour);
    let mut cx = x;
    let mut buf = [0u8; 4];
    for ch in s.chars() {
        cx = draw_text(fb, &ctx.fonts, &st, cx, y, ch.encode_utf8(&mut buf)) + 1;
    }
    cx - 1
}

pub fn small_text_width(ctx: &UiContext, s: &str) -> i32 {
    let mut buf = [0u8; 4];
    let n = s.chars().count() as i32;
    s.chars().map(|ch| text_width(&ctx.fonts, SMALL, ch.encode_utf8(&mut buf))).sum::<i32>() + (n - 1).max(0)
}

pub fn small_text_centered(fb: &mut Surface, ctx: &UiContext, cx: i32, y: i32, colour: u8, s: &str) -> i32 {
    let w = small_text_width(ctx, s);
    small_text(fb, ctx, cx - w / 2, y, colour, s)
}

/// Solid header band with a white small-caps title (text 2 px below the band top, 6 px in).
pub fn header(fb: &mut Surface, ctx: &UiContext, r: Rect, text: &str) {
    fb.fill_rect(r, BORDER);
    small_text(fb, ctx, r.x + 6, r.y + 2, WHITE, text);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Grey top/left line, light face (CHANGE, BUY, unselected tabs).
    Plain,
    /// White face (the selected tab).
    Selected,
    /// Light face with a dark right/bottom edge (RENAME).
    Raised,
    /// Red face with dark red edges (EXIT).
    Exit,
}

pub fn button(fb: &mut Surface, ctx: &UiContext, r: Rect, label: &str, style: ButtonStyle) {
    let (face, text) = match style {
        ButtonStyle::Plain | ButtonStyle::Raised => (LIGHT, BORDER),
        ButtonStyle::Selected => (WHITE, BORDER),
        ButtonStyle::Exit => (RED, DARK_RED),
    };
    fb.fill_rect(r, face);
    fb.hline(r.x, r.right() - 1, r.y, GREY);
    fb.vline(r.x, r.y, r.bottom() - 1, GREY);
    match style {
        ButtonStyle::Raised => {
            fb.vline(r.right() - 1, r.y + 1, r.bottom() - 1, BORDER);
            fb.hline(r.x + 1, r.right() - 1, r.bottom() - 1, BORDER);
        }
        ButtonStyle::Exit => {
            fb.vline(r.right() - 1, r.y + 1, r.bottom() - 1, DARK_RED);
            fb.hline(r.x + 1, r.right() - 1, r.bottom() - 1, DARK_RED);
        }
        _ => {}
    }
    small_text_centered(fb, ctx, r.x + r.w / 2 + 1, r.y + 2, text, label);
}

/// 8x8 resource icons in SP257 (top-left of the cell; colours 0 and 3 are transparent).
pub const ICON_FOOD: (i32, i32) = (128, 32);
pub const ICON_SHIELD: (i32, i32) = (136, 32);
pub const ICON_TRADE: (i32, i32) = (144, 32);
pub const ICON_LUX: (i32, i32) = (128, 40);
pub const ICON_TAX: (i32, i32) = (152, 32);
pub const ICON_SCI: (i32, i32) = (144, 40);

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

/// SP299 holds the 18x8 improvement / wonder pictograms on a 19x10 grid (8 per row).
pub fn imp_icon(fb: &mut Surface, ctx: &UiContext, cell: usize, x: i32, y: i32) {
    let pic = ctx.pic("SP299");
    let src = Rect::new(161 + 19 * (cell % 8) as i32, 52 + 10 * (cell / 8) as i32, 18, 8);
    fb.blit_part(&pic.surf, src, x, y, None);
}

/// Pictogram cell for an improvement id (best visual match of the SP299 sheet).
pub fn improvement_cell(id: u8) -> usize {
    use crate::rules::improvement_ids as imp;
    match id {
        imp::PALACE => 0,
        imp::BARRACKS => 1,
        imp::GRANARY => 2,
        imp::TEMPLE => 5,
        imp::MARKETPLACE => 8,
        imp::LIBRARY => 9,
        imp::COURTHOUSE => 19,
        imp::CITY_WALLS => 17,
        imp::AQUEDUCT => 25,
        imp::BANK => 33,
        imp::CATHEDRAL => 10,
        imp::UNIVERSITY => 18,
        imp::MASS_TRANSIT => 35,
        imp::COLOSSEUM => 16,
        imp::FACTORY => 26,
        imp::MFG_PLANT => 34,
        imp::SDI_DEFENSE => 39,
        imp::RECYCLING_CNTR => 11,
        imp::POWER_PLANT => 3,
        imp::HYDRO_PLANT => 7,
        imp::NUCLEAR_PLANT => 27,
        imp::SS_STRUCTURAL => 39,
        imp::SS_COMPONENT => 36,
        imp::SS_MODULE => 40,
        _ => 0,
    }
}

/// Pictogram cell for a wonder id.
pub fn wonder_cell(id: u8) -> usize {
    use crate::rules::wonder_ids as w;
    match id {
        w::PYRAMIDS => 12,
        w::HANGING_GARDENS => 20,
        w::COLOSSUS => 28,
        w::LIGHTHOUSE => 13,
        w::GREAT_LIBRARY => 24,
        w::ORACLE => 22,
        w::GREAT_WALL => 21,
        w::MAGELLANS_EXPEDITION => 29,
        w::MICHELANGELOS_CHAPEL => 37,
        w::SHAKESPEARES_THEATRE => 14,
        w::COPERNICUS_OBSERVATORY => 6,
        w::SETI_PROGRAM => 31,
        w::ISAAC_NEWTONS_COLLEGE => 23,
        w::JS_BACHS_CATHEDRAL => 30,
        w::DARWINS_VOYAGE => 38,
        w::WOMENS_SUFFRAGE => 15,
        w::HOOVER_DAM => 7,
        w::MANHATTAN_PROJECT => 40,
        w::UNITED_NATIONS => 32,
        w::APOLLO_PROGRAM => 36,
        w::CURE_FOR_CANCER => 41,
        _ => 12,
    }
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
