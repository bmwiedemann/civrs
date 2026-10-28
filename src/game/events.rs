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

/// One ranked civilization of the historian's report.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistorianRow {
    pub slot: Slot,
    pub palace_level: i32,
    pub has_cities: bool,
    pub shown: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum GameEvent {
    CityGrew(CityId),
    CityShrank(CityId),
    /// No longer produced by the city turn (see `FoodExhausted`); kept so that older saves
    /// load.
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
    /// `adjective` and `civ` name the destroyed tribe: the slot may already hold its successor.
    CivDestroyed { slot: Slot, by: Slot, adjective: String, civ: u8 },
    BarbariansLanded { at: (i32, i32), near_city: Option<CityId>, target: Slot },
    BarbarianUprising { at: (i32, i32), target: Slot },
    BarbarianLeaderCaptured { slot: Slot, gold: i32 },
    Pollution { city: CityId, at: (i32, i32) },
    GlobalWarming,
    Disaster { city: CityId, kind: Disaster },
    Revolution { slot: Slot },
    AnarchyEnded { slot: Slot },
    GovernmentChanged { slot: Slot, govt: Govt },
    /// The *TAXES warning: the treasury is low and shrinking (`Game::treasury_warning`).
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
    /// `slot` established an embassy with `with` (no longer raised: the dossier shows it; the
    /// variant stays for saves with pending events).
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
    /// "The war between\nthe X and the\nY has ended.": two computer players at war lost
    /// contact; the human has an embassy with one of them (CIV.EXE 0x14aa4).
    WarEnded { a: Slot, b: Slot },
    /// "X and\nY cancel\npeace treaty." (CIV.EXE 0x15340): computer players `a` and `b`
    /// go to war; the human has an embassy with one of them.
    TreatyCancelled { a: Slot, b: Slot },
    /// "Sneak attack by\n<adjective> forces!" (CIV.EXE 0x19e9e): `by` attacked `on`, a human
    /// still at peace with it, or with a treaty it had dropped (`diplomacy::flags::SNEAK`).
    SneakAttack { by: Slot, on: Slot },
    /// "<Victims> cancel\npeace treaty\nwith <attackers>." (CIV.EXE 0x19f2c): a fight ended the
    /// treaty between `by` and `victim`, one of them the human.
    TreatyBroken { by: Slot, victim: Slot },
    /// The SCHISM note of HELP.TXT about `Slot` after an audience (CIV.EXE o 0x79c6).
    Schism(Slot),
    /// "Scientists alarmed by\nrising temperatures.\nGlobal Warming feared!" (CIV.EXE 0x3b2e):
    /// the warming counter reached 12 with more than 6 polluted squares.
    GlobalWarmingFeared,
    /// "Development of ADV\nmakes existing Barracks\nobsolete." (CIV.EXE 0xc6ea): the human
    /// learned Gunpowder or Combustion and its Barracks are gone.
    BarracksObsolete { slot: Slot, adv: AdvanceId },
    /// `historian_report` (o 0x10b57): historian 0..3 (`rules::HISTORIANS`), category 0..4
    /// (`rules::DEMOGRAPHIC_CATEGORIES`) and the civilizations from best to worst, each with
    /// its palace level (`tri5_level` of its cities' size + happy - unhappy), whether it has
    /// cities and whether the human may see its row.
    HistorianReport { historian: u8, category: u8, rows: Vec<HistorianRow> },
    /// The game ends again after the human kept playing (ds:0x4cd8 bit 0x100): the ending
    /// without `end_game_scores` (0x2bc1).
    GameEndedAgain(EndReason),
    /// "Food storage exhausted\nin CITY!\nSettlers lost." / "...\nFamine feared." (CIV.EXE
    /// 0xd0a4, ds:0x24f5 / 0x2510 / 0x2522), shown to the human owner: the city's store was
    /// negative at its turn. `name` because a city of size 1 starves to death.
    FoodExhausted { city: CityId, name: String, owner: Slot, settlers_lost: bool },
    /// "CITY requires an AQUEDUCT\nfor further growth." (CIV.EXE 0xd221, ds:0x2534): the city
    /// grew past size 10 without one and shrank back.
    AqueductNeeded(CityId),
    /// "SDI protects <city>." at (80,100), for everyone (`nuclear_attack` CIV.EXE 0x1a51f):
    /// the city's units survive a nuclear blast.
    SdiProtects { city: CityId },
    /// "Discontented citizens of\nCITY revolt:\nGovernment Collapses!" (CIV.EXE 0x127d6, the
    /// human's newspaper): a Democracy's city stayed in disorder for a second turn.
    GovernmentCollapsed { slot: Slot, city: CityId },
    /// "<Plural> rebel!\nCivil War in\n<city>.\n<Adjective> influence\nsuspected." at
    /// (100,32), or "<Plural> declare\nindependence in\n<city>" when `to` is the barbarians
    /// (`city_revolt` CIV.EXE o 0x1e1d4, ds:0x533c / 0x5322): the city of `from` was incited by
    /// a Diplomat of `to` or defected to it; shown when the human is one of them or has an
    /// embassy with either.
    CityRebels { city: CityId, from: Slot, to: Slot },
    /// KING.TXT *UMAX, once a turn (ds:0xd760): the human's city finished a unit while the
    /// civilization had all its 127 unit slots in use (create_unit 0x8d24).
    UnitLimit(Slot),
    /// "Civil Disorder in\nCITY! Mayor\nflees in panic." as a plain message (CIV.EXE 0x126a5):
    /// a further turn of disorder (`first` false), or a first one after this turn's riot
    /// animation (`Disorder`).
    DisorderOngoing { city: CityId, first: bool },
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
