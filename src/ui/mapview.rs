//! The main map screen: viewport, menu bar, sidebar, unit orders and end-of-turn processing.

use crate::game::turn::{TurnRunner, TurnStep};
use crate::game::{tile_flags, Decision, Dir8, Map, MoveOutcome, UnitId, UnitOrder};
use crate::gfx::text::{draw_text, draw_text_centered};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::{Event, Key, MouseButton};
use crate::rules::{self, UnitTypeId, MAP_H, MAP_W, SLOT_COLOURS};
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
pub(crate) enum Pending {
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
    /// The "Will you?" menu of a caravan stopped before one of our cities.
    CaravanMenu(UnitId, crate::game::CityId, Dir8),
    /// A Diplomat's offer to bribe a lone foreign unit: (diplomat, unit, direction).
    Bribe(UnitId, UnitId, Dir8),
    /// The menu of a Diplomat at a foreign city.
    Diplomat(UnitId, crate::game::CityId),
    /// "We have signed a peace treaty..." before a Diplomat steals from a treaty partner.
    TreatyBreak(UnitId, crate::game::CityId),
    /// "Dissidents in ... will revolt for $N."
    Incite(UnitId, crate::game::CityId),
    /// "We have signed a peace treaty..." before a unit attacks or enters the land of a treaty
    /// partner: (unit, direction, partner).
    AttackTreaty(UnitId, Dir8, crate::game::Slot),
}

pub struct MapView {
    pub vx: i32,
    pub vy: i32,
    mode: Mode,
    pending: Pending,
    runner: Option<TurnRunner>,
    current_decision: Option<Decision>,
    /// A decision raised outside the turn runner (the Pyramids' government choice right after
    /// a revolution), answered by the dialog on top and applied directly.
    loose_decision: Option<Decision>,
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
            loose_decision: None,
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

    /// The view's left column wraps around with the map (`vx` in 0..MAP_W).
    pub fn center_on(&mut self, x: i32, y: i32) {
        self.vx = Map::wrap_x(x - 6);
        self.vy = (y - 5).clamp(0, MAP_H as i32 - VIEW_H);
    }

    fn ensure_visible(&mut self, x: i32, y: i32) {
        let rel = Map::wrap_x(x - self.vx);
        if rel < 1 || rel >= VIEW_W - 1 || y < self.vy + 1 || y >= self.vy + VIEW_H - 1 {
            self.center_on(x, y);
        }
    }

    fn tile_at_pixel(&self, px: i32, py: i32) -> Option<(i32, i32)> {
        if px < VIEW_X || py < VIEW_Y || px >= VIEW_X + VIEW_W * TILE || py >= VIEW_Y + VIEW_H * TILE {
            return None;
        }
        let x = Map::wrap_x(self.vx + (px - VIEW_X) / TILE);
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
                TurnStep::Combat { defender_at, visible, won, attacker, defender, defender_owner, .. } => {
                    if visible {
                        // CIV.EXE: the "won" variants only when the human owns the winning unit
                        let human_won = !won && defender_owner == ctx.game().human_slot;
                        Self::battle_sound(ctx, attacker, defender, won, human_won);
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
                if *e == crate::game::RuleError::Zoc {
                    ctx.audio.play(crate::audio::Sound::IllegalMove);
                }
                self.pending = Pending::Error;
                Transition::Push(s)
            }
            None => Transition::None,
        }
    }

    /// The battle sound as CIV.EXE picks it (combat routine at 0x19632): a winning bomber
    /// has its own sound; otherwise the "won" fanfare when the human owns the winning unit,
    /// "lost" for the rest (including AI vs AI), with the explosion variants when the
    /// winner has attack above 4 or defence above 2.
    fn battle_sound(ctx: &UiContext, attacker: UnitTypeId, defender: UnitTypeId, attacker_won: bool, human_won: bool) {
        if attacker_won && attacker == rules::unit_ids::BOMBER {
            ctx.audio.play(crate::audio::Sound::Bomber);
            return;
        }
        let winner = &rules::UNITS[if attacker_won { attacker } else { defender } as usize];
        let modern = winner.att > 4 || winner.def > 2;
        ctx.audio.play(crate::audio::battle_sound(human_won, modern));
    }

    fn try_move(&mut self, ctx: &mut UiContext, d: Dir8) -> Transition {
        self.move_active(ctx, d, false)
    }

    /// Move the active unit; `keep_moving` completes a caravan's move into one of our cities.
    fn move_active(&mut self, ctx: &mut UiContext, d: Dir8, keep_moving: bool) -> Transition {
        let Some(id) = ctx.game().active_unit else { return Transition::None };
        let before = ctx.game().unit(id).pos();
        let attacker_kind = ctx.game().unit(id).kind;
        let r = if keep_moving { ctx.game_mut().move_on(id, d) } else { ctx.game_mut().try_move(id, d) };
        match r {
            Ok(outcome) => {
                if let MoveOutcome::Attacked { won, defender, .. } = outcome {
                    Self::battle_sound(ctx, attacker_kind, defender, won, won);
                }
                let g = ctx.game();
                let alive = g.unit_opt(id).is_some();
                if alive {
                    let (x, y) = g.unit(id).pos();
                    if (x, y) != before {
                        self.ensure_visible(x, y);
                    }
                }
                let t = match outcome {
                    MoveOutcome::BreakTreaty(o) => self.treaty_question(ctx, id, d, o),
                    MoveOutcome::EnteredForeignCity(c) => self.foreign_city_menu(ctx, id, c),
                    MoveOutcome::CaravanArrived(c) => self.caravan_menu(ctx, id, c, d),
                    MoveOutcome::BribeOffer(t) => self.bribe_offer(ctx, id, t, d),
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

    /// "We have signed a\npeace treaty with\nthe <Plural>!" Cancel action. / Break treaty. (CIV.EXE
    /// 0x1a3cf, at (80, 80)).
    fn treaty_question(&mut self, ctx: &mut UiContext, id: UnitId, d: Dir8, o: crate::game::Slot) -> Transition {
        let lines = vec!["We have signed a".into(), "peace treaty with".into(), format!("the {}!", ctx.game().player(o).plural())];
        self.pending = Pending::AttackTreaty(id, d, o);
        Transition::Push(Box::new(MenuDialog::new(lines, vec!["Cancel action.".into(), "Break treaty.".into()]).with_pos(80, 80)))
    }

    fn foreign_city_menu(&mut self, ctx: &mut UiContext, id: UnitId, c: crate::game::CityId) -> Transition {
        if ctx.game().unit(id).kind == rules::unit_ids::CARAVAN {
            // a caravan entering a foreign city sets up the route at once (CIV.EXE 0x5d15)
            return self.caravan_choice(ctx, id, c, 1);
        }
        crate::ui::unit_actions::diplomat_menu(self, ctx, id, c)
    }

    /// "Will you?" when a caravan reaches one of our cities (CIV.EXE 0x5cbe): the trade
    /// route is greyed when the city builds a wonder and lies within 10 squares of home.
    fn caravan_menu(&mut self, ctx: &mut UiContext, id: UnitId, c: crate::game::CityId, d: Dir8) -> Transition {
        let g = ctx.game();
        let u = g.unit(id);
        let city = g.city(c);
        let wonder = matches!(city.production, crate::game::Production::Wonder(_));
        let near = u.home.and_then(|h| g.city_opt(h)).map(|h| Map::dist(h.pos(), city.pos()) < 10).unwrap_or(true);
        let mut items = vec!["Keep moving".to_string(), "Establish trade route".to_string()];
        if wonder {
            items.push("Help build WONDER.".into());
        }
        let disabled = items.iter().enumerate().map(|(i, _)| i == 1 && wonder && near).collect();
        self.pending = Pending::CaravanMenu(id, c, d);
        Transition::Push(Box::new(MenuDialog::new(vec!["Will you?".into()], items).with_pos(100, 80).with_disabled(disabled)))
    }

    /// "ADJ UNIT\nwill desert for N$.\nTreasury: N$" with "Forget it." / "Pay", the latter only
    /// when the treasury holds the price (CIV.EXE o 0x1dc4b).
    fn bribe_offer(&mut self, ctx: &mut UiContext, id: UnitId, t: UnitId, d: Dir8) -> Transition {
        let g = ctx.game();
        let tu = g.unit(t);
        let cost = g.bribe_cost(t);
        let gold = g.human().gold;
        let lines = vec![
            format!("{} {}", g.player(tu.owner).adjective(), tu.info().name),
            format!("will desert for {cost}$."),
            format!("Treasury: {gold}$"),
        ];
        if cost > gold {
            return Transition::Push(Box::new(MessageBox::new(lines)));
        }
        self.pending = Pending::Bribe(id, t, d);
        Transition::Push(Box::new(MenuDialog::new(lines, vec!["Forget it.".into(), "Pay".into()])))
    }

    /// Carry out a caravan's choice at city `c`: 0 keep moving, 1 trade route, 2 help build
    /// the wonder.
    fn caravan_choice(&mut self, ctx: &mut UiContext, id: UnitId, c: crate::game::CityId, choice: usize) -> Transition {
        use crate::game::turn::CaravanAction;
        let action = match choice {
            1 => CaravanAction::TradeRoute(c),
            2 => CaravanAction::HelpWonder(c),
            _ => CaravanAction::KeepMoving,
        };
        if let Err(e) = ctx.game_mut().caravan_action(id, action) {
            return self.error(ctx, &e);
        }
        if let Some(s) = self.next_event_screen(ctx) {
            self.pending = Pending::Event;
            return Transition::Push(s);
        }
        if ctx.game().unit_opt(id).is_none() {
            self.activate_next(ctx);
        }
        Transition::None
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
                // the terraforming lines name the result as CIV.EXE 0x1c1fe does: a square
                // whose irrigation or mine changes its terrain reads "Change to <terrain>"
                let tile = g.active_unit.and_then(|a| g.unit_opt(a)).map(|u| g.map.tile(u.x, u.y).terrain.info());
                let terraform = |f: Option<rules::Terraform>, build: &str, key: char| match f {
                    Some(rules::Terraform::Becomes(t)) => format!("Change to {}  {key}", t.name()),
                    _ => format!("{build}  {key}"),
                };
                let v: Vec<String> = vec![
                    "No Orders  space".into(),
                    "Found New City  b".into(),
                    "Build Road  r".into(),
                    terraform(tile.map(|i| i.irrigate), "Build Irrigation", 'i'),
                    terraform(tile.map(|i| i.mine), "Build Mines", 'm'),
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
                    !settlers || tile.map_or(true, |i| i.irrigate == rules::Terraform::No),
                    !settlers || tile.map_or(true, |i| i.mine == rules::Terraform::No),
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
            3 => {
                // SpaceShips is greyed until some civ has started a ship (0x1c710)
                disabled = vec![false, false, false, false, false, !crate::ui::spaceship_ui::any_ship_started(g)];
                vec!["Wonders of World F7".into(), "Top 5 Cities     F8".into(), "Civ. Score       F9".into(), "World Map       F10".into(), "Demographics    F11".into(), "SpaceShips      F12".into()]
            }
            _ => vec!["Complete".into(), "Civilization Advances".into(), "City Improvements".into(), "Military Units".into(), "Terrain Types".into(), "Miscellaneous".into()],
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
                // the unit stays active with its moves (CIV.EXE 0x4f28 ends without a move)
                let r = ctx.game_mut().set_home(id);
                match r {
                    Ok(()) => Transition::None,
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
            let nx = Map::wrap_x(cursor.0 + dx);
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
                Ok(MoveOutcome::CaravanArrived(c)) => {
                    let g = ctx.game_mut();
                    g.unit_mut(id).order = UnitOrder::None;
                    g.unit_mut(id).goto = None;
                    g.active_unit = Some(id);
                    return self.caravan_menu(ctx, id, c, dir);
                }
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
                let (x, y) = (Map::wrap_x(self.vx + tx), self.vy + ty);
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
                let (x, y) = (Map::wrap_x(self.vx + tx), self.vy + ty);
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
            let (px, py) = (VIEW_X + Map::wrap_x(cursor.0 - self.vx) * TILE, VIEW_Y + (cursor.1 - self.vy) * TILE);
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
        // the view box, split in two where it runs over the east-west edge
        let (top, bottom) = (MINIMAP.y + self.vy, MINIMAP.y + self.vy + VIEW_H - 1);
        for i in 0..VIEW_W {
            let x = MINIMAP.x + Map::wrap_x(self.vx + i);
            fb.put(x, top, theme::WHITE);
            fb.put(x, bottom, theme::WHITE);
        }
        for y in top..=bottom {
            fb.put(MINIMAP.x + self.vx, y, theme::WHITE);
            fb.put(MINIMAP.x + Map::wrap_x(self.vx + VIEW_W - 1), y, theme::WHITE);
        }
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
        // palace strip (CIV.EXE 0x340f): cyan, with a white bar of two pixels per turn of
        // peace after 1 AD along its top, and the palace on a green ground line (0x3823)
        let strip = Rect::new(3, 60, 74, 11);
        fb.fill_rect(strip, theme::CYAN);
        let peace = p.peace_turns as i32;
        if peace > 1 {
            fb.fill_rect(Rect::new(3, 60, (2 * peace).min(60), 2), theme::WHITE);
        }
        fb.hline(3, 76, 71, theme::GREEN);
        if p.capital.is_some() {
            draw_mini_palace(fb, ctx, &p.palace(), 8, 58);
        }
        let st = TextStyle::new(0, theme::TEXT);
        // the original appends the font's person ('#') and coin ('$') glyphs to the figures
        let pop = g.total_population(g.human_slot);
        if pop > 0 {
            draw_text(fb, &ctx.fonts, &st, 2, 73, &format!("{}#", format_pop(pop)));
        }
        draw_text(fb, &ctx.fonts, &st, 2, 81, &g.year_text());
        // research progress bulb and global-warming sun: 8x8 cells of SP299, four stages each
        let sheet = ctx.pic("SP299");
        // 0x3544 leaves out the early-advance minimum of the real cost
        let k = if g.year() < 0 { 1 } else { 2 };
        let cost = p.research_count as i32 * g.research_multiplier(g.human_slot, false) * k;
        let stage = (p.beakers * 4 / (cost + 1)).clamp(0, 3);
        fb.blit_part(&sheet.surf, Rect::new(160 + 8 * stage, 120, 8, 8), 48, 80, None);
        if g.polluted_tiles > 0 {
            let stage = (g.polluted_tiles as i32 / 4).clamp(0, 3);
            fb.blit_part(&sheet.surf, Rect::new(160 + 8 * stage, 128, 8, 8), 56, 80, None);
        }
        let sci = p.sci() / 10;
        draw_text(fb, &ctx.fonts, &st, 2, 89, &format!("{}$ {}.{}.{}", p.gold, p.lux / 10, p.tax / 10, sci));
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
                if let Some(d) = self.loose_decision.take() {
                    let reply = dialogs::reply_for(&d, &result);
                    ctx.game_mut().apply_decision(&d, reply);
                }
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
            Pending::Bribe(_, t, d) => {
                if result == DialogResult::Choice(1) && ctx.game().unit_opt(t).is_some() {
                    let slot = ctx.game().human_slot;
                    ctx.game_mut().bribe_unit(slot, t);
                    return self.move_active(ctx, d, false);
                }
                Transition::None
            }
            Pending::CaravanMenu(id, c, d) => match result {
                DialogResult::Choice(0) => self.move_active(ctx, d, true),
                DialogResult::Choice(i) => self.caravan_choice(ctx, id, c, i),
                _ => Transition::None,
            },
            Pending::QuitConfirm => {
                if result == DialogResult::No {
                    // second option = yes (see YesNo ordering above: ("No","Yes"))
                    return Transition::Quit;
                }
                Transition::None
            }
            Pending::Diplomat(id, c) => match result {
                DialogResult::Choice(i) => crate::ui::unit_actions::diplomat_choice(self, ctx, id, c, i),
                _ => Transition::None,
            },
            Pending::TreatyBreak(id, c) => match result {
                DialogResult::Choice(1) => crate::ui::unit_actions::treaty_broken(self, ctx, id, c),
                _ => Transition::None,
            },
            Pending::Incite(id, c) => match result {
                DialogResult::Choice(i) => crate::ui::unit_actions::incite_choice(self, ctx, id, c, i),
                _ => Transition::None,
            },
            // senate_check_attack (CIV.EXE 0x1a3b4): "Break treaty." goes ahead unless the
            // Senate overrules; "Cancel action." cancels the unit's goto
            Pending::AttackTreaty(id, d, o) => match result {
                DialogResult::Choice(1) if ctx.game().senate_overrules(ctx.game().human_slot) => {
                    self.pending = Pending::Event;
                    Transition::Push(Box::new(MessageBox::new(vec!["Overruled by".into(), "the Senate.".into(), "Action canceled.".into()]).with_pos(100, 80)))
                }
                DialogResult::Choice(1) => {
                    ctx.game_mut().treaty_break_ok = Some(o);
                    if ctx.game().unit_opt(id).is_some() && ctx.game().active_unit == Some(id) {
                        return self.move_active(ctx, d, false);
                    }
                    ctx.game_mut().treaty_break_ok = None;
                    Transition::None
                }
                _ => {
                    if ctx.game().unit_opt(id).is_some() {
                        let u = ctx.game_mut().unit_mut(id);
                        u.goto = None;
                        if u.order == UnitOrder::Goto {
                            u.order = UnitOrder::None;
                        }
                    }
                    Transition::None
                }
            },
            Pending::None => Transition::None,
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
                    self.vx = Map::wrap_x(self.vx + dx);
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
                        // CIV.EXE 0x479c: a square next to the active unit moves it there, any
                        // other explored square shows the Civilopedia page of its terrain
                        if let Some(id) = g.active_unit {
                            let (ux, uy) = g.unit(id).pos();
                            if let Some(d) = Dir8::from_delta(tx - ux, ty - uy) {
                                return self.try_move(ctx, d);
                            }
                        }
                        if g.explored(me, tx, ty) {
                            let t = g.map.tile(tx, ty).terrain;
                            return Transition::Push(crate::ui::civilopedia::entry_screen(crate::ui::pedia_data::Entry::Terrain(t), 0));
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

    /// Event dialog with map-view specific preparation (centre on nuclear strikes).
    /// Queue the game's new events and return the dialog for the first one that has one.
    fn next_event_screen(&mut self, ctx: &mut UiContext) -> Option<Box<dyn Screen>> {
        let events = ctx.game_mut().take_events();
        self.queued.extend(events);
        while let Some(e) = self.queued.pop_front() {
            if let crate::game::GameEvent::NeedsDecision(d) = &e {
                if self.runner.is_none() && self.loose_decision.is_none() {
                    self.loose_decision = Some(d.clone());
                    return Some(dialogs::screen_for_decision(ctx, d));
                }
                continue;
            }
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
        let blast = match e {
            crate::game::GameEvent::NuclearStrike { at, .. } => Some(*at),
            crate::game::GameEvent::Meltdown { city } => ctx.game().city_opt(*city).filter(|c| c.owner == ctx.game().human_slot).map(|c| c.pos()),
            _ => None,
        };
        if let Some(at) = blast {
            self.vx = Map::wrap_x(at.0 - VIEW_W / 2);
            self.vy = (at.1 - VIEW_H / 2).clamp(0, MAP_H as i32 - VIEW_H);
        }
        // barbarians: an uprising shows its city (0x2fcb), a landing scrolls only when the
        // square is outside the inner window (scroll_to 0x09505)
        let me = ctx.game().human_slot;
        let view = match e {
            crate::game::GameEvent::BarbarianUprising { at, target } if *target == me => ctx.game().nearest_city_exe(at.0, at.1).0.map(|c| ctx.game().city(c).pos()),
            crate::game::GameEvent::BarbariansLanded { at, target, .. } if *target == me => {
                let inside = (2..=13).contains(&Map::delta_x(self.vx, at.0)) && (self.vy + 2..=self.vy + 9).contains(&at.1);
                (!inside).then_some(*at)
            }
            _ => None,
        };
        if let Some(at) = view {
            self.vx = Map::wrap_x(at.0 - 8);
            self.vy = (at.1 - 6).clamp(0, MAP_H as i32 - VIEW_H);
        }
        dialogs::screen_for_event(ctx, e)
    }

    pub(crate) fn set_pending(&mut self, p: Pending) {
        self.pending = p;
    }

    /// Show the next queued game event, or else move on to the next unit when the active
    /// one is gone.
    pub(crate) fn show_events(&mut self, ctx: &mut UiContext) -> Transition {
        if let Some(s) = self.next_event_screen(ctx) {
            self.pending = Pending::Event;
            return Transition::Push(s);
        }
        if ctx.game().active_unit.is_none() {
            self.activate_next(ctx);
        }
        Transition::None
    }

    pub fn show_error(&mut self, ctx: &mut UiContext, e: &crate::game::RuleError) -> Transition {
        self.error(ctx, e)
    }
}

/// The palace in the status panel (CIV.EXE 0x3823): each building section above level 0 as
/// a 7x15 piece of SP257, 7 px apart from `x`. A style row of SP257 (y 16 * style) holds four
/// levels of five pieces from x 160: left tower, left wall, centre, right wall, right tower.
fn draw_mini_palace(fb: &mut Surface, ctx: &UiContext, palace: &crate::game::palace::Palace, x: i32, y: i32) {
    let lv = |i: usize| palace.level.get(i).copied().unwrap_or(-1);
    for s in 0..9usize {
        let level = lv(s);
        if level <= 0 {
            continue;
        }
        let piece = match s {
            0..=3 if lv(s.wrapping_sub(1)) == -1 || s == 1 => 0,
            0..=3 => 1,
            5..=8 if lv(s + 1) == -1 || s == 7 => 4,
            5..=8 => 3,
            _ => 2,
        };
        let idx = (level as i32 - 1).clamp(0, 3) * 5 + piece;
        let src = Rect::new(160 + 7 * idx, 16 * palace.style[s] as i32, 7, 15);
        fb.blit_part(&ctx.sp.surf, src, x + 7 * s as i32, y - 1, Some(0));
    }
}
