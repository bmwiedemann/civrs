//! The main map screen: viewport, menu bar, sidebar, unit orders and end-of-turn processing.

use crate::game::turn::{TurnRunner, TurnStep};
use crate::game::{tile_flags, AutoStop, Decision, Dir8, Map, MoveOutcome, UnitId, UnitOrder};
use crate::gfx::text::draw_text;
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
/// The sidebar minimap, 80x50 at (0, 8) (`draw_map_view` 0x1a68e); clicks scroll the map.
pub const MINIMAP: Rect = Rect::new(0, 8, 80, 50);
pub const MENU_H: i32 = 8;

/// What a mouse press on the map screen hits, in move_units' order (CIV.EXE 0x466e..0x479c):
/// above y 8 the pull-down menu of column x / 60 (`game_menu` 0x1c026, any button); left of
/// x 80 the minimap (y < 58), the palace strip (y < 72) or the panels below; else the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SideHit {
    Menu(usize),
    Minimap,
    Palace,
    UnitBox,
    Map,
}

fn side_hit(mx: i32, my: i32) -> SideHit {
    if my < MENU_H {
        SideHit::Menu((mx / 60).clamp(0, 4) as usize)
    } else if mx >= VIEW_X {
        SideHit::Map
    } else if my < MINIMAP.bottom() {
        SideHit::Minimap
    } else if my < 72 {
        SideHit::Palace
    } else {
        SideHit::UnitBox
    }
}

/// Whether a GoTo picked with the mouse may head for a square (CIV.EXE 0x4940..0x49de, on an
/// explored square): land units need land, ships the ocean or a city, aircraft go anywhere.
fn goto_target_ok(domain: rules::Domain, ocean: bool, city: bool) -> bool {
    match domain {
        rules::Domain::Land => !ocean,
        rules::Domain::Sea => ocean || city,
        rules::Domain::Air => true,
    }
}

/// `pick_unit_from_stack(civ, unit)` (CIV.EXE 0x9dc5) with the mouse on: up to 12 of our units
/// on a clicked square in a box at (100, 96 - 8n), each with its sprite at (106, y), name (and
/// " (V)") at (128, y) in 15 and home city at (128, y + 8) in 14, 16 px apart. A click on a
/// row (x 101..219) wakes that unit; with more than one unit the box stays for the next
/// click. A key or a click elsewhere closes it. The result is the last woken unit, else the
/// square's best defender (0x8ee5), as `Choice(unit id)`.
struct StackPicker {
    units: Vec<UnitId>,
    chosen: UnitId,
}

impl StackPicker {
    fn new(mut units: Vec<UnitId>, default: UnitId) -> StackPicker {
        units.truncate(12);
        StackPicker { units, chosen: default }
    }

    fn top(&self) -> i32 {
        96 - 8 * self.units.len() as i32
    }

    /// The row a press at (x, y) hits (0xa022..0xa058).
    fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        if x <= 100 || x >= 220 {
            return None;
        }
        let d = y - self.top() - 5;
        // C division: a press just above the first row still counts as row 0
        let row = d / 16;
        (d > -16 && (row as usize) < self.units.len()).then_some(row as usize)
    }
}

impl Screen for StackPicker {
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        match ev {
            Event::Key { key: Key::Unknown, .. } => Transition::None,
            Event::Key { down: true, .. } => Transition::Pop(DialogResult::Choice(self.chosen as usize)),
            Event::MouseButton { button: MouseButton::Left, x, y, down: true } => match self.row_at(*x, *y) {
                Some(row) => {
                    let id = self.units[row];
                    ctx.game_mut().wake_picked_unit(id);
                    self.chosen = id;
                    if self.units.len() > 1 {
                        Transition::None
                    } else {
                        Transition::Pop(DialogResult::Choice(id as usize))
                    }
                }
                None => Transition::Pop(DialogResult::Choice(self.chosen as usize)),
            },
            Event::MouseButton { down: true, .. } => Transition::Pop(DialogResult::Choice(self.chosen as usize)),
            _ => Transition::None,
        }
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let g = ctx.game();
        let n = self.units.len() as i32;
        let (x, y, w, h) = (100, self.top(), 120, 16 * n + 5);
        // draw_frame(x, y, w, h, 3) (0x1d1a2)
        fb.fill_rect(Rect::new(x + 1, y + 1, w, h), 3);
        crate::ui::widgets::bevel(fb, x + 1, y + 1, w, h, 15, 8);
        fb.rect_outline(Rect::new(x, y, w + 3, h + 3), 0);
        for (i, &id) in self.units.iter().enumerate() {
            let ry = y + 5 + 16 * i as i32;
            let u = g.unit(id);
            tilerender::draw_unit(fb, ctx, g, id, 106, ry, false);
            let mut name = u.info().name.to_string();
            if u.veteran {
                name += " (V)";
            }
            draw_text(fb, &ctx.fonts, &TextStyle::new(0, 15), 128, ry, &name);
            let home = u.home.and_then(|h| g.city_opt(h)).map(|c| c.name.clone()).unwrap_or_else(|| "NONE".into());
            draw_text(fb, &ctx.fonts, &TextStyle::new(0, 14), 128, ry + 8, &home);
        }
    }
}

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
    /// Retire (ds:0xdc48 = 2): the quit question, then the end-of-game scores (0x2c20).
    Retire,
    /// View Replay: the replay of a finished game that goes on (ds:0x4cd8 bit 8).
    Replay,
    /// An order: the key (with Shift) it posts, as the original stores it in ds:0xd4ca.
    Key(Key, bool),
    Advisor(u8),
    Civilopedia(usize),
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Normal,
    /// Choosing a goto destination for the active unit (move_units [bp-0x36]).
    Goto,
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
    /// The stack box of a clicked square (`pick_unit_from_stack` 0x9dc5) is open.
    StackPick,
    FoundCityConfirm(UnitId),
    QuitConfirm,
    RetireConfirm,
    /// The retirement screens are over: leave the game.
    Retired,
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
    /// "Attack at N/3 strength?" (CIV.EXE 0x5fc4): "Attack" moves again, "Cancel" drops the
    /// unit's goto (0x5ef3).
    WeakAttack(UnitId, Dir8),
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
    /// The active unit's blink phase: shown (true) or hidden; see `update`.
    blink: bool,
    /// The frame the current wait for a key began (each key or click starts `blink_wait`
    /// 0x8932 again, which begins with the hidden phase).
    blink_from: u64,
    last_centered: Option<UnitId>,
    bar_menu_open: Option<usize>,
    /// The last pull-down menu, and the one left on the screen behind the Options list
    /// (game_menu_game 0x1c161 runs it without restoring the screen), with its chosen row.
    last_pulldown: Option<MenuDialog>,
    menu_behind: Option<(MenuDialog, usize)>,
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
    /// The turn whose unit loop has used up its advice latch (`move_units` [bp-0x8]: set by
    /// FIRSTMOVE on turn 0 and by the first Settlers that becomes active).
    advice_turn: Option<u32>,
    /// 't' (CIV.EXE 0x71a7, ds:0xdcfc = 1): the map without units until the next key or click.
    terrain_only: bool,
    /// The keyboard-only look cursor's square, while it is on (see `look_key`).
    look: Option<(i32, i32)>,
    /// The loop's pause before it centres the view on a unit off the screen (CIV.EXE
    /// 0x439a): the ticks left and the view to show then (`adopt_view`).
    recentre: Option<(u32, i32, i32)>,
}

/// Ticks the unit loop waits before it centres the view on a unit off the screen (0x4396).
const RECENTRE_TICKS: u32 = 30;

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
            blink_from: 0,
            last_centered: None,
            bar_menu_open: None,
            last_pulldown: None,
            menu_behind: None,
            bar_actions: Vec::new(),
            status_line: String::new(),
            first_turn_hint: false,
            tutorial_shown: false,
            auto_end: 0,
            queued: Default::default(),
            advice_turn: None,
            terrain_only: false,
            look: None,
            recentre: None,
        }
    }

    /// Put (x, y) at view column 7, row 6, as CIV.EXE does for units, the minimap and FindCity
    /// (`draw_map_view(civ, x - 7, y - 6)`, e.g. o 0x2486, 0x43b8); draw_map_view clamps the
    /// row to 0..38. The view's left column wraps around with the map (`vx` in 0..MAP_W).
    pub fn center_on(&mut self, x: i32, y: i32) {
        self.vx = Map::wrap_x(x - 7);
        self.vy = (y - 6).clamp(0, MAP_H as i32 - VIEW_H);
    }

    /// `scroll_to` (CIV.EXE 0x9505), before a unit's move or fight is shown.
    fn ensure_visible(&mut self, x: i32, y: i32) {
        (self.vx, self.vy) = crate::game::Game::view_scrolled_to((self.vx, self.vy), x, y);
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
        self.activate_with(ctx, |g| g.next_active_unit());
    }

    /// `activate_next` from unit `from` itself (after 'u').
    fn activate_from(&mut self, ctx: &mut UiContext, from: UnitId) {
        self.activate_with(ctx, |g| g.next_active_unit_at(from));
    }

    fn activate_with(&mut self, ctx: &mut UiContext, next: impl FnOnce(&mut crate::game::Game) -> Option<UnitId>) {
        // the choice prefers the units in the view and centres the view on any other
        // (Game::next_active_unit, move_units 0x434f..0x43c9)
        let g = ctx.game_mut();
        g.view = Some((self.vx, self.vy));
        g.active_unit = next(g);
        self.adopt_active(ctx);
    }

    /// The active unit still has its turn: the human's, with moves, waiting for orders or
    /// moving by itself.
    fn active_takes_turn(&self, ctx: &UiContext) -> bool {
        let g = ctx.game();
        g.active_unit.is_some_and(|a| g.is_auto_unit(a) || g.unit_opt(a).is_some_and(|u| u.owner == g.human_slot && u.moves_left > 0 && u.order == UnitOrder::None))
    }

    /// Take the game's view. One that the loop centred on a unit off the screen
    /// (`UnitLoop::centred`) comes after the loop's 30-tick pause (0x439a), the old view
    /// standing meanwhile; keys pressed in the pause go (`flush_input` 0x4403 after it).
    fn adopt_view(&mut self, ctx: &mut UiContext) {
        let instant = ctx.instant;
        let g = ctx.game_mut();
        let centred = std::mem::take(&mut g.unit_loop.centred);
        if let Some((vx, vy)) = g.view {
            if centred && !instant {
                self.recentre = Some((RECENTRE_TICKS, vx, vy));
            } else {
                self.vx = vx;
                self.vy = vy;
            }
        }
    }

    /// Follow the game's choice of the active unit and its view.
    fn adopt_active(&mut self, ctx: &mut UiContext) {
        self.adopt_view(ctx);
        let g = ctx.game_mut();
        if let Some(id) = g.active_unit {
            self.last_centered = Some(id);
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
                // the human's loop starts in the view as it stands (move_units 0x434f)
                g.view = Some((self.vx, self.vy));
                let step = runner.step(g, &mut ctx.ai);
                // a message about a square moves the map there (`Game::show_map_at`)
                if let Some((vx, vy)) = g.view {
                    (self.vx, self.vy) = (vx, vy);
                }
                step
            };
            match step {
                TurnStep::Done => {
                    self.runner = None;
                    self.after_turn(ctx);
                    if let Some(slot) = autosave_slot(ctx.game()) {
                        self.pending = Pending::Event;
                        return Transition::Push(Box::new(crate::ui::loadgame::SlotPage::autosave(slot)));
                    }
                    return Transition::None;
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
        // a civrs extra besides CIV.EXE's 50-turn AutoSave (autosave_slot): with AutoSave on,
        // every turn is also saved silently to slot 9
        if ctx.game().options.autosave {
            let path = crate::save::slot_path(&ctx.save_dir, 9);
            let _ = crate::save::save_game(&path, ctx.game());
        }
        self.last_centered = None;
        // the turn's loop has started from the first record (move_units 0x4149) and taken
        // its first unit (`Phase::HumanStart`); one that moves by itself goes on in `update`
        self.adopt_active(ctx);
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
        // a zone of control: tune 37 for the human's unit, the *ZOC note only with Instant
        // Advice (CIV.EXE 0x5b0e..0x5b2e)
        if *e == crate::game::RuleError::Zoc {
            ctx.audio.play(crate::audio::Sound::IllegalMove);
            if !ctx.game().options.instant_advice {
                return Transition::None;
            }
        }
        // *PMAX from the Domestic Advisor at (64, 80) (o 0x1b882)
        if *e == crate::game::RuleError::Other("PMAX") {
            let lines = ctx.texts.king.section("PMAX").map(crate::assets::text::paragraphs).unwrap_or_default();
            self.pending = Pending::Error;
            return Transition::Push(Box::new(crate::ui::widgets::MenuDialog::new(lines, Vec::new()).with_pos(64, 80).with_speaker(crate::ui::widgets::SPEAKER_DOMESTIC)));
        }
        match dialogs::error_screen(ctx, e) {
            Some(s) => {
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
                    MoveOutcome::WeakAttack(m) => {
                        self.pending = Pending::WeakAttack(id, d);
                        Transition::Push(Box::new(MenuDialog::new(vec!["Attack at".into(), format!("{m}/3 strength?")], vec!["Cancel".into(), "Attack".into()]).with_pos(100, 16)))
                    }
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
    /// 0x1a3cf, at (80, 80), from the Foreign Minister, 0x1a3ff).
    /// The move again after a "Break treaty." or "Attack" answer; the answers hold while the
    /// other question is open, and are dropped once the move is done.
    fn confirmed_move(&mut self, ctx: &mut UiContext, d: Dir8) -> Transition {
        let t = self.move_active(ctx, d, false);
        if !matches!(self.pending, Pending::AttackTreaty(..) | Pending::WeakAttack(..)) {
            ctx.game_mut().weak_attack_ok = false;
            ctx.game_mut().treaty_break_ok = None;
        }
        t
    }

    fn treaty_question(&mut self, ctx: &mut UiContext, id: UnitId, d: Dir8, o: crate::game::Slot) -> Transition {
        let lines = vec!["We have signed a".into(), "peace treaty with".into(), format!("the {}!", ctx.game().player(o).plural())];
        self.pending = Pending::AttackTreaty(id, d, o);
        Transition::Push(Box::new(MenuDialog::new(lines, vec!["Cancel action.".into(), "Break treaty.".into()]).with_pos(80, 80).with_speaker(crate::ui::widgets::SPEAKER_FOREIGN)))
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
                add(&mut items, "Retire", BarAction::Retire, false);
                add(&mut items, "QUIT to DOS", BarAction::Quit, false);
                if g.scoring_over {
                    add(&mut items, "View Replay", BarAction::Replay, false);
                }
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
                        add(&mut items, "Build Fortress \u{8f}f", key('f'), !construction);
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
                    // a transport or Carrier with a stack successor (+0xa, 0x1c5bb)
                    if (kind.role == rules::AiRole::Transport || u.kind == rules::unit_ids::CARRIER) && u.next.is_some() {
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
        let menu = MenuDialog::new(vec![], items).with_pos(x, 8).with_disabled(disabled);
        self.last_pulldown = Some(menu.clone());
        Transition::Push(Box::new(menu))
    }

    fn bar_menu_choice(&mut self, ctx: &mut UiContext, _menu: usize, choice: usize) -> Transition {
        match self.bar_actions.get(choice).copied().unwrap_or(BarAction::Nothing) {
            BarAction::Rate(lux) => Transition::Push(crate::ui::reports::rate_dialog(ctx, lux)),
            BarAction::FindCity => Transition::Push(crate::ui::reports::find_city_dialog(ctx)),
            BarAction::Options => {
                self.menu_behind = self.last_pulldown.take().map(|m| (m, choice));
                Transition::Push(crate::ui::reports::options_dialog(ctx))
            }
            BarAction::Save => Transition::Push(crate::ui::reports::save_dialog(ctx)),
            BarAction::Revolution => {
                self.pending = Pending::RevolutionConfirm;
                Transition::Push(Box::new(YesNo::new(vec!["Are you sure you want a REVOLUTION?".into()], "_No thanks.", "_Yes, we need a new government.").with_pos(64, 80)))
            }
            action @ (BarAction::Quit | BarAction::Retire) => {
                self.pending = if action == BarAction::Retire { Pending::RetireConfirm } else { Pending::QuitConfirm };
                Transition::Push(Box::new(YesNo::new(vec!["Are you sure you".into(), "want to Quit?".into()], "Keep Playing", "Yes, Quit").with_pos(100, 80)))
            }
            BarAction::Key(key, shift) => self.key_order(ctx, key, crate::platform::Mods { shift, ctrl: false, alt: false }),
            BarAction::Advisor(n) => Transition::Push(crate::ui::reports::advisor(ctx, n)),
            BarAction::Replay => Transition::Push(Box::new(crate::ui::replay::Replay::new())),
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
            // (Tab is no wait: with a mouse CIV.EXE ignores it, 0x58f3 -> 0x72e0; without one
            // it switches the keyboard look cursor, which civrs does not have)
            Key::Char('w') => {
                if let Some(id) = active {
                    ctx.game_mut().view = Some((self.vx, self.vy));
                    ctx.game_mut().wait_unit(id);
                    let g = ctx.game_mut();
                    if g.active_unit == Some(id) || g.active_unit.is_none() {
                        g.active_unit = Some(id);
                    }
                    self.adopt_view(ctx);
                }
                Transition::None
            }
            // 'f' fortifies, or builds a fortress with Settlers (0x5137); 'F' is no key of
            // move_units (0x4c1d, 0x583b)
            Key::Char('f') if mods.shift => Transition::None,
            Key::Char('f') => self.unit_order(ctx, UnitOrder::Fortify),
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
                        return Transition::Push(Box::new(MenuDialog::new(lines, vec!["Cancel action.".into(), "Break treaty.".into()]).with_pos(80, 80).with_speaker(crate::ui::widgets::SPEAKER_FOREIGN)));
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
                // a unit in a city is rehomed and stays active with its moves (CIV.EXE 0x4f28
                // ends without a move); an aircraft elsewhere flies back to base (0x4f94)
                if ctx.game_mut().press_home(id).is_some() {
                    return self.run_goto(ctx, id);
                }
                Transition::None
            }
            Key::Char('u') => {
                // the loop goes on from the stack's first passenger (0x4e64); a ship without
                // passengers stays active (0x4e27)
                if let Some(next) = active.and_then(|id| ctx.game_mut().unload(id)) {
                    self.activate_from(ctx, next);
                }
                Transition::None
            }
            Key::Char('d') if mods.shift => {
                // 'D' deletes the unit at once, no question asked (0x4c35 -> 0x4c5a)
                let Some(id) = active else { return Transition::None };
                ctx.game_mut().disband(id);
                self.activate_next(ctx);
                Transition::None
            }
            // 'g' (0x4e83) starts the goto choice: a click picks the square; in the keyboard-only
            // mode the look cursor does, switched on at the unit if it is off (0x4e99)
            Key::Char('g') => {
                if let Some(id) = active {
                    if ctx.keyboard_only && self.look.is_none() {
                        self.look = Some(ctx.game().unit(id).pos());
                    }
                    self.mode = Mode::Goto;
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

    /// The keys of `move_units`' second switch (CIV.EXE 0x58c1) that are no unit orders and
    /// that the map view does not handle elsewhere. None leaves the key to the caller.
    fn command_key(&mut self, ctx: &mut UiContext, key: Key, mods: crate::platform::Mods) -> Option<Transition> {
        let plain = !mods.alt && !mods.ctrl;
        // Shift+keypad (the keypad's digits, 0x4737..0x5133) scrolls the view 4 squares
        // (0x6ec4: draw_map_view(human, vx + 4dx, vy + 4dy), which wraps x and clamps y)
        if mods.shift && plain {
            if let Some(d) = Self::direction(key) {
                let (dx, dy) = d.delta();
                self.vx = Map::wrap_x(self.vx + 4 * dx);
                self.vy = (self.vy + 4 * dy).clamp(0, MAP_H as i32 - VIEW_H);
                return Some(Transition::None);
            }
        }
        let t = match key {
            // '+' / '=' and '-' / '_': the tax and luxuries menus (0x6fd9, 0x70bf)
            Key::Plus if plain => Transition::Push(crate::ui::reports::rate_dialog(ctx, false)),
            Key::Minus if plain => Transition::Push(crate::ui::reports::rate_dialog(ctx, true)),
            // '?' / '/': FindCity (o 0x1ebd0)
            Key::Char('/') if plain => Transition::Push(crate::ui::reports::find_city_dialog(ctx)),
            // 'S' saves, except on turn 0 (0x6f3a)
            Key::Char('s') if mods.shift && plain => {
                if ctx.game().turn == 0 {
                    Transition::None
                } else {
                    Transition::Push(crate::ui::reports::save_dialog(ctx))
                }
            }
            // 't': the map without units until a key or click (0x71a7)
            Key::Char('t') if plain && !mods.shift => {
                self.terrain_only = true;
                Transition::None
            }
            // Alt-V: "Sounds ON" / "Sounds OFF" (0x722f flips option bit 0x10)
            Key::Char('v') if mods.alt => {
                let o = &mut ctx.game_mut().options;
                o.sound = !o.sound;
                let line = format!("Sounds {}", if o.sound { "ON" } else { "OFF" });
                Transition::Push(Box::new(MenuDialog::new(vec![line], Vec::new()).with_pos(100, 80)))
            }
            // Alt-R: re-draw the leaders' traits, "Leaders randomized." (0x71d9)
            Key::Char('r') if mods.alt => {
                ctx.game_mut().randomize_leaders();
                Transition::Push(Box::new(MenuDialog::new(vec!["Leaders randomized.".to_string()], Vec::new()).with_pos(100, 80)))
            }
            _ => return None,
        };
        Some(t)
    }

    /// The direction of a cursor or keypad key.
    fn direction(key: Key) -> Option<Dir8> {
        match key {
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
        }
    }

    /// `tile_on_screen` (0x7d35): (x, y) is in the 15x12 map window.
    fn on_view(&self, x: i32, y: i32) -> bool {
        Map::wrap_x(x - self.vx) < VIEW_W && (0..VIEW_H).contains(&(y - self.vy))
    }

    /// The look cursor of the keyboard-only mode (ds:0x1a3c = 0; `move_units` [bp-0x4], square
    /// [bp-0xe]/[bp-0x12]). Tab (0x72e0) switches it: on at the active unit when that is in
    /// the map window, else at the view centre (vx + 7, vy + 6) (0x7326..0x7372); Esc (0x72d3
    /// -> 0x4b40) switches it off. While it is on, the cursor and keypad keys move it instead
    /// of the unit (0x4f99: x wraps, y stays in 0..49; off the window the view moves to
    /// (x - 8, y - 7)), and Enter clicks its square (0x4605 -> 0x4909). `blink_wait` (0x8a43)
    /// outlines it in white.
    fn look_key(&mut self, ctx: &mut UiContext, key: Key, mods: crate::platform::Mods) -> Option<Transition> {
        if mods.alt || mods.ctrl || mods.shift {
            return None;
        }
        match (key, self.look) {
            (Key::Tab, Some(_)) | (Key::Esc, Some(_)) => self.look = None,
            (Key::Esc, None) => {}
            (Key::Tab, None) => {
                let g = ctx.game();
                let at = g.active_unit.map(|id| g.unit(id).pos()).filter(|&(x, y)| self.on_view(x, y));
                self.look = Some(at.unwrap_or((Map::wrap_x(self.vx + 7), self.vy + 6)));
            }
            (Key::Enter, Some((x, y))) => return Some(self.left_click_tile(ctx, x, y)),
            (k, Some((x, y))) => {
                let (dx, dy) = Self::direction(k)?.delta();
                let (nx, ny) = (Map::wrap_x(x + dx), (y + dy).clamp(0, MAP_H as i32 - 1));
                self.look = Some((nx, ny));
                if !self.on_view(nx, ny) {
                    self.vx = Map::wrap_x(nx - 8);
                    self.vy = (ny - 7).clamp(0, MAP_H as i32 - VIEW_H);
                }
            }
            _ => return None,
        }
        Some(Transition::None)
    }

    /// A left click on map square (tx, ty) (0x4909), or Enter on the look cursor (0x4605).
    fn left_click_tile(&mut self, ctx: &mut UiContext, tx: i32, ty: i32) -> Transition {
        let g = ctx.game();
        let me = g.human_slot;
        if self.mode == Mode::Goto {
            // 0x4913: the target must be explored and suit the unit: land
            // units no ocean, ships ocean or a city, aircraft anything
            self.mode = Mode::Normal;
            let Some(id) = g.active_unit else { return Transition::None };
            let terrain = g.map.tile(tx, ty).terrain;
            if g.explored(me, tx, ty) && goto_target_ok(g.unit(id).info().domain, terrain == rules::Terrain::Ocean, g.city_at(tx, ty).is_some()) {
                return self.confirm_goto(ctx, (tx, ty));
            }
            return Transition::None;
        }
        if let Some(c) = g.city_at(tx, ty) {
            if g.city(c).owner == me {
                self.pending = Pending::Event;
                return self.open_city(ctx, c);
            }
        }
        // 0x4ac5: one of our units there opens the stack box (0x9dc5), even
        // for a lone unit; elsewhere the view centres on the square
        // for the square's first unit (unit_at 0x4acb): its stack ring, its best defender
        // by best_defender_at (0x9ddd)
        if let Some(first) = g.unit_at(tx, ty).filter(|u| g.unit(*u).owner == me) {
            let own = g.stack_ring(first, 12);
            let default = g.best_defender_at(first);
            self.pending = Pending::StackPick;
            return Transition::Push(Box::new(StackPicker::new(own, default)));
        }
        self.center_on(tx, ty);
        Transition::None
    }

    /// A key while the goto's square is chosen: 'g' again ends the choice (0x4f21 flips
    /// [bp-0x36]), and in the keyboard-only mode switches the look cursor off (0x4ec0). Any
    /// other key does what it does outside the choice (None): the keypad moves the unit, or
    /// the look cursor, whose Enter clicks the square (0x4605 -> 0x4909). As a civrs extra Esc
    /// also ends the choice (in CIV.EXE it does nothing with a mouse, 0x72d3).
    fn goto_key(&mut self, ctx: &mut UiContext, key: Key, mods: crate::platform::Mods) -> Option<Transition> {
        let end = key == Key::Esc || key == Key::Char('g') && !(mods.alt || mods.ctrl || mods.shift);
        if !end {
            return None;
        }
        if ctx.keyboard_only {
            self.look = None;
        }
        self.mode = Mode::Normal;
        Some(Transition::None)
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

    /// Move a unit along its goto path as far as its moves allow; an aircraft returning to
    /// base flies on the same way, the unit loop pressing 'h' for it before every step
    /// (CIV.EXE 0x4528..0x4556, `Game::press_home`).
    /// `Game::run_auto_unit` for the active unit `id`, with the menus a step may open; then
    /// the unit keeps the focus when it has moves left without an order (0x6d7f -> 0x442c),
    /// or the loop goes on.
    fn run_goto(&mut self, ctx: &mut UiContext, id: UnitId) -> Transition {
        let g = ctx.game_mut();
        g.active_unit = Some(id);
        g.view = Some((self.vx, self.vy));
        let stop = g.run_auto_unit(id);
        if let Some((vx, vy)) = g.view {
            (self.vx, self.vy) = (vx, vy);
        }
        if let AutoStop::Menu(out, dir) = stop {
            match out {
                MoveOutcome::CaravanArrived(c) => return self.caravan_menu(ctx, id, c, dir),
                MoveOutcome::EnteredForeignCity(c) => return self.foreign_city_menu(ctx, id, c),
                MoveOutcome::BribeOffer(t) => return self.bribe_offer(ctx, id, t, dir),
                MoveOutcome::BreakTreaty(o) => return self.treaty_question(ctx, id, dir, o),
                _ => {}
            }
        }
        let g = ctx.game_mut();
        let keep = g.unit_opt(id).is_some_and(|u| u.owner == g.human_slot && u.order == UnitOrder::None && u.moves_left > 0);
        if keep {
            g.unit_waits(id);
        }
        // a Settlers whose job the loop's key press refused waits for orders after the key's
        // message (the texts of ERROR.TXT; civrs's own refusals stay quiet here)
        if let AutoStop::Refused(e) = stop {
            if keep && e.section().is_some() {
                let (x, y) = ctx.game().unit(id).pos();
                self.ensure_visible(x, y);
                self.last_centered = Some(id);
                self.mode = Mode::Normal;
                return self.error(ctx, &e);
            }
        }
        if let Some(s) = self.next_event_screen(ctx) {
            self.pending = Pending::Event;
            return Transition::Push(s);
        }
        if keep {
            let (x, y) = ctx.game().unit(id).pos();
            self.ensure_visible(x, y);
            self.last_centered = Some(id);
            self.mode = Mode::Normal;
        } else {
            self.activate_next(ctx);
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
                if self.terrain_only {
                    tilerender::draw_tile_without_units(fb, ctx, g, me, x, y, px, py);
                    continue;
                }
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
        // city names, as draw_map_view (0x1a7b8..0x1a824) draws the labels draw_tile queued
        // for the cities it showed: none in the bottom row (y >= 184), the name fitted to
        // 327 - x pixels, in 11 at (max(x - 8, 80), y + 16)
        fb.set_clip(None);
        for ty in 0..VIEW_H - 1 {
            for tx in 0..VIEW_W {
                let (x, y) = (Map::wrap_x(self.vx + tx), self.vy + ty);
                if !g.explored(me, x, y) {
                    continue;
                }
                if let Some(c) = g.city_at(x, y).filter(|c| g.city_size_shown(*c).is_some()) {
                    let (px, py) = (VIEW_X + tx * TILE, VIEW_Y + ty * TILE);
                    let name = crate::ui::reports::fit_text(ctx, &g.city(c).name, 327 - px);
                    // draw_text_shadow (0x18a6): black one row down, then the colour
                    let lx = (px - 8).max(VIEW_X);
                    draw_text(fb, &ctx.fonts, &TextStyle::new(0, theme::BLACK), lx, py + 17, &name);
                    draw_text(fb, &ctx.fonts, &TextStyle::new(0, theme::CYAN), lx, py + 16, &name);
                }
            }
        }
        fb.set_clip(Some(Rect::new(VIEW_X, VIEW_Y, VIEW_W * TILE, VIEW_H * TILE)));
        // the look cursor: draw_outline(x, y, 15, 15, col), a 16x16 frame, white while the
        // active unit blinks off (0x8a43), black while it shows (0x8aff)
        if let Some((x, y)) = self.look.filter(|&(x, y)| self.on_view(x, y)) {
            let (px, py) = (VIEW_X + Map::wrap_x(x - self.vx) * TILE, VIEW_Y + (y - self.vy) * TILE);
            fb.rect_outline(Rect::new(px, py, 16, 16), if self.blink { 0 } else { 15 });
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
    /// land in 2 (`unit_look` 0x81b4), the cities the human owns or has seen (+8 != 0, 0x1a97c,
    /// explored or not) as a dot in the
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
            if (0..50).contains(&my) && g.city_size_shown(c.id).is_some() {
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
        // draw_palace (0x3823) draws it while the civ has a city (ds:0x6b66), Palace or not
        if !g.cities_of(g.human_slot).is_empty() {
            crate::ui::palace::draw_small_palace(fb, ctx, &p.palace(), 8, 58);
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
                // blink_wait with unit 0x80 (0x8a31 / 0x8aed): in 7 while a unit would be
                // hidden, then in 0
                if !g.options.end_turn_confirm {
                    let flash = TextStyle::new(0, if self.blink { theme::TEXT } else { theme::GREY });
                    draw_text(fb, &ctx.fonts, &flash, 4, 124, "End of Turn");
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
                        // draw_unit_panel (0x788d): lines 8 px apart from y 99
                        draw_text(fb, &ctx.fonts, &st, 4, y, p.adjective());
                        y += 8;
                        draw_text(fb, &ctx.fonts, &st, 4, y, u.info().name);
                        y += 8;
                        if u.veteran {
                            draw_text(fb, &ctx.fonts, &st, 8, y, "Veteran");
                            y += 8;
                        }
                        // "Moves: n[.thirds]", for aircraft "(n + fuel left * rate)" after it
                        let m = u.moves_left as i32;
                        let mut moves = format!("Moves: {}", (m / 3).clamp(0, 99));
                        if m % 3 != 0 {
                            moves += &format!(".{}", m % 3);
                        }
                        let info = u.info();
                        if info.range != 0 {
                            let left = info.range.saturating_sub(u.fuel) as i32;
                            moves += &format!("({})", (m / 3).clamp(0, 99) + left * info.moves as i32);
                        }
                        draw_text(fb, &ctx.fonts, &st, 4, y, &moves);
                        y += 8;
                        let home = u.home.and_then(|h| g.city_opt(h)).map(|c| c.name.clone()).unwrap_or_else(|| "NONE".into());
                        // strcat_city_name ("NONE" for no home) in the panel's font (o 0x7ad9)
                        draw_text(fb, &ctx.fonts, &st, 4, y, &home);
                        y += 8;
                        draw_text(fb, &ctx.fonts, &st, 4, y, &format!("({})", g.map.tile(u.x, u.y).terrain.info().name));
                        y += 8;
                        let t = g.map.tile(u.x, u.y);
                        let road = if t.has(tile_flags::RAILROAD) { Some("(RailRoad)") } else { t.has(tile_flags::ROAD).then_some("(Road)") };
                        let farm = if t.has(tile_flags::IRRIGATION) { Some("(Irrigation)") } else { t.has(tile_flags::MINE).then_some("(Mining)") };
                        for label in [road, farm, t.has(tile_flags::POLLUTION).then_some("(Pollution)")].into_iter().flatten() {
                            draw_text(fb, &ctx.fonts, &st, 4, y, label);
                            y += 8;
                        }
                        // the other units: a city's two stashed garrison units first, then the
                        // stack from this unit on, 16 px apart, four to a row, while y < 184;
                        // "+" at (74, 192) for the rest
                        y += 4;
                        let mut x = 8;
                        if let Some(c) = g.city_at(u.x, u.y) {
                            for (kind, _) in g.city(c).garrison_units() {
                                tilerender::draw_unit_kind(fb, ctx, kind, u.owner, x, y);
                                x += 16;
                            }
                        }
                        // (the ring links +0xa from the unit's successor round to it, 0x7cb9)
                        for o in g.stack_ring(id, 128).iter().skip(1) {
                            if y >= 184 {
                                draw_text(fb, &ctx.fonts, &st, 74, 192, "+");
                                break;
                            }
                            tilerender::draw_unit(fb, ctx, g, *o, x, y, false);
                            x += 16;
                            if x > 64 {
                                x = 8;
                                y += 16;
                            }
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
    fn on_exit(&mut self, ctx: &mut UiContext) {
        ctx.goto_pointer = false;
    }

    fn on_enter(&mut self, ctx: &mut UiContext) {
        let g = ctx.game_mut();
        if g.active_unit.is_none() {
            g.active_unit = g.next_active_unit();
        }
        if let Some((vx, vy)) = g.view {
            // the game's view, e.g. a loaded original game's (`sve::import`)
            (self.vx, self.vy) = (vx, vy);
        } else if let Some(id) = g.active_unit {
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
        // blink_wait (0x8932): the square without the active unit for 10 ticks, then with it
        // for 10 (0x8a1b / 0x8ad4 delay 10), as long as no key is waiting
        self.blink = (ctx.frame.saturating_sub(self.blink_from) / 10) % 2 == 1;
        // the goto choice with a mouse shows the GO pointer (`app::draw_cursor`)
        ctx.goto_pointer = self.mode == Mode::Goto && !ctx.keyboard_only;
        if let Some((n, vx, vy)) = self.recentre.take() {
            if n > 1 {
                self.recentre = Some((n - 1, vx, vy));
            } else {
                (self.vx, self.vy) = (vx, vy);
            }
            return Transition::None;
        }
        if let Some((x, y)) = ctx.center_request.take() {
            self.center_on(x, y);
        }
        // turn 0 on Chieftain: the screen-layout tutorial first (game_turn 0x2470 -> overlay 2
        // +0x17d9), then the FIRSTMOVE note. The leader's anthem fades after it (tune 1 at
        // 0x24bf, on every level); on Chieftain the tutorial fades it as it closes.
        if self.first_turn_hint && !self.tutorial_shown {
            self.tutorial_shown = true;
            let g = ctx.game();
            if g.turn == 0 && g.setup.difficulty == crate::rules::Difficulty::Chieftain {
                return Transition::Push(Box::new(LayoutTutorial::new(ctx, (Map::wrap_x(self.vx + 7), self.vy + 6))));
            }
            ctx.audio.fade_out();
        }
        // then the civ loop of turn 0 up to the human (game_turn 0x24d7): the civilizations
        // before it move first
        if self.first_turn_hint && self.runner.is_none() {
            if let Some(r) = ctx.game_mut().take_opening() {
                ctx.game_mut().active_unit = None;
                self.runner = Some(r);
                self.mode = Mode::Processing;
                self.pause = 0;
            }
        }
        if self.first_turn_hint && self.mode != Mode::Processing {
            self.first_turn_hint = false;
            // the first unit of turn 0 gets FIRSTMOVE (0x456b), and no Settlers advice follows
            // that turn
            self.advice_turn = Some(ctx.game().turn);
            if let Some(s) = ctx.game_mut().hint("FIRSTMOVE").and_then(|e| dialogs::screen_for_event(ctx, &e)) {
                self.pending = Pending::Event;
                return Transition::Push(s);
            }
        }
        if self.mode == Mode::Processing {
            return self.process(ctx);
        }
        if matches!(self.pending, Pending::None) && self.runner.is_none() {
            // the sidebar is drawn with every map view (draw_map_view 0x1a6a5) and after the
            // city screen, the rates and a caravan (0x4ab8, 0x70b7, 0x14894): its population
            // milestone (0x370a)
            ctx.game_mut().population_milestone();
            // what the loop's looks brought (an audience, 0x41e3 -> civs_meet), then the unit
            // it came to moves if it moves by itself (0x44b3)
            if !ctx.game().events.is_empty() {
                return self.show_events(ctx);
            }
            if self.mode == Mode::Normal {
                if let Some(id) = ctx.game().active_unit.filter(|id| ctx.game().is_auto_unit(*id)) {
                    return self.run_goto(ctx, id);
                }
            }
        }
        if let Some(s) = self.settler_advice(ctx) {
            self.pending = Pending::Event;
            return Transition::Push(s);
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
        self.menu_behind = None;
        match pending {
            Pending::Decision => {
                if let (Some(d), Some(runner)) = (self.current_decision.take(), self.runner.as_mut()) {
                    let reply = dialogs::reply_for(&d, &result);
                    let done = Self::done_playing(&d, &reply);
                    runner.resolve(ctx.game.as_mut().expect("game"), reply);
                    if let Some(t) = done {
                        return t;
                    }
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
                    let done = Self::done_playing(&d, &reply);
                    ctx.game_mut().apply_decision(&d, reply);
                    if let Some(t) = done {
                        return t;
                    }
                }
                // events produced while the dialog was up (audiences, unit actions)
                if let Some(s) = self.next_event_screen(ctx) {
                    self.pending = Pending::Event;
                    return Transition::Push(s);
                }
                // the loop goes on once the active unit is done (a unit that moves by itself
                // goes on in `update`)
                if self.mode != Mode::Processing && !self.active_takes_turn(ctx) {
                    self.activate_next(ctx);
                }
                Transition::None
            }
            Pending::BarMenu(i) => match result {
                DialogResult::Choice(c) => self.bar_menu_choice(ctx, i, c),
                _ => Transition::None,
            },
            Pending::StackPick => {
                // 0x4b0f: the box's unit becomes the active one unless it still sentries or
                // is fortified
                if let DialogResult::Choice(i) = result {
                    let id = i as UnitId;
                    let g = ctx.game_mut();
                    if g.unit_opt(id).is_some() && !matches!(g.unit(id).order, UnitOrder::Sentry | UnitOrder::Fortified) {
                        g.pick_unit(id);
                        g.active_unit = Some(id);
                        self.mode = Mode::Normal;
                    }
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
                                    ctx.game_mut().set_city_name(c, name.trim().to_string());
                                }
                                // FIRSTPRODUCT comes before the founding animation
                                let note = ctx.game_mut().take_events().into_iter().find_map(|e| match e {
                                    crate::game::GameEvent::Hint(_) => dialogs::screen_for_event(ctx, &e),
                                    _ => None,
                                });
                                self.activate_next(ctx);
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
            Pending::RetireConfirm => {
                if result != DialogResult::No {
                    return Transition::None;
                }
                let first = ctx.game_mut().retire();
                self.pending = Pending::Retired;
                Transition::Push(if first {
                    crate::ui::endgame::end_screen(ctx, crate::game::EndReason::Retired)
                } else {
                    crate::ui::endgame::end_again_screen(ctx, crate::game::EndReason::Retired)
                })
            }
            Pending::Retired => Transition::Quit,
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
                    Transition::Push(crate::ui::unit_actions::senate_overrules_box())
                }
                DialogResult::Choice(1) if ctx.game().active_unit == Some(id) => self.unit_order(ctx, UnitOrder::Pillage),
                _ => Transition::None,
            },
            Pending::WeakAttack(id, d) => match result {
                DialogResult::Choice(1) if ctx.game().unit_opt(id).is_some() && ctx.game().active_unit == Some(id) => {
                    ctx.game_mut().weak_attack_ok = true;
                    self.confirmed_move(ctx, d)
                }
                _ => {
                    ctx.game_mut().weak_attack_ok = false;
                    ctx.game_mut().treaty_break_ok = None;
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
            Pending::AttackTreaty(id, d, o) => match result {
                DialogResult::Choice(1) if ctx.game().senate_overrules(ctx.game().human_slot) => {
                    self.pending = Pending::Event;
                    Transition::Push(crate::ui::unit_actions::senate_overrules_box())
                }
                DialogResult::Choice(1) => {
                    ctx.game_mut().treaty_break_ok = Some(o);
                    if ctx.game().unit_opt(id).is_some() && ctx.game().active_unit == Some(id) {
                        return self.confirmed_move(ctx, d);
                    }
                    ctx.game_mut().treaty_break_ok = None;
                    ctx.game_mut().weak_attack_ok = false;
                    Transition::None
                }
                _ => {
                    ctx.game_mut().weak_attack_ok = false;
                    if ctx.game().unit_opt(id).is_some() {
                        let u = ctx.game_mut().unit_mut(id);
                        u.goto = None;
                        if u.order == UnitOrder::Goto {
                            u.order = UnitOrder::None;
                        }
                    }
                    // the territory rule's question came after the step's cost (0x657d): a
                    // unit left without moves is done
                    if !self.active_takes_turn(ctx) {
                        self.activate_next(ctx);
                    }
                    Transition::None
                }
            },
            Pending::None => Transition::None,
        }
    }

    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if self.mode == Mode::Processing || self.recentre.is_some() {
            return Transition::None;
        }
        if matches!(ev, Event::Key { down: true, key, .. } if *key != Key::Unknown) || matches!(ev, Event::MouseButton { down: true, .. }) {
            self.blink_from = ctx.frame;
        }
        // 't' waits for any key or click (0x13d7:0x91e) and then draws the units again
        if self.terrain_only {
            if matches!(ev, Event::Key { down: true, key, .. } if *key != Key::Unknown) || matches!(ev, Event::MouseButton { down: true, .. }) {
                self.terrain_only = false;
            }
            return Transition::None;
        }
        match ev {
            Event::Key { key, mods, down: true, .. } => {
                if self.mode == Mode::Goto {
                    if let Some(t) = self.goto_key(ctx, *key, *mods) {
                        return t;
                    }
                }
                if ctx.keyboard_only {
                    if let Some(t) = self.look_key(ctx, *key, *mods) {
                        return t;
                    }
                }
                if let Some(t) = self.command_key(ctx, *key, *mods) {
                    return t;
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
                    // Enter and Space end the turn only when no unit is left to move (0x6f25:
                    // unit index 0x80); with an active unit Enter does nothing
                    Key::Enter | Key::Space if ctx.game().active_unit.is_none() || self.mode == Mode::EndOfTurn => {
                        self.start_end_turn(ctx);
                        Transition::None
                    }
                    Key::Enter => Transition::None,
                    Key::Char('g') if mods.alt || mods.ctrl => self.bar_menu(ctx, 0),
                    Key::Char('o') if mods.alt || mods.ctrl => self.bar_menu(ctx, 1),
                    Key::Char('a') if mods.alt || mods.ctrl => self.bar_menu(ctx, 2),
                    Key::Char('w') if mods.alt || mods.ctrl => self.bar_menu(ctx, 3),
                    Key::Char('c') if mods.alt || mods.ctrl => self.bar_menu(ctx, 4),
                    // Alt-H: both GENERAL PRINCIPLES notes (key 0x2300, 0x58c4 -> 0x7274)
                    Key::Char('h') if mods.alt => {
                        let notes: Vec<_> = ["HELP1", "HELP2"].iter().filter_map(|s| ctx.game_mut().hint(s)).collect();
                        self.queued.extend(notes);
                        self.show_events(ctx)
                    }
                    // F1..F10 only (0x3b00..0x4400); Demographics and SpaceShips have no key
                    Key::F(n) if (1..=10).contains(n) => Transition::Push(crate::ui::reports::advisor(ctx, *n)),
                    Key::Char('s') if mods.ctrl => {
                        let path = crate::save::slot_path(&ctx.save_dir, 0);
                        let r = crate::save::save_game(&path, ctx.game());
                        self.pending = Pending::Event;
                        Transition::Push(Box::new(MessageBox::text(&match r {
                            Ok(()) => "Game saved.".to_string(),
                            Err(e) => format!("Save failed: {e}"),
                        })))
                    }
                    // Alt-Q (0x1000 -> 0x728c); Ctrl-Q is a civrs extra, no key of move_units
                    Key::Char('q') if mods.ctrl || mods.alt => {
                        self.pending = Pending::QuitConfirm;
                        Transition::Push(Box::new(YesNo::new(vec!["Are you sure you".into(), "want to Quit?".into()], "Keep Playing", "Yes, Quit").with_pos(100, 80)))
                    }
                    // Esc (0x72d3) only ends the look cursor of the keyboard-only mode; with a
                    // mouse it does nothing
                    Key::Esc => Transition::None,
                    // other Alt/Ctrl combinations are no unit orders (Alt-R 0x1300 is not 'r')
                    _ if mods.alt || mods.ctrl => Transition::None,
                    // the unit switch (0x4c1d) knows the shifted letters D and P only
                    Key::Char(c) if mods.shift && !matches!(c, 'd' | 'p' | 'f') => Transition::None,
                    k => self.key_order(ctx, *k, *mods),
                }
            }
            Event::MouseButton { button, x, y, down: true } => {
                let (mx, my) = (*x, *y);
                match side_hit(mx, my) {
                    SideHit::Menu(i) => return self.bar_menu(ctx, i),
                    SideHit::Minimap => {
                        // o 0x46b2: the clicked minimap square, in the minimap's scrolled frame
                        let (mx0, my0) = self.minimap_origin(ctx);
                        self.center_on(Map::wrap_x(mx + mx0), (my - 8 + my0).clamp(0, 49));
                        return Transition::None;
                    }
                    SideHit::Palace => return Transition::Push(crate::ui::palace::palace_screen(ctx, Vec::new())),
                    SideHit::UnitBox => {
                        if self.mode == Mode::EndOfTurn {
                            self.start_end_turn(ctx);
                        }
                        return Transition::None;
                    }
                    SideHit::Map => {}
                }
                let Some((tx, ty)) = self.tile_at_pixel(mx, my) else { return Transition::None };
                let g = ctx.game();
                let me = g.human_slot;
                match button {
                    MouseButton::Right => {
                        // CIV.EXE 0x479c: a square next to the active unit (around the map's
                        // seam too) becomes its goto, which ends a GoTo choice; any other
                        // explored square shows the Civilopedia page of its terrain
                        if let Some(id) = g.active_unit {
                            let (ux, uy) = g.unit(id).pos();
                            if let Some(d) = Dir8::from_delta(Map::delta_x(ux, tx), ty - uy) {
                                self.mode = Mode::Normal;
                                return self.try_move(ctx, d);
                            }
                        }
                        if g.explored(me, tx, ty) {
                            let t = g.map.tile(tx, ty).terrain;
                            return Transition::Push(crate::ui::civilopedia::entry_screen(crate::ui::pedia_data::Entry::Terrain(t), 0));
                        }
                        Transition::None
                    }
                    MouseButton::Left => self.left_click_tile(ctx, tx, ty),
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
        if let Some((m, sel)) = self.menu_behind.as_mut() {
            m.paint_with(fb, ctx, Some(*sel));
        }
    }

    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        water_cycles(ctx.game_palette(), ctx.frame)
    }

    fn draws_parent(&self) -> bool {
        false
    }
}

/// The map's water animation: setup_palette_cycling (0x1da03) defines the colour rings
/// 0x60..0x67, 0x68..0x6f and 0x70..0x7f, each turned by cycle_step (0:0x44a) every 15 ticks;
/// after k steps colour `first + i` shows the ring's colour `i - k`.
pub fn water_cycles(mut pal: crate::assets::Palette, tick: u64) -> crate::assets::Palette {
    let k = (tick / 15) as usize;
    for (first, last) in [(0x60usize, 0x67usize), (0x68, 0x6f), (0x70, 0x7f)] {
        let n = last - first + 1;
        let ring: Vec<[u8; 3]> = pal.0[first..=last].to_vec();
        for i in 0..n {
            pal.0[first + i] = ring[(i + n - k % n) % n];
        }
    }
    pal
}

impl MapView {
    /// "No, I'm done." to *FAME (ds:0xdc48 = 1): ADSCREEN and the end of the program (0x2c16).
    fn done_playing(d: &Decision, reply: &crate::game::DecisionReply) -> Option<Transition> {
        (matches!(d, Decision::KeepPlaying { .. }) && *reply == crate::game::DecisionReply::No).then(|| Transition::Push(Box::new(crate::ui::endgame::AdScreen)))
    }


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

    /// The advice of `move_units` when a unit becomes active (CIV.EXE 0x456b..0x45c9): on
    /// turn 0 FIRSTMOVE, later the Settlers advice for the first Settlers of the turn; either
    /// uses up the turn's latch, whether or not a note comes.
    fn settler_advice(&mut self, ctx: &mut UiContext) -> Option<Box<dyn Screen>> {
        if self.mode != Mode::Normal || !matches!(self.pending, Pending::None) || self.runner.is_some() {
            return None;
        }
        let id = ctx.game().active_unit?;
        let turn = ctx.game().turn;
        // only a unit that waits for orders (0x455e); one on goto moves first
        if self.advice_turn == Some(turn) || ctx.game().unit_opt(id)?.order != UnitOrder::None {
            return None;
        }
        let e = if turn == 0 {
            ctx.game_mut().hint("FIRSTMOVE")
        } else if ctx.game().unit_opt(id)?.kind == crate::rules::unit_ids::SETTLERS {
            ctx.game_mut().settler_hint(id)
        } else {
            return None;
        };
        self.advice_turn = Some(turn);
        dialogs::screen_for_event(ctx, &e?)
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

/// The AutoSave option saves every 50 turns, into slots 4..8 in turn (game_turn 0x2769..0x27b6:
/// turn % 50 == 0, option bit 1 of ds:0x19c0). CIV.EXE rotates through 4..9
/// (`save_game((turn / 50 - 1) % 6 + 4)`); civrs leaves slot 9 to its own save of every turn
/// (`after_turn`). CIV.EXE saves as the new turn begins, before any civ's turn; civrs saves once
/// the turn processing is over, where its saves resume.
fn autosave_slot(g: &crate::game::Game) -> Option<usize> {
    (g.options.autosave && g.turn > 0 && g.turn.is_multiple_of(50)).then(|| (g.turn as usize / 50 - 1) % 5 + 4)
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
    /// game_turn plays tune 1 (fade) once the tutorial returns (0x24bf).
    fn on_exit(&mut self, ctx: &mut UiContext) {
        ctx.audio.fade_out();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Game, GameSetup};
    use crate::platform::Mods;

    fn key(k: Key, mods: Mods) -> Event {
        Event::Key { key: k, mods, down: true, repeat: false }
    }

    const SHIFT: Mods = Mods { shift: true, ctrl: false, alt: false };
    const ALT: Mods = Mods { shift: false, ctrl: false, alt: true };
    const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
    const CTRL: Mods = Mods { shift: false, ctrl: true, alt: false };

    fn press(button: MouseButton, x: i32, y: i32) -> Event {
        Event::MouseButton { button, x, y, down: true }
    }

    #[test]
    fn autosave_every_fifty_turns_into_slots_4_to_8() {
        let mut g = Game::new(GameSetup { seed: 3, ..Default::default() }, None);
        g.options.autosave = true;
        let slot = |g: &mut Game, t: u32| {
            g.turn = t;
            autosave_slot(g)
        };
        assert_eq!(slot(&mut g, 49), None);
        assert_eq!(slot(&mut g, 50), Some(4));
        assert_eq!(slot(&mut g, 100), Some(5));
        assert_eq!(slot(&mut g, 250), Some(8));
        assert_eq!(slot(&mut g, 300), Some(4));
        g.options.autosave = false;
        assert_eq!(slot(&mut g, 50), None);
    }

    #[test]
    fn mouse_regions_follow_move_units() {
        // the menu is the column x / 60, over the whole bar (0x467f)
        assert_eq!(side_hit(0, 0), SideHit::Menu(0));
        assert_eq!(side_hit(59, 7), SideHit::Menu(0));
        assert_eq!(side_hit(60, 3), SideHit::Menu(1));
        assert_eq!(side_hit(319, 7), SideHit::Menu(4));
        assert_eq!(side_hit(10, 8), SideHit::Minimap);
        assert_eq!(side_hit(79, 57), SideHit::Minimap);
        assert_eq!(side_hit(10, 58), SideHit::Palace);
        assert_eq!(side_hit(10, 71), SideHit::Palace);
        assert_eq!(side_hit(10, 72), SideHit::UnitBox);
        assert_eq!(side_hit(10, 199), SideHit::UnitBox);
        assert_eq!(side_hit(80, 8), SideHit::Map);
    }

    #[test]
    fn goto_targets_suit_the_unit() {
        use rules::Domain::*;
        assert!(goto_target_ok(Land, false, false));
        assert!(!goto_target_ok(Land, true, false));
        assert!(goto_target_ok(Sea, true, false));
        assert!(!goto_target_ok(Sea, false, false));
        assert!(goto_target_ok(Sea, false, true));
        assert!(goto_target_ok(Air, true, false));
    }

    #[test]
    fn stack_box_rows() {
        let p = StackPicker::new(vec![4, 5, 6], 5);
        // three rows from y 72 + 5, 16 px each, x 101..219
        assert_eq!(p.top(), 72);
        assert_eq!(p.row_at(150, 77), Some(0));
        assert_eq!(p.row_at(150, 70), Some(0), "C division rounds a small negative to row 0");
        assert_eq!(p.row_at(150, 93), Some(1));
        assert_eq!(p.row_at(150, 124), Some(2));
        assert_eq!(p.row_at(150, 125), None);
        assert_eq!(p.row_at(100, 80), None);
        assert_eq!(p.row_at(220, 80), None);
    }

    #[test]
    fn clicking_the_unit_box_with_a_unit_does_nothing() {
        let Some((mut ctx, mut mv, id)) = view() else { return };
        let before = ctx.game().unit(id).clone();
        assert!(matches!(mv.handle(&press(MouseButton::Left, 20, 150), &mut ctx), Transition::None));
        assert_eq!(ctx.game().active_unit, Some(id));
        assert_eq!(ctx.game().unit(id).order, before.order);
        assert!(matches!(mv.handle(&press(MouseButton::Left, 20, 60), &mut ctx), Transition::Push(_)), "the palace strip opens the palace");
    }

    #[test]
    fn a_click_on_our_unit_opens_the_stack_box_and_a_row_wakes_it() {
        let Some((mut ctx, mut mv, id)) = view() else { return };
        let (ux, uy) = ctx.game().unit(id).pos();
        mv.center_on(ux, uy);
        ctx.game_mut().unit_mut(id).order = UnitOrder::Sentry;
        ctx.game_mut().active_unit = None;
        let px = VIEW_X + Map::wrap_x(ux - mv.vx) * TILE + 4;
        let py = VIEW_Y + (uy - mv.vy) * TILE + 4;
        let Transition::Push(mut boxed) = mv.handle(&press(MouseButton::Left, px, py), &mut ctx) else { panic!("no stack box") };
        let n = ctx.game().units_at(ux, uy).len().min(12) as i32;
        let row_y = 96 - 8 * n + 5;
        let t = boxed.handle(&press(MouseButton::Left, 150, row_y), &mut ctx);
        assert_eq!(ctx.game().unit(id).order, UnitOrder::None, "the row's unit is woken");
        if n == 1 {
            let Transition::Pop(r) = t else { panic!("a lone unit closes the box") };
            mv.resume(r, &mut ctx);
            assert_eq!(ctx.game().active_unit, Some(id));
        }
    }

    /// A map view over a new game whose first human unit is active.
    fn view() -> Option<(UiContext, MapView, UnitId)> {
        let dir = crate::assets::AssetDir::locate(None).ok()?;
        let mut ctx = UiContext::new(dir, std::env::temp_dir().join(format!("civrs-mapkeys-{}", std::process::id()))).ok()?;
        let mut g = Game::new(GameSetup { seed: 3, ..Default::default() }, None);
        let _ = g.take_events();
        let id = g.units_of(g.human_slot)[0];
        g.active_unit = Some(id);
        ctx.game = Some(g);
        let mut mv = MapView::new();
        mv.vx = 20;
        mv.vy = 20;
        Some((ctx, mv, id))
    }

    #[test]
    fn the_water_colours_turn_every_15_ticks() {
        let mut pal = crate::assets::Palette::default();
        for i in 0..256 {
            pal.0[i] = [i as u8, 0, 0];
        }
        assert_eq!(water_cycles(pal, 14).0, pal.0);
        let one = water_cycles(pal, 15);
        // colour first + i shows the ring's i - 1
        assert_eq!((one.0[0x60][0], one.0[0x61][0], one.0[0x67][0]), (0x67, 0x60, 0x66));
        assert_eq!((one.0[0x70][0], one.0[0x7f][0], one.0[0x5f][0], one.0[0x80][0]), (0x7f, 0x7e, 0x5f, 0x80));
        assert_eq!(water_cycles(pal, 8 * 15).0[0x60], [0x60, 0, 0]);
    }

    #[test]
    fn shifted_letters_are_no_unit_orders() {
        let Some((mut ctx, mut mv, id)) = view() else { return };
        // 'S' is Save (nothing on turn 0), not Sentry; Alt-R is not a road order but
        // "Leaders randomized." (0x71d9)
        assert!(matches!(mv.handle(&key(Key::Char('s'), SHIFT), &mut ctx), Transition::None));
        assert!(matches!(mv.handle(&key(Key::Char('r'), ALT), &mut ctx), Transition::Push(_)));
        // Tab does nothing with a mouse (0x72e0)
        mv.handle(&key(Key::Tab, NONE), &mut ctx);
        assert_eq!(ctx.game().active_unit, Some(id));
        assert_eq!(ctx.game().unit(id).order, UnitOrder::None);
        assert_eq!(ctx.game().unit(id).order, UnitOrder::None);
        ctx.game_mut().set_turn(5);
        assert!(matches!(mv.handle(&key(Key::Char('s'), SHIFT), &mut ctx), Transition::Push(_)));
        assert_eq!(ctx.game().unit(id).order, UnitOrder::None);
        // plain s still sentries
        mv.handle(&key(Key::Char('s'), NONE), &mut ctx);
        assert_eq!(ctx.game().unit(id).order, UnitOrder::Sentry);
    }

    #[test]
    fn keyboard_only_look_cursor() {
        let Some((mut ctx, mut mv, id)) = view() else { return };
        // with a mouse Esc does nothing (0x72d3)
        assert!(matches!(mv.handle(&key(Key::Esc, NONE), &mut ctx), Transition::None));
        assert_eq!(mv.look, None);
        ctx.keyboard_only = true;
        let (ux, uy) = ctx.game().unit(id).pos();
        mv.center_on(ux, uy);
        mv.handle(&key(Key::Tab, NONE), &mut ctx);
        assert_eq!(mv.look, Some((ux, uy)), "starts on the active unit");
        // the keypad moves the cursor, not the unit
        mv.handle(&key(Key::Right, NONE), &mut ctx);
        assert_eq!(mv.look, Some((Map::wrap_x(ux + 1), uy)));
        assert_eq!(ctx.game().unit(id).pos(), (ux, uy));
        mv.handle(&key(Key::Esc, NONE), &mut ctx);
        assert_eq!(mv.look, None);
        // with the unit off the window it starts at the view centre
        mv.vx = Map::wrap_x(ux + 40);
        mv.handle(&key(Key::Tab, NONE), &mut ctx);
        assert_eq!(mv.look, Some((Map::wrap_x(mv.vx + 7), mv.vy + 6)));
        mv.handle(&key(Key::Tab, NONE), &mut ctx);
        assert_eq!(mv.look, None);
    }

    #[test]
    fn goto_keys_with_a_mouse_and_with_the_keyboard_only() {
        let Some((mut ctx, mut mv, id)) = view() else { return };
        // Alt-Q asks to quit (0x1000 -> 0x728c), and so does Ctrl-Q as a civrs extra
        assert!(matches!(mv.handle(&key(Key::Char('q'), ALT), &mut ctx), Transition::Push(_)));
        mv.pending = Pending::None;
        assert!(matches!(mv.handle(&key(Key::Char('q'), CTRL), &mut ctx), Transition::Push(_)));
        mv.pending = Pending::None;
        // 'g' again ends the goto choice (0x4f21); Esc does too as a civrs extra (CIV.EXE
        // ignores it with a mouse, 0x72d3)
        mv.handle(&key(Key::Char('g'), NONE), &mut ctx);
        assert_eq!(mv.mode, Mode::Goto);
        mv.handle(&key(Key::Char('g'), NONE), &mut ctx);
        assert_eq!(mv.mode, Mode::Normal);
        mv.handle(&key(Key::Char('g'), NONE), &mut ctx);
        mv.handle(&key(Key::Esc, NONE), &mut ctx);
        assert_eq!(mv.mode, Mode::Normal);
        assert_eq!(mv.look, None);
        // keyboard only: 'g' puts the look cursor on the unit (0x4e99), the keypad moves it,
        // 'g' again switches both off (0x4ec0)
        ctx.keyboard_only = true;
        let (ux, uy) = ctx.game().unit(id).pos();
        mv.center_on(ux, uy);
        mv.handle(&key(Key::Char('g'), NONE), &mut ctx);
        assert_eq!((mv.mode, mv.look), (Mode::Goto, Some((ux, uy))));
        mv.handle(&key(Key::Right, NONE), &mut ctx);
        assert_eq!(mv.look, Some((Map::wrap_x(ux + 1), uy)));
        assert_eq!(ctx.game().unit(id).pos(), (ux, uy));
        mv.handle(&key(Key::Char('g'), NONE), &mut ctx);
        assert_eq!((mv.mode, mv.look), (Mode::Normal, None));
        // Enter clicks the cursor's square, which ends the choice (0x4605 -> 0x4909)
        mv.handle(&key(Key::Char('g'), NONE), &mut ctx);
        mv.handle(&key(Key::Enter, NONE), &mut ctx);
        assert_eq!(mv.mode, Mode::Normal);
    }

    #[test]
    fn command_keys_open_their_menus() {
        let Some((mut ctx, mut mv, _)) = view() else { return };
        for k in [Key::Plus, Key::Minus, Key::Char('/')] {
            assert!(matches!(mv.handle(&key(k, NONE), &mut ctx), Transition::Push(_)), "{k:?}");
        }
        // Enter with a unit to move does not end the turn (0x6f25)
        mv.handle(&key(Key::Enter, NONE), &mut ctx);
        assert!(mv.runner.is_none());
        assert!(ctx.game().active_unit.is_some());
    }

    #[test]
    fn shift_keypad_scrolls_four_squares_and_t_hides_units() {
        let Some((mut ctx, mut mv, id)) = view() else { return };
        let pos = ctx.game().unit(id).pos();
        mv.handle(&key(Key::Kp(9), SHIFT), &mut ctx);
        assert_eq!((mv.vx, mv.vy), (24, 16));
        assert_eq!(ctx.game().unit(id).pos(), pos, "the unit stays");
        mv.handle(&key(Key::Char('t'), NONE), &mut ctx);
        assert!(mv.terrain_only);
        // the next key only ends the view
        mv.handle(&key(Key::Char('s'), NONE), &mut ctx);
        assert!(!mv.terrain_only);
        assert_eq!(ctx.game().unit(id).order, UnitOrder::None);
    }
}
