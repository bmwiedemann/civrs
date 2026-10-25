//! The ADVISORS reports F1-F6: City Status, Military Status, Intelligence, Attitude Survey,
//! Trade Report and Science Report.

use crate::assets::Palette;
use crate::gfx::{Rect, Surface};
use crate::platform::{Event, Key};
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

/// F3 `intelligence_report()` (ovl 14 +0xd43, o 0x13ab8) on a colour-1 page: per civ in the
/// game, 24 px apart from y 32, a colour-9 line (4..316, y - 2) and "<civ>: <leader>" at (8, y)
/// in 15 with a shadow. The human and civs with an embassy add government, treasury and (on
/// Chieftain and Warlord) the units at (160, y) in the civ's colour, then from (16, y + 8) its
/// standing with each civ in 7, 96 px apart, wrapping past x 240: ALLIED, PEACE or WAR (contact
/// without peace). Other civs get "No embassy established." at (96, y) in their colour (CIV.EXE
/// draws no name there). Foreign rows with an embassy carry an "INFO<n>" button at
/// (281, 24n + 20)-(318, 24n + 29); 1..7 or a click right of x 281 on row (y - 28) / 24 opens
/// [`Dossier`] for that row, any other key or click closes the report.
pub struct Intelligence {
    /// Row -> civ for the rows that show full information.
    rows: Vec<Option<crate::game::Slot>>,
    guard: u32,
}

impl Intelligence {
    pub fn new(ctx: &UiContext) -> Intelligence {
        let g = ctx.game();
        let me = g.human_slot;
        let rows = g.alive_slots().into_iter().map(|s| (s == me || g.human().has_embassy(s)).then_some(s)).collect();
        Intelligence { rows, guard: 2 }
    }

    fn open(&self, row: i32, ctx: &UiContext) -> Transition {
        let me = ctx.game().human_slot;
        match usize::try_from(row).ok().and_then(|r| self.rows.get(r).copied().flatten()) {
            Some(s) if s != me => Transition::Push(Box::new(Dossier::new(s))),
            _ => Transition::Pop(DialogResult::Done),
        }
    }
}

impl Screen for Intelligence {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }
    fn resume(&mut self, _result: DialogResult, _ctx: &mut UiContext) -> Transition {
        self.guard = 2;
        Transition::None
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        match ev {
            Event::Key { down: true, key: Key::Char(c @ '1'..='7'), .. } => self.open(*c as i32 - '1' as i32, ctx),
            Event::Key { down: true, key, .. } if !matches!(key, Key::Unknown) => Transition::Pop(DialogResult::Done),
            // C division truncates: rows reach up from y 5
            Event::MouseButton { down: true, x, y, .. } if *x > 281 => self.open((*y - 28) / 24, ctx),
            Event::MouseButton { down: true, .. } => Transition::Pop(DialogResult::Done),
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        use crate::game::diplomacy::flags;
        use crate::ui::widgets::draw_coded_text;
        fb.fill(1);
        title_block(fb, ctx, "INTELLIGENCE REPORT");
        let g = ctx.game();
        let me = g.human_slot;
        let line = |fb: &mut Surface, x: i32, y: i32, t: &str, c: u8| draw_coded_text(fb, ctx, FONT, x, y, t, c);
        for (n, s) in g.alive_slots().into_iter().enumerate() {
            let mut y = 32 + 24 * n as i32;
            fb.hline(4, 316, y - 2, 9);
            let p = g.player(s);
            let colour = SLOT_COLOURS[s as usize].light;
            let Some(_) = self.rows.get(n).copied().flatten() else {
                line(fb, 96, y, "No embassy established.", colour);
                continue;
            };
            let name = format!("{}: {}", p.plural(), p.leader);
            line(fb, 8, y + 1, &name, 0);
            line(fb, 8, y, &name, 15);
            let mut info = format!("{}, {}$", rules::GOVERNMENTS[p.govt as usize].adjective, p.gold);
            if g.difficulty() as u8 <= 1 {
                info += &format!(", {} Units.", g.units_of(s).len());
            }
            line(fb, 160, y, &info, colour);
            y += 8;
            let mut x = 16;
            for o in 1..8u8 {
                if x > 240 {
                    x = 16;
                    y += 8;
                }
                let f = p.flags(o);
                let state = if f & flags::PEACE != 0 {
                    if f & flags::ALLIANCE != 0 { ":\u{8f} ALLIED" } else { ":\u{8f} PEACE" }
                } else if f & (flags::CONTACT | flags::PEACE) == flags::CONTACT {
                    ":\u{8c} WAR"
                } else {
                    continue;
                };
                line(fb, x, y, &format!("{}{state}", g.player(o).plural()), 7);
                x += 96;
            }
            if s != me {
                let r = 24 * (n as i32 + 1);
                draw_button(fb, ctx, Rect::new(281, r + 20, 38, 10), &format!("INFO{}", n + 1), colour);
            }
        }
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.game_palette()
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        true
    }
}

/// `draw_button(x1, y1, x2, y2, text, col)` (0xc91:0x71b5) in the report font, `r` covering
/// x1..=x2, y1..=y2: fill up to x2-1/y2-1, top and left lines in col+8 (7 from col 8 up),
/// bottom and right in 8 (col-8), the label in col^8 centred on ((x1+x2)/2+1, (y1+y2)/2-2).
fn draw_button(fb: &mut Surface, ctx: &UiContext, r: Rect, label: &str, col: u8) {
    let (light, dark) = if col < 8 { (col + 8, 8) } else { (7, col - 8) };
    let (x2, y2) = (r.right() - 1, r.bottom() - 1);
    fb.fill_rect(Rect::new(r.x, r.y, r.w - 1, r.h - 1), col);
    fb.hline(r.x, x2, r.y, light);
    fb.vline(r.x, r.y, y2, light);
    fb.hline(r.x + 1, x2, y2, dark);
    fb.vline(x2, r.y, y2, dark);
    let w = crate::gfx::text::text_width(&ctx.fonts, FONT, label);
    crate::ui::widgets::draw_coded_text(fb, ctx, FONT, (r.x + x2) / 2 + 1 - w / 2, (r.y + y2) / 2 - 2, label, col ^ 8);
}

/// `civ_dossier(civ)` (ovl 14 +0x1164, o 0x13ed9) on a colour-1 page under the human's title
/// block: "Subject: the <civ>" in 15 with a shadow at (16, 32); from y 44 in colour 9 at x 16,
/// 8 px apart, the leader, the leader's traits in 7 at x 24, then after 4 px the capital (the
/// last city with a Palace), government, treasury and, on Chieftain and Warlord, the units;
/// "Foreign Affairs:" with "At PEACE"/"At WAR with the <civ>." per contact, and after 4 px
/// "Technologies:" newest first until y 192. Shown by an embassy and "Check Intelligence".
pub struct Dossier {
    civ: crate::game::Slot,
    guard: u32,
}

impl Dossier {
    pub fn new(civ: crate::game::Slot) -> Dossier {
        Dossier { civ, guard: 2 }
    }
}

impl Screen for Dossier {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } if self.guard == 0 => Transition::Pop(DialogResult::Done),
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        use crate::ui::widgets::draw_coded_text;
        fb.fill(1);
        title_block(fb, ctx, "INTELLIGENCE REPORT");
        let g = ctx.game();
        let s = self.civ;
        let p = g.player(s);
        let line = |fb: &mut Surface, x: i32, y: i32, t: &str, c: u8| draw_coded_text(fb, ctx, FONT, x, y, t, c);
        let subject = format!("Subject: the {}", p.plural());
        line(fb, 16, 33, &subject, 0);
        line(fb, 16, 32, &subject, 15);
        let mut y = 44;
        line(fb, 16, y, &format!("Leader:\u{8f} {} {}", p.title(), p.leader), 9);
        y += 8;
        let civ = p.civ();
        for (v, pos, neg) in [(civ.aggressive, "Aggressive", "Friendly"), (civ.expansionist, "Expansionistic", "Perfectionist"), (civ.civilized, "Civilized", "Militaristic")] {
            if v != 0 {
                line(fb, 24, y, if v > 0 { pos } else { neg }, 7);
                y += 8;
            }
        }
        y += 4;
        let capital = g.all_cities().filter(|c| c.owner == s && c.has_improvement(rules::improvement_ids::PALACE)).last().map_or("NONE".to_string(), |c| c.name.clone());
        line(fb, 16, y, &format!("Capital:\u{8f} {capital}"), 9);
        y += 8;
        line(fb, 16, y, &format!("Government:\u{8f} {}", rules::GOVERNMENTS[p.govt as usize].adjective), 9);
        y += 8;
        line(fb, 16, y, &format!("Treasury:\u{8f} {}$", p.gold), 9);
        y += 8;
        if g.difficulty() as u8 <= 1 {
            line(fb, 16, y, &format!("Military:\u{8f} {} Units.", g.units_of(s).len()), 9);
            y += 12;
        }
        line(fb, 16, y, "Foreign Affairs:", 9);
        y += 8;
        for o in 1..8u8 {
            let f = p.flags(o);
            if f & crate::game::diplomacy::flags::CONTACT == 0 {
                continue;
            }
            let state = if f & crate::game::diplomacy::flags::PEACE != 0 { "At\u{8f} PEACE" } else { "At\u{8c} WAR" };
            line(fb, 24, y, &format!("{state}\u{87} with the {}.", g.player(o).plural()), 7);
            y += 8;
        }
        y += 4;
        line(fb, 16, y, "Technologies:", 9);
        y += 8;
        for a in (0..72u8).rev() {
            if p.knows(a) {
                line(fb, 24, y, rules::ADVANCES[a as usize].name, 7);
                y += 8;
                if y >= 192 {
                    break;
                }
            }
        }
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

/// F4 `attitude_survey` (ovl 14 +0x15f4) on a colour-9 page: per city, 10 px apart from y 32,
/// "<city>:" at (16, y) in 15, its citizens from x 80 + (4 * row & 7) at y - 3 (124 px wide),
/// a cyan box (212, y - 1, 90, 10) with the pictograms of its Temple, MarketPlace, Bank,
/// Cathedral and Colosseum (mask 0x2618) 18 px apart; rows past y 190 go on the next page.
/// Then "Population: <n> Happy:x% Content:y% Unhappy:z%" at (16, y + 8), shares of the total
/// size (1 when there are no citizens, so all is content).
pub struct AttitudeSurvey {
    cities: Vec<crate::game::CityId>,
    page: usize,
}

const ATTITUDE_ROWS: usize = 16;

impl AttitudeSurvey {
    pub fn new(ctx: &UiContext) -> AttitudeSurvey {
        AttitudeSurvey { cities: human_cities(ctx), page: 0 }
    }
    fn pages(&self) -> usize {
        self.cities.len().div_ceil(ATTITUDE_ROWS).max(1)
    }
}

impl Screen for AttitudeSurvey {
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
        fb.fill(9);
        title_block(fb, ctx, "ATTITUDE SURVEY");
        let g = ctx.game();
        let st = crate::gfx::TextStyle::new(FONT, 15);
        let mut y = 32;
        for (i, id) in self.cities.iter().enumerate().skip(self.page * ATTITUDE_ROWS).take(ATTITUDE_ROWS) {
            let c = g.city(*id);
            crate::gfx::text::draw_text(fb, &ctx.fonts, &st, 16, y, &format!("{}:", c.name));
            crate::ui::cityscreen::draw_citizens(fb, ctx, *id, 80 + ((4 * i as i32) & 7), y - 3, 124);
            fb.fill_rect(Rect::new(212, y - 1, 90, 10), 11);
            let mut x = 212;
            for k in [4u8, 5, 10, 11, 14] {
                if c.has_improvement(k) {
                    if let Some(cell) = crate::ui::city_draw::improvement_cell(k) {
                        crate::ui::city_draw::imp_icon(fb, ctx, cell, x, y - 1);
                    }
                    x += 18;
                }
            }
            y += 10;
        }
        if self.page + 1 < self.pages() {
            return;
        }
        let me = g.human_slot;
        let (mut size, mut happy, mut unhappy) = (0i32, 0i32, 0i32);
        for id in &self.cities {
            let h = g.city_happiness(*id);
            size += g.city(*id).size as i32;
            happy += h.happy;
            unhappy += h.unhappy;
        }
        let size = size.max(1);
        // append_percent (0x14592): 100 * part / total, or 10 * part / (total / 10) from 300 on
        let pct = |part: i32| if size < 300 { 100 * part / size } else { 10 * part / (size / 10) };
        let pop = if g.total_population(me) == 0 { "00,000".to_string() } else { commas(g.total_population(me)) };
        let line = format!("Population: {pop} Happy:{}% Content:{}% Unhappy:{}%", pct(happy), pct(size - happy - unhappy), pct(unhappy));
        crate::gfx::text::draw_text(fb, &ctx.fonts, &st, 16, y + 8, &line);
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
