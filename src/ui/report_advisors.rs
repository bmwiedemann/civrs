//! The ADVISORS reports F1-F6: City Status, Military Status, Intelligence, Attitude Survey,
//! Trade Report and Science Report.

use crate::assets::Palette;
use crate::gfx::{Rect, Surface};
use crate::platform::{Event, Key, MouseButton};
use crate::game::Relation;
use crate::rules::{self, SLOT_COLOURS};
use crate::ui::context::UiContext;
use crate::ui::reports::*;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::tilerender::{blit_sprite_remap, civ_remap, sp};

/// Human cities in founding order.
fn human_cities(ctx: &UiContext) -> Vec<crate::game::CityId> {
    let g = ctx.game();
    let mut v = g.cities_of(g.human_slot);
    v.sort_by_key(|c| g.city(*c).founded_turn);
    v
}

// ---- F1 City Status --------------------------------------------------------------------------

pub fn paint_city_status(fb: &mut Surface, ctx: &UiContext) {
    title_block(fb, ctx, "CITY STATUS");
    let g = ctx.game();
    let cities = human_cities(ctx);
    let pitch = if cities.len() > 20 { 7 } else { LINE_H };
    for (i, id) in cities.iter().enumerate() {
        let y = 32 + i as i32 * pitch;
        if y > 192 {
            break;
        }
        let c = g.city(*id);
        let yl = g.city_yield(*id);
        let name_col = if c.disorder { [247, 85, 85] } else { WHITE };
        text(fb, ctx, 8, y, &c.name, name_col);
        let res = format!("{}-{}{GLYPH_FOOD} {}{GLYPH_SHIELD} {}{GLYPH_TRADE}", c.size, yl.food, yl.shields, yl.trade);
        text(fb, ctx, 80, y, &res, WHITE);
        let end = text(fb, ctx, 172, y, c.production.name(), WHITE);
        text(fb, ctx, end + 4, y, &format!("({}/{})", c.shields, g.production_cost(c.id, c.production)), GREY);
    }
}

// ---- F2 Military Status ----------------------------------------------------------------------

/// F2 `military_status` (ovl 14 +0x3ad): one row per unit type the civ has counted
/// (`Player::unit_counter`, ds:0xb3be) or in production, 9 px apart from y 32: a colour-9 rule
/// at y - 2 from x 36, the icon at (2 or 18 alternately, y - 4), the name at (36, y) in 15,
/// "(att/def/moves)" at 112 in 11, "N active" at 168 in 15 and "N in production" at 232 in 11.
pub fn paint_military(fb: &mut Surface, ctx: &UiContext) {
    title_block(fb, ctx, "MILITARY STATUS");
    portrait(fb, ctx, 0);
    let g = ctx.game();
    let slot = g.human_slot;
    let counter = g.player(slot).unit_counter;
    let mut building = [0i32; 28];
    for c in g.cities_of(slot) {
        if let crate::game::Production::Unit(u) = g.city(c).production {
            building[u as usize % 28] += 1;
        }
    }
    let map = civ_remap(slot);
    let st = |c: u8| crate::gfx::TextStyle::new(FONT, c);
    let (mut y, mut row) = (32, 0);
    for k in 0..28usize {
        if counter[k] <= 0 && building[k] <= 0 {
            continue;
        }
        fb.hline(36, 319, y - 2, 9);
        blit_sprite_remap(fb, &ctx.sp.surf, sp::unit(k as u8), 2 + 16 * (row & 1), y - 4, &map);
        row += 1;
        let u = &rules::UNITS[k];
        crate::gfx::text::draw_text(fb, &ctx.fonts, &st(15), 36, y, u.name);
        crate::gfx::text::draw_text(fb, &ctx.fonts, &st(11), 112, y, &format!("({}/{}/{})", u.att, u.def, u.moves));
        if counter[k] > 0 {
            crate::gfx::text::draw_text(fb, &ctx.fonts, &st(15), 168, y, &format!("{} active", counter[k]));
        }
        if building[k] > 0 {
            crate::gfx::text::draw_text(fb, &ctx.fonts, &st(11), 232, y, &format!("{} in production", building[k]));
        }
        y += 9;
    }
}

/// F2's second page, `military_losses` (ovl 14 +0x61f): one row per unit type (0..26) any civ
/// has lost (`Player::units_lost_by_type`), 7 px apart from y 28: a colour-8 rule at y - 1 from
/// x 36, the icon, the name in 15, the human's losses at 104 in 12, then every other civ's 28 px
/// further on in its colour; a box in the human's colour around the human's column.
pub fn paint_military_losses(fb: &mut Surface, ctx: &UiContext) {
    title_block(fb, ctx, "MILITARY LOSSES");
    let g = ctx.game();
    let me = g.human_slot;
    let map = civ_remap(me);
    let (mut y, mut row) = (28, 0);
    for k in 0..27usize {
        if !(0..8u8).any(|s| g.player(s).units_lost_by_type[k] != 0) {
            continue;
        }
        fb.hline(36, 319, y - 1, 8);
        blit_sprite_remap(fb, &ctx.sp.surf, sp::unit(k as u8), 2 + 16 * (row & 1), y - 4, &map);
        row += 1;
        let text = |fb: &mut Surface, x: i32, n: i16, c: u8| {
            crate::gfx::text::draw_text(fb, &ctx.fonts, &crate::gfx::TextStyle::new(FONT, c), x, y, &n.to_string());
        };
        crate::gfx::text::draw_text(fb, &ctx.fonts, &crate::gfx::TextStyle::new(FONT, 15), 36, y, rules::UNITS[k].name);
        let mine = g.player(me).units_lost_by_type[k];
        if mine != 0 {
            text(fb, 104, mine, 12);
        }
        let mut x = 104;
        for s in 1..8u8 {
            if s == me {
                continue;
            }
            x += 28;
            let n = g.player(s).units_lost_by_type[k];
            if n != 0 {
                text(fb, x, n, SLOT_COLOURS[s as usize].light);
            }
        }
        y += 7;
    }
    fb.rect_outline(Rect::new(100, 25, 19, y - 25 + 1), SLOT_COLOURS[me as usize].light);
}

// ---- F3 Intelligence -------------------------------------------------------------------------

/// Living foreign civilisations the human has met.
fn known_civs(g: &crate::game::Game) -> Vec<crate::game::Slot> {
    g.contacts_of(g.human_slot).into_iter().filter(|s| *s != 0).collect()
}

pub fn paint_intelligence(fb: &mut Surface, ctx: &UiContext) {
    title_block(fb, ctx, "INTELLIGENCE REPORT");
    portrait(fb, ctx, 2);
    let g = ctx.game();
    let me = g.human_slot;
    let civs = known_civs(g);
    if civs.is_empty() {
        text(fb, ctx, 8, 40, "No contact with other civilizations.", WHITE);
        return;
    }
    for (i, s) in civs.iter().enumerate() {
        let y = 32 + i as i32 * 24;
        let p = g.player(*s);
        let treaty = match g.relation(me, *s) {
            Relation::War => "War",
            Relation::Peace => "Peace",
            Relation::NoContact => "No contact",
        };
        text(fb, ctx, 8, y, &format!("{} {} of the {}", p.title(), p.leader, p.plural()), WHITE);
        text(fb, ctx, 200, y, treaty, if treaty == "War" { [247, 85, 85] } else { CYAN });
        if g.human().has_embassy(*s) {
            let line = format!(
                "{}, {}{GLYPH_COIN}, {} cities, {} advances",
                rules::GOVERNMENTS[p.govt as usize].name,
                p.gold,
                g.cities_of(*s).len(),
                p.advance_count() + p.future_techs as u32
            );
            text(fb, ctx, 16, y + 8, &line, CYAN);
            let cap = p.capital.and_then(|c| g.city_opt(c)).map(|c| c.name.clone()).unwrap_or_else(|| "none".into());
            // computer players keep no research goal (CIV.EXE choose_research 0x0bb4a)
            let researching = p.researching.map(|a| format!(" Researching {}.", rules::ADVANCES[a as usize].name)).unwrap_or_default();
            text(fb, ctx, 16, y + 16, &format!("Capital: {cap}.{researching}"), CYAN);
        } else {
            text(fb, ctx, 16, y + 8, "No embassy: no further information.", GREY);
        }
    }
}

// ---- F4 Attitude Survey ----------------------------------------------------------------------

/// How civilization `s` stands towards the human, in the words of CIV.EXE's dossier (o 0x14256
/// "At PEACE" / "At WAR") and civ status screen (o 0x12329 ": Vendetta", ": Contact").
pub fn standing(g: &crate::game::Game, s: crate::game::Slot) -> String {
    let me = g.human_slot;
    let mut w = match g.relation(s, me) {
        Relation::Peace => "At PEACE".to_string(),
        Relation::War => "At WAR".to_string(),
        Relation::NoContact => "No contact".to_string(),
    };
    if g.diplo(s, me) & crate::game::diplomacy::flags::VENDETTA != 0 {
        w += ": Vendetta";
    }
    w
}

/// Attitude Survey: the empire's mood, then the known civilisations; Enter on one opens the
/// audience a Diplomat's "Meet with King" would (CIV.EXE has no such shortcut: its F4 survey,
/// o 0x14369, only lists the cities; civrs keeps it).
pub struct AttitudeSurvey {
    civs: Vec<crate::game::Slot>,
    sel: usize,
    guard: u32,
}

impl AttitudeSurvey {
    pub fn new(ctx: &UiContext) -> AttitudeSurvey {
        AttitudeSurvey { civs: known_civs(ctx.game()), sel: 0, guard: 2 }
    }

    fn row_y(i: usize) -> i32 {
        56 + i as i32 * 16
    }

    fn audience(&self, _ctx: &mut UiContext) -> Transition {
        match self.civs.get(self.sel).copied() {
            Some(slot) => {
                let g = _ctx.game();
                let at = g.player(slot).capital.and_then(|c| g.city_opt(c)).map(|c| c.pos()).unwrap_or((-1, -1));
                Transition::Push(crate::ui::diplomacy_ui::human_audience(slot, at))
            }
            None => Transition::Pop(DialogResult::Done),
        }
    }
}

impl Screen for AttitudeSurvey {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        let n = self.civs.len();
        match ev {
            Event::Key { key, down: true, .. } => match key {
                Key::Up | Key::Kp(8) if n > 0 => self.sel = (self.sel + n - 1) % n,
                Key::Down | Key::Kp(2) if n > 0 => self.sel = (self.sel + 1) % n,
                Key::Enter | Key::Space if n > 0 => return self.audience(ctx),
                Key::Unknown => {}
                _ => return Transition::Pop(DialogResult::Done),
            },
            Event::MouseMove { x, y } => {
                if (8..300).contains(x) {
                    let i = (y - Self::row_y(0) + 2).div_euclid(16);
                    if i >= 0 && (i as usize) < n {
                        self.sel = i as usize;
                    }
                }
            }
            Event::MouseButton { button: MouseButton::Left, x, y, down: true } => {
                let i = (y - Self::row_y(0) + 2).div_euclid(16);
                if (8..300).contains(x) && i >= 0 && (i as usize) < n {
                    self.sel = i as usize;
                    return self.audience(ctx);
                }
                return Transition::Pop(DialogResult::Done);
            }
            Event::MouseButton { down: true, .. } => return Transition::Pop(DialogResult::Done),
            _ => {}
        }
        Transition::None
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        page(fb, ctx, BG_LIGHT_BLUE);
        title_block(fb, ctx, "ATTITUDE SURVEY");
        let g = ctx.game();
        let me = g.human_slot;
        let (mut happy, mut content, mut unhappy) = (0i64, 0i64, 0i64);
        for c in g.cities_of(me) {
            let h = g.city_happiness(c);
            happy += h.happy as i64;
            content += h.content as i64;
            unhappy += h.unhappy as i64;
        }
        let total = (happy + content + unhappy).max(1);
        let pct = |v: i64| v * 100 / total;
        let pop = commas(g.total_population(me));
        let pop = if g.total_population(me) == 0 { "00,000".to_string() } else { pop };
        let line = format!("Population: {pop} Happy:{}% Content:{}% Unhappy:{}%", pct(happy), pct(content), pct(unhappy));
        text(fb, ctx, 16, 40, &line, WHITE);
        if self.civs.is_empty() {
            return;
        }
        for (i, s) in self.civs.iter().enumerate() {
            let y = Self::row_y(i);
            let p = g.player(*s);
            if i == self.sel {
                fb.fill_rect(Rect::new(8, y - 2, 292, 16), col(ctx, BG_BLUE));
            }
            let hostile = !g.treaty(*s, me);
            text(fb, ctx, 16, y, &format!("{} {} of the {}", p.title(), p.leader, p.plural()), WHITE);
            let line = format!("{}{}", standing(g, *s), if g.human().has_embassy(*s) { " (embassy)" } else { "" });
            text(fb, ctx, 24, y + 8, &line, if hostile { [247, 85, 85] } else { CYAN });
        }
        tiny(fb, ctx, 8, 192, "SELECT A CIVILIZATION AND PRESS ENTER FOR AN AUDIENCE.", WHITE);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.game_palette()
    }
    fn draws_parent(&self) -> bool {
        false
    }
}

// ---- F5 Trade Report ---------------------------------------------------------------------------

/// F5 `trade_report` (ovl 14 +0x7f1): "City Trade" at (8, 32) in 15; from y 40, 8 px apart,
/// each city's name (fitted to 62 px) shadowed at (16, y) and " lux\\/tax$/sci~" at (80, y) in
/// 10, or " (Disorder)" in 12 (then it adds nothing to the totals); a page holds rows above
/// y 180. Then "Total Income: N$" at (8, y + 4), "Discoveries: N turns" at (8, y + 12) (the
/// research cost over the science, x1.5 with SETI), and in the right column "Maintenance Costs"
/// at (160, 32) with "<count> <improvement>, <cost>$" rows in 14 from y 40 and "Total Cost".
pub struct TradeReport {
    rows: Vec<(String, String, bool)>,
    page: usize,
}

const TRADE_ROWS: usize = 18;

pub fn trade_report(ctx: &UiContext) -> Box<dyn Screen> {
    let g = ctx.game();
    let rows = human_cities(ctx)
        .iter()
        .map(|id| {
            let c = g.city(*id);
            let name = fit_text(ctx, &c.name, 62);
            if c.disorder {
                (name, " \u{8c}(Disorder)".to_string(), true)
            } else {
                let y = g.city_yield(*id);
                (name, format!(" {}\\/{}$/{}~", y.lux, y.tax, y.sci), false)
            }
        })
        .collect();
    Box::new(TradeReport { rows, page: 0 })
}

impl TradeReport {
    fn pages(&self) -> usize {
        self.rows.len().div_ceil(TRADE_ROWS).max(1)
    }
}

impl Screen for TradeReport {
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => {
                self.page += 1;
                if self.page >= self.pages() {
                    Transition::Pop(DialogResult::Done)
                } else {
                    Transition::None
                }
            }
            _ => Transition::None,
        }
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let g = ctx.game();
        fb.fill(2);
        title_block(fb, ctx, "TRADE REPORT");
        let st = |c: u8| crate::gfx::TextStyle::new(FONT, c);
        let draw = |fb: &mut Surface, x: i32, y: i32, c: u8, s: &str| crate::ui::widgets::draw_coded_text(fb, ctx, FONT, x, y, s, c);
        if self.page == 0 {
            portrait(fb, ctx, 1);
            crate::gfx::text::draw_text(fb, &ctx.fonts, &st(15), 8, 32, "City Trade");
        }
        let mut y = 40;
        for (name, line, _) in self.rows.iter().skip(self.page * TRADE_ROWS).take(TRADE_ROWS) {
            crate::gfx::text::draw_text(fb, &ctx.fonts, &st(0), 16, y + 1, name);
            crate::gfx::text::draw_text(fb, &ctx.fonts, &st(15), 16, y, name);
            draw(fb, 80, y, 10, line);
            y += 8;
        }
        if self.page + 1 < self.pages() {
            return;
        }
        // the totals, the discovery time and the maintenance column close the last page
        let me = g.human_slot;
        let (mut tax, mut sci) = (0, 0);
        for id in human_cities(ctx) {
            if !g.city(id).disorder {
                let yl = g.city_yield(id);
                tax += yl.tax;
                sci += yl.sci;
            }
        }
        crate::gfx::text::draw_text(fb, &ctx.fonts, &st(10), 8, y + 4, &format!("Total Income: {tax}$"));
        if g.owner_has_wonder(me, rules::wonder_ids::SETI_PROGRAM) {
            sci += sci / 2;
        }
        if sci != 0 && y < 180 {
            let era = if g.year() < 0 { 1 } else if g.year() < 1750 { 2 } else { 3 };
            let turns = era * g.player(me).research_count as i32 * (2 * g.setup.difficulty as i32 + 6) / sci;
            crate::gfx::text::draw_text(fb, &ctx.fonts, &st(10), 8, y + 12, &format!("Discoveries: {turns} turns"));
        }
        crate::gfx::text::draw_text(fb, &ctx.fonts, &st(15), 160, 32, "Maintenance Costs");
        let mut costs = [0i32; 25];
        let mut count = [0i32; 25];
        for id in human_cities(ctx) {
            for k in 2..=24u8 {
                if g.city(id).has_improvement(k) {
                    costs[k as usize] += g.improvement_upkeep(me, k);
                    count[k as usize] += 1;
                }
            }
        }
        let (mut y, mut total) = (40, 0);
        for k in 2..=24usize {
            if costs[k] == 0 {
                continue;
            }
            if y < 180 {
                let line = format!("{} {}, {}$", count[k], rules::IMPROVEMENTS[k].name, costs[k]);
                crate::gfx::text::draw_text(fb, &ctx.fonts, &st(14), 160, y, &line);
                y += 8;
            }
            total += costs[k];
        }
        crate::gfx::text::draw_text(fb, &ctx.fonts, &st(14), 160, y + 4, &format!("Total Cost: {total}$"));
    }

    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.game_palette()
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

// ---- F6 Science Report -------------------------------------------------------------------------

pub fn paint_science(fb: &mut Surface, ctx: &UiContext) {
    title_block(fb, ctx, "SCIENCE REPORT");
    let g = ctx.game();
    let p = g.human();
    // research box (CIV.EXE overlay 0x12ec0): one bulb per beaker collected, `step` eighths of a
    // pixel apart, in a box as wide as the advance costs
    let cost = g.research_cost(g.human_slot);
    let step = (2432 / (cost + 1)).clamp(1, 64);
    let w = (step * cost / 8 + 8).clamp(8, 304);
    let bx = 160 - w / 2;
    fb.fill_rect(Rect::new(bx, 25, w, 16), col(ctx, BG_LIGHT_BLUE));
    // the portrait is redrawn over a box that reaches it
    if bx > 40 {
        portrait(fb, ctx, 3);
    }
    let bulb = Rect::new(129, 41, 7, 7);
    let mut acc = 0;
    for _ in 0..p.beakers.clamp(0, cost) {
        fb.blit_part(&ctx.sp.surf, bulb, bx + acc / 8, 33, Some(0));
        acc += step;
    }
    if let Some(a) = p.researching {
        let line = format!("Researching {}", rules::ADVANCES[a as usize].name);
        text_c(fb, ctx, 161, 26, &line, BLACK);
        text_c(fb, ctx, 160, 26, &line, WHITE);
    }
    // known advances in index order, three per row
    let mut known: Vec<&str> = p.known_advances().iter().map(|a| rules::ADVANCES[*a as usize].name).collect();
    if p.future_techs > 0 {
        known.push("Future Tech.");
    }
    let mut y = 42;
    for (i, name) in known.iter().enumerate() {
        text(fb, ctx, 8 + (i % 3) as i32 * 100, y, name, CYAN);
        if i % 3 == 2 {
            y += 7;
            if y > 192 {
                break;
            }
        }
    }
}
