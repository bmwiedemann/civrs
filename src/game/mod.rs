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
pub mod stack;
pub mod turn;
pub mod unit;
pub mod wonders;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// The options a new game starts with: ds:0x19c0 = 0xfa (o 0x446c), everything but Instant
/// Advice and End of Turn.
impl Default for Options {
    fn default() -> Self {
        Options {
            autosave: true,
            animations: true,
            instant_advice: false,
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
    /// The origin of the human's map view as CIV.EXE keeps it (ds:0xd4cc, ds:0xd75e), which
    /// steers the choice of the next unit (`next_active_unit`); the map view keeps it in step
    /// with its own. None (a loaded game) puts no unit on screen.
    #[serde(skip)]
    pub view: Option<(i32, i32)>,
    /// move_units' unit loop as it stands between two picks (locals of 0x4046, fresh at each
    /// turn and after a load): the record of the last unit it took, the record before the
    /// last one it took off the screen ([bp-0x46], 0x7f to begin with) and whether the loop
    /// has come round to that record since ([bp-0x2e]), after which a unit outside the view
    /// is taken too (`next_active_unit`).
    #[serde(skip)]
    pub unit_loop: UnitLoop,
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
    /// Once a mask of the instant-advice notes already shown; CIV.EXE keeps no such memory,
    /// so only `u32::MAX` means something now: it mutes the notes (`Game::hint`, used by the
    /// headless previews). Stored after the game body in version-2 saves.
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
    /// Computer players (bit per slot) whose audience with the human is queued but not over.
    /// CIV.EXE holds the audience while their units move, and so does the turn runner (its
    /// unit loop stops for the answer, `unit_ai::start_units`); where nobody answers in the
    /// middle of the loop (`unit_ai::move_units`) their units keep the peace with the human
    /// until it is over. Not saved.
    #[serde(skip)]
    pub audience_pending: u8,
    /// A computer's unit whose step opened an audience with the human and also entered a
    /// hut: the move routine holds the audience in unit_look (0x6ae2) before hut_outcome
    /// (0x6b05), so the hut waits for the answer (`unit_ai::resume_units`). Not saved.
    #[serde(skip)]
    pub deferred_hut: Option<UnitId>,
    /// A computer's unit look (unit_look 0x8038) that stopped at the audience its meeting
    /// opened with the human (civs_meet 0x8491), and the neighbour it stopped at: the rest
    /// waits for the answer (`Game::finish_look`, before `deferred_hut`). Not saved.
    #[serde(skip)]
    pub deferred_look: Option<(UnitId, usize)>,
    /// A computer's capture whose civil war opened an audience with the human (the units it
    /// re-creates look, civs_meet 0x1508d): the rest of capture_city and of the step waits
    /// for the answer (`Game::finish_capture`, before `finish_look`). Not saved.
    #[serde(skip)]
    pub deferred_capture: Option<DeferredCapture>,
    /// ds:0x3934 between audiences: -1 (none, the initial value), else 0 after a
    /// `diplomacy_close` outside an audience (see `diplo_ai::open`). Not saved.
    #[serde(skip, default = "no_audience")]
    pub diplo_latch: i8,
    /// The human answered "Break treaty." for an attack on (or entering the land of) this
    /// civilization: the next move goes ahead (CIV.EXE 0x1a3b4 returned 1). Not saved.
    #[serde(skip)]
    pub treaty_break_ok: Option<Slot>,
    /// The human's step that the territory rule asked about after taking its cost and roll
    /// (0x6565): with `treaty_break_ok` the same step goes on without them. Not saved.
    #[serde(skip)]
    pub treaty_step: Option<(UnitId, Dir8)>,
    /// The human has confirmed an attack below full strength (`MoveOutcome::WeakAttack`).
    #[serde(skip)]
    pub weak_attack_ok: bool,
    /// The human's gold when the last *TAXES check ran (`treasury_warning`; CIV.EXE keeps the
    /// gold of the `game_turn` start at [bp-0x1a]); none after a load. Not saved.
    #[serde(skip)]
    pub taxes_mark: Option<i32>,
    /// ds:0xb1e8 during a city's turn: the human's city opens its city screen after this turn
    /// (`process_city` sets it for Population decrease 0xd769, Settlers, Diplomat or Caravan
    /// built 0xe3f8, an improvement or wonder built 0xe604, new disorder 0x126dd; an AUTO
    /// city's own choice clears it, 0xeb87). Its research is then not completed (0x12d53).
    /// Not saved.
    #[serde(skip)]
    pub city_attention: bool,
    /// The cases for Despotism and for the Republic, ds:0xe3c2 and ds:0xdb42, which the
    /// government choice of `ai_civ_turn` reads (0x15fe6, `ai::govern`). One pair for all:
    /// cleared as a civilization's cities come up (update_civ_tax_rates 0xa5ec), then every
    /// run of the city routine but the drawing one adds its worked squares (work_tile 0x132c1
    /// sets ds:0x2494 for the square values 0x1353a / 0x13552) and its turn its units
    /// (0x102f5), whatever the city's owner. Not saved: a civilization's cities come before
    /// its government choice in its turn.
    #[serde(skip)]
    pub gov_case: (i32, i32),
    /// The human's science of this turn's cities on Chieftain while the research menu they
    /// brought up is unanswered: CIV.EXE's menu is answered at once and the cities after it
    /// count their science twice when someone else already found the chosen goal (0x12c93);
    /// civrs adds it when the menu is answered (`Game::apply_decision`). Not saved.
    #[serde(skip)]
    pub chieftain_science: i32,
    /// Happy and unhappy citizens as the last run of the city routine left them (ds:0x70e2 /
    /// ds:0x70e4): a city's turn reads them for its score and its return value (0x12dbe,
    /// 0x13223) after the runs it may have caused itself (the advisor's on a change of mood,
    /// every city's after an advance). Not saved.
    #[serde(skip)]
    pub last_moods: (i32, i32),
    /// The open city screen's distance to the capital (ds:0x6b30): process_city works it out
    /// in its setup (0xc94b..0xcd65), which only opening the screen runs; every action on the
    /// screen (a sale, 0x121ac) redraws it from 0xcd65, so a sold Palace leaves the city's
    /// corruption as it was until the screen is opened again. Not saved.
    #[serde(skip)]
    pub screen_distance: Option<(CityId, i32)>,
    /// With `exe_quirks`: the specialists a city's turn placed (process_city [bp-0x8] =
    /// [bp-0x50], 0xdbef). The run's happiness rebalances with that local (0xf4d9, 0xf54e,
    /// 0xf7c4, 0xfa5c, 0xfb83) even after a completed Settlers shrank the city and the
    /// advisor's run (0xaa1b) placed its workers anew: CW2b t411, Ayotzinco 16 -> 15 with ten
    /// specialists counted where nine are left, one happy citizen less. Not saved.
    #[serde(skip)]
    pub exe_specialists: Option<(CityId, i32)>,
    /// choose_production's menu list [bp-0xde] (entry 0 the city's item, 0xaa35, then what it
    /// can build) and its best entry [bp-0x144] (0xb7df) as the calls of the re-choice loop in
    /// progress (0x18bef, `Game::rechoose_all`, `city_ai::rechoose`) left them; None outside
    /// such a loop. [bp-0x144] is set only by an item valued below 999 and never initialised,
    /// so a computer city with nothing below 999 takes the entry the index of an earlier city
    /// of the loop names (0xb864..0xb86a), with `exe_quirks`: N t145, Corinth gets Athens's
    /// fifth entry, a Temple. The loop's calls are made at one stack depth with nothing between
    /// them reaching that frame; what the first call finds there is not known. Not saved.
    #[serde(skip)]
    pub exe_advice_menu: Option<(Vec<Production>, Option<usize>)>,
    /// With `exe_quirks`: the totals (`economy::TurnTotals`, ds:0x70da..0x70de, ds:0x6546,
    /// ds:0x6c98, ds:0xb882) as the last quiet run of the city routine left them
    /// (`city_routine_quiet`), and the city whose turn reads its trade in place of its own
    /// squares' trade (`trade_carry`). CIV.EXE keeps a city's totals in globals: the
    /// advisor's run (0xaa1b) that a completed production starts inside the city's turn
    /// overwrites them, so the turn's trade split (0xf0d9, 0xf136) starts from the advisor's
    /// total, routes included, and adds the routes again, and the rest of the turn reads
    /// the advisor's shields, food, units abroad and pollution factors. civrs keeps the
    /// city's own totals. Not saved.
    #[serde(skip)]
    pub nested_totals: Option<economy::TurnTotals>,
    #[serde(skip)]
    pub trade_carry: Option<(CityId, i32)>,
    /// The units abroad (ds:0x6546) a city's turn counted before its production
    /// (`process_city`), which its happiness reads in place of a count of the units now.
    /// Not saved.
    #[serde(skip)]
    pub abroad_carry: Option<(CityId, i32)>,
    /// ds:0xb882 as the last run of the city routine left it (its pollution advance count,
    /// 0xcbfd): with `exe_quirks` the goto's cached grid test reads its low byte when the
    /// unit is 8 squares east and 6 north of the waypoint (`Game::local_search`). Not saved.
    #[serde(skip)]
    pub exe_pollution_techs: i32,
    /// Map page 2 at (x + 160, 50..199) as a CIV.EXE save's .MAP has it (picture data under
    /// the unit owner layer), for `exe_quirks` reads that miss the layer
    /// (`Game::exe_garrison_check`). Empty but for imported saves. Not saved.
    #[serde(skip)]
    pub exe_page_band: Vec<u8>,
    /// The global warming counter ds:0xb1ea (0..99): one step a turn towards twice the
    /// polluted squares minus four per warming so far (`global_warming_check`); stored after
    /// the game body in version-9 saves.
    #[serde(skip)]
    pub warming_counter: u16,
    /// The population milestone ds:0x1c24 of `draw_sidebar_status` (1 when the program
    /// starts, `Game::population_milestone`): hundreds of thousands below 10, millions + 9 from
    /// there; stored after the game body in version-25 saves (CIV.EXE does not save it).
    #[serde(skip)]
    pub pop_milestone: u16,
    /// ds:0x3484: -1 in a new game, -3 after loading one, -2 and then 0 at the next two turn
    /// changes (0x2939..0x2960); below -1 the milestone moves on without a word. Not saved.
    #[serde(skip)]
    pub milestone_quiet: i16,
    /// The per-civ score accumulator ds:0xddfe: zeroed as each civ's turn comes in game_turn's
    /// loop (0x25f5, also for the civs a loaded game skips) and by `civ_init` (o 0x4a47), the
    /// size + happy - unhappy of each of its cities as process_city counts them (0x12dd9);
    /// then the turn change adds 25 per wonder and takes the human's pollution penalty
    /// (0x2907..0x2960). Only the historian's palaces read it (o 0x10e32), before the turn
    /// change's additions. Not saved by civrs: a load zeroes it (game_turn resumes at the human).
    #[serde(skip)]
    pub score_acc: [i32; 8],
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
    /// A computer's step as `unit_ai::do_step` hands it to the move code, which runs the
    /// computer's own tests on it after the step's cost and move roll (`movement::AiStep`).
    /// Not saved.
    #[serde(skip)]
    pub ai_step: Option<movement::AiStep>,
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
    /// ds:0xd760: *UMAX was shown this turn (`GameEvent::UnitLimit`). Not saved.
    #[serde(skip)]
    pub unit_limit_shown: bool,
    /// ds:0x6b92: the riot animation ran this turn. Not saved.
    #[serde(skip)]
    pub riot_shown: bool,
    /// The traits of the 16 leaders (index = civ id; the leader records ds:0x14e2 + 0x3a * civ,
    /// +0x30..+0x34), from `rules::CIVS` until Alt-R re-draws them (`randomize_leaders`, 0x71d9).
    /// Not saved: CIV.EXE's SVE holds no leader record (serialize_game o 0xf75b), so a loaded
    /// game takes the shipped traits (CIV.EXE keeps a re-draw until it exits).
    #[serde(skip, default = "rules::initial_traits")]
    pub leader_traits: [rules::Traits; 16],
    /// Play CIV.EXE's mistakes that civrs otherwise leaves out where they decide what is
    /// drawn from the random stream (for the differential test's state dump only, as
    /// `use_exe_land_links`): the Chieftain wonder purchase's prerequisite (0x3c4a), the goto's
    /// local search choosing a step by the neighbours' costs alone (0x1eae2), the advisor's war
    /// test reading the one word ds:0x6ed4 of the last ai_civ_turn (0xb7c7), a capture leaving
    /// the last square of the radius out of the site/owner layer (0x13fff), destroy_city's
    /// sweep walking the old owner's record of the conqueror's unit (0xa8bc), the local
    /// search's grid read past its end (0x1e765), the first-city test reading the free
    /// city records (o 0x1b5df), the block search starting from block (0, 0) after failed
    /// entry probes (0x1e19d, the scratch unit's goto bytes left by 0x1ebe9), the capture of an empty city
    /// refused by a stale square owner of the mover's civ (0x6a65), the unit routine running
    /// on for a Diplomat it has disbanded (0x16a95) and the production advice counting the
    /// units on squares numbered as the city's continent, ocean bodies too (0xb0c1).
    #[serde(skip)]
    pub exe_quirks: bool,
    /// With `exe_quirks`: the si the caller of the quiet runs of the city routine in progress
    /// keeps, which their garrison check takes for y (`restash_garrison_quiet`,
    /// `exe_garrison_check`); None where it is not known (the check is then made as meant).
    #[serde(skip)]
    pub quiet_si: Option<QuietSi>,
    /// The owner byte (+0xb) CIV.EXE's free city records keep: destroy_city frees a record by
    /// its +6 byte alone (0xa7cc). With `exe_quirks` founding reads it (o 0x1b5df). Imported
    /// from a save, set when a city goes; not saved.
    #[serde(skip)]
    pub exe_city_owner: Vec<u8>,
    /// The square (x, y) each free unit record of CIV.EXE still holds, at `civ * 128 +
    /// record`: delete_unit (0x8d6c) frees a record by its type (+3 = 0xff, 0x8e67), clears
    /// its moves (+4) and its stack link (+0xa, stack_unlink 0x1a2df) and leaves the status,
    /// position, fuel, goto, heading, seen mask and home. With `exe_quirks` destroy_city's
    /// sweep reads them (`exe_destroy_sweep`). Imported from a save, set when a unit goes;
    /// (0, 0) for records never used (as a new game's); not saved.
    #[serde(skip)]
    pub exe_unit_ghost: Vec<(u8, u8)>,
    /// The stack link (+0xa) each free unit record of CIV.EXE holds, at `civ * 128 + record`:
    /// 0xff once delete_unit freed it (0x1ba98 -> stack_unlink 0x1a2df), 0 for a record never
    /// used since the unit table was zeroed (a new game's: civ_init o 0x4a28 frees the
    /// records by type and status only and leaves the links). With `exe_quirks` destroy_city's
    /// sweep follows them (`exe_for_each_in_stack`). Imported from a save; not saved (empty:
    /// 0xff).
    #[serde(skip)]
    pub exe_dead_link: Vec<u8>,
    /// With `exe_quirks`: the squares (`Map::idx`) whose layer x+160 lost its unit bit 8 while
    /// units stand there, as when a free record's delete writes its owner over a square it
    /// once held (`exe_destroy_sweep`): `unit_at` and `unit_owner_at` (0x1bade, 0x1bb66) then
    /// find no unit there, so a unit stepping in starts a ring of its own (0x67ce), until a
    /// step onto the square writes owner | 8 again (`set_tile_owner` 0x1392). Imported from a
    /// save; not saved.
    #[serde(skip)]
    pub exe_unit_bit_lost: Vec<bool>,
    /// CIV.EXE's count of a civilization's cities building each unit type (ds:0xd308 +
    /// 0x38 * civ, read by the production advisor for Diplomats 0xb3d2, Nuclear 0xb5cb,
    /// Bombers 0xb5fe and every ship and aircraft 0xb665) less the cities building it now,
    /// at `civ`. ai_civ_turn recounts the civ's row (0x159cb / 0x15a5a) and civ_init zeroes it
    /// (o5 0x4fcb); the changes of production in between move it by one (dec/inc pairs at
    /// 0x18c7c, 0xebb9, 0x117e4, 0x12756, ...), but a computer's new city (o 0x1b824), a
    /// capture (0x142f8 / 0x1434c), the Palace a captured capital's civ starts (0x13e2f) and
    /// the cities a civil war or a revolt hands over change production or owner without
    /// them (`Game::exe_d308_add`). Imported from a save (stored count less the cities'),
    /// kept with `exe_quirks` only; not saved.
    #[serde(skip)]
    pub exe_d308_drift: Vec<[i16; 28]>,
    /// With `exe_quirks`: units whose home byte (+0xb) still names a free city record, as
    /// destroy_city leaves the units of other civilizations homed there (it disbands only the
    /// owner's, 0xa89c): a city founded in that record supports them (create_city writes no
    /// unit). Imported from a save, set when a city goes; not saved.
    #[serde(skip)]
    pub exe_stale_home: Vec<(UnitId, CityId)>,
    /// With `exe_quirks`: a Settlers' job bits (unit +0 & 0xc2) where they are not those of
    /// its order: the job keys OR their bits into the status ('r' 0x532f, 'i' 0x54d5, 'm'
    /// 0x5670, 'f' 0x520b, 'p' 0x50ba) and a finished or refused job clears its own alone, so
    /// a computer's Settlers whose routine switches jobs keeps the rest (`Game::work_step`).
    /// Cleared by a computer's step (0x6810). Imported from a save; not saved.
    #[serde(skip)]
    pub exe_job_bits: std::collections::HashMap<UnitId, u8>,
}

/// `wonders_built` entry of a wonder whose city was destroyed: built, owned by no one, listed
/// as "(Destroyed)" (CIV.EXE stores city 0x80). No city has this id.
pub const WONDER_LOST: CityId = CityId::MAX;

/// Unit slots per civilization (create_unit CIV.EXE 0x8b62; KING.TXT *UMAX).
pub const MAX_UNITS: usize = 127;

impl Game {
    /// The traits of the leader of `slot` (`leader_traits` of its civilization).
    pub fn traits(&self, slot: Slot) -> rules::Traits {
        self.leader_traits[self.player(slot).civ as usize & 15]
    }

    /// Alt-R "Leaders randomized." (CIV.EXE 0x71d9): each of the 16 leaders draws aggression,
    /// expansion and civilized traits as random(3) - 1, in that order.
    pub fn randomize_leaders(&mut self) {
        for c in 0..16 {
            let aggressive = self.rng.random(0x71ea, 3) as i8 - 1;
            let expansionist = self.rng.random(0x71fb, 3) as i8 - 1;
            let civilized = self.rng.random(0x720c, 3) as i8 - 1;
            self.leader_traits[c] = rules::Traits { aggressive, expansionist, civilized };
        }
    }
}

fn no_audience() -> i8 {
    -1
}

/// A capture that waits for the audience its civil war opened (`Game::deferred_capture`).
#[derive(Clone, Debug, PartialEq)]
pub struct DeferredCapture {
    pub id: UnitId,
    pub city: CityId,
    pub cost: u8,
    /// The mover as it was before its step.
    pub attacker: Unit,
    pub from: Slot,
    pub was_capital: bool,
    /// The rest of the step (`Game::capture_step_tail`): the mover's civ, the city square,
    /// the city's old owner and the mover's goto before the step.
    pub step: Option<(Slot, (i32, i32), Slot, Option<(i32, i32)>)>,
}

/// The si a caller of the city routine's quiet runs holds (`Game::quiet_si`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuietSi {
    /// A loop over the cities with 0x1c * the city's index in si.
    Index,
    /// A value of its own.
    Fixed(i32),
}

/// The state of move_units' unit loop between two picks (`Game::unit_loop`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitLoop {
    /// The record of the unit taken last.
    pub last: u8,
    /// The record before the last unit taken off the screen ([bp-0x46]).
    pub mark: u8,
    /// The loop has passed `mark` since ([bp-0x2e]).
    pub round: bool,
    /// [bp-0x8] set: the units no longer look as the loop passes them (0x41b7). The latch
    /// closes when the first unit waits for orders on turn 0 (0x4599) or the first Settlers
    /// on any turn (0x45c9), and when the loop runs out of units (0x76cd).
    pub looks_done: bool,
    /// The last pick took a unit off the screen and centred the view on it (0x43c9), after
    /// the loop's 30-tick pause (0x439a, which waits unless ds:0xb278 == 1; nothing in CIV.EXE
    /// writes ds:0xb278, so the pause always comes). The map view pauses before it shows the
    /// new view (`MapView::adopt_view`).
    pub centred: bool,
    /// A unit has been taken since the loop began: its record ([bp-0x32]) stays where it
    /// was when the unit it took is gone (lost in a fight), so the loop goes on after it
    /// (0x4174) rather than from record 0.
    pub started: bool,
}

impl Default for UnitLoop {
    fn default() -> Self {
        UnitLoop { last: 0, mark: 0x7f, round: false, looks_done: false, centred: false, started: false }
    }
}

/// How `Game::run_auto_unit` left a unit that moves by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoStop {
    /// The unit has used its moves, or waits for orders.
    Done,
    /// A step in direction `Dir8` needs the player's answer (`CaravanArrived`,
    /// `EnteredForeignCity` or `BribeOffer`; the goto is gone), or `BreakTreaty` (the goto
    /// is gone for a partner's land, 0x656d; a fight's question keeps it until "Cancel
    /// action.", 0x19a1a).
    Menu(MoveOutcome, Dir8),
    /// The Settlers' job is no longer possible: the order is gone and the unit, its moves
    /// kept, waits for orders; the key's message, if CIV.EXE has one, is shown.
    Refused(RuleError),
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
    /// and moves again. The unit has not moved; for its land or empty city (the territory
    /// rule, 0x6565) the step's cost is spent and its move roll passed, and the move again
    /// goes on without them (`Game::treaty_step`).
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
            if s == 0 {
                // civ_init's start for every slot (o 0x4975..): no research done, taxes and
                // science 4 tenths each; the rest of it is skipped for the barbarians
                p.research_count = 0;
                p.tax = 40;
                p.lux = 20;
            }
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
            view: None,
            unit_loop: UnitLoop::default(),
            rng,
            setup: setup.clone(),
            // Instant Advice is on only for a Chieftain (o 0x4576)
            options: Options { instant_advice: setup.difficulty == Difficulty::Chieftain, ..Options::default() },
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
            deferred_hut: None,
            deferred_look: None,
            deferred_capture: None,
            diplo_latch: -1,
            treaty_break_ok: None,
            treaty_step: None,
            weak_attack_ok: false,
            taxes_mark: None,
            city_attention: false,
            warming_counter: 0,
            pop_milestone: 1,
            milestone_quiet: -1,
            score_acc: [0; 8],
            gov_case: (0, 0),
            chieftain_science: 0,
            last_moods: (0, 0),
            screen_distance: None,
            exe_specialists: None,
            exe_advice_menu: None,
            nested_totals: None,
            trade_carry: None,
            abroad_carry: None,
            exe_pollution_techs: 0,
            exe_page_band: Vec::new(),
            first_discoverer: vec![0; 72],
            historian_timer: 80 + historian,
            history: replay::History::default(),
            counting_losses: false,
            ai_step: None,
            catch_up: 0,
            free_research: 0,
            opening: true,
            unit_limit_shown: false,
            riot_shown: false,
            leader_traits: rules::initial_traits(),
            exe_quirks: false,
            quiet_si: None,
            exe_city_owner: Vec::new(),
            exe_unit_ghost: Vec::new(),
            exe_dead_link: vec![0; 8 * 128],
            exe_unit_bit_lost: Vec::new(),
            exe_d308_drift: Vec::new(),
            exe_stale_home: Vec::new(),
            exe_job_bits: std::collections::HashMap::new(),
        };
        game.place_starting_units(&slots, &world, &mut wr);
        // the game's draws go on in CIV.EXE's one stream
        game.rng = Rng::from_ms(wr);
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
        self.ai_memory.world_counts = Some((world.land_sizes.map(i32::from), world.sites.map(i32::from)));
        for &(slot, _, (x, y)) in slots {
            // create_unit (o 0x4f40), which looks around (0x8cd3)
            self.create_unit(slot, rules::unit_ids::SETTLERS, x, y);
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
                // (o 0x55a1)
                self.create_unit(slot, rules::unit_ids::SETTLERS, x, y);
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

    /// The units of `slot` in the order of their records (`Unit::record`), as CIV.EXE's
    /// per-civ loops visit them.
    pub fn units_of(&self, slot: Slot) -> Vec<UnitId> {
        let mut v: Vec<(u8, UnitId)> = self.all_units().filter(|u| u.owner == slot).map(|u| (u.record, u.id)).collect();
        v.sort_unstable();
        v.into_iter().map(|(_, id)| id).collect()
    }

    /// The lowest record of `slot`'s unit table no unit holds (create_unit 0x8b5f..0x8b80).
    pub fn free_record(&self, slot: Slot) -> u8 {
        let mut used = [false; 256];
        for u in self.all_units().filter(|u| u.owner == slot) {
            used[u.record as usize] = true;
        }
        used.iter().position(|b| !b).unwrap_or(255) as u8
    }

    /// Give every unit a record of its owner's table in id order (saves before version 24).
    pub fn number_unit_records(&mut self) {
        let mut next = [0u8; 8];
        for u in self.units.iter_mut().flatten() {
            let n = &mut next[u.owner as usize & 7];
            u.record = *n;
            *n = n.saturating_add(1);
        }
    }

    pub fn cities_of(&self, slot: Slot) -> Vec<CityId> {
        self.all_cities().filter(|c| c.owner == slot).map(|c| c.id).collect()
    }

    /// The units on (x, y) in the order of their records, so that the first is the square's
    /// first unit (`unit_at` 0x1bade) when one civ holds it.
    pub fn units_at(&self, x: i32, y: i32) -> Vec<UnitId> {
        let x = Map::wrap_x(x);
        let mut v: Vec<(u8, UnitId)> = self.all_units().filter(|u| u.x == x && u.y == y).map(|u| (u.record, u.id)).collect();
        v.sort_unstable();
        v.into_iter().map(|(_, id)| id).collect()
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
        // move_units picks it with best_defender_stack (CIV.EXE 0x5a91, 0x605c, 0x111e): from
        // the square's first unit (unit_at 0x1bade) the highest `defence_value` of the first
        // ten of its stack ring, the first of equals, else that first unit; the attacker,
        // walls and fortresses play no part
        let _ = attacker;
        let first = self.unit_at(x, y)?;
        Some(self.best_defender_stack(first))
    }

    /// `best_defender_stack(civ, unit)` (CIV.EXE 0x8f7e): the unit of highest
    /// `defence_value` among the ten units of `first`'s stack ring the walk (0x92a7) visits,
    /// the first of equals; `first` when none counts.
    pub fn best_defender_stack(&self, first: UnitId) -> UnitId {
        let mut best = (first, -1);
        for id in self.stack_from(first) {
            if let Some(d) = self.defence_value(id) {
                if d > best.1 {
                    best = (id, d);
                }
            }
        }
        best.0
    }

    /// Units or a city at (x, y) whose owner `slot` may fight (`Game::may_fight`).
    pub fn is_enemy_at(&self, slot: Slot, x: i32, y: i32) -> bool {
        match self.owner_at(x, y) {
            Some(o) => self.may_fight(slot, o),
            None => false,
        }
    }

    /// `Game::hostile`, except that a computer player keeps the peace with the human while an
    /// audience between them is queued (`audience_pending`; only where its unit loop does
    /// not stop for the answer).
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

    /// Recompute what `slot` sees now (its units' 3x3 or 5x5 and its cities' 3x3) and mark
    /// what its units' looks explore (`unit_look` 0x8038..0x8236): the barbarians explore
    /// nothing; a computer civ the eight squares around and, with 5x5 sight, the outer ring
    /// (a ship's on ocean only), but not the unit's own square, which only its creation marks
    /// (create_unit 0x8bcc); the human that square too. A city's sight explores nothing.
    pub fn refresh_vision(&mut self, slot: Slot) {
        self.recompute_vision(slot, true);
    }

    /// What every civilization sees, without any unit looking: a loaded game (`load_files`
    /// o 0xf5b0 reads the save and the map and runs no `unit_look`), so squares beside a unit
    /// stay unexplored when the save has them so (an edited save's units).
    pub fn refresh_all_sight(&mut self) {
        for s in 0..8u8 {
            self.recompute_vision(s, false);
        }
    }

    fn recompute_vision(&mut self, slot: Slot, looks: bool) {
        let bit = 1u8 << slot;
        for v in self.vision.iter_mut() {
            *v &= !bit;
        }
        // (x, y, radius, outer ring on ocean only, a unit's look)
        let mut spots: Vec<(i32, i32, i32, bool, bool)> = Vec::new();
        for u in self.all_units() {
            if u.owner == slot {
                spots.push((u.x, u.y, u.sight_range(), u.is_sea(), looks));
            }
        }
        for c in self.all_cities() {
            if c.owner == slot {
                spots.push((c.x, c.y, 1, false, false));
            }
        }
        let human = slot == self.human_slot;
        for (x, y, r, sea, look) in spots {
            for yy in (y - r)..=(y + r) {
                for xx in (x - r)..=(x + r) {
                    let outer = (xx - x).abs() > 1 || (yy - y).abs() > 1;
                    if Map::in_bounds(xx, yy) && !(sea && outer && !self.map.tile(xx, yy).is_ocean()) {
                        let i = Map::idx(xx, yy);
                        self.vision[i] |= bit;
                        let centre = (xx, yy) == (x, y);
                        if look && slot != 0 && (human || !centre) {
                            self.map.tiles[i].explored |= bit;
                            // the human's looks show him the squares' improvements (0x8078)
                            if human {
                                self.map.update_seen(xx, yy);
                            }
                        }
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
        u.record = self.free_record(owner);
        // create_unit (CIV.EXE 0x8b51) homes the unit to the nearest city when that city is
        // the owner's (0x8c0c), else to none
        u.home = self.nearest_city_exe(x, y).0.filter(|c| self.city(*c).owner == owner);
        if id as usize == self.units.len() {
            self.units.push(Some(u));
        } else {
            self.units[id as usize] = Some(u);
        }
        // into the square's stack ring after its first unit (create_unit 0x8ba9)
        self.stack_link(id, x, y);
        self.players[owner as usize].stats.units_built += 1;
        self.players[owner as usize].unit_counter[kind as usize % 28] += 1;
        self.note_occupant(owner, x, y);
        // its square becomes known to its owner, the barbarians too (0x8bcc)
        if let Some(t) = self.map.get_mut(x, y) {
            t.explored |= 1 << owner;
        }
        // create_unit 0x8b51 starts the unit without a heading
        self.ai_memory.heading.remove(&id);
        self.barbarian_memory.heading.remove(&id);
        id
    }

    pub fn remove_unit(&mut self, id: UnitId) {
        // a ship with room for others that goes down at sea takes every land unit of its
        // owner on the square along, a Carrier every aircraft too (delete_unit 0x8dba,
        // 0x8e06); elsewhere what it carried stays
        let Some(dead) = self.unit_opt(id).cloned() else { return };
        let lost: Vec<UnitId> = if dead.carries_units() && self.map.tile(dead.x, dead.y).is_ocean() {
            let carrier = dead.kind == rules::unit_ids::CARRIER;
            self.all_units().filter(|u| u.id != id && u.owner == dead.owner && u.pos() == dead.pos() && if carrier { u.is_air() } else { u.is_land() }).map(|u| u.id).collect()
        } else {
            Vec::new()
        };
        for c in lost.into_iter().chain([id]) {
            // out of the square's stack ring (delete_unit 0x8e83)
            if self.unit_opt(c).is_some() {
                self.stack_unlink(c);
            }
            if let Some(u) = self.units[c as usize].take() {
                self.note_unit_ghost(u.owner, u.record, u.x, u.y);
                self.exe_stale_home.retain(|(s, _)| *s != c);
                self.exe_job_bits.remove(&c);
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

    /// Whether `slot` has a free unit slot: create_unit (CIV.EXE 0x8b51) uses slots 0..126.
    pub fn unit_room(&self, slot: Slot) -> bool {
        self.all_units().filter(|u| u.owner == slot).count() < MAX_UNITS
    }

    /// `add_unit` with create_unit's limit (CIV.EXE 0x8b62): `None` when all of `owner`'s
    /// unit slots are in use; a human then gets the *UMAX box once a turn (0x8d24, ds:0xd760).
    pub fn try_add_unit(&mut self, owner: Slot, kind: UnitTypeId, x: i32, y: i32) -> Option<UnitId> {
        if !self.unit_room(owner) {
            if owner == self.human_slot && !std::mem::replace(&mut self.unit_limit_shown, true) {
                self.push_event(GameEvent::UnitLimit(owner));
            }
            return None;
        }
        Some(self.add_unit(owner, kind, x, y))
    }

    /// `create_unit` (CIV.EXE 0x8b51) in play: `try_add_unit`, and the new unit looks around at
    /// once (`unit_look` 0x8038 from 0x8cd3: sightings, contacts, goals, and the gotos of the
    /// foreign units next to it dropped). Only a garrison let out of a city does not look
    /// (city_unstash_garrison sets ds:0x20f4, 0x7e80); loading a game and the tests place
    /// units with `add_unit`.
    ///
    /// It never writes the moves byte (+4), and a free record has none (delete_unit 0x8e6c
    /// clears it), so the unit waits for its owner's next move loop to get its moves (0x40a4):
    /// a unit made in the middle of a turn (a respawned tribe's Settlers, a bribed or
    /// re-created unit) has no moves till then.
    pub fn create_unit(&mut self, owner: Slot, kind: UnitTypeId, x: i32, y: i32) -> Option<UnitId> {
        let id = self.try_add_unit(owner, kind, x, y)?;
        self.unit_mut(id).moves_left = 0;
        self.unit_look(id);
        Some(id)
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
    /// city's name is not given again; civrs takes the names the replay has logged, and the
    /// entries of the cities there are (a renamed city keeps its entry, `City::name_index`).
    pub fn next_city_name(&mut self, slot: Slot) -> String {
        let civ = self.players[slot as usize].civ as usize;
        let mut used: Vec<String> = self.all_cities().map(|c| c.name.clone()).collect();
        used.extend(self.history.names.iter().cloned());
        let held: Vec<u8> = self.all_cities().map(|c| c.name_index).collect();
        let list = &rules::CIVS[civ].cities;
        let table = rules::city_names::name_table();
        let own = if civ == 0 || civ == 8 { 224 } else { 16 * (if civ > 7 { civ - 2 } else { civ - 1 }) };
        for i in (own..own + 16).chain(224..256).chain(0..256) {
            if !used.iter().any(|u| u == table[i]) && !held.contains(&(i as u8)) {
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
        // `exe_stale_home`: the units whose home byte names the record are its own now
        let (now, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.exe_stale_home).into_iter().partition(|(_, c)| *c == id && self.exe_quirks);
        self.exe_stale_home = rest;
        for (u, _) in now {
            if let Some(u) = self.units.get_mut(u as usize).and_then(|u| u.as_mut()).filter(|u| u.home.is_none()) {
                u.home = Some(id);
            }
        }
        // found_city's first-city test (o 0x1b5df..0x1b616) goes over all 128 records by their
        // owner byte, free ones too: with `exe_quirks` a city the civilization's slot lost
        // before (the tribe destroyed and back as its other tribe) means no Palace
        let first_city = self.cities_of(owner).is_empty()
            && !(self.exe_quirks && self.exe_city_owner.iter().enumerate().any(|(i, o)| i as CityId != id && *o == owner && self.city_opt(i as CityId).is_none()));
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
            known_size: 0,
            coastal: false,
            hydro_site: false,
            name_index: city::NO_NAME_INDEX,
            trade: 0,
        };
        c.name_index = city::name_table_index(&c.name);
        c.coastal = self.founding_coastal(owner, x, y);
        c.hydro_site = self.founding_hydro_site(x, y);
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
            // terrain table ds:0x44a < -1)
            if matches!(t.terrain.info().irrigate, rules::Terraform::Bonus(_)) {
                t.set(tile_flags::IRRIGATION, true);
            }
        }
        // add_improvement (0x1bcd9) shows the human's own city's road at once
        if owner == self.human_slot {
            self.map.update_seen(x, y);
        }
        self.claim_city_site(owner, x, y);
        self.auto_assign(id);
        // a city sees only its 3x3 block (14_city_screen / 26_turn_12 captures)
        self.map.explore(owner, x, y, 1);
        // the human knows a new foreign city at once when one of his cities is within two
        // squares (o 0x1b734..0x1b783) or he has the Apollo Program, which also shows him its
        // square (o 0x1b828); either way as size 1
        let h = self.human_slot;
        if owner != h {
            let near = (-2..=2).any(|dy| (-2..=2).any(|dx| self.city_at(x + dx, y + dy).is_some_and(|c| self.city(c).owner == h)));
            if near {
                self.city_mut(id).known_size = 1;
            }
            if self.owner_has_wonder(h, rules::wonder_ids::APOLLO_PROGRAM) {
                self.map.explore(h, x, y, 0);
                self.city_mut(id).known_size = 1;
            }
        }
        // found_city o 0x1b4e4
        self.replay_city(replay::kind::CITY, owner, id);
        id
    }

    /// Gives every city square the road, and the irrigation where it adds food, that founding
    /// puts there (o 0x1b4f2, 0x1b525), for saves from before civrs stored them. A mine on the
    /// square stays and keeps the irrigation out.
    pub fn repair_city_squares(&mut self) {
        let squares: Vec<(i32, i32)> = self.cities.iter().flatten().map(|c| (c.x, c.y)).collect();
        for (x, y) in squares {
            if let Some(t) = self.map.get_mut(x, y) {
                t.set(tile_flags::ROAD, true);
                if !t.has(tile_flags::MINE) && matches!(t.terrain.info().irrigate, rules::Terraform::Bonus(_)) {
                    t.set(tile_flags::IRRIGATION, true);
                }
            }
        }
    }

    /// The coastal bit a city founded by `owner` at (x,y) gets (o 0x1b667..0x1b6b9): an Ocean
    /// square among the eight around whose body of water is larger than 256 squares
    /// (`continent_size` 0x1a68:0x1963), or any Ocean square for the human.
    pub fn founding_coastal(&mut self, owner: Slot, x: i32, y: i32) -> bool {
        let human = owner == self.human_slot;
        Map::neighbours8(x, y).any(|(nx, ny)| self.map.is_ocean(nx, ny) && (human || self.ocean_size(nx, ny) > 256))
    }

    /// The Hydro Plant site bit a city founded at (x,y) gets (o 0x1b6be..0x1b6d2): a River or
    /// Mountains square among the eight around.
    pub fn founding_hydro_site(&self, x: i32, y: i32) -> bool {
        Map::neighbours8(x, y).any(|(nx, ny)| matches!(self.map.terrain(nx, ny), Terrain::River | Terrain::Mountains))
    }

    /// Remove a city from the map (destroyed) in its owner's turn; see `destroy_city`.
    pub fn remove_city(&mut self, id: CityId) {
        let owner = self.city(id).owner;
        self.destroy_city(id, owner);
    }

    /// `destroy_city` (CIV.EXE 0xa76a): remove city `id` in the turn of civ `by` (ds:0x6b90,
    /// whose action or city turn destroys it).
    /// `exe_quirks`: destroy_city's sweep of its square (0xa8bc..0xa8eb) takes the first unit
    /// there by the map page's mark, after a capture the conqueror's (0x6a65 placed it first),
    /// and walks the stack of the old owner's unit of that record number
    /// (for_each_in_stack(owner, unit, kill_nonland_cb 0x949f)). An empty record's type 0xff
    /// reads a domain from before the unit table, not land, so delete_unit counts the type -1:
    /// the Caravan counter of the civilization before the owner goes down (0x8e63). Not copied
    /// in play (a sane sweep finds the conqueror's own land unit and deletes nothing).
    /// A free record (`exe_unit_ghost`) is deleted again: with link 0xff remove_city_marker
    /// (0x1ba98) writes the owner on the layer x+160 of the square the record last held
    /// (0x1be55, the unit bit 8 cleared), and the losses of the type -1 count as the previous
    /// civilization's Caravans when the losses are being counted (0x8d9d). A record never
    /// used links to record 0 (`exe_dead_link`): the walk goes on there, so a dead record 0
    /// is deleted too, or record 0's ring is swept, up to ten records (0x92a7).
    pub fn exe_destroy_sweep(&mut self, owner: Slot, record: u8) {
        self.exe_for_each_in_stack(owner, record, false);
    }

    /// The unit of `owner`'s record `record`, if the record is in use.
    fn exe_record_unit(&self, owner: Slot, record: u8) -> Option<UnitId> {
        self.all_units().find(|u| u.owner == owner && u.record == record).map(|u| u.id)
    }

    /// The stack link (+0xa) of `owner`'s record `record`: a unit's ring, else the free
    /// record's link (`exe_dead_link`).
    fn exe_link(&self, owner: Slot, record: u8) -> u8 {
        match self.exe_record_unit(owner, record) {
            Some(id) => self.unit(id).next.map_or(0xff, |n| self.unit(n).record),
            None => self.exe_dead_link.get(owner as usize * 128 + record as usize).copied().unwrap_or(0xff),
        }
    }

    fn exe_set_dead_link(&mut self, owner: Slot, record: u8, link: u8) {
        if self.exe_dead_link.len() < 8 * 128 {
            self.exe_dead_link.resize(8 * 128, 0xff);
        }
        self.exe_dead_link[owner as usize * 128 + record as usize] = link;
    }

    /// `for_each_in_stack(civ, unit, cb)` (CIV.EXE 0x92a7) from a record that may be free:
    /// up to ten records, each link read before the callback, up to a 0xff link or back at
    /// the first. The callback kills the units not of the land (`kill_nonland_cb` 0x949f) or,
    /// with `land`, those of the land (0x946c); a free record's type 0xff reads domain 27
    /// from before the unit table (ds:0x1116), never land.
    fn exe_for_each_in_stack(&mut self, owner: Slot, first: u8, land: bool) {
        let mut cur = first;
        for _ in 0..10 {
            let next = self.exe_link(owner, cur);
            match self.exe_record_unit(owner, cur) {
                Some(id) => {
                    if self.unit(id).is_land() == land {
                        self.remove_unit(id);
                    }
                }
                None if !land => self.exe_delete_free_record(owner, cur),
                None => {}
            }
            if next == 0xff || next == first {
                break;
            }
            cur = next;
        }
    }

    /// The records `for_each_in_stack(civ, record)` (0x92a7) visits, without a callback.
    fn exe_walk(&self, owner: Slot, first: u8) -> Vec<u8> {
        let mut v = vec![first];
        let mut cur = first;
        while v.len() < 10 {
            let next = self.exe_link(owner, cur);
            if next == 0xff || next == first {
                break;
            }
            v.push(next);
            cur = next;
        }
        v
    }

    /// The units a stack count (0x7e6:0x124d mode 2) finds on (x, y) for `civ`: none without
    /// a unit by `unit_at`, else the first ten of that unit's ring. With `exe_quirks` the walk
    /// takes `civ`'s record of that unit's number (CIV.EXE passes the searching civ), free
    /// records and other squares included.
    pub(crate) fn exe_stack_count(&self, civ: Slot, x: i32, y: i32) -> i32 {
        let Some(head) = self.unit_at(x, y) else { return 0 };
        if self.exe_quirks {
            self.exe_walk(civ, self.unit(head).record).len() as i32
        } else {
            self.stack_from(head).len() as i32
        }
    }

    /// `delete_unit` (CIV.EXE 0x8d6c) on a free record (type 0xff, `exe_destroy_sweep`).
    fn exe_delete_free_record(&mut self, owner: Slot, record: u8) {
        let i = owner as usize * 128 + record as usize;
        let (x, y) = self.exe_unit_ghost.get(i).copied().unwrap_or((0, 0));
        let (x, y) = (x as i32, y as i32);
        let p = (owner > 0).then(|| owner as usize - 1);
        if let (Some(p), true) = (p, self.counting_losses) {
            self.players[p].units_lost_by_type[27] += 1;
        }
        // type -1 carries 127 (ds:0x1124): a linked record on Ocean drowns the land units its
        // links lead to (0x8dba..0x8df0)
        if self.exe_link(owner, record) != 0xff && Map::in_bounds(x, y) && self.map.tile(x, y).is_ocean() {
            self.exe_for_each_in_stack(owner, record, true);
        }
        if let Some(p) = p {
            self.players[p].unit_counter[27] -= 1;
        }
        let link = self.exe_link(owner, record);
        if link != 0xff {
            // stack_unlink (0x1a2df): the predecessor points past it (free records only: no
            // ring of units leads to a free record)
            if self.exe_link(owner, link) == record {
                if self.exe_record_unit(owner, link).is_none() {
                    self.exe_set_dead_link(owner, link, 0xff);
                }
            } else {
                let mut cur = link;
                let mut n = 0;
                loop {
                    cur = self.exe_link(owner, cur);
                    n += 1;
                    if n >= 32 || cur == 0xff || self.exe_link(owner, cur) == record {
                        break;
                    }
                }
                if n < 32 && cur != 0xff && self.exe_record_unit(owner, cur).is_none() {
                    self.exe_set_dead_link(owner, cur, link);
                }
            }
            self.exe_set_dead_link(owner, record, 0xff);
        } else {
            self.note_occupant(owner, x, y);
            if Map::in_bounds(x, y) {
                if self.exe_unit_bit_lost.len() != rules::MAP_W * rules::MAP_H {
                    self.exe_unit_bit_lost = vec![false; rules::MAP_W * rules::MAP_H];
                }
                self.exe_unit_bit_lost[Map::idx(x, y)] = true;
            }
        }
    }

    /// Whether the layer x+160 of (x, y) lacks the unit bit though units stand there
    /// (`exe_unit_bit_lost`).
    pub fn exe_unit_bit_lost_at(&self, x: i32, y: i32) -> bool {
        self.exe_quirks && Map::in_bounds(x, y) && self.exe_unit_bit_lost.get(Map::idx(Map::wrap_x(x), y)).copied().unwrap_or(false)
    }

    /// Keep the square a unit record held when its unit goes (`exe_unit_ghost`).
    pub(crate) fn note_unit_ghost(&mut self, owner: Slot, record: u8, x: i32, y: i32) {
        let i = owner as usize * 128 + record as usize;
        if self.exe_unit_ghost.len() < 8 * 128 {
            self.exe_unit_ghost.resize(8 * 128, (0, 0));
        }
        if let Some(g) = self.exe_unit_ghost.get_mut(i) {
            *g = (x as u8, y as u8);
        }
        // delete_unit unlinks it (0x1ba98 -> stack_unlink 0x1a2df writes 0xff)
        self.exe_set_dead_link(owner, record, 0xff);
    }

    pub fn destroy_city(&mut self, id: CityId, by: Slot) {
        // destroy_city 0xa793 logs the city as gone
        self.replay_city(replay::kind::CITY, 0xff, id);
        let (x, y, owner) = {
            let c = self.city(id);
            (c.x, c.y, c.owner)
        };
        // its owner's census count goes down at once (0xa7c8)
        if let Some(n) = self.ai_memory.census.city_total.get_mut(owner as usize) {
            *n -= 1;
        }
        for t in self.map.tiles.iter_mut() {
            if t.worked_by == Some(id) {
                t.worked_by = None;
            }
        }
        // add_improvement(0) (0xa7da -> 0x1bcd9) clears both improvement nibbles of the
        // square: the city's road and irrigation go, and so does the river-sprite bit 0x80
        if let Some(t) = self.map.get_mut(x, y) {
            t.city = None;
            t.flags &= tile_flags::HUT;
        }
        // it shows the human the bare square in his own turn (0x1bd5e); else a human who knows
        // the square sees it (destroy_city 0xa80a)
        if by == self.human_slot || self.explored(self.human_slot, x, y) {
            self.map.update_seen(x, y);
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
                if self.exe_quirks {
                    self.exe_stale_home.push((u.id, id));
                }
            }
        }
        // every route to it is cleared (destroy_city 0xa812), so a new city in the slot has
        // none; the slot is left free where it is (0xa83a), the others keep theirs
        for c in self.cities.iter_mut().flatten() {
            for i in 0..3 {
                if c.route_slot(i) == Some(id) {
                    c.set_route_slot(i, None);
                }
            }
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
        if self.exe_city_owner.len() <= id as usize {
            self.exe_city_owner.resize(id as usize + 1, 0);
        }
        self.exe_city_owner[id as usize] = owner;
        // the site/owner layer gets the generator's values back (0xa8f3..0xa968): on the city
        // radius, and on the 24 squares beyond where no other city is within 3 (nearest
        // city 0x1d43:0xf8)
        for (k, (dx, dy)) in barbarians::CLAIM.iter().enumerate() {
            let (nx, ny) = (Map::wrap_x(x + dx), y + dy);
            if !Map::in_bounds(nx, ny) {
                continue;
            }
            let (near, d) = self.nearest_city_exe(nx, ny);
            if k <= 20 || near.is_none() || d > 3 {
                let v = self.map.site_at(nx, ny);
                self.map.set_claim(nx, ny, v);
            }
        }
    }

    /// The next unit of the human player the loop comes to (move_units 0x4144..0x43d8): the
    /// records from the one after the active unit, wrapping, first those inside the map view
    /// (columns vx+1..vx+14, rows vy+1..vy+9, 0x434f..0x437f), and only after a whole round
    /// without one the first of the others, on which the view is centred (0x43c9).
    /// CIV.EXE compares the columns without the east-west wrap, so near x = 0 it takes
    /// visible units for hidden ones; civrs wraps but for `exe_quirks` (where it decides the
    /// order of the looks and of the units moving by themselves).
    ///
    /// The unit may be one that moves by itself (`is_auto_unit`): a goto, taken by the same
    /// rule, or a Settlers at work or an aircraft returning to base, taken wherever it is
    /// (flags 0xc2 skip the view test, 0x4345). The caller moves it or presses its job's key
    /// (`run_auto_unit`, 0x4492..0x4556 -> 0x4bbe) and goes on with the loop, so these units
    /// move and work in record order among the ones waiting for orders.
    /// Every unit the loop passes looks around first while `UnitLoop::looks_done` is open
    /// (0x41b7 -> unit_look 0x41e3), and one that is fortifying becomes fortified
    /// (`loop_passes`, 0x4239).
    pub fn next_active_unit(&mut self) -> Option<UnitId> {
        let after = self.active_unit;
        self.next_active_unit_from(after, 1, true)
    }

    /// `next_active_unit` going on from unit `from` itself, as the loop does after 'u'
    /// (0x4e64 sets the loop's unit to the one before `next_unit_in_stack`).
    /// It also makes the record before `from` the loop's mark (0x4e78).
    pub fn next_active_unit_at(&mut self, from: UnitId) -> Option<UnitId> {
        if let Some(u) = self.unit_opt(from) {
            self.unit_loop.mark = u.record.wrapping_sub(1) & 127;
            self.unit_loop.round = false;
        }
        self.next_active_unit_from(Some(from), 0, true)
    }

    /// The loop looks at every unit with moves and without the sentry or fortified bits
    /// (0x41fb), passengers at sea included. `prefer_view` false is the loop's "whole round
    /// done" flag ([bp-0x2e]) set, as 'w' sets it (0x4dd7): the next unit is taken wherever
    /// it is.
    ///
    /// The loop runs over the records 0..127 again and again from the one after the unit it
    /// took last (0x4174, 0x7691 -> 0x4144). At the record before the last unit it took off
    /// the screen (127 at the start of the turn) it notes that it has come round (0x4150);
    /// before that a unit outside the view is passed over (0x4381), after it one is taken,
    /// the view centred on it, and that unit becomes the new mark (0x438f..0x43db). So once
    /// no unit in the view wants orders, the next one off the screen is the first from that
    /// mark on, not the first after the unit that just finished.
    fn next_active_unit_from(&mut self, from: Option<UnitId>, skip: usize, prefer_view: bool) -> Option<UnitId> {
        let slot = self.human_slot;
        // the loop leaves the unit it had: without moves, its end-of-moves checks (0x6dad ->
        // 0x750f, `unit_moves_done`). (After 'u' CIV.EXE runs them on the record before the
        // unit it goes on with, 0x4e71; not copied.)
        if let Some(f) = from.filter(|f| skip == 1 && self.unit_opt(*f).is_some_and(|u| u.owner == slot)) {
            self.unit_moves_done(f);
        }
        let ids = self.units_of(slot);
        let mut by_record: [Option<UnitId>; 128] = [None; 128];
        for id in &ids {
            by_record[self.unit(*id).record as usize & 127] = Some(*id);
        }
        // a unit deleted since (disbanded, a caravan's help) leaves the loop at its record
        let at = from.and_then(|a| self.unit_opt(a)).filter(|u| u.owner == slot).map(|u| u.record).unwrap_or(self.unit_loop.last);
        let mut i = if from.is_some() || self.unit_loop.started { (at as usize + skip) & 127 } else { 0 };
        self.unit_loop.centred = false;
        let wants = |u: &Unit| u.moves_left > 0 && (matches!(u.order, UnitOrder::None | UnitOrder::Goto | UnitOrder::Home) || u.order.is_work());
        if !ids.iter().any(|id| wants(self.unit(*id))) {
            // the loop runs out (0x7691): it passes the rest of its round, then the latch
            // closes for the End of Turn wait (0x76cd)
            for r in i..128 {
                if let Some(id) = by_record[r] {
                    if !self.unit_loop.looks_done {
                        self.unit_look(id);
                    }
                    self.loop_passes(id);
                }
            }
            self.unit_loop.looks_done = true;
            return None;
        }
        if !prefer_view {
            self.unit_loop.round = true;
        }
        for _ in 0..3 * 128 {
            if i as u8 == self.unit_loop.mark {
                self.unit_loop.round = true;
            }
            if let Some(id) = by_record[i] {
                if !self.unit_loop.looks_done {
                    self.unit_look(id);
                }
                self.loop_passes(id);
            }
            // (a look never takes a unit of the looking civ off the map; a garrison stashed
            // as the loop passed takes units off, `unit_opt`)
            if let Some(id) = by_record[i].filter(|id| self.unit_opt(*id).is_some_and(wants)) {
                let u = self.unit(id);
                // (`exe_quirks`: the columns compared unwrapped, vx < x <= vx + 14, 0x4357 /
                // 0x4372, so a view at the east edge hides the units at the map's west end and
                // the loop passes, every unit looking, to the end of its round; C7 t336)
                let shown = self.view.is_some_and(|(vx, vy)| {
                    let col = if self.exe_quirks { (vx + 1..=vx + 14).contains(&u.x) } else { (1..=14).contains(&Map::wrap_x(u.x - vx)) };
                    col && (1..=9).contains(&(u.y - vy))
                });
                let (on_screen, (x, y)) = (shown || u.order == UnitOrder::Home || u.order.is_work(), u.pos());
                if on_screen || self.unit_loop.round {
                    if !on_screen {
                        self.view = Some(Self::view_centred_on(x, y));
                        self.unit_loop.mark = (i as u8).wrapping_sub(1) & 127;
                        self.unit_loop.round = false;
                        self.unit_loop.centred = true;
                    }
                    self.unit_loop.last = i as u8;
                    self.unit_loop.started = true;
                    self.unit_waits(id);
                    return Some(id);
                }
            }
            i = (i + 1) & 127;
        }
        None
    }

    /// A unit of the human's that moves or works by itself when his loop comes to it
    /// (0x44ac: flags 0xc2 or a goto): on goto, an aircraft returning to base, or a Settlers
    /// at work, for which the loop presses the job's key again (0x44bd..0x452f).
    pub fn is_auto_unit(&self, id: UnitId) -> bool {
        self.unit_opt(id).is_some_and(|u| u.owner == self.human_slot && u.moves_left > 0 && (matches!(u.order, UnitOrder::Goto | UnitOrder::Home) || u.order.is_work()))
    }

    /// The loop passes the human's unit `id` (0x41fb..0x4327): one that is fortifying and has
    /// its moves becomes fortified (`complete_fortify`).
    fn loop_passes(&mut self, id: UnitId) {
        if self.unit_opt(id).is_some_and(|u| u.order == UnitOrder::Fortify && u.moves_left > 0) {
            self.complete_fortify(id);
        }
    }

    /// The human's unit `id` waits for orders (0x455e): on turn 0 (FIRSTMOVE, 0x456b) and for
    /// a Settlers (the Settlers advice, 0x459e) the loop's look latch closes, whether or not a
    /// note comes. CIV.EXE shares the latch [bp-0x8] between these notes and the looks, so
    /// after the first Settlers of a turn waits no unit of the human looks at the loop's pass.
    pub fn unit_waits(&mut self, id: UnitId) {
        let Some(u) = self.unit_opt(id) else { return };
        if u.order == UnitOrder::None && (self.turn == 0 || u.kind == crate::rules::unit_ids::SETTLERS) {
            self.unit_loop.looks_done = true;
        }
    }

    /// The loop starts afresh, as move_units does at each turn and after a load.
    pub fn reset_unit_loop(&mut self) {
        self.unit_loop = UnitLoop::default();
    }

    /// Tests: the human's turn start with his loop run to its end, every unit it takes
    /// skipped (Space).
    #[cfg(test)]
    pub fn run_human_loop(&mut self) {
        self.reset_moves(self.human_slot);
        self.active_unit = None;
        self.reset_unit_loop();
        while let Some(id) = self.next_active_unit() {
            self.skip_unit(id);
            self.active_unit = Some(id);
        }
    }

    /// The next unit after 'w' (0x4dd2): the loop goes on to the next unit in record order,
    /// in the view or not.
    pub fn next_active_unit_after_wait(&mut self) -> Option<UnitId> {
        let after = self.active_unit;
        self.next_active_unit_from(after, 1, false)
    }

    /// The view origin that puts (x, y) at column 7, row 6 (0x1a68:0xe).
    pub fn view_centred_on(x: i32, y: i32) -> (i32, i32) {
        (Map::wrap_x(x - 7), (y - 6).clamp(0, crate::rules::MAP_H as i32 - 12))
    }

    /// `draw_map_view(human, vx, vy)` (0x1a68e) called by a message about a square: the
    /// human's map view moves there (x wrapped, y clamped to 0..38), which steers the next
    /// unit the human's loop takes (`next_active_unit`). Pollution (0x12ee9) and a meltdown
    /// (0x12fab) at the square - (8, 6), an uprising near a city (0x2fcb) at the city - (8, 6),
    /// a disaster in a foreign city the human has explored (o 0x1c010) at the city - (10, 6),
    /// a revolt (o 0x1e22b) at the city - (8, 5), a unit of the human's bribed (o 0x1dd7b)
    /// at the unit - (8, 6).
    pub fn show_map_at(&mut self, vx: i32, vy: i32) {
        self.view = Some((Map::wrap_x(vx), vy.clamp(0, crate::rules::MAP_H as i32 - 12)));
    }

    /// The view after `scroll_to` (0x9505), which keeps a moving unit in sight: a square
    /// outside columns vx+2..vx+13 or rows vy+2..vy+9 gets the view origin (x - 8, y - 6).
    pub fn view_scrolled_to(view: (i32, i32), x: i32, y: i32) -> (i32, i32) {
        let rel = Map::wrap_x(x - view.0);
        if (2..=13).contains(&rel) && (view.1 + 2..=view.1 + 9).contains(&y) {
            view
        } else {
            (Map::wrap_x(x - 8), (y - 6).clamp(0, crate::rules::MAP_H as i32 - 12))
        }
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
    fn unit_looks_explore_by_civ_and_cities_explore_nothing() {
        // unit_look 0x8038: the human from the unit's square, a computer civ around it only,
        // the barbarians nothing beyond the square create_unit marks (0x8bcc)
        let mut g = Game::new(GameSetup { seed: 9, ..Default::default() }, None);
        let h = g.human_slot;
        let ai = (1..8u8).find(|s| *s != h).unwrap();
        for y in 20..30 {
            for x in 30..60 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Grassland;
                t.explored = 0;
            }
        }
        let (u0, ua, uh) = (g.add_unit(0, rules::unit_ids::LEGION, 33, 25), g.add_unit(ai, rules::unit_ids::LEGION, 43, 25), g.add_unit(h, rules::unit_ids::LEGION, 53, 25));
        for (id, slot) in [(u0, 0), (ua, ai), (uh, h)] {
            let (x, y) = g.unit(id).pos();
            g.map.get_mut(x, y).unwrap().explored = 0;
            g.refresh_vision(slot);
        }
        assert!(!g.explored(0, 33, 25) && !g.explored(0, 34, 25));
        assert!(!g.explored(ai, 43, 25) && g.explored(ai, 44, 26));
        assert!(g.explored(h, 53, 25) && g.explored(h, 52, 24));
        // a city sees its 3x3 but explores none of it
        g.remove_unit(ua);
        g.add_city(ai, "Seen".into(), 40, 22);
        g.map.get_mut(41, 23).unwrap().explored = 0;
        g.refresh_vision(ai);
        assert!(g.visible_to(ai, 41, 23) && !g.explored(ai, 41, 23));
    }

    #[test]
    fn a_loaded_game_sees_without_its_units_looking() {
        // load_files (o 0xf5b0) runs no unit_look: an edited save's unit leaves the squares
        // around it unexplored (TRBR t37: the German Legions' scores, 0x18617)
        let mut g = Game::new(GameSetup { seed: 9, ..Default::default() }, None);
        let ai = (1..8u8).find(|s| *s != g.human_slot).unwrap();
        for y in 20..30 {
            for x in 30..40 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Grassland;
                t.explored = 0;
            }
        }
        g.add_unit(ai, rules::unit_ids::MILITIA, 35, 25);
        g.refresh_all_sight();
        assert!(g.visible_to(ai, 36, 26) && !g.explored(ai, 36, 26));
        g.refresh_vision(ai);
        assert!(g.explored(ai, 36, 26));
    }

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
    fn instant_advice_starts_on_only_for_a_chieftain() {
        // o 0x446c: options 0xfa; o 0x4576: bit 0 set at Chieftain
        let g = Game::new(GameSetup { seed: 9, difficulty: Difficulty::Chieftain, ..Default::default() }, None);
        assert!(g.options.instant_advice);
        let g = Game::new(GameSetup { seed: 9, difficulty: Difficulty::Prince, ..Default::default() }, None);
        assert!(!g.options.instant_advice);
        assert!(g.options.autosave && g.options.palace && !g.options.end_turn_confirm);
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

    #[test]
    fn the_end_year_ends_a_continued_game_again() {
        let mut g = Game::new(GameSetup { seed: 9, ..Default::default() }, None);
        g.end_game(EndReason::Spaceship(g.human_slot));
        g.process_world();
        g.take_events();
        g.apply_decision(&Decision::KeepPlaying { reason: EndReason::Spaceship(g.human_slot) }, DecisionReply::Yes);
        // 0x2b47 compares the year with 2100 - 20 * difficulty whatever ended before
        let end = rules::DIFFICULTY[g.setup.difficulty as usize].end_year;
        g.year = end - 1;
        g.process_world();
        let ev = g.take_events();
        assert!(ev.contains(&GameEvent::GameEndedAgain(EndReason::TimeLimit)));
        assert!(ev.iter().any(|e| matches!(e, GameEvent::NeedsDecision(Decision::KeepPlaying { .. }))));
    }
}

#[cfg(test)]
mod new_game_tests {
    use super::*;

    #[test]
    fn a_unit_made_in_play_waits_for_its_owners_move_loop() {
        // create_unit 0x8b51 never writes the moves byte, which delete_unit clears (0x8e6c)
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        let s = (1..8).find(|s| *s != g.human_slot && g.player(*s).alive).unwrap();
        let (x, y) = g.units_of(s).first().map(|u| g.unit(*u).pos()).unwrap();
        let id = g.create_unit(s, rules::unit_ids::LEGION, x, y).unwrap();
        assert_eq!(g.unit(id).moves_left, 0);
        g.reset_moves(s);
        assert_eq!(g.unit(id).moves_left, 3);
    }

    #[test]
    fn exe_quirks_give_no_palace_to_a_slot_that_lost_a_city_before() {
        // found_city's first-city test (o 0x1b5df) reads the owner byte of the free records
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        let s = (1..8).find(|s| *s != g.human_slot && g.player(*s).alive).unwrap();
        for (x, y) in [(20, 20), (24, 20), (28, 20)] {
            let t = g.map.get_mut(x, y).unwrap();
            t.terrain = rules::Terrain::Plains;
        }
        // two cities gone: the new one takes the first free record, the other still says `s`
        let a = g.add_city(s, "Old".into(), 20, 20);
        let b = g.add_city(s, "Older".into(), 28, 20);
        g.destroy_city(a, 0);
        g.destroy_city(b, 0);
        g.exe_quirks = true;
        let n = g.add_city(s, "New".into(), 24, 20);
        assert!(!g.city(n).has_improvement(rules::improvement_ids::PALACE));
        g.remove_city(n);
        g.exe_quirks = false;
        let n = g.add_city(s, "Sane".into(), 24, 20);
        assert!(g.city(n).has_improvement(rules::improvement_ids::PALACE));
    }

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
        // the founding left road and irrigation, took the mine and kept the pollution
        let t = g.map.tile(20, 20);
        assert_eq!(t.flags & (tile_flags::ROAD | tile_flags::IRRIGATION | tile_flags::MINE | tile_flags::POLLUTION), tile_flags::ROAD | tile_flags::IRRIGATION | tile_flags::POLLUTION);
        g.remove_city(c);
        // a destroyed city's name stays used (es:0x540, o 0x1b3c5)
        assert_eq!(g.next_city_name(s), list[1]);
        // a renamed city keeps its entry, so its first name is not given again
        let renamed = g.add_city(s, list[1].to_string(), 40, 40);
        g.rename_city(renamed, "Elsewhere").unwrap();
        assert_eq!(g.next_city_name(s), list[2]);
        for (i, n) in list.iter().enumerate().skip(2) {
            g.add_city(s, n.to_string(), 2 + 2 * i as i32, 30);
        }
        assert_eq!(g.next_city_name(s), rules::SPARE_CITY_NAMES[0]);
    }

    #[test]
    fn a_destroyed_city_leaves_a_bare_square() {
        // destroy_city 0xa7da: add_improvement(0) clears every improvement bit, river sprite
        // bit 0x80 included; the human sees it at once in his own turn or where he has looked
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        let me = g.human_slot;
        let other = g.civ_slots().into_iter().find(|s| *s != me && *s != 0).unwrap();
        for (x, by) in [(20, me), (30, other)] {
            g.map.get_mut(x, 20).unwrap().terrain = rules::Terrain::River;
            let c = g.add_city(other, format!("C{x}"), x, 20);
            g.map.get_mut(x, 20).unwrap().flags |= tile_flags::RAILROAD | tile_flags::RIVER_ALT;
            g.map.update_seen(x, 20);
            g.destroy_city(c, by);
            assert_eq!(g.map.tile(x, 20).flags, 0);
        }
        assert_eq!(g.map.seen[Map::idx(20, 20)], 0);
        if !g.explored(me, 30, 20) {
            assert_ne!(g.map.seen[Map::idx(30, 20)], 0);
        }
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

    #[test]
    fn the_barbarians_start_with_civ_inits_rates() {
        let g = Game::new(GameSetup { seed: 4763, ..Default::default() }, None);
        let b = g.player(0);
        assert_eq!((b.tax, b.lux, b.research_count), (40, 20, 0));
    }

    #[test]
    fn alt_r_redraws_every_leaders_traits() {
        // 0x71d9: 16 leaders x (aggression, expansion, civilized) = random(3) - 1
        let mut g = Game::new(GameSetup { seed: 5, ..Default::default() }, None);
        assert_eq!(g.leader_traits, rules::initial_traits());
        let s = g.human_slot;
        assert_eq!(g.traits(s), g.player(s).civ().traits());
        let mut seen = std::collections::HashSet::new();
        for _ in 0..4 {
            g.randomize_leaders();
            for t in g.leader_traits {
                for v in [t.aggressive, t.expansionist, t.civilized] {
                    assert!((-1..=1).contains(&v));
                    seen.insert(v);
                }
            }
        }
        assert_eq!(seen.len(), 3);
        assert_eq!(g.traits(s), g.leader_traits[g.player(s).civ as usize]);
        // not saved: the save body skips the table
        let back: Game = bincode::deserialize(&bincode::serialize(&g).unwrap()).unwrap();
        assert_eq!(back.leader_traits, rules::initial_traits());
    }
}
