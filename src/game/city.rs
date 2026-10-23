use super::map::{CityId, Slot};
use crate::rules::{ImprovementId, UnitTypeId, WonderId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Production {
    Unit(UnitTypeId),
    Improvement(ImprovementId),
    Wonder(WonderId),
}

impl Production {
    pub fn name(&self) -> &'static str {
        match self {
            Production::Unit(u) => crate::rules::UNITS[*u as usize].name,
            Production::Improvement(i) => crate::rules::IMPROVEMENTS[*i as usize].name,
            Production::Wonder(w) => crate::rules::WONDERS[*w as usize].name,
        }
    }
    /// Cost in shields.
    pub fn cost(&self) -> i32 {
        10 * match self {
            Production::Unit(u) => crate::rules::UNITS[*u as usize].cost as i32,
            Production::Improvement(i) => crate::rules::IMPROVEMENTS[*i as usize].cost as i32,
            Production::Wonder(w) => crate::rules::WONDERS[*w as usize].cost as i32,
        }
    }
    pub fn class(&self) -> u8 {
        match self {
            Production::Unit(_) => 0,
            Production::Improvement(_) => 1,
            Production::Wonder(_) => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Specialist {
    Entertainer,
    Taxman,
    Scientist,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct City {
    pub id: CityId,
    pub owner: Slot,
    /// Original founder (for score / civil war).
    pub founder: Slot,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub size: u8,
    /// Food stored.
    pub food: i32,
    /// Shields accumulated toward current production.
    pub shields: i32,
    pub production: Production,
    /// Bit per ImprovementId (1..=24).
    pub improvements: u32,
    /// Bit per WonderId (1..=21).
    pub wonders: u32,
    /// Bit per CITY_RADIUS index (1..=20) of worked tiles (index 0, the centre, is implicit).
    pub worked: u32,
    pub specialists: Vec<Specialist>,
    pub trade_routes: Vec<CityId>,
    pub disorder: bool,
    pub disorder_turns: u8,
    pub celebrating: bool,
    /// Something was bought this turn (no further changes allowed).
    pub bought: bool,
    /// Unused: CIV.EXE has no penalty for changing production (kept for the save layout).
    pub changed_class: bool,
    pub auto_build: bool,
    pub founded_turn: u32,
    /// Accumulated pollution chance bookkeeping.
    pub pollution_pending: u8,
    /// Shields contributed by caravans towards a wonder.
    pub caravan_shields: i32,
}

impl City {
    pub fn pos(&self) -> (i32, i32) {
        (self.x, self.y)
    }
    pub fn has_improvement(&self, id: ImprovementId) -> bool {
        self.improvements & (1u32 << id) != 0
    }
    pub fn add_improvement(&mut self, id: ImprovementId) {
        self.improvements |= 1u32 << id;
    }
    pub fn remove_improvement(&mut self, id: ImprovementId) {
        self.improvements &= !(1u32 << id);
    }
    pub fn has_wonder(&self, id: WonderId) -> bool {
        self.wonders & (1u32 << id) != 0
    }
    pub fn add_wonder(&mut self, id: WonderId) {
        self.wonders |= 1u32 << id;
    }
    pub fn works(&self, radius_idx: u8) -> bool {
        radius_idx == 0 || self.worked & (1u32 << radius_idx) != 0
    }
    pub fn set_worked(&mut self, radius_idx: u8, on: bool) {
        if radius_idx == 0 {
            return;
        }
        if on {
            self.worked |= 1u32 << radius_idx;
        } else {
            self.worked &= !(1u32 << radius_idx);
        }
    }
    pub fn workers(&self) -> u32 {
        self.worked.count_ones()
    }
    pub fn improvement_list(&self) -> Vec<ImprovementId> {
        (1..=24u8).filter(|i| self.has_improvement(*i)).collect()
    }
    pub fn wonder_list(&self) -> Vec<WonderId> {
        (1..=21u8).filter(|i| self.has_wonder(*i)).collect()
    }
    /// Food needed to grow.
    pub fn granary_size(&self) -> i32 {
        (self.size as i32 + 1) * 10
    }
    /// Population display figure (Civ1: sum of 10000 * i for i in 1..=size).
    pub fn population(&self) -> i64 {
        let n = self.size as i64;
        n * (n + 1) / 2 * 10_000
    }
}
