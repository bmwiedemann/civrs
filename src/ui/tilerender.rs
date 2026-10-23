//! Drawing map tiles from the original sprite sheets (TER257 / SP257).
//!
//! Land = flat base tile (SP257 cell 0,4) + terrain overlay from TER257 (row = terrain,
//! column = mask of orthogonal neighbours with the same terrain, N=1 E=2 S=4 W=8).
//! Ocean = TER257 row 10, column = mask of orthogonal land neighbours.
//! River = base + SP257 row 4 variant by mask of river/ocean neighbours.

use crate::game::{tile_flags, Game, Map, Slot, UnitOrder};
use crate::gfx::{Rect, Surface};
use crate::rules::{self, Terrain, SLOT_COLOURS};
use crate::ui::context::UiContext;

pub const TILE: i32 = 16;

#[inline]
pub fn cell(cx: i32, cy: i32) -> Rect {
    Rect::new(cx * 16, cy * 16, 16, 16)
}

/// Cells in SP257 (cx, cy).
pub mod sp {
    use super::cell;
    use crate::gfx::Rect;
    pub const BASE_LAND: (i32, i32) = (0, 4);
    pub const CURSOR_GO: (i32, i32) = (2, 2);
    pub const POLLUTION: (i32, i32) = (3, 2);
    pub const IRRIGATION: (i32, i32) = (4, 2);
    /// Map icons 5 and 0x1f of CIV.EXE's icon table (ds:0xd4ce, filled at o 0x59d7): icons
    /// 0..6 are cells (i, 2), icons 0x10.. row 7. Cell (7, 2) is the TORCH cursor.
    pub const MINE: (i32, i32) = (5, 2);
    pub const HUT: (i32, i32) = (15, 7);
    /// 15x15 overlays drawn at (+1,+1): city buildings (colour 5 = owner's dark shade), walls
    pub const CITY_TEXTURE: (i32, i32) = (193, 113);
    pub const CITY_WALLS: (i32, i32) = (209, 113);
    pub const FORTIFIED: (i32, i32) = (13, 7);
    pub const FORTRESS: (i32, i32) = (14, 7);
    pub const PATTERN_CHECKER: (i32, i32) = (18, 11);
    pub const PATTERN_STONE: (i32, i32) = (19, 11);
    pub fn road(d: usize) -> Rect {
        cell(d as i32, 3)
    }
    pub fn rail(d: usize) -> Rect {
        cell(8 + d as i32, 6)
    }
    pub fn river(mask: u8) -> Rect {
        cell(mask as i32, 4)
    }
    pub fn special(t: crate::rules::Terrain) -> Rect {
        cell(t as i32, 7)
    }
    pub fn unit(kind: u8) -> Rect {
        cell((kind % 20) as i32, 10 + (kind / 20) as i32)
    }
    pub fn explosion(frame: u8) -> Rect {
        cell((frame % 8) as i32, 6)
    }
    /// Dotted black fringe along one side of an explored tile next to unexplored land
    /// (0 = N, 1 = E, 2 = S, 3 = W).
    pub fn fog_edge(side: usize) -> Rect {
        cell(5 + side as i32, 8)
    }
    pub fn citizen(idx: u8) -> Rect {
        // two 8px figures per cell starting at cell (0,8)
        Rect::new((idx as i32) * 8, 8 * 16, 8, 16)
    }
}

/// Colour remap table turning the unit sprite placeholders (10 light, 2 dark) into a slot's colours.
pub fn civ_remap(slot: Slot) -> [u8; 256] {
    let mut m = [0u8; 256];
    for (i, v) in m.iter_mut().enumerate() {
        *v = i as u8;
    }
    let c = SLOT_COLOURS[(slot & 7) as usize];
    m[10] = c.light;
    m[2] = c.dark;
    m[3] = c.dark;
    // the original draws the sprites' index 15 in cyan on the map
    m[15] = 11;
    m
}

/// Blit a 16x16 sprite cell with colour 0 transparent, masking the 1px guide line (colour 3 or 9)
/// that some cells carry on their top row / left column.
pub fn blit_sprite(fb: &mut Surface, sheet: &Surface, r: Rect, x: i32, y: i32) {
    for yy in 0..r.h {
        for xx in 0..r.w {
            let v = sheet.get(r.x + xx, r.y + yy);
            if v == 0 {
                continue;
            }
            if (yy == 0 || xx == 0) && (v == 3 || v == 9) {
                continue;
            }
            fb.put(x + xx, y + yy, v);
        }
    }
}

pub fn blit_sprite_remap(fb: &mut Surface, sheet: &Surface, r: Rect, x: i32, y: i32, map: &[u8; 256]) {
    for yy in 0..r.h {
        for xx in 0..r.w {
            let v = sheet.get(r.x + xx, r.y + yy);
            if v == 0 {
                continue;
            }
            if (yy == 0 || xx == 0) && (v == 3 || v == 9) {
                continue;
            }
            fb.put(x + xx, y + yy, map[v as usize]);
        }
    }
}

/// Draw the terrain (no units, no city) of map tile (x,y) at pixel (px,py).
pub fn draw_terrain(fb: &mut Surface, ctx: &UiContext, game: &Game, x: i32, y: i32, px: i32, py: i32) {
    let map = &game.map;
    let t = map.tile(x, y);
    let ter = &ctx.ter.surf;
    let sp = &ctx.sp.surf;
    // drawing order of CIV.EXE 0x1aa40: the land base, irrigation (not under a city), the
    // river or terrain sprite, the grassland shield, fortress, roads, mine, special
    // resource, hut and pollution, so irrigation never covers rivers or resources
    let irrigation = || t.has(tile_flags::IRRIGATION) && t.city.is_none();
    match t.terrain {
        Terrain::Ocean => {
            let mask = map.land_mask4(x, y);
            fb.blit_part(ter, cell(mask as i32, 10), px, py, None);
        }
        Terrain::River => {
            fb.blit_part(sp, cell(sp::BASE_LAND.0, sp::BASE_LAND.1), px, py, None);
            if irrigation() {
                blit_sprite(fb, sp, cell(sp::IRRIGATION.0, sp::IRRIGATION.1), px, py);
            }
            let mask = map.river_mask(x, y);
            blit_sprite(fb, sp, sp::river(mask), px, py);
        }
        land => {
            fb.blit_part(sp, cell(sp::BASE_LAND.0, sp::BASE_LAND.1), px, py, None);
            if irrigation() {
                blit_sprite(fb, sp, cell(sp::IRRIGATION.0, sp::IRRIGATION.1), px, py);
            }
            let mask = map.same_terrain_mask(x, y);
            blit_sprite(fb, ter, cell(mask as i32, land as i32), px, py);
        }
    }
    // the shield of a Grassland square (CIV.EXE 0x1afac): SP257 icon 15 at (+4, +4)
    if t.terrain == Terrain::Grassland && t.shield {
        fb.blit_part(sp, Rect::new(153, 41, 7, 7), px + 4, py + 4, Some(0));
    }
    if t.has(tile_flags::FORTRESS) {
        blit_sprite(fb, sp, cell(sp::FORTRESS.0, sp::FORTRESS.1), px, py);
    }
    // roads / railroads: one piece per neighbour that also has one (or a city)
    if t.has(tile_flags::ROAD) || t.has(tile_flags::RAILROAD) {
        let rail_here = t.has(tile_flags::RAILROAD);
        let rmask = map.road_mask8(x, y, false);
        let railmask = map.road_mask8(x, y, true);
        let mut any = false;
        for d in 0..8usize {
            if rmask & (1 << d) != 0 {
                any = true;
                if rail_here && railmask & (1 << d) != 0 {
                    blit_sprite(fb, sp, sp::rail(d), px, py);
                } else {
                    blit_sprite(fb, sp, sp::road(d), px, py);
                }
            }
        }
        if !any {
            // isolated road: small dot in the centre
            let c = if rail_here { 8 } else { 6 };
            fb.fill_rect(Rect::new(px + 7, py + 7, 2, 2), c);
        }
    }
    if t.has(tile_flags::MINE) {
        blit_sprite(fb, sp, cell(sp::MINE.0, sp::MINE.1), px, py);
    }
    if t.special {
        blit_sprite(fb, sp, sp::special(t.terrain), px, py);
    }
    if t.has(tile_flags::HUT) {
        blit_sprite(fb, sp, cell(sp::HUT.0, sp::HUT.1), px, py);
    }
    if t.has(tile_flags::POLLUTION) {
        blit_sprite(fb, sp, cell(sp::POLLUTION.0, sp::POLLUTION.1), px, py);
    }
}

/// Draw a city on the map tile at pixel (px,py), as CIV.EXE 0x1b2be does: a white frame, the
/// owner's dark frame along the top and right, the owner's light fill, the building texture
/// (SP257 cell 12,7) in the dark shade, the size in black, a black outline when units are
/// inside and the wall overlay (cell 13,7).
pub fn draw_city(fb: &mut Surface, ctx: &UiContext, game: &Game, city: crate::game::CityId, px: i32, py: i32) {
    let c = game.city(city);
    let col = SLOT_COLOURS[(c.owner & 7) as usize];
    fb.rect_outline(Rect::new(px + 1, py + 1, 14, 14), 15);
    fb.rect_outline(Rect::new(px + 2, py + 1, 13, 13), col.dark);
    fb.fill_rect(Rect::new(px + 2, py + 2, 12, 12), col.light);
    let mut map = [0u8; 256];
    for (i, v) in map.iter_mut().enumerate() {
        *v = i as u8;
    }
    map[5] = col.dark;
    blit_sprite_remap(fb, &ctx.sp.surf, Rect::new(sp::CITY_TEXTURE.0, sp::CITY_TEXTURE.1, 15, 15), px + 1, py + 1, &map);
    if c.disorder {
        blit_sprite(fb, &ctx.sp.surf, sp::citizen(4), px + 5, py + 2);
    } else {
        let st = crate::gfx::TextStyle::new(0, 0);
        let x = px + if c.size < 10 { 6 } else { 3 };
        crate::gfx::text::draw_text(fb, &ctx.fonts, &st, x, py + 5, &c.size.to_string());
    }
    if !game.units_at(c.x, c.y).is_empty() {
        fb.rect_outline(Rect::new(px, py, 16, 16), 0);
    }
    if c.has_improvement(rules::improvement_ids::CITY_WALLS) {
        blit_sprite(fb, &ctx.sp.surf, Rect::new(sp::CITY_WALLS.0, sp::CITY_WALLS.1, 15, 15), px + 1, py + 1);
    }
}

/// Draw a unit sprite (with civ colours) at pixel (px,py).
pub fn draw_unit(fb: &mut Surface, ctx: &UiContext, game: &Game, id: crate::game::UnitId, px: i32, py: i32, stacked: bool) {
    let u = game.unit(id);
    let map = civ_remap(u.owner);
    let kind = if u.leader { rules::unit_ids::DIPLOMAT } else { u.kind };
    if stacked {
        // second copy offset to indicate a stack
        blit_sprite_remap(fb, &ctx.sp.surf, sp::unit(kind), px + 1, py + 1, &map);
    }
    blit_sprite_remap(fb, &ctx.sp.surf, sp::unit(kind), px, py, &map);
    // status marks as CIV.EXE 0x1b639 draws them: the fortification over a fortified unit,
    // "F" while fortifying, "G" on the human's units with a goto, and the settlers' job letter
    // in a grey frame; letters at (+4, +7) with a black shadow below, light blue on slot 1's
    // white units, white otherwise
    let letter_colour = if u.owner == 1 { 9 } else { 15 };
    let letter = |fb: &mut Surface, s: &str| {
        crate::gfx::text::draw_text(fb, &ctx.fonts, &crate::gfx::TextStyle::new(0, 0), px + 4, py + 8, s);
        crate::gfx::text::draw_text(fb, &ctx.fonts, &crate::gfx::TextStyle::new(0, letter_colour), px + 4, py + 7, s);
    };
    match u.order {
        UnitOrder::Fortified => blit_sprite(fb, &ctx.sp.surf, cell(sp::FORTIFIED.0, sp::FORTIFIED.1), px, py),
        UnitOrder::Fortify => letter(fb, "F"),
        _ => {}
    }
    if u.owner == game.human_slot && u.goto.is_some() {
        letter(fb, "G");
    }
    let job = match u.order {
        UnitOrder::Road => Some("R"),
        UnitOrder::Irrigate => Some("I"),
        UnitOrder::Mine => Some("M"),
        UnitOrder::Fortress => Some("F"),
        UnitOrder::CleanPollution => Some("P"),
        _ => None,
    };
    if let Some(j) = job.filter(|_| !u.is_air()) {
        letter(fb, j);
        fb.rect_outline(Rect::new(px - 1, py - 1, 16, 16), 7);
    }
    // a sentried unit is greyed: colours 5 and 8 become 7
    if u.order == UnitOrder::Sentry {
        for yy in py..py + 16 {
            for xx in px..px + 16 {
                if matches!(fb.get(xx, yy), 5 | 8) {
                    fb.put(xx, yy, 7);
                }
            }
        }
    }
}

/// Draw everything on map tile (x,y) for the viewing slot: terrain, city, units; black if unexplored.
pub fn draw_tile(fb: &mut Surface, ctx: &UiContext, game: &Game, slot: Slot, x: i32, y: i32, px: i32, py: i32, hide_unit: Option<crate::game::UnitId>) {
    if !Map::in_bounds(x, y) || !game.explored(slot, x, y) {
        fb.fill_rect(Rect::new(px, py, 16, 16), 0);
        return;
    }
    draw_terrain(fb, ctx, game, x, y, px, py);
    draw_fog_edges(fb, ctx, game, slot, x, y, px, py);
    if let Some(c) = game.city_at(x, y) {
        draw_city(fb, ctx, game, c, px, py);
    }
    if game.visible_to(slot, x, y) {
        let units: Vec<_> = game.units_at(x, y).into_iter().filter(|u| Some(*u) != hide_unit && game.unit(*u).carried_by.is_none() || game.unit(*u).carried_by.map(|c| game.unit_opt(c).is_none()).unwrap_or(false)).collect();
        if let Some(&top) = units.first() {
            // In a city only show units when there are no city sprites? Civ1 shows the city only.
            if game.city_at(x, y).is_none() || game.city(game.city_at(x, y).unwrap()).owner != game.unit(top).owner {
                draw_unit(fb, ctx, game, top, px, py, units.len() > 1);
            }
        }
    }
}

/// Fringe the sides of explored tile (x,y) that border unexplored tiles (off-map counts as
/// explored).
pub fn draw_fog_edges(fb: &mut Surface, ctx: &UiContext, game: &Game, slot: Slot, x: i32, y: i32, px: i32, py: i32) {
    for (side, (dx, dy)) in [(0, -1), (1, 0), (0, 1), (-1, 0)].into_iter().enumerate() {
        let (nx, ny) = (x + dx, y + dy);
        if Map::in_bounds(nx, ny) && !game.explored(slot, nx, ny) {
            blit_sprite(fb, &ctx.sp.surf, sp::fog_edge(side), px, py);
        }
    }
}

/// Draw the polar ice-cap edge on the top and bottom map rows.
pub fn draw_polar_edge(fb: &mut Surface, ctx: &UiContext, y: i32, px: i32, py: i32) {
    let sp = &ctx.sp.surf;
    if y == 0 {
        blit_sprite(fb, sp, cell(11, 8), px, py - 8);
    } else if y == rules::MAP_H as i32 - 1 {
        blit_sprite(fb, sp, cell(11, 8), px, py + 8);
    }
}
