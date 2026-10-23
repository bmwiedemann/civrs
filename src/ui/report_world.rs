//! The WORLD reports F7-F11: Wonders of the World, Top Five Cities, Civilization Score,
//! World Map and Demographics (SpaceShips, F12, is the ship viewer of `spaceship_ui`).

use crate::gfx::{Rect, Surface};
use crate::rules::{self, Terrain, SLOT_COLOURS};
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

pub fn paint_wonders(fb: &mut Surface, ctx: &UiContext) {
    text_sh(fb, ctx, 80, 12, "Wonders of the World", WHITE, BLACK);
    let g = ctx.game();
    let mut y = 32;
    let mut any = false;
    for w in 1..=21u8 {
        let Some(cid) = g.wonders_built[w as usize] else { continue };
        let Some(c) = g.city_opt(cid) else { continue };
        any = true;
        let obsolete = g.wonder_obsolete(w);
        let colour = if obsolete { GREY } else { WHITE };
        text_sh(fb, ctx, 16, y, rules::WONDERS[w as usize].name, colour, BLACK);
        let owner = g.player(c.owner);
        text_sh(fb, ctx, 176, y, &format!("{} ({})", c.name, owner.adjective()), colour, BLACK);
        if obsolete {
            tiny(fb, ctx, 290, y + 1, "OBS.", GREY);
        }
        y += LINE_H;
    }
    if !any {
        text_sh(fb, ctx, 16, 40, "No Wonders of the World have been built yet.", WHITE, BLACK);
    }
}

// ---- F8 Top Five Cities ----------------------------------------------------------------------

pub fn paint_top_five(fb: &mut Surface, ctx: &UiContext) {
    text_sh(fb, ctx, 80, 12, "The Top Five Cities in the World", WHITE, BLACK);
    let g = ctx.game();
    let mut cities: Vec<&crate::game::City> = g.all_cities().collect();
    // size, then wonders, then founding order
    cities.sort_by(|a, b| b.size.cmp(&a.size).then(b.wonder_list().len().cmp(&a.wonder_list().len())).then(a.founded_turn.cmp(&b.founded_turn)));
    let white = col(ctx, WHITE);
    for (i, c) in cities.iter().take(5).enumerate() {
        let y = 32 + 30 * i as i32;
        let r = Rect::new(8, y, 304, 26);
        fb.rect_outline(r, white);
        let owner = g.player(c.owner);
        text_c_sh(fb, ctx, 160, y + 4, &format!("{}. {} ({})", i + 1, c.name, owner.adjective()), WHITE, BLACK);
        let h = g.city_happiness(c.id);
        citizens_row(fb, ctx, 49, y + 9, h.happy, h.content, h.unhappy, 160);
        let wonders: Vec<&str> = c.wonder_list().iter().map(|w| rules::WONDERS[*w as usize].name).collect();
        for (j, w) in wonders.iter().take(2).enumerate() {
            tiny(fb, ctx, 216, y + 12 + 7 * j as i32, &w.to_ascii_uppercase(), WHITE);
        }
        if wonders.len() > 2 {
            tiny(fb, ctx, 290, y + 19, &format!("+{}", wonders.len() - 2), WHITE);
        }
    }
}

// ---- F9 Civilization Score -----------------------------------------------------------------

pub fn paint_score(fb: &mut Surface, ctx: &UiContext) {
    title_block(fb, ctx, "CIVILIZATION SCORE");
    let g = ctx.game();
    let me = g.human_slot;
    let s = g.score(me);
    let adj = g.human().adjective();
    let (mut happy, mut content) = (0, 0);
    for c in g.cities_of(me) {
        let h = g.city_happiness(c);
        happy += h.happy;
        content += h.content + g.city(c).specialists.len() as i32;
    }
    let citizens = s.happy * 2 + s.content;
    text_sh(fb, ctx, 16, 26, &format!("{adj} Citizens ({citizens})"), WHITE, BLACK);
    citizens_row(fb, ctx, 16, 34, happy, content, 0, 288);
    let achievements = s.wonders * 20 + s.future_tech * 5 + s.peace * 3 + s.spaceship;
    text_sh(fb, ctx, 16, 50, &format!("{adj} Achievements ({achievements})"), WHITE, BLACK);
    let mut y = 59;
    let mut detail = |fb: &mut Surface, line: String| {
        text_sh(fb, ctx, 24, y, &line, WHITE, BLACK);
        y += LINE_H;
    };
    if s.wonders > 0 {
        detail(fb, format!("Wonders of the World: {} x 20", s.wonders));
    }
    if s.future_tech > 0 {
        detail(fb, format!("Future Technologies: {} x 5", s.future_tech));
    }
    if s.peace > 0 {
        detail(fb, format!("Years of peace: {} turns x 3", s.peace));
    }
    if s.spaceship > 0 {
        detail(fb, format!("Spaceship: {}", s.spaceship));
    }
    if s.pollution > 0 {
        detail(fb, format!("Pollution: {} x -10", s.pollution));
    }
    let y = if y == 59 { 60 } else { y + 1 };
    text_sh(fb, ctx, 16, y, &format!("Total Score: {}", s.total), WHITE, BLACK);
    let rating = g.rating_name(me);
    let adjective = rules::ADJECTIVES[crate::game::score::adjective_index(s.percent)];
    text_c_sh(fb, ctx, 160, 182, &format!("Civilization rating: {}% ({adjective}, like {rating})", s.percent), WHITE, BLACK);
    // rating bar
    let bar = Rect::new(35, 192, 250, 7);
    fb.fill_rect(bar, col(ctx, DARK_GREY));
    let w = (bar.w * s.percent.clamp(0, 100) / 100).max(0);
    if w > 0 {
        fb.fill_rect(Rect::new(bar.x, bar.y, w, bar.h), col(ctx, CYAN));
    }
}

// ---- F10 World Map -----------------------------------------------------------------------------

fn map_colour(t: Terrain) -> [u8; 3] {
    match t {
        Terrain::Ocean => [85, 73, 162],
        Terrain::Forest | Terrain::Jungle | Terrain::Swamp => [24, 146, 24],
        Terrain::Hills | Terrain::Mountains => [190, 182, 85],
        Terrain::Desert => [223, 194, 125],
        Terrain::Tundra | Terrain::Arctic => [182, 190, 210],
        Terrain::River => [85, 89, 162],
        _ => [113, 146, 49],
    }
}

/// The whole 80x50 world at 4x4 pixels per tile, explored tiles only, cities as civ-coloured dots.
pub fn paint_world_map(fb: &mut Surface, ctx: &UiContext) {
    let g = ctx.game();
    let me = g.human_slot;
    let mut cols = [0u8; 12];
    for (i, c) in cols.iter_mut().enumerate() {
        *c = col(ctx, map_colour(terrain_at(i)));
    }
    for y in 0..rules::MAP_H as i32 {
        for x in 0..rules::MAP_W as i32 {
            if !g.explored(me, x, y) {
                continue;
            }
            let t = g.map.tile(x, y).terrain;
            fb.fill_rect(Rect::new(x * 4, y * 4, 4, 4), cols[(t as usize).min(11)]);
        }
    }
    for c in g.all_cities() {
        if !g.explored(me, c.x, c.y) {
            continue;
        }
        let colour = SLOT_COLOURS[(c.owner & 7) as usize].light;
        fb.fill_rect(Rect::new(c.x * 4, c.y * 4 + 1, 4, 2), colour);
        fb.fill_rect(Rect::new(c.x * 4 + 1, c.y * 4, 2, 4), colour);
        fb.put(c.x * 4 + 1, c.y * 4 + 1, col(ctx, WHITE));
        fb.put(c.x * 4 + 2, c.y * 4 + 2, col(ctx, WHITE));
    }
}

fn terrain_at(i: usize) -> Terrain {
    const ALL: [Terrain; 12] = [
        Terrain::Desert,
        Terrain::Plains,
        Terrain::Grassland,
        Terrain::Forest,
        Terrain::Hills,
        Terrain::Mountains,
        Terrain::Tundra,
        Terrain::Arctic,
        Terrain::Swamp,
        Terrain::Jungle,
        Terrain::Ocean,
        Terrain::River,
    ];
    // the Terrain discriminants follow this order (see rules::terrain)
    ALL.iter().copied().find(|t| *t as usize == i).unwrap_or(Terrain::Grassland)
}

// ---- F11 Demographics --------------------------------------------------------------------------

fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (1, 11) | (2, 12) | (3, 13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// Value of a demographics category for a slot (mirrors `Game::demographics`).
fn demographic_value(g: &crate::game::Game, slot: crate::game::Slot, category: usize) -> i64 {
    match category {
        0 => g.player(slot).gold as i64,
        1 => g.power(slot) as i64,
        2 => g.player(slot).advance_count() as i64 + g.player(slot).future_techs as i64,
        3 => g.happiness_ratio(slot),
        _ => g.total_population(slot),
    }
}

fn demographic_text(category: usize, v: i64) -> String {
    match category {
        0 => format!("{v}{GLYPH_COIN}"),
        1 => format!("{v} power"),
        2 => format!("{v} advances"),
        3 => format!("{v}%"),
        _ => commas(v),
    }
}

pub fn paint_demographics(fb: &mut Surface, ctx: &UiContext) {
    title_block(fb, ctx, "DEMOGRAPHICS");
    let g = ctx.game();
    let me = g.human_slot;
    let d = g.demographics(me);
    let alive = g.alive_slots();
    text_sh(fb, ctx, 16, 36, "Category", WHITE, BLACK);
    text_sh(fb, ctx, 120, 36, "Value", WHITE, BLACK);
    text_sh(fb, ctx, 184, 36, "Rank", WHITE, BLACK);
    text_sh(fb, ctx, 224, 36, "Best", WHITE, BLACK);
    fb.hline(16, 304, 44, col(ctx, WHITE));
    for (i, e) in d.entries.iter().enumerate() {
        let y = 50 + i as i32 * 20;
        tiny(fb, ctx, 16, y + 1, e.category, WHITE);
        text_sh(fb, ctx, 120, y, &demographic_text(i, e.value), WHITE, BLACK);
        text_sh(fb, ctx, 184, y, &ordinal(e.rank), WHITE, BLACK);
        let best = alive.iter().copied().max_by_key(|s| demographic_value(g, *s, i));
        if let Some(b) = best {
            let known = b == me || g.human().has_contact(b);
            let name = if known { g.player(b).plural().to_string() } else { "Unknown".to_string() };
            text_sh(fb, ctx, 224, y, &name, if b == me { CYAN } else { WHITE }, BLACK);
        }
    }
    text_c_sh(fb, ctx, 160, 168, &format!("Civilizations ranked: {}", alive.len()), WHITE, BLACK);
}
