//! The city screen, laid out pixel for pixel after the original (see the measurements in
//! the reverse-engineering report): title and citizens, CITY RESOURCES, unit support, the 5x5
//! city map, improvements with sell dots, FOOD STORAGE, the INFO / HAPPY / MAP / VIEW tabs,
//! the production box with CHANGE / BUY and the RENAME / EXIT buttons.

use crate::game::{CityId, Production, RuleError, Specialist, UnitOrder};
use crate::gfx::{Rect, Surface};
use crate::platform::{Event, Key, MouseButton};
use crate::rules::{self, improvement_ids};
use crate::ui::city_draw::*;
use crate::ui::context::UiContext;
use crate::ui::pedia_data::Entry;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::tilerender::{draw_fog_edges, draw_terrain};
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
    /// 'a' (CIV.EXE 0x118fb..0x11b11): the blinking selection among the units in the city,
    /// moved with the cursor keys; Enter or Space wakes the selected unit, Esc ends it.
    unit_cursor: Option<usize>,
    /// 's' (CIV.EXE 0x11b14..0x11c31): the blinking row among the improvements of the listed
    /// page (wonders excluded), moved with Up/Down; Enter or Space asks to sell it.
    sell_cursor: Option<usize>,
    /// 'p' (0x11cce..0x11f42): the white square on the city map, (dx, dy) from the centre,
    /// moved with the cursor keys; Enter or Space acts as a click on it, Esc ends it.
    tile_cursor: Option<(i32, i32)>,
}

pub fn city_screen(c: CityId) -> Box<dyn Screen> {
    Box::new(CityScreen { city: c, tab: Tab::Info, pending: Pending::None, advisors: None, start: None, imp_start: 0, view_only: false, unit_cursor: None, sell_cursor: None, tile_cursor: None })
}

/// City screen that opens on a tab or dialog: "happy", "map", "view", "change", "buy", "rename", "sell".
pub fn city_screen_at(c: CityId, start: &str) -> Box<dyn Screen> {
    Box::new(CityScreen { city: c, tab: Tab::Info, pending: Pending::None, advisors: None, start: Some(start.to_string()), imp_start: 0, view_only: false, unit_cursor: None, sell_cursor: None, tile_cursor: None })
}

/// City screen of a foreign city investigated by a Diplomat (CIV.EXE o 0x1d6a2).
pub fn city_screen_view(c: CityId) -> Box<dyn Screen> {
    Box::new(CityScreen { city: c, tab: Tab::Info, pending: Pending::None, advisors: None, start: None, imp_start: 0, view_only: true, unit_cursor: None, sell_cursor: None, tile_cursor: None })
}

// ---- layout (coordinates include the 1-px borders) ---------------------------------------

const P_TITLE: Rect = Rect::new(2, 1, 207, 21);
const P_RES: Rect = Rect::new(2, 23, 123, 43);
const P_SUPPORT: Rect = Rect::new(2, 67, 123, 38);
const P_MAP: Rect = Rect::new(127, 23, 82, 82);
const P_IMP: Rect = Rect::new(211, 1, 107, 97);
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
/// The citizens row: draw_citizens(city, 8, 8, specialists, 192) (CIV.EXE 0x10ed0).
const CITIZEN_X: i32 = 8;
const CITIZEN_W: i32 = 192;
const CITIZEN_Y: i32 = 8;
const B_MORE: Rect = Rect::new(287, 88, 29, 9);
/// The city radius in CIV.EXE's order (ds:0x2498 / ds:0x24c2), the centre last.
const EXE_RADIUS: [(i32, i32); 21] = [
    (0, -1), (1, 0), (0, 1), (-1, 0), (1, -1), (1, 1), (-1, 1), (-1, -1), (0, -2), (2, 0), (0, 2),
    (-2, 0), (-1, -2), (1, -2), (2, -1), (2, 1), (1, 2), (-1, 2), (-2, 1), (-2, -1), (0, 0),
];
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
        if let Some(sel) = self.unit_cursor {
            // the selection loop reads only the keyboard (0x119cb)
            let Event::Key { key, down: true, .. } = ev else { return Transition::None };
            let units = self.info_units(ctx);
            let last = units.len().saturating_sub(1) as i32;
            let sel = sel as i32;
            let moved = match key {
                Key::Up | Key::Kp(8) => sel - 6,
                Key::Left | Key::Kp(4) => sel - 1,
                Key::Right | Key::Kp(6) => sel + 1,
                Key::Down | Key::Kp(2) => sel + 6,
                _ => sel,
            }
            .clamp(0, last) as usize;
            self.unit_cursor = Some(moved);
            match key {
                Key::Enter | Key::Space => {
                    if let Some(&id) = units.get(moved) {
                        ctx.game_mut().wake_unit(id);
                    }
                }
                Key::Esc => self.unit_cursor = None,
                _ => {}
            }
            return Transition::None;
        }
        if let Some(sel) = self.sell_cursor {
            let Event::Key { key, down: true, .. } = ev else { return Transition::None };
            let rows = self.sell_rows(ctx);
            match key {
                Key::Up | Key::Kp(8) => self.sell_cursor = Some(sel.saturating_sub(1)),
                Key::Down | Key::Kp(2) => self.sell_cursor = Some((sel + 1).min(rows.len().saturating_sub(1))),
                Key::Enter | Key::Space => {
                    self.sell_cursor = None;
                    if let Some(&(_, id)) = rows.get(sel) {
                        return self.sell(id);
                    }
                }
                // Esc goes on through the key tests and so leaves the city screen (0x11c34)
                Key::Esc => return Transition::Pop(DialogResult::Done),
                _ => {}
            }
            return Transition::None;
        }
        if let Some((dx, dy)) = self.tile_cursor {
            let Event::Key { key, down: true, .. } = ev else { return Transition::None };
            let (mx, my) = match key {
                Key::Left | Key::Kp(4) => (-1, 0),
                Key::Home | Key::Kp(7) => (-1, -1),
                Key::Up | Key::Kp(8) => (0, -1),
                Key::PageUp | Key::Kp(9) => (1, -1),
                Key::Right | Key::Kp(6) => (1, 0),
                Key::End | Key::Kp(1) => (-1, 1),
                Key::Down | Key::Kp(2) => (0, 1),
                Key::PageDown | Key::Kp(3) => (1, 1),
                _ => (0, 0),
            };
            // the square must lie in the city radius and be known to the player (0x11e70)
            let (nx, ny) = (dx + mx, dy + my);
            let g = ctx.game();
            let c = g.city(self.city);
            if nx.abs() <= 2 && ny.abs() <= 2 && nx.abs() + ny.abs() < 4 && g.explored(g.human_slot, crate::game::Map::wrap_x(c.x as i32 + nx), c.y as i32 + ny) {
                self.tile_cursor = Some((nx, ny));
            }
            match key {
                Key::Enter | Key::Space => {
                    let (dx, dy) = self.tile_cursor.unwrap_or((0, 0));
                    self.map_square(ctx, dx, dy);
                }
                Key::Esc => self.tile_cursor = None,
                _ => {}
            }
            return Transition::None;
        }
        match ev {
            // any key the loop does not know leaves the city screen (0x11fd5 -> 0x125c0)
            Event::Key { key, mods, down: true, .. } => match key {
                Key::Unknown => Transition::None,
                Key::Char('i') if !mods.shift => self.tab_click(0),
                Key::Char('h') if !mods.shift => self.tab_click(1),
                Key::Char('m') if !mods.shift => self.tab_click(2),
                Key::Char('v') if !mods.shift => self.view(),
                Key::Char('c') if !mods.shift => self.change(ctx),
                Key::Char('b') if !mods.shift => self.buy(ctx),
                Key::Char('r') if !mods.shift => self.rename(ctx),
                // 's' only while nothing was sold this turn (city flag +6 bit 7), else it
                // leaves the screen like other keys
                Key::Char('s') if !mods.shift && !self.sold_this_turn(ctx) && !self.sell_rows(ctx).is_empty() => {
                    self.sell_cursor = Some(0);
                    Transition::None
                }
                Key::Char('p') if !mods.shift => {
                    self.tile_cursor = Some((0, 0));
                    Transition::None
                }
                // 'M' is MORE when the list has a second page (0x11f45, local flag bit 1)
                Key::Char('m') if mods.shift && self.has_more(ctx) => {
                    self.toggle_more(ctx);
                    Transition::None
                }
                Key::Char('a') if !mods.shift => {
                    // starts on the first unit without orders (flags & 0xcf), else the first
                    let g = ctx.game();
                    let units = self.info_units(ctx);
                    if !units.is_empty() {
                        self.unit_cursor = Some(units.iter().position(|id| matches!(g.unit(*id).order, UnitOrder::None | UnitOrder::Goto | UnitOrder::Skip)).unwrap_or(0));
                    }
                    Transition::None
                }
                Key::Char(d @ '1'..='9') if !mods.shift => self.cycle_specialist(ctx, (*d as u8 - b'1') as usize),
                Key::Char('a') if mods.shift => self.toggle_auto(ctx),
                _ => Transition::Pop(DialogResult::Done),
            },
            Event::MouseButton { button, x, y, down: true } => self.click(ctx, *button, *x, *y),
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
        // the owner's units on the city square in the order of their records (0x101da)
        g.units_of(c.owner).into_iter().filter(|id| g.unit(*id).pos() == c.pos()).take(18).collect()
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

    /// The list has a second page (the MORE button, local flag bit 1 at 0x104d9).
    fn has_more(&self, ctx: &UiContext) -> bool {
        self.imp_start > 0 || self.listed(ctx).1.is_some()
    }

    fn toggle_more(&mut self, ctx: &UiContext) {
        self.imp_start = self.listed(ctx).1.unwrap_or(0);
    }

    /// The improvements of the listed page with their list rows: what 's' steps through.
    fn sell_rows(&self, ctx: &UiContext) -> Vec<(usize, u8)> {
        let (list, _) = self.listed(ctx);
        list.iter().enumerate().filter_map(|(row, p)| if let Production::Improvement(id) = p { Some((row, *id)) } else { None }).collect()
    }

    fn sold_this_turn(&self, ctx: &UiContext) -> bool {
        let g = ctx.game();
        g.player(g.city(self.city).owner).sold_cities.contains(&self.city)
    }

    /// The step of the citizens row, which the click test reuses (0x121e0).
    fn citizen_pitch(n: usize) -> i32 {
        citizen_step(n as i32, CITIZEN_W)
    }

    // ---- actions ---------------------------------------------------------------------------

    fn change(&mut self, ctx: &mut UiContext) -> Transition {
        // CHANGE (the "AUTO." button too) turns AUTO off before the menu opens (CIV.EXE 0x117ae)
        ctx.game_mut().city_mut(self.city).auto_build = false;
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
        let items: Vec<String> = list
            .iter()
            .map(|p| {
                let Some(p) = p else { return "more...".to_string() };
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
        // popup_menu3 at (80, 8) with Civilopedia help (ds:0x2fa0, 0xba14); choose_production
        // sets no row (ds:0x2f9a), so the cursor starts on the first item, not the current one
        // a right-button (or Alt-H) choice shows the item's Civilopedia page and asks again (0xba5b)
        let pages = list
            .iter()
            .map(|p| match p {
                Some(Production::Unit(k)) => Some(Entry::Unit(*k)),
                Some(Production::Improvement(i)) => Some(Entry::Improvement(*i)),
                Some(Production::Wonder(w)) => Some(Entry::Wonder(*w)),
                None => None,
            })
            .collect();
        self.pending = Pending::Change(list, next);
        let menu = MenuDialog::new(vec![title], items).with_pos(80, 8).with_help_entries(pages);
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
        self.map_square(ctx, dx, dy);
    }

    /// A click on the city map square (dx, dy) from the centre (CIV.EXE 0x123ac).
    fn map_square(&mut self, ctx: &mut UiContext, dx: i32, dy: i32) {
        // the city square only recomputes the city (CIV.EXE 0x123ca)
        if dx == 0 && dy == 0 {
            ctx.game_mut().fix_workers(self.city);
            return;
        }
        if let Some(idx) = rules::CITY_RADIUS.iter().position(|&(rx, ry)| (rx as i32, ry as i32) == (dx, dy)) {
            ctx.game_mut().toggle_tile(self.city, idx as u8);
        }
    }

    /// A click on the citizens row (CIV.EXE 0x121c7): below size 5 only the message; else the
    /// specialist (x - 16) / pitch - workers (C division) changes its kind.
    fn citizen_click(&mut self, ctx: &mut UiContext, mx: i32) -> Transition {
        let c = ctx.game().city(self.city);
        if c.size < 5 {
            return self.specialists_need_size_5();
        }
        let n = c.size as usize;
        let workers = (n - c.specialists.len()) as i32;
        let i = (mx - 16) / Self::citizen_pitch(n) - workers;
        if i < 0 {
            return Transition::None;
        }
        self.cycle_specialist(ctx, i as usize)
    }

    /// The 0x13d54 message: specialists other than entertainers need size 5.
    fn specialists_need_size_5(&mut self) -> Transition {
        self.pending = Pending::Ignore;
        let lines = ["A City must have five", "population units to support", "taxmen or scientists."];
        Transition::Push(Box::new(MessageBox::new(lines.iter().map(|l| l.to_string()).collect()).with_pos(32, 32)))
    }

    /// Entertainer -> taxman -> scientist -> entertainer for specialist `si`, as a click or the
    /// keys '1'..'9' do (CIV.EXE 0x11c34..0x11cb3, 0x12217); refused below size 5.
    fn cycle_specialist(&mut self, ctx: &mut UiContext, si: usize) -> Transition {
        let c = ctx.game().city(self.city);
        let Some(&s) = c.specialists.get(si) else { return Transition::None };
        if c.size < 5 {
            return self.specialists_need_size_5();
        }
        let next = match s {
            Specialist::Entertainer => Specialist::Taxman,
            Specialist::Taxman => Specialist::Scientist,
            Specialist::Scientist => Specialist::Entertainer,
        };
        ctx.game_mut().set_specialist(self.city, si, next);
        Transition::None
    }

    /// A mouse press on the city screen, tested in the order of the input loop (CIV.EXE
    /// 0x11391..0x125c0). Whatever no test takes leaves the screen (0x125c0), the right button
    /// everywhere but on CHANGE, where it flips AUTO.
    fn click(&mut self, ctx: &mut UiContext, button: MouseButton, x: i32, y: i32) -> Transition {
        let band = (106..=116).contains(&y);
        if button == MouseButton::Right {
            // 0x1139b: a right click on CHANGE acts as Shift-A
            if band && (230..270).contains(&x) {
                return self.toggle_auto(ctx);
            }
            return Transition::Pop(DialogResult::Done);
        }
        if band {
            // 0x114e8, 0x11786, 0x11889: BUY from x 296, CHANGE at 230..269, the tabs 32 px
            // wide from x 96
            if x >= 296 {
                return self.buy(ctx);
            }
            if (230..270).contains(&x) {
                return self.change(ctx);
            }
            if (96..224).contains(&x) {
                return self.tab_click(((x - 96) / 32) as usize);
            }
        }
        // 0x11f8b: RENAME
        if (231..270).contains(&x) && y > 190 {
            return self.rename(ctx);
        }
        // MORE flips to the overflow page and back (CIV.EXE 0x11fd8)
        if self.has_more(ctx) && x >= 288 && y > 88 && y < 96 {
            self.toggle_more(ctx);
            return Transition::None;
        }
        // sell dots (0x1201f): row (y - 2) / 6, truncated as in C
        if x >= 300 && y < 94 {
            let (list, _) = self.listed(ctx);
            // the Palace too; nothing once the city has sold this turn (city flag 0x80)
            if let Some(Production::Improvement(id)) = list.get(((y - 2) / 6) as usize).copied() {
                if !self.sold_this_turn(ctx) {
                    return self.sell(id);
                }
            }
        }
        // 0x121b2: the citizens row
        if x < 200 && y < 20 {
            return self.citizen_click(ctx, x);
        }
        // a click on one of the city's units on the INFO tab wakes it (0x12271; the original
        // finds the unit with a 16 px column pitch although they are drawn 18 px apart, and
        // its right bound 0x2580 lets columns 7..13 wake units of later rows: civrs takes the
        // columns up to the drawn units' right edge only)
        if matches!(self.tab, Tab::Info) && (100..211).contains(&x) && y >= 116 {
            let i = ((y - 116) / 16 * 6 + (x - 100) / 16) as usize;
            if let Some(&id) = self.info_units(ctx).get(i) {
                ctx.game_mut().wake_unit(id);
                return Transition::None;
            }
        }
        // 0x12366: rows 24..103 are the city map, squares outside the radius do nothing
        if (24..104).contains(&y) {
            self.map_click(ctx, x, y);
            return Transition::None;
        }
        Transition::Pop(DialogResult::Done)
    }

    /// Shift-A or a right click on CHANGE: AUTO on asks the production advisor at once; with
    /// nothing worth it the flag stays until the city finishes its item (CIV.EXE 0x113d7..0x11411).
    fn toggle_auto(&mut self, ctx: &mut UiContext) -> Transition {
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
        let row = CitizenRow { size: c.size as i32, happy: h.happy, content: h.content, unhappy: h.unhappy, extra: h.extra, specialists: &c.specialists };
        row.draw(fb, ctx, CITIZEN_X, CITIZEN_Y, CITIZEN_W);
    }

    fn draw_resources(&self, fb: &mut Surface, ctx: &UiContext) {
        panel(fb, ctx, P_RES);
        header(fb, ctx, Rect::new(P_RES.x, P_RES.y, P_RES.w, 9), "CITY RESOURCES");
        let g = ctx.game();
        let y = g.city_yield(self.city);
        // CIV.EXE 0x10a1b..0x10b80: food, shields and trade in rows 8 px apart from (4, 33),
        // 116 / (n + 1) px apart (at most 8), a 4 px gap after the food the citizens eat and
        // after the shields that go to units, 2 px between the trade kept and the corruption
        let support = y.shield_upkeep;
        let rows = [(ICON_FOOD, y.food, y.food.max(y.food_upkeep).min(999)), (ICON_SHIELD, y.shields, y.shields), (ICON_TRADE, y.trade, y.trade)];
        for (r, (src, n, spread)) in rows.into_iter().enumerate() {
            let pitch = (116 / (spread + 1)).clamp(1, 8);
            let ry = 33 + 8 * r as i32;
            let mut x = 4;
            for i in 0..n {
                x += match r {
                    0 if i == y.food_upkeep => 4,
                    1 if support != 0 && i == support => 4,
                    2 if i == y.trade - y.corruption => 2,
                    _ => 0,
                };
                icon(fb, ctx, src, x, ry);
                x += pitch;
            }
        }
        // luxuries, taxes and science at y 57 (0x10b83..0x10ca4), on a half-pixel grid of
        // 224 / (lux + tax + sci + corruption + 2) (at most 16) from 8, 8 more after a group
        let pitch = (224 / (y.lux + y.tax + y.sci + y.corruption + 2)).clamp(1, 16);
        let mut x2 = 8;
        for (n, src) in [(y.lux, ICON_LUX), (y.tax, ICON_TAX), (y.sci, ICON_SCI)] {
            for _ in 0..n {
                icon(fb, ctx, src, x2 / 2, 57);
                x2 += pitch;
            }
            if n != 0 {
                x2 += 8;
            }
        }
        // shortages (0x10ca4..0x10ea6): the food and shields missing, drawn where a surplus
        // would be and turned black (white 15 -> 0), and the corruption blackened
        if y.food_upkeep > y.food {
            let pitch = (116 / (y.food_upkeep + 1)).clamp(1, 8);
            for i in y.food..y.food_upkeep {
                icon(fb, ctx, ICON_FOOD, i * pitch + 8, 33);
            }
            recolour(fb, Rect::new(pitch * y.food + 8, 33, (y.food_upkeep - y.food) * pitch + 4, 8), WHITE, BLACK);
        }
        if y.shields < support {
            // the pitch of the shields produced, not of the support (0x10db7)
            let pitch = (116 / (y.shields + 1)).clamp(1, 8);
            for i in y.shields..support {
                icon(fb, ctx, ICON_SHIELD, i * pitch + 8, 41);
            }
            recolour(fb, Rect::new(pitch * y.shields + 8, 41, (support - y.shields) * pitch, 8), WHITE, BLACK);
        }
        if y.corruption != 0 {
            let pitch = (116 / (y.trade + 1)).clamp(1, 8);
            recolour(fb, Rect::new((y.trade - y.corruption) * pitch + 6, 49, pitch * y.corruption + 2, 8), WHITE, BLACK);
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
                icon(fb, ctx, ICON_SHIELD, x + 8, y + 12);
            }
            let (src, n) = if u.kind == rules::unit_ids::SETTLERS {
                (ICON_FOOD, govt.settler_food as i32)
            } else {
                (ICON_UNHAPPY, g.unit_unhappiness(self.city, u))
            };
            for k in 0..n.min(2) {
                icon(fb, ctx, src, x + 2 * k, y + 12);
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

    /// `draw_tile` (0x1aa40) of a city map square: terrain, the fringe towards squares the
    /// human has not seen and a city.
    fn draw_square(&self, fb: &mut Surface, ctx: &UiContext, tx: i32, ty: i32, px: i32, py: i32) {
        let g = ctx.game();
        draw_terrain(fb, ctx, g, tx, ty, px, py);
        draw_fog_edges(fb, ctx, g, g.human_slot, tx, ty, px, py);
        if let Some(city) = g.city_at(tx, ty) {
            crate::ui::tilerender::draw_city_in_font(fb, ctx, g, city, px, py, SMALL);
        }
    }

    /// The squares of the 5x5 city map that other cities work (CIV.EXE 0xca55..0xcbf7): those
    /// of every city at most 5 away, its centre included, indexed [dx + 2][dy + 2]. (The
    /// original compares the x coordinates without wrapping them; civrs wraps.)
    fn taken_squares(&self, ctx: &UiContext) -> [[bool; 5]; 5] {
        let g = ctx.game();
        let c = g.city(self.city);
        let mut taken = [[false; 5]; 5];
        for other in g.all_cities() {
            if other.pos() == c.pos() || crate::game::Map::civ_distance(c.pos(), other.pos()) > 5 {
                continue;
            }
            for (idx, &(rx, ry)) in rules::CITY_RADIUS.iter().enumerate() {
                if !other.works(idx as u8) {
                    continue;
                }
                let dx = crate::game::Map::delta_x(c.x, other.x + rx as i32);
                let dy = other.y + ry as i32 - c.y;
                if dx.abs() <= 2 && dy.abs() <= 2 {
                    taken[(dx + 2) as usize][(dy + 2) as usize] = true;
                }
            }
        }
        taken
    }

    fn draw_map(&self, fb: &mut Surface, ctx: &UiContext) {
        use crate::game::Map;
        panel(fb, ctx, P_MAP);
        let g = ctx.game();
        let c = g.city(self.city);
        let owner = c.owner;
        let at = |dx: i32, dy: i32| (Map::wrap_x(c.x + dx), c.y + dy, MAP_ORIGIN.0 + (dx + 2) * 16, MAP_ORIGIN.1 + (dy + 2) * 16);
        // CIV.EXE 0xcebd..0xd004: the centre, then the 21 squares in the original's radius
        // order; a human city leaves the squares its owner has not seen to the panel
        // background (a foreign one shows them all); a square with units of another civ
        // shows them as the map does; squares other cities work get a colour 12 outline
        let (tx, ty, px, py) = at(0, 0);
        self.draw_square(fb, ctx, tx, ty, px, py);
        let taken = self.taken_squares(ctx);
        for (dx, dy) in EXE_RADIUS {
            let (tx, ty, px, py) = at(dx, dy);
            if !Map::in_bounds(tx, ty) || (owner == g.human_slot && !g.explored(owner, tx, ty)) {
                continue;
            }
            self.draw_square(fb, ctx, tx, ty, px, py);
            if g.unit_owner_at(tx, ty).is_some_and(|o| o != owner) {
                crate::ui::tilerender::draw_tile_units(fb, ctx, g, g.human_slot, tx, ty, px, py, None, SMALL);
                continue;
            }
            if taken[(dx + 2) as usize][(dy + 2) as usize] {
                fb.rect_outline(Rect::new(px, py, 16, 16), RED);
            }
        }
        // the worked squares, in the same order (0xd684 -> work_tile 0x13233): the square is
        // drawn again, then its food, shields and trade from (+1, +1), 8, 5 or 3 px apart
        // for up to 4, 6 or more items and 8 px between rows, which may run into the square
        // below; an empty square shows icon 13 at (+5, +5)
        for (dx, dy) in EXE_RADIUS {
            let idx = rules::CITY_RADIUS.iter().position(|&r| r == (dx as i8, dy as i8)).unwrap() as u8;
            if !c.works(idx) {
                continue;
            }
            let Some((tx, ty)) = g.radius_tile(self.city, idx) else { continue };
            let (_, _, px, py) = at(dx, dy);
            self.draw_square(fb, ctx, tx, ty, px, py);
            for (src, ox, oy) in work_tile_icons(g.tile_yield(owner, tx, ty, Some(self.city))) {
                icon(fb, ctx, src, px + ox, py + oy);
            }
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
        // the 's' row blinks: colour 9 -> 1, 10 ticks, 1 -> 9, 10 ticks (0x11b36)
        if let Some(sel) = self.sell_cursor {
            if let Some(&(row, _)) = self.sell_rows(ctx).get(sel) {
                if (ctx.frame / 16) % 2 == 0 {
                    recolour(fb, Rect::new(252, 6 * row as i32 + 3, 56, 7), 1, 9);
                }
            }
        }
    }

    fn draw_food(&self, fb: &mut Surface, ctx: &UiContext) {
        // CIV.EXE 0x10866..0x109c0: a colour 1 box (2, 106, 91, 80 / f * f + 12) for the food
        // factor f (10 for the human), "FOOD STORAGE" at (8, 108), the stippled store (3, 115,
        // size * dx + 9, 80 / f * f + 2) with dx = clamp(80 / size, 1, 8), and the food in rows
        // of size + 1 from (4, 116), 80 / f px apart
        let g = ctx.game();
        let c = g.city(self.city);
        let size = (c.size as i32).max(1);
        let factor = g.cost_factor(c.owner).max(1);
        let dx = (80 / size).clamp(1, 8);
        let dy = 80 / factor;
        fb.fill_rect(Rect::new(2, 106, 91, factor * dy + 12), BORDER);
        small_text(fb, ctx, 8, 108, WHITE, "FOOD STORAGE");
        fill_stipple(fb, ctx, Rect::new(3, 115, size * dx + 9, factor * dy + 2));
        // a granary marks the food kept after growth with a line across the middle (0x1090a)
        if c.has_improvement(improvement_ids::GRANARY) {
            fb.hline(5, size * dx + 9, 155, BORDER);
        }
        for i in 0..c.food.clamp(0, (size + 1) * factor) {
            icon(fb, ctx, ICON_FOOD, (i % (size + 1)) * dx + 4, (i / (size + 1)) * dy + 116);
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
            button(fb, ctx, *r, TAB_LABELS[i], ButtonStyle::Plain);
        }
        // the current tab's face turns white: recolour 9 -> 15 at (0x60 + 0x21 tab, 107), 32x7
        // (CIV.EXE 0xf03d)
        recolour(fb, Rect::new(96 + 33 * sel, 107, 32, 7), LIGHT, WHITE);
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
            // the 'a' selection blinks: background, 10 ticks, the unit, 10 ticks (0x1197e)
            if self.unit_cursor != Some(i) || (ctx.frame / 16) % 2 == 0 {
                crate::ui::tilerender::draw_unit_in_font(fb, ctx, g, *id, x, y, false, SMALL);
            }
            let home = u.home.and_then(|h| g.city_opt(h)).map(|c| c.name.to_ascii_uppercase()).unwrap_or_else(|| "NONE".into());
            let abbrev: String = home.chars().take(3).chain(std::iter::once('.')).collect();
            small_text(fb, ctx, x, y + 15, BLACK, &abbrev);
        }
        // the trade routes (0x10f05..0x11073): "<city>:+<n>} " at (98, 179 + 6 slot) in colour
        // 10, n = (both cities' stored trade + 4) / 8, or / 16 between cities of one civilization
        for (i, r) in c_routes(g, self.city).into_iter().enumerate() {
            small_text(fb, ctx, 98, 179 + 6 * i as i32, 10, &r);
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
        let g = ctx.game();
        let c = g.city(self.city);
        let h = g.city_happiness(self.city);
        // draw_citizens(city, 100, y, specialists, 92) (0xf51b); the pictograms of the
        // buildings (0xf5d4), the martial law units (0xf8c3) and the unhappy faces of units
        // abroad (0xfa1d) go first, 16, 2 and 2 px apart leftwards from x 208, and the
        // citizens over them; the luxury icon (0xf5ac) and WONDERS (0xfbab) come after
        for (r, step) in h.steps.iter().enumerate().take(5) {
            let y = 116 + 16 * r as i32;
            match &step.row {
                HappyRow::Buildings(ids) => {
                    for (i, id) in ids.iter().enumerate() {
                        if let Some(cell) = improvement_cell(*id) {
                            imp_icon(fb, ctx, cell, 208 - 16 * i as i32, y);
                        }
                    }
                }
                HappyRow::MartialLaw(kinds) => {
                    for (i, k) in kinds.iter().enumerate() {
                        unit_box(fb, ctx, *k, c.owner, 208 - 2 * i as i32, y - 1);
                    }
                }
                HappyRow::Military(faces) => {
                    for i in 0..*faces {
                        icon(fb, ctx, ICON_UNHAPPY, 208 - 2 * i, y + 4);
                    }
                }
                _ => {}
            }
            let row = CitizenRow { size: c.size as i32, happy: step.happy, content: step.content, unhappy: step.unhappy, extra: step.extra, specialists: &c.specialists };
            row.draw(fb, ctx, 100, y, 92);
            match &step.row {
                HappyRow::Luxuries => icon(fb, ctx, ICON_LUX, 208, y + 4),
                HappyRow::Wonders => {
                    small_text(fb, ctx, 190, y + 5, WHITE, "WONDERS");
                }
                _ => {}
            }
            fb.hline(100, 222, y + 14, BORDER);
        }
    }

    /// The MAP tab, `draw_city_minimap` (0x13bbd): a black box (101, 117, 120, 75), each
    /// square the human has seen as a 2x2 block, columns from the human's capital x - 40,
    /// at 3/2 scale with the offsets ds:0x280c / ds:0x2810 by the parity of column and row,
    /// colour 1 for Ocean and 2 for land, and a colour 9 outline (100, 117)-(221, 192); then
    /// the dots of `minimap_dot` (0x13cf0): the supported units in colour 7 (0x101a9), the
    /// cities at the ends of the trade routes in 10 (0x11042) and the city itself in 15
    /// (0x1109b).
    fn draw_minimap(&self, fb: &mut Surface, ctx: &UiContext) {
        const DX: [i32; 4] = [0, 2, 0, 1];
        const DY: [i32; 4] = [-1, 0, 1, 2];
        let g = ctx.game();
        let c = g.city(self.city);
        let human = g.human_slot;
        // ds:0xdcec[civ], the capital's x (no capital: 0)
        let cap = g.player(human).capital.and_then(|id| g.city_opt(id)).map_or(0, |k| k.x);
        fb.fill_rect(Rect::new(101, 117, 120, 75), BLACK);
        for y in 0..rules::MAP_H as i32 {
            for col in 0..rules::MAP_W as i32 {
                let x = crate::game::Map::wrap_x(cap + col - 40);
                if !g.explored(human, x, y) {
                    continue;
                }
                let d = ((y & 1) * 2 + (col & 1)) as usize;
                let colour = if g.map.tile(x, y).terrain == rules::Terrain::Ocean { BORDER } else { 2 };
                fb.fill_rect(Rect::new(col / 2 * 3 + DX[d] + 101, y / 2 * 3 + DY[d] + 118, 2, 2), colour);
            }
        }
        fb.rect_outline(Rect::new(100, 117, 122, 76), LIGHT);
        let dot = |fb: &mut Surface, x: i32, y: i32, colour: u8| {
            let col = crate::game::Map::wrap_x(x - cap + 40);
            fb.fill_rect(Rect::new(3 * col / 2 + 101, 3 * y / 2 + 118, 2, 2), colour);
        };
        for (id, _) in g.shield_costs(self.city) {
            let u = g.unit(id);
            dot(fb, u.x, u.y, GREY);
        }
        for r in c.routes() {
            if let Some(other) = g.city_opt(r) {
                dot(fb, other.x, other.y, 10);
            }
        }
        dot(fb, c.x, c.y, WHITE);
    }

    fn draw_production(&self, fb: &mut Surface, ctx: &UiContext) {
        let g = ctx.game();
        let c = g.city(self.city);
        let factor = g.cost_factor(c.owner);
        let (k, step, r) = stock_layout(c.production.cost() / 10, c.shields, factor);
        fb.fill_rect(r, BORDER);
        fill_stipple(fb, ctx, Rect::new(231, 116, r.w - 2, r.h - 18));
        // CHANGE (or AUTO.) and BUY on the human's cities only (0x106fb)
        if c.owner == g.human_slot {
            button(fb, ctx, B_CHANGE, if c.auto_build { "AUTO." } else { "CHANGE" }, ButtonStyle::Plain);
            // production that cannot be completed: the button face flashes yellow, colours 9
            // and 14 swapping every 10 ticks (CIV.EXE 0x112e1)
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
        }
        match c.production {
            Production::Unit(u) => unit_box(fb, ctx, u, c.owner, ITEM_FRAME.0, ITEM_FRAME.1),
            // improvements and wonders by name, centred over the buttons (0x1079f)
            other => {
                small_text_centered(fb, ctx, 274, 100, WHITE, &fit_small_text(ctx, other.name(), 86));
            }
        }
        let per_row = factor * k;
        for i in 0..c.shields.max(0) {
            icon(fb, ctx, ICON_SHIELD, 232 + (i % per_row) * step / k, 117 + (i / per_row) * 8);
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

/// The icons `work_tile` (0x13233, mode 1) puts on a worked square of the city map, with
/// their offsets from the square's corner: the food, shields and trade from (1, 1), 8, 5 or
/// 3 px apart for up to 4, 6 or more items, a new row 8 px down once a row's x has passed 8
/// px; an empty square gets icon 13 at (5, 5).
fn work_tile_icons((f, s, t): (i32, i32, i32)) -> Vec<((i32, i32), i32, i32)> {
    let total = f.max(0) + s.max(0) + t.max(0);
    if total == 0 {
        return vec![(ICON_UNHAPPY, 5, 5)];
    }
    let step = if total <= 4 { 8 } else if total <= 6 { 5 } else { 3 };
    let (mut ox, mut oy) = (0, 0);
    let mut v = Vec::new();
    for (src, n) in [(ICON_FOOD, f), (ICON_SHIELD, s), (ICON_TRADE, t)] {
        for _ in 0..n {
            v.push((src, 1 + ox, 1 + oy));
            if ox < 8 {
                ox += step;
            } else {
                ox = 0;
                oy += 8;
            }
        }
    }
    v
}

/// The trade route lines of the INFO tab (CIV.EXE 0x10f05): "<city>:+<n>} ".
fn c_routes(g: &crate::game::Game, city: CityId) -> Vec<String> {
    let c = g.city(city);
    c.routes()
        .filter_map(|r| {
            let other = g.city_opt(r)?;
            let sum = other.trade + c.trade + 4;
            let n = if other.owner == c.owner { sum / 16 } else { sum / 8 };
            Some(format!("{}:+{}}} ", other.name.to_ascii_uppercase(), n))
        })
        .take(3)
        .collect()
}

/// The step of `draw_citizens` (0xc91:0x6eca, 0x137ef): 7 px, or width / size when seven
/// per citizen would be wider than `width` (C division).
pub fn citizen_step(size: i32, width: i32) -> i32 {
    if 7 * size > width {
        width / size
    } else {
        7
    }
}

/// What `draw_citizens` draws: `size` sets the step, then the groups and the specialists.
struct CitizenRow<'a> {
    size: i32,
    happy: i32,
    content: i32,
    unhappy: i32,
    /// The first unhappy figures shown in red (ds:0x6542).
    extra: i32,
    specialists: &'a [Specialist],
}

/// The x of each figure in a citizens row from `x` with step `step`: the happy citizens, 2 px,
/// the content ones, 2 px more when there were any, the unhappy ones, 4 px, the specialists
/// (0x13816..0x13942). The first 2-px gap tests a local the original never sets (0x13845
/// `[bp-2]`); the city screen captures show it taken (content figures from x + 2 with no
/// happy ones), so it is always taken here.
fn citizen_xs(x: i32, step: i32, happy: i32, content: i32, unhappy: i32, specialists: i32) -> Vec<i32> {
    let mut out = Vec::new();
    let mut x = x;
    for _ in 0..happy.max(0) {
        out.push(x);
        x += step;
    }
    x += 2;
    for _ in 0..content.max(0) {
        out.push(x);
        x += step;
    }
    if content > 0 {
        x += 2;
    }
    for _ in 0..unhappy.max(0) {
        out.push(x);
        x += step;
    }
    x += 4;
    for _ in 0..specialists.max(0) {
        out.push(x);
        x += step;
    }
    out
}

impl CitizenRow<'_> {
    /// Figures alternate male / female within each group (index & 1, 0x13819), the extra
    /// unhappy ones get recolour(x, y, 12, 14, 5 -> 12) (0x138eb), specialists show their
    /// kind's figure (ds:0x6ea0[type]).
    fn draw(&self, fb: &mut Surface, ctx: &UiContext, x: i32, y: i32, width: i32) {
        let step = citizen_step(self.size.max(1), width);
        let xs = citizen_xs(x, step, self.happy, self.content, self.unhappy, self.specialists.len() as i32);
        let mut xs = xs.into_iter();
        for (count, kind) in [(self.happy, 0), (self.content, 1), (self.unhappy, 2)] {
            for i in 0..count.max(0) {
                let Some(x) = xs.next() else { return };
                citizen(fb, ctx, citizen_index(kind == 0, kind == 1, kind == 2, i as usize), x, y);
                if kind == 2 && i < self.extra {
                    recolour(fb, Rect::new(x, y, 12, 14), 5, 12);
                }
            }
        }
        for (s, x) in self.specialists.iter().zip(xs) {
            citizen(fb, ctx, specialist_index(*s), x, y);
        }
    }
}

/// `draw_citizens(city, x, y, .., width)` (0xc91:0x6eca): the city's happy, content and
/// unhappy citizens and its specialists as [`citizen_xs`] places them.
pub fn draw_citizens(fb: &mut Surface, ctx: &UiContext, city: crate::game::CityId, x: i32, y: i32, width: i32) {
    let g = ctx.game();
    let c = g.city(city);
    let h = g.city_happiness(city);
    let row = CitizenRow { size: c.size as i32, happy: h.happy, content: h.content, unhappy: h.unhappy, extra: h.extra, specialists: &c.specialists };
    row.draw(fb, ctx, x, y, width);
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
        // and stores the city's trade less corruption (0xf13d)
        g.note_trade(self.city);
        self.draw_title(fb, ctx);
        self.draw_resources(fb, ctx);
        self.draw_support(fb, ctx);
        self.draw_map(fb, ctx);
        // the 'p' square blinks in a white outline (0x11d2b: outline(x, y, 15, 15, 15))
        if let Some((dx, dy)) = self.tile_cursor {
            if (ctx.frame / 16) % 2 == 0 {
                fb.rect_outline(Rect::new(MAP_ORIGIN.0 + (dx + 2) * 16, MAP_ORIGIN.1 + (dy + 2) * 16, 16, 16), 15);
            }
        }
        self.draw_improvements(fb, ctx);
        self.draw_food(fb, ctx);
        self.draw_tabs(fb, ctx);
        self.draw_production(fb, ctx);
        // EXIT and RENAME on the human's cities only (0x11158)
        if ctx.game().city(self.city).owner == ctx.game().human_slot {
            button(fb, ctx, B_RENAME, "RENAME", ButtonStyle::Plain);
            button(fb, ctx, B_EXIT, "EXIT", ButtonStyle::Exit);
        }
        self.draw_advisors(fb, ctx);
    }

    /// The map's water cycles go on behind the city screen: the timer ISR turns them until
    /// something stops them (0x1d43:0x655), and process_city does not.
    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        crate::ui::mapview::water_cycles(ctx.game_palette(), ctx.frame)
    }

    fn draws_parent(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Game, GameSetup};
    use crate::platform::Mods;

    #[test]
    fn citizens_row_follows_draw_citizens() {
        // 7 px until 7 * size passes the width, then width / size
        assert_eq!(citizen_step(27, 192), 7);
        assert_eq!(citizen_step(28, 192), 6);
        assert_eq!(citizen_step(14, 92), 6);
        assert_eq!(citizen_step(13, 92), 7);
        // content only: 2 px in (the unset local's gap), as the captures show
        assert_eq!(citizen_xs(8, 7, 0, 1, 0, 0), vec![10]);
        // 2 happy, 2 content, 1 unhappy, 1 specialist
        assert_eq!(citizen_xs(8, 7, 2, 2, 1, 1), vec![8, 15, 24, 31, 40, 51]);
        // no content: no second gap
        assert_eq!(citizen_xs(100, 7, 1, 0, 1, 0), vec![100, 109]);
    }

    #[test]
    fn worked_squares_pack_their_icons_as_work_tile() {
        let at = |v: Vec<((i32, i32), i32, i32)>| v.into_iter().map(|(_, x, y)| (x, y)).collect::<Vec<_>>();
        // up to 4: two to a row, 8 px apart
        assert_eq!(at(work_tile_icons((2, 1, 1))), vec![(1, 1), (9, 1), (1, 9), (9, 9)]);
        // 5 or 6: three to a row, 5 px apart
        assert_eq!(at(work_tile_icons((3, 1, 1))), vec![(1, 1), (6, 1), (11, 1), (1, 9), (6, 9)]);
        // more: four to a row, 3 px apart, the third row running into the square below
        assert_eq!(at(work_tile_icons((4, 1, 4))).last(), Some(&(1, 17)));
        assert_eq!(work_tile_icons((2, 0, 1))[2].0, ICON_TRADE);
        // nothing: the "0" icon in the middle
        assert_eq!(work_tile_icons((0, 0, 0)), vec![(ICON_UNHAPPY, 5, 5)]);
    }

    #[test]
    fn city_screen_units_are_the_map_icons() {
        let Some((mut ctx, cs)) = screen() else { return };
        let (c, slot) = (cs.city, ctx.game().human_slot);
        let (x, y) = ctx.game().city(c).pos();
        let g = ctx.game_mut();
        for kind in [rules::unit_ids::MILITIA, rules::unit_ids::CHARIOT] {
            let id = g.add_unit(slot, kind, x, y);
            g.unit_mut(id).home = Some(c);
        }
        g.city_mut(c).production = Production::Unit(rules::unit_ids::CATAPULT);
        let mut fb = Surface::screen();
        let mut cs = cs;
        cs.draw(&mut fb, &mut ctx);
        // the supported units at (8, 69) and (24, 69) and the production at (264, 100) are
        // draw_icon's map icons (0x1017d, 0x10794): drawing them again changes nothing
        let mut again = fb.clone();
        let g = ctx.game();
        let kinds: Vec<u8> = g.shield_costs(c).iter().map(|(id, _)| g.unit(*id).kind).collect();
        for (i, k) in kinds.iter().enumerate() {
            crate::ui::tilerender::draw_unit_kind(&mut again, &ctx, *k, slot, 8 + 16 * i as i32, 69);
        }
        crate::ui::tilerender::draw_unit_kind(&mut again, &ctx, rules::unit_ids::CATAPULT, slot, 264, 100);
        assert_eq!(again.px, fb.px);
        // and not the old 16x16 cells one pixel up and left
        let mut cell = fb.clone();
        crate::ui::tilerender::draw_unit_kind(&mut cell, &ctx, rules::unit_ids::CATAPULT, slot, 263, 99);
        assert_ne!(cell.px, fb.px);
    }

    #[test]
    fn squares_other_cities_work_are_marked() {
        let Some((mut ctx, cs)) = screen() else { return };
        let g = ctx.game_mut();
        let (x, y) = g.city(cs.city).pos();
        // a second city 4 east working the square 2 west of it (2 east of ours)
        let land = (3..=5).find(|dx| g.map.is_land(x + dx, y) && g.city_at(x + dx, y).is_none());
        let dx = land.expect("land east of the city");
        let s = g.add_unit(g.human_slot, rules::unit_ids::SETTLERS, x + dx, y);
        let other = g.found_city(s).expect("a second city");
        let idx = rules::CITY_RADIUS.iter().position(|&r| r == ((2 - dx) as i8, 0)).unwrap() as u8;
        g.city_mut(other).worked = 0;
        g.city_mut(other).set_worked(idx, true);
        let taken = cs.taken_squares(&ctx);
        assert!(taken[4][2], "the square 2 east is worked by the other city");
        assert!(!taken[0][2]);
    }

    #[test]
    fn names_are_fitted_on_their_full_advances() {
        let Some((ctx, _)) = screen() else { return };
        assert_eq!(fit_small_text(&ctx, "MASS TRANSIT", 56), "MASS TRANS.");
        assert_eq!(fit_small_text(&ctx, "GRANARY", 56), "GRANARY");
    }

    fn key(k: Key) -> Event {
        Event::Key { key: k, mods: Mods::default(), down: true, repeat: false }
    }

    fn press(button: MouseButton, x: i32, y: i32) -> Event {
        Event::MouseButton { button, x, y, down: true }
    }

    #[test]
    fn clicks_nobody_takes_leave_the_city_screen() {
        let Some((mut ctx, mut cs)) = screen() else { return };
        // the food box and a right click off CHANGE end the screen (0x125c0)
        assert!(matches!(cs.handle_event(&press(MouseButton::Left, 40, 150), &mut ctx), Transition::Pop(_)));
        assert!(matches!(cs.handle_event(&press(MouseButton::Right, 150, 60), &mut ctx), Transition::Pop(_)));
        // the city map rows never leave, even outside the radius
        assert!(matches!(cs.handle_event(&press(MouseButton::Left, 20, 60), &mut ctx), Transition::None));
    }

    #[test]
    fn a_right_click_on_change_flips_auto() {
        let Some((mut ctx, mut cs)) = screen() else { return };
        assert!(!ctx.game().city(cs.city).auto_build);
        assert!(matches!(cs.handle_event(&press(MouseButton::Right, 240, 110), &mut ctx), Transition::None));
        assert!(ctx.game().city(cs.city).auto_build);
    }

    #[test]
    fn button_band_regions() {
        let Some((mut ctx, mut cs)) = screen() else { return };
        // the tabs are 32 px wide from x 96 (0x1189e)
        cs.handle_event(&press(MouseButton::Left, 128, 116), &mut ctx);
        assert!(matches!(cs.tab, Tab::Happy));
        cs.handle_event(&press(MouseButton::Left, 191, 106), &mut ctx);
        assert!(matches!(cs.tab, Tab::Map));
        assert!(matches!(cs.handle_event(&press(MouseButton::Left, 95, 110), &mut ctx), Transition::Pop(_)), "x 95 is no tab");
        // BUY reaches to the right edge (x >= 296)
        assert!(matches!(cs.handle_event(&press(MouseButton::Left, 316, 116), &mut ctx), Transition::Push(_)));
    }

    /// A context with a new human city and its screen.
    fn screen() -> Option<(UiContext, CityScreen)> {
        let dir = crate::assets::AssetDir::locate(None).ok()?;
        let mut ctx = UiContext::new(dir, std::env::temp_dir().join(format!("civrs-citykeys-{}", std::process::id()))).ok()?;
        let mut g = Game::new(GameSetup { seed: 3, ..Default::default() }, None);
        let settlers = g.units_of(g.human_slot).into_iter().find(|u| g.unit(*u).kind == rules::unit_ids::SETTLERS)?;
        let c = g.found_city(settlers).ok()?;
        let _ = g.take_events();
        ctx.game = Some(g);
        Some((ctx, CityScreen { city: c, tab: Tab::Info, pending: Pending::None, advisors: None, start: None, imp_start: 0, view_only: false, unit_cursor: None, sell_cursor: None, tile_cursor: None }))
    }

    #[test]
    fn p_moves_a_square_over_the_city_map_and_enter_works_it() {
        let Some((mut ctx, mut cs)) = screen() else { return };
        let idx = rules::CITY_RADIUS.iter().position(|&(x, y)| (x, y) == (1, 0)).unwrap() as u8;
        let mut expected = ctx.game().clone();
        expected.toggle_tile(cs.city, idx);
        cs.handle_event(&key(Key::Char('p')), &mut ctx);
        assert_eq!(cs.tile_cursor, Some((0, 0)));
        cs.handle_event(&key(Key::Right), &mut ctx);
        assert_eq!(cs.tile_cursor, Some((1, 0)));
        assert!(matches!(cs.handle_event(&key(Key::Enter), &mut ctx), Transition::None));
        assert_eq!(ctx.game().city(cs.city).worked, expected.city(cs.city).worked);
        assert_eq!(cs.tile_cursor, Some((1, 0)), "the square stays for the next one");
        // the corners of the 5x5 square are outside the radius
        let g = ctx.game_mut();
        let (x, y) = (g.city(cs.city).x as i32, g.city(cs.city).y as i32);
        let slot = g.human_slot;
        g.map.explore(slot, x, y, 2);
        for _ in 0..3 {
            cs.handle_event(&key(Key::PageUp), &mut ctx);
        }
        assert_eq!(cs.tile_cursor, Some((2, -1)));
        cs.handle_event(&key(Key::Esc), &mut ctx);
        assert_eq!(cs.tile_cursor, None);
        assert!(ctx.game().city_opt(cs.city).is_some());
    }

    #[test]
    fn the_change_menu_starts_on_the_first_item() {
        // choose_production sets no menu row (ds:0x2f9a): Enter at once takes the first item,
        // whatever the city builds
        let Some((mut ctx, mut cs)) = screen() else { return };
        let list = ctx.game().available_production(cs.city);
        ctx.game_mut().city_mut(cs.city).production = *list.last().unwrap();
        let Transition::Push(mut menu) = cs.handle_event(&key(Key::Char('c')), &mut ctx) else { panic!("no menu") };
        menu.on_enter(&mut ctx);
        let Transition::Pop(r) = menu.handle(&key(Key::Enter), &mut ctx) else { panic!("Enter does not choose") };
        cs.resume(r, &mut ctx);
        assert_eq!(ctx.game().city(cs.city).production, list[0]);
    }

    #[test]
    fn s_selects_an_improvement_and_other_keys_leave() {
        let Some((mut ctx, mut cs)) = screen() else { return };
        assert!(matches!(cs.handle_event(&key(Key::Char('z')), &mut ctx), Transition::Pop(_)));
        assert!(matches!(cs.handle_event(&key(Key::Unknown), &mut ctx), Transition::None));
        // the capital's Palace and a Barracks: Down moves to the second, Enter asks
        ctx.game_mut().city_mut(cs.city).add_improvement(improvement_ids::BARRACKS);
        let rows = cs.sell_rows(&ctx);
        assert_eq!(rows.len(), 2);
        assert!(matches!(cs.handle_event(&key(Key::Char('s')), &mut ctx), Transition::None));
        assert_eq!(cs.sell_cursor, Some(0));
        cs.handle_event(&key(Key::Down), &mut ctx);
        cs.handle_event(&key(Key::Down), &mut ctx);
        assert_eq!(cs.sell_cursor, Some(1));
        assert!(matches!(cs.handle_event(&key(Key::Enter), &mut ctx), Transition::Push(_)));
        assert!(matches!(cs.pending, Pending::Sell(id) if id == rows[1].1));
        // once the city has sold this turn 's' leaves the screen
        cs.pending = Pending::None;
        let owner = ctx.game().city(cs.city).owner;
        ctx.game_mut().player_mut(owner).sold_cities.push(cs.city);
        assert!(matches!(cs.handle_event(&key(Key::Char('s')), &mut ctx), Transition::Pop(_)));
    }
}
