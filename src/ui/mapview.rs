//! The main map screen: viewport, menu bar, sidebar, unit orders and end-of-turn processing.

use crate::game::turn::{TurnRunner, TurnStep};
use crate::game::{tile_flags, Decision, Dir8, Map, MoveOutcome, UnitId, UnitOrder};
use crate::gfx::text::{draw_text, draw_text_centered};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::{Event, Key, MouseButton};
use crate::rules::{self, UnitTypeId, MAP_H, SLOT_COLOURS};
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

/// A line of a pull-down menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BarAction {
    Rate(bool),
    FindCity,
    Options,
    Save,
    Revolution,
    Quit,
    /// An order: the key (with Shift) it posts, as the original stores it in ds:0xd4ca.
    Key(Key, bool),
    Advisor(u8),
    Civilopedia(usize),
    Nothing,
}

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
    /// senate_check_attack before a pillage near a treaty partner's city (CIV.EXE 0x4cea).
    PillageTreaty(UnitId, crate::game::Slot),
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
    /// What each line of the open pull-down menu does.
    bar_actions: Vec<BarAction>,
    pub status_line: String,
    /// Show the FIRSTMOVE hint when the view first appears.
    pub first_turn_hint: bool,
    /// The Chieftain screen-layout tutorial has been shown (once, before FIRSTMOVE).
    tutorial_shown: bool,
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
            bar_actions: Vec::new(),
            status_line: String::new(),
            first_turn_hint: false,
            tutorial_shown: false,
            auto_end: 0,
            queued: Default::default(),
            hint_checked: None,
        }
    }

    /// Put (x, y) at view column 7, row 6, as CIV.EXE does for units, the minimap and FindCity
    /// (`draw_map_view(civ, x - 7, y - 6)`, e.g. o 0x2486, 0x43b8); draw_map_view clamps the
    /// row to 0..38. The view's left column wraps around with the map (`vx` in 0..MAP_W).
    pub fn center_on(&mut self, x: i32, y: i32) {
        self.vx = Map::wrap_x(x - 7);
        self.vy = (y - 6).clamp(0, MAP_H as i32 - VIEW_H);
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

    /// The five pull-down menus as CIV.EXE's segment 0x1c02 builds them: `menu(text, x, 8)` at
    /// x = 16, 72, 112, 144, 182 (framed, on the stone). GAME greys Save Game on turn 0
    /// (ds:0xb276 = 0x10) and has a selectable blank line before Retire. ORDERS (0x1c1fe) lists
    /// only the orders the active unit has, each posting its hotkey (shown in colour 15 behind a
    /// 0x8f code); Build Irrigation is greyed without water nearby, Build Fortress without
    /// Construction. WORLD greys SpaceShips until a ship exists (ds:0xb276 = 0x20).
    fn bar_menu(&mut self, ctx: &mut UiContext, i: usize) -> Transition {
        let g = ctx.game();
        let mut items: Vec<String> = Vec::new();
        let mut actions: Vec<BarAction> = Vec::new();
        let mut disabled: Vec<bool> = Vec::new();
        let mut add = |items: &mut Vec<String>, text: &str, action: BarAction, grey: bool| {
            items.push(text.to_string());
            actions.push(action);
            disabled.push(grey);
        };
        let x = [16, 72, 112, 144, 182][i];
        match i {
            0 => {
                let first_turn = g.turn == 0;
                add(&mut items, "Tax Rate", BarAction::Rate(false), false);
                add(&mut items, "Luxuries Rate", BarAction::Rate(true), false);
                add(&mut items, "FindCity", BarAction::FindCity, false);
                add(&mut items, "Options", BarAction::Options, false);
                add(&mut items, "Save Game", BarAction::Save, first_turn);
                add(&mut items, "REVOLUTION!", BarAction::Revolution, false);
                add(&mut items, " ", BarAction::Nothing, false);
                add(&mut items, "Retire", BarAction::Quit, false);
                add(&mut items, "QUIT to DOS", BarAction::Quit, false);
            }
            1 => {
                let key = |c: char| BarAction::Key(Key::Char(c), false);
                let unit = g.active_unit.and_then(|a| g.unit_opt(a));
                // without an active unit the menu does not open (o 0x1c206)
                if unit.is_none() {
                    return Transition::None;
                }
                add(&mut items, "No Orders \u{8f}space", BarAction::Key(Key::Space, false), false);
                if let Some(u) = unit {
                    let (x, y) = u.pos();
                    let flags = g.map.tile(x, y).flags;
                    let info = g.map.tile(x, y).terrain.info();
                    let in_city = g.city_at(x, y).is_some();
                    let kind = rules::UNITS[u.kind as usize];
                    if u.kind == rules::unit_ids::SETTLERS {
                        add(&mut items, if in_city { "Add to City \u{8f}b" } else { "Found New City \u{8f}b" }, key('b'), false);
                        if flags & tile_flags::ROAD == 0 {
                            add(&mut items, "Build Road \u{8f}r", key('r'), false);
                        } else if flags & tile_flags::RAILROAD == 0 && g.player(u.owner).knows(rules::advance_ids::RAILROAD) {
                            add(&mut items, "Build RailRoad \u{8f}r", key('r'), false);
                        }
                        if flags & tile_flags::IRRIGATION == 0 {
                            match info.irrigate {
                                rules::Terraform::Bonus(_) => add(&mut items, "Build Irrigation \u{8f}i", key('i'), !g.irrigation_water(x, y)),
                                rules::Terraform::Becomes(t) => add(&mut items, &format!("Change to {} \u{8f}i", t.name()), key('i'), false),
                                rules::Terraform::No => {}
                            }
                        }
                        if flags & tile_flags::MINE == 0 {
                            match info.mine {
                                rules::Terraform::Bonus(_) => add(&mut items, "Build Mines \u{8f}m", key('m'), false),
                                rules::Terraform::Becomes(t) => add(&mut items, &format!("Change to {} \u{8f}m", t.name()), key('m'), false),
                                rules::Terraform::No => {}
                            }
                        }
                        if flags & tile_flags::POLLUTION != 0 {
                            add(&mut items, "Clean up Pollution \u{8f}p", key('p'), false);
                        }
                        let construction = g.player(u.owner).knows(rules::advance_ids::CONSTRUCTION);
                        add(&mut items, "Build Fortress \u{8f}f", BarAction::Key(Key::Char('f'), true), !construction);
                    } else if kind.domain == rules::Domain::Land {
                        add(&mut items, "Fortify \u{8f}f", key('f'), false);
                    }
                    add(&mut items, "Wait \u{8f}w", key('w'), false);
                    add(&mut items, "Sentry \u{8f}s", key('s'), false);
                    add(&mut items, "GoTo", key('g'), false);
                    let improved = flags & (tile_flags::ROAD | tile_flags::RAILROAD | tile_flags::IRRIGATION | tile_flags::MINE) != 0;
                    if improved && u.kind < 26 && u.kind != rules::unit_ids::FIGHTER {
                        add(&mut items, "Pillage \u{8f}P", BarAction::Key(Key::Char('p'), true), false);
                    }
                    if in_city {
                        add(&mut items, "Home City \u{8f}h", key('h'), false);
                    }
                    if (kind.role == rules::AiRole::Transport || u.kind == rules::unit_ids::CARRIER) && u.seen_by != 0xff {
                        add(&mut items, "Unload \u{8f}u", key('u'), false);
                    }
                    add(&mut items, " ", BarAction::Nothing, false);
                    add(&mut items, "Disband Unit \u{8f}D", BarAction::Key(Key::Char('d'), true), false);
                }
            }
            2 => {
                for (n, t) in ["City Status (F1)", "Military Advisor (F2)", "Intelligence Advisor (F3)", "Attitude Advisor (F4)", "Trade Advisor (F5)", "Science Advisor (F6)"].iter().enumerate() {
                    add(&mut items, t, BarAction::Advisor(n as u8 + 1), false);
                }
            }
            3 => {
                let no_ship = !crate::ui::spaceship_ui::any_ship_started(g);
                for (n, t) in ["Wonders of the World (F7)", "Top 5 Cities (F8)", "Civilization Score (F9)", "World Map (F10)", "Demographics", "SpaceShips"].iter().enumerate() {
                    add(&mut items, t, BarAction::Advisor(n as u8 + 7), n == 5 && no_ship);
                }
            }
            _ => {
                for (n, t) in ["Complete", "Civilization Advances", "City Improvements", "Military Units", "Terrain Types", "Miscellaneous"].iter().enumerate() {
                    add(&mut items, t, BarAction::Civilopedia(n), false);
                }
            }
        }
        self.bar_actions = actions;
        self.pending = Pending::BarMenu(i);
        self.bar_menu_open = Some(i);
        Transition::Push(Box::new(MenuDialog::new(vec![], items).with_pos(x, 8).with_disabled(disabled)))
    }

    fn bar_menu_choice(&mut self, ctx: &mut UiContext, _menu: usize, choice: usize) -> Transition {
        match self.bar_actions.get(choice).copied().unwrap_or(BarAction::Nothing) {
            BarAction::Rate(lux) => Transition::Push(crate::ui::reports::rate_dialog(ctx, lux)),
            BarAction::FindCity => Transition::Push(crate::ui::reports::find_city_dialog(ctx)),
            BarAction::Options => Transition::Push(crate::ui::reports::options_dialog(ctx)),
            BarAction::Save => Transition::Push(crate::ui::reports::save_dialog(ctx)),
            BarAction::Revolution => {
                self.pending = Pending::RevolutionConfirm;
                Transition::Push(Box::new(YesNo::new(vec!["Are you sure you want a REVOLUTION?".into()], "_No thanks.", "_Yes, we need a new government.").with_pos(64, 80)))
            }
            BarAction::Quit => {
                self.pending = Pending::QuitConfirm;
                Transition::Push(Box::new(YesNo::new(vec!["Are you sure you".into(), "want to Quit?".into()], "Keep Playing", "Yes, Quit").with_pos(100, 80)))
            }
            BarAction::Key(key, shift) => self.key_order(ctx, key, crate::platform::Mods { shift, ctrl: false, alt: false }),
            BarAction::Advisor(n) => Transition::Push(crate::ui::reports::advisor(ctx, n)),
            BarAction::Civilopedia(c) => Transition::Push(crate::ui::civilopedia::category(ctx, c)),
            BarAction::Nothing => Transition::None,
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
                    // 'P' asks senate_check_attack about the nearest city's owner (0x4cea)
                    if let Some(o) = active.and_then(|id| ctx.game().pillage_treaty(id)) {
                        let lines = vec!["We have signed a".into(), "peace treaty with".into(), format!("the {}!", ctx.game().player(o).plural())];
                        self.pending = Pending::PillageTreaty(active.unwrap(), o);
                        return Transition::Push(Box::new(MenuDialog::new(lines, vec!["Cancel action.".into(), "Break treaty.".into()]).with_pos(80, 80)));
                    }
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
                Transition::Push(Box::new(crate::ui::widgets::CityNamePrompt::new(&name)))
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
            let dest = u.goto;
            let Some(dir) = ctx.game_mut().next_goto_step(id) else {
                let g = ctx.game_mut();
                g.unit_mut(id).order = UnitOrder::None;
                g.unit_mut(id).goto = None;
                break;
            };
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

    /// `draw_menu_bar` (0x336a): the titles "G\x87AME" ... at y 1 in colour 15, the rest of
    /// each after its first letter in 7 (the 0x87 code); an open menu does not change them.
    fn draw_menubar(&self, fb: &mut Surface, ctx: &UiContext) {
        fb.fill_rect(Rect::new(0, 0, 320, MENU_H), theme::BLACK);
        for (label, x) in MENU_LABELS.iter() {
            let end = draw_text(fb, &ctx.fonts, &TextStyle::new(0, 15), *x, 1, &label[..1]);
            draw_text(fb, &ctx.fonts, &TextStyle::new(0, 7), end, 1, &label[1..]);
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

    /// Map square at the minimap's top left: column `vx - 32`, row `vy - 19` (0 with Navigation).
    fn minimap_origin(&self, ctx: &UiContext) -> (i32, i32) {
        let navigation = ctx.game().human().knows(rules::advance_ids::NAVIGATION);
        (self.vx - 32, if navigation { 0 } else { self.vy - 19 })
    }

    /// The sidebar minimap as `draw_map_view` (0x1a68e, o 0x1a826..0x1aa28) draws it: 80x50 of
    /// the minimap band, scrolled so that map column `vx - 32` is its left edge and row
    /// `vy - 19` its top (row 0 once the civ knows Navigation); explored water in colour 1,
    /// land in 2 (`unit_look` 0x81b4), the cities the human has seen or owns as a dot in the
    /// owner's light colour, then the 17x10 view outline in 15.
    fn draw_minimap(&self, fb: &mut Surface, ctx: &UiContext) {
        let g = ctx.game();
        let me = g.human_slot;
        let (mx0, my0) = self.minimap_origin(ctx);
        fb.fill_rect(Rect::new(0, 8, 80, 50), 0);
        for my in 0..50 {
            let y = my0 + my;
            if !(0..MAP_H as i32).contains(&y) {
                continue;
            }
            for mx in 0..80 {
                let x = Map::wrap_x(mx0 + mx);
                if g.explored(me, x, y) {
                    let water = g.map.tile(x, y).terrain == rules::Terrain::Ocean;
                    fb.put(mx, 8 + my, if water { 1 } else { 2 });
                }
            }
        }
        for c in g.all_cities() {
            let my = c.y - my0;
            if (0..50).contains(&my) && (c.owner == me || g.explored(me, c.x, c.y)) {
                fb.put(Map::wrap_x(c.x - mx0), 8 + my, SLOT_COLOURS[(c.owner & 7) as usize].light);
            }
        }
        // the active unit blinks on the minimap too: white while it is hidden on the map
        if !self.blink {
            if let Some(u) = g.active_unit.and_then(|a| g.unit_opt(a)) {
                let my = u.y - my0;
                if (0..50).contains(&my) {
                    fb.put(Map::wrap_x(u.x - mx0), 8 + my, 15);
                }
            }
        }
        crate::ui::widgets::bevel(fb, 0, 8, 79, 49, 15, 8);
        fb.rect_outline(Rect::new(self.vx - mx0 - 1, self.vy - my0 + 7, 18, 11), 15);
    }

    fn draw_sidebar(&self, fb: &mut Surface, ctx: &UiContext) {
        let g = ctx.game();
        let p = g.human();
        // the status box and the unit panel are draw_panel_frame(0, 58, 80, 39) (0x33fe) and
        // (0, 97, 80, 103) (0x3ec4): stone texture from each panel's corner, 15/8 bevel
        crate::ui::widgets::panel_frame(fb, ctx, Rect::new(0, 58, 80, 39));
        crate::ui::widgets::panel_frame(fb, ctx, Rect::new(0, 97, 80, 103));
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
        // the sun follows the warming counter ds:0xb1ea (0x35b1): shown when it is not 0
        if g.warming_counter > 0 {
            let stage = (g.warming_counter as i32 / 4).clamp(0, 3);
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
                        // strcat_city_name ("NONE" for no home) in the panel's font (o 0x7ad9)
                        draw_text(fb, &ctx.fonts, &st, 4, y, &home);
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
        // turn 0 on Chieftain: the screen-layout tutorial first (game_turn 0x2470 -> overlay 2
        // +0x17d9), then the FIRSTMOVE note
        if self.first_turn_hint && !self.tutorial_shown {
            self.tutorial_shown = true;
            let g = ctx.game();
            if g.turn == 0 && g.setup.difficulty == crate::rules::Difficulty::Chieftain {
                return Transition::Push(Box::new(LayoutTutorial::new(ctx, (Map::wrap_x(self.vx + 7), self.vy + 6))));
            }
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
            return Transition::Push(Box::new(YesNo::new(vec!["Are you sure you".into(), "want to Quit?".into()], "Keep Playing", "Yes, Quit").with_pos(100, 80)));
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
            Pending::PillageTreaty(id, _) => match result {
                DialogResult::Choice(1) if ctx.game().senate_overrules(ctx.game().human_slot) => {
                    self.pending = Pending::Event;
                    Transition::Push(Box::new(MessageBox::new(vec!["Overruled by".into(), "the Senate.".into(), "Action canceled.".into()]).with_pos(100, 80)))
                }
                DialogResult::Choice(1) if ctx.game().active_unit == Some(id) => self.unit_order(ctx, UnitOrder::Pillage),
                _ => Transition::None,
            },
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
                        Transition::Push(Box::new(YesNo::new(vec!["Are you sure you".into(), "want to Quit?".into()], "Keep Playing", "Yes, Quit").with_pos(100, 80)))
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
                    // o 0x46b2: the clicked minimap square, in the minimap's scrolled frame
                    let (mx0, my0) = self.minimap_origin(ctx);
                    self.center_on(Map::wrap_x(mx + mx0), (my - 8 + my0).clamp(0, 49));
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
        ctx.game_palette()
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

/// `screen_layout_tutorial` (overlay 2 +0x17d9, 0x234e): over the first map of a Chieftain game,
/// each of the eight squares around the view centre gets a label with its terrain, resource or
/// "Village": a line in 15 from (200 + 16 dx, 112 + 16 dy) to a framed popup at (170 + 64 dx,
/// 100 + 48 dy) (`tutorial_label` 0x24cf, line to the label's y + 6); then "Menu Bar",
/// "Map Window" and "Active Unit" the same way. Any key or click ends it.
struct LayoutTutorial {
    labels: Vec<((i32, i32, i32, i32), String)>,
}

impl LayoutTutorial {
    fn new(ctx: &UiContext, centre: (i32, i32)) -> LayoutTutorial {
        let g = ctx.game();
        let mut labels = Vec::new();
        for d in crate::game::Dir8::ALL {
            let (dx, dy) = d.delta();
            let (x, y) = (Map::wrap_x(centre.0 + dx), centre.1 + dy);
            let Some(t) = g.map.get(x, y) else { continue };
            let info = t.terrain.info();
            let text = if t.has(tile_flags::HUT) {
                "Village".to_string()
            } else if t.special {
                info.special.map(|s| s.name).unwrap_or(info.name).to_string()
            } else {
                info.name.to_string()
            };
            labels.push(((200 + 16 * dx, 112 + 16 * dy, 170 + 64 * dx, 100 + 48 * dy), text));
        }
        labels.push(((160, 6, 200, 16), "Menu Bar".into()));
        labels.push(((48, 32, 88, 24), "Map Window".into()));
        labels.push(((40, 128, 88, 170), "Active Unit".into()));
        LayoutTutorial { labels }
    }
}

impl Screen for LayoutTutorial {
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => Transition::Pop(DialogResult::Done),
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        for ((x1, y1, x2, y2), text) in &self.labels {
            fb.line(*x1, *y1, *x2, *y2 + 6, 15);
            let mut m = MenuDialog::new(vec![text.clone()], vec![]).with_pos(*x2, *y2);
            m.on_enter(ctx);
            m.draw(fb, ctx);
        }
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}
