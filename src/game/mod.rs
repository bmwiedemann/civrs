//! Game state and rules. Pure data + logic, no I/O, deterministic given the seed.
//!
//! Module ownership: `mod.rs` holds the `Game` struct, construction and queries. Each other
//! module adds `impl Game` blocks for its area (economy, happiness, movement, orders, combat,
//! turn processing, diplomacy, barbarians, pollution, spaceship, score).

pub mod barbarians;
pub mod city;
pub mod combat;
pub mod diplomacy;
pub mod economy;
pub mod events;
pub mod goals;
pub mod goto;
pub mod happiness;
pub mod hints;
pub mod map;
pub mod mapgen;
pub mod movement;
pub mod orders;
pub mod palace;
pub mod player;
pub mod pollution;
pub mod production;
pub mod replay;
pub mod rng;
pub mod science;
pub mod score;
pub mod spaceship;
pub mod turn;
pub mod unit;

pub use city::{City, Production, Specialist};
pub use events::{Decision, DecisionReply, EndReason, GameEvent, HutOutcome};
pub use map::{tile_flags, CityId, Dir8, Map, Slot, Tile, UnitId, WorldParams};
pub use player::{Player, Relation, Spaceship};
pub use rng::Rng;
pub use unit::{Unit, UnitOrder};

use crate::assets::EarthMap;
use crate::rules::{self, Difficulty, Terrain, UnitTypeId};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Rule violations, mapped 1:1 to ERROR.TXT sections where one exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleError {
    Zoc,
    Trireme,
    Fuel,
    Amphib,
    Fighter,
    Occupy,
    Pillage,
    Settlers,
    NoWater,
    NoMine,
    NoIrr,
    AddCity,
    NotEnoughGold,
    NoMoves,
    /// Generic refusal with a short message.
    Other(&'static str),
}

impl RuleError {
    /// ERROR.TXT section (None for errors without a text).
    pub fn section(&self) -> Option<&'static str> {
        Some(match self {
            RuleError::Zoc => "ZOC",
            RuleError::Trireme => "TRIREME",
            RuleError::Fuel => "FUEL",
            RuleError::Amphib => "AMPHIB",
            RuleError::Fighter => "FIGHTER",
            RuleError::Occupy => "OCCUPY",
            RuleError::Pillage => "PILLAGE",
            RuleError::Settlers => "SETTLERS",
            RuleError::NoWater => "NOWATER",
            RuleError::NoMine => "NOMINE",
            RuleError::NoIrr => "NOIRR",
            RuleError::AddCity => "ADDCITY",
            _ => return None,
        })
    }
    pub fn message(&self) -> &'static str {
        match self {
            RuleError::NotEnoughGold => "Not enough gold.",
            RuleError::NoMoves => "No movement points left.",
            RuleError::Other(s) => s,
            _ => "",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorldSource {
    Random(WorldParams),
    Earth,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GameSetup {
    pub difficulty: Difficulty,
    /// Number of civilisations including the human (3..=7).
    pub num_civs: u8,
    /// Civ id of the human player (1..=15, not 8).
    pub human_civ: u8,
    pub leader_name: String,
    pub world: WorldSource,
    pub seed: u64,
    /// The plural typed for the human's tribe after Esc at "Pick your tribe..." (only used when
    /// the game is created).
    #[serde(skip)]
    pub tribe_name: Option<String>,
}

impl Default for GameSetup {
    fn default() -> Self {
        GameSetup {
            difficulty: Difficulty::Chieftain,
            num_civs: 7,
            human_civ: 1,
            leader_name: String::new(),
            world: WorldSource::Random(WorldParams::normal()),
            seed: 1,
            tribe_name: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Options {
    pub autosave: bool,
    pub animations: bool,
    pub instant_advice: bool,
    pub sound: bool,
    pub end_turn_confirm: bool,
    pub enemy_moves: bool,
    pub civilopedia_text: bool,
    pub palace: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            autosave: true,
            animations: true,
            instant_advice: true,
            sound: true,
            end_turn_confirm: false,
            enemy_moves: true,
            civilopedia_text: true,
            palace: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Game {
    pub map: Map,
    pub units: Vec<Option<Unit>>,
    pub cities: Vec<Option<City>>,
    /// Indexed by slot 0..7 (0 = barbarians).
    pub players: Vec<Player>,
    pub turn: u32,
    pub human_slot: Slot,
    pub active_unit: Option<UnitId>,
    pub rng: Rng,
    pub setup: GameSetup,
    pub options: Options,
    /// City that built each wonder (index = WonderId); `WONDER_LOST` once that city is gone.
    pub wonders_built: [Option<CityId>; 22],
    pub polluted_tiles: u16,
    pub global_warming_count: u8,
    pub ended: Option<EndReason>,
    /// Scoring stopped (player chose to keep playing after the end).
    pub scoring_over: bool,
    /// The *FAME question for the current ending is out (asked once per ending).
    #[serde(skip)]
    pub keep_playing_asked: bool,
    /// Per-slot visibility bitmap (bit per slot) recomputed by `refresh_vision`.
    pub vision: Vec<u8>,
    #[serde(default)]
    pub events: VecDeque<GameEvent>,
    /// Manhattan Project built by anyone.
    pub nukes_enabled: bool,
    /// Apollo Program built by anyone.
    pub apollo_built: bool,
    /// Units on this tile are being observed by the human (for "enemy moves" animation).
    pub last_combat: Option<((i32, i32), (i32, i32), bool)>,
    /// Instant-advice hints already shown (bit = index in `hints::HINTS`); stored after the
    /// game body in version-2 saves.
    #[serde(skip)]
    pub hints_shown: u32,
    /// The calendar year ds:0xb220, advanced with the turn by `advance_calendar`; stored after
    /// the game body in version-6 saves (older saves take `rules::calendar_year(turn)`).
    #[serde(skip)]
    pub year: i32,
    /// Barbarian headings and the last-occupant map layer (only the layer is saved, in a trailer).
    #[serde(skip)]
    pub barbarian_memory: barbarians::BarbarianMemory,
    /// The computer players' goal lists, continent policies and unit bytes civrs's records
    /// lack (`goals::AiMemory`); the persistent goals and policies go into the version-9 save
    /// trailer, the rest is not saved.
    #[serde(skip)]
    pub ai_memory: goals::AiMemory,
    /// Computer players (bit per slot) whose audience with the human is queued but not over:
    /// CIV.EXE holds the audience while their units move, civrs after their turn, so until
    /// then their units keep the peace with the human. Not saved.
    #[serde(skip)]
    pub audience_pending: u8,
    /// ds:0x3934 between audiences: -1 (none, the initial value), else 0 after a
    /// `diplomacy_close` outside an audience (see `diplo_ai::open`). Not saved.
    #[serde(skip, default = "no_audience")]
    pub diplo_latch: i8,
    /// The human answered "Break treaty." for an attack on (or entering the land of) this
    /// civilization: the next move goes ahead (CIV.EXE 0x1a3b4 returned 1). Not saved.
    #[serde(skip)]
    pub treaty_break_ok: Option<Slot>,
    /// The human has confirmed an attack below full strength (`MoveOutcome::WeakAttack`).
    #[serde(skip)]
    pub weak_attack_ok: bool,
    /// The human's gold when the last *TAXES check ran (`treasury_warning`; CIV.EXE keeps the
    /// gold of the `game_turn` start at [bp-0x1a]); none after a load. Not saved.
    #[serde(skip)]
    pub taxes_mark: Option<i32>,
    /// The global warming counter ds:0xb1ea (0..99): one step a turn towards twice the
    /// polluted squares minus four per warming so far (`global_warming_check`); stored after
    /// the game body in version-9 saves.
    #[serde(skip)]
    pub warming_counter: u16,
    /// The first discoverer of each advance, far 0x26f1:[0x4b0 + 2 * adv] & 7 (0 = none yet):
    /// set by `grant_advance` after turn 0 (0xc743) and kept when that civ is destroyed; stored
    /// after the game body in version-15 saves.
    #[serde(skip)]
    pub first_discoverer: Vec<u8>,
    /// The historian's turn ds:0x1c26 (80 + random(50) at the start, then 20 + random(40)
    /// after each report, 0x28de); stored after the game body in version-17 saves.
    #[serde(skip)]
    pub historian_timer: u32,
    /// The replay log and the power history (`replay::History`); stored after the game body
    /// in version-18 saves.
    #[serde(skip)]
    pub history: replay::History,
    /// ds:0x70d8: set while an attack is resolved, so `remove_unit` counts the dead in
    /// `Player::units_lost_by_type` (delete_unit 0x8d71). Not saved.
    #[serde(skip)]
    pub counting_losses: bool,
    /// The research catch-up bonus ds:0xd2de of this turn (`update_research_catch_up`); not
    /// saved, recomputed on loading.
    #[serde(skip)]
    pub catch_up: i32,
    /// Free advances of Darwin's Voyage the human gets once the research menu is answered.
    /// Not saved.
    #[serde(skip)]
    pub free_research: u8,
    /// A new game whose turn 0 has not yet run the civilizations before the human
    /// (`TurnRunner::opening`). Not saved: a saved game is in the human's turn.
    #[serde(skip)]
    pub opening: bool,
}

/// `wonders_built` entry of a wonder whose city was destroyed: built, owned by no one, listed
/// as "(Destroyed)" (CIV.EXE stores city 0x80). No city has this id.
pub const WONDER_LOST: CityId = CityId::MAX;

fn no_audience() -> i8 {
    -1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveOutcome {
    Moved,
    /// Attacked an enemy unit; `won` from the mover's view.
    Attacked { won: bool, at: (i32, i32), defender: UnitTypeId },
    CapturedCity(CityId),
    DestroyedCity,
    EnteredHut,
    /// Boarded a ship / landed on a carrier.
    Boarded(UnitId),
    /// A non-combat unit entered a foreign city (caravan/diplomat): the UI shows a menu.
    EnteredForeignCity(CityId),
    /// A human's caravan reached one of its own cities where it may trade or help a wonder
    /// (`Game::caravan_menu_wanted`); it has not moved: the UI asks, and `move_on` enters.
    CaravanArrived(CityId),
    /// Move not possible (no error text).
    Blocked,
    /// A human's Diplomat met this lone foreign unit: the UI offers to bribe it
    /// (`Game::bribe_cost`, `Game::bribe_unit`) and moves on after a bribe.
    BribeOffer(UnitId),
    /// The human's unit would attack, take a city of or enter the land of this treaty partner:
    /// "We have signed a peace treaty with the X! Cancel action. / Break treaty." (CIV.EXE
    /// 0x1a3b4); after "Break treaty." (and no Senate) the UI sets `Game::treaty_break_ok`
    /// and moves again. The unit has not moved.
    BreakTreaty(Slot),
    /// The human's unit would attack with these thirds of a move: "Attack at N/3 strength?
    /// Cancel / Attack" (CIV.EXE 0x5fc4, menu at (100,16)); after "Attack" the UI sets
    /// `Game::weak_attack_ok` and moves again. The unit has not moved.
    WeakAttack(u8),
}

impl Game {
    /// Create a new game: generate the world, place civilisations and starting units.
    ///
    /// As CIV.EXE: the world generator's random numbers (`MsRand`) go on into the new-game
    /// setup (o 0x04175: the historian timer draws random(50)) and `civ_init` (o 0x0493c) for
    /// slots 1..7 in turn: a computer civilization draws its tribe (random(16) until it is one
    /// of the slot's two) and then its start square (`mapgen::start_square`). "N
    /// Civilizations" lets slots 1..N play (ds:0x6b8a = 0xff >> (7 - N)); CIV.EXE only offers
    /// those slots' tribes, civrs lets the human's slot take the place of the highest one.
    /// CIV.EXE draws no random number between `srand` and the world generator, so a game seed
    /// below 0x8000 makes the same world and starts as CIV.EXE does with that clock seed
    /// (checked in DOSBox, see the `mapgen` and `new_game_tests` tests).
    pub fn new(setup: GameSetup, earth: Option<&EarthMap>) -> Game {
        let rng = Rng::new(setup.seed);
        let seed32 = (setup.seed & 0xFFFF_FFFF) as u32 ^ (setup.seed >> 32) as u32;
        let world = match (&setup.world, earth) {
            (WorldSource::Earth, Some(em)) => mapgen::earth(em, seed32),
            (WorldSource::Random(p), _) => mapgen::generate(*p, seed32),
            (WorldSource::Earth, None) => mapgen::generate(WorldParams::normal(), seed32),
        };
        let mut wr = world.rng;
        let historian = wr.random(50) as u32;
        let human_slot = rules::slot_for_civ(setup.human_civ);
        let n = setup.num_civs.clamp(3, 7);
        let mut active: Vec<u8> = (1..=n).collect();
        if !active.contains(&human_slot) {
            active[n as usize - 1] = human_slot;
        }
        let mut unit0 = [(0i32, 0i32); 8];
        let mut slots: Vec<(u8, u8, (i32, i32))> = Vec::new();
        for s in 1..8u8 {
            if !active.contains(&s) {
                continue;
            }
            let human = s == human_slot;
            let civ = if human {
                setup.human_civ
            } else {
                loop {
                    let r = wr.random(16) as u8;
                    if r & 7 == s {
                        break r;
                    }
                }
            };
            if let Some(p) = mapgen::start_square(&world.map, &world.sites, &mut wr, &unit0[1..s as usize], human, civ) {
                unit0[s as usize] = p;
                slots.push((s, civ, p));
            }
        }
        let mut players: Vec<Player> = (0..8u8).map(|s| {
            let mut p = Player::new(s, s, false);
            p.in_game = s == 0;
            p
        }).collect();
        for (s, civ, _) in &slots {
            let human = *s == human_slot;
            let mut p = Player::new(*s, *civ, human);
            p.in_game = true;
            if human && !setup.leader_name.is_empty() {
                p.leader = setup.leader_name.clone();
            }
            if let Some(t) = setup.tribe_name.as_ref().filter(|t| human && !t.is_empty()) {
                p.tribe_names = Some(player::tribe_names(t));
            }
            players[*s as usize] = p;
        }
        let map = world.map.clone();
        let mut game = Game {
            map,
            units: Vec::new(),
            cities: Vec::new(),
            players,
            turn: 0,
            human_slot,
            active_unit: None,
            rng,
            setup: setup.clone(),
            options: Options::default(),
            wonders_built: [None; 22],
            polluted_tiles: 0,
            global_warming_count: 0,
            ended: None,
            scoring_over: false,
            keep_playing_asked: false,
            vision: vec![0; rules::MAP_W * rules::MAP_H],
            events: VecDeque::new(),
            nukes_enabled: false,
            apollo_built: false,
            last_combat: None,
            hints_shown: 0,
            year: rules::calendar_year(0),
            barbarian_memory: Default::default(),
            ai_memory: Default::default(),
            audience_pending: 0,
            diplo_latch: -1,
            treaty_break_ok: None,
            weak_attack_ok: false,
            taxes_mark: None,
            warming_counter: 0,
            first_discoverer: vec![0; 72],
            historian_timer: 80 + historian,
            history: replay::History::default(),
            counting_losses: false,
            catch_up: 0,
            free_research: 0,
            opening: true,
        };
        game.place_starting_units(&slots, &world, &mut wr);
        game.refresh_all_vision();
        game.active_unit = game.next_active_unit();
        game
    }

    /// The start units (`civ_init` o 0x0493c: one Settlers on the start square) and the
    /// make-up of `start_report` (o 0x0515b, `mapgen::start_bonuses`): an extra Settlers for a
    /// start more than 3 points worse than the best and one free advance per point still
    /// missing (`choose_research(civ, 0)` 0xa5e:0x156a, which grants at once at turn 0), civ by
    /// civ. The draws continue CIV.EXE's random stream `wr` after the start squares, so the
    /// advances match the original's for the same seed. Every civ starts without gold, the
    /// human with 50 at Chieftain (o 0x0476d).
    fn place_starting_units(&mut self, slots: &[(u8, u8, (i32, i32))], world: &mapgen::World, wr: &mut rng::MsRand) {
        for &(slot, _, (x, y)) in slots {
            self.add_unit(slot, rules::unit_ids::SETTLERS, x, y);
            self.players[slot as usize].gold = 0;
            self.map.explore(slot, x, y, 1);
            // civ_init ends by zeroing the unit counters and losses (o5 0x4fc2..0x4fcb)
            self.players[slot as usize].unit_counter = [0; 28];
            self.players[slot as usize].units_lost_by_type = [0; 28];
            if !self.players[slot as usize].human {
                self.ai_initial_rates(slot);
            }
        }
        let starts: Vec<(Slot, (i32, i32))> = slots.iter().map(|&(s, _, p)| (s, p)).collect();
        let bonus = mapgen::start_bonuses(&self.map, &world.land_sizes, &starts);
        for (&(slot, (x, y)), b) in starts.iter().zip(&bonus) {
            if b.settlers {
                self.add_unit(slot, rules::unit_ids::SETTLERS, x, y);
            }
            for _ in 0..b.advances {
                // random(4 * value) of the MS C library: signed, so a negative value never wins
                if let Some(a) = crate::ai::tech_ai::draw_by(self, slot, None, |n| wr.random(n)) {
                    self.grant_advance(slot, a, science::AdvanceSource::Discovered);
                }
            }
        }
        if self.setup.difficulty == Difficulty::Chieftain {
            let h = self.human_slot as usize;
            self.players[h].gold = 50;
        }
    }

    // ---- queries -------------------------------------------------------------------------

    /// The current year (ds:0xb220).
    pub fn year(&self) -> i32 {
        self.year
    }

    /// The year the coming turn will have (0x26ed): one year per turn from 1750 on once any
    /// civ's spaceship is in flight.
    pub fn next_year(&self) -> i32 {
        let launched = (1..8u8).any(|s| self.player(s).spaceship.launched());
        rules::advance_year(self.year, launched)
    }

    /// Turn counter and year advance at the end of a turn (0x26ed..0x2769). Reaching 1 AD
    /// doubles every civ's beakers (0x273f): from then on an advance costs twice as many.
    pub fn advance_calendar(&mut self) {
        let next = self.next_year();
        if self.year < 0 && next >= 0 {
            for p in self.players.iter_mut() {
                p.beakers *= 2;
            }
        }
        self.turn += 1;
        self.year = next;
    }

    /// Put the game at `turn` with the year of the launch-free calendar (tests and tools).
    pub fn set_turn(&mut self, turn: u32) {
        self.turn = turn;
        self.year = rules::calendar_year(turn);
    }

    pub fn year_text(&self) -> String {
        rules::year_text(self.year())
    }

    pub fn human(&self) -> &Player {
        &self.players[self.human_slot as usize]
    }

    pub fn human_mut(&mut self) -> &mut Player {
        let s = self.human_slot as usize;
        &mut self.players[s]
    }

    pub fn player(&self, slot: Slot) -> &Player {
        &self.players[slot as usize]
    }

    pub fn player_mut(&mut self, slot: Slot) -> &mut Player {
        &mut self.players[slot as usize]
    }

    pub fn difficulty(&self) -> Difficulty {
        self.setup.difficulty
    }

    /// Slots of civilisations in the game (excluding barbarians), alive or not.
    pub fn civ_slots(&self) -> Vec<Slot> {
        (1..8u8).filter(|s| self.players[*s as usize].in_game).collect()
    }

    pub fn alive_slots(&self) -> Vec<Slot> {
        (1..8u8).filter(|s| self.players[*s as usize].in_game && self.players[*s as usize].alive).collect()
    }

    pub fn unit(&self, id: UnitId) -> &Unit {
        self.units[id as usize].as_ref().expect("dead unit")
    }

    pub fn unit_mut(&mut self, id: UnitId) -> &mut Unit {
        self.units[id as usize].as_mut().expect("dead unit")
    }

    pub fn unit_opt(&self, id: UnitId) -> Option<&Unit> {
        self.units.get(id as usize).and_then(|u| u.as_ref())
    }

    pub fn city(&self, id: CityId) -> &City {
        self.cities[id as usize].as_ref().expect("dead city")
    }

    pub fn city_mut(&mut self, id: CityId) -> &mut City {
        self.cities[id as usize].as_mut().expect("dead city")
    }

    pub fn city_opt(&self, id: CityId) -> Option<&City> {
        self.cities.get(id as usize).and_then(|c| c.as_ref())
    }

    pub fn all_units(&self) -> impl Iterator<Item = &Unit> {
        self.units.iter().filter_map(|u| u.as_ref())
    }

    pub fn all_cities(&self) -> impl Iterator<Item = &City> {
        self.cities.iter().filter_map(|c| c.as_ref())
    }

    pub fn units_of(&self, slot: Slot) -> Vec<UnitId> {
        self.all_units().filter(|u| u.owner == slot).map(|u| u.id).collect()
    }

    pub fn cities_of(&self, slot: Slot) -> Vec<CityId> {
        self.all_cities().filter(|c| c.owner == slot).map(|c| c.id).collect()
    }

    pub fn units_at(&self, x: i32, y: i32) -> Vec<UnitId> {
        let x = Map::wrap_x(x);
        self.all_units().filter(|u| u.x == x && u.y == y).map(|u| u.id).collect()
    }

    pub fn city_at(&self, x: i32, y: i32) -> Option<CityId> {
        self.map.get(x, y).and_then(|t| t.city)
    }

    /// Owner of the units/city on a tile, if any.
    pub fn owner_at(&self, x: i32, y: i32) -> Option<Slot> {
        if let Some(c) = self.city_at(x, y) {
            return Some(self.city(c).owner);
        }
        let x = Map::wrap_x(x);
        self.all_units().find(|u| u.x == x && u.y == y).map(|u| u.owner)
    }

    /// The defender of a tile.
    pub fn best_defender(&self, x: i32, y: i32, attacker: Option<&Unit>) -> Option<UnitId> {
        // move_units picks it with best_defender_stack (CIV.EXE 0x111e): the highest
        // `defence_value`, the first of equals, else the square's first unit; the attacker,
        // walls and fortresses play no part
        let _ = attacker;
        let here = self.units_at(x, y);
        let mut best: Option<(UnitId, i32)> = None;
        for &id in &here {
            if let Some(d) = self.defence_value(id) {
                if best.map(|(_, bd)| d > bd).unwrap_or(true) {
                    best = Some((id, d));
                }
            }
        }
        best.map(|(id, _)| id).or(here.first().copied())
    }

    /// Units or a city at (x, y) whose owner `slot` may fight (`Game::may_fight`).
    pub fn is_enemy_at(&self, slot: Slot, x: i32, y: i32) -> bool {
        match self.owner_at(x, y) {
            Some(o) => self.may_fight(slot, o),
            None => false,
        }
    }

    /// `Game::hostile`, except that a computer player keeps the peace with the human while an
    /// audience between them is queued (`audience_pending`).
    pub fn may_fight(&self, slot: Slot, o: Slot) -> bool {
        self.hostile(slot, o) && !(o == self.human_slot && slot != 0 && self.audience_pending & (1 << slot) != 0)
    }

    pub fn foreign_at(&self, slot: Slot, x: i32, y: i32) -> bool {
        matches!(self.owner_at(x, y), Some(o) if o != slot)
    }

    pub fn explored(&self, slot: Slot, x: i32, y: i32) -> bool {
        self.map.get(x, y).map(|t| t.explored_by(slot)).unwrap_or(false)
    }

    /// Tile currently in sight of the slot's units or cities.
    pub fn visible_to(&self, slot: Slot, x: i32, y: i32) -> bool {
        Map::in_bounds(x, y) && self.vision[Map::idx(x, y)] & (1 << slot) != 0
    }

    pub fn refresh_vision(&mut self, slot: Slot) {
        let bit = 1u8 << slot;
        for v in self.vision.iter_mut() {
            *v &= !bit;
        }
        // (x, y, radius, outer ring on ocean only)
        let mut spots: Vec<(i32, i32, i32, bool)> = Vec::new();
        for u in self.all_units() {
            if u.owner == slot {
                spots.push((u.x, u.y, u.sight_range(), u.is_sea()));
            }
        }
        for c in self.all_cities() {
            if c.owner == slot {
                spots.push((c.x, c.y, 1, false));
            }
        }
        for (x, y, r, sea) in spots {
            for yy in (y - r)..=(y + r) {
                for xx in (x - r)..=(x + r) {
                    let outer = (xx - x).abs() > 1 || (yy - y).abs() > 1;
                    if Map::in_bounds(xx, yy) && !(sea && outer && !self.map.tile(xx, yy).is_ocean()) {
                        self.vision[Map::idx(xx, yy)] |= bit;
                        self.map.tiles[Map::idx(xx, yy)].explored |= bit;
                    }
                }
            }
        }
    }

    pub fn refresh_all_vision(&mut self) {
        for s in 0..8u8 {
            self.refresh_vision(s);
        }
    }

    // ---- mutation helpers ------------------------------------------------------------------

    pub fn add_unit(&mut self, owner: Slot, kind: UnitTypeId, x: i32, y: i32) -> UnitId {
        let id = self.units.iter().position(|u| u.is_none()).unwrap_or(self.units.len()) as UnitId;
        let mut u = Unit::new(id, owner, kind, x, y, self.turn);
        // create_unit (CIV.EXE 0x8b51) homes the unit to the nearest city when that city is
        // the owner's (0x8c0c), else to none
        u.home = self.nearest_city_exe(x, y).0.filter(|c| self.city(*c).owner == owner);
        if id as usize == self.units.len() {
            self.units.push(Some(u));
        } else {
            self.units[id as usize] = Some(u);
        }
        self.players[owner as usize].stats.units_built += 1;
        self.players[owner as usize].unit_counter[kind as usize % 28] += 1;
        self.note_occupant(owner, x, y);
        // create_unit 0x8b51 starts the unit without a heading
        self.ai_memory.heading.remove(&id);
        self.barbarian_memory.heading.remove(&id);
        id
    }

    pub fn remove_unit(&mut self, id: UnitId) {
        // units carried by this one are lost too
        let carried: Vec<UnitId> = self.all_units().filter(|u| u.carried_by == Some(id)).map(|u| u.id).collect();
        for c in carried.into_iter().chain([id]) {
            if let Some(u) = self.units[c as usize].take() {
                let p = &mut self.players[u.owner as usize];
                p.unit_counter[u.kind as usize % 28] -= 1;
                if self.counting_losses {
                    p.units_lost_by_type[u.kind as usize % 28] += 1;
                }
            }
        }
        if self.active_unit == Some(id) {
            self.active_unit = None;
        }
    }

    pub fn push_event(&mut self, e: GameEvent) {
        let (before, after) = self.hints_for_event(&e);
        self.events.extend(before);
        self.events.push_back(e);
        self.events.extend(after);
    }

    pub fn take_events(&mut self) -> Vec<GameEvent> {
        self.events.drain(..).collect()
    }

    /// Next city name for a civilisation (found_city o 0x1b3c5): the first name of the
    /// 256-name table (16 per tribe 1..7, 9..15, then 32 spare ones) never given to a city
    /// in this game, looking in the tribe's own 16, then the spare ones, then the whole table
    /// from the start. CIV.EXE marks a name used for good (es:0x540[name]), so a destroyed
    /// city's name is not given again; civrs takes the names the replay has logged.
    pub fn next_city_name(&mut self, slot: Slot) -> String {
        let civ = self.players[slot as usize].civ as usize;
        let mut used: Vec<String> = self.all_cities().map(|c| c.name.clone()).collect();
        used.extend(self.history.names.iter().cloned());
        let list = &rules::CIVS[civ].cities;
        let table: Vec<&str> = (1..16).filter(|t| *t != 8).flat_map(|t| rules::CIVS[t].cities.iter().copied()).chain(rules::SPARE_CITY_NAMES.iter().copied()).collect();
        let own = if civ == 0 || civ == 8 { 224 } else { 16 * (if civ > 7 { civ - 2 } else { civ - 1 }) };
        for i in (own..own + 16).chain(224..256).chain(0..256) {
            if !used.iter().any(|u| u == table[i]) {
                return table[i].to_string();
            }
        }
        // generate "<Name> II", "<Name> III"...
        let mut k = 2;
        loop {
            for n in list.iter() {
                let cand = format!("{n} {}", roman(k));
                if !used.iter().any(|u| *u == cand) {
                    return cand;
                }
            }
            k += 1;
        }
    }

    pub fn add_city(&mut self, owner: Slot, name: String, x: i32, y: i32) -> CityId {
        let id = self.cities.iter().position(|c| c.is_none()).unwrap_or(self.cities.len()) as CityId;
        let first_city = self.cities_of(owner).is_empty();
        let production = self.first_production(owner);
        let mut c = City {
            id,
            owner,
            founder: owner,
            name,
            x,
            y,
            size: 1,
            food: 0,
            shields: 0,
            production,
            improvements: 0,
            wonders: 0,
            worked: 0,
            specialists: Vec::new(),
            trade_routes: Vec::new(),
            disorder: false,
            disorder_turns: 0,
            celebrating: false,
            bought: false,
            tech_stolen: false,
            auto_build: false,
            founded_turn: self.turn,
            pollution_pending: 0,
            caravan_shields: 0,
            garrison: city::no_garrison(),
            garrison_out: false,
        };
        if first_city {
            c.add_improvement(rules::improvement_ids::PALACE);
            self.players[owner as usize].capital = Some(id);
        }
        if id as usize == self.cities.len() {
            self.cities.push(Some(c));
        } else {
            self.cities[id as usize] = Some(c);
        }
        if let Some(t) = self.map.get_mut(x, y) {
            t.city = Some(id);
            t.set(tile_flags::HUT, false);
            // "A city founded in a Fortress square destroys the Fortress"; the mine goes too and
            // the square gets a road (CIV.EXE o 0x1b4f2)
            t.set(tile_flags::FORTRESS, false);
            t.set(tile_flags::MINE, false);
            t.set(tile_flags::ROAD, true);
            // and irrigation where it adds food without changing the terrain (o 0x1b525:
            // terrain table ds:0x44a < -1), which stays when the city is gone
            if matches!(t.terrain.info().irrigate, rules::Terraform::Bonus(_)) {
                t.set(tile_flags::IRRIGATION, true);
            }
        }
        self.auto_assign(id);
        // a city sees only its 3x3 block (14_city_screen / 26_turn_12 captures)
        self.map.explore(owner, x, y, 1);
        // found_city o 0x1b4e4
        self.replay_city(replay::kind::CITY, owner, id);
        id
    }

    /// Remove a city from the map (destroyed).
    pub fn remove_city(&mut self, id: CityId) {
        // destroy_city 0xa793 logs the city as gone
        self.replay_city(replay::kind::CITY, 0xff, id);
        let (x, y, owner) = {
            let c = self.city(id);
            (c.x, c.y, c.owner)
        };
        for t in self.map.tiles.iter_mut() {
            if t.worked_by == Some(id) {
                t.worked_by = None;
            }
        }
        if let Some(t) = self.map.get_mut(x, y) {
            t.city = None;
        }
        // destroy_city (CIV.EXE 0xa76a) disbands the owner's units homed here (0x7e6:0xf0c)
        // and the ships and aircraft on the square (kill_nonland_cb 0x949f); others homed
        // here lose their home
        let doomed: Vec<UnitId> = self
            .all_units()
            .filter(|u| u.owner == owner && (u.home == Some(id) || ((u.x, u.y) == (x, y) && !u.is_land())))
            .map(|u| u.id)
            .collect();
        for u in doomed {
            self.remove_unit(u);
        }
        for u in self.units.iter_mut().flatten() {
            if u.home == Some(id) {
                u.home = None;
            }
        }
        // every route to it is cleared (destroy_city 0xa812), so a new city in the slot has none
        for c in self.cities.iter_mut().flatten() {
            c.trade_routes.retain(|r| *r != id);
        }
        // its wonders are lost for good (destroy_city sets ds:0x6ea8[w] = 0x80)
        for w in self.wonders_built.iter_mut() {
            if *w == Some(id) {
                *w = Some(WONDER_LOST);
            }
        }
        if self.players[owner as usize].capital == Some(id) {
            self.players[owner as usize].capital = None;
        }
        self.cities[id as usize] = None;
    }

    /// Cycle to the next unit of the human player that wants orders.
    pub fn next_active_unit(&mut self) -> Option<UnitId> {
        let slot = self.human_slot;
        let n = self.units.len();
        if n == 0 {
            return None;
        }
        let start = self.active_unit.map(|a| a as usize + 1).unwrap_or(0);
        for k in 0..n {
            let i = (start + k) % n;
            if let Some(u) = &self.units[i] {
                if u.owner == slot && u.moves_left > 0 && u.order == UnitOrder::None && u.carried_by.map(|c| self.unit_opt(c).is_none()).unwrap_or(true) {
                    return Some(u.id);
                }
            }
        }
        None
    }

    /// Any human unit still able to act this turn?
    pub fn human_units_pending(&self) -> bool {
        self.all_units().any(|u| u.owner == self.human_slot && u.moves_left > 0 && u.order == UnitOrder::None)
    }

    pub fn terrain_name(&self, x: i32, y: i32) -> String {
        let t = self.map.tile(x, y);
        let info = t.terrain.info();
        match (&info.special, t.special) {
            (Some(sp), true) => format!("{} ({})", info.name, sp.name),
            _ => info.name.to_string(),
        }
    }

    pub fn is_human(&self, slot: Slot) -> bool {
        slot == self.human_slot
    }

    pub fn total_population(&self, slot: Slot) -> i64 {
        self.all_cities().filter(|c| c.owner == slot).map(|c| c.population()).sum()
    }

    /// Civilisation display name for a slot ("Romans").
    pub fn civ_name(&self, slot: Slot) -> &str {
        self.player(slot).plural()
    }
}

pub fn roman(n: u32) -> String {
    let mut s = String::new();
    let mut n = n;
    for (v, r) in [(10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")] {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// Terrain helper used in several modules.
pub fn terrain_is_land(t: Terrain) -> bool {
    t != Terrain::Ocean
}

#[cfg(test)]
mod calendar_tests {
    use super::*;

    #[test]
    fn calendar_skips_year_zero_and_doubles_the_beakers() {
        let mut g = Game::new(GameSetup { seed: 9, ..Default::default() }, None);
        assert_eq!((g.turn, g.year()), (0, -4000));
        g.set_turn(199);
        assert_eq!(g.year(), -20);
        g.players[2].beakers = 7;
        g.advance_calendar();
        assert_eq!((g.turn, g.year()), (200, 1));
        assert_eq!(g.players[2].beakers, 14);
        g.advance_calendar();
        assert_eq!((g.turn, g.year()), (201, 20));
        assert_eq!(g.players[2].beakers, 14);
    }

    #[test]
    fn a_launched_ship_slows_the_calendar_to_a_year_a_turn() {
        let mut g = Game::new(GameSetup { seed: 9, ..Default::default() }, None);
        g.set_turn(355);
        assert_eq!(g.year(), 1760);
        g.advance_calendar();
        assert_eq!(g.year(), 1762);
        g.players[3].spaceship.launched_turn = Some(g.turn);
        g.advance_calendar();
        assert_eq!((g.turn, g.year()), (357, 1763));
        // a cancelled ship (capital captured) gives the two-year steps back
        g.players[3].spaceship = Spaceship::default();
        g.advance_calendar();
        assert_eq!(g.year(), 1765);
    }

    #[test]
    fn retirement_warning_and_end_of_game_years() {
        let mut g = Game::new(GameSetup { seed: 9, difficulty: Difficulty::Chieftain, ..Default::default() }, None);
        let h = g.human_slot;
        g.set_turn(400);
        g.year = 2079;
        g.process_world();
        assert!(g.take_events().iter().any(|e| *e == GameEvent::RetirementWarning(h)));
        assert_eq!(g.ended, None);
        g.year = 2098;
        g.process_world();
        assert_eq!(g.ended, None);
        // the game ends as the calendar reaches 2100 - 20 * difficulty (0x2a98)
        g.year = 2099;
        g.process_world();
        assert_eq!(g.ended, Some(EndReason::TimeLimit));
    }

    #[test]
    fn keep_playing_is_asked_per_ending_and_a_second_ending_has_no_scores() {
        let mut g = Game::new(GameSetup { seed: 9, ..Default::default() }, None);
        let asked = |ev: &[GameEvent]| ev.iter().any(|e| matches!(e, GameEvent::NeedsDecision(Decision::KeepPlaying { .. })));
        g.end_game(EndReason::Spaceship(g.human_slot));
        g.process_world();
        let ev = g.take_events();
        assert!(ev.contains(&GameEvent::GameEnded(EndReason::Spaceship(g.human_slot))) && asked(&ev));
        g.process_world();
        assert!(!asked(&g.take_events()));
        // "Yes, keep playing."; then a conquest: no scores again and no question (0x2c09)
        g.apply_decision(&Decision::KeepPlaying { reason: EndReason::Spaceship(g.human_slot) }, DecisionReply::Yes);
        g.end_game(EndReason::Conquest);
        g.process_world();
        let ev = g.take_events();
        assert!(ev.contains(&GameEvent::GameEndedAgain(EndReason::Conquest)) && !asked(&ev));
    }
}

#[cfg(test)]
mod new_game_tests {
    use super::*;

    #[test]
    fn city_names_are_given_once_in_table_order() {
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        let s = g.human_slot;
        let list = rules::CIVS[g.player(s).civ as usize].cities;
        let first = g.next_city_name(s);
        assert_eq!(first, list[0]);
        {
            let t = g.map.get_mut(20, 20).unwrap();
            t.terrain = rules::Terrain::Plains;
            t.flags = tile_flags::POLLUTION | tile_flags::MINE;
        }
        let c = g.add_city(s, first, 20, 20);
        g.remove_city(c);
        // the founding left road and irrigation, took the mine and kept the pollution
        let t = g.map.tile(20, 20);
        assert_eq!(t.flags & (tile_flags::ROAD | tile_flags::IRRIGATION | tile_flags::MINE | tile_flags::POLLUTION), tile_flags::ROAD | tile_flags::IRRIGATION | tile_flags::POLLUTION);
        // a destroyed city's name stays used (es:0x540, o 0x1b3c5)
        assert_eq!(g.next_city_name(s), list[1]);
        for (i, n) in list.iter().enumerate().skip(1) {
            g.add_city(s, n.to_string(), 2 + 2 * i as i32, 30);
        }
        assert_eq!(g.next_city_name(s), rules::SPARE_CITY_NAMES[0]);
    }

    #[test]
    fn a_destroyed_citys_wonders_stay_built_and_owned_by_no_one() {
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        let me = g.human_slot;
        g.player_mut(me).learn(rules::advance_ids::BRONZE_WORKING);
        let c = g.add_city(me, "Rome".into(), 20, 20);
        let again = g.add_city(me, "Veii".into(), 30, 20);
        assert!(g.can_build_wonder(again, rules::wonder_ids::COLOSSUS));
        g.wonders_built[rules::wonder_ids::COLOSSUS as usize] = Some(c);
        g.remove_city(c);
        assert_eq!(g.wonders_built[rules::wonder_ids::COLOSSUS as usize], Some(WONDER_LOST));
        assert!(!g.owner_has_wonder(me, rules::wonder_ids::COLOSSUS));
        assert!(!g.can_build_wonder(again, rules::wonder_ids::COLOSSUS));
    }

    #[test]
    fn a_new_game_starts_where_civ_exe_does() {
        // the DOSBox game of `mapgen::tests::matches_a_world_made_by_civ_exe`
        let g = Game::new(GameSetup { seed: 0x5fa8, ..Default::default() }, None);
        let mut starts: Vec<(Slot, u8, (i32, i32))> = Vec::new();
        for u in g.all_units() {
            assert_eq!(u.kind, rules::unit_ids::SETTLERS);
            starts.push((u.owner, g.player(u.owner).civ, u.pos()));
        }
        starts.sort();
        let want: Vec<(Slot, u8, (i32, i32))> =
            [(70, 34), (10, 23), (41, 35), (32, 37), (15, 32), (50, 39), (34, 27)].iter().enumerate().map(|(i, p)| (i as u8 + 1, i as u8 + 1, *p)).collect();
        assert_eq!(starts, want);
        assert_eq!(g.player(1).gold, 50);
        assert!((2..8).all(|s| g.player(s).gold == 0));
        assert_eq!((1..8).map(|s| g.player(s).advance_count()).collect::<Vec<_>>(), vec![1, 0, 1, 1, 1, 2, 3]);
    }

    #[test]
    fn start_advances_follow_civ_exe_random_stream() {
        // DOSBox run 1 (reference/original-screens/16_advisor_F6.png): clock seed 4763, Romans
        // at Chieftain against 6 civilizations; start_report's choose_research draws continue
        // the generator's stream
        let g = Game::new(GameSetup { seed: 4763, ..Default::default() }, None);
        let h = g.player(g.human_slot);
        assert_eq!(h.advance_count(), 2);
        assert!(h.knows(rules::advance_ids::BRONZE_WORKING) && h.knows(rules::advance_ids::CEREMONIAL_BURIAL));
    }

    #[test]
    fn the_earth_starts_where_civ_exe_does() {
        // a DOSBox EARTH game: clock seed 0x68e4, 7 civilizations at Chieftain, English
        let Ok(assets) = crate::assets::AssetDir::locate(None) else { return };
        let Ok(em) = assets.load_earth() else { return };
        let g = Game::new(GameSetup { seed: 0x68e4, human_civ: 14, world: WorldSource::Earth, ..Default::default() }, Some(&em));
        // the polar rows with the Tundra squares the generator drew (o 0x09bca)
        let rows = [
            "f7ffff7f7f7ff7ffffffffff77fffffffffffff7ffffffffff7fff7fffffffff7ffff7ff77fff7ff",
            "11711111711171711117111711111111711111171111711111111717117711111117711111117117",
            "71711111111117711771111711171711111111771111177111111177111111711111111111111171",
            "fffffffffffffff777f7fff7f7f7ffffffffffffff7ffff7fffffff777fff7f7fffffffff7777fff",
        ];
        for (row, y) in rows.iter().zip([0, 1, 48, 49]) {
            let ours: String = (0..80).map(|x| format!("{:x}", g.map.terrain(x, y).info().earth_code)).collect();
            assert_eq!(&ours, row, "row {y}");
        }
        // every computer civilization drew its second tribe; all start on CIV.EXE's squares
        for s in 1..8 {
            let civ = g.player(s).civ;
            assert_eq!(civ, s + 8);
            let u = g.all_units().find(|u| u.owner == s).unwrap();
            assert_eq!(u.pos(), mapgen::EARTH_STARTS[civ as usize]);
        }
    }

    #[test]
    fn fewer_civilizations_play_the_first_slots() {
        let g = Game::new(GameSetup { seed: 77, num_civs: 3, human_civ: 2, ..Default::default() }, None);
        assert_eq!(g.alive_slots(), vec![1, 2, 3]);
        let g = Game::new(GameSetup { seed: 77, num_civs: 3, human_civ: 14, ..Default::default() }, None);
        assert_eq!(g.alive_slots(), vec![1, 2, 6]);
    }
}
