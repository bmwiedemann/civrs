//! Things that happen during turn processing which the UI must show, and decisions the
//! engine needs from a player before it can continue.

use super::city::Production;
use super::map::{CityId, Slot, UnitId};
use crate::rules::{AdvanceId, Govt, UnitTypeId, WonderId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum EndReason {
    /// All rival civilisations destroyed.
    Conquest,
    /// Human civilisation destroyed.
    Destroyed,
    /// Spaceship reached Alpha Centauri (slot of winner).
    Spaceship(Slot),
    /// Reached the final year of the difficulty level.
    TimeLimit,
    Retired,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Disaster {
    Plague,
    Flood,
    Volcano,
    Famine,
    Fire,
    Pirates,
}

impl Disaster {
    /// KING.TXT section name.
    pub fn section(self) -> &'static str {
        match self {
            Disaster::Plague => "D1",
            Disaster::Flood => "D2",
            Disaster::Volcano => "D3",
            Disaster::Famine => "D4",
            Disaster::Fire => "D5",
            Disaster::Pirates => "D6",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum HutOutcome {
    Gold(i32),
    Advance(AdvanceId),
    Mercenaries(UnitTypeId),
    Barbarians,
    Empty,
    Settlers,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum GameEvent {
    CityGrew(CityId),
    CityShrank(CityId),
    Famine(CityId),
    /// A unit was disbanded for lack of food/shields.
    UnitDisbanded { unit_kind: UnitTypeId, city: CityId, reason: String },
    ItemBuilt { city: CityId, item: Production },
    /// Production could not be completed (e.g. wonder built elsewhere, settlers in size-1 city).
    ProductionBlocked { city: CityId, item: Production },
    WonderBuilt { city: CityId, wonder: WonderId, owner: Slot },
    WonderObsolete { wonder: WonderId, owner: Slot },
    Disorder(CityId),
    DisorderEnded(CityId),
    Celebration(CityId),
    CelebrationEnded(CityId),
    AdvanceDiscovered { slot: Slot, adv: AdvanceId },
    /// Advance acquired by other means (hut, Great Library, capture, diplomacy, theft).
    AdvanceAcquired { slot: Slot, adv: AdvanceId, source: String },
    UnitLost { slot: Slot, kind: UnitTypeId, at: (i32, i32), reason: String },
    UnitVeteran(UnitId),
    CityCaptured { city: CityId, by: Slot, from: Slot, gold: i32, adv: Option<AdvanceId> },
    CityDestroyed { name: String, at: (i32, i32), by: Slot, from: Slot },
    CivilWar { slot: Slot, new_slot: Slot },
    CivDestroyed { slot: Slot, by: Slot },
    BarbariansLanded { at: (i32, i32), near_city: Option<CityId>, target: Slot },
    BarbarianUprising { at: (i32, i32), target: Slot },
    BarbarianLeaderCaptured { slot: Slot, gold: i32 },
    Pollution { city: CityId, at: (i32, i32) },
    GlobalWarming,
    Disaster { city: CityId, kind: Disaster },
    Revolution { slot: Slot },
    AnarchyEnded { slot: Slot },
    GovernmentChanged { slot: Slot, govt: Govt },
    /// No longer produced (buildings that cannot be paid for are sold); kept so that older
    /// saves with one pending still load.
    LowFunds { slot: Slot },
    ImprovementSold { city: CityId, improvement: u8, gold: i32 },
    SpaceshipLaunched(Slot),
    SpaceshipArrived(Slot),
    SpaceshipLost(Slot),
    PalaceAddition(Slot),
    PeaceTreaty { a: Slot, b: Slot },
    WarDeclared { a: Slot, b: Slot },
    ContactMade { a: Slot, b: Slot },
    /// `goods` indexes `turn::CARAVAN_GOODS`.
    TradeRouteMade { city: CityId, with: CityId, bonus: i32, goods: u8 },
    HutEntered { unit: UnitId, slot: Slot, outcome: HutOutcome },
    CityFounded { city: CityId },
    /// Advisor hint keyed by HELP.TXT section.
    Hint(String),
    /// Headline for the "newspaper" style news (KING.TXT NEWS sections).
    News { section: String, rplc1: String, rplc2: String },
    UnitExpired { slot: Slot, kind: UnitTypeId, at: (i32, i32) },
    /// A nuclear weapon detonated at `at` (city hit, if any).
    NuclearStrike { at: (i32, i32), by: Slot, city: Option<CityId> },
    /// `slot` established an embassy with `with`.
    EmbassyEstablished { slot: Slot, with: Slot },
    GameEnded(EndReason),
    TurnStarted(u32),
    /// The engine needs a decision from the human; `TurnRunner` turns this into a
    /// `TurnStep::Decision` in order with the surrounding events.
    NeedsDecision(Decision),
    /// Advisor hint about a tile worked by `city` (HELP.TXT `$US` = the city's name).
    HintAt { section: String, city: CityId },
    /// The Nuclear Plant of a city in disorder melted down.
    Meltdown { city: CityId },
    /// The treasury could not pay for a building, which was sold.
    CantMaintain { city: CityId, improvement: u8 },
}

/// A choice the engine needs from the human player before continuing.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum Decision {
    ChooseResearch { slot: Slot, options: Vec<AdvanceId> },
    ChooseGovernment { slot: Slot, options: Vec<Govt> },
    /// Opens the city screen because production completed / needs attention.
    ProductionChoice { city: CityId, reason: String },
    /// Treasury empty: pick an improvement to sell (candidates) or let the engine choose.
    /// No longer produced (CIV.EXE sells without asking); kept so that older saves with one
    /// pending still load.
    SellForUpkeep { slot: Slot, city: CityId, candidates: Vec<u8> },
    /// An AI leader requests an audience.
    Diplomacy(super::diplomacy::DiplomacyState),
    /// Game over for scoring purposes: keep playing?
    KeepPlaying { reason: EndReason },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum DecisionReply {
    Choice(usize),
    Yes,
    No,
    Cancel,
}
