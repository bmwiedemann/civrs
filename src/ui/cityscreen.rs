//! The city screen, laid out pixel for pixel after the original (see the measurements in
//! the reverse-engineering report): title and citizens, CITY RESOURCES, unit support, the 5x5
//! city map, improvements with sell dots, FOOD STORAGE, the INFO / HAPPY / MAP / VIEW tabs,
//! the production box with CHANGE / BUY and the RENAME / EXIT buttons.

use crate::game::{CityId, Production, RuleError, Specialist, UnitOrder};
use crate::gfx::text::wrap;
use crate::gfx::{Rect, Surface};
use crate::platform::{Event, Key, MouseButton};
use crate::rules::{self, improvement_ids};
use crate::ui::city_draw::*;
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::tilerender::{draw_city, draw_fog_edges, draw_terrain};
use crate::ui::widgets::{draw_styled_panel, MenuDialog, MessageBox, PanelStyle, TextInput, YesNo};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Info,
    Happy,
    Map,
}

enum Pending {
    None,
    Change(Vec<Production>),
    Buy,
    Sell(u8),
    Rename,
    Ignore,
}

pub struct CityScreen {
    pub city: CityId,
    tab: Tab,
    pending: Pending,
    /// Advisor speech texts shown under the "What shall we build" menu.
    advisors: Option<(String, String)>,
    /// Action performed on the first frame (headless renders of the sub-views / dialogs).
    start: Option<String>,
}

pub fn city_screen(c: CityId) -> Box<dyn Screen> {
    Box::new(CityScreen { city: c, tab: Tab::Info, pending: Pending::None, advisors: None, start: None })
}

/// City screen that opens on a tab or dialog: "happy", "map", "view", "change", "buy", "rename", "sell".
pub fn city_screen_at(c: CityId, start: &str) -> Box<dyn Screen> {
    Box::new(CityScreen { city: c, tab: Tab::Info, pending: Pending::None, advisors: None, start: Some(start.to_string()) })
}

// ---- layout (coordinates include the 1-px borders) ---------------------------------------

const P_TITLE: Rect = Rect::new(2, 1, 207, 21);
const P_RES: Rect = Rect::new(2, 23, 123, 43);
const P_SUPPORT: Rect = Rect::new(2, 67, 123, 38);
const P_MAP: Rect = Rect::new(127, 23, 82, 82);
const P_IMP: Rect = Rect::new(211, 1, 107, 97);
const P_FOOD: Rect = Rect::new(2, 106, 91, 92);
/// Dark interior of the FOOD STORAGE panel; the stippled storage box sits at its left edge,
/// (size+1) icons wide and ten rows high (a size-1 city: (3,115)-(19,196)).
const FOOD_AREA: Rect = Rect::new(3, 114, 90, 84);
const FOOD_ROWS: i32 = 10;
const P_TABS: Rect = Rect::new(95, 105, 133, 93);
const TAB_RECTS: [Rect; 4] = [Rect::new(95, 106, 33, 8), Rect::new(129, 106, 31, 8), Rect::new(161, 106, 32, 8), Rect::new(194, 106, 32, 8)];
const TAB_LABELS: [&str; 4] = ["INFO", "HAPPY", "MAP", "VIEW"];
const B_CHANGE: Rect = Rect::new(231, 106, 32, 8);
const B_BUY: Rect = Rect::new(294, 106, 18, 8);
const ITEM_FRAME: (i32, i32) = (264, 100);
const B_RENAME: Rect = Rect::new(231, 190, 42, 9);
const B_EXIT: Rect = Rect::new(284, 190, 33, 9);
const MAP_ORIGIN: (i32, i32) = (128, 24);
const CITIZEN_X: i32 = 10;
const CITIZEN_Y: i32 = 8;
const MINIMAP: Rect = Rect::new(101, 116, 120, 76);

/// Shield-stock rows (one per 10 shields of cost), their pitch and the production panel.
fn stock_layout(cost: i32) -> (i32, i32, Rect) {
    let rows = (cost / 10).max(1);
    let pitch = (72 / rows).clamp(1, 8);
    let bottom = 115 + rows * pitch + 1;
    (rows, pitch, Rect::new(230, 98, 88, bottom - 98 + 1))
}

fn order_abbrev(o: UnitOrder) -> &'static str {
    match o {
        UnitOrder::None => "NON.",
        UnitOrder::Fortify | UnitOrder::Fortified => "FOR.",
        UnitOrder::Sentry => "SEN.",
        UnitOrder::Goto => "GOTO",
        UnitOrder::Road => "ROAD",
        UnitOrder::Irrigate => "IRR.",
        UnitOrder::Mine => "MINE",
        UnitOrder::Fortress => "FTR.",
        UnitOrder::CleanPollution => "POL.",
        UnitOrder::Pillage => "PIL.",
        _ => "SKIP",
    }
}

/// Advisor text for an item from PRODUCE.TXT (shared sections fall through to the next
/// non-empty one), with `$RPLC1` replaced and the `^` line breaks removed.
fn produce_text(ctx: &UiContext, name: &str) -> Option<String> {
    let names: Vec<String> = ctx.texts.produce.names().map(|s| s.to_string()).collect();
    let start = names.iter().position(|n| n.eq_ignore_ascii_case(name))?;
    for n in &names[start..] {
        if let Some(lines) = ctx.texts.produce.section(n) {
            if !lines.is_empty() {
                let text = lines.join(" ").replace("$RPLC1", name).to_ascii_uppercase();
                let text = text.split('^').map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n");
                return Some(text);
            }
        }
    }
    None
}

fn error_box(ctx: &UiContext, e: &RuleError) -> Option<Box<dyn Screen>> {
    if let Some(sec) = e.section() {
        if let Some(lines) = ctx.texts.error.section(sec) {
            return Some(Box::new(MessageBox::new(crate::assets::text::paragraphs(lines))));
        }
    }
    let m = e.message();
    if m.is_empty() {
        None
    } else {
        Some(Box::new(MessageBox::text(m)))
    }
}

impl CityScreen {
    fn error(&mut self, ctx: &UiContext, e: &RuleError) -> Transition {
        match error_box(ctx, e) {
            Some(s) => {
                self.pending = Pending::Ignore;
                Transition::Push(s)
            }
            None => Transition::None,
        }
    }

    /// Units shown in the INFO view: supported by the city first, then the others present.
    fn info_units(&self, ctx: &UiContext) -> Vec<crate::game::UnitId> {
        let g = ctx.game();
        let c = g.city(self.city);
        let mut v = g.supported_units(self.city);
        for id in g.units_at(c.x, c.y) {
            if !v.contains(&id) && g.unit(id).owner == c.owner {
                v.push(id);
            }
        }
        v
    }

    /// Improvements then wonders of the city, as listed in the improvements panel.
    fn listed(&self, ctx: &UiContext) -> Vec<Production> {
        let c = ctx.game().city(self.city);
        let mut v: Vec<Production> = c.improvement_list().into_iter().map(Production::Improvement).collect();
        v.extend(c.wonder_list().into_iter().map(Production::Wonder));
        v
    }

    fn citizen_pitch(n: usize) -> i32 {
        if n == 0 {
            7
        } else {
            (186 / n as i32).clamp(2, 7)
        }
    }

    // ---- actions ---------------------------------------------------------------------------

    fn change(&mut self, ctx: &mut UiContext) -> Transition {
        let g = ctx.game();
        let c = g.city(self.city);
        let y = g.city_yield(self.city);
        let surplus = y.shield_surplus().max(1);
        let list = g.available_production(self.city);
        let mut sel = 0;
        let items: Vec<String> = list
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if *p == c.production {
                    sel = i;
                }
                let remaining = (p.cost() - if *p == c.production { c.shields } else { 0 }).max(0);
                let turns = ((remaining + surplus - 1) / surplus).max(1);
                match p {
                    Production::Unit(k) => {
                        let u = &rules::UNITS[*k as usize];
                        format!("{} ({} turns, ADM:{}/{}/{})", p.name(), turns, u.att, u.def, u.moves)
                    }
                    _ => format!("{} ({} turns)", p.name(), turns),
                }
            })
            .collect();
        // advisors: the best defender and the first useful improvement
        let defender = g.best_defender_type(self.city);
        let military = produce_text(ctx, defender.name()).unwrap_or_else(|| format!("WE SHOULD BUILD {}.", defender.name().to_ascii_uppercase()));
        let domestic = list
            .iter()
            .find_map(|p| match p {
                Production::Improvement(i) if *i != improvement_ids::PALACE => produce_text(ctx, p.name()),
                _ => None,
            })
            .unwrap_or_else(|| "OUR CITIES ARE WELL PROVIDED FOR.".to_string());
        self.advisors = Some((military, domestic));
        let title = format!("What shall we build in {}?", c.name);
        self.pending = Pending::Change(list);
        Transition::Push(Box::new(MenuDialog::new(vec![title], items).with_pos(81, 9).with_selection(sel).with_title_colour(crate::ui::theme::WHITE)))
    }

    fn buy(&mut self, ctx: &mut UiContext) -> Transition {
        let g = ctx.game();
        let c = g.city(self.city);
        let cost = g.buy_cost(self.city);
        if cost <= 0 {
            return self.error(ctx, &RuleError::Other("Production is already complete."));
        }
        let gold = g.player(c.owner).gold;
        let lines = vec!["Cost to complete".to_string(), format!("{}: {}$", c.production.name(), cost), format!("Treasury: {}$", gold)];
        self.pending = Pending::Buy;
        Transition::Push(Box::new(YesNo::new(lines, "Yes", "No").with_pos(100, 80)))
    }

    fn sell(&mut self, id: u8) -> Transition {
        let name = rules::IMPROVEMENTS[id as usize].name;
        let price = rules::IMPROVEMENTS[id as usize].cost as i32 * 10;
        self.pending = Pending::Sell(id);
        Transition::Push(Box::new(YesNo::new(vec![format!("Sell {} for {}$?", name, price)], "Yes", "No").with_pos(100, 80)))
    }

    fn rename(&mut self, ctx: &mut UiContext) -> Transition {
        let name = ctx.game().city(self.city).name.clone();
        self.pending = Pending::Rename;
        Transition::Push(Box::new(TextInput::new(vec!["City Name...".into()], &name, 12).with_pos(80, 80).with_style(PanelStyle::Cyan)))
    }

    fn view(&mut self) -> Transition {
        self.pending = Pending::Ignore;
        Transition::Push(crate::ui::city_view::city_view(self.city))
    }

    fn tab_click(&mut self, i: usize) -> Transition {
        match i {
            0 => self.tab = Tab::Info,
            1 => self.tab = Tab::Happy,
            2 => self.tab = Tab::Map,
            _ => return self.view(),
        }
        Transition::None
    }

    fn map_click(&mut self, ctx: &mut UiContext, mx: i32, my: i32) {
        let dx = (mx - MAP_ORIGIN.0).div_euclid(16) - 2;
        let dy = (my - MAP_ORIGIN.1).div_euclid(16) - 2;
        if dx == 0 && dy == 0 {
            ctx.game_mut().auto_assign(self.city);
            return;
        }
        if let Some(idx) = rules::CITY_RADIUS.iter().position(|&(rx, ry)| (rx as i32, ry as i32) == (dx, dy)) {
            ctx.game_mut().toggle_tile(self.city, idx as u8);
        }
    }

    fn citizen_click(&mut self, ctx: &mut UiContext, mx: i32) {
        let g = ctx.game();
        let c = g.city(self.city);
        let n = c.size as usize;
        let workers = n - c.specialists.len();
        let pitch = Self::citizen_pitch(n);
        if mx < CITIZEN_X {
            return;
        }
        let i = ((mx - CITIZEN_X) / pitch) as usize;
        let i = if i >= workers { (mx - CITIZEN_X - 4).max(0) as usize / pitch as usize } else { i };
        if i < workers || i >= n {
            return;
        }
        let si = i - workers;
        let next = match c.specialists[si] {
            Specialist::Entertainer => Specialist::Taxman,
            Specialist::Taxman => Specialist::Scientist,
            Specialist::Scientist => Specialist::Entertainer,
        };
        ctx.game_mut().set_specialist(self.city, si, next);
    }

    fn click(&mut self, ctx: &mut UiContext, x: i32, y: i32) -> Transition {
        if B_EXIT.contains(x, y) {
            return Transition::Pop(DialogResult::Done);
        }
        if B_RENAME.contains(x, y) {
            return self.rename(ctx);
        }
        if B_CHANGE.contains(x, y) {
            return self.change(ctx);
        }
        if B_BUY.contains(x, y) {
            return self.buy(ctx);
        }
        for (i, r) in TAB_RECTS.iter().enumerate() {
            if r.contains(x, y) {
                return self.tab_click(i);
            }
        }
        if P_MAP.inset(1).contains(x, y) {
            self.map_click(ctx, x, y);
            return Transition::None;
        }
        if P_IMP.inset(1).contains(x, y) && y >= 2 {
            let row = ((y - 2) / 8) as usize;
            if let Some(Production::Improvement(id)) = self.listed(ctx).get(row).copied() {
                if id != improvement_ids::PALACE {
                    return self.sell(id);
                }
                return self.error(ctx, &RuleError::Other("The Palace cannot be sold."));
            }
            return Transition::None;
        }
        if P_TITLE.inset(1).contains(x, y) && y >= CITIZEN_Y {
            self.citizen_click(ctx, x);
        }
        Transition::None
    }

    // ---- drawing ---------------------------------------------------------------------------

    fn draw_title(&self, fb: &mut Surface, ctx: &UiContext) {
        panel(fb, P_TITLE);
        let g = ctx.game();
        let c = g.city(self.city);
        let title = format!("{} (POP:{})", c.name.to_ascii_uppercase(), crate::ui::mapview::format_pop(c.population()));
        small_text_centered(fb, ctx, P_TITLE.x + P_TITLE.w / 2, 2, WHITE, &title);
        let h = g.city_happiness(self.city);
        let n = c.size as usize;
        let pitch = Self::citizen_pitch(n);
        let mut x = CITIZEN_X;
        let mut pos = 0usize;
        for (count, kind) in [(h.happy, 0), (h.content, 1), (h.unhappy, 2)] {
            for _ in 0..count.max(0) {
                let idx = citizen_index(kind == 0, kind == 1, kind == 2, pos);
                citizen(fb, ctx, idx, x, CITIZEN_Y);
                x += pitch;
                pos += 1;
            }
        }
        if !c.specialists.is_empty() {
            x += 4;
            for s in &c.specialists {
                citizen(fb, ctx, specialist_index(*s), x, CITIZEN_Y);
                x += pitch;
            }
        }
    }

    fn draw_resources(&self, fb: &mut Surface, ctx: &UiContext) {
        panel(fb, P_RES);
        header(fb, ctx, Rect::new(P_RES.x, P_RES.y, P_RES.w, 9), "CITY RESOURCES");
        let g = ctx.game();
        let y = g.city_yield(self.city);
        let w = P_RES.w - 4;
        icon_row(fb, ctx, ICON_FOOD, 2, 32, y.food, y.food_upkeep, w);
        icon_row(fb, ctx, ICON_SHIELD, 2, 40, y.shields, y.shield_upkeep, w);
        icon_row(fb, ctx, ICON_TRADE, 2, 47, y.trade, y.corruption, w);
        // luxuries, taxes and science, each group separated by a small gap
        let mut x = 2;
        for (n, src) in [(y.lux, ICON_LUX), (y.tax, ICON_TAX), (y.sci, ICON_SCI)] {
            if n <= 0 {
                continue;
            }
            let total = y.lux.max(0) + y.tax.max(0) + y.sci.max(0);
            let pitch = if total * 8 + 8 <= w { 8 } else { ((w - 8) / total.max(1)).max(1) };
            for i in 0..n {
                icon(fb, ctx, src, x + i * pitch, 55, None);
            }
            x += n * pitch + 4;
        }
    }

    fn draw_support(&self, fb: &mut Surface, ctx: &UiContext) {
        panel(fb, P_SUPPORT);
        let g = ctx.game();
        let c = g.city(self.city);
        let y = g.city_yield(self.city);
        let w = P_SUPPORT.w - 4;
        // shields taken by unit upkeep, food taken by settlers, gold for maintenance
        icon_row(fb, ctx, ICON_SHIELD, 2, 69, y.shield_upkeep, 0, w);
        icon_row(fb, ctx, ICON_FOOD, 2, 77, y.food_upkeep - 2 * c.size as i32, 0, w);
        icon_row(fb, ctx, ICON_TAX, 2, 86, y.gold_upkeep, 0, w);
    }

    fn draw_tile_icons(&self, fb: &mut Surface, ctx: &UiContext, px: i32, py: i32, yields: (i32, i32, i32), outline: Option<u8>) {
        let (f, s, t) = yields;
        let mut list = Vec::new();
        list.extend(std::iter::repeat(ICON_FOOD).take(f.max(0) as usize));
        list.extend(std::iter::repeat(ICON_SHIELD).take(s.max(0) as usize));
        list.extend(std::iter::repeat(ICON_TRADE).take(t.max(0) as usize));
        let n = list.len() as i32;
        if n == 0 {
            return;
        }
        let per_row = if n <= 4 { 2 } else if n <= 6 { 3 } else { 4 };
        let rows = (n + per_row - 1) / per_row;
        let px_pitch = 16 / per_row;
        let py_pitch = (16 / rows).min(8);
        for (i, src) in list.iter().enumerate() {
            let i = i as i32;
            icon(fb, ctx, *src, px + (i % per_row) * px_pitch, py + (i / per_row) * py_pitch, outline);
        }
    }

    fn draw_map(&self, fb: &mut Surface, ctx: &UiContext) {
        panel(fb, P_MAP);
        let g = ctx.game();
        let c = g.city(self.city);
        let slot = c.owner;
        for dy in -2..=2 {
            for dx in -2..=2 {
                // the corners are outside the city radius: the panel shows through
                if dx * dx == 4 && dy * dy == 4 {
                    continue;
                }
                let (tx, ty) = (c.x + dx, c.y + dy);
                let px = MAP_ORIGIN.0 + (dx + 2) * 16;
                let py = MAP_ORIGIN.1 + (dy + 2) * 16;
                // unexplored (and off-map) squares leave the panel background, as in the original
                if !crate::game::Map::in_bounds(tx, ty) || !g.explored(slot, tx, ty) {
                    continue;
                }
                draw_terrain(fb, ctx, g, tx, ty, px, py);
                draw_fog_edges(fb, ctx, g, slot, tx, ty, px, py);
                if let Some(city) = g.city_at(tx, ty) {
                    draw_city(fb, ctx, g, city, px, py);
                }
            }
        }
        // worked tiles show their yields
        for idx in 0..21u8 {
            if !c.works(idx) {
                continue;
            }
            let (rx, ry) = rules::CITY_RADIUS[idx as usize];
            let Some((tx, ty)) = g.radius_tile(self.city, idx) else { continue };
            let px = MAP_ORIGIN.0 + (rx as i32 + 2) * 16;
            let py = MAP_ORIGIN.1 + (ry as i32 + 2) * 16;
            let yields = g.tile_yield(slot, tx, ty, Some(self.city));
            self.draw_tile_icons(fb, ctx, px, py, yields, None);
        }
    }

    fn draw_improvements(&self, fb: &mut Surface, ctx: &UiContext) {
        panel(fb, P_IMP);
        let old = fb.clip();
        fb.set_clip(Some(P_IMP.inset(1)));
        for (i, p) in self.listed(ctx).iter().enumerate() {
            let y = 2 + 8 * i as i32;
            match p {
                Production::Improvement(id) => {
                    imp_icon(fb, ctx, improvement_cell(*id), 233, y);
                    sell_dot(fb, 308, y + 1);
                }
                Production::Wonder(w) => imp_icon(fb, ctx, wonder_cell(*w), 233, y),
                Production::Unit(_) => {}
            }
            small_text(fb, ctx, 253, y + 2, WHITE, &p.name().to_ascii_uppercase());
        }
        fb.set_clip(Some(old));
    }

    fn draw_food(&self, fb: &mut Surface, ctx: &UiContext) {
        panel(fb, P_FOOD);
        header(fb, ctx, Rect::new(P_FOOD.x, P_FOOD.y, P_FOOD.w, 8), "FOOD STORAGE");
        fb.fill_rect(FOOD_AREA, BORDER);
        let c = ctx.game().city(self.city);
        let cols = c.size as i32 + 1;
        // 8 px per icon column, squeezed once the box would leave the panel
        let max_w = FOOD_AREA.w - 1;
        let pitch_x = 8.min((max_w - 9) / (cols - 1).max(1)).max(1);
        let w = if cols > 1 { (cols - 1) * pitch_x + 9 } else { 9 };
        let store = Rect::new(FOOD_AREA.x, FOOD_AREA.y + 1, w.min(max_w), FOOD_AREA.h - 2);
        fill_stipple(fb, store);
        let n = c.food.clamp(0, c.granary_size());
        for k in 0..n {
            let (col, row) = (k % cols, k / cols);
            if row >= FOOD_ROWS {
                break;
            }
            icon(fb, ctx, ICON_FOOD, store.x + 1 + col * pitch_x, store.y + 1 + row * 8, None);
        }
    }

    fn draw_tabs(&self, fb: &mut Surface, ctx: &UiContext) {
        panel(fb, P_TABS);
        let sel = match self.tab {
            Tab::Info => 0,
            Tab::Happy => 1,
            Tab::Map => 2,
        };
        for (i, r) in TAB_RECTS.iter().enumerate() {
            let style = if i == sel { ButtonStyle::Selected } else { ButtonStyle::Plain };
            button(fb, ctx, *r, TAB_LABELS[i], style);
        }
        fb.hline(P_TABS.x + 1, P_TABS.right() - 2, 114, BORDER);
        match self.tab {
            Tab::Info => self.draw_info(fb, ctx),
            Tab::Happy => self.draw_happy(fb, ctx),
            Tab::Map => self.draw_minimap(fb, ctx),
        }
    }

    fn draw_info(&self, fb: &mut Surface, ctx: &UiContext) {
        let g = ctx.game();
        for (i, id) in self.info_units(ctx).iter().enumerate().take(21) {
            let u = g.unit(*id);
            let x = 100 + 18 * (i as i32 % 7);
            let y = 116 + 24 * (i as i32 / 7);
            let kind = if u.leader { rules::unit_ids::DIPLOMAT } else { u.kind };
            unit_box(fb, ctx, kind, u.owner, x, y);
            small_text(fb, ctx, x, y + 16, BLACK, order_abbrev(u.order));
        }
    }

    fn draw_happy(&self, fb: &mut Surface, ctx: &UiContext) {
        let g = ctx.game();
        let c = g.city(self.city);
        let h = g.city_happiness(self.city);
        let n = c.size as usize;
        let pitch = (120 / n.max(1) as i32).clamp(2, 7);
        for (r, (_, happy, content, unhappy)) in h.steps.iter().enumerate().take(5) {
            let y = 116 + 16 * r as i32;
            let mut x = 102;
            let mut pos = 0usize;
            for (count, kind) in [(*happy, 0), (*content, 1), (*unhappy, 2)] {
                for _ in 0..count.max(0) {
                    citizen(fb, ctx, citizen_index(kind == 0, kind == 1, kind == 2, pos), x, y);
                    x += pitch;
                    pos += 1;
                }
            }
            if !c.specialists.is_empty() {
                x += 4;
                for s in &c.specialists {
                    citizen(fb, ctx, specialist_index(*s), x, y);
                    x += pitch;
                }
            }
            fb.hline(100, 222, y + 14, BORDER);
        }
    }

    fn draw_minimap(&self, fb: &mut Surface, ctx: &UiContext) {
        fb.fill_rect(MINIMAP, BLACK);
        let g = ctx.game();
        let c = g.city(self.city);
        let slot = c.owner;
        let (w, h) = (rules::MAP_W as i32, rules::MAP_H as i32);
        for ty in 0..h {
            for tx in 0..w {
                if !g.explored(slot, tx, ty) {
                    continue;
                }
                let x0 = MINIMAP.x + tx * MINIMAP.w / w;
                let x1 = MINIMAP.x + (tx + 1) * MINIMAP.w / w;
                let y0 = MINIMAP.y + ty * MINIMAP.h / h;
                let y1 = MINIMAP.y + (ty + 1) * MINIMAP.h / h;
                let colour = if g.map.is_land(tx, ty) { 2 } else { BORDER };
                fb.fill_rect(Rect::new(x0, y0, (x1 - x0).max(1), (y1 - y0).max(1)), colour);
            }
        }
        for other in g.all_cities() {
            if g.explored(slot, other.x, other.y) {
                let x = MINIMAP.x + other.x * MINIMAP.w / w;
                let y = MINIMAP.y + other.y * MINIMAP.h / h;
                let col = rules::SLOT_COLOURS[(other.owner & 7) as usize].light;
                fb.fill_rect(Rect::new(x, y, 2, 2), col);
            }
        }
    }

    fn draw_production(&self, fb: &mut Surface, ctx: &UiContext) {
        let g = ctx.game();
        let c = g.city(self.city);
        let cost = c.production.cost();
        let (_, pitch, r) = stock_layout(cost);
        panel(fb, r);
        fb.fill_rect(Rect::new(r.x + 1, r.y + 1, r.w - 2, 115 - r.y - 1), BORDER);
        button(fb, ctx, B_CHANGE, "CHANGE", ButtonStyle::Plain);
        button(fb, ctx, B_BUY, "BUY", ButtonStyle::Plain);
        match c.production {
            Production::Unit(k) => unit_box(fb, ctx, k, c.owner, ITEM_FRAME.0, ITEM_FRAME.1),
            other => {
                item_frame(fb, ITEM_FRAME.0, ITEM_FRAME.1, c.owner);
                let old = fb.clip();
                fb.set_clip(Some(Rect::new(ITEM_FRAME.0 + 1, ITEM_FRAME.1 + 1, 13, 13)));
                let cell = match other {
                    Production::Wonder(w) => wonder_cell(w),
                    Production::Improvement(i) => improvement_cell(i),
                    Production::Unit(_) => 0,
                };
                imp_icon(fb, ctx, cell, ITEM_FRAME.0 - 2, ITEM_FRAME.1 + 4);
                fb.set_clip(Some(old));
            }
        }
        let n = c.shields.clamp(0, cost);
        for k in 0..n {
            icon(fb, ctx, ICON_SHIELD, r.x + 1 + (k % 10) * 8, 115 + (k / 10) * pitch, None);
        }
        if c.auto_build {
            small_text(fb, ctx, r.x + 3, r.bottom() + 2, WHITE, "AUTO");
        }
    }

    fn draw_advisors(&self, fb: &mut Surface, ctx: &UiContext) {
        let Some((mil, dom)) = &self.advisors else { return };
        for (r, head, text) in [(Rect::new(7, 161, 125, 26), "MILITARY ADVISOR:", mil), (Rect::new(166, 162, 133, 25), "DOMESTIC ADVISOR:", dom)] {
            draw_styled_panel(fb, ctx, r, PanelStyle::Grey);
            let mut y = r.y + 4;
            small_text(fb, ctx, r.x + 5, y, BLACK, head);
            for para in text.split('\n') {
                for line in wrap(&ctx.fonts, SMALL, r.w - 24, para) {
                    y += 6;
                    if y + 5 < r.bottom() {
                        small_text(fb, ctx, r.x + 5, y, BLACK, &line);
                    }
                }
            }
        }
    }
}

impl Screen for CityScreen {
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        let Some(start) = self.start.take() else { return Transition::None };
        match start.as_str() {
            "happy" => self.tab_click(1),
            "map" => self.tab_click(2),
            "view" => self.view(),
            "change" => self.change(ctx),
            "buy" => self.buy(ctx),
            "rename" => self.rename(ctx),
            "sell" => self.sell(improvement_ids::BARRACKS),
            _ => Transition::None,
        }
    }

    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if ctx.game().city_opt(self.city).is_none() {
            return Transition::Pop(DialogResult::Done);
        }
        match ev {
            Event::Key { key, mods, down: true, .. } => match key {
                Key::Esc | Key::Enter => Transition::Pop(DialogResult::Done),
                Key::Char('x') => Transition::Pop(DialogResult::Done),
                Key::Char('i') => self.tab_click(0),
                Key::Char('h') => self.tab_click(1),
                Key::Char('m') => self.tab_click(2),
                Key::Char('v') => self.view(),
                Key::Char('c') => self.change(ctx),
                Key::Char('b') => self.buy(ctx),
                Key::Char('r') => self.rename(ctx),
                Key::Char('a') if mods.shift => {
                    let c = ctx.game_mut().city_mut(self.city);
                    c.auto_build = !c.auto_build;
                    Transition::None
                }
                _ => Transition::None,
            },
            Event::MouseButton { button: MouseButton::Left, x, y, down: true } => self.click(ctx, *x, *y),
            _ => Transition::None,
        }
    }

    fn resume(&mut self, result: DialogResult, ctx: &mut UiContext) -> Transition {
        let pending = std::mem::replace(&mut self.pending, Pending::None);
        self.advisors = None;
        if ctx.game().city_opt(self.city).is_none() {
            return Transition::Pop(DialogResult::Done);
        }
        let r = match (pending, result) {
            (Pending::Change(list), DialogResult::Choice(i)) => match list.get(i) {
                Some(p) => ctx.game_mut().set_production(self.city, *p),
                None => Ok(()),
            },
            (Pending::Buy, DialogResult::Yes) => ctx.game_mut().buy(self.city),
            (Pending::Sell(id), DialogResult::Yes) => ctx.game_mut().sell_improvement(self.city, id).map(|_| ()),
            (Pending::Rename, DialogResult::Text(name)) => ctx.game_mut().rename_city(self.city, &name),
            _ => Ok(()),
        };
        match r {
            Ok(()) => Transition::None,
            Err(e) => self.error(ctx, &e),
        }
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        fb.fill(BLACK);
        if ctx.game().city_opt(self.city).is_none() {
            return;
        }
        self.draw_title(fb, ctx);
        self.draw_resources(fb, ctx);
        self.draw_support(fb, ctx);
        self.draw_map(fb, ctx);
        self.draw_improvements(fb, ctx);
        self.draw_food(fb, ctx);
        self.draw_tabs(fb, ctx);
        self.draw_production(fb, ctx);
        button(fb, ctx, B_RENAME, "RENAME", ButtonStyle::Raised);
        button(fb, ctx, B_EXIT, "EXIT", ButtonStyle::Exit);
        self.draw_advisors(fb, ctx);
    }

    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.sp.pal
    }

    fn draws_parent(&self) -> bool {
        false
    }
}
