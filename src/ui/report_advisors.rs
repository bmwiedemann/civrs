//! The ADVISORS reports F1-F6: City Status, Military Status, Intelligence, Attitude Survey,
//! Trade Report and Science Report.

use crate::assets::Palette;
use crate::gfx::{Rect, Surface};
use crate::platform::{Event, Key, MouseButton};
use crate::game::Relation;
use crate::rules;
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
        text(fb, ctx, end + 4, y, &format!("({}/{})", c.shields, c.production.cost()), GREY);
    }
}

// ---- F2 Military Status ----------------------------------------------------------------------

pub fn paint_military(fb: &mut Surface, ctx: &UiContext) {
    title_block(fb, ctx, "MILITARY STATUS");
    portrait(fb, ctx, 0);
    let g = ctx.game();
    let slot = g.human_slot;
    // units by type, in unit-table order
    let mut counts = [0u32; 28];
    let mut in_city = [0u32; 28];
    for u in g.all_units().filter(|u| u.owner == slot) {
        let k = u.kind as usize % 28;
        counts[k] += 1;
        if g.city_at(u.x, u.y).is_some() {
            in_city[k] += 1;
        }
    }
    let kinds: Vec<usize> = (0..28).filter(|k| counts[*k] > 0).collect();
    let pitch = if kinds.len() > 18 { 16 } else { 18 };
    let rows = ((190 - 32) / pitch).max(1) as usize;
    let map = civ_remap(slot);
    for (i, k) in kinds.iter().enumerate() {
        let colx = (i / rows) as i32;
        let row = (i % rows) as i32;
        if colx > 1 {
            break;
        }
        let x = 8 + colx * 144;
        let y = 32 + row * pitch;
        blit_sprite_remap(fb, &ctx.sp.surf, sp::unit(*k as u8), x, y - 1, &map);
        text(fb, ctx, x + 20, y + 1, rules::UNITS[*k].name, WHITE);
        let detail = if in_city[*k] > 0 { format!("x{}  ({} in cities)", counts[*k], in_city[*k]) } else { format!("x{}", counts[*k]) };
        tiny(fb, ctx, x + 20, y + 9, &detail, CYAN);
    }
    let total: u32 = counts.iter().sum();
    if total == 0 {
        text(fb, ctx, 8, 40, "No units.", WHITE);
    } else {
        let active = g.cities_of(slot).iter().map(|c| g.supported_units(*c).len() as u32).sum::<u32>();
        text(fb, ctx, 8, 192, &format!("{total} units, {active} supported by cities"), CYAN);
    }
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
            let researching = p.researching.map(|a| rules::ADVANCES[a as usize].name).unwrap_or("nothing");
            text(fb, ctx, 16, y + 16, &format!("Capital: {cap}. Researching {researching}."), CYAN);
        } else {
            text(fb, ctx, 16, y + 8, "No embassy: no further information.", GREY);
        }
    }
}

// ---- F4 Attitude Survey ----------------------------------------------------------------------

/// Civ1 attitude words, from best to worst.
pub fn attitude_word(a: i32) -> &'static str {
    match a {
        80..=i32::MAX => "Worshipful",
        60..=79 => "Enthusiastic",
        40..=59 => "Cordial",
        20..=39 => "Receptive",
        -19..=19 => "Neutral",
        -39..=-20 => "Uncooperative",
        -59..=-40 => "Icy",
        -79..=-60 => "Belligerent",
        _ => "Hostile",
    }
}

/// Attitude Survey: the empire's mood, then the known civilisations; Enter on one asks for an
/// audience.
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
            Some(slot) => Transition::Push(crate::ui::diplomacy_ui::human_audience(slot)),
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
            let treaty = match g.relation(me, *s) {
                Relation::War => "at WAR",
                _ => "at peace",
            };
            let att = crate::ai::diplo_ai::attitude(g, *s, me);
            text(fb, ctx, 16, y, &format!("{} {} of the {}", p.title(), p.leader, p.plural()), WHITE);
            let line = format!("{} towards us, {treaty}{}", attitude_word(att), if g.human().has_embassy(*s) { " (embassy)" } else { "" });
            text(fb, ctx, 24, y + 8, &line, if att < -19 { [247, 85, 85] } else { CYAN });
        }
        tiny(fb, ctx, 8, 192, "SELECT A CIVILIZATION AND PRESS ENTER FOR AN AUDIENCE.", WHITE);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.sp.pal
    }
    fn draws_parent(&self) -> bool {
        false
    }
}

// ---- F5 Trade Report ---------------------------------------------------------------------------

pub fn paint_trade(fb: &mut Surface, ctx: &UiContext) {
    title_block(fb, ctx, "TRADE REPORT");
    portrait(fb, ctx, 1);
    let g = ctx.game();
    let cities = human_cities(ctx);
    text(fb, ctx, 8, 33, "City Trade", WHITE);
    let pitch = if cities.len() > 17 { 7 } else { LINE_H };
    let (mut income, mut routes) = (0, 0);
    let mut y = 40;
    for id in &cities {
        let c = g.city(*id);
        let yl = g.city_yield(*id);
        income += yl.tax;
        routes += c.trade_routes.len();
        if y <= 176 {
            text_sh(fb, ctx, 16, y, &c.name, WHITE, BLACK);
            let line = format!("{}{GLYPH_LUX}/{}{GLYPH_COIN}/{}{GLYPH_BULB}", yl.lux, yl.tax, yl.sci);
            text(fb, ctx, 86, y, &line, LIGHT_GREEN);
            if !c.trade_routes.is_empty() {
                tiny(fb, ctx, 132, y + 1, &format!("{}RT", c.trade_routes.len()), LIGHT_GREEN);
            }
            y += pitch;
        }
    }
    let y = y + 4;
    text(fb, ctx, 8, y, &format!("Total Income: {income}{GLYPH_COIN}"), LIGHT_GREEN);
    if routes > 0 {
        text(fb, ctx, 8, y + 8, &format!("Trade routes: {routes}"), LIGHT_GREEN);
    }
    // maintenance by improvement type
    text(fb, ctx, 160, 33, "Maintenance Costs", WHITE);
    let mut counts = [0i32; 25];
    for id in &cities {
        for imp in g.city(*id).improvement_list() {
            counts[imp as usize] += 1;
        }
    }
    let mut y = 41;
    let mut total = 0;
    for (imp, n) in counts.iter().enumerate().filter(|(_, n)| **n > 0) {
        let info = &rules::IMPROVEMENTS[imp];
        let cost = *n * info.upkeep as i32;
        total += cost;
        if y <= 176 && info.upkeep > 0 {
            text(fb, ctx, 160, y, &format!("{} x{n}", info.name), PALE_YELLOW);
            text_r(fb, ctx, 276, y, &format!("{cost}{GLYPH_COIN}"), PALE_YELLOW);
            y += LINE_H;
        }
    }
    let y = if y == 41 { 45 } else { y + 4 };
    text(fb, ctx, 160, y, &format!("Total Cost: {total}{GLYPH_COIN}"), PALE_YELLOW);
}

// ---- F6 Science Report -------------------------------------------------------------------------

pub fn paint_science(fb: &mut Surface, ctx: &UiContext) {
    title_block(fb, ctx, "SCIENCE REPORT");
    portrait(fb, ctx, 3);
    let g = ctx.game();
    let p = g.human();
    // current research box
    let (line1, line2) = match p.researching {
        Some(a) => {
            let cost = g.research_cost(g.human_slot);
            let per_turn = g.science_income(g.human_slot).max(0);
            let left = (cost - p.beakers).max(0);
            let turns = if per_turn > 0 { (left + per_turn - 1) / per_turn } else { 0 };
            let eta = if per_turn > 0 { format!("{turns} turns") } else { "never".to_string() };
            (format!("Researching: {}", rules::ADVANCES[a as usize].name), format!("{}/{cost}{GLYPH_BULB} ({eta})", p.beakers))
        }
        None => ("Researching: nothing".to_string(), format!("{}{GLYPH_BULB} saved", p.beakers)),
    };
    let w = width(ctx, &line1).max(width(ctx, &line2)).max(80) + 8;
    let bx = (160 - w / 2).clamp(8, 270 - w);
    fb.fill_rect(Rect::new(bx, 25, w, 16), col(ctx, BG_LIGHT_BLUE));
    text(fb, ctx, bx + 4, 26, &line1, WHITE);
    text(fb, ctx, bx + 4, 34, &line2, WHITE);
    // known advances in columns
    let mut known: Vec<&str> = p.known_advances().iter().map(|a| rules::ADVANCES[*a as usize].name).collect();
    known.sort_unstable();
    if p.future_techs > 0 {
        known.push("Future Tech.");
    }
    if known.is_empty() {
        text(fb, ctx, 8, 43, "No advances yet.", CYAN);
        return;
    }
    let (rows, cols, pitch, colw, small) = if known.len() <= 57 { (19, 3, 8, 100, false) } else { (21, 4, 7, 78, true) };
    for (i, name) in known.iter().enumerate() {
        let c = i / rows;
        if c >= cols {
            break;
        }
        let x = 8 + c as i32 * colw;
        let y = 43 + (i % rows) as i32 * pitch;
        if small {
            tiny(fb, ctx, x, y, name, CYAN);
        } else {
            text(fb, ctx, x, y, name, CYAN);
        }
    }
    if p.future_techs > 1 {
        text(fb, ctx, 200, 192, &format!("Future Techs: {}", p.future_techs), CYAN);
    }
}
