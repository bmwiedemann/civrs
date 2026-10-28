//! The city screen, laid out pixel for pixel after the original (see the measurements in
//! the reverse-engineering report): title and citizens, CITY RESOURCES, unit support, the 5x5
//! city map, improvements with sell dots, FOOD STORAGE, the INFO / HAPPY / MAP / VIEW tabs,
//! the production box with CHANGE / BUY and the RENAME / EXIT buttons.

use crate::game::{CityId, Production, RuleError, Specialist};
use crate::gfx::{Rect, Surface};
use crate::platform::{Event, Key, MouseButton};
use crate::rules::{self, improvement_ids};
use crate::ui::city_draw::*;
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::tilerender::{draw_city, draw_fog_edges, draw_terrain};
use crate::ui::widgets::{CityNamePrompt, MenuDialog, MessageBox, YesNo};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Info,
    Happy,
    Map,
}

enum Pending {
    None,
    /// The production menu's entries (None = "more...") and the page "more..." opens.
    Change(Vec<Option<Production>>, ChangePage),
    Buy,
    Sell(u8),
    Rename,
    Ignore,
}

/// Pages of the production menu (CIV.EXE 0xa9fd): everything while it fits in 30 lines
/// (the title included), else the units, then the improvements and wonders, each ending
/// in "more..." that opens the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChangePage {
    All,
    Units,
    Buildings,
}

pub struct CityScreen {
    pub city: CityId,
    tab: Tab,
    pending: Pending,
    /// Advisor speech texts shown under the "What shall we build" menu.
    advisors: Option<(AdvisorNote, AdvisorNote)>,
    /// Action performed on the first frame (headless renders of the sub-views / dialogs).
    start: Option<String>,
    /// First improvement slot of the listed page (MORE flips between 0 and the overflow).
    imp_start: usize,
    /// A foreign city seen by an investigating Diplomat: only the tabs work, anything else
    /// closes the screen.
    view_only: bool,
}

pub fn city_screen(c: CityId) -> Box<dyn Screen> {
    Box::new(CityScreen { city: c, tab: Tab::Info, pending: Pending::None, advisors: None, start: None, imp_start: 0, view_only: false })
}

/// City screen that opens on a tab or dialog: "happy", "map", "view", "change", "buy", "rename", "sell".
pub fn city_screen_at(c: CityId, start: &str) -> Box<dyn Screen> {
    Box::new(CityScreen { city: c, tab: Tab::Info, pending: Pending::None, advisors: None, start: Some(start.to_string()), imp_start: 0, view_only: false })
}

/// City screen of a foreign city investigated by a Diplomat (CIV.EXE o 0x1d6a2).
pub fn city_screen_view(c: CityId) -> Box<dyn Screen> {
    Box::new(CityScreen { city: c, tab: Tab::Info, pending: Pending::None, advisors: None, start: None, imp_start: 0, view_only: true })
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
const P_TABS: Rect = Rect::new(95, 106, 133, 92);
/// Buttons as draw_button's inclusive (x1, y1)..(x2, y2) (CIV.EXE 0x10737 ff.).
const TAB_RECTS: [Rect; 4] = [Rect::new(95, 106, 34, 9), Rect::new(129, 106, 32, 9), Rect::new(161, 106, 33, 9), Rect::new(194, 106, 33, 9)];
const TAB_LABELS: [&str; 4] = ["INFO", "HAPPY", "MAP", "VIEW"];
const B_CHANGE: Rect = Rect::new(231, 106, 33, 9);
const B_BUY: Rect = Rect::new(294, 106, 18, 9);
const ITEM_FRAME: (i32, i32) = (264, 100);
const B_RENAME: Rect = Rect::new(231, 190, 42, 9);
const B_EXIT: Rect = Rect::new(284, 190, 33, 9);
const MAP_ORIGIN: (i32, i32) = (128, 24);
const CITIZEN_X: i32 = 10;
const CITIZEN_Y: i32 = 8;
const MINIMAP: Rect = Rect::new(101, 116, 120, 76);
const B_MORE: Rect = Rect::new(287, 88, 29, 9);
/// Rows of the improvement list (from y 2, 6 px apart; CIV.EXE 0x103ad).
const IMP_ROWS: usize = 14;

/// Production box (CIV.EXE 0x10660..0x10866) for a table cost `cost` (in tens) and `store`
/// shields: squeeze k = clamp((cost - 1) / 10 + 1, (store - 1) / 100 + 1, 99), rows of
/// factor * k shields at 8 / k px (80 / factor for computer owners) from (232, 117), 8 px
/// apart; inner size w = 80 + (8k - 8) / k, h = (cost - 1) / k * 8 + 8. Returns (k, step, box)
/// with the colour 1 box at (230, 99, w + 3, h + 19).
fn stock_layout(cost: i32, store: i32, factor: i32) -> (i32, i32, Rect) {
    let k = ((cost - 1) / 10 + 1).max((store - 1) / 100 + 1).min(99);
    let step = 80 / factor;
    let w = factor * step + (8 * k - 8) / k;
    let h = (cost - 1) / k * 8 + 8;
    (k, step, Rect::new(230, 99, w + 3, h + 19))
}

/// Lines of an advisor's note (none without a recommendation).
type AdvisorNote = Option<Vec<String>>;

/// An advisor's note under the production menu (0xb8b4): the heading, the item's PRODUCE.TXT
/// entry with `$RPLC1` = its name, wrapped at 50 characters (wrap_text 0x1ecb6).
fn advisor_lines(ctx: &UiContext, head: &str, name: &str) -> Vec<String> {
    let mut text = format!("{head}\n");
    if let Some(t) = ctx.texts.produce.entry_text(name) {
        text += &t.replace("$RPLC1", name);
    }
    let text = crate::assets::text::wrap_text(&text, 50);
    text.strip_suffix('\n').unwrap_or(&text).split('\n').map(str::to_string).collect()
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
    fn handle_event(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if ctx.game().city_opt(self.city).is_none() {
            return Transition::Pop(DialogResult::Done);
        }
        if self.view_only {
            return match ev {
                Event::Key { key: Key::Unknown, .. } => Transition::None,
                Event::Key { key: Key::Char('i'), down: true, .. } => self.tab_click(0),
                Event::Key { key: Key::Char('h'), down: true, .. } => self.tab_click(1),
                Event::Key { key: Key::Char('m'), down: true, .. } => self.tab_click(2),
                Event::Key { down: true, .. } => Transition::Pop(DialogResult::Done),
                Event::MouseButton { button: MouseButton::Left, x, y, down: true } => match TAB_RECTS.iter().position(|r| r.contains(*x, *y)) {
                    Some(i) => self.tab_click(i),
                    None => Transition::Pop(DialogResult::Done),
                },
                _ => Transition::None,
            };
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
                    // AUTO on asks the production advisor at once; with nothing worth it the
                    // flag stays until the city finishes its item (CIV.EXE 0x113d7..0x11411)
                    let g = ctx.game_mut();
                    let c = g.city_mut(self.city);
                    c.auto_build = !c.auto_build;
                    if c.auto_build {
                        if let Some(p) = g.auto_production(self.city) {
                            g.city_mut(self.city).production = p;
                        }
                    }
                    Transition::None
                }
                _ => Transition::None,
            },
            Event::MouseButton { button: MouseButton::Left, x, y, down: true } => self.click(ctx, *x, *y),
            _ => Transition::None,
        }
    }

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
    /// The city's own units on its square, as listed on the INFO tab (at most 18).
    fn info_units(&self, ctx: &UiContext) -> Vec<crate::game::UnitId> {
        let g = ctx.game();
        let c = g.city(self.city);
        g.units_at(c.x, c.y).into_iter().filter(|id| g.unit(*id).owner == c.owner).take(18).collect()
    }

    /// Improvements then wonders of the city, as listed in the improvements panel.
    /// The listed page: the wonders (first page only), then the improvements from `imp_start`
    /// up to 14 rows, and the improvement slot the next page starts at, if any.
    fn listed(&self, ctx: &UiContext) -> (Vec<Production>, Option<usize>) {
        let c = ctx.game().city(self.city);
        let mut v: Vec<Production> = Vec::new();
        if self.imp_start == 0 {
            v.extend(c.wonder_list().into_iter().map(Production::Wonder));
        }
        for id in c.improvement_list() {
            let k = id as usize - 1;
            if k < self.imp_start {
                continue;
            }
            if v.len() >= IMP_ROWS {
                return (v, Some(k));
            }
            v.push(Production::Improvement(id));
        }
        (v, None)
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
        self.change_page(ctx, ChangePage::All)
    }

    fn change_page(&mut self, ctx: &mut UiContext, page: ChangePage) -> Transition {
        let g = ctx.game();
        let c = g.city(self.city);
        let y = g.city_yield(self.city);
        let surplus = y.shield_surplus().clamp(1, 99);
        let all = g.available_production(self.city);
        let page = match page {
            ChangePage::All if 1 + all.len() > 30 => ChangePage::Units,
            p => p,
        };
        let mut list: Vec<Option<Production>> = all
            .iter()
            .filter(|p| match page {
                ChangePage::All => true,
                ChangePage::Units => matches!(p, Production::Unit(_)),
                ChangePage::Buildings => !matches!(p, Production::Unit(_)),
            })
            .map(|p| Some(*p))
            .collect();
        let next = match page {
            ChangePage::All => ChangePage::All,
            ChangePage::Units => ChangePage::Buildings,
            ChangePage::Buildings => ChangePage::Units,
        };
        if page != ChangePage::All {
            list.push(None);
        }
        let mut sel = 0;
        let items: Vec<String> = list
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let Some(p) = p else { return "more...".to_string() };
                if *p == c.production {
                    sel = i;
                }
                // append_turns_to_build (0xbab3): the store counts against every item
                let turns = ((p.cost() - c.shields - 1) / surplus + 1).clamp(1, 999);
                let turns = format!("{turns} turn{}", if turns == 1 { "" } else { "s" });
                match p {
                    Production::Unit(k) => {
                        let u = &rules::UNITS[*k as usize];
                        format!("{} ({}, ADM:{}/{}/{})", p.name(), turns, u.att, u.def, u.moves)
                    }
                    // an obsolete wonder is offered with a "*" (0xae52)
                    Production::Wonder(w) if g.wonder_marked_obsolete(*w) => format!("*{} ({})", p.name(), turns),
                    _ => format!("{} ({})", p.name(), turns),
                }
            })
            .collect();
        // with Instant Advice the advisors name the best unit ([bp-0xc]) and the best
        // improvement ([bp-0x48]) of the valuation (0xb8b4..0xb9ed)
        let advice = if ctx.game().options.instant_advice { ctx.game_mut().build_advice(self.city).ok() } else { None };
        let g = ctx.game();
        let c = g.city(self.city);
        let military = advice.as_ref().and_then(|a| a.unit).map(|(p, _)| advisor_lines(ctx, "Military Advisor:", p.name()));
        let domestic = advice.as_ref().and_then(|a| a.improvement).map(|(p, _)| advisor_lines(ctx, "Domestic Advisor:", p.name()));
        self.advisors = Some((military, domestic));
        let title = format!("What shall we build in {}?", c.name);
        // the small font (CIV.EXE's font 2) from 21 lines on and on either page (0xb9fe)
        let small = 1 + all.len() > 20 || page != ChangePage::All;
        self.pending = Pending::Change(list, next);
        // popup_menu3 at (80, 8) with Civilopedia help (ds:0x2fa0, 0xba14)
        let menu = MenuDialog::new(vec![title], items).with_pos(80, 8).with_selection(sel).with_help();
        Transition::Push(Box::new(if small { menu.with_font(1) } else { menu }))
    }

    fn buy(&mut self, ctx: &mut UiContext) -> Transition {
        let g = ctx.game();
        let c = g.city(self.city);
        let cost = g.buy_cost(self.city);
        if cost <= 0 {
            return self.error(ctx, &RuleError::Other("Production is already complete."));
        }
        let gold = g.player(c.owner).gold;
        // 0x115e5: "Yes/No" only when the treasury covers it; a unit in disorder cannot be
        // bought ("CIVIL DISORDER"); popup at (100, 80)
        let mut lines = vec!["Cost to complete".to_string(), format!("{}: ${}", c.production.name(), cost), format!("Treasury: ${}", gold)];
        let unit = matches!(c.production, Production::Unit(_));
        if unit && c.disorder {
            lines.push("CIVIL DISORDER".into());
        } else if gold >= cost {
            self.pending = Pending::Buy;
            return Transition::Push(Box::new(YesNo::new(lines, "Yes", "No").with_pos(100, 80)));
        }
        self.pending = Pending::Ignore;
        Transition::Push(Box::new(MenuDialog::new(lines, Vec::new()).with_pos(100, 80)))
    }

    fn sell(&mut self, id: u8) -> Transition {
        let name = rules::IMPROVEMENTS[id as usize].name;
        let price = crate::game::economy::sell_price(id);
        self.pending = Pending::Sell(id);
        // 0x120ab: "No." first, "Yes." sells; popup at (128, 80)
        let q = vec!["Do you want to sell".to_string(), format!("your {name} for {price}$?")];
        Transition::Push(Box::new(YesNo::no_first(q, "No.", "Yes.").with_pos(128, 80)))
    }

    fn rename(&mut self, ctx: &mut UiContext) -> Transition {
        let name = ctx.game().city(self.city).name.clone();
        self.pending = Pending::Rename;
        Transition::Push(Box::new(CityNamePrompt::new(&name)))
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
        // the city square only recomputes the city (CIV.EXE 0x123ca)
        if dx == 0 && dy == 0 {
            ctx.game_mut().fix_workers(self.city);
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
        // a click on one of the city's units on the INFO tab wakes it (0x12271; the original
        // finds the unit with a 16 px column pitch although they are drawn 18 px apart)
        if matches!(self.tab, Tab::Info) && (100..211).contains(&x) && y >= 116 {
            let i = ((y - 116) / 16 * 6 + (x - 100) / 16) as usize;
            if let Some(&id) = self.info_units(ctx).get(i) {
                ctx.game_mut().wake_unit(id);
                return Transition::None;
            }
        }
        if P_MAP.inset(1).contains(x, y) {
            self.map_click(ctx, x, y);
            return Transition::None;
        }
        // MORE flips to the overflow page and back (CIV.EXE 0x11fd8)
        let (list, more) = self.listed(ctx);
        if (more.is_some() || self.imp_start > 0) && x >= 288 && y > 88 && y < 96 {
            self.imp_start = more.unwrap_or(0);
            return Transition::None;
        }
        // sell dots (0x1201f)
        if x >= 300 && y < 94 && y >= 2 {
            let row = ((y - 2) / 6) as usize;
            // the Palace too; nothing once the city has sold this turn (city flag 0x80)
            if let Some(Production::Improvement(id)) = list.get(row).copied() {
                if !ctx.game().player(ctx.game().city(self.city).owner).sold_cities.contains(&self.city) {
                    return self.sell(id);
                }
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
        panel(fb, ctx, P_TITLE);
        let g = ctx.game();
        let c = g.city(self.city);
        let title = format!("{} (POP:{})", c.name.to_ascii_uppercase(), crate::ui::mapview::format_pop(c.population()));
        small_text_centered(fb, ctx, 104, 2, WHITE, &title);
        // the citizens row is refilled before the figures (0x10ea6)
        fill_stipple(fb, ctx, Rect::new(8, 8, 200, 13));
        let h = g.city_happiness(self.city);
        let n = c.size as usize;
        let pitch = Self::citizen_pitch(n);
        let mut x = CITIZEN_X;
        let mut pos = 0usize;
        for (count, kind) in [(h.happy, 0), (h.content, 1), (h.unhappy, 2)] {
            for i in 0..count.max(0) {
                let idx = citizen_index(kind == 0, kind == 1, kind == 2, pos);
                citizen(fb, ctx, idx, x, CITIZEN_Y);
                // extra unhappiness beyond the size (ds:0x6542, CIV.EXE 0xc91:0x6eca)
                if kind == 2 && i < h.extra {
                    recolour(fb, Rect::new(x, CITIZEN_Y, 12, 14), 5, 12);
                }
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
        panel(fb, ctx, P_RES);
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
        panel(fb, ctx, P_SUPPORT);
        // the units the city supports (CIV.EXE 0xff78): seven to a row from (8, 69), 16 px
        // apart, the third row overlapping at y 77; under each its upkeep: a shield once the
        // free units are used up, food for Settlers, unhappy faces for units abroad
        let g = ctx.game();
        let c = g.city(self.city);
        let govt = rules::GOVERNMENTS[g.player(c.owner).govt as usize];
        let (mut x, mut y) = (8, 69);
        for (id, costs) in g.shield_costs(self.city) {
            let u = g.unit(id);
            if costs {
                icon(fb, ctx, ICON_SHIELD, x + 8, y + 12, None);
            }
            let (src, n) = if u.kind == rules::unit_ids::SETTLERS {
                (ICON_FOOD, govt.settler_food as i32)
            } else {
                (ICON_UNHAPPY, g.unit_unhappiness(self.city, u))
            };
            for k in 0..n.min(2) {
                icon(fb, ctx, src, x + 2 * k, y + 12, None);
            }
            let kind = if u.leader { rules::unit_ids::DIPLOMAT } else { u.kind };
            unit_box(fb, ctx, kind, u.owner, x, y);
            x += 16;
            if x >= 112 {
                x = 8;
                y += 16;
                if y > 85 {
                    y -= 24;
                }
            }
        }
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
        panel(fb, ctx, P_MAP);
        let g = ctx.game();
        let c = g.city(self.city);
        let slot = c.owner;
        for dy in -2..=2 {
            for dx in -2..=2 {
                // the corners are outside the city radius: the panel shows through
                if dx * dx == 4 && dy * dy == 4 {
                    continue;
                }
                let (tx, ty) = (crate::game::Map::wrap_x(c.x + dx), c.y + dy);
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
        panel(fb, ctx, P_IMP);
        // CIV.EXE 0x103ad: names 6 px apart, the pictograms alternating between x 233 and
        // 213 two pixels higher; names cut to 63 (wonders) or 56 px
        let (list, more) = self.listed(ctx);
        for (i, p) in list.iter().enumerate() {
            let y = 2 + 6 * i as i32;
            let ix = if i % 2 == 0 { 233 } else { 213 };
            let max = match p {
                Production::Improvement(id) => {
                    sell_dot(fb, 309, y + 1);
                    if let Some(cell) = improvement_cell(*id) {
                        imp_icon(fb, ctx, cell, ix, y - 2);
                    }
                    56
                }
                Production::Wonder(w) => {
                    imp_icon(fb, ctx, wonder_cell(*w), ix, y - 2);
                    63
                }
                Production::Unit(_) => 56,
            };
            small_text(fb, ctx, 253, y + 2, WHITE, &fit_small_text(ctx, &p.name().to_ascii_uppercase(), max));
        }
        // the first pictogram reaches over the panel's top edge, which is drawn again
        fb.hline(231, 250, 0, 0);
        fb.hline(231, 250, 1, 1);
        if more.is_some() || self.imp_start > 0 {
            button(fb, ctx, B_MORE, "MORE", ButtonStyle::Plain);
        }
    }

    fn draw_food(&self, fb: &mut Surface, ctx: &UiContext) {
        panel(fb, ctx, P_FOOD);
        header(fb, ctx, Rect::new(P_FOOD.x, P_FOOD.y, P_FOOD.w, 8), "FOOD STORAGE");
        fb.fill_rect(FOOD_AREA, BORDER);
        let c = ctx.game().city(self.city);
        let cols = c.size as i32 + 1;
        // 8 px per icon column, squeezed once the box would leave the panel
        let max_w = FOOD_AREA.w - 1;
        let pitch_x = 8.min((max_w - 9) / (cols - 1).max(1)).max(1);
        let w = if cols > 1 { (cols - 1) * pitch_x + 9 } else { 9 };
        let store = Rect::new(FOOD_AREA.x, FOOD_AREA.y + 1, w.min(max_w), FOOD_AREA.h - 2);
        fill_stipple(fb, ctx, store);
        // a granary marks the food kept after growth with a line across the middle (0x1090a)
        if c.has_improvement(improvement_ids::GRANARY) {
            fb.hline(5, (cols - 1) * pitch_x + 9, 155, 1);
        }
        let n = c.food.clamp(0, ctx.game().food_box(self.city));
        for k in 0..n {
            let (col, row) = (k % cols, k / cols);
            if row >= FOOD_ROWS {
                break;
            }
            icon(fb, ctx, ICON_FOOD, store.x + 1 + col * pitch_x, store.y + 1 + row * 8, None);
        }
    }

    fn draw_tabs(&self, fb: &mut Surface, ctx: &UiContext) {
        panel(fb, ctx, P_TABS);
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
        // CIV.EXE 0x101da: six to a row, 18 px apart, rows 16 px apart, each over the first
        // three letters of its home city
        let g = ctx.game();
        for (i, id) in self.info_units(ctx).iter().enumerate() {
            let u = g.unit(*id);
            let x = 100 + 18 * (i as i32 % 6);
            let y = 116 + 16 * (i as i32 / 6);
            // the map's unit routine (0x1b639), so a fortified unit gets its fortification
            // and the order letters show as on the map
            crate::ui::tilerender::draw_unit(fb, ctx, g, *id, x - 1, y - 1, false);
            let home = u.home.and_then(|h| g.city_opt(h)).map(|c| c.name.to_ascii_uppercase()).unwrap_or_else(|| "NONE".into());
            let abbrev: String = home.chars().take(3).chain(std::iter::once('.')).collect();
            small_text(fb, ctx, x, y + 15, BLACK, &abbrev);
        }
        // the pollution risk: one smokestack (the map's pollution sprite) per pollution point,
        // from x 98 at y 161/162 alternately, at most 8 px apart (CIV.EXE 0x110ad)
        let points = g.pollution_points(self.city);
        if points > 0 {
            use crate::ui::tilerender::{blit_sprite, cell, sp};
            let step = (100 / points.clamp(1, 99)).clamp(1, 8);
            for i in 0..points {
                blit_sprite(fb, &ctx.sp.surf, cell(sp::POLLUTION.0, sp::POLLUTION.1), 98 + i * step, 161 + (i & 1));
            }
        }
    }

    /// The HAPPY tab (CIV.EXE 0xf505..0xfbd0): one row of citizens per happiness step the
    /// original draws, 16 px apart from y 116 with a line under each, and the step's
    /// pictograms right of the citizens.
    fn draw_happy(&self, fb: &mut Surface, ctx: &UiContext) {
        use crate::game::happiness::HappyRow;
        use crate::ui::tilerender::{blit_sprite_remap, civ_remap, sp};
        let g = ctx.game();
        let c = g.city(self.city);
        let h = g.city_happiness(self.city);
        // draw_citizens(city, 100, y, specialists, 92) (0xc91:0x6eca): 7 px apart, or
        // 92 / size when that does not fit
        let n = c.size as i32;
        let pitch = if 7 * n > 92 { 92 / n.max(1) } else { 7 };
        for (r, step) in h.steps.iter().enumerate().take(5) {
            let y = 116 + 16 * r as i32;
            let mut x = 102;
            let mut pos = 0usize;
            for (count, kind) in [(step.happy, 0), (step.content, 1), (step.unhappy, 2)] {
                for i in 0..count.max(0) {
                    citizen(fb, ctx, citizen_index(kind == 0, kind == 1, kind == 2, pos), x, y);
                    // the first ds:0x6542 unhappy figures: recolour(x, y, 12, 14, 5 -> 12)
                    if kind == 2 && i < step.extra {
                        recolour(fb, Rect::new(x, y, 12, 14), 5, 12);
                    }
                    x += pitch;
                    pos += 1;
                }
                // 2 px after the content group when it is not empty; the gap after the happy
                // group tests a local the original never sets [approx: taken as "any happy"]
                if count > 0 && kind < 2 {
                    x += 2;
                }
            }
            if !c.specialists.is_empty() {
                x += 4;
                for s in &c.specialists {
                    citizen(fb, ctx, specialist_index(*s), x, y);
                    x += pitch;
                }
            }
            match &step.row {
                HappyRow::Base => {}
                HappyRow::Luxuries => icon(fb, ctx, ICON_LUX, 208, y + 4, None),
                HappyRow::Buildings(ids) => {
                    for (i, id) in ids.iter().enumerate() {
                        if let Some(cell) = improvement_cell(*id) {
                            imp_icon(fb, ctx, cell, 208 + 16 * i as i32, y);
                        }
                    }
                }
                HappyRow::MartialLaw(kinds) => {
                    let map = civ_remap(c.owner);
                    for (i, k) in kinds.iter().enumerate() {
                        blit_sprite_remap(fb, &ctx.sp.surf, sp::unit(*k), 208 - 2 * i as i32, y - 1, &map);
                    }
                }
                HappyRow::Military(faces) => {
                    for i in 0..*faces {
                        icon(fb, ctx, ICON_UNHAPPY, 208 - 2 * i, y + 4, None);
                    }
                }
                HappyRow::Wonders => {
                    small_text(fb, ctx, 190, y + 5, WHITE, "WONDERS");
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
        let factor = g.cost_factor(c.owner);
        let (k, step, r) = stock_layout(c.production.cost() / 10, c.shields, factor);
        fb.fill_rect(r, BORDER);
        fill_stipple(fb, ctx, Rect::new(231, 116, r.w - 2, r.h - 18));
        button(fb, ctx, B_CHANGE, if c.auto_build { "AUTO." } else { "CHANGE" }, ButtonStyle::Plain);
        // production that cannot be completed: the button face flashes yellow, colours 9 and
        // 14 swapping every 10 ticks (CIV.EXE 0x112e1)
        if g.production_invalid(self.city) && (ctx.frame / 10) % 2 == 0 {
            for y in B_CHANGE.y..B_CHANGE.bottom() {
                for x in B_CHANGE.x..B_CHANGE.right() {
                    if fb.get(x, y) == LIGHT {
                        fb.put(x, y, crate::ui::theme::YELLOW);
                    }
                }
            }
        }
        button(fb, ctx, B_BUY, "BUY", ButtonStyle::Plain);
        match c.production {
            Production::Unit(u) => unit_box(fb, ctx, u, c.owner, ITEM_FRAME.0, ITEM_FRAME.1),
            // improvements and wonders by name, centred over the buttons (0x1079f)
            other => {
                small_text_centered(fb, ctx, 274, 100, WHITE, &fit_small_text(ctx, other.name(), 86));
            }
        }
        let per_row = factor * k;
        for i in 0..c.shields.max(0) {
            icon(fb, ctx, ICON_SHIELD, 232 + (i % per_row) * step / k, 117 + (i / per_row) * 8, None);
        }
    }

    /// The advisors' notes, drawn once without input (ds:0xdb38) at (8, 160) and (164, 160) in
    /// the city screen's small font.
    fn draw_advisors(&self, fb: &mut Surface, ctx: &mut UiContext) {
        let Some((mil, dom)) = &self.advisors else { return };
        for (x, lines) in [(8, mil), (164, dom)] {
            if let Some(lines) = lines {
                MenuDialog::new(lines.clone(), Vec::new()).with_pos(x, 160).with_font(SMALL).paint(fb, ctx);
            }
        }
    }
}

/// `draw_citizens(city, x, y, .., width)` (0xc91:0x6eca): the city's happy, content and
/// unhappy citizens 7 px apart (width / size when that does not fit), the extra unhappy ones
/// recoloured, then its specialists after a 4-px gap.
pub fn draw_citizens(fb: &mut Surface, ctx: &UiContext, city: crate::game::CityId, x: i32, y: i32, width: i32) {
    let g = ctx.game();
    let c = g.city(city);
    let h = g.city_happiness(city);
    let n = c.size as i32;
    let pitch = if n > 0 && 7 * n > width { (width / n).max(1) } else { 7 };
    let mut x = x;
    let mut pos = 0usize;
    for (count, kind) in [(h.happy, 0), (h.content, 1), (h.unhappy, 2)] {
        for i in 0..count.max(0) {
            citizen(fb, ctx, citizen_index(kind == 0, kind == 1, kind == 2, pos), x, y);
            if kind == 2 && i < h.extra {
                recolour(fb, Rect::new(x, y, 12, 14), 5, 12);
            }
            x += pitch;
            pos += 1;
        }
    }
    if !c.specialists.is_empty() {
        x += 4;
        for sp in &c.specialists {
            citizen(fb, ctx, specialist_index(*sp), x, y);
            x += pitch;
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
        let t = self.handle_event(ev, ctx);
        // leaving the city screen ends process_city's mode 1 at 0x131d2, which puts a garrison
        // that was let out back into the record
        if matches!(t, Transition::Pop(_)) {
            ctx.game_mut().restash_garrison(self.city);
        }
        t
    }

    fn resume(&mut self, result: DialogResult, ctx: &mut UiContext) -> Transition {
        let pending = std::mem::replace(&mut self.pending, Pending::None);
        self.advisors = None;
        if ctx.game().city_opt(self.city).is_none() {
            return Transition::Pop(DialogResult::Done);
        }
        let r = match (pending, result) {
            (Pending::Change(list, next), DialogResult::Choice(i)) => match list.get(i) {
                Some(Some(p)) => ctx.game_mut().set_production(self.city, *p),
                Some(None) => return self.change_page(ctx, next),
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
        // drawing the city screen (process_city mode 1) lets the garrison out (0xcda0) and,
        // silently, re-places the workers of a city that has more than citizens (0xd70d)
        let g = ctx.game_mut();
        g.unstash_garrison(self.city);
        if g.city(self.city).workers() > g.city(self.city).size as u32 {
            g.fix_workers(self.city);
        }
        self.draw_title(fb, ctx);
        self.draw_resources(fb, ctx);
        self.draw_support(fb, ctx);
        self.draw_map(fb, ctx);
        self.draw_improvements(fb, ctx);
        self.draw_food(fb, ctx);
        self.draw_tabs(fb, ctx);
        self.draw_production(fb, ctx);
        button(fb, ctx, B_RENAME, "RENAME", ButtonStyle::Plain);
        button(fb, ctx, B_EXIT, "EXIT", ButtonStyle::Exit);
        self.draw_advisors(fb, ctx);
    }

    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.game_palette()
    }

    fn draws_parent(&self) -> bool {
        false
    }
}
