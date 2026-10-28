//! Turn processing: the resumable `TurnRunner` state machine, per-city processing (food,
//! production, upkeep, pollution, disasters, disorder), science, unit hazards, government
//! changes, world events (global warming, spaceship flights, end of game), decisions and the
//! caravan / diplomat special actions.
//!
//! `TurnRunner` is a resumable state machine: the UI calls `step` repeatedly; when it returns
//! a `Decision`, the UI shows a dialog and calls `resolve` before stepping again. Decisions
//! raised while the engine processes a player are queued as `GameEvent::NeedsDecision` so they
//! appear in order with the surrounding events.

use super::events::{Decision, DecisionReply, Disaster, EndReason, GameEvent};
use super::map::{CityId, Map, Slot, UnitId};
use super::{Game, Production, RuleError};
use crate::ai::AiDriver;
use crate::rules::{self, advance_ids as adv, improvement_ids as imp, unit_ids, wonder_ids as won, AdvanceId, Govt, ImprovementId, Terrain, UnitTypeId};
use serde::{Deserialize, Serialize};

/// `palace_threshold(n)` (CIV.EXE 0x3a67): the score that brings palace level `n`,
/// `(sum of i * (7 - difficulty) for i in 0..=n) / 2 + 1`.
pub fn palace_threshold(n: i32, difficulty: i32) -> i32 {
    (0..=n).map(|i| i * (7 - difficulty)).sum::<i32>() / 2 + 1
}

#[derive(Clone, Debug, PartialEq)]
pub enum TurnStep {
    Event(GameEvent),
    Decision(Decision),
    /// An AI unit moved (for the "enemy moves" animation); `visible` to the human.
    AiMove { unit: UnitId, from: (i32, i32), to: (i32, i32), visible: bool },
    Combat { attacker_at: (i32, i32), defender_at: (i32, i32), won: bool, visible: bool, attacker: UnitTypeId, defender: UnitTypeId, defender_owner: Slot },
    Done,
}

/// Where a `TurnRunner` stands in CIV.EXE's turn loop (game_turn 0x2412). Each civilization
/// in slot order (barbarians first) has its cities processed (update_civ_tax_rates 0x250d), then
/// its politics (0x252b), then its units move (0x25af); after civ 7 come the barbarian raids,
/// global warming and the other world events (0x2620..0x26e7), then the turn counter and the
/// year advance (0x26ec). The human moves its units interactively, so a runner starts after
/// them, runs the civilizations after the human, the world and the new turn, and the
/// civilizations before the human, and stops after the human's cities and politics.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    /// The end of the human's units (hazards).
    HumanUnits,
    /// The civilizations after the human (`ai_index` = next slot).
    LaterCivs,
    World,
    NewTurn,
    /// The barbarians and the civilizations before the human (`ai_index` = next slot).
    EarlierCivs,
    /// The human's cities and politics, before its units move.
    HumanStart,
    Done,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnRunner {
    pub phase: Phase,
    pub ai_index: usize,
    pub pending: std::collections::VecDeque<TurnStepSer>,
    pub waiting: Option<Decision>,
    /// The opening of a new game ran first (`Game::opening`); go on with a whole turn.
    #[serde(default)]
    pub then_turn: bool,
}

/// Serialisable subset of TurnStep (events/decisions only).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum TurnStepSer {
    Event(GameEvent),
    Decision(Decision),
    AiMove { unit: UnitId, from: (i32, i32), to: (i32, i32), visible: bool },
    Combat { attacker_at: (i32, i32), defender_at: (i32, i32), won: bool, visible: bool, attacker: UnitTypeId, defender: UnitTypeId, defender_owner: Slot },
}

impl From<TurnStepSer> for TurnStep {
    fn from(s: TurnStepSer) -> TurnStep {
        match s {
            TurnStepSer::Event(e) => TurnStep::Event(e),
            TurnStepSer::Decision(d) => TurnStep::Decision(d),
            TurnStepSer::AiMove { unit, from, to, visible } => TurnStep::AiMove { unit, from, to, visible },
            TurnStepSer::Combat { attacker_at, defender_at, won, visible, attacker, defender, defender_owner } => TurnStep::Combat { attacker_at, defender_at, won, visible, attacker, defender, defender_owner },
        }
    }
}

impl From<TurnStep> for Option<TurnStepSer> {
    fn from(s: TurnStep) -> Option<TurnStepSer> {
        match s {
            TurnStep::Event(GameEvent::NeedsDecision(d)) | TurnStep::Decision(d) => Some(TurnStepSer::Decision(d)),
            TurnStep::Event(e) => Some(TurnStepSer::Event(e)),
            TurnStep::AiMove { unit, from, to, visible } => Some(TurnStepSer::AiMove { unit, from, to, visible }),
            TurnStep::Combat { attacker_at, defender_at, won, visible, attacker, defender, defender_owner } => Some(TurnStepSer::Combat { attacker_at, defender_at, won, visible, attacker, defender, defender_owner }),
            TurnStep::Done => None,
        }
    }
}

impl Default for TurnRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnRunner {
    pub fn new() -> TurnRunner {
        TurnRunner { phase: Phase::HumanUnits, ai_index: 0, pending: Default::default(), waiting: None, then_turn: false }
    }

    /// Turn 0 of a new game up to the human's first move: the civilizations before the human
    /// (game_turn runs its civ loop from civ 0 at turn 0 too), then the human's (empty) cities
    /// and its politics.
    pub fn opening() -> TurnRunner {
        TurnRunner { phase: Phase::EarlierCivs, ..TurnRunner::new() }
    }

    /// One computer civilization's turn in game_turn's civ loop: its cities and rates
    /// (0x250d), politics (0x252b), then `AiDriver::play_turn` (ai_civ_turn 0x2536,
    /// ai_reset_goals 0x2541, move_units 0x25af) with the units' work and hazards.
    fn civ_turn(&mut self, g: &mut Game, ai: &mut dyn AiDriver, slot: Slot) {
        if slot == 0 {
            // the barbarians' cities (0xa5e2 for civ 0), then their units
            g.reset_moves(0);
            for c in g.cities_of(0) {
                if g.city_opt(c).is_some() {
                    g.process_city(c);
                }
            }
            self.run_ai(g, ai, 0);
            g.process_unit_hazards(0);
            self.drain_events(g);
            return;
        }
        if slot == g.human_slot || !g.player(slot).in_game || !g.player(slot).alive {
            return;
        }
        g.process_player_cities(slot);
        self.drain_events(g);
        g.process_player_turn_start(slot);
        g.reset_moves(slot);
        g.progress_work(slot);
        self.drain_events(g);
        self.run_ai(g, ai, slot);
        g.process_unit_hazards(slot);
        self.drain_events(g);
    }

    fn drain_events(&mut self, g: &mut Game) {
        for e in g.take_events() {
            match e {
                GameEvent::NeedsDecision(d) => self.pending.push_back(TurnStepSer::Decision(d)),
                e => self.pending.push_back(TurnStepSer::Event(e)),
            }
        }
    }

    /// Run an AI driver for a slot, collecting what it reports.
    fn run_ai(&mut self, g: &mut Game, ai: &mut dyn AiDriver, slot: Slot) {
        let mut sink: Vec<TurnStepSer> = Vec::new();
        ai.play_turn(g, slot, &mut |s: TurnStep| {
            if let Some(ser) = Option::<TurnStepSer>::from(s) {
                sink.push(ser);
            }
        });
        self.pending.extend(sink);
        self.drain_events(g);
    }

    /// Advance processing until something must be shown or decided, or the turn is done.
    pub fn step(&mut self, g: &mut Game, ai: &mut dyn AiDriver) -> TurnStep {
        if let Some(d) = &self.waiting {
            return TurnStep::Decision(d.clone());
        }
        loop {
            if let Some(s) = self.pending.pop_front() {
                if let TurnStepSer::Decision(d) = &s {
                    // decisions for AI players never reach the UI
                    if !g.is_human(decision_slot(g, d)) {
                        continue;
                    }
                    self.waiting = Some(d.clone());
                }
                return s.into();
            }
            match self.phase {
                Phase::HumanUnits => {
                    g.process_unit_hazards(g.human_slot);
                    self.drain_events(g);
                    self.phase = Phase::LaterCivs;
                    self.ai_index = g.human_slot as usize + 1;
                }
                Phase::LaterCivs => {
                    if self.ai_index >= 8 {
                        self.phase = Phase::World;
                        continue;
                    }
                    let slot = self.ai_index as Slot;
                    self.ai_index += 1;
                    self.civ_turn(g, ai, slot);
                }
                Phase::World => {
                    // barbarians() (0x2620) raids, then warming, disasters, power, treasury
                    g.barbarian_spawn();
                    g.process_world();
                    self.drain_events(g);
                    self.phase = Phase::NewTurn;
                }
                Phase::NewTurn => {
                    g.unit_limit_shown = false;
                    g.advance_calendar();
                    g.fifty_turn_block();
                    g.historian_check();
                    g.palace_growth_check();
                    self.drain_events(g);
                    self.phase = Phase::EarlierCivs;
                    self.ai_index = 0;
                }
                Phase::EarlierCivs => {
                    if self.ai_index >= g.human_slot as usize {
                        self.phase = Phase::HumanStart;
                        continue;
                    }
                    let slot = self.ai_index as Slot;
                    self.ai_index += 1;
                    self.civ_turn(g, ai, slot);
                }
                Phase::HumanStart => {
                    let slot = g.human_slot;
                    g.process_player_cities(slot);
                    self.drain_events(g);
                    g.process_player_turn_start(slot);
                    g.reset_moves(slot);
                    g.progress_work(slot);
                    g.refresh_all_vision();
                    g.active_unit = None;
                    g.active_unit = g.next_active_unit();
                    self.drain_events(g);
                    self.phase = if std::mem::take(&mut self.then_turn) { Phase::HumanUnits } else { Phase::Done };
                }
                Phase::Done => return TurnStep::Done,
            }
        }
    }

    /// Supply the answer to the pending decision.
    pub fn resolve(&mut self, g: &mut Game, reply: DecisionReply) {
        if let Some(d) = self.waiting.take() {
            g.apply_decision(&d, reply);
            self.drain_events(g);
        }
    }

    pub fn is_done(&self) -> bool {
        self.phase == Phase::Done && self.pending.is_empty() && self.waiting.is_none()
    }
}

/// The player a decision belongs to.
fn decision_slot(g: &Game, d: &Decision) -> Slot {
    match d {
        Decision::ChooseResearch { slot, .. } | Decision::ChooseGovernment { slot, .. } | Decision::SellForUpkeep { slot, .. } => *slot,
        Decision::ProductionChoice { city, .. } => g.city_opt(*city).map(|c| c.owner).unwrap_or(g.human_slot),
        Decision::Diplomacy(s) => s.human,
        Decision::KeepPlaying { .. } => g.human_slot,
        Decision::SpaceshipPart { slot, .. } | Decision::SpaceshipLaunch { slot } | Decision::StealAdvance { slot, .. } => *slot,
    }
}

/// What a caravan does when it reaches a city.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaravanAction {
    TradeRoute(CityId),
    HelpWonder(CityId),
    KeepMoving,
}

/// What a diplomat does when it reaches a foreign city: the lines of CIV.EXE's menu (o 0x1d575)
/// in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiplomatAction {
    Embassy,
    Investigate,
    StealTech,
    Sabotage,
    /// `subvert`: pay twice the price so that a treaty partner keeps the peace (offered only
    /// at peace).
    InciteRevolt { subvert: bool },
    MeetKing,
}

impl DiplomatAction {
    /// The menu line `i` (0..=5) of "Diplomat arrives in ...".
    pub fn from_menu(i: usize) -> Option<DiplomatAction> {
        Some(match i {
            0 => DiplomatAction::Embassy,
            1 => DiplomatAction::Investigate,
            2 => DiplomatAction::StealTech,
            3 => DiplomatAction::Sabotage,
            4 => DiplomatAction::InciteRevolt { subvert: false },
            5 => DiplomatAction::MeetKing,
            _ => return None,
        })
    }
}

/// Lines of the diplomat's menu.
pub const DIPLOMAT_MENU: [&str; 6] = ["Establish Embassy", "Investigate City", "Steal Technology", "Industrial Sabotage", "Incite a Revolt", "Meet with King"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiplomatResult {
    Embassy,
    Investigated,
    /// The improvement destroyed, or None when the production was lost.
    Sabotaged(Option<ImprovementId>),
    Stole(AdvanceId),
    /// The city revolted; `cost` gold was paid.
    Incited { cost: i32 },
    /// "No new technology found." (the diplomat stays).
    NoTechnology,
    /// An audience with the city owner's leader follows (the diplomat stays, without moves).
    MetKing,
}

/// Shields a caravan adds to a wonder.
pub const CARAVAN_WONDER_SHIELDS: i32 = 50;

/// Goods a caravan carries in the arrival message, by unit number (CIV.EXE ds:0x1972).
pub const CARAVAN_GOODS: [&str; 8] = ["Silk", "Silver", "Wine", "Copper", "Gems", "Dye", "Salt", "Spice"];

impl Game {
    /// Distance from (x, y) to the nearest city of `slot` with a Palace, at most 16 (CIV.EXE
    /// 0x156ed).
    pub fn capital_distance(&self, slot: Slot, x: i32, y: i32) -> i32 {
        self.all_cities().filter(|c| c.owner == slot && c.has_improvement(imp::PALACE)).map(|c| Map::civ_distance(c.pos(), (x, y))).fold(16, i32::min)
    }

    /// Gold a Diplomat pays to bribe unit `target` (CIV.EXE o 0x1dbfa): (its owner's gold +
    /// 750) / (distance to its capital + 2) x the unit's cost in tens, halved for anything but
    /// Settlers.
    pub fn bribe_cost(&self, target: UnitId) -> i32 {
        let t = self.unit(target);
        let cost = (self.player(t.owner).gold + 750) / (self.capital_distance(t.owner, t.x, t.y) + 2) * t.info().cost as i32;
        if t.kind == unit_ids::SETTLERS {
            cost
        } else {
            cost / 2
        }
    }

    /// `by` pays for unit `target`, which is replaced by a new unit of the same type of its own
    /// (CIV.EXE o 0x1de0d). Returns the new unit.
    pub fn bribe_unit(&mut self, by: Slot, target: UnitId) -> UnitId {
        let cost = self.bribe_cost(target);
        let t = self.unit(target).clone();
        self.remove_unit(target);
        let new = self.add_unit(by, t.kind, t.x, t.y);
        // the new unit is also seen by the unit's former owner (o 0x1de89)
        self.unit_mut(new).seen_by |= 1 << t.owner;
        self.player_mut(by).gold -= cost;
        self.refresh_vision(by);
        self.refresh_vision(t.owner);
        self.push_event(GameEvent::UnitBribed { kind: t.kind, from: t.owner, by, at: (t.x, t.y) });
        new
    }

    /// Begin end-of-turn processing for the human player. A new game whose opening did not
    /// run yet (`opening`) runs it first.
    pub fn end_turn(&mut self) -> TurnRunner {
        match self.take_opening() {
            Some(mut r) => {
                r.then_turn = true;
                r
            }
            None => TurnRunner::new(),
        }
    }

    /// The opening of a new game (`TurnRunner::opening`), once, before the human's first move.
    pub fn take_opening(&mut self) -> Option<TurnRunner> {
        std::mem::take(&mut self.opening).then(TurnRunner::opening)
    }

    /// City growth, production, science and gold for one player: the first step of its turn
    /// in CIV.EXE's civ loop (update_civ_tax_rates 0xa5e2, called at 0x250d).
    pub fn process_player_cities(&mut self, slot: Slot) {
        // update_civ_tax_rates (0xa5e2) notes a city above size 5 with as many unhappy as
        // happy citizens (bit 0) and a city in disorder (bit 1)
        let mut flags = 0u8;
        for c in self.cities_of(slot) {
            if self.city_opt(c).is_some() {
                let balance = self.process_city(c);
                if let Some(city) = self.city_opt(c) {
                    if balance == Some(0) && city.size > 5 {
                        flags |= 1;
                    }
                    if city.disorder {
                        flags |= 2;
                    }
                }
            }
        }
        if !self.player(slot).human && slot != 0 {
            self.ai_tax_rates(slot, flags);
        }
        // the treasury holds at most 30000 gold, capped after the civ's cities (0x2515)
        if self.player(slot).gold > 30000 {
            self.player_mut(slot).gold = 30000;
        }
        if !self.player(slot).human && self.player(slot).gold < 0 {
            self.player_mut(slot).gold = 0;
        }
        self.player_mut(slot).sold_cities.clear();
    }

    /// Start-of-turn bookkeeping: anarchy countdown, peace-turn score, reset per-turn flags.
    pub fn process_player_turn_start(&mut self, slot: Slot) {
        if !self.player(slot).alive {
            return;
        }
        self.apollo_reveal(slot);
        self.anarchy_check(slot);
        // the peaceful turns ds:0xd808 (one count for the world): one more at the human's turn
        // from 1 AD on (0x259a); any fight between two civilizations resets them
        // (`Game::fight_diplomacy`)
        if slot == self.human_slot && self.year() >= 1 {
            for p in self.players.iter_mut() {
                p.peace_turns += 1;
            }
        }
        // treaties to break, lapsing contacts, requests for help (0x1495a)
        self.civ_politics(slot);
    }

    /// GAME > Retire answered "Yes, Quit" (ds:0xdc48 = 2): the game ends now and the human's
    /// score is frozen for the end-of-game screens (0x2c20).
    pub fn retire(&mut self) {
        if self.ended.is_none() {
            self.ended = Some(EndReason::Retired);
        }
        if !self.scoring_over {
            self.scoring_over = true;
            let human = self.human_slot;
            let total = self.score(human).total;
            self.player_mut(human).final_score = Some(total);
        }
    }

    /// The game ends for `reason` unless it already has: the first ending shows the scores,
    /// one after the human kept playing does not (ds:0x4cd8 bit 0x100).
    pub fn end_game(&mut self, reason: EndReason) {
        if self.ended.is_some() {
            return;
        }
        self.ended = Some(reason);
        self.keep_playing_asked = false;
        self.push_event(if self.scoring_over { GameEvent::GameEndedAgain(reason) } else { GameEvent::GameEnded(reason) });
    }

    /// Pollution, global warming, spaceship flight, end-of-game checks (once per turn, after all
    /// players and before the turn counter advances).
    pub fn process_world(&mut self) {
        self.global_warming_check();
        self.disaster_check();
        // the power ranking (0x3121): the power history, then the world against a leading human
        self.record_power();
        self.gang_up_on_leader();
        self.treasury_warning();
        self.spaceship_arrivals();
        // the coming year is the one CIV.EXE compares after its advance (0x2a28, 0x2a98):
        // a warning 20 years ahead, the end in the year 2100 - 20 * difficulty itself
        let end_year = rules::DIFFICULTY[self.setup.difficulty as usize].end_year;
        let year = self.next_year();
        if year == end_year - 20 {
            self.push_event(GameEvent::RetirementWarning(self.human_slot));
        }
        if !self.scoring_over && year >= end_year {
            self.end_game(EndReason::TimeLimit);
        }
        // the game is over: freeze the score once, and after the ending (0x2bd7) a conquest
        // quits while any other ending asks *FAME whether to keep playing
        if let Some(reason) = self.ended {
            let human = self.human_slot;
            if !self.scoring_over {
                self.scoring_over = true;
                let total = self.score(human).total;
                self.player_mut(human).final_score = Some(total);
            }
            if !self.keep_playing_asked && !matches!(reason, EndReason::Destroyed | EndReason::Conquest) && self.player(human).alive {
                self.keep_playing_asked = true;
                self.push_event(GameEvent::NeedsDecision(Decision::KeepPlaying { reason }));
            }
        }
    }

    /// A computer player's rates for its next turn, set after its cities (update_civ_tax_rates
    /// 0xa699..0xa760), in tenths: luxuries start from the last ones (10 - sci - tax), one more
    /// when a city is in disorder (`flags` bit 1) and science + taxes exceed 6, one less every
    /// fourth turn when no city is in disorder or balanced (`flags` 0) and science + taxes are
    /// below 8, within 0..4; science is the leader's "civilized" trait + (10 - luxuries) / 2, one
    /// more with more gold than turn + 100, none at all once Robotics is known; taxes get the
    /// rest. No government limit applies.
    pub fn ai_tax_rates(&mut self, slot: Slot, flags: u8) {
        let turn = self.turn as i32;
        let p = self.player(slot);
        let (sci, tax) = ((100 - p.tax as i32 - p.lux as i32) / 10, p.tax as i32 / 10);
        let mut lux = 10 - (sci + tax);
        if flags & 2 != 0 && sci + tax > 6 {
            lux += 1;
        }
        if turn & 3 == 0 && flags == 0 && sci + tax < 8 {
            lux -= 1;
        }
        let lux = lux.clamp(0, 4);
        let mut sci = p.civ().civilized as i32 + (10 - lux) / 2;
        if p.gold > turn + 100 {
            sci += 1;
        }
        if p.knows(adv::ROBOTICS) {
            sci = 0;
        }
        let tax = 10 - (sci + lux);
        let p = self.player_mut(slot);
        p.tax = (tax * 10) as u8;
        p.lux = (lux * 10) as u8;
    }

    /// A computer player's first rates (civ_init o 0x4e0b): science the "civilized" trait + 3
    /// tenths, taxes 9 - science, luxuries 1. (The human starts at 5/0/5, o 0x475b.)
    pub fn ai_initial_rates(&mut self, slot: Slot) {
        let sci = self.player(slot).civ().civilized as i32 + 3;
        let p = self.player_mut(slot);
        p.tax = ((9 - sci) * 10) as u8;
        p.lux = 10;
    }

    /// `set_government` (CIV.EXE 0x15031) gives a computer player taxes of 4 - government / 2
    /// tenths; its science stays and luxuries take the rest (none when the two exceed 10, which
    /// civrs cannot hold: then science gets the rest).
    pub fn ai_government_rates(&mut self, slot: Slot) {
        let p = self.player(slot);
        let sci = (100 - p.tax as i32 - p.lux as i32) / 10;
        let tax = 4 - p.govt as i32 / 2;
        let p = self.player_mut(slot);
        p.tax = (tax * 10) as u8;
        p.lux = ((10 - sci - tax).max(0) * 10) as u8;
    }

    /// The *TAXES warning (CIV.EXE 0x268a): while the game goes on, a human with more than one
    /// city, less than 100 gold and a treasury that would be empty within ten turns at this
    /// turn's loss (`(gold - gold at the turn's start) * 10 + gold < 0`) is told. The turn's
    /// start is this check's previous run (`taxes_mark`, not saved: a loaded game skips one).
    pub fn treasury_warning(&mut self) {
        let me = self.human_slot;
        let gold = self.player(me).gold;
        let Some(start) = self.taxes_mark.replace(gold) else { return };
        if self.ended.is_none() && self.player(me).alive && self.cities_of(me).len() > 1 && gold < 100 && (gold - start) * 10 + gold < 0 {
            self.push_event(GameEvent::LowFunds { slot: me });
        }
    }

    /// Palace growth (CIV.EXE 0x2966..0x29ec), once a turn after the year advances: when the
    /// human has a city with a Palace, the palace option is on and scoring goes on, a score
    /// (`civ_score`) of at least `palace_threshold(level + 1)` raises the palace level
    /// (ds:0x1c20), and the palace screen offers an addition up to level 37.
    pub fn palace_growth_check(&mut self) {
        let me = self.human_slot;
        if !self.options.palace || self.scoring_over || !self.all_cities().any(|c| c.owner == me && c.has_improvement(imp::PALACE)) {
            return;
        }
        let level = self.player(me).palace_level as i32;
        if self.score(me).total < palace_threshold(level + 1, self.setup.difficulty as i32) {
            return;
        }
        let p = self.player_mut(me);
        p.palace_level = p.palace_level.saturating_add(1);
        if p.palace_level <= 37 {
            self.push_event(GameEvent::PalaceAddition(me));
        }
    }

    // ---- cities -----------------------------------------------------------------------------

    /// Process one city for one turn, in the order of CIV.EXE's city routine (process_city
    /// 0xc94b, mode 0): the food store of the turns before first (famine 0xd007, growth
    /// 0xd187), then the workers (0xd454), food and shields into the stores (0xddd9), the
    /// production (0xde46..0xeb15) and a computer city's purchases (0xec0d); with this turn's
    /// buildings and units the trade split and the happiness (0xf084, 0xf3f8), the units the
    /// shields cannot support (0xfd20), disorder and celebration (0x125eb), taxes and science
    /// (0x12955, 0x12bde), and for the human pollution, meltdown and upkeep (0x12de9..0x131d2).
    /// The disorder and celebration flags the city has during its turn are those of its last
    /// turn: they keep this turn's shields out of the store and decide the tile yields.
    /// Returns happy − unhappy as the routine does (0x13223), `None` when the city is gone.
    pub fn process_city(&mut self, c: CityId) -> Option<i32> {
        let slot = self.city(c).owner;
        // a store out of 0..=999 (a shortage leaves it negative) starts again from 0 (0xc977)
        {
            let cm = self.city_mut(c);
            if !(0..=999).contains(&cm.shields) {
                cm.shields = 0;
            }
        }
        if !self.process_food(c) {
            return None;
        }
        self.assign_workers(c, true);
        let y = self.city_yield_raw(c);
        let was_disorder = self.city(c).disorder;
        {
            // the store takes shields less support, even when that is negative; nothing while
            // the city was in disorder (0xde1e)
            let cm = self.city_mut(c);
            cm.food += y.food_surplus();
            if !was_disorder {
                cm.shields += y.shield_surplus();
            }
        }
        // production
        let prod = self.city(c).production;
        if self.city(c).shields >= self.production_cost(c, prod) {
            self.complete_production(c, prod);
        }
        if self.city_opt(c).is_none() {
            return None;
        }
        // a computer city's purchases and alert (CIV.EXE 0xec0d..0xef79), before its taxes
        if !self.player(slot).human {
            let net = if was_disorder { 0 } else { y.shield_surplus() };
            crate::ai::city_ai::city_turn_purchase(self, c, net);
        }
        // trade split and happiness with the buildings and units of this turn, counted before
        // the units the city cannot support go (0xfbe6 follows 0xf3f8)
        let h = self.city_happiness(c);
        self.disband_unsupported(c, y.shields);
        self.city_moods(c, &h, y.food - (y.food_upkeep - 2 * self.city(c).size as i32));
        // gold & science (0x12955: no taxes in disorder or under Anarchy, none for the
        // barbarians, 0x125e1; 0x12bde: science whatever the disorder, none under Anarchy)
        let t = self.city_yield_raw(c);
        if slot != 0 && !h.disorder && self.player(slot).govt != Govt::Anarchy {
            self.player_mut(slot).gold += t.tax;
        }
        self.city_science(c, t.sci);
        // hazards and upkeep, the human's cities only (0x12ddd)
        self.pollution_roll(c);
        self.meltdown_roll(c);
        self.pay_upkeep(c);
        if let Some(cm) = self.cities[c as usize].as_mut() {
            cm.bought = false;
        }
        self.restash_garrison(c);
        Some(h.happy - h.unhappy)
    }

    /// Disorder and celebration after the city's turn (CIV.EXE 0x125eb..0x12bd1; not for the
    /// barbarians, 0x125e1). Fewer happy than unhappy citizens: disorder (a computer city that
    /// falls into it, or comes out of it, asks its advisor for production again, 0x1275a /
    /// 0x128f8). A celebration (no unhappy citizen, size 3 or more, half happy, not Anarchy)
    /// that goes on from the turn before makes a city of the Republic or Democracy grow by one
    /// while it has a food surplus, above size 9 only with an Aqueduct (0x12ab2..0x12b12).
    /// `food_after_settlers` is the food of the worked squares less what the city's Settlers
    /// eat (ds:0x70da − food * ds:0xe3c6), which must beat twice the size.
    fn city_moods(&mut self, c: CityId, h: &super::happiness::Happiness, food_after_settlers: i32) {
        let slot = self.city(c).owner;
        if slot == 0 {
            return;
        }
        let human = self.player(slot).human;
        let was_disorder = self.city(c).disorder;
        let was_celebrating = self.city(c).celebrating;
        {
            let cm = self.city_mut(c);
            cm.disorder = h.disorder;
            cm.celebrating = h.celebration;
            if h.disorder {
                cm.disorder_turns = cm.disorder_turns.saturating_add(1);
            } else {
                cm.disorder_turns = 0;
            }
        }
        if h.disorder != was_disorder {
            self.push_event(if h.disorder { GameEvent::Disorder(c) } else { GameEvent::DisorderEnded(c) });
            if !human {
                let next = self.ai_production(c);
                self.city_mut(c).production = next;
            }
        }
        // a Democracy falls in the city loop (0x127ba), so the cities after this one work
        // under Anarchy
        if self.city(c).disorder_turns >= 2 && self.player(slot).govt == Govt::Democracy {
            self.player_mut(slot).senate_blocks = self.player(slot).senate_blocks.saturating_add(1);
            self.government_collapse(slot, c);
        }
        if h.celebration && !was_celebrating {
            self.push_event(GameEvent::Celebration(c));
        } else if !h.celebration && was_celebrating {
            self.push_event(GameEvent::CelebrationEnded(c));
        }
        if h.celebration && was_celebrating && self.player(slot).govt as u8 >= Govt::Republic as u8 {
            let city = self.city(c);
            let size = city.size as i32;
            if 2 * size < food_after_settlers && (size < 10 || city.has_improvement(imp::AQUEDUCT)) {
                self.city_mut(c).size += 1;
                self.fix_workers(c);
                self.push_event(GameEvent::CityGrew(c));
            }
        }
    }

    /// Units the city's shields cannot support go, the farthest from the city first (CIV.EXE
    /// 0xfbe6..0xff75): while the support of the units homed here, counted again with those
    /// built this turn, exceeds `shields` (ds:0x70dc, the shields of the worked squares after
    /// Factory and power, disorder or not, from before the production), the unit with the
    /// longest travel distance (0x1d43:0x27f) from it is disbanded, the first in unit order on
    /// a tie ("CITY can't support UNIT. Unit Disbanded." for the human).
    fn disband_unsupported(&mut self, c: CityId, shields: i32) {
        loop {
            let upkeep = self.shield_costs(c).iter().filter(|(_, cost)| *cost).count() as i32 + self.garrison_upkeep(c);
            if upkeep <= shields {
                return;
            }
            let Some(victim) = self.farthest_supported_unit(c) else { return };
            let kind = self.unit(victim).kind;
            self.remove_unit(victim);
            self.push_event(GameEvent::UnitDisbanded { unit_kind: kind, city: c, reason: "shortage of shields".into() });
        }
    }

    /// Supported unit farthest from its home city (disbanded first on shortages): the longest
    /// travel distance, the first in unit order on a tie; Diplomats and Caravans cost no
    /// shields and are never chosen (CIV.EXE 0xfdd9..0xfe42).
    fn farthest_supported_unit(&self, c: CityId) -> Option<UnitId> {
        let city = self.city(c);
        let mut best: Option<(UnitId, i32)> = None;
        for id in self.supported_units(c) {
            let u = self.unit(id);
            if matches!(u.kind, unit_ids::DIPLOMAT | unit_ids::CARAVAN) {
                continue;
            }
            let d = Map::civ_distance((u.x, u.y), (city.x, city.y));
            if best.map(|b| d > b.1).unwrap_or(true) {
                best = Some((id, d));
            }
        }
        best.map(|b| b.0)
    }

    /// The city's food store as the turns before left it (CIV.EXE 0xd007..0xd26b), at the start
    /// of its turn. Below zero, famine: "Food storage exhausted in CITY!" and a Settlers unit
    /// homed here is disbanded ("Settlers lost."), or else the city shrinks ("Famine feared."),
    /// a city of size 1 starving to death (destroy_city 0xa76a); the store is emptied. Full
    /// ((size + 1) * cost factor): the city grows and a Granary refills the store with 5 * the
    /// new size + 5 (0xd1c7); a city that so passes size 10 without an Aqueduct shrinks back
    /// at once, the store as refilled ("CITY requires an AQUEDUCT for further growth.").
    /// Returns false when the city is gone.
    fn process_food(&mut self, c: CityId) -> bool {
        let owner = self.city(c).owner;
        if self.city(c).food < 0 {
            // the first Settlers in unit order (0xd037)
            let settler = self.supported_units(c).into_iter().find(|id| self.unit(*id).kind == unit_ids::SETTLERS);
            let name = self.city(c).name.clone();
            self.push_event(GameEvent::FoodExhausted { city: c, name, owner, settlers_lost: settler.is_some() });
            if let Some(id) = settler {
                self.remove_unit(id);
            } else {
                self.city_mut(c).size -= 1;
                if self.city(c).size == 0 {
                    self.city_wiped_out(c);
                    return false;
                }
                self.push_event(GameEvent::CityShrank(c));
            }
            self.city_mut(c).food = 0;
        }
        if self.city(c).food >= self.food_box(c) {
            let granary = self.city(c).has_improvement(imp::GRANARY);
            let aqueduct = self.city(c).has_improvement(imp::AQUEDUCT);
            let cm = self.city_mut(c);
            cm.size += 1;
            cm.food = if granary { 5 * cm.size as i32 + 5 } else { 0 };
            if cm.size > 10 && !aqueduct {
                cm.size -= 1;
                self.push_event(GameEvent::AqueductNeeded(c));
            } else {
                self.push_event(GameEvent::CityGrew(c));
            }
        }
        true
    }

    /// A city emptied by famine or by its last citizen leaving as Settlers goes as CIV.EXE's
    /// destroy_city (0xa76a) takes it (`remove_city` disbands the units it supports and the
    /// non-land units in its square), and its owner may be out of the game (0xd159, 0xe041).
    fn city_wiped_out(&mut self, c: CityId) {
        let owner = self.city(c).owner;
        self.remove_city(c);
        self.city_lost_check(owner, 0);
    }

    /// Pay the city's buildings one by one, in improvement order; one the treasury cannot pay
    /// for is sold for its full cost, the treasury having been set to 0 ("CITY can't maintain
    /// BUILDING.", CIV.EXE 0x13033..0x131cf).
    fn pay_upkeep(&mut self, c: CityId) {
        let slot = self.city(c).owner;
        if !self.pays_upkeep(slot) {
            return;
        }
        for i in self.city(c).improvement_list() {
            if i == imp::PALACE {
                continue;
            }
            let cost = self.improvement_upkeep(slot, i);
            self.player_mut(slot).gold -= cost;
            if self.player(slot).gold < 0 {
                self.player_mut(slot).gold = super::economy::sell_price(i);
                self.city_mut(c).remove_improvement(i);
                self.push_event(GameEvent::CantMaintain { city: c, improvement: i });
            }
        }
    }

    /// Sell an improvement to cover upkeep (ignores the one-sale-per-turn rule).
    fn force_sell(&mut self, c: CityId, id: ImprovementId) {
        if !self.city(c).has_improvement(id) || id == imp::PALACE {
            return;
        }
        let gold = super::economy::sell_price(id);
        let slot = self.city(c).owner;
        self.city_mut(c).remove_improvement(id);
        self.player_mut(slot).gold += gold;
        self.push_event(GameEvent::ImprovementSold { city: c, improvement: id, gold });
    }

    /// Once a turn after turn 50 (CIV.EXE 0x262d) a random one of the 128 city slots is tried:
    /// a city of size 5 or more whose owner has more than one city (ds:0x6b66) meets
    /// `city_disaster`. (The
    /// original reuses its loop counter for the slot, so slots 0 and 1 lead to another try.)
    pub fn disaster_check(&mut self) {
        if self.turn <= 50 {
            return;
        }
        let mut i = 0u32;
        while i < 2 {
            i = self.rng.below(128);
            let Some(city) = self.city_opt(i as CityId) else {
                i += 1;
                continue;
            };
            if self.cities_of(city.owner).len() > 1 && city.size >= 5 {
                self.city_disaster(i as CityId);
            }
            i += 1;
        }
    }

    /// A random event in a city (CIV.EXE o 0x1b8b5): `random(10)` picks an earthquake (hills
    /// nearby), plague, flood (river nearby), volcano (mountains nearby), famine, fire, pirates
    /// (ocean nearby) or, 3 times in 10, unrest if the city has more unhappy than happy
    /// citizens. Each natural disaster needs the advance behind the improvement that prevents
    /// it (plague also stops with Medicine); the surrounding terrain includes the city square.
    fn city_disaster(&mut self, c: CityId) {
        let city = self.city(c).clone();
        let owner = city.owner;
        let near = |t: Terrain| std::iter::once((city.x, city.y)).chain(Map::neighbours8(city.x, city.y)).any(|(x, y)| Map::in_bounds(x, y) && self.map.tile(x, y).terrain == t);
        let (hills, river, mountains, ocean) = (near(Terrain::Hills), near(Terrain::River), near(Terrain::Mountains), near(Terrain::Ocean));
        let p = self.player(owner);
        let knows_prereq = |i: ImprovementId| rules::IMPROVEMENTS[i as usize].prereq.map(|a| p.knows(a)).unwrap_or(true);
        let lacks = |i: ImprovementId| !city.has_improvement(i) && knows_prereq(i);
        let (no_aqueduct, no_walls, no_temple, no_granary) = (lacks(imp::AQUEDUCT), lacks(imp::CITY_WALLS), lacks(imp::TEMPLE), lacks(imp::GRANARY));
        let medicine = p.knows(adv::MEDICINE);
        // an improvement other than the Palace among the first 16 (the original tests one word)
        let has_low_improvement = (2..=16).any(|i| city.has_improvement(i));
        let lose_citizens = |g: &mut Game, div: u8| {
            let loss = g.city(c).size / div;
            g.shrink_city(c, loss as i32);
        };
        let kind = match self.rng.below(10) {
            0 if hills && has_low_improvement => Disaster::Earthquake(self.random_improvement_lost(c)),
            1 if !medicine && no_aqueduct => {
                lose_citizens(self, 4);
                Disaster::Plague
            }
            2 if river && no_walls => {
                if city.size > 1 {
                    lose_citizens(self, 4);
                }
                Disaster::Flood
            }
            3 if mountains && no_temple => {
                if city.size > 1 {
                    lose_citizens(self, 3);
                }
                Disaster::Volcano
            }
            4 if no_granary => {
                self.city_mut(c).food = 0;
                lose_citizens(self, 3);
                Disaster::Famine
            }
            5 if has_low_improvement && no_aqueduct => Disaster::Fire(self.random_improvement_lost(c)),
            6 if ocean && !city.has_improvement(imp::BARRACKS) => {
                let cm = self.city_mut(c);
                cm.food = 0;
                cm.shields = 0;
                Disaster::Pirates
            }
            7..=9 => return self.city_unrest(c),
            _ => return,
        };
        self.push_event(GameEvent::Disaster { city: c, kind });
    }

    /// Remove a random improvement (draws of `random(24)` until one the city has, never the
    /// Palace) and return it.
    fn random_improvement_lost(&mut self, c: CityId) -> ImprovementId {
        loop {
            let bit = self.rng.below(24);
            let i = bit as ImprovementId + 1;
            if bit != 0 && self.city(c).has_improvement(i) {
                self.city_mut(c).remove_improvement(i);
                return i;
            }
        }
    }

    /// Unrest (CIV.EXE o 0x1bc5d): a city with fewer happy than unhappy citizens loses its
    /// food and shields ("Rioting in / Corruption in / Scandal in CITY! Citizens demand ..."
    /// naming the first of Temple, Courthouse, Marketplace and Cathedral it lacks, else lower
    /// taxes). If its owner has more than three cities (ds:0x6b66) such a city, unless it has
    /// the Palace, then
    /// defects to the owner of the city with the best `32 * (happy - unhappy) / distance`
    /// above 4 among the city slots before it.
    pub(crate) fn city_unrest(&mut self, c: CityId) {
        let h = self.city_happiness(c);
        if h.happy >= h.unhappy {
            return;
        }
        let city = self.city(c).clone();
        let wording = self.rng.below(3) as u8;
        let demand = [imp::TEMPLE, imp::COURTHOUSE, imp::MARKETPLACE]
            .into_iter()
            .find(|i| !city.has_improvement(*i))
            .or_else(|| (self.player(city.owner).knows(adv::RELIGION) && !city.has_improvement(imp::CATHEDRAL)).then_some(imp::CATHEDRAL));
        self.push_event(GameEvent::Disaster { city: c, kind: Disaster::Unrest { wording, demand } });
        let cm = self.city_mut(c);
        cm.food = 0;
        cm.shields = 0;
        if city.has_improvement(imp::PALACE) || self.cities_of(city.owner).len() <= 3 {
            return;
        }
        let mut best = (4, None);
        for i in 0..128 as CityId {
            if i == c {
                break;
            }
            let Some(other) = self.city_opt(i) else { break };
            let d = Map::civ_distance(other.pos(), city.pos()).max(1);
            let oh = self.city_happiness(i);
            let score = 32 * (oh.happy - oh.unhappy) / d;
            if score > best.0 {
                best = (score, Some(i));
            }
        }
        let Some(admired) = best.1 else { return };
        let to = self.city(admired).owner;
        if to == city.owner {
            return;
        }
        self.push_event(GameEvent::CityDefected { city: c, admired, from: city.owner });
        // city_revolt (o 0x1e06a) hands it over with capture_city (0x13d76), whose capture
        // message the human reads when involved or with an embassy
        self.transfer_city(c, to);
        self.refresh_vision(to);
        self.refresh_vision(city.owner);
        self.city_lost_check(city.owner, to);
    }

    /// Reduce a city's size by `n` (never below 1).
    pub fn shrink_city(&mut self, c: CityId, n: i32) {
        for _ in 0..n {
            if self.city(c).size <= 1 {
                break;
            }
            self.city_mut(c).size -= 1;
            self.fix_workers(c);
        }
        self.push_event(GameEvent::CityShrank(c));
    }

    /// Destroy a random improvement (never the Palace); returns the id destroyed.
    pub fn destroy_random_improvement(&mut self, c: CityId) -> Option<ImprovementId> {
        let cands = self.sellable_improvements(c);
        let id = *self.rng.pick(&cands)?;
        self.city_mut(c).remove_improvement(id);
        Some(id)
    }

    /// Production after an improvement or wonder is finished: a human's city stays on it, so
    /// the city screen's CHANGE button flashes until something else is chosen; the computer asks
    /// its production advisor (CIV.EXE 0xe720), an AUTO city of the human takes the advisor's
    /// improvement, and with none worth it AUTO goes off (0xeb15..0xec08).
    fn after_building(&mut self, c: CityId, human: bool) {
        if !human {
            let next = self.ai_production(c);
            self.city_mut(c).production = next;
        } else if self.city(c).auto_build {
            match self.auto_production(c) {
                Some(next) => self.city_mut(c).production = next,
                None => self.city_mut(c).auto_build = false,
            }
        }
    }

    /// Finish building `prod` in city `c`. The human's store is emptied after anything it
    /// completes, a computer city's after a wonder; otherwise the shields beyond the cost stay
    /// (CIV.EXE 0xe3c4, 0xe6f9).
    pub fn complete_production(&mut self, c: CityId, prod: Production) {
        let (x, y, owner) = {
            let city = self.city(c);
            (city.x, city.y, city.owner)
        };
        let human = self.is_human(owner);
        match prod {
            Production::Unit(k) => {
                // Settlers need a second citizen only on Chieftain (0xde78)
                if k == unit_ids::SETTLERS && self.city(c).size <= 1 && self.setup.difficulty == rules::Difficulty::Chieftain {
                    self.push_event(GameEvent::ProductionBlocked { city: c, item: prod });
                    return;
                }
                // logged before a computer Diplomat looks for its place (0xdf0c)
                self.replay_unit_built(owner, k);
                // a computer Diplomat starts out next to a human city, heading for it; with no
                // such city the shields are given back and nothing is built (CIV.EXE 0xded6,
                // 0xe124..0xe379); either way the city re-chooses, as after every unit (0xe3b1).
                // (In CIV.EXE that code cannot run: 0xded6 does not create the computer's
                // Diplomat, so 0xe05d sends it to 0xe3bc, which empties the store and builds
                // nothing; civrs does what the code meant.)
                if k == unit_ids::DIPLOMAT && !human {
                    if let Some(((dx, dy), target)) = self.ai_diplomat_site(owner) {
                        let id = self.add_unit(owner, k, dx, dy);
                        self.unit_mut(id).home = Some(c);
                        let (tx, ty) = self.city(target).pos();
                        self.set_goto(id, tx, ty);
                        self.city_mut(c).shields -= self.production_cost(c, prod);
                        self.refresh_vision(owner);
                        self.push_event(GameEvent::ItemBuilt { city: c, item: prod });
                    }
                    let next = self.ai_rechoose(c);
                    self.city_mut(c).production = next;
                    return;
                }
                // with every unit slot in use the shields are spent on nothing and the store
                // is emptied (create_unit 0x8b62 returns -1; 0xdeb8, 0xdf9d -> 0xe3bc), and the
                // human hears *UMAX once a turn
                if !self.unit_room(owner) {
                    self.city_mut(c).shields = 0;
                    if human && !std::mem::replace(&mut self.unit_limit_shown, true) {
                        self.push_event(GameEvent::UnitLimit(owner));
                    }
                    return;
                }
                let id = self.add_unit(owner, k, x, y);
                self.unit_mut(id).home = Some(c);
                // "New units are Veterans when produced at a city containing a BARRACKS": every
                // kind of unit (CIV.EXE 0xdf72)
                if self.city(c).has_improvement(imp::BARRACKS) {
                    self.unit_mut(id).veteran = true;
                }
                let cost = self.production_cost(c, prod);
                self.city_mut(c).shields = if human { 0 } else { self.city(c).shields - cost };
                // Settlers take a citizen, unless it is the last one of the civilization's only
                // city; the last citizen of a city leaves it empty: the city is destroyed and
                // the Settlers start anew on its square, homed by create_unit 0x8b51 like any new unit
                // (0xdfa7..0xe04e)
                if k == unit_ids::SETTLERS && (self.city(c).size > 1 || self.cities_of(owner).len() > 1) {
                    self.city_mut(c).size -= 1;
                    if self.city(c).size == 0 {
                        self.city_wiped_out(c);
                        self.add_unit(owner, k, x, y);
                        self.refresh_vision(owner);
                        return;
                    }
                    self.fix_workers(c);
                }
                self.refresh_vision(owner);
                self.push_event(GameEvent::ItemBuilt { city: c, item: prod });
                // a computer city re-chooses after every unit (city_rechoose_production 0x18c4c,
                // called at 0xe3b1)
                if !human {
                    let next = self.ai_rechoose(c);
                    self.city_mut(c).production = next;
                }
            }
            Production::Improvement(i) => {
                // still set to an improvement the city has: nothing is built, the shields
                // wait for a new choice (CIV.EXE 0xe4f1)
                if self.city(c).has_improvement(i) {
                    self.after_building(c, human);
                    return;
                }
                let cost = self.production_cost(c, prod);
                self.city_mut(c).shields = if human { 0 } else { self.city(c).shields - cost };
                if matches!(i, imp::SS_STRUCTURAL | imp::SS_COMPONENT | imp::SS_MODULE) {
                    // the part goes on the ship after an AI city re-chose its production
                    // (CIV.EXE 0xe719 before 0xe766)
                    self.push_event(GameEvent::ItemBuilt { city: c, item: prod });
                    self.after_building(c, human);
                    self.spaceship_part_built(c, i - imp::SS_STRUCTURAL);
                    if human {
                        self.push_event(GameEvent::NeedsDecision(Decision::ProductionChoice { city: c, reason: "built".into() }));
                    }
                    return;
                } else {
                    self.city_mut(c).add_improvement(i);
                    if i == imp::PALACE {
                        for other in self.cities_of(owner) {
                            if other != c {
                                self.city_mut(other).remove_improvement(imp::PALACE);
                            }
                        }
                        self.player_mut(owner).capital = Some(c);
                    }
                }
                self.push_event(GameEvent::ItemBuilt { city: c, item: prod });
                self.after_building(c, human);
                if human {
                    self.push_event(GameEvent::NeedsDecision(Decision::ProductionChoice { city: c, reason: "built".into() }));
                }
            }
            Production::Wonder(w) => {
                if self.wonders_built[w as usize].is_some() {
                    self.push_event(GameEvent::ProductionBlocked { city: c, item: prod });
                    self.after_building(c, human);
                    return;
                }
                self.wonders_built[w as usize] = Some(c);
                self.replay_log(super::replay::kind::WONDER, &[owner, w + 1]);
                let cm = self.city_mut(c);
                cm.add_wonder(w);
                // the store is emptied after a wonder, whoever built it (0xe6f9)
                cm.shields = 0;
                cm.caravan_shields = 0;
                self.push_event(GameEvent::WonderBuilt { city: c, wonder: w, owner });
                self.on_wonder_built(owner, w);
                self.after_building(c, human);
                if human {
                    self.push_event(GameEvent::NeedsDecision(Decision::ProductionChoice { city: c, reason: "built".into() }));
                }
            }
        }
    }

    /// "Makes all cities on the map visible": the owner of the Apollo Program sees every city
    /// square (CIV.EXE o 0x1b828, as the cities are updated each turn).
    pub fn apollo_reveal(&mut self, slot: Slot) {
        if !self.owner_has_wonder(slot, won::APOLLO_PROGRAM) {
            return;
        }
        let sites: Vec<(i32, i32)> = self.all_cities().map(|c| c.pos()).collect();
        for (x, y) in sites {
            self.map.explore(slot, x, y, 0);
        }
    }

    /// Immediate effects of completing a wonder.
    fn on_wonder_built(&mut self, owner: Slot, w: u8) {
        match w {
            won::MANHATTAN_PROJECT => self.nukes_enabled = true,
            won::APOLLO_PROGRAM => {
                self.apollo_built = true;
                self.apollo_reveal(owner);
            }
            won::DARWINS_VOYAGE => self.darwins_voyage(owner),
            _ => {}
        }
    }

    // ---- science ----------------------------------------------------------------------------

    // ---- units ------------------------------------------------------------------------------

    /// Triremes away from coast, aircraft out of fuel.
    pub fn process_unit_hazards(&mut self, slot: Slot) {
        let ids = self.units_of(slot);
        for id in ids {
            let Some(u) = self.unit_opt(id).cloned() else { continue };
            let in_own_city = self.city_at(u.x, u.y).map(|c| self.city(c).owner == slot).unwrap_or(false);
            // only the human's Triremes are lost at sea (CIV.EXE 0x75da)
            if u.kind == unit_ids::TRIREME && self.player(slot).human && !in_own_city && self.map.land_mask8(u.x, u.y) == 0 && self.rng.chance(50) {
                self.remove_unit(id);
                self.push_event(GameEvent::UnitLost { slot, kind: u.kind, at: (u.x, u.y), reason: "lost at sea".into() });
                continue;
            }
            if u.is_air() {
                let on_carrier = u.carried_by.map(|cr| self.unit_opt(cr).is_some()).unwrap_or(false);
                if in_own_city || on_carrier {
                    self.unit_mut(id).fuel = 0;
                    continue;
                }
                let fuel = u.fuel + 1;
                self.unit_mut(id).fuel = fuel;
                let range = rules::UNITS[u.kind as usize].range.max(1);
                if fuel >= range {
                    self.remove_unit(id);
                    self.push_event(GameEvent::UnitLost { slot, kind: u.kind, at: (u.x, u.y), reason: "ran out of fuel".into() });
                }
            }
        }
        self.refresh_vision(slot);
    }

    // ---- government -------------------------------------------------------------------------

    /// Governments the civ may choose: those whose advance it knows, or all of them with the
    /// Pyramids (CIV.EXE 0x14e39 skips the advance checks for an owner of wonder 1).
    pub fn governments_available(&self, slot: Slot) -> Vec<Govt> {
        let p = self.player(slot);
        let all = self.owner_has_wonder(slot, won::PYRAMIDS);
        Govt::ALL.iter().copied().filter(|g| *g != Govt::Anarchy && (all || rules::GOVERNMENTS[*g as usize].prereq.map(|a| p.knows(a)).unwrap_or(true))).collect()
    }

    /// The human's revolution (CIV.EXE 0x6f50, after *REV): the government is Anarchy at once,
    /// written directly (no `set_government`, so nothing is logged and the vendettas stay until
    /// the new government); `anarchy_check` ends it, the Pyramids included, at a later turn.
    pub fn revolution(&mut self, slot: Slot) {
        if self.player(slot).govt == Govt::Anarchy {
            return;
        }
        self.player_mut(slot).govt = Govt::Anarchy;
        for c in self.cities_of(slot) {
            self.city_mut(c).celebrating = false;
        }
        self.push_event(GameEvent::Revolution { slot });
    }

    /// A Democracy's city in disorder for a second turn in a row (CIV.EXE 0x127ba, inside the
    /// city loop): "Discontented citizens of CITY revolt: Government Collapses!" (the human's
    /// newspaper), and `set_government(civ, 0)` logs Anarchy and forgets the vendettas. The
    /// human's revolution flag ([civ][0] & 4, 0x12820, kept in `anarchy_turns`) makes the
    /// anarchy last past one more turn with `turn & 3 == 0`.
    pub fn government_collapse(&mut self, slot: Slot, city: CityId) {
        self.push_event(GameEvent::GovernmentCollapsed { slot, city });
        self.choose_government(slot, Govt::Anarchy);
        if self.player(slot).human {
            self.player_mut(slot).anarchy_turns = 1;
        }
    }

    /// The anarchy part of `civ_turn_politics` (CIV.EXE 0x1495a), at the start of the civ's
    /// turn: on turns with `turn & 3 == 0` the human's anarchy ends (or, with the revolution
    /// flag set, only the flag is cleared); otherwise, and for computer players, it ends on
    /// those turns or at once for an owner of the Pyramids. `set_government(civ, 1)` then asks
    /// the human and gives a computer player Despotism.
    pub fn anarchy_check(&mut self, slot: Slot) {
        if self.player(slot).govt != Govt::Anarchy {
            return;
        }
        let fourth = self.turn & 3 == 0;
        if self.player(slot).human && fourth && self.player(slot).anarchy_turns != 0 {
            self.player_mut(slot).anarchy_turns = 0;
            return;
        }
        if fourth || self.owner_has_wonder(slot, won::PYRAMIDS) {
            self.player_mut(slot).anarchy_turns = 0;
            self.push_event(GameEvent::AnarchyEnded { slot });
            self.offer_government(slot);
        }
    }

    /// Anarchy is over (`set_government(civ, 1)`, 0x14df7): the human picks a government, a
    /// computer player gets Despotism.
    fn offer_government(&mut self, slot: Slot) {
        let options = self.governments_available(slot);
        if self.player(slot).human {
            self.push_event(GameEvent::NeedsDecision(Decision::ChooseGovernment { slot, options }));
        } else {
            self.choose_government(slot, Govt::Despotism);
        }
    }

    /// A government for a reply that names none (CIV.EXE asks again instead).
    pub fn ai_pick_government(&self, slot: Slot) -> Govt {
        let opts = self.governments_available(slot);
        let civ = self.player(slot).civ();
        let at_war = (1..8u8).any(|o| o != slot && self.player(o).in_game && self.player(slot).at_war(o));
        let order: Vec<Govt> = if at_war {
            vec![Govt::Communism, Govt::Monarchy, Govt::Republic]
        } else if civ.civilized > 0 {
            vec![Govt::Democracy, Govt::Republic, Govt::Monarchy, Govt::Communism]
        } else {
            vec![Govt::Republic, Govt::Monarchy, Govt::Communism]
        };
        order.into_iter().find(|g| opts.contains(g)).unwrap_or(Govt::Despotism)
    }

    /// Install a government (ends anarchy).
    pub fn choose_government(&mut self, slot: Slot, g: Govt) {
        if g != Govt::Anarchy && !self.governments_available(slot).contains(&g) {
            return;
        }
        if self.player(slot).govt != g {
            self.forget_vendettas(slot);
            self.replay_log(super::replay::kind::GOVERNMENT, &[slot, g as u8]);
        }
        {
            let p = self.player_mut(slot);
            p.govt = g;
            p.anarchy_turns = 0;
        }
        if self.player(slot).human {
            self.clamp_rates(slot);
        } else {
            self.ai_government_rates(slot);
        }
        self.push_event(GameEvent::GovernmentChanged { slot, govt: g });
    }

    // ---- decisions --------------------------------------------------------------------------

    /// Apply a decision reply.
    pub fn apply_decision(&mut self, d: &Decision, reply: DecisionReply) {
        match d {
            Decision::ChooseResearch { slot, options } => {
                let pick = match reply {
                    DecisionReply::Choice(i) => options.get(i).copied(),
                    _ => options.first().copied(),
                };
                if let Some(a) = pick {
                    self.set_research(*slot, a);
                    self.research_chosen(*slot);
                }
            }
            Decision::ChooseGovernment { slot, options } => {
                let pick = match reply {
                    DecisionReply::Choice(i) => options.get(i).copied(),
                    _ => None,
                };
                let g = pick.unwrap_or_else(|| self.ai_pick_government(*slot));
                self.choose_government(*slot, g);
            }
            Decision::ProductionChoice { .. } => {}
            Decision::SellForUpkeep { slot, city, candidates } => {
                if self.city_opt(*city).is_some() && self.player(*slot).gold < 0 {
                    let pick = match reply {
                        DecisionReply::Choice(i) => candidates.get(i).copied(),
                        _ => None,
                    };
                    let id = pick.or_else(|| candidates.iter().copied().min_by_key(|i| rules::IMPROVEMENTS[*i as usize].cost));
                    if let Some(id) = id {
                        self.force_sell(*city, id);
                    }
                }
                if self.player(*slot).gold < 0 {
                    self.player_mut(*slot).gold = 0;
                }
            }
            // the audience screen answers its screens itself and replies `Cancel` at the end;
            // other callers (tests, headless runs) answer one screen at a time
            Decision::Diplomacy(state) => {
                let choice = match reply {
                    DecisionReply::Choice(i) => i,
                    DecisionReply::Yes => 0,
                    DecisionReply::No => 1,
                    DecisionReply::Cancel => return,
                };
                match crate::ai::diplo_ai::respond(self, state, choice) {
                    Some(next) => self.push_event(GameEvent::NeedsDecision(Decision::Diplomacy(next))),
                    None => self.audience_pending &= !(1 << state.ai),
                }
            }
            Decision::KeepPlaying { .. } => {
                if reply == DecisionReply::Yes {
                    self.ended = None;
                }
            }
            Decision::SpaceshipPart { slot, kind } => {
                let choice = match reply {
                    DecisionReply::Choice(i) => i,
                    _ => 0,
                };
                self.place_spaceship_part(*slot, *kind, choice);
            }
            Decision::SpaceshipLaunch { slot } => {
                if matches!(reply, DecisionReply::Choice(1) | DecisionReply::Yes) && self.spaceship_can_launch(*slot) {
                    let _ = self.launch_spaceship(*slot);
                }
            }
            Decision::StealAdvance { slot, from, options } => {
                let i = if let DecisionReply::Choice(i) = reply { i } else { 0 };
                if let Some(a) = options.get(i).or(options.first()).copied() {
                    if self.grant_advance(*slot, a, super::science::AdvanceSource::From(*from)) {
                        self.push_event(GameEvent::AdvanceAcquired { slot: *slot, adv: a, source: "conquest".into() });
                    }
                }
            }
        }
    }

    // ---- caravans ---------------------------------------------------------------------------

    /// A caravan next to (or in) a city acts.
    pub fn caravan_action(&mut self, id: UnitId, action: CaravanAction) -> Result<(), RuleError> {
        let u = self.unit_opt(id).cloned().ok_or(RuleError::Other("No such unit."))?;
        if u.kind != unit_ids::CARAVAN {
            return Err(RuleError::Other("Only Caravans can do that."));
        }
        match action {
            CaravanAction::KeepMoving => Ok(()),
            CaravanAction::HelpWonder(target) => {
                let city = self.city_opt(target).ok_or(RuleError::Other("No such city."))?;
                if Map::dist((u.x, u.y), (city.x, city.y)) > 1 {
                    return Err(RuleError::Other("The caravan must be next to the city."));
                }
                if city.owner != u.owner {
                    return Err(RuleError::Other("Caravans can only help your own wonders."));
                }
                if !matches!(city.production, Production::Wonder(_)) {
                    return Err(RuleError::Other("That city is not building a wonder."));
                }
                let cm = self.city_mut(target);
                cm.shields += CARAVAN_WONDER_SHIELDS;
                cm.caravan_shields += CARAVAN_WONDER_SHIELDS;
                self.remove_unit(id);
                self.refresh_vision(u.owner);
                Ok(())
            }
            CaravanAction::TradeRoute(target) => {
                let home = u.home.ok_or(RuleError::Other("The caravan has no home city."))?;
                if self.city_opt(home).is_none() {
                    return Err(RuleError::Other("The caravan's home city no longer exists."));
                }
                let city = self.city_opt(target).cloned().ok_or(RuleError::Other("No such city."))?;
                if Map::dist((u.x, u.y), (city.x, city.y)) > 1 {
                    return Err(RuleError::Other("The caravan must be next to the city."));
                }
                if target == home {
                    return Err(RuleError::Other("A city cannot trade with itself."));
                }
                // CIV.EXE 0x146be: the revenue goes to gold and research alike, then the route
                // is recorded (if it is new and strong enough) in the home city only
                let bonus = self.trade_route_revenue(u.owner, home, target);
                let p = self.player_mut(u.owner);
                p.gold += bonus;
                p.beakers += bonus;
                self.remove_unit(id);
                self.refresh_vision(u.owner);
                self.add_trade_route(home, target);
                self.push_event(GameEvent::TradeRouteMade { city: home, with: target, bonus, goods: (id & 7) as u8 });
                Ok(())
            }
        }
    }

    /// One-off revenue of a caravan of `slot` from `home` arriving in `target` (CIV.EXE
    /// 0x146be): `(distance + 10) * (trade of both) / 24`, halved on the same continent and
    /// halved again between cities of one civilization; a third less each when the target's
    /// owner knows the Railroad, then Flight. CIV.EXE looks up the home city's continent at
    /// (x, x) (0x14738 pushes x twice); civrs uses the home city's real tile.
    pub fn trade_route_revenue(&self, slot: Slot, home: CityId, target: CityId) -> i32 {
        let (a, b) = (self.city(home), self.city(target));
        let mut v = (Map::dist(a.pos(), b.pos()) + 10) * (self.visible_trade(home) + self.visible_trade(target)) / 24;
        if self.map.tile(a.x, a.y).continent == self.map.tile(b.x, b.y).continent {
            v /= 2;
        }
        if b.owner == slot {
            v /= 2;
        }
        for a in [adv::RAILROAD, adv::FLIGHT] {
            if self.player(b.owner).knows(a) {
                v -= v / 3;
            }
        }
        v
    }

    /// Record a route of city `c` (CIV.EXE 0x148a7): up to three; when full, the new route
    /// replaces the weakest one if it is stronger (by the partner's trade, doubled abroad).
    fn add_trade_route(&mut self, c: CityId, other: CityId) {
        if self.city(c).trade_routes.contains(&other) {
            return;
        }
        if self.city(c).trade_routes.len() < 3 {
            self.city_mut(c).trade_routes.push(other);
            return;
        }
        let owner = self.city(c).owner;
        let weight = |r: CityId| self.city_opt(r).map(|rc| self.visible_trade(r) * if rc.owner != owner { 2 } else { 1 }).unwrap_or(-1);
        let routes = self.city(c).trade_routes.clone();
        let (i, w) = routes.iter().enumerate().map(|(i, r)| (i, weight(*r))).min_by_key(|(_, w)| *w).unwrap();
        if w < weight(other) {
            self.city_mut(c).trade_routes[i] = other;
        }
    }

    /// Does a caravan of `slot` coming into its own city `target` ask what to do (CIV.EXE
    /// 0x5c37)? Only when the city builds a wonder, or it is 10 or more squares from the
    /// caravan's home or on another continent; otherwise the caravan simply moves in.
    pub fn caravan_menu_wanted(&self, u: &super::Unit, target: CityId) -> bool {
        let city = self.city(target);
        if city.owner != u.owner {
            return false;
        }
        if matches!(city.production, Production::Wonder(_)) {
            return true;
        }
        let Some(home) = u.home.and_then(|h| self.city_opt(h)) else { return false };
        Map::dist(home.pos(), city.pos()) >= 10 || self.map.tile(home.x, home.y).continent != self.map.tile(city.x, city.y).continent
    }

    // ---- diplomats --------------------------------------------------------------------------

    /// Greyed lines of the diplomat's menu (`DIPLOMAT_MENU`) for a diplomat of `slot` at `city`
    /// (CIV.EXE o 0x1d575, mask ds:0xb276): the embassy when there is one already or the city
    /// is barbarian, stealing when this city was robbed before, inciting a city with a Palace,
    /// and meeting the king of the barbarians.
    pub fn diplomat_menu_disabled(&self, slot: Slot, city: CityId) -> [bool; 6] {
        let c = self.city(city);
        let barbarian = c.owner == 0;
        [self.player(slot).has_embassy(c.owner) || barbarian, false, c.tech_stolen, false, c.has_improvement(imp::PALACE), barbarian]
    }

    /// Price of inciting `city` to revolt (CIV.EXE o 0x1d9b7): `(owner's gold + 1000) /
    /// (distance to the owner's capital + 3) * size`, halved when the city is in disorder.
    /// None for the capital itself.
    pub fn incite_cost(&self, city: CityId) -> Option<i32> {
        let c = self.city(city);
        let dist = self.capital_distance(c.owner, c.x, c.y);
        if dist == 0 {
            return None;
        }
        let mut cost = (self.player(c.owner).gold + 1000) / (dist + 3) * c.size as i32;
        if c.disorder {
            cost /= 2;
        }
        Some(cost)
    }

    /// A diplomat next to a foreign city acts (CIV.EXE o 0x1d575). Stealing from a civilisation
    /// at peace breaks the treaty (the caller asks first, `senate_blocks_war` permitting), and so
    /// does inciting one unless it is a subversion at twice the price.
    pub fn diplomat_action(&mut self, id: UnitId, action: DiplomatAction, city: CityId) -> Result<DiplomatResult, RuleError> {
        let u = self.unit_opt(id).cloned().ok_or(RuleError::Other("No such unit."))?;
        if u.kind != unit_ids::DIPLOMAT {
            return Err(RuleError::Other("Only Diplomats can do that."));
        }
        let target = self.city_opt(city).cloned().ok_or(RuleError::Other("No such city."))?;
        if Map::dist((u.x, u.y), (target.x, target.y)) > 1 {
            return Err(RuleError::Other("The diplomat must be next to the city."));
        }
        let us = u.owner;
        let them = target.owner;
        if them == us {
            return Err(RuleError::Other("That is one of your own cities."));
        }
        let result = match action {
            DiplomatAction::Embassy => {
                self.establish_embassy(us, them);
                DiplomatResult::Embassy
            }
            DiplomatAction::Investigate => DiplomatResult::Investigated,
            DiplomatAction::StealTech => {
                if target.tech_stolen {
                    return Err(RuleError::Other("Technology was stolen from this city before."));
                }
                // the thief has passed senate_check_attack (the caller asks); the treaty ends,
                // with no vendetta (o 0x1d73b)
                self.clear_diplo_flag(us, them, super::diplomacy::flags::PEACE);
                let Some(a) = self.steal_advance(id, city) else { return Ok(DiplomatResult::NoTechnology) };
                DiplomatResult::Stole(a)
            }
            DiplomatAction::Sabotage => DiplomatResult::Sabotaged(self.sabotage_city(city)),
            DiplomatAction::InciteRevolt { subvert } => {
                let Some(cost) = self.incite_cost(city) else { return Err(RuleError::Other("A capital cannot be incited to revolt.")) };
                // subversion needs a treaty in the inciter's word (o 0x1d9f4)
                let at_peace = self.treaty(us, them);
                if subvert && !at_peace {
                    return Err(RuleError::Other("Only a city at peace can be subverted."));
                }
                let cost = if subvert { cost * 2 } else { cost };
                if self.player(us).gold < cost {
                    return Err(RuleError::NotEnoughGold);
                }
                // inciting a treaty partner's city ends the treaty and earns its vendetta; a
                // subversion keeps the peace (o 0x1daf5..0x1db10)
                if !subvert && self.treaty(them, us) {
                    self.clear_diplo_flag(them, us, super::diplomacy::flags::PEACE);
                    self.raise_flag(them, us, super::diplomacy::flags::VENDETTA);
                }
                self.player_mut(us).gold -= cost;
                self.remove_unit(id);
                self.transfer_city(city, us);
                self.refresh_vision(us);
                self.refresh_vision(them);
                self.city_lost_check(them, us);
                return Ok(DiplomatResult::Incited { cost });
            }
            DiplomatAction::MeetKing => {
                if them == 0 {
                    return Err(RuleError::Other("The barbarians have no king to meet."));
                }
                self.unit_mut(id).moves_left = 0;
                return Ok(DiplomatResult::MetKing);
            }
        };
        self.remove_unit(id);
        self.refresh_vision(us);
        Ok(result)
    }

    /// Steal Technology (CIV.EXE o 0x1d757): 72 tries from a random advance on, the first one
    /// the city's owner knows and the thief lacks (a barbarian thief never finds one); it is
    /// granted and the city marked as robbed. None when nothing was found.
    fn steal_advance(&mut self, id: UnitId, city: CityId) -> Option<AdvanceId> {
        let us = self.unit(id).owner;
        let them = self.city(city).owner;
        let start = self.rng.below(rules::advances::ADVANCE_COUNT as u32) as usize;
        let a = (0..rules::advances::ADVANCE_COUNT)
            .map(|i| ((start + i) % rules::advances::ADVANCE_COUNT) as AdvanceId)
            .find(|a| us != 0 && self.player(them).knows(*a) && !self.player(us).knows(*a))?;
        self.grant_advance(us, a, super::science::AdvanceSource::From(them));
        self.player_mut(us).stats.techs_stolen += 1;
        self.city_mut(city).tech_stolen = true;
        Some(a)
    }

    /// Industrial Sabotage (CIV.EXE o 0x1d84a): half of the time (or when nothing is stored) a
    /// random improvement other than the Palace, found within 98 draws (a 99th is drawn and
    /// thrown away, o 0x1d880); otherwise (or when none was found) the stored shields. Returns
    /// the improvement destroyed.
    fn sabotage_city(&mut self, city: CityId) -> Option<ImprovementId> {
        let target = self.city(city).clone();
        let mut destroyed = None;
        if self.rng.below(2) != 0 || target.shields == 0 {
            for n in 1..=99 {
                let i = self.rng.below(24) as ImprovementId + 1;
                if n == 99 {
                    break;
                }
                if i != imp::PALACE && target.has_improvement(i) {
                    destroyed = Some(i);
                    break;
                }
            }
        }
        match destroyed {
            Some(i) => self.city_mut(city).remove_improvement(i),
            None => self.city_mut(city).shields = 0,
        }
        destroyed
    }

    /// A computer Diplomat next to the foreign city `city` (CIV.EXE o 0x1d681): when the city
    /// is a human's who is at peace with the Diplomat's civilization the Diplomat simply
    /// leaves (it is deleted, None). Otherwise it always steals, without breaking any treaty;
    /// at a city robbed before, or when it finds nothing to steal, it sabotages instead. The
    /// Diplomat is used up either way. Everyone is told, whoever was involved: "PLURAL
    /// steal\nADVANCE." or "ITEM production sabotaged\nin CITY." / "BUILDING destroyed\nin
    /// CITY." (popups at (80,80), 0x1d817 / 0x1d9a3), as `DiplomatStole` / `CitySabotaged`.
    pub fn ai_diplomat_action(&mut self, id: UnitId, city: CityId) -> Option<DiplomatResult> {
        let u = self.unit_opt(id)?.clone();
        let target = self.city_opt(city)?.clone();
        let (us, them) = (u.owner, target.owner);
        if u.kind != unit_ids::DIPLOMAT || them == us || Map::dist(u.pos(), target.pos()) > 1 {
            return None;
        }
        if self.is_human(them) && self.treaty(them, us) {
            self.remove_unit(id);
            self.refresh_vision(us);
            return None;
        }
        let stolen = if target.tech_stolen { None } else { self.steal_advance(id, city) };
        let result = match stolen {
            Some(adv) => {
                self.push_event(GameEvent::DiplomatStole { by: us, from: them, adv, city });
                DiplomatResult::Stole(adv)
            }
            None => {
                let production = target.production.name().to_string();
                let improvement = self.sabotage_city(city);
                self.push_event(GameEvent::CitySabotaged { city, by: us, improvement, production });
                DiplomatResult::Sabotaged(improvement)
            }
        };
        self.remove_unit(id);
        self.refresh_vision(us);
        Some(result)
    }

    /// Where a computer civilization's new Diplomat goes (CIV.EXE 0xe12e): it is not set up in
    /// the city that built it but next to a human city. For every human city the unit of
    /// `slot` nearest to it is taken (0x1d43:0x16d); the score |distance − 3| (3 for a city
    /// robbed before) must be at most 3, the unit must stand on land on the city's continent;
    /// the lowest score wins, ties by a running random choice. Nothing while `slot` is at peace
    /// with the human. Returns the unit's square and the target city.
    pub fn ai_diplomat_site(&mut self, slot: Slot) -> Option<((i32, i32), CityId)> {
        let mut rng = self.rng.clone();
        let r = self.diplomat_site_with(slot, &mut |n| rng.below(n as u32) as i32);
        self.rng = rng;
        r
    }

    /// Is there an `ai_diplomat_site` for `slot` (without drawing random numbers)?
    pub fn ai_diplomat_site_exists(&self, slot: Slot) -> bool {
        self.diplomat_site_with(slot, &mut |_| 1).is_some()
    }

    fn diplomat_site_with(&self, slot: Slot, random: &mut dyn FnMut(i32) -> i32) -> Option<((i32, i32), CityId)> {
        let mut best: Option<((i32, i32), CityId)> = None;
        let mut best_score = 0x7fff;
        let mut ties = 0;
        for city in self.all_cities().filter(|c| self.is_human(c.owner)) {
            // the unit of `slot` closest to the city (the first one of equal distance)
            let Some(u) = self.all_units().filter(|u| u.owner == slot).min_by_key(|u| Map::civ_distance(u.pos(), city.pos())) else { continue };
            let score = if city.tech_stolen { 3 } else { (Map::civ_distance(u.pos(), city.pos()) - 3).abs() };
            if score > best_score {
                continue;
            }
            if score == best_score {
                ties += 1;
                if random(ties) != 0 {
                    continue;
                }
            }
            if self.map.is_ocean(u.x, u.y) || self.map.tile(u.x, u.y).continent != self.map.tile(city.x, city.y).continent {
                continue;
            }
            // (0xe287: a tie that is taken restarts the count at 1, a new best keeps it)
            if score == best_score {
                ties = 1;
            } else {
                best_score = score;
            }
            best = Some((u.pos(), city.id));
        }
        let humans_at_peace = (1..8).any(|h| self.is_human(h) && self.treaty(slot, h));
        if best_score > 3 || humans_at_peace {
            return None;
        }
        best
    }

    /// A city revolts to `to` (CIV.EXE o 0x1e06a, then 0x13d76 with `by_diplomat` set): the old
    /// owner's units next to it on land change sides, those homed there elsewhere are disbanded;
    /// the city is plundered, an advance is taken (`capture_steal`) and it loses one citizen
    /// (never the last) and its Palace, Temple, Courthouse and Cathedral (`capture_losses`).
    pub fn transfer_city(&mut self, c: CityId, to: Slot) {
        let from = self.city(c).owner;
        let (gold, adv) = self.revolt_city(c, to);
        self.push_event(GameEvent::CityCaptured { city: c, by: to, from, gold, adv });
    }

    /// The work of `transfer_city`, without the announcement; returns the plunder and the
    /// advance taken.
    fn revolt_city(&mut self, c: CityId, to: Slot) -> (i32, Option<AdvanceId>) {
        // stashed units change sides with the others (CIV.EXE would keep them in the record of
        // a city it can no longer let them out of)
        self.unstash_garrison(c);
        let (pos, from, size) = {
            let city = self.city(c);
            (city.pos(), city.owner, city.size)
        };
        // city_revolt's announcement (o 0x1e1d4), before capture_city's own message
        self.push_event(GameEvent::CityRebels { city: c, from, to });
        // the leaders remember, as for a city taken by force (capture_city 0x13da6, 0x13e5c)
        if to == self.human_slot && from != 0 {
            self.player_mut(from).last_contact = -2;
        }
        if from == self.human_slot && to != 0 {
            self.player_mut(to).last_contact = -1;
        }
        let gold = self.plunder_value(c);
        self.player_mut(from).gold -= gold;
        if to != 0 {
            self.player_mut(to).gold += gold;
        }
        // capture_city logs the city as taken (0x13f97) and drops the new owner's city-site
        // goals around it (0x13faf)
        self.replay_city(super::replay::kind::CAPTURE, to, c);
        self.remove_goals_near(to, super::goals::goal::SETTLE, pos.0, pos.1, 4);
        let kept = self.capture_losses(true);
        let mut gone = Vec::new();
        let mut gone_heading = Vec::new();
        for u in self.units.iter_mut().flatten().filter(|u| u.owner == from) {
            if Map::dist((u.x, u.y), pos) <= 1 && !self.map.is_ocean(u.x, u.y) {
                // deleted and re-created for the new owner (o 0x1de40 / 0x1de6e)
                self.players[from as usize].unit_counter[u.kind as usize % 28] -= 1;
                self.players[to as usize].unit_counter[u.kind as usize % 28] += 1;
                // re-created for the new owner by create_unit (0x8b51: no veteran status,
                // orders or goto), seen by the old owner and homed to the city (o 0x1e3af,
                // 0x1e3b6); its moves are left alone
                let fresh = super::Unit::new(u.id, to, u.kind, u.x, u.y, self.turn);
                *u = super::Unit { moves_left: u.moves_left, home: Some(c), seen_by: 1 << to | 1 << from, ..fresh };
                gone_heading.push(u.id);
            } else if u.home == Some(c) {
                gone.push(u.id);
            }
        }
        for id in gone {
            self.remove_unit(id);
        }
        for id in gone_heading {
            self.ai_memory.heading.remove(&id);
            self.barbarian_memory.heading.remove(&id);
        }
        // the trade routes stay, both ways (capture_city 0x13d76 leaves the route bytes)
        let unrest = self.captured_in_disorder(to);
        {
            let cm = self.city_mut(c);
            cm.owner = to;
            cm.size = if size > 1 { size - 1 } else { size };
            cm.disorder = unrest;
            cm.celebrating = false;
            cm.disorder_turns = unrest as u8;
            cm.auto_build = false;
            cm.garrison_out = false;
            cm.specialists.clear();
            cm.production = Production::Unit(unit_ids::MILITIA);
            cm.shields = 0;
            cm.caravan_shields = 0;
            cm.improvements &= kept;
        }
        // steal_advance after the messages (0x14232), as for a city taken by force
        let adv = self.capture_steal(to, from);
        self.players[to as usize].stats.cities_captured += 1;
        self.players[from as usize].stats.cities_lost += 1;
        self.auto_assign(c);
        self.rechoose_after_capture(to);
        (gold, adv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::Ai;
    use crate::game::GameSetup;
    use crate::rules::Difficulty;

    fn fixture() -> (Game, CityId) {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        (g, c)
    }

    #[test]
    fn city_grows_with_food() {
        let (mut g, c) = fixture();
        g.city_mut(c).food = g.food_box(c) - 1;
        let y = g.city_yield(c);
        assert!(y.food_surplus() >= 1, "{y:?}");
        // the store fills this turn; the city grows at the start of its next one (0xd187)
        g.process_city(c);
        assert_eq!(g.city(c).size, 1);
        assert!(g.city(c).food >= g.food_box(c));
        g.process_city(c);
        assert_eq!(g.city(c).size, 2);
        assert!(g.take_events().iter().any(|e| matches!(e, GameEvent::CityGrew(_))));
        let city = g.city(c);
        assert_eq!(city.workers() as usize + city.specialists.len(), 2);
    }

    #[test]
    fn famine_takes_settlers_then_citizens_and_a_last_citizen_starves() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        let (x, y) = g.city(c).pos();
        for id in g.units_of(slot) {
            g.remove_unit(id);
        }
        let settlers = g.add_unit(slot, unit_ids::SETTLERS, x + 1, y);
        g.unit_mut(settlers).home = Some(c);
        g.city_mut(c).size = 2;
        g.city_mut(c).food = -1;
        assert!(g.process_food(c));
        assert!(g.unit_opt(settlers).is_none());
        assert_eq!((g.city(c).size, g.city(c).food), (2, 0));
        assert!(g.take_events().iter().any(|e| matches!(e, GameEvent::FoodExhausted { settlers_lost: true, .. })));
        g.city_mut(c).food = -1;
        assert!(g.process_food(c));
        assert_eq!((g.city(c).size, g.city(c).food), (1, 0));
        assert!(g.take_events().iter().any(|e| matches!(e, GameEvent::FoodExhausted { settlers_lost: false, .. })));
        // size 1: destroy_city (0xd149), the units it supports with it
        let militia = g.add_unit(slot, unit_ids::MILITIA, x, y);
        g.unit_mut(militia).home = Some(c);
        g.city_mut(c).food = -1;
        assert!(!g.process_food(c));
        assert!(g.city_opt(c).is_none());
        assert!(g.unit_opt(militia).is_none());
    }

    #[test]
    fn a_city_without_aqueduct_grows_past_10_and_shrinks_back() {
        let (mut g, c) = fixture();
        g.city_mut(c).size = 10;
        g.city_mut(c).food = g.food_box(c);
        assert!(g.process_food(c));
        assert_eq!((g.city(c).size, g.city(c).food), (10, 0));
        assert!(g.take_events().contains(&GameEvent::AqueductNeeded(c)));
        // the Granary refills for the size it reached for a moment: 5 * 11 + 5
        g.city_mut(c).add_improvement(imp::GRANARY);
        g.city_mut(c).food = g.food_box(c);
        assert!(g.process_food(c));
        assert_eq!((g.city(c).size, g.city(c).food), (10, 60));
        g.city_mut(c).add_improvement(imp::AQUEDUCT);
        g.city_mut(c).food = g.food_box(c);
        assert!(g.process_food(c));
        assert_eq!(g.city(c).size, 11);
    }

    #[test]
    fn settlers_from_a_size_1_city_above_chieftain() {
        let mut g = Game::new(GameSetup { seed: 42, difficulty: Difficulty::Prince, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        let (x, y) = g.city(c).pos();
        let count = |g: &Game| g.units_of(slot).into_iter().filter(|id| g.unit(*id).kind == unit_ids::SETTLERS).count();
        let before = count(&g);
        // the only city keeps its citizen (0xdfcb)
        g.city_mut(c).shields = 30;
        g.complete_production(c, Production::Unit(unit_ids::SETTLERS));
        assert_eq!(g.city(c).size, 1);
        assert_eq!(count(&g), before + 1);
        // with a second city the last citizen leaves and the city is gone (0xdfec)
        let other = g.add_city(slot, "Other".into(), x + 6, y);
        g.city_mut(c).shields = 30;
        g.complete_production(c, Production::Unit(unit_ids::SETTLERS));
        assert!(g.city_opt(c).is_none());
        assert!(g.city_opt(other).is_some());
        // create_unit 0x8b51 homes them to the nearest city, the civ's other one
        let new = g.units_at(x, y).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        assert_eq!(g.unit(new).home, Some(other));
    }

    #[test]
    fn the_humans_store_empties_on_completion_the_computers_keeps_the_rest() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        g.set_production(c, Production::Unit(unit_ids::MILITIA)).unwrap();
        g.city_mut(c).shields = 17;
        g.complete_production(c, Production::Unit(unit_ids::MILITIA));
        assert_eq!(g.city(c).shields, 0);
        let (x, y) = g.city(c).pos();
        let ai = (1..8).find(|s| *s != slot && g.player(*s).in_game).unwrap();
        let theirs = g.add_city(ai, "Theirs".into(), x + 6, y);
        let cost = g.production_cost(theirs, Production::Unit(unit_ids::MILITIA));
        g.city_mut(theirs).shields = cost + 3;
        g.complete_production(theirs, Production::Unit(unit_ids::MILITIA));
        assert_eq!(g.city(theirs).shields, 3);
        // a wonder empties the computer's store too (0xe6f9)
        let wonder = Production::Wonder(won::COLOSSUS);
        g.city_mut(theirs).shields = g.production_cost(theirs, wonder) + 3;
        g.complete_production(theirs, wonder);
        assert_eq!(g.city(theirs).shields, 0);
    }

    #[test]
    fn disorder_keeps_shields_and_taxes_out_but_not_science() {
        let mut g = Game::new(GameSetup { seed: 42, difficulty: Difficulty::Emperor, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        let (x, y) = g.city(c).pos();
        for id in g.units_of(slot) {
            g.remove_unit(id);
        }
        g.player_mut(slot).tax = 50;
        g.player_mut(slot).lux = 0;
        g.player_mut(slot).researching = Some(adv::ALPHABET);
        g.city_mut(c).size = 6;
        g.fix_workers(c);
        // a taxman and a scientist bring 2 gold and 2 beakers whatever the trade
        let worked: Vec<u8> = (1..21u8).filter(|i| g.city(c).works(*i)).take(2).collect();
        for i in worked {
            g.toggle_tile(c, i);
        }
        g.set_specialist(c, 0, crate::game::Specialist::Taxman);
        g.set_specialist(c, 1, crate::game::Specialist::Scientist);
        g.set_production(c, Production::Improvement(imp::BARRACKS)).unwrap();
        // a unit beyond the free ones: the shields of a city in disorder still support it
        for _ in 0..7 {
            let id = g.add_unit(slot, unit_ids::SETTLERS, x, y);
            g.unit_mut(id).home = Some(c);
        }
        g.city_mut(c).food = 50;
        let t = g.city_yield_raw(c);
        assert!(g.city_happiness(c).disorder && t.sci > 0 && t.tax > 0 && t.shields >= 1, "{t:?}");
        g.city_mut(c).disorder = true;
        let (gold, beakers, units) = (g.player(slot).gold, g.player(slot).beakers, g.units_of(slot).len());
        g.process_city(c);
        assert_eq!(g.city(c).shields, 0);
        assert_eq!(g.player(slot).gold, gold);
        assert!(g.player(slot).beakers >= beakers + t.sci);
        assert_eq!(g.units_of(slot).len(), units);
        assert!(g.city(c).disorder);
    }

    #[test]
    fn a_celebrating_republic_grows_each_turn() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        g.player_mut(slot).govt = Govt::Republic;
        g.player_mut(slot).tax = 0;
        g.player_mut(slot).lux = 100;
        g.city_mut(c).size = 4;
        g.city_mut(c).food = 0;
        g.fix_workers(c);
        g.city_mut(c).add_wonder(won::SHAKESPEARES_THEATRE);
        g.wonders_built[won::SHAKESPEARES_THEATRE as usize] = Some(c);
        g.city_mut(c).add_wonder(won::HANGING_GARDENS);
        g.wonders_built[won::HANGING_GARDENS as usize] = Some(c);
        let h = g.city_happiness(c);
        let y = g.city_yield_raw(c);
        assert!(h.celebration && y.food_surplus() > 0, "{h:?} {y:?}");
        // the first turn of the celebration only starts it
        g.process_city(c);
        assert_eq!(g.city(c).size, 4);
        assert!(g.city(c).celebrating);
        g.city_mut(c).food = 0;
        g.process_city(c);
        assert_eq!(g.city(c).size, 5);
    }

    #[test]
    fn the_farthest_unit_goes_first_on_a_shortage() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        let (x, y) = g.city(c).pos();
        g.player_mut(slot).govt = Govt::Monarchy;
        for id in g.units_of(slot) {
            g.remove_unit(id);
        }
        // travel distance (0x1d43:0x27f): 3 + 1 / 2 = 3 for both; the first one goes
        let a = g.add_unit(slot, unit_ids::MILITIA, x + 3, y + 1);
        let b = g.add_unit(slot, unit_ids::MILITIA, x + 3, y);
        let near = g.add_unit(slot, unit_ids::MILITIA, x, y);
        for id in [a, b, near] {
            g.unit_mut(id).home = Some(c);
        }
        assert_eq!(g.farthest_supported_unit(c), Some(a));
        let shields = g.city_yield_raw(c).shields;
        g.disband_unsupported(c, shields);
        let left = [a, b, near].iter().filter(|id| g.unit_opt(**id).is_some()).count() as i32;
        assert_eq!(left, shields.min(3));
        assert!(g.unit_opt(near).is_some() || shields == 0);
    }

    #[test]
    fn militia_completes() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        let before = g.units_of(slot).len();
        g.city_mut(c).shields = 10;
        g.process_city(c);
        let after = g.units_of(slot);
        assert_eq!(after.len(), before + 1);
        let new = after.iter().find(|id| g.unit(**id).kind == unit_ids::MILITIA).unwrap();
        assert_eq!(g.unit(*new).home, Some(c));
        assert!(g.take_events().iter().any(|e| matches!(e, GameEvent::ItemBuilt { .. })));
        // Settlers blocked in a size-1 city
        g.set_production(c, Production::Unit(unit_ids::SETTLERS)).unwrap();
        g.city_mut(c).shields = 40;
        g.process_city(c);
        assert!(g.take_events().iter().any(|e| matches!(e, GameEvent::ProductionBlocked { .. })));
        assert_eq!(g.city(c).size, 1);
    }

    #[test]
    fn improvement_completion_asks_human() {
        let (mut g, c) = fixture();
        g.set_production(c, Production::Improvement(imp::BARRACKS)).unwrap();
        g.city_mut(c).shields = 40;
        g.process_city(c);
        assert!(g.city(c).has_improvement(imp::BARRACKS));
        let ev = g.take_events();
        assert!(ev.iter().any(|e| matches!(e, GameEvent::NeedsDecision(Decision::ProductionChoice { .. }))));
        // the city stays on the finished building (CHANGE flashes) and does not build it again
        assert_eq!(g.city(c).production, Production::Improvement(imp::BARRACKS));
        assert!(g.production_invalid(c));
        g.city_mut(c).shields = 40;
        g.process_city(c);
        assert!(!g.take_events().iter().any(|e| matches!(e, GameEvent::ItemBuilt { .. })));
        assert!(g.city(c).shields >= 40);
    }

    #[test]
    fn upkeep_shortage_disbands_and_bankruptcy_sells() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        let (x, y) = g.city(c).pos();
        // despotism: units up to the city's size are free; the others need shields the size-1
        // city cannot make
        for _ in 0..6 {
            let id = g.add_unit(slot, unit_ids::MILITIA, x, y);
            g.unit_mut(id).home = Some(c);
        }
        let y0 = g.city_yield(c);
        assert!(y0.shield_surplus() < 0);
        g.process_city(c);
        assert!(g.city_yield(c).shield_surplus() >= 0);
        assert!(g.take_events().iter().any(|e| matches!(e, GameEvent::UnitDisbanded { .. })));
        // bankruptcy: the building the treasury cannot pay for is sold at once for its cost
        g.player_mut(slot).tax = 0;
        g.player_mut(slot).lux = 0;
        g.city_mut(c).add_improvement(imp::CATHEDRAL);
        g.player_mut(slot).gold = 0;
        g.process_city(c);
        assert!(g.take_events().iter().any(|e| *e == GameEvent::CantMaintain { city: c, improvement: imp::CATHEDRAL }));
        assert!(!g.city(c).has_improvement(imp::CATHEDRAL));
        assert_eq!(g.player(slot).gold, 160);
    }

    #[test]
    fn only_the_human_pays_upkeep_and_barracks_get_dearer() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        g.city_mut(c).add_improvement(imp::TEMPLE);
        g.city_mut(c).add_improvement(imp::BARRACKS);
        g.setup.difficulty = crate::rules::Difficulty::Chieftain;
        assert_eq!(g.city_upkeep(c), 1, "Barracks free at Chieftain");
        g.setup.difficulty = crate::rules::Difficulty::Emperor;
        g.player_mut(slot).learn(adv::GUNPOWDER);
        g.player_mut(slot).learn(adv::COMBUSTION);
        assert_eq!(g.city_upkeep(c), 1 + 4);
        g.player_mut(slot).govt = Govt::Anarchy;
        assert_eq!(g.city_upkeep(c), 0, "no upkeep under Anarchy");
        g.player_mut(slot).govt = Govt::Despotism;
        g.player_mut(slot).human = false;
        assert_eq!(g.city_upkeep(c), 0, "computer players pay nothing");
    }

    #[test]
    fn pyramids_make_every_government_available() {
        let (mut g, c) = fixture();
        let slot = g.human_slot;
        assert_eq!(g.governments_available(slot), vec![Govt::Despotism]);
        g.city_mut(c).add_wonder(won::PYRAMIDS);
        g.wonders_built[won::PYRAMIDS as usize] = Some(c);
        assert_eq!(g.governments_available(slot).len(), 5);
    }

    /// Records which civilization plays at which turn.
    struct Recorder(Vec<(Slot, u32)>);

    impl AiDriver for Recorder {
        fn play_turn(&mut self, g: &mut Game, slot: Slot, _sink: &mut dyn FnMut(TurnStep)) {
            self.0.push((slot, g.turn));
        }
    }

    fn run(g: &mut Game, r: &mut TurnRunner, ai: &mut dyn AiDriver) {
        loop {
            match r.step(g, ai) {
                TurnStep::Done => break,
                TurnStep::Decision(_) => r.resolve(g, DecisionReply::Choice(0)),
                _ => {}
            }
        }
    }

    #[test]
    fn civilizations_play_in_slot_order_around_the_turn_counter() {
        // game_turn 0x2412: civ 0..7 at turn T, then the counter; the human moves inside the
        // loop, so the civilizations after it end turn T and those before it open turn T + 1
        let mut g = Game::new(GameSetup { seed: 42, num_civs: 7, human_civ: 4, ..Default::default() }, None);
        let h = g.human_slot;
        assert!(h > 1 && h < 7, "human slot {h}");
        let mut ai = Recorder(Vec::new());
        let mut r = g.take_opening().expect("a new game opens");
        run(&mut g, &mut r, &mut ai);
        assert!(g.take_opening().is_none());
        let before: Vec<Slot> = (0..h).filter(|s| *s == 0 || g.player(*s).alive).collect();
        let after: Vec<Slot> = (h + 1..8).filter(|s| g.player(*s).alive).collect();
        assert_eq!(ai.0, before.iter().map(|s| (*s, 0)).collect::<Vec<_>>());
        ai.0.clear();
        let mut r = g.end_turn();
        run(&mut g, &mut r, &mut ai);
        let want: Vec<(Slot, u32)> = after.iter().map(|s| (*s, 0)).chain(before.iter().map(|s| (*s, 1))).collect();
        assert_eq!(ai.0, want);
        assert_eq!(g.turn, 1);
    }

    #[test]
    fn the_humans_cities_are_processed_before_its_units_move() {
        // 0x250d before 0x25af: a city grows at the start of the human's turn, after the
        // computer players have moved
        let (mut g, c) = fixture();
        g.opening = false;
        g.city_mut(c).food = 1000;
        struct Sizes(CityId, Vec<u8>);
        impl AiDriver for Sizes {
            fn play_turn(&mut self, g: &mut Game, _slot: Slot, _sink: &mut dyn FnMut(TurnStep)) {
                self.1.push(g.city(self.0).size);
            }
        }
        let mut ai = Sizes(c, Vec::new());
        let mut r = g.end_turn();
        run(&mut g, &mut r, &mut ai);
        assert!(!ai.1.is_empty() && ai.1.iter().all(|s| *s == 1), "{:?}", ai.1);
        assert!(g.city(c).size > 1);
        assert_eq!(g.turn, 1);
    }

    #[test]
    fn no_unit_beyond_127_and_the_human_hears_it_once_a_turn() {
        // create_unit 0x8b62 / 0x8d24 (*UMAX, ds:0xd760); the shields are gone (0xdeb8, 0xe3bc)
        let (mut g, c) = fixture();
        let me = g.human_slot;
        let (x, y) = g.city(c).pos();
        while g.unit_room(me) {
            g.add_unit(me, unit_ids::MILITIA, x, y);
        }
        let before = g.units_of(me).len();
        g.city_mut(c).shields = 50;
        g.complete_production(c, Production::Unit(unit_ids::MILITIA));
        g.city_mut(c).shields = 50;
        g.complete_production(c, Production::Unit(unit_ids::MILITIA));
        assert_eq!(g.units_of(me).len(), before);
        assert_eq!(g.city(c).shields, 0);
        assert_eq!(g.take_events().iter().filter(|e| matches!(e, GameEvent::UnitLimit(_))).count(), 1);
    }

    #[test]
    fn gold_is_capped_at_30000_after_the_cities() {
        // 0x2515: every civ, after update_civ_tax_rates
        let (mut g, _c) = fixture();
        let me = g.human_slot;
        g.player_mut(me).gold = 31000;
        g.process_player_cities(me);
        assert_eq!(g.player(me).gold, 30000);
    }

    #[test]
    fn computer_rates_follow_the_civilized_trait_and_the_cities() {
        // 0xa699..0xa760 in tenths: lux from the last rates +-1, sci = civilized + (10 - lux) / 2
        let (mut g, _c) = fixture();
        let ai = g.alive_slots().into_iter().find(|s| *s != g.human_slot).unwrap();
        let civilized = g.player(ai).civ().civilized as i32;
        // civ_init (o 0x4e0b): sci civilized + 3, tax 9 - sci, lux 1; the human 5/0/5
        assert_eq!((g.player(ai).tax as i32, g.player(ai).lux), ((6 - civilized) * 10, 10));
        assert_eq!((g.human().tax, g.human().lux), (50, 0));
        g.set_turn(5);
        g.player_mut(ai).gold = 0;
        g.player_mut(ai).forget(adv::ROBOTICS);
        g.player_mut(ai).tax = 40;
        g.player_mut(ai).lux = 20;
        // a riot: lux 2 + 1
        g.ai_tax_rates(ai, 2);
        let sci = civilized + (10 - 3) / 2;
        assert_eq!((g.player(ai).lux, g.player(ai).tax as i32), (30, (10 - sci - 3) * 10));
        // every fourth turn without trouble, while sci + tax < 8: one less
        g.set_turn(8);
        g.ai_tax_rates(ai, 0);
        assert_eq!(g.player(ai).lux, 20);
        // rich: one more for science; Robotics: none
        g.player_mut(ai).gold = 500;
        g.set_turn(9);
        g.ai_tax_rates(ai, 1);
        assert_eq!(100 - g.player(ai).tax as i32 - g.player(ai).lux as i32, (civilized + 4 + 1) * 10);
        g.player_mut(ai).learn(adv::ROBOTICS);
        g.ai_tax_rates(ai, 1);
        assert_eq!(g.player(ai).tax + g.player(ai).lux, 100);
        // set_government (0x15031): taxes 4 - government / 2
        g.player_mut(ai).learn(adv::MONARCHY);
        g.choose_government(ai, Govt::Monarchy);
        assert_eq!(g.player(ai).tax, 30);
    }

    #[test]
    fn treasury_warning_when_gold_would_run_out_in_ten_turns() {
        // 0x268a: > 1 city, gold < 100, (gold - start) * 10 + gold < 0
        let (mut g, c) = fixture();
        let me = g.human_slot;
        let (x, y) = g.city(c).pos();
        let s = g.add_unit(me, unit_ids::SETTLERS, x + 3, y);
        g.found_city(s).unwrap();
        g.take_events();
        let warned = |g: &mut Game| g.take_events().iter().any(|e| matches!(e, GameEvent::LowFunds { .. }));
        g.player_mut(me).gold = 90;
        g.treasury_warning();
        assert!(!warned(&mut g), "no mark yet");
        g.player_mut(me).gold = 80;
        g.treasury_warning();
        assert!(warned(&mut g), "-10 * 10 + 80 < 0");
        g.player_mut(me).gold = 75;
        g.treasury_warning();
        assert!(!warned(&mut g), "-5 * 10 + 75 >= 0");
        g.player_mut(me).gold = 10;
        g.city_mut(c).owner = 0;
        g.treasury_warning();
        assert!(!warned(&mut g), "one city left");
    }

    #[test]
    fn palace_grows_with_the_score() {
        // 0x3a67: (sum i * (7 - d)) / 2 + 1
        assert_eq!((palace_threshold(1, 0), palace_threshold(2, 0), palace_threshold(3, 4)), (4, 11, 10));
        let (mut g, c) = fixture();
        let me = g.human_slot;
        assert!(g.city(c).has_improvement(imp::PALACE));
        g.take_events();
        let need = palace_threshold(g.player(me).palace_level as i32 + 1, g.setup.difficulty as i32);
        let score = g.score(me).total;
        g.palace_growth_check();
        assert_eq!(g.player(me).palace_level > 0, score >= need);
        g.player_mut(me).palace_level = 0;
        g.player_mut(me).peace_turns = 30;
        g.set_turn(250);
        assert!(g.score(me).total >= need);
        g.palace_growth_check();
        assert_eq!(g.player(me).palace_level, 1);
        assert!(g.take_events().contains(&GameEvent::PalaceAddition(me)));
        g.options.palace = false;
        g.player_mut(me).palace_level = 0;
        g.palace_growth_check();
        assert_eq!(g.player(me).palace_level, 0);
    }

    #[test]
    fn anarchy_ends_on_every_fourth_turn_or_with_the_pyramids() {
        // 0x1495a: turn & 3 == 0, or the Pyramids; a computer player gets Despotism
        let (mut g, c) = fixture();
        let slot = g.human_slot;
        let asked = |g: &mut Game| g.take_events().iter().any(|e| matches!(e, GameEvent::NeedsDecision(Decision::ChooseGovernment { .. })));
        g.player_mut(slot).govt = Govt::Anarchy;
        g.take_events();
        g.set_turn(13);
        g.anarchy_check(slot);
        assert!(!asked(&mut g));
        g.set_turn(16);
        g.anarchy_check(slot);
        assert!(asked(&mut g));
        // the collapse flag holds the human for one more such turn
        g.player_mut(slot).anarchy_turns = 1;
        g.anarchy_check(slot);
        assert!(!asked(&mut g));
        assert_eq!(g.player(slot).anarchy_turns, 0);
        g.anarchy_check(slot);
        assert!(asked(&mut g));
        // the Pyramids end it at the next turn, not at the revolution
        g.city_mut(c).add_wonder(won::PYRAMIDS);
        g.wonders_built[won::PYRAMIDS as usize] = Some(c);
        g.player_mut(slot).govt = Govt::Monarchy;
        g.set_turn(17);
        g.revolution(slot);
        assert!(!asked(&mut g));
        g.set_turn(18);
        g.anarchy_check(slot);
        assert!(asked(&mut g));
        let ai = g.alive_slots().into_iter().find(|s| *s != slot).unwrap();
        g.player_mut(ai).govt = Govt::Anarchy;
        g.set_turn(20);
        g.anarchy_check(ai);
        assert_eq!(g.player(ai).govt, Govt::Despotism);
    }

    #[test]
    fn democracy_falls_in_the_city_loop() {
        // 0x127ba: the second turn of disorder; the human's flag is set (0x12820)
        let (mut g, c) = fixture();
        let slot = g.human_slot;
        g.player_mut(slot).govt = Govt::Democracy;
        g.city_mut(c).disorder_turns = 1;
        g.city_mut(c).size = 8;
        g.player_mut(slot).lux = 0;
        g.player_mut(slot).tax = 0;
        g.take_events();
        g.process_city(c);
        assert!(g.city(c).disorder);
        assert_eq!(g.player(slot).govt, Govt::Anarchy);
        assert_eq!(g.player(slot).anarchy_turns, 1);
        assert!(g.take_events().contains(&GameEvent::GovernmentCollapsed { slot, city: c }));
    }

    #[test]
    fn revolution_and_government() {
        let (mut g, _c) = fixture();
        let slot = g.human_slot;
        g.player_mut(slot).learn(adv::MONARCHY);
        g.set_turn(10);
        g.revolution(slot);
        assert_eq!(g.player(slot).govt, Govt::Anarchy);
        for t in 11..=12 {
            g.set_turn(t);
            g.process_player_turn_start(slot);
        }
        let ev = g.take_events();
        let opts = ev.iter().find_map(|e| match e {
            GameEvent::NeedsDecision(Decision::ChooseGovernment { options, .. }) => Some(options.clone()),
            _ => None,
        }).expect("government decision");
        assert!(opts.contains(&Govt::Monarchy));
        let i = opts.iter().position(|g| *g == Govt::Monarchy).unwrap();
        g.apply_decision(&Decision::ChooseGovernment { slot, options: opts }, DecisionReply::Choice(i));
        assert_eq!(g.player(slot).govt, Govt::Monarchy);
        assert!(g.player(slot).tax <= 70);
    }

    #[test]
    fn caravan_arrival_and_trade_route_rules() {
        use crate::game::{Dir8, MoveOutcome};
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        for y in 2..20 {
            for x in 2..40 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = crate::rules::Terrain::Grassland;
                t.flags = 0;
            }
        }
        g.map.compute_continents();
        let ids: Vec<_> = g.all_units().map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        let ids: Vec<_> = g.all_cities().map(|c| c.id).collect();
        for c in ids {
            g.cities[c as usize] = None;
        }
        let slot = g.human_slot;
        let home = g.add_city(slot, "Home".into(), 5, 5);
        let near = g.add_city(slot, "Near".into(), 10, 5);
        let far = g.add_city(slot, "Far".into(), 20, 5);
        let caravan = |g: &mut Game, x| {
            let id = g.add_unit(slot, unit_ids::CARAVAN, x, 5);
            g.unit_mut(id).home = Some(home);
            id
        };
        // CIV.EXE 0x5c37: a city within 10 squares asks only while it builds a wonder
        let a = caravan(&mut g, 9);
        assert_eq!(g.try_move(a, Dir8::E), Ok(MoveOutcome::Moved));
        g.city_mut(near).production = Production::Wonder(won::PYRAMIDS);
        let b = caravan(&mut g, 9);
        assert_eq!(g.try_move(b, Dir8::E), Ok(MoveOutcome::CaravanArrived(near)));
        assert_eq!(g.unit(b).pos(), (9, 5), "the caravan waits for the answer");
        assert_eq!(g.move_on(b, Dir8::E), Ok(MoveOutcome::Moved));
        // 10+ squares from home: the trade route is offered
        let c = caravan(&mut g, 19);
        assert_eq!(g.try_move(c, Dir8::E), Ok(MoveOutcome::CaravanArrived(far)));
        let expected = (15 + 10) * (g.visible_trade(home) + g.visible_trade(far)) / 24 / 2 / 2;
        assert_eq!(g.trade_route_revenue(slot, home, far), expected);
        let (gold, beakers) = (g.player(slot).gold, g.player(slot).beakers);
        assert!(g.caravan_action(c, CaravanAction::TradeRoute(far)).is_ok());
        assert_eq!(g.player(slot).gold, gold + expected);
        assert_eq!(g.player(slot).beakers, beakers + expected);
        assert_eq!(g.city(home).trade_routes, vec![far]);
        assert!(g.city(far).trade_routes.is_empty());
        // Railroad, then Flight, each take a third off
        g.player_mut(slot).learn(adv::RAILROAD);
        let v = 25 * (g.visible_trade(home) + g.visible_trade(far)) / 24 / 4;
        assert_eq!(g.trade_route_revenue(slot, home, far), v - v / 3);
        // a full list only takes a stronger route
        g.city_mut(home).trade_routes = vec![far, far, far];
        g.add_trade_route(home, near);
        let weaker = g.visible_trade(near) <= g.visible_trade(far);
        assert_eq!(g.city(home).trade_routes.contains(&near), !weaker);
    }

    #[test]
    fn runs_twenty_turns_resolving_decisions() {
        for diff in [Difficulty::Chieftain, Difficulty::Emperor] {
            let mut g = Game::new(GameSetup { seed: 42, difficulty: diff, ..Default::default() }, None);
            let slot = g.human_slot;
            let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
            g.found_city(settler).unwrap();
            let mut ai = Ai::new(7);
            for _ in 0..20 {
                let mut r = g.end_turn();
                let mut guard = 0;
                loop {
                    match r.step(&mut g, &mut ai) {
                        TurnStep::Done => break,
                        TurnStep::Decision(_) => r.resolve(&mut g, DecisionReply::Choice(0)),
                        _ => {}
                    }
                    guard += 1;
                    assert!(guard < 10_000, "runner did not finish");
                }
                assert!(r.is_done());
            }
            assert_eq!(g.turn, 20);
            assert!(g.score(slot).total >= 0);
            assert!(g.player(slot).researching.is_some() || g.player(slot).beakers == 0);
        }
    }

    #[test]
    fn caravan_trade_route() {
        let (mut g, c) = fixture();
        let slot = g.city(c).owner;
        // second city far away, on a coast off the polar rows (it works the sea's trade)
        let (cx, cy) = g.city(c).pos();
        let mut other = None;
        for y in 2..rules::MAP_H as i32 - 2 {
            for x in 0..rules::MAP_W as i32 {
                if g.map.is_land(x, y) && g.map.is_coastal(x, y) && Map::dist((x, y), (cx, cy)) >= 12 && Map::neighbours8(x, y).all(|(nx, ny)| g.city_at(nx, ny).is_none()) {
                    other = Some((x, y));
                    break;
                }
            }
            if other.is_some() {
                break;
            }
        }
        let (ox, oy) = other.expect("a far away land tile");
        let c2 = g.add_city(slot, "Other".into(), ox, oy);
        let car = g.add_unit(slot, unit_ids::CARAVAN, ox, oy);
        g.unit_mut(car).home = Some(c);
        let gold = g.player(slot).gold;
        assert!(g.caravan_action(car, CaravanAction::TradeRoute(c2)).is_ok());
        assert!(g.unit_opt(car).is_none());
        assert!(g.player(slot).gold > gold);
        assert_eq!(g.city(c).trade_routes, vec![c2]);
        assert!(g.city(c2).trade_routes.is_empty(), "only the home city gets the route");
        // CIV.EXE 0xf141: a route between own cities adds (partner + own trade + 4) / 16
        let t = g.raw_trade(c);
        assert_eq!(g.city_yield(c).trade, t + (g.visible_trade(c2) + t + 4) / 16);
    }
}

#[cfg(test)]
mod diplomat_tests {
    use super::*;
    use crate::game::{GameSetup, Relation};

    /// A flat grassland arena (x, y in 5..30) on one continent, emptied of units; the human
    /// and two computer slots `a` and `b`, all at war.
    fn arena() -> (Game, Slot, Slot, Slot) {
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        let h = g.human_slot;
        let others: Vec<Slot> = (1..8).filter(|s| *s != h).take(2).collect();
        let (a, b) = (others[0], others[1]);
        for s in [a, b] {
            g.players[s as usize].in_game = true;
            g.players[s as usize].alive = true;
        }
        let ids: Vec<UnitId> = g.all_units().map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        for y in 5..30 {
            for x in 5..30 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Grassland;
                t.flags = 0;
                t.continent = 1;
            }
        }
        g.set_relation(h, a, Relation::War);
        g.set_relation(h, b, Relation::War);
        g.set_relation(a, b, Relation::War);
        (g, h, a, b)
    }

    /// A city of `owner` at (20, 20) knowing everything `thief` knows plus the Alphabet.
    fn steal_target(g: &mut Game, owner: Slot, thief: Slot) -> CityId {
        let c = g.add_city(owner, "Target".into(), 20, 20);
        g.player_mut(thief).advances &= !(1u128 << adv::ALPHABET);
        let known = g.player(thief).known_advances();
        for a in known {
            g.player_mut(owner).learn(a);
        }
        g.player_mut(owner).learn(adv::ALPHABET);
        c
    }

    #[test]
    fn ai_diplomat_steals_at_peace_without_breaking_it_then_sabotages() {
        let (mut g, _h, a, b) = arena();
        g.set_relation(a, b, Relation::Peace);
        let c = steal_target(&mut g, b, a);
        let d = g.add_unit(a, unit_ids::DIPLOMAT, 19, 20);
        assert_eq!(g.ai_diplomat_action(d, c), Some(DiplomatResult::Stole(adv::ALPHABET)));
        assert!(g.player(a).knows(adv::ALPHABET) && g.city(c).tech_stolen);
        assert_eq!(g.relation(a, b), Relation::Peace, "an AI theft breaks no treaty");
        assert!(g.unit_opt(d).is_none());
        assert!(g.take_events().iter().any(|e| matches!(e, GameEvent::DiplomatStole { by, adv: x, .. } if *by == a && *x == adv::ALPHABET)));
        // the city was robbed: the next one sabotages
        g.city_mut(c).shields = 25;
        g.city_mut(c).add_improvement(imp::BARRACKS);
        let d2 = g.add_unit(a, unit_ids::DIPLOMAT, 19, 20);
        let Some(DiplomatResult::Sabotaged(what)) = g.ai_diplomat_action(d2, c) else { panic!("no sabotage") };
        match what {
            Some(i) => assert!(i == imp::BARRACKS && !g.city(c).has_improvement(imp::BARRACKS)),
            None => assert_eq!(g.city(c).shields, 0),
        }
        assert!(g.unit_opt(d2).is_none());
        assert!(g.take_events().iter().any(|e| matches!(e, GameEvent::CitySabotaged { city, by, .. } if *city == c && *by == a)));
    }

    #[test]
    fn ai_diplomat_sabotages_when_nothing_can_be_stolen() {
        let (mut g, _h, a, b) = arena();
        let c = g.add_city(b, "Poor".into(), 20, 20);
        g.player_mut(b).advances = 0;
        g.city_mut(c).shields = 10;
        let d = g.add_unit(a, unit_ids::DIPLOMAT, 19, 20);
        assert_eq!(g.ai_diplomat_action(d, c), Some(DiplomatResult::Sabotaged(None)));
        assert_eq!(g.city(c).shields, 0);
        assert!(!g.city(c).tech_stolen);
    }

    #[test]
    fn ai_diplomat_leaves_a_human_at_peace_alone() {
        let (mut g, h, a, _b) = arena();
        let c = steal_target(&mut g, h, a);
        g.set_relation(h, a, Relation::Peace);
        let d = g.add_unit(a, unit_ids::DIPLOMAT, 19, 20);
        assert_eq!(g.ai_diplomat_action(d, c), None);
        assert!(g.unit_opt(d).is_none(), "the Diplomat is deleted");
        assert!(!g.player(a).knows(adv::ALPHABET) && !g.city(c).tech_stolen);
        // at war it steals from the human too
        g.set_relation(h, a, Relation::War);
        let d = g.add_unit(a, unit_ids::DIPLOMAT, 19, 20);
        assert_eq!(g.ai_diplomat_action(d, c), Some(DiplomatResult::Stole(adv::ALPHABET)));
    }

    #[test]
    fn ai_diplomat_is_built_next_to_a_human_city() {
        let (mut g, h, a, _b) = arena();
        let target = g.add_city(h, "Target".into(), 20, 20);
        let home = g.add_city(a, "Home".into(), 8, 8);
        let warrior = g.add_unit(a, unit_ids::MILITIA, 16, 21);
        assert_eq!(Map::civ_distance((16, 21), (20, 20)), 4);
        assert_eq!(g.ai_diplomat_site(a), Some(((16, 21), target)));
        // too far: |distance - 3| above 3
        g.unit_mut(warrior).x = 9;
        assert!(!g.ai_diplomat_site_exists(a));
        g.city_mut(home).production = Production::Unit(unit_ids::DIPLOMAT);
        let cost = g.production_cost(home, Production::Unit(unit_ids::DIPLOMAT));
        g.city_mut(home).shields = cost;
        g.complete_production(home, Production::Unit(unit_ids::DIPLOMAT));
        assert_eq!(g.city(home).shields, cost, "the shields are given back");
        assert!(!g.all_units().any(|u| u.kind == unit_ids::DIPLOMAT));
        // and the city chooses again, as after any unit (0xe3b1)
        let advice = g.ai_rechoose(home);
        assert_eq!(g.city(home).production, advice);
        g.city_mut(home).production = Production::Unit(unit_ids::DIPLOMAT);
        // near enough, but at peace with the human: none either
        g.unit_mut(warrior).x = 16;
        g.set_relation(h, a, Relation::Peace);
        assert!(!g.ai_diplomat_site_exists(a));
        g.set_relation(h, a, Relation::War);
        g.complete_production(home, Production::Unit(unit_ids::DIPLOMAT));
        let d = g.all_units().find(|u| u.kind == unit_ids::DIPLOMAT).unwrap();
        assert_eq!((d.pos(), d.goto, d.home), ((16, 21), Some((20, 20)), Some(home)));
        assert_eq!(g.city(home).shields, 0);
        let advice = g.ai_rechoose(home);
        assert_eq!(g.city(home).production, advice);
    }

    #[test]
    fn ai_diplomat_heads_for_the_nearest_city_or_disbands() {
        let (mut g, _h, a, b) = arena();
        g.add_city(a, "Own".into(), 10, 10);
        let foreign = g.add_city(b, "Foreign".into(), 20, 10);
        let d = g.add_unit(a, unit_ids::DIPLOMAT, 12, 10);
        crate::ai::unit_ai::act(&mut g, d, &mut |_| {});
        assert!(g.unit_opt(d).is_none(), "nearest city is its own: disbanded");
        let d = g.add_unit(a, unit_ids::DIPLOMAT, 17, 10);
        crate::ai::unit_ai::act(&mut g, d, &mut |_| {});
        let u = g.unit(d);
        assert_eq!(u.goto, Some(g.city(foreign).pos()));
        assert!(Map::dist(u.pos(), g.city(foreign).pos()) <= 1);
    }
}
