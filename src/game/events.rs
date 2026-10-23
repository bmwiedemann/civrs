//! Things that happen during turn processing which the UI must show, and decisions the
//! engine needs from a player before it can continue.

use super::city::Production;
use super::map::{CityId, Slot, UnitId};
use crate::rules::{AdvanceId, Govt, ImprovementId, UnitTypeId, WonderId};
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
    /// The improvement burnt down.
    Fire(ImprovementId),
    Pirates,
    /// The improvement destroyed.
    Earthquake(ImprovementId),
    /// "Rioting in" / "Corruption in" / "Scandal in" (`wording` 0..=2); the improvement the
    /// citizens demand, None for lower taxes.
    Unrest { wording: u8, demand: Option<ImprovementId> },
}

impl Disaster {
    /// KING.TXT section name (the earthquake and unrest texts are in CIV.EXE).
    pub fn section(self) -> Option<&'static str> {
        Some(match self {
            Disaster::Plague => "D1",
            Disaster::Flood => "D2",
            Disaster::Volcano => "D3",
            Disaster::Famine => "D4",
            Disaster::Fire(_) => "D5",
            Disaster::Pirates => "D6",
            Disaster::Earthquake(_) | Disaster::Unrest { .. } => return None,
        })
    }
}

/// What a goody hut held (CIV.EXE `hut_outcome` 0x0978d).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum HutOutcome {
    /// "valuable metal deposits worth 50$".
    Gold(i32),
    /// "scrolls of ancient wisdom" with this advance.
    Advance(AdvanceId),
    /// "a friendly tribe of skilled mercenaries".
    Mercenaries(UnitTypeId),
    /// "You have unleashed a horde of barbarians!"
    Barbarians,
    /// No longer produced; kept so that older saves load.
    Empty,
    /// No longer produced; kept so that older saves load.
    Settlers,
    /// "an advanced tribe": a city founded on the square (none past the city limit).
    AdvancedTribe(Option<CityId>),
    /// The scrolls held no advance the civ could learn.
    EmptyScrolls,
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
    /// No longer produced (see `SpaceshipCancelled`); kept so that older saves load.
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
    /// A Diplomat of `by` bought a unit of `from`.
    UnitBribed { kind: UnitTypeId, from: Slot, by: Slot, at: (i32, i32) },
    /// "Residents of CITY admire the prosperity of OTHER." — the city went over from `from`
    /// to the owner of `admired` (CIV.EXE o 0x1beb0).
    CityDefected { city: CityId, admired: CityId, from: Slot },
    /// An AI civ put a part on its ship: "<Plural> build space ship." (CIV.EXE o 0x1842b).
    SpaceshipPartBuilt(Slot),
    /// The capital fell: "<Adj> spaceship returns to Earth." (launched) / "construction
    /// cancelled." (CIV.EXE 0x14364). Replaces `SpaceshipLost`.
    SpaceshipCancelled { slot: Slot, launched: bool },
    /// A computer Diplomat of `by` stole `adv` in `city` of `from` (CIV.EXE o 0x1d7c6; the
    /// human is told whoever was involved). New variants go last: saves keep loading.
    DiplomatStole { by: Slot, from: Slot, adv: AdvanceId, city: CityId },
    /// A computer Diplomat of `by` sabotaged `city`: `improvement` destroyed, or else the
    /// stored shields of `production` (its name) lost (CIV.EXE o 0x1d8d7 / 0x1d91e).
    CitySabotaged { city: CityId, by: Slot, improvement: Option<u8>, production: String },
    /// "<Title> <Leader> plans retirement in 20 years." for the human `Slot`, 20 years
    /// before the end of the game (CIV.EXE 0x2a28).
    RetirementWarning(Slot),
    /// "Population decrease\nin CITY.": the human's city has more working squares than
    /// citizens at its turn and places its workers anew (CIV.EXE 0xd70d, ds:0x255f).
    PopulationDecrease(CityId),
    /// "Science Advisor: I recommend we develop ADV." before the human's research menu on
    /// Chieftain with Instant Advice (CIV.EXE 0x0bfc1).
    ScienceAdvice { slot: Slot, adv: AdvanceId },
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
    /// "Type?" for a finished SS Component (`kind` 1: Propulsion / Fuel) or SS Module (`kind`
    /// 2: Habitation / Life Support / Solar Panel) (CIV.EXE o 0x1853f).
    SpaceshipPart { slot: Slot, kind: u8 },
    /// The ship screen after a human's part, with its LAUNCH button when the ship can go:
    /// "Confirm spaceship launch." No Launch / Launch CONFIRMED! (CIV.EXE o 0x18842, 0x180fe);
    /// `Choice(1)` launches.
    SpaceshipLaunch { slot: Slot },
    /// "Select one...": the advance a human captor takes from `from` with a city
    /// (`steal_advance` CIV.EXE 0x14468); the menu cannot be cancelled.
    StealAdvance { slot: Slot, from: Slot, options: Vec<AdvanceId> },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum DecisionReply {
    Choice(usize),
    Yes,
    No,
    Cancel,
}
