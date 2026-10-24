use super::map::{CityId, Slot};
use crate::rules::{AdvanceId, Govt};
use serde::{Deserialize, Serialize};

/// How one civilization stands towards another, as CIV.EXE's screens put it: a summary of
/// the diplomacy flags `Player::diplo` (`diplomacy::flags`): `Peace` with the treaty bit,
/// `War` with contact and no treaty, else `NoContact`. Saves before version 8 stored only
/// this (`Player::legacy_relations`).
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
    /// The relations of saves before version 8 (read only when loading those).
    pub legacy_relations: [Relation; 8],
    /// The embassies (bit per slot) of saves before version 8 (read only when loading those).
    pub legacy_embassies: u8,
    /// No longer used (saves before version 8: an attitude memory of civrs' own).
    pub legacy_attitude: [i8; 8],
    /// No longer used (saves before version 8: the turn of the last audience).
    pub legacy_last_audience: [u32; 8],
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
    /// CIV.EXE's diplomacy flags ds:0xe498[slot][other] (`diplomacy::flags`); stored after the
    /// game body in version-8 saves, derived from `legacy_relations` for older ones.
    #[serde(skip)]
    pub diplo: [u16; 8],
    /// ds:0xd2e4[slot]: the turn of the computer civilization's last talk with the human, which
    /// keeps it from asking again for 16 turns (-1 never; -2 the human took one of its cities).
    /// Stored after the game body in version-8 saves.
    #[serde(skip)]
    pub last_contact: i32,
    /// ds:0xb3be[0x38 * civ]: units of each type, as CIV.EXE counts them: up in create_unit
    /// (0x8c5d), down in delete_unit (0x8e63), and zeroed by `civ_init` (o5 0x4fc7) after the
    /// start units are placed, so those are not counted (and their loss takes it below zero).
    /// Read by the F2 report, the production advice and the Settlers hints. Stored after the
    /// game body in version-13 saves.
    #[serde(skip)]
    pub unit_counter: [i16; 28],
    /// es:0x253c[0x38 * civ] (far segment ds:0x643c): units of each type lost in fights,
    /// counted by delete_unit while an attack is resolved; the F2 "MILITARY LOSSES" page.
    /// Stored after the game body in version-13 saves.
    #[serde(skip)]
    pub units_lost_by_type: [i16; 28],
    /// Score frozen when the game ended for this player.
    pub final_score: Option<i32>,
    /// Number of times the player's cities rioted under democracy (civil war bookkeeping).
    pub senate_blocks: u8,
    /// Cities that already sold an improvement this turn (one sale per city per turn).
    #[serde(default)]
    pub sold_cities: Vec<CityId>,
    /// The tribe's own plural and adjective, when the human named it (`ask_tribe_name`,
    /// overlay 23 +0x173, writes ds:0x1992 / ds:0x1982). Stored after the game body in
    /// version-14 saves.
    #[serde(skip)]
    pub tribe_names: Option<(String, String)>,
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
            legacy_relations: [Relation::NoContact; 8],
            legacy_embassies: 0,
            legacy_attitude: [0; 8],
            legacy_last_audience: [0; 8],
            spaceship: Spaceship::default(),
            peace_turns: 0,
            palace_level: 0,
            palace_parts: crate::game::palace::Palace::default().encode(),
            city_name_cursor: 0,
            stats: PlayerStats::default(),
            destroyed_turn: None,
            hints_shown: 0,
            diplo: [0; 8],
            last_contact: -1,
            unit_counter: [0; 28],
            units_lost_by_type: [0; 28],
            final_score: None,
            senate_blocks: 0,
            sold_cities: Vec::new(),
            tribe_names: None,
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
    pub fn adjective(&self) -> &str {
        self.tribe_names.as_ref().map_or(self.civ().adjective, |t| t.1.as_str())
    }
    pub fn plural(&self) -> &str {
        self.tribe_names.as_ref().map_or(self.civ().plural, |t| t.0.as_str())
    }
    pub fn title(&self) -> &'static str {
        let g = crate::rules::GOVERNMENTS[self.govt as usize];
        if self.civ().female { g.title_f } else { g.title_m }
    }
    /// The diplomacy flags towards `other` (ds:0xe498[slot][other]).
    pub fn flags(&self, other: Slot) -> u16 {
        self.diplo[other as usize]
    }
    /// Summary of the flags (see `Relation`).
    pub fn relation(&self, other: Slot) -> Relation {
        use crate::game::diplomacy::flags::*;
        let f = self.flags(other);
        if f & PEACE != 0 {
            Relation::Peace
        } else if f & CONTACT != 0 {
            Relation::War
        } else {
            Relation::NoContact
        }
    }
    /// CIV.EXE's war: contact and no treaty (`flags & 3 == 1`, e.g. 0x15495, the
    /// intelligence report 0x14279).
    pub fn at_war(&self, other: Slot) -> bool {
        use crate::game::diplomacy::flags::*;
        self.flags(other) & (CONTACT | PEACE) == CONTACT
    }
    /// A peace treaty with `other`, from this side (bit 1).
    pub fn treaty(&self, other: Slot) -> bool {
        self.flags(other) & crate::game::diplomacy::flags::PEACE != 0
    }
    pub fn has_contact(&self, other: Slot) -> bool {
        self.flags(other) & crate::game::diplomacy::flags::CONTACT != 0
    }
    /// An embassy with `other` (bit 6).
    pub fn has_embassy(&self, other: Slot) -> bool {
        self.flags(other) & crate::game::diplomacy::flags::EMBASSY != 0
    }
    pub fn is_barbarian(&self) -> bool {
        self.slot == 0
    }
}

/// `ask_tribe_name`'s names for a typed plural: the plural itself, and the adjective made by
/// dropping a final 's' and cutting it to 10 characters.
pub fn tribe_names(plural: &str) -> (String, String) {
    let adj = plural.strip_suffix('s').unwrap_or(plural);
    (plural.to_string(), adj.chars().take(10).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn typed_tribe_names_drop_the_plural_s_for_the_adjective() {
        assert_eq!(super::tribe_names("Vandals"), ("Vandals".into(), "Vandal".into()));
        assert_eq!(super::tribe_names("Ostrogothics"), ("Ostrogothics".into(), "Ostrogothi".into()));
        assert_eq!(super::tribe_names("Inuit"), ("Inuit".into(), "Inuit".into()));
    }
}
