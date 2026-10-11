//! The main map screen: viewport, menu bar, sidebar, unit orders and end-of-turn processing.

use crate::game::turn::{TurnRunner, TurnStep};
use crate::game::{tile_flags, Decision, Dir8, Map, MoveOutcome, UnitId, UnitOrder};
use crate::gfx::text::{draw_text, draw_text_centered};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::{Event, Key, MouseButton};
use crate::rules::{self, MAP_H, MAP_W, SLOT_COLOURS};
use crate::ui::context::UiContext;
use crate::ui::dialogs;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::theme;
use crate::ui::tilerender::{self, draw_tile, TILE};
use crate::ui::widgets::{MenuDialog, MessageBox, YesNo};

pub const VIEW_X: i32 = 80;
pub const VIEW_Y: i32 = 8;
pub const VIEW_W: i32 = 15;
pub const VIEW_H: i32 = 12;
pub const MINIMAP: Rect = Rect::new(0, 9, 80, 48);
pub const MENU_H: i32 = 8;

const MENU_LABELS: [(&str, i32); 5] = [("GAME", 8), ("ORDERS", 64), ("ADVISORS", 128), ("WORLD", 192), ("CIVILOPEDIA", 240)];
/// Dropdown rectangles (x, y, w, h) from the original.
const MENU_RECTS: [Rect; 5] = [Rect::new(17, 9, 91, 79), Rect::new(80, 9, 114, 95), Rect::new(113, 9, 151, 55), Rect::new(145, 9, 160, 55), Rect::new(183, 9, 128, 55)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Normal,
    /// Choosing a goto destination for the active unit.
    Goto { cursor: (i32, i32) },
    /// Waiting for the player to confirm end of turn / nothing to do.
    EndOfTurn,
    /// Processing the end of the turn (AI moves etc.).
    Processing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    None,
    Decision,
    Event,
    /// Player answered a menu-bar menu: which bar menu.
    BarMenu(usize),
    EndTurnConfirm,
    CityPick(i32, i32),
    DisbandConfirm(UnitId),
    FoundCityConfirm(UnitId),
    QuitConfirm,
    RevolutionConfirm,
    Error,
    /// A note in front of the founding animation of the first city.
    CityFounded(crate::game::CityId),
}

pub struct MapView {
    pub vx: i32,
    pub vy: i32,
    mode: Mode,
    pending: Pending,
    runner: Option<TurnRunner>,
    current_decision: Option<Decision>,
    /// Frames to pause before continuing processing (enemy move animation).
    pause: u32,
    blink: bool,
    last_centered: Option<UnitId>,
    bar_menu_open: Option<usize>,
    pub status_line: String,
    /// Show the FIRSTMOVE hint when the view first appears.
    pub first_turn_hint: bool,
    auto_end: u32,
    /// Events taken from the game but not yet shown (one dialog at a time).
    queued: std::collections::VecDeque<crate::game::GameEvent>,
    /// Active unit and tile last checked for Settlers advice.
    hint_checked: Option<(UnitId, (i32, i32))>,
}

impl Default for MapView {
    fn default() -> Self {
        Self::new()
    }
}

impl MapView {
    pub fn new() -> MapView {
        MapView {
            vx: 0,
            vy: 0,
            mode: Mode::Normal,
            pending: Pending::None,
            runner: None,
            current_decision: None,
            pause: 0,
            blink: true,
            last_centered: None,
            bar_menu_open: None,
            status_line: String::new(),
            first_turn_hint: false,
            auto_end: 0,
            queued: Default::default(),
            hint_checked: None,
        }
    }

    pub fn center_on(&mut self, x: i32, y: i32) {
        self.vx = (x - 6).clamp(0, MAP_W as i32 - VIEW_W);
        self.vy = (y - 5).clamp(0, MAP_H as i32 - VIEW_H);
    }

    fn ensure_visible(&mut self, x: i32, y: i32) {
        if x < self.vx + 1 || x >= self.vx + VIEW_W - 1 || y < self.vy + 1 || y >= self.vy + VIEW_H - 1 {
            self.center_on(x, y);
        }
    }

    fn tile_at_pixel(&self, px: i32, py: i32) -> Option<(i32, i32)> {
        if px < VIEW_X || py < VIEW_Y || px >= VIEW_X + VIEW_W * TILE || py >= VIEW_Y + VIEW_H * TILE {
            return None;
        }
        let x = self.vx + (px - VIEW_X) / TILE;
        let y = self.vy + (py - VIEW_Y) / TILE;
        if Map::in_bounds(x, y) {
            Some((x, y))
        } else {
            None
        }
    }

    fn activate_next(&mut self, ctx: &mut UiContext) {
        let g = ctx.game_mut();
        g.active_unit = g.next_active_unit();
        if let Some(id) = g.active_unit {
            let (x, y) = g.unit(id).pos();
            if self.last_centered != Some(id) {
                self.ensure_visible(x, y);
                self.last_centered = Some(id);
            }
            self.mode = Mode::Normal;
        } else {
            self.mode = Mode::EndOfTurn;
        }
    }

    fn start_end_turn(&mut self, ctx: &mut UiContext) {
        let g = ctx.game_mut();
        g.active_unit = None;
        self.runner = Some(g.end_turn());
        self.mode = Mode::Processing;
        self.pause = 0;
    }

    /// Step the turn runner; returns a transition when a dialog must be shown.
    fn process(&mut self, ctx: &mut UiContext) -> Transition {
        if self.pause > 0 {
            self.pause -= 1;
            return Transition::None;
        }
        let Some(mut runner) = self.runner.take() else { return Transition::None };
        let mut result = Transition::None;
        for _ in 0..200 {
            let step = {
                let g = ctx.game.as_mut().expect("game");
                runner.step(g, &mut ctx.ai)
            };
            match step {
                TurnStep::Done => {
                    self.runner = None;
                    self.after_turn(ctx);
                    return self.continue_gotos(ctx);
                }
                TurnStep::Decision(d) => {
                    self.current_decision = Some(d.clone());
                    self.pending = Pending::Decision;
                    result = Transition::Push(dialogs::screen_for_decision(ctx, &d));
                    break;
                }
                TurnStep::Event(e) => {
                    if let Some(s) = self.event_screen(ctx, &e) {
                        self.pending = Pending::Event;
                        result = Transition::Push(s);
                        break;
                    }
                }
                TurnStep::AiMove { to, visible, .. } => {
                    if visible && ctx.game().options.enemy_moves {
                        self.ensure_visible(to.0, to.1);
                        self.pause = 6;
                        break;
                    }
                }
                TurnStep::Combat { defender_at, visible, .. } => {
                    if visible {
                        self.ensure_visible(defender_at.0, defender_at.1);
                        self.pause = 20;
                        break;
                    }
                }
            }
        }
        self.runner = Some(runner);
        result
    }

    fn after_turn(&mut self, ctx: &mut UiContext) {
        if ctx.game().options.autosave {
            let path = crate::save::slot_path(&ctx.save_dir, 9);
            let _ = crate::save::save_game(&path, ctx.game());
        }
        self.last_centered = None;
        self.activate_next(ctx);
    }

    fn unit_order(&mut self, ctx: &mut UiContext, o: UnitOrder) -> Transition {
        let Some(id) = ctx.game().active_unit else { return Transition::None };
        let r = ctx.game_mut().give_order(id, o);
        match r {
            Ok(()) => {
                self.activate_next(ctx);
                Transition::None
            }
            Err(e) => self.error(ctx, &e),
        }
    }

    fn error(&mut self, ctx: &mut UiContext, e: &crate::game::RuleError) -> Transition {
        match dialogs::error_screen(ctx, e) {
            Some(s) => {
                self.pending = Pending::Error;
                Transition::Push(s)
            }
            None => Transition::None,
        }
    }

    fn try_move(&mut self, ctx: &mut UiContext, d: Dir8) -> Transition {
        let Some(id) = ctx.game().active_unit else { return Transition::None };
        let before = ctx.game().unit(id).pos();
        let r = ctx.game_mut().try_move(id, d);
        match r {
            Ok(outcome) => {
                let g = ctx.game();
                let alive = g.unit_opt(id).is_some();
                if alive {
                    let (x, y) = g.unit(id).pos();
                    if (x, y) != before {
                        self.ensure_visible(x, y);
                    }
                }
                let t = match outcome {
                    MoveOutcome::EnteredForeignCity(c) => self.foreign_city_menu(ctx, id, c),
                    _ => Transition::None,
                };
                // show any events (huts, captures)
                if let Some(s) = self.next_event_screen(ctx) {
                    self.pending = Pending::Event;
                    return Transition::Push(s);
                }
                if matches!(t, Transition::None) {
                    let g = ctx.game();
                    if g.unit_opt(id).map(|u| u.moves_left == 0 || u.order != UnitOrder::None).unwrap_or(true) {
                        self.activate_next(ctx);
                    }
                }
                t
            }
            Err(e) => self.error(ctx, &e),
        }
    }

    fn foreign_city_menu(&mut self, ctx: &mut UiContext, id: UnitId, c: crate::game::CityId) -> Transition {
        let g = ctx.game();
        let u = g.unit(id);
        let name = g.city(c).name.clone();
        if u.kind == rules::unit_ids::CARAVAN {
            self.pending = Pending::None;
            self.status_line = format!("Caravan at {name}");
            Transition::Push(Box::new(MenuDialog::new(vec![format!("Caravan arrives at {name}.")], vec!["Establish trade route".into(), "Help build wonder".into(), "Keep moving".into()])))
        } else {
            Transition::Push(Box::new(MenuDialog::new(vec![format!("Diplomat arrives at {name}.")], vec!["Establish embassy".into(), "Investigate city".into(), "Sabotage production".into(), "Steal technology".into(), "Incite a revolt".into(), "Keep moving".into()])))
        }
    }

    fn open_city(&mut self, _ctx: &mut UiContext, c: crate::game::CityId) -> Transition {
        Transition::Push(crate::ui::cityscreen::city_screen(c))
    }

    fn bar_menu(&mut self, ctx: &mut UiContext, i: usize) -> Transition {
        let g = ctx.game();
        let r = MENU_RECTS[i];
        let mut gaps: Vec<usize> = Vec::new();
        let mut disabled: Vec<bool> = Vec::new();
        let items: Vec<String> = match i {
            0 => {
                gaps.push(6);
                vec!["Tax Rate".into(), "Luxuries Rate".into(), "FindCity".into(), "Options".into(), "Save Game".into(), "REVOLUTION!".into(), "Retire".into(), "QUIT to DOS".into()]
            }
            1 => {
                let v: Vec<String> = vec![
                    "No Orders  space".into(),
                    "Found New City  b".into(),
                    "Build Road  r".into(),
                    "Build Irrigation  i".into(),
                    "Build Mines  m".into(),
                    "Build Fortress  F".into(),
                    "Fortify  f".into(),
                    "Wait  w".into(),
                    "Sentry  s".into(),
                    "GoTo  g".into(),
                    "Pillage  P".into(),
                    "Clean Pollution  p".into(),
                    "Home City  h".into(),
                    "Unload  u".into(),
                    "Disband Unit  D".into(),
                ];
                gaps.push(14);
                let (settlers, military, ship) = match g.active_unit.and_then(|a| g.unit_opt(a)) {
                    Some(u) => (u.kind == rules::unit_ids::SETTLERS, u.info().att > 0, u.is_sea()),
                    None => (false, false, false),
                };
                let has_unit = g.active_unit.is_some();
                let knows_construction = g.human().knows(rules::advance_ids::CONSTRUCTION);
                disabled = vec![
                    !has_unit,
                    !settlers,
                    !settlers,
                    !settlers,
                    !settlers,
                    !settlers || !knows_construction,
                    !has_unit || settlers,
                    !has_unit,
                    !has_unit,
                    !has_unit,
                    !military,
                    !settlers,
                    !has_unit,
                    !ship,
                    !has_unit,
                ];
                v
            }
            2 => vec!["City Status      F1".into(), "Military Advisor F2".into(), "Intelligence     F3".into(), "Attitude Advisor F4".into(), "Trade Advisor    F5".into(), "Science Advisor  F6".into()],
            3 => vec!["Wonders of World F7".into(), "Top 5 Cities     F8".into(), "Civ. Score       F9".into(), "World Map       F10".into(), "Demographics    F11".into(), "SpaceShips      F12".into()],
            _ => vec!["Civilization Advances".into(), "City Improvements".into(), "Military Units".into(), "Terrain Types".into(), "Miscellaneous".into()],
        };
        self.pending = Pending::BarMenu(i);
        self.bar_menu_open = Some(i);
        let mut m = MenuDialog::new(vec![], items).with_pos(r.x, r.y).with_size(r.w, r.h).with_gaps(gaps);
        if !disabled.is_empty() {
            m = m.with_disabled(disabled);
        }
        Transition::Push(Box::new(m))
    }

    fn bar_menu_choice(&mut self, ctx: &mut UiContext, menu: usize, choice: usize) -> Transition {
        match (menu, choice) {
            (0, 0) => Transition::Push(crate::ui::reports::rate_dialog(ctx, false)),
            (0, 1) => Transition::Push(crate::ui::reports::rate_dialog(ctx, true)),
            (0, 2) => Transition::Push(crate::ui::reports::find_city_dialog(ctx)),
            (0, 3) => Transition::Push(crate::ui::reports::options_dialog(ctx)),
            (0, 4) => Transition::Push(crate::ui::reports::save_dialog(ctx)),
            (0, 5) => {
                self.pending = Pending::RevolutionConfirm;
                Transition::Push(Box::new(YesNo::new(vec!["Are you sure you want a REVOLUTION?".into()], "No thanks.", "Yes, we need a new government.").with_pos(80, 81)))
            }
            (0, 6) => {
                self.pending = Pending::QuitConfirm;
                Transition::Push(Box::new(YesNo::new(vec!["Are you sure you".into(), "want to Quit?".into()], "Keep Playing", "Yes, Quit").with_pos(101, 81)))
            }
            (0, 7) => {
                self.pending = Pending::QuitConfirm;
                Transition::Push(Box::new(YesNo::new(vec!["Are you sure you".into(), "want to Quit?".into()], "Keep Playing", "Yes, Quit").with_pos(101, 81)))
            }
            (1, c) => {
                let key = match c {
                    0 => Key::Space,
                    1 => Key::Char('b'),
                    2 => Key::Char('r'),
                    3 => Key::Char('i'),
                    4 => Key::Char('m'),
                    5 => Key::Char('f'),
                    6 => Key::Char('f'),
                    7 => Key::Char('w'),
                    8 => Key::Char('s'),
                    9 => Key::Char('g'),
                    10 => Key::Char('p'),
                    11 => Key::Char('p'),
                    12 => Key::Char('h'),
                    13 => Key::Char('u'),
                    14 => Key::Char('d'),
                    _ => Key::Unknown,
                };
                let shift = matches!(c, 5 | 10 | 14);
                self.key_order(ctx, key, crate::platform::Mods { shift, ctrl: false, alt: false })
            }
            (2, c) => Transition::Push(crate::ui::reports::advisor(ctx, c as u8 + 1)),
            (3, c) => Transition::Push(crate::ui::reports::advisor(ctx, c as u8 + 7)),
            (4, c) => Transition::Push(crate::ui::civilopedia::category(ctx, c)),
            _ => Transition::None,
        }
    }

    fn key_order(&mut self, ctx: &mut UiContext, key: Key, mods: crate::platform::Mods) -> Transition {
        let active = ctx.game().active_unit;
        match key {
            Key::Space => {
                if let Some(id) = active {
                    ctx.game_mut().skip_unit(id);
                    self.activate_next(ctx);
                }
                Transition::None
            }
            Key::Char('w') | Key::Tab => {
                if let Some(id) = active {
                    ctx.game_mut().wait_unit(id);
                    let g = ctx.game_mut();
                    if g.active_unit == Some(id) || g.active_unit.is_none() {
                        g.active_unit = Some(id);
                    }
                    if let Some(a) = ctx.game().active_unit {
                        let (x, y) = ctx.game().unit(a).pos();
                        self.ensure_visible(x, y);
                    }
                }
                Transition::None
            }
            Key::Char('f') => {
                if mods.shift {
                    self.unit_order(ctx, UnitOrder::Fortress)
                } else {
                    self.unit_order(ctx, UnitOrder::Fortify)
                }
            }
            Key::Char('s') => self.unit_order(ctx, UnitOrder::Sentry),
            Key::Char('r') => self.unit_order(ctx, UnitOrder::Road),
            Key::Char('i') => self.unit_order(ctx, UnitOrder::Irrigate),
            Key::Char('m') => self.unit_order(ctx, UnitOrder::Mine),
            Key::Char('p') => {
                if mods.shift {
                    self.unit_order(ctx, UnitOrder::Pillage)
                } else {
                    self.unit_order(ctx, UnitOrder::CleanPollution)
                }
            }
            Key::Char('b') => {
                let Some(id) = active else { return Transition::None };
                let g = ctx.game();
                let u = g.unit(id);
                if u.kind != rules::unit_ids::SETTLERS {
                    return self.error(ctx, &crate::game::RuleError::Settlers);
                }
                if let Some(c) = g.city_at(u.x, u.y) {
                    let name = g.city(c).name.clone();
                    self.pending = Pending::FoundCityConfirm(id);
                    return Transition::Push(Box::new(YesNo::new(vec![format!("Add settlers to {name}?")], "Yes", "No")));
                }
                // propose a name
                let mut g2 = g.clone();
                let name = g2.next_city_name(u.owner);
                self.pending = Pending::FoundCityConfirm(id);
                Transition::Push(Box::new(crate::ui::widgets::TextInput::new(vec!["City Name...".into()], &name, 12).with_pos(80, 100).with_style(crate::ui::widgets::PanelStyle::Cyan)))
            }
            Key::Char('h') => {
                let Some(id) = active else { return Transition::None };
                let r = ctx.game_mut().set_home(id);
                match r {
                    Ok(()) => {
                        self.activate_next(ctx);
                        Transition::None
                    }
                    Err(e) => self.error(ctx, &e),
                }
            }
            Key::Char('u') => {
                if let Some(id) = active {
                    ctx.game_mut().unload(id);
                    self.activate_next(ctx);
                }
                Transition::None
            }
            Key::Char('d') if mods.shift => {
                let Some(id) = active else { return Transition::None };
                let name = ctx.game().unit(id).info().name;
                self.pending = Pending::DisbandConfirm(id);
                Transition::Push(Box::new(YesNo::new(vec![format!("Disband {name}?")], "No", "Yes")))
            }
            Key::Char('g') => {
                if let Some(id) = active {
                    let (x, y) = ctx.game().unit(id).pos();
                    self.mode = Mode::Goto { cursor: (x, y) };
                }
                Transition::None
            }
            Key::Char('c') => {
                if let Some(id) = active {
                    let (x, y) = ctx.game().unit(id).pos();
                    self.center_on(x, y);
                }
                Transition::None
            }
            _ => Transition::None,
        }
    }

    fn handle_goto_key(&mut self, ctx: &mut UiContext, key: Key) -> Transition {
        let Mode::Goto { cursor } = self.mode else { return Transition::None };
        let d = match key {
            Key::Up => Some(Dir8::N),
            Key::Down => Some(Dir8::S),
            Key::Left => Some(Dir8::W),
            Key::Right => Some(Dir8::E),
            Key::Home => Some(Dir8::NW),
            Key::PageUp => Some(Dir8::NE),
            Key::End => Some(Dir8::SW),
            Key::PageDown => Some(Dir8::SE),
            Key::Kp(n) => Dir8::from_keypad(n),
            _ => None,
        };
        if let Some(d) = d {
            let (dx, dy) = d.delta();
            let nx = (cursor.0 + dx).clamp(0, MAP_W as i32 - 1);
            let ny = (cursor.1 + dy).clamp(0, MAP_H as i32 - 1);
            self.mode = Mode::Goto { cursor: (nx, ny) };
            self.ensure_visible(nx, ny);
            return Transition::None;
        }
        match key {
            Key::Enter | Key::Char('g') => self.confirm_goto(ctx, cursor),
            Key::Esc => {
                self.mode = Mode::Normal;
                Transition::None
            }
            _ => Transition::None,
        }
    }

    fn confirm_goto(&mut self, ctx: &mut UiContext, dest: (i32, i32)) -> Transition {
        self.mode = Mode::Normal;
        let Some(id) = ctx.game().active_unit else { return Transition::None };
        let g = ctx.game_mut();
        if g.unit(id).pos() == dest {
            g.unit_mut(id).goto = None;
            g.unit_mut(id).order = UnitOrder::None;
            return Transition::None;
        }
        g.set_goto(id, dest.0, dest.1);
        self.run_goto(ctx, id)
    }

    /// Move a unit along its goto path as far as its moves allow.
    fn run_goto(&mut self, ctx: &mut UiContext, id: UnitId) -> Transition {
        for _ in 0..40 {
            let g = ctx.game();
            let Some(u) = g.unit_opt(id) else { break };
            if u.order != UnitOrder::Goto || u.moves_left == 0 {
                break;
            }
            let Some(dir) = g.next_goto_step(id) else {
                let g = ctx.game_mut();
                g.unit_mut(id).order = UnitOrder::None;
                g.unit_mut(id).goto = None;
                break;
            };
            let dest = u.goto;
            let r = ctx.game_mut().try_move(id, dir);
            match r {
                Ok(MoveOutcome::Moved) | Ok(MoveOutcome::Boarded(_)) => {
                    let g = ctx.game_mut();
                    if let Some(u) = g.unit_opt(id) {
                        if Some(u.pos()) == dest {
                            g.unit_mut(id).order = UnitOrder::None;
                            g.unit_mut(id).goto = None;
                        }
                    }
                }
                Ok(MoveOutcome::EnteredHut) => {}
                Ok(_) | Err(_) => {
                    let g = ctx.game_mut();
                    if let Some(u) = g.unit_opt(id) {
                        let _ = u;
                        g.unit_mut(id).order = UnitOrder::None;
                        g.unit_mut(id).goto = None;
                    }
                    break;
                }
            }
        }
        if let Some(s) = self.next_event_screen(ctx) {
            self.pending = Pending::Event;
            return Transition::Push(s);
        }
        self.activate_next(ctx);
        Transition::None
    }

    /// Continue goto orders of all units at the start of the human turn.
    fn continue_gotos(&mut self, ctx: &mut UiContext) -> Transition {
        let ids: Vec<UnitId> = ctx.game().all_units().filter(|u| u.owner == ctx.game().human_slot && u.order == UnitOrder::Goto && u.moves_left > 0).map(|u| u.id).collect();
        for id in ids {
            let t = self.run_goto(ctx, id);
            if !matches!(t, Transition::None) {
                return t;
            }
        }
        Transition::None
    }

    // ---- drawing ------------------------------------------------------------------------

    fn draw_menubar(&self, fb: &mut Surface, ctx: &UiContext) {
        fb.fill_rect(Rect::new(0, 0, 320, MENU_H), theme::BLACK);
        for (i, (label, x)) in MENU_LABELS.iter().enumerate() {
            let open = self.bar_menu_open == Some(i);
            let first = TextStyle::new(0, if open { theme::CYAN } else { theme::WHITE });
            let rest = TextStyle::new(0, if open { theme::CYAN } else { theme::GREY });
            let end = draw_text(fb, &ctx.fonts, &first, *x, 0, &label[..1]);
            draw_text(fb, &ctx.fonts, &rest, end, 0, &label[1..]);
        }
    }

    fn draw_viewport(&self, fb: &mut Surface, ctx: &UiContext) {
        let g = ctx.game();
        let me = g.human_slot;
        let active = g.active_unit;
        fb.set_clip(Some(Rect::new(VIEW_X, VIEW_Y, VIEW_W * TILE, VIEW_H * TILE)));
        for ty in 0..VIEW_H {
            for tx in 0..VIEW_W {
                let (x, y) = (self.vx + tx, self.vy + ty);
                let (px, py) = (VIEW_X + tx * TILE, VIEW_Y + ty * TILE);
                // the active unit blinks: hide it on the off phase
                let hide = if !self.blink { active.filter(|a| g.unit_opt(*a).map(|u| u.pos() == (x, y)).unwrap_or(false)) } else { None };
                // draw the active unit on top of its stack
                draw_tile(fb, ctx, g, me, x, y, px, py, hide);
                if self.blink {
                    if let Some(a) = active {
                        if let Some(u) = g.unit_opt(a) {
                            if u.pos() == (x, y) && g.explored(me, x, y) {
                                let stacked = g.units_at(x, y).len() > 1;
                                tilerender::draw_unit(fb, ctx, g, a, px, py, stacked);
                            }
                        }
                    }
                }
            }
        }
        // city names
        for ty in 0..VIEW_H {
            for tx in 0..VIEW_W {
                let (x, y) = (self.vx + tx, self.vy + ty);
                if !g.explored(me, x, y) {
                    continue;
                }
                if let Some(c) = g.city_at(x, y) {
                    let city = g.city(c);
                    let (px, py) = (VIEW_X + tx * TILE, VIEW_Y + ty * TILE);
                    let st = TextStyle::shadowed(0, theme::CYAN, theme::BLACK);
                    draw_text_centered(fb, &ctx.fonts, &st, px + 8, py + 16, &city.name);
                }
            }
        }
        if let Mode::Goto { cursor } = self.mode {
            let (px, py) = (VIEW_X + (cursor.0 - self.vx) * TILE, VIEW_Y + (cursor.1 - self.vy) * TILE);
            tilerender::blit_sprite(fb, &ctx.sp.surf, tilerender::cell(tilerender::sp::CURSOR_GO.0, tilerender::sp::CURSOR_GO.1), px, py);
        }
        fb.set_clip(None);
    }

    fn draw_minimap(&self, fb: &mut Surface, ctx: &UiContext) {
        let g = ctx.game();
        let me = g.human_slot;
        fb.fill_rect(MINIMAP, theme::BLACK);
        for y in 0..MAP_H as i32 {
            for x in 0..MAP_W as i32 {
                if !g.explored(me, x, y) {
                    continue;
                }
                let t = g.map.tile(x, y);
                let c = match t.terrain {
                    rules::Terrain::Ocean => 1,
                    rules::Terrain::River => 9,
                    rules::Terrain::Arctic | rules::Terrain::Tundra => 7,
                    rules::Terrain::Mountains | rules::Terrain::Hills => 6,
                    rules::Terrain::Desert | rules::Terrain::Plains => 14,
                    rules::Terrain::Forest | rules::Terrain::Jungle | rules::Terrain::Swamp => 2,
                    _ => 10,
                };
                fb.put(MINIMAP.x + x, MINIMAP.y + y, c);
            }
        }
        for c in g.all_cities() {
            if g.explored(me, c.x, c.y) {
                fb.put(MINIMAP.x + c.x, MINIMAP.y + c.y, SLOT_COLOURS[(c.owner & 7) as usize].light);
            }
        }
        fb.rect_outline(Rect::new(MINIMAP.x + self.vx, MINIMAP.y + self.vy, VIEW_W, VIEW_H), theme::WHITE);
    }

    fn draw_sidebar(&self, fb: &mut Surface, ctx: &UiContext) {
        let g = ctx.game();
        let p = g.human();
        let sp = &ctx.sp.surf;
        // status panel (checker pattern) and unit panel (stone pattern)
        let status = Rect::new(0, 59, 80, 37);
        let unit_panel = Rect::new(0, 98, 80, 102);
        fb.fill_pattern(status, sp, tilerender::cell(tilerender::sp::PATTERN_STONE.0, tilerender::sp::PATTERN_STONE.1));
        fb.fill_pattern(unit_panel, sp, tilerender::cell(tilerender::sp::PATTERN_STONE.0, tilerender::sp::PATTERN_STONE.1));
        // frame lines: dark top (y=8) and right column, light left column and bottom
        fb.hline(0, 79, 8, theme::DARK_GREY);
        fb.vline(79, 8, 199, theme::DARK_GREY);
        fb.vline(0, 9, 199, theme::WHITE);
        fb.hline(0, 78, 199, theme::WHITE);
        fb.hline(0, 78, 57, theme::WHITE);
        fb.hline(0, 79, 58, theme::DARK_GREY);
        fb.hline(0, 78, 96, theme::WHITE);
        fb.hline(0, 79, 97, theme::DARK_GREY);
        // civilisation progress bar
        let strip = Rect::new(3, 60, 74, 11);
        fb.fill_rect(strip, theme::CYAN);
        fb.hline(3, 76, 71, theme::GREEN);
        let st = TextStyle::new(0, theme::TEXT);
        let pop = g.total_population(g.human_slot);
        if pop > 0 {
            let end = draw_text(fb, &ctx.fonts, &st, 2, 73, &format_pop(pop));
            draw_people_glyph(fb, end + 1, 73);
        }
        let end = draw_text(fb, &ctx.fonts, &st, 2, 81, &g.year_text());
        draw_jewel_glyph(fb, end + 4, 82);
        let sci = p.sci() / 10;
        let end = draw_text(fb, &ctx.fonts, &st, 2, 89, &format!("{}", p.gold));
        draw_coin_glyph(fb, end + 1, 90);
        draw_text(fb, &ctx.fonts, &st, 27, 89, &format!("{}.{}.{}", p.tax / 10, p.lux / 10, sci));
        let mut y = 99;
        match self.mode {
            Mode::EndOfTurn => {
                if !g.options.end_turn_confirm {
                    draw_text(fb, &ctx.fonts, &st, 4, 124, "End of Turn");
                }
                draw_text(fb, &ctx.fonts, &st, 4, 136, "Press Enter");
                draw_text(fb, &ctx.fonts, &st, 4, 144, "to continue");
            }
            Mode::Processing => {
                draw_text(fb, &ctx.fonts, &st, 4, 124, "Please wait...");
            }
            _ => {
                if let Some(id) = g.active_unit {
                    if let Some(u) = g.unit_opt(id) {
                        draw_text(fb, &ctx.fonts, &st, 4, y, p.adjective());
                        y += 8;
                        let vet = if u.veteran { " (V)" } else { "" };
                        draw_text(fb, &ctx.fonts, &st, 4, y, &format!("{}{}", u.info().name, vet));
                        y += 8;
                        let m = u.moves_left;
                        let moves = if m % 3 == 0 { format!("{}", m / 3) } else { format!("{}/3", m) };
                        draw_text(fb, &ctx.fonts, &st, 4, y, &format!("Moves: {moves}"));
                        y += 8;
                        let home = u.home.and_then(|h| g.city_opt(h)).map(|c| c.name.clone()).unwrap_or_else(|| "NONE".into());
                        draw_text(fb, &ctx.fonts, &TextStyle::new(if u.home.is_none() { 6 } else { 0 }, theme::TEXT), 4, y, &home);
                        y += 8;
                        draw_text(fb, &ctx.fonts, &st, 4, y, &format!("({})", g.terrain_name(u.x, u.y)));
                        y += 8;
                        let t = g.map.tile(u.x, u.y);
                        for (flag, label) in [
                            (tile_flags::ROAD, "Road"),
                            (tile_flags::RAILROAD, "Railroad"),
                            (tile_flags::IRRIGATION, "Irrigation"),
                            (tile_flags::MINE, "Mine"),
                            (tile_flags::FORTRESS, "Fortress"),
                            (tile_flags::POLLUTION, "Pollution"),
                        ] {
                            if t.has(flag) {
                                draw_text(fb, &ctx.fonts, &st, 4, y, &format!("({label})"));
                                y += 8;
                            }
                        }
                        // other units in the stack
                        let others: Vec<UnitId> = g.units_at(u.x, u.y).into_iter().filter(|o| *o != id).collect();
                        let mut x = 8;
                        y = y.max(140);
                        for o in others.iter().take(4) {
                            tilerender::draw_unit(fb, ctx, g, *o, x, y, false);
                            x += 18;
                        }
                    }
                }
            }
        }
        if !self.status_line.is_empty() {
            draw_text(fb, &ctx.fonts, &TextStyle::new(1, theme::TEXT), 3, 190, &self.status_line);
        }
    }
}

/// Small coin glyph after the treasury figure.
fn draw_coin_glyph(fb: &mut Surface, x: i32, y: i32) {
    for (dx, dy) in [(1, 0), (2, 0), (0, 1), (3, 1), (0, 2), (3, 2), (1, 3), (2, 3)] {
        fb.put(x + dx, y + dy, theme::BLACK);
    }
    fb.put(x + 1, y + 1, theme::BLACK);
}

/// Small jewel glyph after the year.
fn draw_jewel_glyph(fb: &mut Surface, x: i32, y: i32) {
    for dx in 0..5 {
        fb.put(x + dx, y, theme::CYAN);
    }
    for dx in 1..4 {
        fb.put(x + dx, y + 1, theme::CYAN);
    }
    fb.put(x + 2, y + 2, theme::CYAN);
    fb.put(x + 2, y + 3, theme::BLACK);
}

/// Small person glyph after the population.
fn draw_people_glyph(fb: &mut Surface, x: i32, y: i32) {
    fb.put(x + 2, y, theme::BLACK);
    for dx in 1..4 {
        fb.put(x + dx, y + 1, theme::BLACK);
    }
    for dx in 0..5 {
        fb.put(x + dx, y + 2, theme::BLACK);
    }
    fb.put(x + 1, y + 3, theme::BLACK);
    fb.put(x + 3, y + 3, theme::BLACK);
    fb.put(x + 1, y + 4, theme::BLACK);
    fb.put(x + 3, y + 4, theme::BLACK);
}

pub fn format_pop(p: i64) -> String {
    let s = p.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

impl Screen for MapView {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        let g = ctx.game_mut();
        if g.active_unit.is_none() {
            g.active_unit = g.next_active_unit();
        }
        if let Some(id) = g.active_unit {
            let (x, y) = g.unit(id).pos();
            self.center_on(x, y);
        } else if let Some(c) = g.cities_of(g.human_slot).first().copied() {
            let (x, y) = g.city(c).pos();
            self.center_on(x, y);
        }
        if ctx.game().active_unit.is_none() {
            self.mode = Mode::EndOfTurn;
        }
    }

    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        self.blink = (ctx.frame / 16) % 2 == 0;
        if let Some((x, y)) = ctx.center_request.take() {
            self.center_on(x, y);
        }
        if self.first_turn_hint {
            self.first_turn_hint = false;
            // the start tile gets no BUILDCITY note on top of this one
            self.hint_checked = ctx.game().active_unit.map(|id| (id, ctx.game().unit(id).pos()));
            if let Some(s) = ctx.game_mut().hint("FIRSTMOVE").and_then(|e| dialogs::screen_for_event(ctx, &e)) {
                self.pending = Pending::Event;
                return Transition::Push(s);
            }
        }
        if let Some(s) = self.settler_advice(ctx) {
            self.pending = Pending::Event;
            return Transition::Push(s);
        }
        if self.mode == Mode::Processing {
            return self.process(ctx);
        }
        if ctx.quit_requested {
            ctx.quit_requested = false;
            self.pending = Pending::QuitConfirm;
            return Transition::Push(Box::new(YesNo::new(vec!["Are you sure you".into(), "want to Quit?".into()], "Keep Playing", "Yes, Quit").with_pos(101, 81)));
        }
        // automatic end of turn when nothing is left to do and the option is off
        if self.mode == Mode::EndOfTurn && !ctx.game().options.end_turn_confirm {
            self.auto_end += 1;
            if self.auto_end > 45 {
                self.auto_end = 0;
                self.start_end_turn(ctx);
            }
        } else {
            self.auto_end = 0;
        }
        Transition::None
    }

    fn resume(&mut self, result: DialogResult, ctx: &mut UiContext) -> Transition {
        let pending = std::mem::replace(&mut self.pending, Pending::None);
        self.bar_menu_open = None;
        match pending {
            Pending::Decision => {
                if let (Some(d), Some(runner)) = (self.current_decision.take(), self.runner.as_mut()) {
                    let reply = dialogs::reply_for(&d, &result);
                    runner.resolve(ctx.game.as_mut().expect("game"), reply);
                    // decisions that open the city screen
                    if let Decision::ProductionChoice { city, .. } = d {
                        if ctx.game().city_opt(city).is_some() {
                            self.pending = Pending::Event;
                            return self.open_city(ctx, city);
                        }
                    }
                }
                Transition::None
            }
            Pending::Event | Pending::Error => {
                // events produced while the dialog was up (audiences, unit actions)
                if let Some(s) = self.next_event_screen(ctx) {
                    self.pending = Pending::Event;
                    return Transition::Push(s);
                }
                if self.mode == Mode::Processing {
                    Transition::None
                } else {
                    let t = self.continue_gotos(ctx);
                    if matches!(t, Transition::None) && ctx.game().active_unit.is_none() {
                        self.activate_next(ctx);
                    }
                    t
                }
            }
            Pending::BarMenu(i) => match result {
                DialogResult::Choice(c) => self.bar_menu_choice(ctx, i, c),
                _ => Transition::None,
            },
            Pending::EndTurnConfirm => {
                if result == DialogResult::Yes {
                    self.start_end_turn(ctx);
                }
                Transition::None
            }
            Pending::CityPick(x, y) => {
                if let DialogResult::Choice(i) = result {
                    let units = ctx.game().units_at(x, y);
                    if let Some(id) = units.get(i) {
                        let g = ctx.game_mut();
                        let u = g.unit_mut(*id);
                        u.order = UnitOrder::None;
                        u.goto = None;
                        g.active_unit = Some(*id);
                        self.mode = Mode::Normal;
                    }
                }
                Transition::None
            }
            Pending::DisbandConfirm(id) => {
                if result == DialogResult::No || result == DialogResult::Choice(1) {
                    // "Yes" is the second item
                    ctx.game_mut().disband(id);
                    self.activate_next(ctx);
                }
                Transition::None
            }
            Pending::FoundCityConfirm(id) => {
                match result {
                    DialogResult::Text(name) => {
                        let r = ctx.game_mut().found_city(id);
                        match r {
                            Ok(c) => {
                                if !name.trim().is_empty() {
                                    ctx.game_mut().city_mut(c).name = name.trim().to_string();
                                }
                                // FIRSTPRODUCT comes before the founding animation
                                let note = ctx.game_mut().take_events().into_iter().find_map(|e| match e {
                                    crate::game::GameEvent::Hint(_) => dialogs::screen_for_event(ctx, &e),
                                    _ => None,
                                });
                                self.activate_next(ctx);
                                self.hint_checked = ctx.game().active_unit.map(|id| (id, ctx.game().unit(id).pos()));
                                if let Some(n) = note {
                                    self.pending = Pending::CityFounded(c);
                                    return Transition::Push(n);
                                }
                                self.pending = Pending::Event;
                                return Transition::Push(crate::ui::cityfounded::city_founded(c));
                            }
                            Err(e) => return self.error(ctx, &e),
                        }
                    }
                    DialogResult::Yes => {
                        let r = ctx.game_mut().found_city(id);
                        if let Err(e) = r {
                            return self.error(ctx, &e);
                        }
                        self.activate_next(ctx);
                        if let Some(s) = self.next_event_screen(ctx) {
                            self.pending = Pending::Event;
                            return Transition::Push(s);
                        }
                    }
                    _ => {}
                }
                Transition::None
            }
            Pending::RevolutionConfirm => {
                if result == DialogResult::No {
                    let slot = ctx.game().human_slot;
                    ctx.game_mut().revolution(slot);
                    if let Some(s) = self.next_event_screen(ctx) {
                        self.pending = Pending::Event;
                        return Transition::Push(s);
                    }
                }
                Transition::None
            }
            Pending::CityFounded(c) => {
                self.pending = Pending::Event;
                Transition::Push(crate::ui::cityfounded::city_founded(c))
            }
            Pending::QuitConfirm => {
                if result == DialogResult::No {
                    // second option = yes (see YesNo ordering above: ("No","Yes"))
                    return Transition::Quit;
                }
                Transition::None
            }
            Pending::None => {
                // generic menus (caravan/diplomat) handled here
                if let DialogResult::Choice(i) = result {
                    if let Some(id) = ctx.game().active_unit {
                        return self.special_unit_choice(ctx, id, i);
                    }
                }
                Transition::None
            }
        }
    }

    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if self.mode == Mode::Processing {
            return Transition::None;
        }
        match ev {
            Event::Key { key, mods, down: true, .. } => {
                if let Mode::Goto { .. } = self.mode {
                    return self.handle_goto_key(ctx, *key);
                }
                // movement
                let dir = match key {
                    Key::Up => Some(Dir8::N),
                    Key::Down => Some(Dir8::S),
                    Key::Left => Some(Dir8::W),
                    Key::Right => Some(Dir8::E),
                    Key::Home => Some(Dir8::NW),
                    Key::PageUp => Some(Dir8::NE),
                    Key::End => Some(Dir8::SW),
                    Key::PageDown => Some(Dir8::SE),
                    Key::Kp(n) => Dir8::from_keypad(*n),
                    _ => None,
                };
                if let Some(d) = dir {
                    if ctx.game().active_unit.is_some() {
                        return self.try_move(ctx, d);
                    }
                    // scroll the view when no unit is active
                    let (dx, dy) = d.delta();
                    self.vx = (self.vx + dx).clamp(0, MAP_W as i32 - VIEW_W);
                    self.vy = (self.vy + dy).clamp(0, MAP_H as i32 - VIEW_H);
                    return Transition::None;
                }
                match key {
                    Key::Enter => {
                        if ctx.game().active_unit.is_none() || self.mode == Mode::EndOfTurn {
                            self.start_end_turn(ctx);
                        } else if ctx.game().options.end_turn_confirm {
                            self.pending = Pending::EndTurnConfirm;
                            return Transition::Push(Box::new(YesNo::new(vec!["End turn now?".into()], "Yes", "No")));
                        } else {
                            self.start_end_turn(ctx);
                        }
                        Transition::None
                    }
                    Key::Char('g') if mods.alt || mods.ctrl => self.bar_menu(ctx, 0),
                    Key::Char('o') if mods.alt || mods.ctrl => self.bar_menu(ctx, 1),
                    Key::Char('a') if mods.alt || mods.ctrl => self.bar_menu(ctx, 2),
                    Key::Char('w') if mods.alt || mods.ctrl => self.bar_menu(ctx, 3),
                    Key::Char('c') if mods.alt || mods.ctrl => self.bar_menu(ctx, 4),
                    Key::F(n) => Transition::Push(crate::ui::reports::advisor(ctx, *n)),
                    Key::Char('s') if mods.ctrl => {
                        let path = crate::save::slot_path(&ctx.save_dir, 0);
                        let r = crate::save::save_game(&path, ctx.game());
                        self.pending = Pending::Event;
                        Transition::Push(Box::new(MessageBox::text(&match r {
                            Ok(()) => "Game saved.".to_string(),
                            Err(e) => format!("Save failed: {e}"),
                        })))
                    }
                    Key::Char('q') if mods.ctrl => {
                        self.pending = Pending::QuitConfirm;
                        Transition::Push(Box::new(YesNo::new(vec!["Are you sure you".into(), "want to Quit?".into()], "Keep Playing", "Yes, Quit").with_pos(101, 81)))
                    }
                    Key::Esc => {
                        self.bar_menu(ctx, 0)
                    }
                    k => self.key_order(ctx, *k, *mods),
                }
            }
            Event::MouseButton { button, x, y, down: true } => {
                let (mx, my) = (*x, *y);
                if my < MENU_H {
                    for (i, (label, lx)) in MENU_LABELS.iter().enumerate() {
                        let w = crate::gfx::text::text_width(&ctx.fonts, 0, label);
                        if mx >= lx - 4 && mx < lx + w + 4 {
                            return self.bar_menu(ctx, i);
                        }
                    }
                    return Transition::None;
                }
                if MINIMAP.contains(mx, my) {
                    self.center_on(mx - MINIMAP.x, my - MINIMAP.y);
                    return Transition::None;
                }
                if mx < VIEW_X {
                    // unit status box: cycle units
                    if *button == MouseButton::Left {
                        if ctx.game().active_unit.is_some() {
                            return self.key_order(ctx, Key::Char('w'), crate::platform::Mods::default());
                        } else if self.mode == Mode::EndOfTurn {
                            self.start_end_turn(ctx);
                        }
                    }
                    return Transition::None;
                }
                let Some((tx, ty)) = self.tile_at_pixel(mx, my) else { return Transition::None };
                if let Mode::Goto { .. } = self.mode {
                    if *button == MouseButton::Left {
                        return self.confirm_goto(ctx, (tx, ty));
                    }
                    self.mode = Mode::Normal;
                    return Transition::None;
                }
                let g = ctx.game();
                let me = g.human_slot;
                match button {
                    MouseButton::Right => {
                        if let Some(id) = g.active_unit {
                            let (ux, uy) = g.unit(id).pos();
                            if let Some(d) = Dir8::from_delta(tx - ux, ty - uy) {
                                return self.try_move(ctx, d);
                            }
                            // far away: goto
                            return self.confirm_goto(ctx, (tx, ty));
                        }
                        Transition::None
                    }
                    MouseButton::Left => {
                        if let Some(c) = g.city_at(tx, ty) {
                            if g.city(c).owner == me {
                                self.pending = Pending::Event;
                                return self.open_city(ctx, c);
                            }
                        }
                        let own: Vec<UnitId> = g.units_at(tx, ty).into_iter().filter(|u| g.unit(*u).owner == me).collect();
                        if own.len() == 1 {
                            let g = ctx.game_mut();
                            let u = g.unit_mut(own[0]);
                            u.order = UnitOrder::None;
                            u.goto = None;
                            g.active_unit = Some(own[0]);
                            self.mode = Mode::Normal;
                            return Transition::None;
                        } else if own.len() > 1 {
                            let items: Vec<String> = own.iter().map(|id| {
                                let u = g.unit(*id);
                                format!("{}{}", u.info().name, u.order.letter().map(|l| format!(" ({l})")).unwrap_or_default())
                            }).collect();
                            self.pending = Pending::CityPick(tx, ty);
                            return Transition::Push(Box::new(MenuDialog::new(vec!["Select unit...".into()], items)));
                        }
                        self.center_on(tx, ty);
                        Transition::None
                    }
                    _ => Transition::None,
                }
            }
            _ => Transition::None,
        }
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        fb.fill(theme::BLACK);
        self.draw_viewport(fb, ctx);
        self.draw_minimap(fb, ctx);
        self.draw_sidebar(fb, ctx);
        self.draw_menubar(fb, ctx);
    }

    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.sp.pal
    }

    fn draws_parent(&self) -> bool {
        false
    }
}

impl MapView {
    fn special_unit_choice(&mut self, ctx: &mut UiContext, id: UnitId, choice: usize) -> Transition {
        crate::ui::unit_actions::special_unit_choice(self, ctx, id, choice)
    }

    /// Event dialog with map-view specific preparation (centre on nuclear strikes).
    /// Queue the game's new events and return the dialog for the first one that has one.
    fn next_event_screen(&mut self, ctx: &mut UiContext) -> Option<Box<dyn Screen>> {
        let events = ctx.game_mut().take_events();
        self.queued.extend(events);
        while let Some(e) = self.queued.pop_front() {
            if let Some(s) = self.event_screen(ctx, &e) {
                return Some(s);
            }
        }
        None
    }

    /// MINING / IRRIGATE / ROAD / BUILDCITY note when an idle Settlers reaches a new tile.
    fn settler_advice(&mut self, ctx: &mut UiContext) -> Option<Box<dyn Screen>> {
        if self.mode != Mode::Normal || !matches!(self.pending, Pending::None) || self.runner.is_some() {
            return None;
        }
        let id = ctx.game().active_unit?;
        let at = (id, ctx.game().unit_opt(id)?.pos());
        if self.hint_checked == Some(at) {
            return None;
        }
        self.hint_checked = Some(at);
        // a new game opens with FIRSTMOVE; the start tile gets no BUILDCITY on top of it
        if ctx.game().turn == 0 {
            if let Some(e) = ctx.game_mut().hint("FIRSTMOVE") {
                return dialogs::screen_for_event(ctx, &e);
            }
        }
        let e = ctx.game_mut().settler_hint(id)?;
        dialogs::screen_for_event(ctx, &e)
    }

    fn event_screen(&mut self, ctx: &mut UiContext, e: &crate::game::GameEvent) -> Option<Box<dyn Screen>> {
        if let crate::game::GameEvent::NuclearStrike { at, .. } = e {
            self.vx = (at.0 - VIEW_W / 2).clamp(0, MAP_W as i32 - VIEW_W);
            self.vy = (at.1 - VIEW_H / 2).clamp(0, MAP_H as i32 - VIEW_H);
        }
        dialogs::screen_for_event(ctx, e)
    }

    pub fn set_pending_event(&mut self) {
        self.pending = Pending::Event;
    }

    pub fn finish_unit(&mut self, ctx: &mut UiContext) {
        self.activate_next(ctx);
    }

    pub fn show_error(&mut self, ctx: &mut UiContext, e: &crate::game::RuleError) -> Transition {
        self.error(ctx, e)
    }
}
