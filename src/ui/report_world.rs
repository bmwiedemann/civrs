//! The WORLD reports F7-F11: Wonders of the World, Top Five Cities, Civilization Score,
//! World Map and Demographics (SpaceShips, F12, is the ship viewer of `spaceship_ui`).

use crate::gfx::{Rect, Surface};
use crate::rules::{self, SLOT_COLOURS};
use crate::ui::context::UiContext;
use crate::ui::reports::*;
use crate::ui::tilerender::{blit_sprite, sp};

/// Row of citizen figures (8x16 cells on a 7-px pitch, alternating male/female): `happy`, then `content`, then
/// `unhappy`; squeezed when they do not fit in `max_w`.
pub fn citizens_row(fb: &mut Surface, ctx: &UiContext, x: i32, y: i32, happy: i32, content: i32, unhappy: i32, max_w: i32) {
    let n = happy + content + unhappy;
    if n <= 0 {
        return;
    }
    let step = if n * 7 <= max_w { 7 } else { (max_w / n).max(2) };
    let mut i = 0;
    for (count, base) in [(happy, 0u8), (content, 2u8), (unhappy, 4u8)] {
        for _ in 0..count {
            let idx = base + (i % 2) as u8;
            blit_sprite(fb, &ctx.sp.surf, sp::citizen(idx), x + i * step, y);
            i += 1;
        }
    }
}

// ---- F7 Wonders of the World --------------------------------------------------------------

/// F7 `wonders_of_the_world` (ovl 12 +0x80d). Nothing is drawn until the first built wonder: with
/// none the key just goes back to the map. Otherwise pages of colour 3 with "The Wonders of the
/// World" shadowed at (100, 12) and from y 32, 24 px apart, a box (8, y)..(311, y+15) in the
/// owner's colour, the wonder's pictogram at (16, y+3) and "The <wonder> of <city>. (<Adj>)"
/// (wonders 1..7; later ones "<wonder> in <city>...", a lost city " (Destroyed)") fitted to
/// 270 px at (40, y+5) in 15; a page ends when the next row would pass y 180.
pub struct WondersScreen {
    rows: Vec<(u8, String, u8)>,
    page: usize,
}

const WONDER_ROWS: usize = 7;

pub fn wonders_screen(ctx: &UiContext) -> Box<dyn crate::ui::screen::Screen> {
    let g = ctx.game();
    let mut rows = Vec::new();
    for w in 1..=21u8 {
        let Some(cid) = g.wonders_built[w as usize] else { continue };
        let name = rules::WONDERS[w as usize].name;
        let mut text = if w <= 7 { format!("The {name}") } else { name.to_string() };
        let colour = match g.city_opt(cid) {
            Some(c) => {
                let adj = g.player(c.owner).adjective();
                text += &format!("{}{}. ({adj})", if w <= 7 { " of " } else { " in " }, c.name);
                SLOT_COLOURS[(c.owner & 7) as usize].light
            }
            None => {
                text += " (Destroyed)";
                SLOT_COLOURS[0].light
            }
        };
        rows.push((w, fit_text(ctx, &text, 270), colour));
    }
    Box::new(WondersScreen { rows, page: 0 })
}

impl crate::ui::screen::Screen for WondersScreen {
    fn handle(&mut self, ev: &crate::platform::Event, _ctx: &mut UiContext) -> crate::ui::screen::Transition {
        use crate::platform::Event;
        use crate::ui::screen::{DialogResult, Transition};
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => {
                self.page += 1;
                if self.page * WONDER_ROWS >= self.rows.len() {
                    Transition::Pop(DialogResult::Done)
                } else {
                    Transition::None
                }
            }
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        if self.rows.is_empty() {
            return;
        }
        fb.fill(3);
        text_sh(fb, ctx, 100, 12, "The Wonders of the World", WHITE, BLACK);
        let st = crate::gfx::TextStyle::new(FONT, 15);
        for (i, (w, text, colour)) in self.rows.iter().skip(self.page * WONDER_ROWS).take(WONDER_ROWS).enumerate() {
            let y = 32 + 24 * i as i32;
            fb.rect_outline(Rect::new(8, y, 304, 16), *colour);
            crate::ui::city_draw::imp_icon(fb, ctx, crate::ui::city_draw::wonder_cell(*w), 16, y + 3);
            crate::gfx::text::draw_text(fb, &ctx.fonts, &st, 40, y + 5, text);
        }
    }
    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.game_palette()
    }
    /// With no wonder the map stays on the screen while the report waits for its key.
    fn draws_parent(&self) -> bool {
        self.rows.is_empty()
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

// ---- F8 Top Five Cities ----------------------------------------------------------------------

/// F8 `top_five_cities` (ovl 3 +0x9ac): every city scores size + happy - unhappy + 10 per
/// wonder in it; the best five (earlier cities first on a tie) on a colour-3 page under "The Top
/// Five Cities in the World" shadowed at (80, 12), 32 px apart from y 32: "N. <city> (<Adj>)"
/// centred at (160, y + 3) in the owner's colour, the citizens at (48, y + 10) in 160 px, the
/// city's wonders from x 48 + min(8 * size, 160) + 8 at y + 12, 19 px apart below x 293, and a
/// box (8, y)..(311, y + 25) in the owner's colour.
pub fn paint_top_five(fb: &mut Surface, ctx: &UiContext) {
    let g = ctx.game();
    let score = |c: &crate::game::City| {
        let h = g.city_happiness(c.id);
        let wonders = (1..=21u8).filter(|w| g.wonders_built[*w as usize] == Some(c.id)).count() as i32;
        c.size as i32 + h.happy - h.unhappy + 10 * wonders
    };
    let mut best: Vec<(i32, crate::game::CityId)> = Vec::new();
    for c in g.all_cities() {
        let s = score(c);
        let at = best.iter().position(|(b, _)| *b < s).unwrap_or(best.len());
        best.insert(at, (s, c.id));
        best.truncate(5);
    }
    text_sh(fb, ctx, 80, 12, "The Top Five Cities in the World", WHITE, BLACK);
    let mut y = 32;
    for (i, (_, id)) in best.iter().enumerate() {
        let c = g.city(*id);
        let colour = SLOT_COLOURS[(c.owner & 7) as usize].light;
        let line = format!("{}. {} ({})", i + 1, c.name, g.player(c.owner).adjective());
        let w = crate::gfx::text::text_width(&ctx.fonts, FONT, &line);
        crate::gfx::text::draw_text(fb, &ctx.fonts, &crate::gfx::TextStyle::new(FONT, colour), 160 - w / 2, y + 3, &line);
        crate::ui::cityscreen::draw_citizens(fb, ctx, *id, 48, y + 10, 160);
        let mut x = 48 + (8 * c.size as i32).clamp(0, 160) + 8;
        for wd in 1..=21u8 {
            if g.wonders_built[wd as usize] == Some(*id) {
                if x < 293 {
                    crate::ui::city_draw::imp_icon(fb, ctx, crate::ui::city_draw::wonder_cell(wd), x, y + 12);
                }
                x += 19;
            }
        }
        fb.rect_outline(Rect::new(8, y, 304, 26), colour);
        y += 32;
    }
}

// ---- F9 Civilization Score -----------------------------------------------------------------

/// F9 `civ_score` (ovl 20 +0xca9) on a colour-3 page: "<Adj> Citizens (N)" shadowed at
/// (16, 26) with one head per citizen (0x1c76d: 8 px apart from (16, 34), a new row 8 px lower
/// past x 292, every other row 4 px in, none below y 160), "<Adj> Achievements (N)" 16 px below
/// the last row with the civ's wonders (pictogram and name in 11, columns 152 px apart), then,
/// 10 px apart from 10 px lower: "SpaceShip: (N[ possible])", one line of "Pollution: (-N)",
/// "Peace: (+N)" and "Future Tech: (+N)", and "Total Score: N" (or "Bonus Score"); a bar
/// (35, 192) 250x7 in 8 filled in 11 for score / 4.
pub fn paint_score(fb: &mut Surface, ctx: &UiContext) {
    title_block(fb, ctx, "CIVILIZATION SCORE");
    let g = ctx.game();
    let me = g.human_slot;
    let s = g.score(me);
    let adj = g.human().adjective();
    // the heads: happy, content, unhappy, then the specialists
    let (mut hx, mut hy) = (999, 26);
    let mut head = |fb: &mut Surface, idx: u8| {
        hx += 8;
        if hx > 292 {
            hy += 8;
            hx = ((hy >> 1) & 4) + 16;
        }
        if hy <= 160 {
            blit_sprite(fb, &ctx.sp.surf, sp::citizen(idx), hx, hy);
        }
    };
    let mut citizens = 0;
    for c in g.cities_of(me) {
        let h = g.city_happiness(c);
        let city = g.city(c);
        for i in 0..h.happy {
            head(fb, (i & 1) as u8);
        }
        for i in 0..h.content {
            head(fb, 2 + (i & 1) as u8);
        }
        for i in 0..h.unhappy {
            head(fb, 4 + (i & 1) as u8);
        }
        for sp_kind in &city.specialists {
            head(fb, crate::ui::city_draw::specialist_index(*sp_kind));
        }
        citizens += city.size as i32 + h.happy - h.unhappy;
    }
    text_sh(fb, ctx, 16, 26, &format!("{adj} Citizens ({citizens})"), WHITE, BLACK);
    let ay = hy + 16;
    let mut y = ay;
    let mut x = 999;
    for w in 1..=21u8 {
        let Some(c) = g.wonders_built[w as usize].and_then(|c| g.city_opt(c)) else { continue };
        if c.owner != me {
            continue;
        }
        x += 152;
        if x > 300 {
            y += 10;
            x = 16;
        }
        if y < 180 {
            crate::ui::city_draw::imp_icon(fb, ctx, crate::ui::city_draw::wonder_cell(w), x, y - 2);
            crate::gfx::text::draw_text(fb, &ctx.fonts, &crate::gfx::TextStyle::new(FONT, 11), x + 20, y, rules::WONDERS[w as usize].name);
        }
    }
    text_sh(fb, ctx, 16, ay, &format!("{adj} Achievements ({})", 20 * s.wonders), WHITE, BLACK);
    let mut y = y + 10;
    if s.spaceship > 0 {
        text_sh(fb, ctx, 16, y, &format!("SpaceShip: ({})", s.spaceship), WHITE, BLACK);
        y += 10;
    }
    let mut line = String::new();
    if s.pollution != 0 {
        line += &format!("Pollution: ({})   ", -10 * s.pollution);
    }
    let peace = g.peace_points(me);
    if g.year() > 0 && g.player(me).peace_turns != 0 {
        line += &format!("Peace: (+{peace}) ");
    }
    if s.future_tech != 0 {
        line += &format!("Future Tech: (+{})", 5 * s.future_tech);
    }
    if !line.is_empty() && y < 184 {
        text_sh(fb, ctx, 16, y, &line, WHITE, BLACK);
        y += 10;
    }
    let label = if g.conquest_bonus().is_some_and(|b| b == s.total) { "Bonus Score" } else { "Total Score" };
    if y < 186 {
        text_sh(fb, ctx, 16, y, &format!("{label}: {}", s.total), WHITE, BLACK);
    }
    fb.fill_rect(Rect::new(35, 192, 250, 7), 8);
    let w = (s.total / 4).clamp(0, 250);
    if w > 0 {
        fb.fill_rect(Rect::new(35, 192, w, 7), 11);
    }
}

// ---- F10 World Map -----------------------------------------------------------------------------

/// F10 `world_map_view(detailed = 1)` (ovl 12 +0x0): the 80x50 world at 4x4 px on black,
/// scrolled so the capital's column (ds:0xdcec, the start square's column before a city is
/// founded) is at column 40, and vertically centred on the rows of the human's units and cities
/// (all rows once row 0 or 49 has been seen). Each square the human has seen is its 4x4
/// terrain sprite from SP299 (160 + 4 * terrain, 111 or 115 by (x + y) & 1); a city the human
/// knows fills its square in the owner's colour, a visible unit is a 3x3 dot in its owner's
/// colour with a black shadow.
pub fn paint_world_map(fb: &mut Surface, ctx: &UiContext) {
    let g = ctx.game();
    let me = g.human_slot;
    let anchor_x = g.human().capital.and_then(|c| g.city_opt(c)).map(|c| c.x).or_else(|| g.all_units().find(|u| u.owner == me).map(|u| u.x)).unwrap_or(40);
    let xoff = 40 - anchor_x;
    let rows: Vec<i32> = g.all_units().filter(|u| u.owner == me).map(|u| u.y).chain(g.all_cities().filter(|c| c.owner == me).map(|c| c.y)).collect();
    let (mut ymin, mut ymax) = (rows.iter().copied().min().unwrap_or(999), rows.iter().copied().max().unwrap_or(0));
    if (0..rules::MAP_W as i32).any(|x| g.explored(me, x, 0) || g.explored(me, x, rules::MAP_H as i32 - 1)) {
        ymin = 0;
        ymax = rules::MAP_H as i32 - 1;
    }
    let yoff = 25 - (ymin + ymax) / 2;
    let sheet = &ctx.pic("SP299").surf;
    for y in 0..rules::MAP_H as i32 {
        let sy = y + yoff;
        if !(0..rules::MAP_H as i32).contains(&sy) {
            continue;
        }
        for x in 0..rules::MAP_W as i32 {
            if !g.explored(me, x, y) {
                continue;
            }
            let (px, py) = (crate::game::Map::wrap_x(x + xoff) * 4, sy * 4);
            let t = g.map.tile(x, y).terrain as i32;
            fb.blit_part(sheet, Rect::new(160 + 4 * t, 111 + 4 * ((x + y) & 1), 4, 4), px, py, None);
            if let Some(c) = g.city_at(x, y).map(|c| g.city(c)) {
                fb.fill_rect(Rect::new(px, py, 4, 4), SLOT_COLOURS[(c.owner & 7) as usize].light);
            } else if let Some(u) = g.units_at(x, y).first().map(|u| g.unit(*u)) {
                if u.owner == me || u.seen_by & (1 << me) != 0 {
                    fb.fill_rect(Rect::new(px + 1, py + 1, 3, 3), 0);
                    fb.fill_rect(Rect::new(px, py, 3, 3), SLOT_COLOURS[(u.owner & 7) as usize].light);
                }
            }
        }
    }
}

// ---- F11 Demographics --------------------------------------------------------------------------

/// F11 `demographics(civ)` (ovl 12 +0xd6d, o 0x10ee2) on a colour-1 screen: "<Adjective>
/// Demographics" centred at y 4, then from y 24, 12 px apart, each statistic's label shadowed at
/// (8, y), the human's value at (104, y) and `print_rank` (o 0x11add): a colour-9 rule (4..316,
/// y - 3), "<rank>st/nd/rd/th" shadowed at (192, y) and, when another civ leads and the human
/// has an embassy with it, "(<civ>: <value><unit>)" ending at x 318 in its colour.
pub struct Demographics {
    rows: Vec<crate::game::score::DemographicRow>,
}

impl Demographics {
    pub fn new(ctx: &UiContext) -> Demographics {
        Demographics { rows: ctx.game().demographics(ctx.game().human_slot) }
    }
}

impl crate::ui::screen::Screen for Demographics {
    fn handle(&mut self, ev: &crate::platform::Event, _ctx: &mut UiContext) -> crate::ui::screen::Transition {
        use crate::platform::Event;
        use crate::ui::screen::{DialogResult, Transition};
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => Transition::Pop(DialogResult::Done),
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        use crate::ui::widgets::draw_coded_text;
        fb.fill(1);
        let g = ctx.game();
        let me = g.human_slot;
        let title = format!("{} Demographics", g.human().adjective());
        let w = crate::gfx::text::text_width(&ctx.fonts, FONT, &title);
        draw_coded_text(fb, ctx, FONT, 160 - w / 2, 4, &title, 15);
        let alive = g.alive_slots();
        let shadowed = |fb: &mut Surface, x: i32, y: i32, t: &str| {
            draw_coded_text(fb, ctx, FONT, x, y + 1, t, 0);
            draw_coded_text(fb, ctx, FONT, x, y, t, 15);
        };
        for (i, r) in self.rows.iter().enumerate() {
            let y = 24 + 12 * i as i32;
            shadowed(fb, 8, y, r.label);
            draw_coded_text(fb, ctx, FONT, 104, y, &r.text, 15);
            let (rank, best, civ) = crate::game::score::demographic_rank(&r.values, r.lower_better, me, &alive);
            let suffix = match rank {
                1 => "st",
                2 => "nd",
                3 => "rd",
                _ => "th",
            };
            fb.hline(4, 316, y - 3, 9);
            shadowed(fb, 192, y, &format!("{rank}{suffix}"));
            if civ != me && g.human().has_embassy(civ) {
                let lead = format!("({}: {}{})", g.player(civ).plural(), best.abs(), r.unit);
                let w = crate::gfx::text::text_width(&ctx.fonts, FONT, &lead);
                draw_coded_text(fb, ctx, FONT, 318 - w, y, &lead, SLOT_COLOURS[civ as usize].light);
            }
        }
    }
    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.game_palette()
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}
