//! The city view (CIV.EXE overlay 0x19174): the HILL meadow with the city laid out on a
//! staggered 19x12 grid. Houses from CITYPIX1 grow around the centre in a random walk seeded
//! with the city name, two straight and two diagonal roads cross it, improvements come from
//! CITYPIX2/CITYPIX3 and wonders from WONDERS/WONDERS2; walls, mass transit, power lines,
//! the aqueduct and the hydro plant have fixed places. The city name and year, or a message
//! box such as "Rome builds / Granary.", go on top.
//!
//! Shown from the VIEW button (with the citizens of POP.PIC along the bottom), after the
//! founding animation, and when a city completes an improvement: then the view first appears
//! without the new building, and after a second it dissolves into the view with it while the
//! build-complete tune plays.

use crate::audio::Sound;
use crate::game::{CityId, Game, Specialist};
use crate::gfx::text::{draw_text, draw_text_centered, text_width};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::Event;
use crate::rules::{advance_ids, improvement_ids as imp, wonder_ids as won};
use crate::ui::context::UiContext;
use crate::ui::reveal::Reveal;
use crate::ui::screen::{DialogResult, Screen, Transition};

/// Grid size: 19 columns, 12 rows; odd rows sit 8 px further right.
const COLS: usize = 19;
const ROWS: usize = 12;
const EMPTY: u8 = 0xff;
const ROAD: u8 = 0xfe;
/// Cells covered by a building or wonder drawn from a neighbouring cell.
const FILLER: u8 = 0x0f;
/// Driver duration of the dissolve that adds a new building (overlay call site 0x19fb1).
const DISSOLVE: u32 = 10;
/// Ticks the view waits before adding the new building.
const REVEAL_DELAY: u32 = 60;

/// Direction offsets 1..8 (ds:0x1882 / ds:0x18e4).
const DX: [i32; 9] = [0, 0, 1, 1, 1, 0, -1, -1, -1];
const DY: [i32; 9] = [0, -1, -1, 0, 1, 1, 1, 0, -1];

/// Sprite rectangles (x0, y0, x1, y1) of the 21 wonders in WONDERS.PIC, or WONDERS2.PIC for
/// the four drawn at fixed places (CIV.EXE far data 0x26e6:0000).
const WONDER_RECTS: [(i32, i32, i32, i32); 22] = [
    (0, 0, 0, 0),
    (131, 54, 318, 82),
    (159, 149, 227, 198),
    (88, 97, 211, 135),
    (229, 116, 268, 198),
    (61, 117, 101, 172),
    (164, 97, 227, 147),
    (1, 38, 66, 118),
    (268, 53, 318, 114),
    (9, 117, 59, 172),
    (126, 1, 182, 53),
    (276, 1, 318, 51),
    (103, 139, 157, 198),
    (184, 1, 224, 69),
    (40, 69, 101, 115),
    (1, 14, 147, 33),
    (253, 1, 274, 47),
    (226, 1, 251, 47),
    (103, 79, 162, 137),
    (270, 116, 318, 198),
    (63, 1, 124, 54),
    (240, 60, 266, 114),
];

/// Wonders with a fixed place in the view (mask 0x808a): Pyramids, Colossus, Great Wall,
/// Hoover Dam, in the order they are drawn.
const FIXED_WONDERS: [u8; 4] = [won::PYRAMIDS, won::HOOVER_DAM, won::COLOSSUS, won::GREAT_WALL];

/// Sheet and rectangle of a wonder's picture: WONDERS2 for the four with a fixed place.
pub fn wonder_sprite(w: u8) -> (&'static str, Rect) {
    let (x0, y0, x1, y1) = WONDER_RECTS[w.clamp(1, 21) as usize];
    (if fixed_wonder(w) { "WONDERS2" } else { "WONDERS" }, Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

fn fixed_wonder(w: u8) -> bool {
    FIXED_WONDERS.contains(&w)
}

/// The C library generator the original seeds with the sum of the city name's characters.
struct MsRand(u32);

impl MsRand {
    fn rand(&mut self) -> i32 {
        self.0 = self.0.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        ((self.0 >> 16) & 0x7fff) as i32
    }
    /// 0..n-1 (`rand() * n >> 15`).
    fn random(&mut self, n: i32) -> i32 {
        (self.rand() * n) >> 15
    }
}

fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    let v = if v < lo { lo } else { v };
    if v > hi {
        hi
    } else {
        v
    }
}

/// The game's distance: the longer leg plus half the shorter one.
fn distance(dx: i32, dy: i32) -> i32 {
    let (a, b) = (dx.abs(), dy.abs());
    if a > b {
        a + b / 2
    } else {
        b + a / 2
    }
}

/// The city laid out on the grid: house types (0..9), roads, improvements (15 + bit of the
/// improvement), wonders (0x40 + wonder) and fillers.
struct Layout {
    cells: [u8; COLS * ROWS],
}

impl Layout {
    fn get(&self, x: i32, y: i32) -> u8 {
        // the original indexes its x*12+y array without bounds: a row off the end wraps
        // into the next column, anything beyond the array counts as empty
        let i = x * ROWS as i32 + y;
        if (0..(COLS * ROWS) as i32).contains(&i) {
            self.cells[i as usize]
        } else {
            EMPTY
        }
    }
    fn set(&mut self, x: i32, y: i32, v: u8) {
        let i = x * ROWS as i32 + y;
        if (0..(COLS * ROWS) as i32).contains(&i) {
            self.cells[i as usize] = v;
        }
    }
    /// Signed view of a cell, as the original compares them.
    fn s(&self, x: i32, y: i32) -> i8 {
        self.get(x, y) as i8
    }

    /// Neighbour in direction `d` on the staggered grid.
    fn neighbour(x: i32, y: i32, d: usize) -> (i32, i32) {
        let adj = if y & 1 == 0 {
            if DY[d] > 0 {
                -1
            } else {
                0
            }
        } else if DY[d] < 0 {
            1
        } else {
            0
        };
        (x + DX[d] + adj, y + DY[d])
    }

    fn build(g: &Game, city: CityId) -> Layout {
        let c = g.city(city);
        let owner = g.player(c.owner);
        let tech = owner.advance_count() as i32 / 2;
        let size = c.size as i32;
        let seed: u32 = c.name.bytes().map(|b| b as i8 as i32).sum::<i32>() as u16 as u32;
        let mut rng = MsRand(seed);
        let mut l = Layout { cells: [EMPTY; COLS * ROWS] };
        // houses: a random walk from the centre that restarts when it strays too far
        let (mut x, mut y) = (9, 11);
        let (mut count, mut attempts) = (0, 0);
        while count < size * 8 {
            if l.get(x, y) != EMPTY {
                attempts += 1;
                if attempts < 999 {
                    count -= 1;
                }
            } else {
                let dist = distance(x - 9, y - 10);
                let hi = size / 4 + 6;
                let lo = tech / 6;
                let v = clamp((tech - dist) / 3 - rng.random(2), lo, hi);
                l.set(x, y, v.min(9) as u8);
                if dist * dist > size * 8 {
                    x = 9;
                    y = 11;
                }
            }
            let d = (rng.random(8) + 1) as usize;
            x = clamp(x + DX[d], 0, 18);
            y = clamp(y + DY[d], 0, 11);
            count += 1;
        }
        // two streets along the rows and two avenues along the diagonals
        let mut street = 0;
        for base in [4, 8] {
            street = rng.random(2) + base;
            for k in 2..17 {
                l.set(k, street, ROAD);
            }
            l.set(18, street, EMPTY);
            l.set(0, street, EMPTY);
        }
        for base in [9, 14] {
            let r = rng.random(4) + base;
            for k in 1..12 {
                l.set(r - (k + 1) / 2, k, ROAD);
            }
        }
        let great_wall = g.wonders_built[won::GREAT_WALL as usize] == Some(city);
        if great_wall {
            for (x, y) in [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (0, 2)] {
                l.set(x, y, FILLER);
            }
        }
        // wonders replace houses (after 400 tries any free place will do)
        for k in 1..=21u8 {
            if g.wonders_built[k as usize] != Some(city) || fixed_wonder(k) {
                continue;
            }
            let mut attempts = 0;
            let (mut x, mut y);
            loop {
                x = rng.random(15) + 1;
                y = rng.random(10) + 1;
                let mut flag = 0;
                if [(0, 0), (1, 0), (2, 0), (-1, 1), (0, 1), (1, 1)].iter().any(|(dx, dy)| l.s(x + dx, y + dy) >= 0) {
                    flag |= 1;
                }
                if attempts > 400 {
                    flag |= 1;
                }
                if [(0, 0), (1, 0), (2, 0), (-1, 1), (0, 1), (1, 1), (2, 1)].iter().any(|(dx, dy)| l.s(x + dx, y + dy) >= FILLER as i8) {
                    flag |= 2;
                }
                if [(0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)].iter().any(|(dx, dy)| l.get(x + dx, y + dy) == ROAD) {
                    flag |= 2;
                }
                if flag == 1 {
                    break;
                }
                attempts += 1;
                if attempts >= 500 {
                    break;
                }
            }
            if attempts < 500 {
                l.set(x, y, 0x40 + k);
                for (dx, dy) in [(1, 0), (2, 0), (0, 1), (1, 1), (2, 1), (-1, 1), (1, -1), (2, -1), (3, -1)] {
                    l.set(x + dx, y + dy, FILLER);
                }
                l.clear_below(x, y);
            }
        }
        // improvements, most expensive first; walls, aqueduct, mass transit, power and
        // hydro plants have places of their own
        for bit in (1..=23u8).rev() {
            let id = bit + 1;
            if !c.has_improvement(id) || matches!(id, imp::CITY_WALLS | imp::AQUEDUCT | imp::MASS_TRANSIT | imp::POWER_PLANT | imp::HYDRO_PLANT) {
                continue;
            }
            let mut attempts = 0;
            let (mut x, mut y);
            let spots = [(0, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
            loop {
                x = rng.random(16) + 1;
                y = rng.random(10) + 1;
                let mut flag = 0;
                if spots.iter().any(|(dx, dy)| l.s(x + dx, y + dy) >= 0) {
                    flag |= 1;
                }
                if attempts > 350 {
                    flag |= 1;
                }
                if spots.iter().any(|(dx, dy)| l.s(x + dx, y + dy) >= FILLER as i8) {
                    flag |= 2;
                }
                if spots.iter().any(|(dx, dy)| l.get(x + dx, y + dy) == ROAD) {
                    flag |= 2;
                }
                if flag == 1 {
                    break;
                }
                attempts += 1;
                if attempts >= 500 {
                    break;
                }
            }
            if attempts < 500 {
                l.set(x, y, bit + FILLER);
                for (dx, dy) in [(1, 0), (0, 1), (1, 1), (-1, 1)] {
                    l.set(x + dx, y + dy, FILLER);
                }
                l.clear_below(x, y);
            }
        }
        if c.has_improvement(imp::HYDRO_PLANT) {
            l.set(18, 8, FILLER);
            l.set(18, 7, FILLER);
            l.set(17, 8, FILLER);
            l.set(17, 7, imp::HYDRO_PLANT - 1 + FILLER);
        }
        if c.has_improvement(imp::AQUEDUCT) {
            l.set(0, street - 1, imp::AQUEDUCT - 1 + FILLER);
            for (x, y) in [(1, street - 1), (2, street - 1), (0, street), (1, street), (2, street)] {
                l.set(x, y, FILLER);
            }
        }
        // roads only run past something
        for y in 0..ROWS as i32 {
            for x in 0..COLS as i32 {
                if l.get(x, y) == ROAD && !(1..=8).any(|d| {
                    let (nx, ny) = Layout::neighbour(x, y, d);
                    l.s(nx, ny) >= 0
                }) {
                    l.set(x, y, EMPTY);
                }
            }
        }
        l
    }

    /// The house two rows below a new building would hide it: the original clears houses of
    /// types 0 and 8 there (any cell whose low three bits are zero).
    fn clear_below(&mut self, x: i32, y: i32) {
        if y < 7 && self.get(x + 1, y + 2) & 7 == 0 {
            self.set(x + 1, y + 2, EMPTY);
        }
    }
}

/// An item that is drawn in the view: improvement ids 1..24, wonders 25..45 (24 + wonder).
pub type ViewItem = u8;

pub fn improvement_item(id: u8) -> ViewItem {
    id
}

pub fn wonder_item(w: u8) -> ViewItem {
    24 + w
}

/// Draw the view of `city` without the item `skip`; `message` replaces the name and year.
pub fn render(fb: &mut Surface, ctx: &UiContext, city: CityId, skip: Option<ViewItem>, message: Option<&[String]>) {
    let hill = ctx.pic("HILL");
    fb.blit(&hill.surf, 0, 0);
    let g = ctx.game();
    let Some(c) = g.city_opt(city) else { return };
    let layout = Layout::build(g, city);
    let owner = g.player(c.owner);
    let houses = ctx.pic("CITYPIX1");
    let buildings = ctx.pic(if owner.knows(advance_ids::COMBUSTION) { "CITYPIX3" } else { "CITYPIX2" });
    let wonders = ctx.pic("WONDERS");
    let wonders2 = ctx.pic("WONDERS2");
    let skipped = |item: ViewItem| skip == Some(item);
    let has = |id: u8| c.has_improvement(id) && !skipped(improvement_item(id));
    let building = |idx: i32| Rect::new(50 * (idx / 4) + 1, 50 * (idx % 4) + 1, 49, 49);
    let road_y = if owner.knows(advance_ids::AUTOMOBILE) { 81 } else { 65 };
    let road = |mask: i32| Rect::new((mask & 7) * 24, road_y + (mask >> 3) * 8, 24, 8);
    // wonders with a fixed place
    for w in FIXED_WONDERS {
        if g.wonders_built[w as usize] != Some(city) || skipped(wonder_item(w)) {
            continue;
        }
        let (x0, y0, x1, y1) = WONDER_RECTS[w as usize];
        let x = x0 + if x0 < 10 { -1 } else { 2 } + if w == won::COLOSSUS { 80 } else { 0 };
        let y = if w <= 7 { 0 } else { y0 - 6 };
        fb.blit_part(&wonders2.surf, Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1), x, y, Some(0));
    }
    // power lines along the horizon
    if has(imp::POWER_PLANT) {
        let mut x = if g.wonders_built[won::GREAT_WALL as usize] == Some(city) { 45 } else { 0 };
        while x < 310 {
            fb.blit_part(&buildings.surf, building(17), x, 2, Some(0));
            x += 45;
        }
    }
    for y in 0..ROWS as i32 {
        for x in 0..COLS as i32 {
            let v = layout.get(x, y);
            let sx = x * 16 + (y & 1) * 8;
            if v == EMPTY {
                // a bare patch of ground next to the built-up area
                if x > 2 && layout.get(x - 1, y) != EMPTY && (x + y) & 1 == 1 {
                    fb.blit_part(&houses.surf, road(0), sx - 5, y * 8 + 50, Some(0));
                }
                continue;
            }
            if v == FILLER {
                continue;
            }
            if v == ROAD {
                let mut mask = 0;
                for d in [1, 3, 5, 7] {
                    let (nx, ny) = Layout::neighbour(x, y, d);
                    mask >>= 1;
                    if layout.get(nx, ny) == ROAD {
                        mask |= 8;
                    }
                }
                fb.blit_part(&houses.surf, road(mask), sx - 5, y * 8 + 50, Some(0));
            } else if v < 16 {
                let t = v as i32 / 2;
                let variant = if v < 2 { (v as i32 & 1) * 2 + (city as i32 & 1) } else { ((x + y) & 1) + (v as i32 & 1) * 2 };
                let src = Rect::new(64 * t + 1 + (variant & 1) * 32, 1 + (variant >> 1) * 32, 31, 31);
                fb.blit_part(&houses.surf, src, sx, y * 8 + 26, Some(0));
            } else if v < 0x40 {
                if skipped(v - FILLER + 1) {
                    continue;
                }
                fb.blit_part(&buildings.surf, building(v as i32 - 16), (sx - 9).clamp(0, 999), y * 8 + 16, Some(0));
            } else {
                let w = v - 0x40;
                if skipped(wonder_item(w)) {
                    continue;
                }
                let (x0, y0, x1, y1) = WONDER_RECTS[w as usize];
                let h = y1 - y0 + 1;
                fb.blit_part(&wonders.surf, Rect::new(x0, y0, x1 - x0 + 1, h), (sx - 9).clamp(0, 999), y * 8 - h + 66, Some(0));
            }
        }
    }
    if has(imp::CITY_WALLS) {
        let mut x = 0;
        while x < 320 {
            if x == 172 {
                x += 19;
            }
            fb.blit_part(&buildings.surf, building(22), x, 108, Some(0));
            x += 43;
        }
        fb.blit_part(&buildings.surf, building(6), 142, 108, Some(0));
    }
    if has(imp::MASS_TRANSIT) {
        let mut x = 0;
        while x < 310 {
            fb.blit_part(&buildings.surf, building(21), x, 115, Some(0));
            x += 46;
        }
        fb.blit_part(&buildings.surf, building(11), 0, 115, Some(0));
    }
    let pal = ctx.pic_pal("HILL");
    let white = pal.nearest([255, 255, 255]);
    let black = pal.nearest([0, 0, 0]);
    let st = TextStyle::shadowed(5, white, black);
    match message {
        Some(lines) => message_box(fb, ctx, 80, 8, lines),
        None => {
            draw_text_centered(fb, &ctx.fonts, &st, 160, 2, &c.name);
            draw_text_centered(fb, &ctx.fonts, &st, 160, 15, &g.year_text());
        }
    }
}

/// The grey stone box with a black frame the original puts messages in (0x1c83:0x1e).
pub fn message_box(fb: &mut Surface, ctx: &UiContext, x: i32, y: i32, lines: &[String]) {
    let pal = ctx.pic_pal("HILL");
    let white = pal.nearest([255, 255, 255]);
    let black = pal.nearest([0, 0, 0]);
    let dark = pal.nearest([109, 109, 109]);
    // the stone pattern of the map screen in the two greys of the HILL palette
    let light_stone = pal.nearest([198, 198, 198]);
    let dark_stone = pal.nearest([158, 158, 158]);
    let pat = crate::ui::tilerender::cell(crate::ui::tilerender::sp::PATTERN_STONE.0, crate::ui::tilerender::sp::PATTERN_STONE.1);
    let lighter = ctx.sp.surf.get(pat.x, pat.y).max(ctx.sp.surf.get(pat.x + 1, pat.y));
    let lh = ctx.fonts.get(5).line_height();
    let w = lines.iter().map(|l| text_width(&ctx.fonts, 5, l)).max().unwrap_or(0) + 11;
    let h = lines.len() as i32 * lh + 11;
    let r = Rect::new(x, y, w, h);
    for yy in r.y..r.bottom() {
        for xx in r.x..r.right() {
            let v = ctx.sp.surf.get(pat.x + (xx - r.x) % pat.w, pat.y + (yy - r.y) % pat.h);
            fb.put(xx, yy, if v == lighter { light_stone } else { dark_stone });
        }
    }
    fb.rect_outline(r, black);
    let inner = r.inset(1);
    fb.hline(inner.x, inner.right() - 1, inner.y, dark);
    fb.vline(inner.right() - 1, inner.y, inner.bottom() - 1, dark);
    fb.vline(inner.x, inner.y + 1, inner.bottom() - 1, white);
    fb.hline(inner.x, inner.right() - 2, inner.bottom() - 1, white);
    // the shadow goes straight below the letters here
    for (i, l) in lines.iter().enumerate() {
        let ty = y + 5 + i as i32 * lh;
        draw_text(fb, &ctx.fonts, &TextStyle::new(5, black), x + 5, ty + 1, l);
        draw_text(fb, &ctx.fonts, &TextStyle::new(5, white), x + 5, ty, l);
    }
}

/// The citizens of POP.PIC along the bottom of the plain view (0x1a294): happy, content and
/// unhappy groups, then the specialists, 12 px apart (closer for big cities).
fn draw_people(fb: &mut Surface, ctx: &UiContext, city: CityId) {
    let g = ctx.game();
    let c = g.city(city);
    let pop = ctx.pic("POP");
    let y_src = if g.player(g.human_slot).knows(advance_ids::INDUSTRIALIZATION) { 1 } else { 52 };
    let sprite = |i: i32| Rect::new(35 * i + 1, y_src, 34, 50);
    let size = c.size as i32;
    let step = if size * 12 > 256 { 256 / size } else { 12 };
    let h = g.city_happiness(city);
    let specialists = c.specialists.len() as i32;
    let workers = clamp(size - specialists, 0, 99);
    let (mut happy, mut unhappy) = (clamp(h.happy, 0, size), clamp(h.unhappy, 0, size));
    while happy + unhappy > workers {
        happy = clamp(happy - 1, 0, size);
        unhappy = clamp(unhappy - 1, 0, size);
    }
    let content = size - happy - unhappy - specialists;
    let mut x = 24;
    for (count, base) in [(happy, 0), (content, 2), (unhappy, 4)] {
        for i in 0..count.max(0) {
            fb.blit_part(&pop.surf, sprite(base + (i & 1)), x, 140, Some(0));
            x += step;
        }
        if base < 4 && count > 0 {
            x += 8;
        }
    }
    x += 12;
    for s in &c.specialists {
        let i = match s {
            Specialist::Taxman => 6,
            Specialist::Scientist => 7,
            Specialist::Entertainer => 8,
        };
        fb.blit_part(&pop.surf, sprite(i), x, 140, Some(0));
        x += step;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// VIEW button of the city screen.
    View,
    /// After the founding animation; the city screen follows.
    Founded,
    /// A completed improvement: shown without it, then dissolving into the view with it.
    Built(ViewItem),
}

pub struct CityView {
    city: CityId,
    kind: Kind,
    guard: u32,
    t: u32,
    message: Option<Vec<String>>,
    reveal: Option<Reveal>,
}

pub fn city_view(c: CityId) -> Box<dyn Screen> {
    Box::new(CityView { city: c, kind: Kind::View, guard: 2, t: 0, message: None, reveal: None })
}

/// The view after "X founded"; closing it opens the city screen.
pub fn founded_view(c: CityId) -> Box<dyn Screen> {
    Box::new(CityView { city: c, kind: Kind::Founded, guard: 2, t: 0, message: None, reveal: None })
}

/// "X builds / Y." over the view, which then adds the new improvement.
pub fn built_view(ctx: &UiContext, c: CityId, item: ViewItem) -> Box<dyn Screen> {
    let name = ctx.game().city(c).name.clone();
    let what = if item > 24 { crate::rules::WONDERS[(item - 24) as usize].name } else { crate::rules::IMPROVEMENTS[item as usize].name };
    let message = Some(vec![format!("{name} builds"), format!("{what}.")]);
    Box::new(CityView { city: c, kind: Kind::Built(item), guard: 2, t: 0, message, reveal: None })
}

impl CityView {
    fn close(&mut self, ctx: &mut UiContext) -> Transition {
        if matches!(self.kind, Kind::Built(_)) {
            ctx.audio.play(Sound::FadeOut);
        }
        match self.kind {
            Kind::Founded => Transition::Replace(crate::ui::cityscreen::city_screen(self.city)),
            _ => Transition::Pop(DialogResult::Done),
        }
    }
}

impl Screen for CityView {
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        self.t += 1;
        if matches!(self.kind, Kind::Built(_)) && self.t == REVEAL_DELAY && self.reveal.is_none() {
            ctx.audio.play(Sound::BuildComplete);
            self.reveal = Some(Reveal::new(ctx, DISSOLVE));
        }
        Transition::None
    }

    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => {
                // a key during the wait shows the finished view at once
                if let Kind::Built(_) = self.kind {
                    if self.reveal.is_none() {
                        let mut r = Reveal::new(ctx, DISSOLVE);
                        r.finish();
                        self.reveal = Some(r);
                        return Transition::None;
                    }
                }
                self.close(ctx)
            }
            _ => Transition::None,
        }
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let msg = self.message.as_deref();
        match self.kind {
            Kind::Built(item) => match self.reveal.as_mut() {
                None => render(fb, ctx, self.city, Some(item), msg),
                Some(r) => {
                    let mut content = Surface::screen();
                    render(&mut content, ctx, self.city, None, msg);
                    let pal = ctx.pic_pal("HILL");
                    r.draw(fb, &content, &pal, &pal);
                }
            },
            Kind::View => {
                render(fb, ctx, self.city, None, None);
                if ctx.game().city_opt(self.city).is_some() {
                    draw_people(fb, ctx, self.city);
                }
            }
            Kind::Founded => render(fb, ctx, self.city, None, None),
        }
    }

    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.pic_pal("HILL")
    }

    fn draws_parent(&self) -> bool {
        false
    }

    fn wants_cursor(&self) -> bool {
        false
    }
}
