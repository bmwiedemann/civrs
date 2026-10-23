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
pub mod happiness;
pub mod hints;
pub mod map;
pub mod mapgen;
pub mod movement;
pub mod orders;
pub mod palace;
pub mod player;
pub mod pollution;
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
    /// City that built each wonder (index = WonderId).
    pub wonders_built: [Option<CityId>; 22],
    pub polluted_tiles: u16,
    pub global_warming_count: u8,
    pub ended: Option<EndReason>,
    /// Scoring stopped (player chose to keep playing after the end).
    pub scoring_over: bool,
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
}

impl Game {
    /// Create a new game: generate the world, place civilisations and starting units.
    pub fn new(setup: GameSetup, earth: Option<&EarthMap>) -> Game {
        let mut rng = Rng::new(setup.seed);
        let seed32 = (setup.seed & 0xFFFF_FFFF) as u32 ^ (setup.seed >> 32) as u32;
        let map = match (&setup.world, earth) {
            (WorldSource::Earth, Some(em)) => mapgen::earth(em, seed32),
            (WorldSource::Random(p), _) => mapgen::generate(*p, seed32),
            (WorldSource::Earth, None) => mapgen::generate(WorldParams::normal(), seed32),
        };
        let human_slot = rules::slot_for_civ(setup.human_civ);
        let mut players: Vec<Player> = (0..8u8).map(|s| {
            let mut p = Player::new(s, s, false);
            p.in_game = s == 0;
            p
        }).collect();
        // choose rival slots (slot 0 is barbarians)
        let mut free: Vec<u8> = (1..8u8).filter(|s| *s != human_slot).collect();
        rng.shuffle(&mut free);
        let n_rivals = (setup.num_civs.clamp(3, 7) - 1) as usize;
        let mut slots: Vec<(u8, u8)> = vec![(human_slot, setup.human_civ)];
        for s in free.into_iter().take(n_rivals) {
            let pair = rules::civs_for_slot(s);
            let civ = pair[rng.below(2) as usize];
            slots.push((s, civ));
        }
        for (s, civ) in &slots {
            let human = *s == human_slot;
            let mut p = Player::new(*s, *civ, human);
            p.in_game = true;
            if human && !setup.leader_name.is_empty() {
                p.leader = setup.leader_name.clone();
            }
            players[*s as usize] = p;
        }
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
            vision: vec![0; rules::MAP_W * rules::MAP_H],
            events: VecDeque::new(),
            nukes_enabled: false,
            apollo_built: false,
            last_combat: None,
            hints_shown: 0,
        };
        game.place_starting_units(&slots);
        game.refresh_all_vision();
        game.active_unit = game.next_active_unit();
        game
    }

    fn place_starting_units(&mut self, slots: &[(u8, u8)]) {
        let diff = self.setup.difficulty as usize;
        let positions: Vec<(i32, i32)> = if self.map.earth {
            slots.iter().map(|(_, civ)| mapgen::snap_to_land(&self.map, mapgen::earth_start(*civ))).collect()
        } else {
            let mut rng = self.rng.clone();
            let p = mapgen::start_positions(&self.map, slots.len(), &mut rng);
            self.rng = rng;
            p
        };
        // remove huts near start positions
        for p in &positions {
            for y in (p.1 - 2)..=(p.1 + 2) {
                for x in (p.0 - 2)..=(p.0 + 2) {
                    if let Some(t) = self.map.get_mut(x, y) {
                        t.set(tile_flags::HUT, false);
                    }
                }
            }
        }
        for (i, (slot, _civ)) in slots.iter().enumerate() {
            let Some(&(x, y)) = positions.get(i) else { continue };
            let human = *slot == self.human_slot;
            let n_settlers = if human {
                rules::DIFFICULTY[diff].human_start_settlers
            } else if diff >= 3 {
                2
            } else {
                1
            };
            for _ in 0..n_settlers {
                self.add_unit(*slot, rules::unit_ids::SETTLERS, x, y);
            }
            // starting knowledge: 1-2 advances (AI gets extra at higher levels), 50 gold
            let n = 1 + self.rng.below(2) as u8 + if human { 0 } else { rules::DIFFICULTY[diff].ai_start_techs };
            for _ in 0..n {
                let opts = self.researchable(*slot);
                if let Some(a) = self.rng.pick(&opts).copied() {
                    self.players[*slot as usize].learn(a);
                }
            }
            self.players[*slot as usize].gold = 50;
            self.map.explore(*slot, x, y, 1);
        }
    }

    // ---- queries -------------------------------------------------------------------------

    pub fn year(&self) -> i32 {
        rules::year_for_turn(self.turn)
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

    /// The best defender on a tile (highest defence value after modifiers).
    pub fn best_defender(&self, x: i32, y: i32, attacker: Option<&Unit>) -> Option<UnitId> {
        let mut best: Option<(UnitId, i32)> = None;
        for id in self.units_at(x, y) {
            let u = self.unit(id);
            if let Some(a) = attacker {
                if a.is_air() && u.is_air() && a.info().att > 0 {
                    // fighters can engage air units
                }
            }
            let d = self.defense_strength(u, attacker);
            if best.map(|(_, bd)| d > bd).unwrap_or(true) {
                best = Some((id, d));
            }
        }
        best.map(|(id, _)| id)
    }

    pub fn is_enemy_at(&self, slot: Slot, x: i32, y: i32) -> bool {
        match self.owner_at(x, y) {
            Some(o) => o != slot && (slot == 0 || o == 0 || self.player(slot).at_war(o)),
            None => false,
        }
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
        let u = Unit::new(id, owner, kind, x, y, self.turn);
        if id as usize == self.units.len() {
            self.units.push(Some(u));
        } else {
            self.units[id as usize] = Some(u);
        }
        self.players[owner as usize].stats.units_built += 1;
        id
    }

    pub fn remove_unit(&mut self, id: UnitId) {
        // units carried by this one are lost too
        let carried: Vec<UnitId> = self.all_units().filter(|u| u.carried_by == Some(id)).map(|u| u.id).collect();
        for c in carried {
            self.units[c as usize] = None;
        }
        if self.active_unit == Some(id) {
            self.active_unit = None;
        }
        self.units[id as usize] = None;
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

    /// Next city name for a civilisation (its own list, then the spare pool).
    pub fn next_city_name(&mut self, slot: Slot) -> String {
        let civ = self.players[slot as usize].civ as usize;
        let used: Vec<String> = self.all_cities().map(|c| c.name.clone()).collect();
        let list = &rules::CIVS[civ].cities;
        let cursor = self.players[slot as usize].city_name_cursor as usize;
        for i in 0..16 {
            let n = list[(cursor + i) % 16];
            if !used.iter().any(|u| u == n) {
                self.players[slot as usize].city_name_cursor = ((cursor + i + 1) % 16) as u8;
                return n.to_string();
            }
        }
        for n in rules::SPARE_CITY_NAMES.iter() {
            if !used.iter().any(|u| u == n) {
                return n.to_string();
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
            production: Production::Unit(rules::unit_ids::MILITIA),
            improvements: 0,
            wonders: 0,
            worked: 0,
            specialists: Vec::new(),
            trade_routes: Vec::new(),
            disorder: false,
            disorder_turns: 0,
            celebrating: false,
            bought: false,
            changed_class: false,
            auto_build: false,
            founded_turn: self.turn,
            pollution_pending: 0,
            caravan_shields: 0,
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
            t.set(tile_flags::POLLUTION, false);
            // "A city founded in a Fortress square destroys the Fortress"; the mine goes too and
            // the square gets a road (CIV.EXE o 0x1b4f2)
            t.set(tile_flags::FORTRESS, false);
            t.set(tile_flags::MINE, false);
            t.set(tile_flags::ROAD, true);
        }
        self.auto_assign(id);
        // a city sees only its 3x3 block (14_city_screen / 26_turn_12 captures)
        self.map.explore(owner, x, y, 1);
        id
    }

    /// Remove a city from the map (destroyed).
    pub fn remove_city(&mut self, id: CityId) {
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
        // units homed here lose their home
        for u in self.units.iter_mut().flatten() {
            if u.home == Some(id) {
                u.home = None;
            }
        }
        for w in self.wonders_built.iter_mut() {
            if *w == Some(id) {
                *w = None;
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
    pub fn civ_name(&self, slot: Slot) -> &'static str {
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
