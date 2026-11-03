//! Civilopedia entry pages: bordered page with picture, title, subtitle, the BLURB history
//! text and a stats page, laid out as in the original (measured on 320x200 captures).

use crate::assets::fonts::FontSet;
use crate::assets::Palette;
use crate::gfx::{Rect, Surface};
use crate::platform::{Event, Key};
use crate::rules::{self, Terraform, Terrain};
use crate::ui::context::UiContext;
use crate::ui::pedia_data::{advance_icon, improvement_picture, terrain_picture, unit_picture, Entry};
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::tilerender::sp;

// Palette indices (SP257 palette): the UI colours of the original pages.
const BLACK: u8 = 0;
const BLUE: u8 = 1;
const GREEN: u8 = 2;
const GREY: u8 = 7;
const LIGHT_BLUE: u8 = 9;
const RED: u8 = 12;
const PAGE: u8 = 15;
const WHITE: u8 = 16;

const FONT_TITLE: usize = 5;
const FONT_TEXT: usize = 6;
/// Every text on the pages is drawn with one extra pixel between glyphs.
const SPACING: i32 = 1;

const BODY_X: i32 = 12;
const BODY_Y: i32 = 76;
/// draw_text_block's widths in 8-pixel units: the history text (o 0xb7e1: 37) and the
/// game-effect notes (o 0xb847: 36)
const BODY_W8: i32 = 37;
const NOTE_W8: i32 = 36;
const BODY_PITCH: i32 = 9;
/// draw_text_block stops once its y passes 192 (0x1ee27): 13 lines from y 76, the rest of a
/// longer history text is never shown (after the key the text is cleared for the notes and
/// stats, o 0xb7ed..0xb809)
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

/// The lines `draw_text_block(width8, ...)` (CIV.EXE 0x1ed3e) makes of `s`: it adds up the
/// glyph widths (`drv_char_width` 0:0x78b: the width table + the font's spacing byte, 1 in
/// every font) from the line's start and, once the sum passes width8 * 8 or at a '\n' or '^',
/// ends the line at the last ' ', '\n' or '^' seen (that one left out; after a '^' also the
/// next character) and counts again from just after it. Blanks are kept as they are: a
/// double blank can leave one at a line's start.
pub fn text_block_lines(fonts: &FontSet, font: usize, width8: i32, s: &str) -> Vec<String> {
    let f = fonts.get(font);
    let b: Vec<char> = s.chars().collect();
    let mut lines = Vec::new();
    let (mut start, mut brk, mut w, mut a) = (0usize, 0usize, 0, 0usize);
    while a < b.len() {
        let c = b[a];
        w += f.advance_of(c) as i32 + crate::assets::fonts::GLYPH_SPACING;
        if matches!(c, ' ' | '\n' | '^') {
            brk = a;
        }
        if matches!(c, '\n' | '^') || w > width8 * 8 {
            lines.push(b[start.min(brk)..brk].iter().collect());
            w = 0;
            start = brk + if c == '^' { 2 } else { 1 };
            a = brk;
        }
        a += 1;
    }
    if w > 0 && start < b.len() {
        lines.push(b[start..].iter().collect());
    }
    lines
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
// Border
// ---------------------------------------------------------------------------------------------

/// Draw the 8-pixel page border as CIV.EXE 0x7e6:0x260a does: one of two tile sets of SP299,
/// picked at random (`style` 0: grey meander at x 192, 1: red/green dots at x 224). Row y 120
/// holds the corners (top-left, top-right, bottom-left, bottom-right), row 128 the edges (top,
/// right, bottom, left).
pub fn draw_border(fb: &mut Surface, ctx: &UiContext, style: u8) {
    let sheet = &ctx.pic("SP299").surf;
    let x0 = 192 + 32 * (style as i32 & 1);
    let tile = |fb: &mut Surface, sx: i32, sy: i32, x: i32, y: i32| fb.blit_part(sheet, Rect::new(sx, sy, 8, 8), x, y, None);
    tile(fb, x0, 120, 0, 0);
    tile(fb, x0 + 8, 120, 312, 0);
    tile(fb, x0 + 24, 120, 312, 192);
    tile(fb, x0 + 16, 120, 0, 192);
    for i in 1..24 {
        tile(fb, x0 + 24, 128, 0, 8 * i);
        tile(fb, x0 + 8, 128, 312, 8 * i);
    }
    for i in 1..39 {
        tile(fb, x0, 128, 8 * i, 0);
        tile(fb, x0 + 16, 128, 8 * i, 192);
    }
}

// ---------------------------------------------------------------------------------------------
// Page content
// ---------------------------------------------------------------------------------------------

enum Item {
    Text { x: i32, y: i32, colour: u8, text: String },
    /// Map sprite of a unit type (16x16 cell) for the "Allows:" list.
    UnitIcon { x: i32, y: i32, kind: u8 },
    /// Text centred on `cx`.
    Centred { cx: i32, y: i32, colour: u8, text: String },
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
    /// Border tile set, picked at random when the entry opens.
    border: u8,
    /// pedia_article with ds:0x3c62 set (o 0xb5ee, 0xb663, 0xb699; set before the advances an
    /// audience grants, o 0x6b37 / 0x6b56 / 0x712d / 0x736b): no picture, the title centred at
    /// x 160, the page filled with colour 255 of the palette in use (the leader's).
    bare: bool,
}

impl EntryScreen {
    pub fn new(entry: Entry, page: usize) -> EntryScreen {
        EntryScreen { entry, pages: Vec::new(), page, guard: 2, border: 0, bare: false }
    }

    /// The page without its picture, over the palette in use (`bare`).
    pub fn bare(entry: Entry) -> EntryScreen {
        EntryScreen { bare: true, ..EntryScreen::new(entry, 0) }
    }


    fn build(&mut self, ctx: &UiContext) {
        let fonts = &ctx.fonts;
        let mut pages: Vec<Vec<Item>> = Vec::new();
        // history text, BODY_LINES lines per page, only with the Civilopedia Text option
        // (ds:0x19c0 bit 0x40, o 0xb7bb); the notes and stats page comes either way
        let history = ctx.game.as_ref().is_none_or(|g| g.options.civilopedia_text);
        let mut body = if history { text_block_lines(fonts, FONT_TEXT, BODY_W8, &self.entry.blurb(ctx)) } else { Vec::new() };
        body.truncate(BODY_LINES);
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
        let note = text_block_lines(fonts, FONT_TEXT, NOTE_W8, &self.entry.blurb2(ctx));
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
            Entry::Advance(a) => {
                self.advance_stats(a, items);
                self.advance_source(ctx, a, items);
            }
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
            // a game concept has its texts only, governments too (o 0xc5ca)
            Entry::Concept(_) => {}
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

    /// How the human player gained the advance, centred under the subtitle (CIV.EXE o 0xbe09):
    /// "(Discovered)", "(from Great Library)" or "(Taken from CIV)"; nothing for advances not
    /// known or known from the start.
    fn advance_source(&self, ctx: &UiContext, a: rules::AdvanceId, items: &mut Vec<Item>) {
        let Some(g) = ctx.game.as_ref() else { return };
        let line = match g.human().advance_source(a) {
            -1 => return,
            0 => "(Discovered)".to_string(),
            -2 => "(from Great Library)".to_string(),
            s => format!("(Taken from {})", g.civ_name(s as u8)),
        };
        items.push(Item::Centred { cx: self.title_centre(), y: 48, colour: GREY, text: line });
    }

    /// The terrain's stats page (o 0xc13f..0xc5ab): the terrain and, unless its food is 2
    /// (Grassland, River), its special resource, each with its name and the lines "Food: N
    /// (M with Irrigation) units.", "Production: N (M with Mining) units." and "Trade: N (M%
    /// with Roads)" where they apply, "nothing" for plain Arctic; then the government notes,
    /// the movement cost and the defence bonus.
    fn terrain_stats(&self, t: Terrain, items: &mut Vec<Item>) {
        let info = t.info();
        let bonus = |tf: Terraform| match tf {
            Terraform::Bonus(n) => Some(n),
            _ => None,
        };
        let (irrigation, mining) = (bonus(info.irrigate), bonus(info.mine));
        let star = |on: bool| if on { "*" } else { "" };
        // CIV.EXE's table gives Grassland 1 shield, shown as "0/1" like River's (o 0xc22f)
        let base_shields = if t == Terrain::Grassland { 1 } else { info.shields };
        let mut variants = vec![(info.name, info.food, base_shields, info.trade)];
        if info.food != 2 {
            if let Some(sp) = &info.special {
                variants.push((sp.name, sp.food, sp.shields, sp.trade));
            }
        }
        let mut y = 84;
        for (k, (name, food, shields, trade)) in variants.into_iter().enumerate() {
            items.push(text(BODY_X, y, BLUE, name));
            y += STAT_PITCH;
            if food != 0 || irrigation.is_some() {
                let with = irrigation.map(|b| format!(" ({}{} with Irrigation)", food + b, star(food > 1))).unwrap_or_default();
                items.push(text(BODY_X + 4, y, LIGHT_BLUE, format!("Food: {food}{}{with} units.", star(food > 2))));
                y += STAT_PITCH;
            }
            if shields != 0 || mining.is_some() {
                let resources = if matches!(t, Terrain::Grassland | Terrain::River) { "0/" } else { "" };
                let with = mining.map(|b| format!(" ({}{} with Mining)", shields + b, star(shields + b > 2))).unwrap_or_default();
                items.push(text(BODY_X + 4, y, LIGHT_BLUE, format!("Production: {resources}{shields}{with} units.")));
                y += STAT_PITCH;
            }
            if trade != 0 || (t as u8) < 3 {
                let roads = if (t as u8) < 3 { format!(" ({}% with Roads)", trade + 1) } else { String::new() };
                let pct = if trade != 0 { "%" } else { "" };
                items.push(text(BODY_X + 4, y, LIGHT_BLUE, format!("Trade: {trade}{pct}{}{roads}", star(trade > 2))));
                y += STAT_PITCH;
            }
            if k == 0 && t == Terrain::Arctic {
                items.push(text(BODY_X + 4, y, LIGHT_BLUE, "nothing"));
                y += STAT_PITCH;
            }
            y += 4;
        }
        items.push(text(BODY_X + 4, y, LIGHT_BLUE, "*  -1 if government is Despotism/Anarchy."));
        y += STAT_PITCH;
        items.push(text(BODY_X + 4, y, LIGHT_BLUE, "%  +1 if government is Republic/Democracy."));
        y += STAT_PITCH + 4;
        items.push(text(BODY_X, y, RED, format!("Movement cost: {} MP", info.move_cost)));
        y += STAT_PITCH;
        items.push(text(BODY_X, y, RED, format!("Defense bonus: +{}%", (info.defense as i32 - 2) * 50)));
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
        if self.bare {
            return 160;
        }
        match self.entry {
            Entry::Advance(_) | Entry::Improvement(_) | Entry::Terrain(_) => 204,
            Entry::Unit(_) => 224,
            _ => 160,
        }
    }

    fn page_colour(&self) -> u8 {
        if self.bare {
            return 255;
        }
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
        // random(2) in the original; the frame counter keeps headless renders repeatable
        self.border = (ctx.frame.wrapping_mul(0x9e37_79b9) >> 13) as u8 & 1;
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
            // every page waits for any key or click (wait_key_or_click 0x1468e, o 0xb7ed), Esc
            // included: on the history page it shows the stats page
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
        draw_border(fb, ctx, self.border);
        if let Some((sheet, r, x, y, key)) = self.picture().filter(|_| !self.bare) {
            let pic = ctx.pic(sheet);
            fb.blit_part(&pic.surf, r, x, y, key);
        }
        let cx = self.title_centre();
        let title = self.entry.name().to_ascii_uppercase();
        draw_spaced_centred(fb, &ctx.fonts, FONT_TITLE, BLACK, cx, 20, &title, SPACING);
        draw_spaced_centred(fb, &ctx.fonts, FONT_TEXT, GREY, cx, 36, self.entry.subtitle(), SPACING);
        // the unit at (216, 48) also on a bare page (o 0xba09..0xba19, after the ds:0x3c62
        // test that leaves out only the picture)
        if let Entry::Unit(u) = self.entry {
            Self::draw_unit_sprite(fb, ctx, u, 215, 47);
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
                    Item::Centred { cx, y, colour, text } => {
                        draw_spaced_centred(fb, &ctx.fonts, FONT_TEXT, *colour, *cx, *y, text, SPACING);
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

    fn parent_palette(&self) -> bool {
        self.bare
    }

    fn wants_cursor(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(t: Terrain) -> Vec<String> {
        let mut items = Vec::new();
        EntryScreen::new(Entry::Terrain(t), 0).terrain_stats(t, &mut items);
        items.into_iter().filter_map(|i| if let Item::Text { text, .. } = i { Some(text) } else { None }).collect()
    }

    #[test]
    fn terrain_pages_have_civ_exes_lines() {
        let desert = lines(Terrain::Desert);
        assert_eq!(&desert[..4], ["Desert", "Food: 0 (1 with Irrigation) units.", "Production: 1 (2 with Mining) units.", "Trade: 0 (1% with Roads)"]);
        assert_eq!(desert[5], "Food: 3* (4* with Irrigation) units.");
        let grass = lines(Terrain::Grassland);
        assert_eq!(&grass[..4], ["Grassland", "Food: 2 (3* with Irrigation) units.", "Production: 0/1 units.", "Trade: 0 (1% with Roads)"]);
        assert_eq!(grass[4], "*  -1 if government is Despotism/Anarchy.", "no resource variant for food 2");
        let arctic = lines(Terrain::Arctic);
        assert_eq!(&arctic[..4], ["Arctic", "nothing", "Seals", "Food: 2 units."]);
        assert_eq!(lines(Terrain::Mountains)[4], "Trade: 6%*");
    }

    fn mono(advance: u8) -> FontSet {
        let font = crate::assets::fonts::Font { first: 32, last: 126, height: 7, bytes_per_row: 1, glyphs: (32..=126).map(|_| crate::assets::fonts::Glyph { advance, rows: Vec::new() }).collect() };
        FontSet(vec![font; 7])
    }

    #[test]
    fn text_blocks_break_as_draw_text_block() {
        // 3 + 1 pixels a glyph: two glyphs to 8 pixels, the third passes
        let f = mono(3);
        assert_eq!(text_block_lines(&f, 6, 1, "ab cd "), ["ab", "cd"]);
        // the blank that passes the width ends the line; a double blank then starts the next
        assert_eq!(text_block_lines(&f, 6, 2, "abcd  ef "), ["abcd", " ef "]);
        // '^' breaks and the blank after it is skipped
        assert_eq!(text_block_lines(&f, 6, 10, "ab^ cd "), ["ab", "cd "]);
        // a word that passes the width goes to the next line whole
        assert_eq!(text_block_lines(&f, 6, 3, "abc defg hi "), ["abc", "defg", "hi "]);
    }

    #[test]
    fn an_audience_page_has_no_picture_and_the_leaders_palette() {
        let e = EntryScreen::bare(Entry::Advance(0));
        assert_eq!((e.title_centre(), e.page_colour()), (160, 255));
        assert!(e.parent_palette());
        let n = EntryScreen::new(Entry::Advance(0), 0);
        assert_eq!((n.title_centre(), n.parent_palette()), (204, false));
    }
}
