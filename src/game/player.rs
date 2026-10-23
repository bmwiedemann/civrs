use super::map::{CityId, Slot};
use crate::rules::{AdvanceId, Govt};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Relation {
    #[default]
    NoContact,
    Peace,
    War,
}

/// A civilisation's spaceship (CIV.EXE overlay 18, see `game::spaceship`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Spaceship {
    /// Parts of each kind placed on `grid` (kept in step with it by `Spaceship::recount`).
    pub structurals: u8,
    pub components: u8,
    pub modules: u8,
    pub launched_turn: Option<u32>,
    /// Saves before version 6 landed the ship on this turn; no longer set (the ship lands in
    /// the year `arrival_year`), kept so that older saves load.
    pub arrival_turn: Option<u32>,
    /// The ship was cancelled when the capital fell (cleared by the next part).
    pub lost: bool,
    /// Ship grid es:0x16e0[civ] (`spaceship::GRID_W` x `spaceship::GRID_H`, 0 = empty, else
    /// part type + 1). Stored after the game body in version-5 saves.
    #[serde(skip)]
    pub grid: crate::game::spaceship::ShipGrid,
    /// Arrival year ds:0xdc10[civ]: projected each time the ship is redrawn until launch, fixed
    /// after it (None = -1). Stored after the game body in version-5 saves.
    #[serde(skip)]
    pub arrival_year: Option<i32>,
    /// The ship reached Alpha Centauri. Stored after the game body in version-5 saves.
    #[serde(skip)]
    pub landed: bool,
    /// Launch year ds:0xe8a8[civ]. Stored after the game body in version-6 saves.
    #[serde(skip)]
    pub launch_year: Option<i32>,
}

impl Spaceship {
    pub fn parts(&self) -> u32 {
        self.structurals as u32 + self.components as u32 + self.modules as u32
    }
    pub fn launched(&self) -> bool {
        self.launched_turn.is_some()
    }
    /// Part type at grid cell (x, y), None when empty or outside the grid.
    pub fn part_at(&self, x: i32, y: i32) -> Option<u8> {
        if !(0..crate::game::spaceship::GRID_W as i32).contains(&x) || !(0..crate::game::spaceship::GRID_H as i32).contains(&y) {
            return None;
        }
        self.grid[x as usize][y as usize].checked_sub(1)
    }
    /// Recompute the part counters from the grid.
    pub fn recount(&mut self) {
        let mut n = [0u8; 3];
        for t in self.grid.iter().flatten().filter_map(|c| c.checked_sub(1)) {
            n[crate::game::spaceship::kind_of(t) as usize] += 1;
        }
        [self.structurals, self.components, self.modules] = n;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct PlayerStats {
    pub units_built: u32,
    pub units_lost: u32,
    pub units_killed: u32,
    pub cities_captured: u32,
    pub cities_lost: u32,
    pub techs_stolen: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Player {
    pub slot: Slot,
    /// Civilisation id (index into rules::CIVS), 0 = barbarians.
    pub civ: u8,
    pub leader: String,
    pub human: bool,
    /// Participates in this game.
    pub in_game: bool,
    pub alive: bool,
    pub govt: Govt,
    /// Turns of anarchy remaining before a government can be chosen.
    pub anarchy_turns: u8,
    pub gold: i32,
    /// Tax rate in percent (multiple of 10). Science = 100 - tax - lux.
    pub tax: u8,
    pub lux: u8,
    /// Bit per AdvanceId (0..=71).
    pub advances: u128,
    pub researching: Option<AdvanceId>,
    pub beakers: i32,
    pub future_techs: u8,
    /// Advances learned since the start plus one (CIV.EXE ds:0xe898); sets the research cost.
    /// Stored after the game body in version-3 saves.
    #[serde(skip)]
    pub research_count: u16,
    /// How each advance was gained (CIV.EXE es:0x1d48): -1 not recorded (not known, or known
    /// from the start), 0 discovered, -2 Great Library, else the slot it was taken from.
    /// Stored after the game body in version-4 saves.
    #[serde(skip)]
    pub advance_source: Vec<i8>,
    pub capital: Option<CityId>,
    pub relations: [Relation; 8],
    /// Bit per slot: we have an embassy with them.
    pub embassies: u8,
    /// Attitude memory of AI towards each slot (-100..100).
    pub attitude: [i8; 8],
    /// Turn of last audience with each slot.
    pub last_audience: [u32; 8],
    pub spaceship: Spaceship,
    /// Consecutive peace turns after 1 AD (score).
    pub peace_turns: u32,
    /// Palace additions awarded so far.
    pub palace_level: u8,
    /// The palace sections (`Palace::encode`).
    pub palace_parts: Vec<u8>,
    /// Next index into the civ's city name list.
    pub city_name_cursor: u8,
    pub stats: PlayerStats,
    /// Turn the player was destroyed (if any).
    pub destroyed_turn: Option<u32>,
    /// The player has been told about this tech's special advisor hint already (bit per hint).
    pub hints_shown: u32,
    /// Score frozen when the game ended for this player.
    pub final_score: Option<i32>,
    /// Number of times the player's cities rioted under democracy (civil war bookkeeping).
    pub senate_blocks: u8,
    /// Cities that already sold an improvement this turn (one sale per city per turn).
    #[serde(default)]
    pub sold_cities: Vec<CityId>,
}

impl Player {
    pub fn new(slot: Slot, civ: u8, human: bool) -> Player {
        let c = &crate::rules::CIVS[civ as usize];
        Player {
            slot,
            civ,
            leader: c.leader.to_string(),
            human,
            in_game: true,
            alive: true,
            govt: Govt::Despotism,
            anarchy_turns: 0,
            gold: 0,
            tax: 50,
            lux: 0,
            advances: 0,
            researching: None,
            beakers: 0,
            future_techs: 0,
            research_count: 1,
            advance_source: vec![-1; 72],
            capital: None,
            relations: [Relation::NoContact; 8],
            embassies: 0,
            attitude: [0; 8],
            last_audience: [0; 8],
            spaceship: Spaceship::default(),
            peace_turns: 0,
            palace_level: 0,
            palace_parts: crate::game::palace::Palace::default().encode(),
            city_name_cursor: 0,
            stats: PlayerStats::default(),
            destroyed_turn: None,
            hints_shown: 0,
            final_score: None,
            senate_blocks: 0,
            sold_cities: Vec::new(),
        }
    }

    pub fn knows(&self, a: AdvanceId) -> bool {
        self.advances & (1u128 << a) != 0
    }
    pub fn learn(&mut self, a: AdvanceId) {
        self.advances |= 1u128 << a;
    }
    pub fn advance_source(&self, a: AdvanceId) -> i8 {
        self.advance_source.get(a as usize).copied().unwrap_or(-1)
    }
    pub fn forget(&mut self, a: AdvanceId) {
        self.advances &= !(1u128 << a);
    }
    pub fn palace(&self) -> crate::game::palace::Palace {
        crate::game::palace::Palace::decode(&self.palace_parts)
    }
    pub fn set_palace(&mut self, p: &crate::game::palace::Palace) {
        self.palace_parts = p.encode();
    }
    pub fn advance_count(&self) -> u32 {
        (self.advances & ((1u128 << 67) - 1)).count_ones()
    }
    pub fn known_advances(&self) -> Vec<AdvanceId> {
        (0..72u8).filter(|a| self.knows(*a) && crate::rules::is_real(*a)).collect()
    }
    pub fn sci(&self) -> u8 {
        100u8.saturating_sub(self.tax).saturating_sub(self.lux)
    }
    pub fn civ(&self) -> &'static crate::rules::Civ {
        &crate::rules::CIVS[self.civ as usize]
    }
    pub fn adjective(&self) -> &'static str {
        self.civ().adjective
    }
    pub fn plural(&self) -> &'static str {
        self.civ().plural
    }
    pub fn title(&self) -> &'static str {
        let g = crate::rules::GOVERNMENTS[self.govt as usize];
        if self.civ().female { g.title_f } else { g.title_m }
    }
    pub fn relation(&self, other: Slot) -> Relation {
        self.relations[other as usize]
    }
    pub fn at_war(&self, other: Slot) -> bool {
        self.relations[other as usize] == Relation::War
    }
    pub fn has_contact(&self, other: Slot) -> bool {
        self.relations[other as usize] != Relation::NoContact
    }
    pub fn has_embassy(&self, other: Slot) -> bool {
        self.embassies & (1 << other) != 0
    }
    pub fn is_barbarian(&self) -> bool {
        self.slot == 0
    }
}
