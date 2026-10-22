//! Civilopedia entry pages: bordered page with picture, title, subtitle, the BLURB history
//! text and a stats page, laid out as in the original (measured on 320x200 captures).

use crate::assets::fonts::FontSet;
use crate::assets::Palette;
use crate::gfx::{Rect, Surface};
use crate::platform::{Event, Key};
use crate::rules::{self, Terraform, Terrain};
use crate::ui::context::UiContext;
use crate::ui::pedia_data::{advance_icon, improvement_picture, terrain_picture, unit_picture, Entry, CONCEPTS};
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::tilerender::sp;

// Palette indices (SP257 palette): the UI colours of the original pages.
const BLACK: u8 = 0;
const BLUE: u8 = 1;
const GREEN: u8 = 2;
const DARK_RED: u8 = 4;
const GREY: u8 = 7;
const LIGHT_BLUE: u8 = 9;
const RED: u8 = 12;
const YELLOW: u8 = 14;
const PAGE: u8 = 15;
const WHITE: u8 = 16;

const FONT_TITLE: usize = 5;
const FONT_TEXT: usize = 6;
/// Every text on the pages is drawn with one extra pixel between glyphs.
const SPACING: i32 = 1;

const BODY_X: i32 = 12;
const BODY_Y: i32 = 76;
const BODY_W: i32 = 297;
/// The game-effect notes wrap a little narrower than the history text (measured).
const NOTE_W: i32 = 290;
const BODY_PITCH: i32 = 9;
const BODY_LINES: usize = 13;
const STAT_PITCH: i32 = 8;

/// Width of `s` in `font` with `spacing` extra pixels per glyph.
pub fn spaced_width(fonts: &FontSet, font: usize, s: &str, spacing: i32) -> i32 {
    let f = fonts.get(font);
    s.chars().map(|c| f.advance_of(c) as i32 + spacing).sum()
}

/// Draw `s` with extra glyph spacing; returns the x after the last glyph.
#[allow(clippy::too_many_arguments)]
pub fn draw_spaced(fb: &mut Surface, fonts: &FontSet, font: usize, colour: u8, x: i32, y: i32, s: &str, spacing: i32) -> i32 {
    let f = fonts.get(font);
    let cell_w = f.cell_width();
    let mut cx = x;
    let n = s.chars().count();
    for (i, ch) in s.chars().enumerate() {
        let c = crate::assets::fonts::char_to_byte(ch);
        match f.glyph(c) {
            Some(g) => {
                // The original blanks the spacing column in front of every glyph, which wipes
                // the pixels a few glyphs ('t', 'u', '-') carry beyond their advance width,
                // except after the last glyph of a string.
                let w = if i + 1 < n { cell_w.min(g.advance as i32) } else { cell_w };
                for (r, bits) in g.rows.iter().enumerate() {
                    if *bits == 0 {
                        continue;
                    }
                    for col in 0..w {
                        if bits & (0x8000 >> col) != 0 {
                            fb.put(cx + col, y + r as i32, colour);
                        }
                    }
                }
                cx += g.advance as i32 + spacing;
            }
            None => cx += f.advance_of('?') as i32 + spacing,
        }
    }
    cx
}

#[allow(clippy::too_many_arguments)]
pub fn draw_spaced_centred(fb: &mut Surface, fonts: &FontSet, font: usize, colour: u8, cx: i32, y: i32, s: &str, spacing: i32) -> i32 {
    let w = spaced_width(fonts, font, s, spacing);
    draw_spaced(fb, fonts, font, colour, cx - w / 2, y, s, spacing)
}

/// Word-wrap with glyph spacing taken into account.
pub fn wrap_spaced(fonts: &FontSet, font: usize, max_w: i32, s: &str, spacing: i32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split(' ').filter(|w| !w.is_empty()) {
        if cur.is_empty() {
            cur = word.to_string();
        } else {
            let candidate = format!("{cur} {word}");
            if spaced_width(fonts, font, &candidate, spacing) <= max_w {
                cur = candidate;
            } else {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
            }
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

// ---------------------------------------------------------------------------------------------
// Borders (8-pixel tiles transcribed from the original captures)
// ---------------------------------------------------------------------------------------------

struct BorderTiles {
    top: [&'static str; 8],
    bottom: [&'static str; 8],
    left: [&'static str; 8],
    right: [&'static str; 8],
    tl: [&'static str; 8],
    tr: [&'static str; 8],
    bl: [&'static str; 8],
    br: [&'static str; 8],
}

/// Red/green/yellow dotted border (advances, improvements, wonders, concepts).
/// G = green, r = red, D = dark red, y = yellow, k = black, . = page.
static DOTS: BorderTiles = BorderTiles {
    top: ["rGrGrGrG", "rrrrrrrr", "GyGGGGGG", "GG.y.yGG", "GGy.y.GG", "GGGGGGyG", "rrrrrrrr", "GrGrGrGr"],
    bottom: ["rGrGrGrG", "rrrrrrrr", "GyGGGGGG", "GG.y.yGG", "GGy.y.GG", "GGGGGGyG", "rrrrrrrr", "GrGrGrGr"],
    left: ["GrGGGGrr", "rrGGGyrG", "GrGy.Grr", "rrG.yGrG", "GrGy.Grr", "rrG.yGrG", "GryGGGrr", "rrGGGGrG"],
    right: ["GrGGGGrr", "rrGGGyrG", "GrGy.Grr", "rrG.yGrG", "GrGy.Grr", "rrG.yGrG", "GryGGGrr", "rrGGGGrG"],
    tl: ["GGGGGGGG", "Grrrrrrr", "GrDrDrDr", "GrrGyk.r", "GrDyGkr.", "Grrkkkkk", "GrDr.kGG", "Grr.rkG."],
    tr: ["GGGGGGGG", "rrrrrrrG", "rDrDrDrG", ".rkyGrrG", "r.kGyDrG", "kkkkkrrG", "GGkr.DrG", ".Gk.rrrG"],
    bl: ["Grrr.kG.", "GrD.rkGG", "Grrkkkkk", "GrDyGk.r", "GrrGykr.", "GrDrDrDr", "Grrrrrrr", "GGGGGGGG"],
    br: [".Gkr.rrG", "GGk.rDrG", "kkkkkrrG", ".rkGyDrG", "r.kyGrrG", "rDrDrDrG", "rrrrrrrG", "GGGGGGGG"],
};

/// Grey meander border (units, terrain). # = grey, . = page.
static MEANDER: BorderTiles = BorderTiles {
    top: ["........", ".#######", ".#.....#", ".#.###.#", ".###.#.#", ".....#.#", "######.#", "........"],
    bottom: ["........", "######.#", ".....#.#", ".###.#.#", ".#.###.#", ".#.....#", ".#######", "........"],
    left: [".######.", "......#.", ".####.#.", ".#..#.#.", ".#.##.#.", ".#.#..#.", ".#.####.", ".#......"],
    right: [".######.", ".#......", ".#.####.", ".#.#..#.", ".#.##.#.", ".#..#.#.", ".####.#.", "......#."],
    tl: ["........", ".#######", ".#.....#", ".#.#####", ".#.#...#", ".#.#...#", ".#.#...#", ".#.#...."],
    tr: ["........", ".######.", ".#....#.", ".####.#.", ".#..#.#.", ".#..#.#.", "##..#.#.", "....#.#."],
    bl: [".###....", ".#.#...#", ".#.#...#", ".#.#...#", ".#.#####", ".#.....#", ".#######", "........"],
    br: ["....###.", "##..#.#.", ".#..#.#.", ".#..#.#.", ".####.#.", ".#....#.", ".######.", "........"],
};

fn border_colour(ch: u8, page: u8) -> u8 {
    match ch {
        b'G' => GREEN,
        b'r' => RED,
        b'D' => DARK_RED,
        b'y' => YELLOW,
        b'k' => BLACK,
        b'#' => GREY,
        _ => page,
    }
}

fn draw_tile(fb: &mut Surface, tile: &[&'static str; 8], x: i32, y: i32, page: u8) {
    for (r, row) in tile.iter().enumerate() {
        for (c, ch) in row.bytes().enumerate() {
            fb.put(x + c as i32, y + r as i32, border_colour(ch, page));
        }
    }
}

/// Draw the 8-pixel page border of the given style over a 320x200 page; the gaps of the
/// pattern are always light grey (15), even on the white pages.
pub fn draw_border(fb: &mut Surface, dotted: bool) {
    let page = PAGE;
    let t = if dotted { &DOTS } else { &MEANDER };
    for x in (8..312).step_by(8) {
        draw_tile(fb, &t.top, x, 0, page);
        draw_tile(fb, &t.bottom, x, 192, page);
    }
    for y in (8..192).step_by(8) {
        draw_tile(fb, &t.left, 0, y, page);
        draw_tile(fb, &t.right, 312, y, page);
    }
    draw_tile(fb, &t.tl, 0, 0, page);
    draw_tile(fb, &t.tr, 312, 0, page);
    draw_tile(fb, &t.bl, 0, 192, page);
    draw_tile(fb, &t.br, 312, 192, page);
}

// ---------------------------------------------------------------------------------------------
// Page content
// ---------------------------------------------------------------------------------------------

enum Item {
    Text { x: i32, y: i32, colour: u8, text: String },
    /// Map sprite of a unit type (16x16 cell) for the "Allows:" list.
    UnitIcon { x: i32, y: i32, kind: u8 },
    /// SP299 pictogram of an improvement or wonder (`city_draw::imp_icon` cell).
    ImpIcon { x: i32, y: i32, cell: usize },
    /// Gold-coin symbol followed by a number ("Maintenance: o3").
    Coin { x: i32, y: i32, colour: u8, text: String },
}

fn text(x: i32, y: i32, colour: u8, s: impl Into<String>) -> Item {
    Item::Text { x, y, colour, text: s.into() }
}

fn advance_name(id: Option<rules::AdvanceId>) -> &'static str {
    id.map(|a| rules::ADVANCES[a as usize].name).unwrap_or("")
}

/// A Civilopedia entry, shown page by page.
pub struct EntryScreen {
    entry: Entry,
    pages: Vec<Vec<Item>>,
    page: usize,
    guard: u32,
}

impl EntryScreen {
    pub fn new(entry: Entry, page: usize) -> EntryScreen {
        EntryScreen { entry, pages: Vec::new(), page, guard: 2 }
    }

    fn wrap_paragraphs(fonts: &FontSet, paras: &[String], max_w: i32) -> Vec<String> {
        let mut lines = Vec::new();
        for p in paras {
            if p.is_empty() {
                lines.push(String::new());
            } else {
                lines.extend(wrap_spaced(fonts, FONT_TEXT, max_w, p, SPACING));
            }
        }
        while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
            lines.pop();
        }
        lines
    }

    fn build(&mut self, ctx: &UiContext) {
        let fonts = &ctx.fonts;
        let mut pages: Vec<Vec<Item>> = Vec::new();
        // history text, BODY_LINES lines per page
        let body = Self::wrap_paragraphs(fonts, &self.entry.blurb(ctx), BODY_W);
        for chunk in body.chunks(BODY_LINES) {
            let mut items = Vec::new();
            for (i, l) in chunk.iter().enumerate() {
                if !l.is_empty() {
                    items.push(text(BODY_X, BODY_Y + i as i32 * BODY_PITCH, BLUE, l.clone()));
                }
            }
            pages.push(items);
        }
        // notes + stats page
        let mut items = Vec::new();
        let note = Self::wrap_paragraphs(fonts, &self.entry.blurb2(ctx), NOTE_W);
        for (i, l) in note.iter().enumerate() {
            if !l.is_empty() {
                items.push(text(BODY_X, BODY_Y + i as i32 * BODY_PITCH, BLUE, l.clone()));
            }
        }
        let stats_y = if note.is_empty() { BODY_Y } else { BODY_Y + note.len() as i32 * BODY_PITCH + 8 };
        let before = items.len();
        self.stats(ctx, &mut items, stats_y);
        if items.len() > before || !note.is_empty() || pages.is_empty() {
            pages.push(items);
        }
        self.pages = pages;
        if self.page >= self.pages.len() {
            self.page = 0;
        }
    }

    fn stats(&self, ctx: &UiContext, items: &mut Vec<Item>, y0: i32) {
        match self.entry {
            Entry::Advance(a) => self.advance_stats(a, items),
            Entry::Improvement(i) => {
                let imp = &rules::IMPROVEMENTS[i as usize];
                let mut y = y0;
                // "Requires " even without an advance (CIV.EXE o 0xbeac)
                items.push(text(BODY_X, y, LIGHT_BLUE, format!("Requires {}", advance_name(imp.prereq))));
                y += STAT_PITCH;
                items.push(text(BODY_X, y, LIGHT_BLUE, format!("Cost: {} shields.", imp.cost as u32 * 10)));
                y += STAT_PITCH;
                let end = spaced_width(&ctx.fonts, FONT_TEXT, "Maintenance: ", SPACING);
                items.push(text(BODY_X, y, RED, "Maintenance: "));
                items.push(Item::Coin { x: BODY_X + end, y, colour: RED, text: imp.upkeep.to_string() });
            }
            Entry::Wonder(w) => {
                let won = &rules::WONDERS[w as usize];
                let mut y = y0;
                items.push(text(BODY_X, y, LIGHT_BLUE, format!("Requires {}", advance_name(won.prereq))));
                y += STAT_PITCH;
                items.push(text(BODY_X, y, LIGHT_BLUE, format!("Cost: {} shields.", won.cost as u32 * 10)));
                y += STAT_PITCH;
                let end = spaced_width(&ctx.fonts, FONT_TEXT, "Maintenance: ", SPACING);
                items.push(text(BODY_X, y, RED, "Maintenance: "));
                items.push(Item::Coin { x: BODY_X + end, y, colour: RED, text: "0".into() });
            }
            Entry::Unit(u) => {
                let ut = &rules::UNITS[u as usize];
                let x = 100;
                let mut y = y0;
                // "Requires " even without an advance; the Nuclear unit also names Nuclear Fission
                // (o 0xbfa2)
                let also = if u == rules::unit_ids::NUCLEAR { " and Nuclear Fission" } else { "" };
                items.push(text(x, y, LIGHT_BLUE, format!("Requires {}{also}", advance_name(ut.prereq))));
                y += STAT_PITCH;
                items.push(text(x, y, LIGHT_BLUE, format!("Cost: {} resources.", ut.cost as u32 * 10)));
                y += STAT_PITCH;
                items.push(text(x, y, RED, format!("Attack Strength: {}", ut.att)));
                y += STAT_PITCH;
                items.push(text(x, y, RED, format!("Defense Strength: {}", ut.def)));
                y += STAT_PITCH;
                items.push(text(x, y, BLACK, format!("Movement Rate: {}", ut.moves)));
            }
            Entry::Terrain(t) => self.terrain_stats(t, items),
            Entry::Concept(c) => {
                // governments: their enabling advance
                let name = CONCEPTS[c].name;
                if let Some(g) = rules::GOVERNMENTS.iter().find(|g| g.name == name) {
                    if g.prereq.is_some() {
                        items.push(text(BODY_X, y0, LIGHT_BLUE, format!("Requires {}", advance_name(g.prereq))));
                    }
                }
            }
        }
    }

    /// The advance's stats page as CIV.EXE o 0xbb2a lays it out: "Requires" (left out
    /// without prerequisites), "Allows:", the advances it leads to 8 px apart, then 4 px
    /// further the units (sprite, red) and improvements / wonders (pictogram, green) 12 px
    /// apart; "Allows:" goes again when nothing follows it.
    fn advance_stats(&self, a: rules::AdvanceId, items: &mut Vec<Item>) {
        let adv = &rules::ADVANCES[a as usize];
        let req = match (adv.prereq[0], adv.prereq[1]) {
            (Some(p), Some(q)) => format!("Requires {} and {}", advance_name(Some(p)), advance_name(Some(q))),
            (Some(p), None) | (None, Some(p)) => format!("Requires {}", advance_name(Some(p))),
            (None, None) => String::new(),
        };
        if !req.is_empty() {
            items.push(text(32, 84, BLUE, req));
        }
        let allows = items.len();
        items.push(text(32, 100, BLUE, "Allows:"));
        let start = 108;
        let mut y = start;
        for (i, other) in rules::ADVANCES.iter().enumerate() {
            if !rules::is_real(i as u8) || other.name.is_empty() {
                continue;
            }
            if other.prereq.contains(&Some(a)) {
                let with = other.prereq.iter().flatten().find(|p| **p != a).map(|p| format!(" (with {})", advance_name(Some(*p)))).unwrap_or_default();
                items.push(text(40, y, LIGHT_BLUE, format!("{}{}", other.name, with)));
                y += STAT_PITCH;
            }
        }
        y += 4;
        for (i, u) in rules::UNITS.iter().enumerate() {
            if u.prereq == Some(a) {
                items.push(Item::UnitIcon { x: 39, y: y - 5, kind: i as u8 });
                items.push(text(60, y, RED, format!("{} unit", u.name)));
                y += 12;
            }
        }
        for (i, imp) in rules::IMPROVEMENTS.iter().enumerate().skip(1) {
            if imp.prereq == Some(a) {
                if let Some(cell) = crate::ui::city_draw::improvement_cell(i as u8) {
                    items.push(Item::ImpIcon { x: 40, y: y - 2, cell });
                }
                items.push(text(60, y, GREEN, format!("{} improvement", imp.name)));
                y += 12;
            }
        }
        for (i, w) in rules::WONDERS.iter().enumerate().skip(1) {
            if w.prereq == Some(a) {
                items.push(Item::ImpIcon { x: 40, y: y - 2, cell: crate::ui::city_draw::wonder_cell(i as u8) });
                items.push(text(60, y, GREEN, format!("{} Wonder", w.name)));
                y += 12;
            }
        }
        if y == start + 4 {
            items.remove(allows);
        }
    }

    fn terrain_stats(&self, t: Terrain, items: &mut Vec<Item>) {
        let info = t.info();
        let yields = |items: &mut Vec<Item>, y: &mut i32, food: u8, shields: u8, trade: u8| {
            let mut any = false;
            for (label, v, tr) in [("Food", food, false), ("Shields", shields, false), ("Trade", trade, true)] {
                if v == 0 {
                    continue;
                }
                any = true;
                let mark = if v >= 3 { " *" } else if tr { " %" } else { "" };
                items.push(text(BODY_X + 4, *y, LIGHT_BLUE, format!("{label}: {v} units.{mark}")));
                *y += STAT_PITCH;
            }
            if !any {
                items.push(text(BODY_X + 4, *y, LIGHT_BLUE, "nothing"));
                *y += STAT_PITCH;
            }
        };
        let mut y = 84;
        items.push(text(BODY_X, y, BLUE, info.name));
        y += STAT_PITCH;
        yields(items, &mut y, info.food, info.shields, info.trade);
        if let Some(sp) = &info.special {
            y += 4;
            items.push(text(BODY_X, y, BLUE, sp.name));
            y += STAT_PITCH;
            yields(items, &mut y, sp.food, sp.shields, sp.trade);
        }
        y += 4;
        items.push(text(BODY_X + 4, y, LIGHT_BLUE, "*  -1 if government is Despotism/Anarchy."));
        y += STAT_PITCH;
        items.push(text(BODY_X + 4, y, LIGHT_BLUE, "%  +1 if government is Republic/Democracy."));
        y += STAT_PITCH + 4;
        items.push(text(BODY_X, y, RED, format!("Movement cost: {} MP", info.move_cost)));
        y += STAT_PITCH;
        items.push(text(BODY_X, y, RED, format!("Defense bonus: +{}%", (info.defense as i32 - 2) * 50)));
        y += STAT_PITCH;
        let terraform = |what: &str, unit: &str, tf: Terraform, turns: u8| -> Option<String> {
            match tf {
                Terraform::No => None,
                Terraform::Bonus(n) => Some(format!("{what}: +{n} {unit} ({turns} turns)")),
                Terraform::Becomes(other) => Some(format!("{what}: becomes {} ({turns} turns)", other.name())),
            }
        };
        if let Some(s) = terraform("Irrigation", "food", info.irrigate, info.irrigate_turns) {
            items.push(text(BODY_X, y, RED, s));
            y += STAT_PITCH;
        }
        if let Some(s) = terraform("Mining", "shields", info.mine, info.mine_turns) {
            items.push(text(BODY_X, y, RED, s));
        }
    }

    /// Picture of the entry: sheet, source rect, destination, transparent key.
    fn picture(&self) -> Option<(&'static str, Rect, i32, i32, Option<u8>)> {
        match self.entry {
            Entry::Advance(a) => advance_icon(a).map(|(s, r)| (s, r, 7, 8, None)),
            Entry::Unit(u) => {
                let (s, r) = unit_picture(u);
                Some((s, r, 9, 13, Some(253)))
            }
            Entry::Terrain(t) => {
                let (s, r) = terrain_picture(t);
                Some((s, r, 76 - r.w / 2, 48 - r.h / 2, Some(253)))
            }
            Entry::Improvement(i) => improvement_picture(i).map(|(s, r)| (s, r, 37, 17, Some(0))),
            _ => None,
        }
    }

    fn title_centre(&self) -> i32 {
        match self.entry {
            Entry::Advance(_) | Entry::Improvement(_) | Entry::Terrain(_) => 204,
            Entry::Unit(_) => 224,
            _ => 160,
        }
    }

    fn page_colour(&self) -> u8 {
        if matches!(self.entry, Entry::Terrain(_) | Entry::Wonder(_) | Entry::Concept(_)) {
            WHITE
        } else {
            PAGE
        }
    }

    fn civ_slot(ctx: &UiContext) -> u8 {
        ctx.game.as_ref().map(|g| g.human_slot).unwrap_or(1)
    }

    /// Map sprite of a unit as the Civilopedia draws it: guide row/column skipped, the
    /// placeholder colours replaced by the civilization's (15 is drawn in cyan as on the map).
    fn draw_unit_sprite(fb: &mut Surface, ctx: &UiContext, kind: u8, x: i32, y: i32) {
        let c = rules::SLOT_COLOURS[(Self::civ_slot(ctx) & 7) as usize];
        let r = sp::unit(kind);
        let sheet = &ctx.sp.surf;
        for yy in 1..r.h {
            for xx in 1..r.w {
                let v = sheet.get(r.x + xx, r.y + yy);
                let v = match v {
                    0 => continue,
                    10 => c.light,
                    2 => c.dark,
                    15 => 11,
                    v => v,
                };
                fb.put(x + xx, y + yy, v);
            }
        }
    }

    fn draw_coin(fb: &mut Surface, x: i32, y: i32, colour: u8) {
        const RING: [&str; 5] = [".###.", "##.##", "#...#", "##.##", ".###."];
        for (r, row) in RING.iter().enumerate() {
            for (c, ch) in row.bytes().enumerate() {
                if ch == b'#' {
                    fb.put(x + c as i32, y + 1 + r as i32, colour);
                }
            }
        }
    }
}

impl Screen for EntryScreen {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        self.build(ctx);
    }

    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }

    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        let next = match ev {
            Event::Key { key: Key::Esc, down: true, .. } => return Transition::Pop(DialogResult::Done),
            Event::Key { key: Key::Unknown, .. } => false,
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => true,
            _ => false,
        };
        if next {
            if self.page + 1 < self.pages.len() {
                self.page += 1;
            } else {
                return Transition::Pop(DialogResult::Done);
            }
        }
        Transition::None
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let page = self.page_colour();
        fb.fill_rect(Rect::new(0, 0, 320, 200), page);
        draw_border(fb, self.entry.dotted_border());
        if let Some((sheet, r, x, y, key)) = self.picture() {
            let pic = ctx.pic(sheet);
            fb.blit_part(&pic.surf, r, x, y, key);
        }
        let cx = self.title_centre();
        let title = self.entry.name().to_ascii_uppercase();
        draw_spaced_centred(fb, &ctx.fonts, FONT_TITLE, BLACK, cx, 20, &title, SPACING);
        draw_spaced_centred(fb, &ctx.fonts, FONT_TEXT, GREY, cx, 36, self.entry.subtitle(), SPACING);
        if let Entry::Unit(u) = self.entry {
            Self::draw_unit_sprite(fb, ctx, u, cx - 9, 47);
        }
        if let Some(items) = self.pages.get(self.page) {
            for it in items {
                match it {
                    Item::Text { x, y, colour, text } => {
                        draw_spaced(fb, &ctx.fonts, FONT_TEXT, *colour, *x, *y, text, SPACING);
                    }
                    Item::UnitIcon { x, y, kind } => {
                        Self::draw_unit_sprite(fb, ctx, *kind, *x, *y);
                    }
                    Item::ImpIcon { x, y, cell } => crate::ui::city_draw::imp_icon(fb, ctx, *cell, *x, *y),
                    Item::Coin { x, y, colour, text } => {
                        Self::draw_coin(fb, *x, *y, *colour);
                        draw_spaced(fb, &ctx.fonts, FONT_TEXT, *colour, *x + 7, *y, text, SPACING);
                    }
                }
            }
        }
    }

    fn palette(&self, ctx: &UiContext) -> Palette {
        let mut base = ctx.sp.pal;
        if self.page_colour() == WHITE {
            // the original's page white is a little darker than the sheet's bright white
            base.0[WHITE as usize] = [247, 247, 247];
        }
        match self.entry {
            Entry::Advance(a) => match advance_icon(a) {
                Some((sheet, _)) => base.merged(&ctx.pic(sheet).pal, 32..=255),
                None => base,
            },
            Entry::Unit(u) => base.merged(&ctx.pic(unit_picture(u).0).pal, 32..=255),
            Entry::Improvement(i) => match improvement_picture(i) {
                Some((sheet, _)) => base.merged(&ctx.pic(sheet).pal, 16..=255),
                None => base,
            },
            _ => base,
        }
    }

    fn draws_parent(&self) -> bool {
        false
    }

    fn wants_cursor(&self) -> bool {
        false
    }
}
