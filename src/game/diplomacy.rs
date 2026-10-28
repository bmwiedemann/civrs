//! Diplomacy state shared by the engine, the AI and the diplomacy screen: CIV.EXE's diplomacy
//! flags (`flags`, ds:0xe498), contact and the computer players' war and peace between
//! themselves (segment 0x1495), the effects of fights and treaties, and the audience
//! description (`DiplomacyState`) whose fields substitute `$RPLC1 $RPLC2 $BUCKS $US $THEM` in
//! the KING.TXT section named by `Proposal`.
//!
//! The audience itself (CIV.EXE overlay 6, `diplomacy_meeting`) lives in `crate::ai::diplo_ai`.
//!
//! Every pair of civilizations has two 16-bit flag words, one per direction:
//! `Player::diplo[other]` = ds:0xe498[16 * slot + 2 * other]. The two words of a pair can
//! differ: a computer player that breaks a treaty unilaterally drops the peace bit from its own
//! word only (`flags::SNEAK`).

use super::map::{Map, Slot};
use super::{Game, GameEvent, Relation};
use crate::rules::{self, unit_ids, wonder_ids as won, Govt};
use serde::{Deserialize, Serialize};

/// The bits of a diplomacy word (decoded from all readers and writers of ds:0xe498).
pub mod flags {
    /// Contact. Set both ways when land units meet (0x1508d), dropped every 16 turns unless the
    /// two met in between (0x14a3f); without `PEACE` it means war ("At WAR", `flags & 3 == 1`).
    /// A civ that leads the world after turn 200 gets it from every computer player without a
    /// treaty (0x3328): war.
    pub const CONTACT: u16 = 0x01;
    /// Peace treaty (0x15386 / 0x153f7 with flag 2). Units of a civ never attack the units of a
    /// civ its own word is at peace with; the human may only after "Break treaty." (0x1a3b4).
    pub const PEACE: u16 = 0x02;
    /// Alliance: shown as ": Allied" with `PEACE` by the civ status screen (o 0x123ca) and
    /// tested by `ai_wants_war` and the audience, but never set for a pair in v474; clearing
    /// `PEACE` clears it too. The word [civ][0] uses this bit as the "government collapsed"
    /// revolution flag (0x12820, 0x1495a, 0x14df7), which civrs does not keep here.
    pub const ALLIANCE: u16 = 0x04;
    /// Vendetta: [a][b] means a holds a grudge against b, which broke a treaty with a by
    /// attacking (0x19faf) or inciting a city (o 0x1db10), or cancelled its treaty with a at a
    /// third party's request (o 0x6cc2, 0x14c09). Computer players with a vendetta between them
    /// neither make war nor peace (0x15183); a vendetta makes a leader hostile in audiences.
    /// Cleared towards everybody when a's government changes (0x15079).
    pub const VENDETTA: u16 = 0x08;
    /// Met since the last 16-turn contact check (set with `CONTACT` by 0x1508d).
    pub const MET: u16 = 0x10;
    /// The two have fought (0x19e43) since the last audience (o 0x797c) or treaty (0x153ed);
    /// [civ][j] is cleared by civ's 32-turn HELPME/CRUSADE check (0x14c29).
    pub const FOUGHT: u16 = 0x20;
    /// Embassy: [a][b] means a has an embassy with b (o 0x1d6ce).
    pub const EMBASSY: u16 = 0x40;
    /// [ai][human]: the computer player had nuclear weapons at an audience (o 0x6631); also set
    /// with `VENDETTA` on [victim][attacker] by a nuclear attack (0x1a5ba). Read only by tests
    /// that cannot fail any more and by the AI's target choice.
    pub const NUCLEAR: u16 = 0x80;
    /// [ai][human]: the computer player wanted war at an audience but was held back by the
    /// treaty (o 0x796a), or the human leads the world (0x335e). Counts as war for the AI's
    /// planning (0x15dd9); each turn of the civ with probability 1/3 it becomes `SNEAK`.
    pub const BREAK: u16 = 0x100;
    /// [a][b]: a has dropped the treaty with b on its own (0x14a34): b still keeps the peace
    /// until a attacks ("Sneak attack by ... forces!", 0x19e90).
    pub const SNEAK: u16 = 0x200;
}

use flags::*;

/// What an audience is about; names mirror KING.TXT sections. New variants go last: saves
/// keep loading.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Proposal {
    /// AI offers to exchange knowledge (`rplc1` = advance they lack... see `diplo_ai`).
    Exchange,
    /// AI demands we cancel our treaty with `other` (`rplc1` = "demand"/"request").
    Cancel,
    /// AI demands tribute (`bucks`).
    Tribute,
    /// AI demands an advance (`rplc1`, `advance`).
    TakeCiv,
    /// AI proposes peace (Reject / Accept).
    Peace,
    /// AI spares us despite our attitude (no choices).
    Attitude,
    /// Declarations of war (no choices).
    Provoke,
    Reject,
    Rid,
    /// AI stays at war after we rejected peace (no choices).
    Mobilize,
    /// "You respond: 'We... welcome peace / have a military proposal / demand tribute".
    Threat,
    /// AI (at peace) asks us to join its war against `other` for `bucks`.
    HelpMe,
    /// AI (at peace) asks us to join its war against `other`.
    Crusade,
    /// AI at war offers an advance for peace.
    GiveCiv,
    /// AI at war offers gold for peace.
    GiveCash,
    /// AI pays tribute we demanded (no choices).
    GiveMore,
    /// AI at war offers all its gold and knowledge for peace.
    Grovel,
    /// Treaty text after peace was agreed (no choices).
    Treaty,
    /// Wonder demonstration (no choices).
    Gape,
    /// The AI is already at war with the civ we proposed to attack (no choices).
    AtWar,
    /// The AI has no contact with the civ we proposed to attack (no choices).
    NoContact,
    /// AI offers to fight `other` for `bucks` (Never mind / Yes, pay).
    Mercenary,
    /// Our treasury cannot cover a promise (no choices).
    Unfortunate,
    /// "An emissary from the <civ> wishes to speak with you. Will you receive him?" over the
    /// map (o 0x7a82): Yes / No / Check Intelligence (with an embassy).
    Emissary,
    /// "Check Intelligence": the UI shows the intelligence report and answers 0.
    Intelligence,
    /// A text of its own (`DiplomacyState::text`, menu `DiplomacyState::menu`).
    Message,
}

impl Proposal {
    pub fn section(self) -> &'static str {
        match self {
            Proposal::Exchange => "EXCHANGE",
            Proposal::Cancel => "CANCEL",
            Proposal::Tribute => "TRIBUTE",
            Proposal::TakeCiv => "TAKECIV",
            Proposal::Peace => "PEACE",
            Proposal::Attitude => "ATTITUDE",
            Proposal::Provoke => "PROVOKE",
            Proposal::Reject => "REJECT",
            Proposal::Rid => "RID",
            Proposal::Mobilize => "MOBILIZE",
            Proposal::Threat => "THREAT",
            Proposal::HelpMe => "HELPME",
            Proposal::Crusade => "CRUSADE",
            Proposal::GiveCiv => "GIVECIV",
            Proposal::GiveCash => "GIVECASH",
            Proposal::GiveMore => "GIVEMORE",
            Proposal::Grovel => "GROVEL",
            Proposal::Treaty => "TREATY",
            Proposal::Gape => "GAPE",
            Proposal::AtWar => "ATWAR",
            Proposal::NoContact => "NOCONTACT",
            Proposal::Mercenary => "MERCENARY",
            Proposal::Unfortunate => "UNFORTUNATE",
            Proposal::Emissary | Proposal::Intelligence | Proposal::Message => "",
        }
    }

    /// Number of `_` menu choices the KING.TXT section offers (0 = just acknowledge).
    pub fn choices(self) -> usize {
        match self {
            Proposal::Threat => 3,
            Proposal::Exchange
            | Proposal::Cancel
            | Proposal::Tribute
            | Proposal::TakeCiv
            | Proposal::Peace
            | Proposal::HelpMe
            | Proposal::Crusade
            | Proposal::GiveCiv
            | Proposal::GiveCash
            | Proposal::Grovel
            | Proposal::Mercenary
            | Proposal::Emissary => 2,
            _ => 0,
        }
    }

    /// The audience ends with a declaration of war by the AI.
    pub fn is_war_declaration(self) -> bool {
        matches!(self, Proposal::Provoke | Proposal::Reject | Proposal::Rid)
    }
}

/// One screen of an audience. The fields up to `stage` are what saves before version 8 stored;
/// the rest is not saved (a saved game never waits in the middle of an audience).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DiplomacyState {
    /// The AI leader in the audience.
    pub ai: Slot,
    /// The human.
    pub human: Slot,
    pub proposal: Proposal,
    pub rplc1: String,
    pub rplc2: String,
    pub bucks: i32,
    /// Advance involved (exchange / demand / gift).
    pub advance: Option<u8>,
    /// Third party (cancel treaty / crusade / help me / mercenary).
    pub other: Option<Slot>,
    /// The audience was asked for by the human (a Diplomat's "Meet with King").
    pub human_initiated: bool,
    /// Unused since version 8.
    pub stage: u8,
    /// `$US` (ds:0x30b8) when not the human's plural name.
    #[serde(skip)]
    pub us: String,
    /// `$THEM` (ds:0x30ba) when not the third party's or the AI's plural name.
    #[serde(skip)]
    pub them: String,
    /// The text of a `Proposal::Message` (lines, `\n` already split).
    #[serde(skip)]
    pub text: Vec<String>,
    /// Menu lines of a `Proposal::Message` or `Proposal::Emissary`.
    #[serde(skip)]
    pub menu: Vec<String>,
    /// Disabled menu lines (CIV.EXE ds:0xb276).
    #[serde(skip)]
    pub disabled: Vec<bool>,
    /// Shown in a plain box over the map, not in the throne room (the emissary was refused).
    #[serde(skip)]
    pub plain: bool,
    /// The war drums (tune 33) play over the anthem.
    #[serde(skip)]
    pub drums: bool,
    /// Where the audience stands (`diplo_ai`); None for a screen that ends it.
    #[serde(skip)]
    pub meeting: Option<Box<Meeting>>,
}

impl DiplomacyState {
    pub fn new(ai: Slot, human: Slot, proposal: Proposal) -> DiplomacyState {
        DiplomacyState {
            ai,
            human,
            proposal,
            rplc1: String::new(),
            rplc2: String::new(),
            bucks: 0,
            advance: None,
            other: None,
            human_initiated: false,
            stage: 0,
            us: String::new(),
            them: String::new(),
            text: Vec::new(),
            menu: Vec::new(),
            disabled: Vec::new(),
            plain: false,
            drums: false,
            meeting: None,
        }
    }

    /// `$US` substitution: the human civilisation's plural name unless set.
    pub fn us(&self, g: &Game) -> String {
        if self.us.is_empty() {
            g.civ_name(self.human).to_string()
        } else {
            self.us.clone()
        }
    }

    /// `$THEM` substitution: the third party when there is one, else the AI civilisation.
    pub fn them(&self, g: &Game) -> String {
        if self.them.is_empty() {
            g.civ_name(self.other.unwrap_or(self.ai)).to_string()
        } else {
            self.them.clone()
        }
    }

    /// Number of menu choices (0 = message only).
    pub fn choices(&self) -> usize {
        if self.menu.is_empty() {
            self.proposal.choices()
        } else {
            self.menu.len()
        }
    }
}

/// The steps of CIV.EXE's `diplomacy_meeting` (o 0x6375), and of the HELPME / CRUSADE offers
/// of 0x14b75, in the order they run; see `diplo_ai`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Step {
    #[default]
    Exchange,
    ExchangeShow,
    ExchangeTake,
    ExchangePick,
    AfterExchange,
    Cancel,
    CancelShow,
    Tribute,
    TributeShow,
    TakeCiv,
    TakeCivShow,
    Hostile,
    ProvokeShow,
    ProvokeRefused,
    RejectShow,
    RidShow,
    RidRefused,
    AfterHostile,
    PeaceShow,
    GiveCiv,
    GiveCash,
    Grovel,
    Mobilize,
    Attitude,
    AttitudeShow,
    Threat,
    ThreatShow,
    Finish,
    /// The end of the 0x14b75 offers (`diplomacy_close`).
    Close,
    HelpMeShow,
    CrusadeShow,
    Done,
}

/// What the screen on display asks.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Await {
    /// Acknowledge and go on with the step.
    #[default]
    Nothing,
    Ack(Step),
    /// The emissary question: received -> `then`, refused -> `refused`.
    Emissary { then: Step, refused: Step },
    /// "Check Intelligence" was shown: ask again.
    Intelligence { then: Step, refused: Step },
    Exchange,
    /// "Select one..." of these advances (the human's pick of an exchange).
    Pick(Vec<u8>),
    Cancel,
    Tribute,
    TakeCiv,
    Peace,
    GiveCiv,
    GiveCash,
    Grovel,
    Threat,
    /// "You must attack the evil:" one of these civilizations.
    Evil(Vec<Slot>),
    Mercenary { other: Slot, cost: i32 },
    HelpMe { other: Slot, bucks: i32 },
    Crusade { other: Slot },
}

/// The audience in progress: the locals of o 0x6375 (stack offsets in the comments).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Meeting {
    pub ai: Slot,
    /// [bp+0xc]: 0 the computer's emissary (contact, 0x1508d), 1 the human's Diplomat.
    pub mode: u8,
    /// Where the meeting happens ([bp+8], [bp+0xa]; the map is centred there).
    pub at: (i32, i32),
    pub step: Step,
    pub awaiting: Await,
    /// ds:0xd804 "the emissary question was asked / the portrait is up".
    pub asked: bool,
    /// [bp-0x34]: the last contact turn before this audience.
    pub last: i32,
    /// [bp-0x24]: the leader wants war.
    pub want: bool,
    /// [bp-0x10]: `want` as first decided.
    pub want_first: bool,
    /// [bp-8]: 0 conciliatory, 1 neutral, 2 demanding.
    pub mood: u8,
    /// [bp-0x2e]: the human's military threat to the leader's cities.
    pub threat: i32,
    /// [bp-6]: the tribute demanded (999 after a demand was made).
    pub tribute: i32,
    /// [bp-0x28]: war wanted but held back by the treaty.
    pub held_back: bool,
    /// [bp-4]: the human leads the world (0x3121's rule).
    pub human_leads: bool,
    /// [bp-0x30]: an enemy of the leader the human is at peace with.
    pub crusade: Option<Slot>,
    /// [bp-0xa]: the highest advance the leader knows and the human does not.
    pub offer: Option<u8>,
    /// [bp-0xc]: the advance of the human's the leader values most.
    pub demand: Option<u8>,
}

impl Game {
    // ---- the flags ------------------------------------------------------------------------

    /// ds:0xe498[a][b].
    pub fn diplo(&self, a: Slot, b: Slot) -> u16 {
        self.player(a).flags(b)
    }

    /// Set `flag` in [a][b] only.
    pub fn raise_flag(&mut self, a: Slot, b: Slot, flag: u16) {
        self.players[a as usize].diplo[b as usize] |= flag;
    }

    /// Clear `flag` in [a][b] only.
    pub fn drop_flag(&mut self, a: Slot, b: Slot, flag: u16) {
        self.players[a as usize].diplo[b as usize] &= !flag;
    }

    /// CIV.EXE 0x15386 `set_diplo_flag(a, b, flag)`: both ways; a treaty also forgets the
    /// fighting (`FOUGHT`). A new treaty goes into the replay.
    pub fn set_diplo_flag(&mut self, a: Slot, b: Slot, flag: u16) {
        if flag & PEACE != 0 && self.player(a).diplo[b as usize] & PEACE == 0 {
            self.replay_log(crate::game::replay::kind::PEACE, &[a << 4 | b]);
        }
        self.raise_flag(a, b, flag);
        self.raise_flag(b, a, flag);
        if flag == PEACE {
            self.drop_flag(a, b, FOUGHT);
            self.drop_flag(b, a, FOUGHT);
        }
    }

    /// CIV.EXE 0x153f7 `clear_diplo_flag(a, b, flag)`: both ways; the end of a treaty also ends
    /// an alliance. A broken treaty goes into the replay.
    pub fn clear_diplo_flag(&mut self, a: Slot, b: Slot, flag: u16) {
        if flag & PEACE != 0 && self.player(a).diplo[b as usize] & PEACE != 0 {
            self.replay_log(crate::game::replay::kind::WAR, &[a << 4 | b]);
        }
        let flag = if flag & PEACE != 0 { flag | ALLIANCE } else { flag };
        self.drop_flag(a, b, flag);
        self.drop_flag(b, a, flag);
    }

    pub fn relation(&self, a: Slot, b: Slot) -> Relation {
        self.player(a).relation(b)
    }

    /// `a`'s word has a peace treaty with `b`.
    pub fn treaty(&self, a: Slot, b: Slot) -> bool {
        self.player(a).treaty(b)
    }

    /// CIV.EXE's war: contact without a treaty in `a`'s word.
    pub fn at_war(&self, a: Slot, b: Slot) -> bool {
        a != b && self.player(a).at_war(b)
    }

    /// Units of `a` may fight units of `b`: anyone but a treaty partner in `a`'s own word
    /// (barbarians fight everybody). Contact does not matter: a computer player only meets a
    /// civ when their land units come next to each other (0x8491).
    pub fn hostile(&self, a: Slot, b: Slot) -> bool {
        a != b && (a == 0 || b == 0 || !self.treaty(a, b))
    }

    /// War for the computer's plans (0x15c06, 0x15dd9, 0x1745d): `flags & 3 == 1`, or a treaty
    /// it means to break (`BREAK`).
    pub fn planning_war(&self, a: Slot, b: Slot) -> bool {
        a != b && (self.at_war(a, b) || self.diplo(a, b) & BREAK != 0)
    }

    /// Set a relation both ways the way civrs used to (tests and old saves): a treaty is
    /// contact and peace, a war contact without peace.
    pub fn set_relation(&mut self, a: Slot, b: Slot, r: Relation) {
        for (x, y) in [(a, b), (b, a)] {
            let f = &mut self.players[x as usize].diplo[y as usize];
            *f &= !(CONTACT | PEACE | ALLIANCE | SNEAK | BREAK);
            *f |= match r {
                Relation::NoContact => 0,
                Relation::Peace => CONTACT | PEACE,
                Relation::War => CONTACT,
            };
        }
    }

    /// Contact without a treaty (both ways): what 0x1508d leaves after a first meeting.
    pub fn make_contact(&mut self, a: Slot, b: Slot) {
        if a == b || a == 0 || b == 0 {
            return;
        }
        self.set_diplo_flag(a, b, CONTACT | MET);
    }

    /// End a treaty between `a` and `b` (both ways) and be in contact: war.
    pub fn declare_war(&mut self, a: Slot, b: Slot) {
        if a == b || a == 0 || b == 0 {
            return;
        }
        self.clear_diplo_flag(a, b, PEACE);
        self.set_diplo_flag(a, b, CONTACT);
    }

    /// A treaty between `a` and `b` (both ways).
    pub fn make_peace(&mut self, a: Slot, b: Slot) {
        if a == b || a == 0 || b == 0 {
            return;
        }
        self.set_diplo_flag(a, b, CONTACT | PEACE);
    }

    /// The Senate of a Republic or Democracy overrules breaking a treaty (CIV.EXE 0x1a42d:
    /// government >= 4).
    pub fn senate_overrules(&self, slot: Slot) -> bool {
        matches!(self.player(slot).govt, Govt::Republic | Govt::Democracy)
    }

    /// `senate_check_attack` (CIV.EXE 0x1a3b4) would not let `slot` act against `on`: a treaty
    /// in `slot`'s word and a Senate. Without a treaty nothing is asked.
    pub fn senate_blocks_war(&self, slot: Slot, on: Slot) -> bool {
        self.treaty(slot, on) && self.senate_overrules(slot)
    }

    /// "Other civilizations always offer to make peace with you": the owner of the Great Wall
    /// or the United Nations (CIV.EXE o 0x66d1, and 0x151b2 between computer players).
    pub fn offered_peace(&self, slot: Slot) -> bool {
        self.owner_has_wonder(slot, won::GREAT_WALL) || self.owner_has_wonder(slot, won::UNITED_NATIONS)
    }

    /// Power ratio of `a` relative to `b` in percent (100 = equal; `score::power`), for civrs'
    /// own nuclear target choice.
    pub fn power_ratio(&self, a: Slot, b: Slot) -> i32 {
        let pa = self.power(a).max(1);
        let pb = self.power(b).max(1);
        (pa * 100 / pb).min(1000)
    }

    /// Civilisations `slot` is at war with in CIV.EXE's sense (contact, no treaty).
    pub fn enemies_of(&self, slot: Slot) -> Vec<Slot> {
        self.civ_slots().into_iter().filter(|s| *s != slot && self.player(*s).alive && self.at_war(slot, *s)).collect()
    }

    /// Civilisations `slot` has contact with and is alive.
    pub fn contacts_of(&self, slot: Slot) -> Vec<Slot> {
        self.civ_slots().into_iter().filter(|s| *s != slot && self.player(*s).alive && self.player(slot).has_contact(*s)).collect()
    }

    /// ds:0x6e82: military strength, attack plus defence of every unit, the stashed garrison
    /// of the cities included (census 0x15848, 0x15acc).
    pub fn military_strength(&self, slot: Slot) -> i32 {
        let units: i32 = self.all_units().filter(|u| u.owner == slot).map(|u| u.info().att as i32 + u.info().def as i32).sum();
        let stashed: i32 = self
            .all_cities()
            .filter(|c| c.owner == slot)
            .flat_map(|c| c.garrison_units().collect::<Vec<_>>())
            .map(|(k, _)| rules::UNITS[k as usize].att as i32 + rules::UNITS[k as usize].def as i32)
            .sum();
        units + stashed
    }

    /// ds:0xb3f0[0x38 * civ] (0x15789): the civ's Nuclear units (the per-type counter of
    /// type 25; the catalogue's "first word of the table" is ds:0xb3be).
    pub fn nuclear_units(&self, slot: Slot) -> i32 {
        self.player(slot).unit_counter[unit_ids::NUCLEAR as usize] as i32
    }

    /// 0xa5e:0x229b `knows_advance`: never Future Tech or the unused slots, never barbarians.
    pub fn knows_advance(&self, slot: Slot, a: u8) -> bool {
        slot != 0 && a < 71 && self.player(slot).knows(a)
    }

    /// Give an advance from one civilisation to another through diplomacy (0xa5e:0x1d14).
    pub fn transfer_advance(&mut self, from: Slot, to: Slot, a: u8, source: &str) {
        if self.grant_advance(to, a, super::science::AdvanceSource::From(from)) {
            self.push_event(GameEvent::AdvanceAcquired { slot: to, adv: a, source: source.to_string() });
        }
    }

    /// Move gold between treasuries (clamped to what the payer has).
    pub fn transfer_gold(&mut self, from: Slot, to: Slot, amount: i32) -> i32 {
        let amount = amount.clamp(0, self.player(from).gold.max(0));
        self.player_mut(from).gold -= amount;
        self.player_mut(to).gold += amount;
        amount
    }

    /// Establish an embassy of `slot` with `with` (o 0x1d6ce: [human][owner] |= 0x40); the
    /// dossier that follows is all the human sees of it.
    pub fn establish_embassy(&mut self, slot: Slot, with: Slot) {
        if slot == with || slot == 0 || with == 0 {
            return;
        }
        self.raise_flag(slot, with, EMBASSY);
    }

    /// The human has an embassy with `a` or `b`: CIV.EXE then reports their wars and treaties.
    fn human_hears_of(&self, a: Slot, b: Slot) -> bool {
        let h = self.human_slot;
        self.player(h).has_embassy(a) || self.player(h).has_embassy(b)
    }

    // ---- contact ----------------------------------------------------------------------------

    /// The contact part of `unit_look` (CIV.EXE 0x8038, 0x8413..0x8491) for the unit `id`: a
    /// land unit on land meets, square by square in the order of the neighbour table
    /// (ds:0x1882), the civilization of the first unit (0x1a68:0x145e) on each land square
    /// next to it unless that unit is an aircraft (`civs_meet(civ, other, x, y)` with the
    /// looker's square). Two squares of the same civ are two meetings.
    pub fn look_for_contacts(&mut self, id: super::UnitId) {
        let Some(u) = self.unit_opt(id).cloned() else { return };
        if u.owner == 0 || !u.is_land() || self.map.is_ocean(u.x, u.y) {
            return;
        }
        for (nx, ny) in Map::neighbours8(u.x, u.y) {
            if self.map.is_ocean(nx, ny) {
                continue;
            }
            let Some(f) = self.units_at(nx, ny).first().copied() else { continue };
            let (other, air) = (self.unit(f).owner, self.unit(f).is_air());
            if other != u.owner && !air {
                self.civs_meet(u.owner, other, u.x, u.y);
            }
        }
    }

    /// CIV.EXE 0x1508d `civs_meet(a, b, x, y)`: units of `a` and `b` touch. With the human an
    /// audience opens when the two have no contact or the last talk is 16 turns old; then both
    /// words get `CONTACT | MET`. Between computer players, on first contact and every fourth
    /// turn (by (a + b + turn) & 3), unless one has a vendetta against the other: war when
    /// either wants it (`ai_wants_war`), the other lacks the Great Wall and the one wanting war
    /// is not a Republic or Democracy; otherwise they swap advances and sign a treaty. The
    /// human hears of it when he has an embassy with either.
    pub fn civs_meet(&mut self, a: Slot, b: Slot, x: i32, y: i32) {
        if a == 0 || b == 0 || a == b || !self.player(a).alive || !self.player(b).alive {
            return;
        }
        let h = self.human_slot;
        let turn = self.turn as i32;
        for (me, ai) in [(a, b), (b, a)] {
            if me == h && ai != h && (!self.player(me).has_contact(ai) || turn - self.player(ai).last_contact >= 16) {
                if let Some(st) = crate::ai::diplo_ai::meeting(self, ai, (x, y), 0) {
                    self.audience_pending |= 1 << ai;
                    self.push_event(GameEvent::NeedsDecision(super::Decision::Diplomacy(st)));
                }
            }
        }
        let old = self.diplo(a, b);
        self.raise_flag(a, b, CONTACT | MET);
        self.raise_flag(b, a, CONTACT | MET);
        if a == h || b == h {
            return;
        }
        if (a as u32 + b as u32 + self.turn) & 3 != 0 && old & CONTACT != 0 {
            return;
        }
        if self.diplo(a, b) & VENDETTA != 0 || self.diplo(b, a) & VENDETTA != 0 {
            return;
        }
        let war_by = |g: &Game, x: Slot, y: Slot| g.ai_wants_war(x, y) && !g.owner_has_wonder(y, won::GREAT_WALL) && !g.senate_overrules(x);
        if war_by(self, a, b) || war_by(self, b, a) {
            if !self.treaty(a, b) && old & CONTACT != 0 {
                return;
            }
            if self.human_hears_of(a, b) {
                self.push_event(if self.treaty(a, b) { GameEvent::TreatyCancelled { a, b } } else { GameEvent::WarDeclared { a, b } });
            }
            self.clear_diplo_flag(a, b, PEACE);
        } else {
            self.exchange_advances(a, b);
            if self.treaty(a, b) {
                return;
            }
            if self.human_hears_of(a, b) {
                self.push_event(GameEvent::PeaceTreaty { a, b });
            }
            self.set_diplo_flag(a, b, PEACE);
        }
    }

    /// CIV.EXE 0x15473 `ai_wants_war(a, b)`: may computer player `a` attack `b`? Not when one
    /// of them is at war with the human and the other weaker than the human (the enemy of my
    /// enemy), nor when `a` has fewer Nuclear units than `b`; not when `b`'s attack on a
    /// continent with cities of `a` beats half of `a`'s attack plus its defence there. Then war
    /// when four times `a`'s attack over `b`'s half attack plus defence, on the continents where
    /// both have forces, reaches 4 + `a`'s other wars (+1 for a treaty with `b`) - its
    /// aggression; else only when `a` has no city there. (Bit 0 of ds:0xe3c4, which would make
    /// it always true, is set in every game.)
    pub fn ai_wants_war(&self, a: Slot, b: Slot) -> bool {
        let h = self.human_slot;
        let strength = |s: Slot| self.military_strength(s);
        let sh = strength(h);
        if self.diplo(a, h) & 3 == CONTACT && strength(b) < sh && self.diplo(b, h) & ALLIANCE == 0 {
            return false;
        }
        if self.diplo(b, h) & 3 == CONTACT && strength(a) < sh && self.diplo(a, h) & ALLIANCE == 0 {
            return false;
        }
        if self.nuclear_units(a) < self.nuclear_units(b) {
            return false;
        }
        let mut wars = (1..8u8).filter(|j| *j != a && *j != b && self.diplo(a, *j) & 3 == CONTACT).count() as i32;
        if self.treaty(a, b) {
            wars += 1;
        }
        wars -= self.player(a).civ().aggressive as i32;
        let census = self.census();
        let (ai, bi) = (a as usize, b as usize);
        let (mut cities, mut att, mut den) = (0, 0, 1);
        for k in 1..15 {
            if census.cities[ai][k] != 0 && census.attack[ai][k] / 2 + census.defence[ai][k] < census.attack[bi][k] {
                return false;
            }
            if census.attack[ai][k] != 0 && census.defence[bi][k] != 0 {
                att += census.attack[ai][k];
                den += census.attack[bi][k] / 2 + census.defence[bi][k];
                cities += census.cities[ai][k];
            }
        }
        4 * att / den >= wars + 4 || cities == 0
    }

    /// CIV.EXE 0x15645 `exchange_advances(a, b)`: when each knows an advance the other lacks,
    /// each gets the other's highest. True if they swapped.
    pub fn exchange_advances(&mut self, a: Slot, b: Slot) -> bool {
        let (mut for_a, mut for_b) = (None, None);
        for x in 0..72u8 {
            if !self.knows_advance(a, x) && self.knows_advance(b, x) {
                for_a = Some(x);
            }
            if !self.knows_advance(b, x) && self.knows_advance(a, x) {
                for_b = Some(x);
            }
        }
        let (Some(x), Some(y)) = (for_a, for_b) else { return false };
        self.grant_advance(a, x, super::science::AdvanceSource::From(b));
        self.grant_advance(b, y, super::science::AdvanceSource::From(a));
        true
    }

    // ---- per turn ---------------------------------------------------------------------------

    /// The diplomacy of CIV.EXE 0x1495a `civ_turn_politics(civ)`, before the civ's units move:
    /// a treaty the civ means to break (`BREAK`) goes with probability 1/3, leaving the other
    /// side's peace in place (`SNEAK`). Every 16 turns contacts lapse unless the two met since
    /// the last check ("The war between the X and the Y has ended." when the human has an
    /// embassy with either). Every 32 turns (turn & 31 == 4 * civ) a computer player that has
    /// fought a civ the human is at peace with may ask the human for help (HELPME / CRUSADE).
    pub fn civ_politics(&mut self, civ: Slot) {
        if !self.player(civ).alive {
            return;
        }
        let h = self.human_slot;
        for j in 1..8u8 {
            if self.diplo(civ, j) & BREAK != 0 && self.rng.below(3) == 0 {
                self.drop_flag(civ, j, BREAK | PEACE);
                self.raise_flag(civ, j, SNEAK);
            }
            if self.turn & 15 != 0 {
                continue;
            }
            let f = self.diplo(civ, j);
            if f & MET == 0 {
                if f & 3 == CONTACT && self.human_hears_of(civ, j) && civ != h && j != h {
                    self.push_event(GameEvent::WarEnded { a: civ, b: j });
                }
                self.drop_flag(civ, j, CONTACT);
                self.drop_flag(j, civ, CONTACT);
            }
            self.drop_flag(civ, j, MET);
            self.drop_flag(j, civ, MET);
        }
        if civ != 0 && civ != h && (self.turn & 31) == 4 * civ as u32 {
            crate::ai::diplo_ai::ask_for_help(self, civ);
        }
    }

    /// The end of CIV.EXE's power ranking (0x3121, once a turn): when the human leads the
    /// world (`world_gangs_up`) every computer player with more than one city turns against
    /// him: a treaty is to be broken (`BREAK`), else contact (war).
    pub fn gang_up_on_leader(&mut self) {
        if !self.world_gangs_up() {
            return;
        }
        let h = self.human_slot;
        for j in 1..8u8 {
            if j == h || self.cities_of(j).len() <= 1 {
                continue;
            }
            if self.treaty(j, h) {
                self.raise_flag(j, h, BREAK);
            } else {
                self.raise_flag(j, h, CONTACT);
            }
        }
    }

    /// A government change makes the civ forget its vendettas (CIV.EXE 0x15068..0x15085).
    pub fn forget_vendettas(&mut self, slot: Slot) {
        for o in 0..8u8 {
            self.drop_flag(slot, o, VENDETTA);
        }
    }

    /// CIV.EXE o 0x7e0e `make_peace(civ)`: the treaty between the human and `ai`, who will not
    /// ask for an audience for 16 turns; at Chieftain the computer's units whose nearest city
    /// is the human's, two squares away or less, are disbanded.
    pub fn sign_treaty_with_human(&mut self, ai: Slot) {
        let h = self.human_slot;
        self.set_diplo_flag(h, ai, CONTACT | PEACE);
        self.player_mut(ai).last_contact = self.turn as i32 + 16;
        if self.setup.difficulty as u8 != 0 {
            return;
        }
        let doomed: Vec<super::UnitId> = self
            .all_units()
            .filter(|u| u.owner == ai)
            .filter(|u| {
                let near = self.all_cities().min_by_key(|c| (Map::civ_distance(c.pos(), u.pos()), c.id));
                near.map(|c| c.owner == h && Map::civ_distance(c.pos(), u.pos()) <= 2).unwrap_or(false)
            })
            .map(|u| u.id)
            .collect();
        for id in doomed {
            if self.unit_opt(id).is_some() {
                self.remove_unit(id);
            }
        }
    }

    // ---- fights and territory ------------------------------------------------------------

    /// The diplomacy of a fight (CIV.EXE 0x19e37..0x19fdd) between `a` (attacker) and `d`:
    /// both remember the fight; any fight between two civilizations ends the run of peaceful
    /// turns (ds:0xd808); a human defender at peace, or an attacker with a pending sneak
    /// attack, hears "Sneak attack by <adj> forces!"; a treaty between them ends ("<D> cancel
    /// peace treaty with <A>." when the human is involved) and a human attacker earns the
    /// defender's vendetta.
    pub fn fight_diplomacy(&mut self, a: Slot, d: Slot) {
        if a == d {
            return;
        }
        let h = self.human_slot;
        self.raise_flag(a, d, FOUGHT);
        self.raise_flag(d, a, FOUGHT);
        if a != 0 && d != 0 {
            for p in self.players.iter_mut() {
                p.peace_turns = 0;
            }
        }
        if (d == h && self.treaty(d, a)) || self.diplo(a, d) & SNEAK != 0 {
            self.push_event(GameEvent::SneakAttack { by: a, on: d });
            self.drop_flag(a, d, SNEAK);
        }
        if self.treaty(d, a) || self.treaty(a, d) {
            if a == h || d == h {
                self.push_event(GameEvent::TreatyBroken { by: a, victim: d });
            }
            if a == h && self.treaty(d, a) {
                self.raise_flag(d, a, VENDETTA);
            }
            self.clear_diplo_flag(a, d, PEACE);
        }
        if a == h {
            self.drop_flag(d, a, SNEAK);
        }
    }

    /// The civilization whose city works the tile in CIV.EXE's owner layer (map page 2 at x+80,
    /// written by `work_tile` 0x132e3 for every square a city evaluates): civrs takes the city
    /// of the nearest civ whose 21 squares include it (an approximation of "the last city that
    /// looked at it").
    pub fn territory_owner(&self, x: i32, y: i32) -> Option<Slot> {
        self.all_cities()
            .filter(|c| rules::CITY_RADIUS.iter().any(|(dx, dy)| Map::wrap_x(c.x + *dx as i32) == x && c.y + *dy as i32 == y))
            .min_by_key(|c| (Map::civ_distance(c.pos(), (x, y)), c.id))
            .map(|c| c.owner)
    }

    /// The territory rule of the move routine (CIV.EXE 0x64bb..0x65b5): a military land unit
    /// (type below Diplomat) of `slot` moving from land onto a developed land square (city,
    /// irrigation, mine or road) without units of its own, in the territory of another
    /// civilization, where one of the two is the human. Returns that civilization.
    pub fn territory_crossed(&self, unit: &super::Unit, to: (i32, i32)) -> Option<Slot> {
        if !unit.is_land() || unit.kind >= unit_ids::DIPLOMAT || self.map.is_ocean(unit.x, unit.y) || self.map.is_ocean(to.0, to.1) {
            return None;
        }
        use super::map::tile_flags as tf;
        let t = self.map.tile(to.0, to.1);
        if t.city.is_none() && !t.has(tf::IRRIGATION) && !t.has(tf::MINE) && !t.has(tf::ROAD) && !t.has(tf::RAILROAD) {
            return None;
        }
        if self.all_units().any(|o| o.pos() == to && o.owner == unit.owner) {
            return None;
        }
        let o = self.territory_owner(to.0, to.1)?;
        let h = self.human_slot;
        (o != 0 && o != unit.owner && (unit.owner == h || o == h)).then_some(o)
    }
}

#[cfg(test)]
mod tests {
    use super::flags::*;
    use crate::game::{Game, GameEvent, GameSetup, Relation};
    use crate::rules::{unit_ids, Terrain};

    fn game() -> Game {
        let mut g = Game::new(GameSetup { seed: 21, num_civs: 4, ..Default::default() }, None);
        for s in 1..5 {
            g.players[s].in_game = true;
            g.players[s].alive = true;
        }
        for y in 5..30 {
            for x in 5..40 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Grassland;
                t.flags = 0;
                t.continent = 1;
            }
        }
        let ids: Vec<_> = g.all_units().map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        assert_eq!(g.human_slot, 1);
        g
    }

    #[test]
    fn flag_helpers_follow_0x15386_and_0x153f7() {
        let mut g = game();
        g.raise_flag(2, 3, FOUGHT | ALLIANCE);
        g.set_diplo_flag(2, 3, PEACE);
        assert_eq!(g.diplo(2, 3) & (PEACE | FOUGHT), PEACE, "a treaty forgets the fighting");
        assert_eq!(g.diplo(3, 2) & PEACE, PEACE);
        g.clear_diplo_flag(3, 2, PEACE);
        assert_eq!(g.diplo(2, 3) & (PEACE | ALLIANCE), 0, "the end of a treaty ends the alliance");
        assert_eq!(g.relation(2, 3), Relation::NoContact);
        g.make_contact(2, 3);
        assert_eq!(g.relation(2, 3), Relation::War);
        assert!(g.at_war(2, 3) && g.hostile(2, 3));
        // no contact: not at war in CIV.EXE's sense, but nothing stops a fight either
        assert!(!g.at_war(2, 4) && g.hostile(2, 4));
    }

    #[test]
    fn computer_players_meet_and_decide_war_or_peace() {
        let mut g = game();
        g.add_city(2, "A".into(), 10, 10);
        g.add_city(3, "B".into(), 30, 10);
        let a = g.add_unit(2, unit_ids::MILITIA, 20, 10);
        g.add_unit(3, unit_ids::MILITIA, 21, 10);
        g.look_for_contacts(a);
        assert_ne!(g.diplo(2, 3) & (CONTACT | MET), 0);
        assert_ne!(g.diplo(3, 2) & (CONTACT | MET), 0);
        // equally weak neighbours: no war, a treaty (no embassy, so no news)
        assert!(g.treaty(2, 3) || g.at_war(2, 3));
        assert!(!g.events.iter().any(|e| matches!(e, GameEvent::PeaceTreaty { .. } | GameEvent::WarDeclared { .. })));
        // a vendetta freezes the relation
        g.set_relation(2, 3, Relation::War);
        g.raise_flag(3, 2, VENDETTA);
        g.turn = 7; // (2 + 3 + turn) & 3 == 0: they look at each other again
        g.civs_meet(2, 3, 21, 10);
        assert_eq!(g.relation(2, 3), Relation::War);
    }

    #[test]
    fn contact_goes_by_the_first_unit_of_each_square() {
        // 0x8413..0x8491: an aircraft heading its stack makes no contact; each square with a
        // foreign land unit is a meeting of its own, here two exchanges of advances
        let mut g = game();
        g.add_city(2, "A".into(), 10, 10);
        g.add_city(3, "B".into(), 30, 10);
        let a = g.add_unit(2, unit_ids::MILITIA, 20, 10);
        g.add_unit(3, unit_ids::FIGHTER, 21, 10);
        g.add_unit(3, unit_ids::MILITIA, 21, 10);
        g.look_for_contacts(a);
        assert_eq!(g.diplo(2, 3) & CONTACT, 0);
        g.set_relation(2, 3, Relation::Peace);
        g.raise_flag(2, 3, MET);
        g.raise_flag(3, 2, MET);
        let ids: Vec<_> = g.units_at(21, 10);
        for id in ids {
            g.remove_unit(id);
        }
        g.add_unit(3, unit_ids::MILITIA, 21, 10);
        g.add_unit(3, unit_ids::MILITIA, 21, 11);
        for s in [2u8, 3] {
            g.player_mut(s).advances = 0;
        }
        for x in [1u8, 2] {
            g.player_mut(2).learn(x);
        }
        for x in [4u8, 5] {
            g.player_mut(3).learn(x);
        }
        g.turn = 7; // (2 + 3 + turn) & 3 == 0
        // Senates keep the peace whatever the leaders want
        g.player_mut(2).govt = crate::rules::Govt::Republic;
        g.player_mut(3).govt = crate::rules::Govt::Republic;
        g.look_for_contacts(a);
        assert!(g.player(2).knows(4) && g.player(2).knows(5));
        assert!(g.player(3).knows(1) && g.player(3).knows(2));
    }

    #[test]
    fn contacts_lapse_every_16_turns_unless_met() {
        let mut g = game();
        g.set_relation(2, 3, Relation::War);
        g.raise_flag(1, 2, EMBASSY);
        g.turn = 32;
        g.civ_politics(2);
        assert_eq!(g.diplo(2, 3) & CONTACT, 0);
        assert!(g.events.iter().any(|e| matches!(e, GameEvent::WarEnded { a: 2, b: 3 })));
        // a treaty survives the lapse of contact
        g.set_relation(2, 4, Relation::Peace);
        g.civ_politics(2);
        assert!(g.treaty(2, 4) && g.diplo(2, 4) & CONTACT == 0);
        // met since the last check: only the MET bit goes
        g.raise_flag(2, 3, CONTACT | MET);
        g.raise_flag(3, 2, CONTACT | MET);
        g.civ_politics(2);
        assert_eq!(g.diplo(2, 3) & (CONTACT | MET), CONTACT);
    }

    #[test]
    fn a_treaty_to_break_becomes_a_one_sided_sneak_attack() {
        let mut g = game();
        g.set_relation(2, 1, Relation::Peace);
        g.raise_flag(2, 1, BREAK);
        g.turn = 3;
        for _ in 0..60 {
            g.civ_politics(2);
        }
        assert_eq!(g.diplo(2, 1) & (PEACE | BREAK | SNEAK), SNEAK);
        assert!(g.treaty(1, 2), "the human's side still has the treaty");
        assert!(g.hostile(2, 1) && !g.hostile(1, 2));
        // the attack: sneak attack message, the treaty ends both ways, no vendetta
        g.fight_diplomacy(2, 1);
        assert!(g.events.iter().any(|e| matches!(e, GameEvent::SneakAttack { by: 2, on: 1 })));
        assert!(g.events.iter().any(|e| matches!(e, GameEvent::TreatyBroken { by: 2, victim: 1 })));
        assert!(!g.treaty(1, 2) && !g.treaty(2, 1));
        assert_eq!(g.diplo(1, 2) & VENDETTA, 0);
        assert_ne!(g.diplo(1, 2) & FOUGHT, 0);
    }

    #[test]
    fn a_human_breaking_a_treaty_earns_a_vendetta() {
        let mut g = game();
        g.set_relation(1, 2, Relation::Peace);
        g.player_mut(1).peace_turns = 9;
        g.fight_diplomacy(1, 2);
        assert!(!g.treaty(2, 1));
        assert_ne!(g.diplo(2, 1) & VENDETTA, 0);
        assert_eq!(g.player(1).peace_turns, 0);
    }

    #[test]
    fn the_leading_human_is_ganged_up_on() {
        let mut g = game();
        g.set_relation(1, 2, Relation::Peace);
        g.gang_up_on_leader();
        assert_eq!(g.diplo(2, 1) & BREAK, 0, "not the leader: nothing happens");
        // the strongest, more than four cities, after turn 200
        g.turn = 201;
        for i in 0..5 {
            let c = g.add_city(1, format!("H{i}"), 6 + 3 * i, 6);
            g.city_mut(c).size = 8;
        }
        g.add_city(2, "A1".into(), 30, 20);
        g.add_city(2, "A2".into(), 33, 20);
        g.add_city(3, "B1".into(), 30, 26);
        g.add_city(3, "B2".into(), 33, 26);
        g.add_city(4, "C1".into(), 36, 26);
        assert!(g.world_gangs_up());
        g.gang_up_on_leader();
        assert_ne!(g.diplo(2, 1) & BREAK, 0, "a treaty partner means to break it");
        assert!(g.treaty(2, 1), "for now");
        assert!(g.at_war(3, 1), "the others are at war at once");
        assert_eq!(g.diplo(4, 1), 0, "a civilization with one city stays out");
    }

    #[test]
    fn computer_wars_and_treaties_are_announced_with_an_embassy() {
        let mut g = game();
        g.raise_flag(1, 2, EMBASSY);
        // b has a strong army next to a's only city: a will not attack, b will
        g.add_city(2, "A".into(), 10, 10);
        g.add_city(3, "B".into(), 30, 10);
        for _ in 0..6 {
            g.add_unit(3, unit_ids::KNIGHTS, 11, 10);
        }
        g.add_unit(2, unit_ids::MILITIA, 10, 10);
        assert!(!g.ai_wants_war(2, 3), "b's attack outweighs a's defence where a has a city");
        assert!(g.ai_wants_war(3, 2), "b has no city at stake");
        g.civs_meet(2, 3, 11, 10);
        assert!(!g.treaty(2, 3));
        assert!(g.events.iter().any(|e| matches!(e, GameEvent::WarDeclared { a: 2, b: 3 })));
        // a Great Wall makes b give up the war, and they sign a treaty
        let c = g.cities_of(2)[0];
        g.wonders_built[crate::rules::wonder_ids::GREAT_WALL as usize] = Some(c);
        g.events.clear();
        g.turn = 7;
        g.civs_meet(2, 3, 11, 10);
        assert!(g.treaty(2, 3) && g.treaty(3, 2));
        assert!(g.events.iter().any(|e| matches!(e, GameEvent::PeaceTreaty { a: 2, b: 3 })));
        // a Republic does not start a war either
        g.wonders_built[crate::rules::wonder_ids::GREAT_WALL as usize] = None;
        g.player_mut(3).govt = crate::rules::Govt::Republic;
        g.turn = 11;
        g.civs_meet(2, 3, 11, 10);
        assert!(g.treaty(2, 3));
    }

    #[test]
    fn the_human_must_break_a_treaty_to_attack_or_enter_a_partners_land() {
        use crate::game::{Dir8, MoveOutcome};
        let mut g = game();
        g.set_relation(1, 2, Relation::Peace);
        let c = g.add_city(2, "A".into(), 20, 10);
        g.city_mut(c).size = 3;
        g.add_unit(2, unit_ids::MILITIA, 20, 10);
        // a road next to the city: the partner's land
        g.map.get_mut(19, 11).unwrap().set(crate::game::tile_flags::ROAD, true);
        let me = g.add_unit(1, unit_ids::LEGION, 18, 11);
        assert_eq!(g.territory_owner(19, 11), Some(2));
        assert_eq!(g.try_move(me, Dir8::E), Ok(MoveOutcome::BreakTreaty(2)));
        assert_eq!(g.unit(me).pos(), (18, 11), "the unit waits for the answer");
        g.treaty_break_ok = Some(2);
        assert_eq!(g.try_move(me, Dir8::E), Ok(MoveOutcome::Moved));
        assert!(!g.treaty(1, 2) && !g.treaty(2, 1), "the treaty is over");
        assert_eq!(g.diplo(2, 1) & VENDETTA, 0, "no vendetta for crossing a border");
        // an attack on a treaty partner: vendetta
        g.set_relation(1, 2, Relation::Peace);
        g.unit_mut(me).moves_left = 3;
        assert_eq!(g.try_move(me, Dir8::NE), Ok(MoveOutcome::BreakTreaty(2)));
        g.treaty_break_ok = Some(2);
        assert!(matches!(g.try_move(me, Dir8::NE), Ok(MoveOutcome::Attacked { .. })));
        assert_ne!(g.diplo(2, 1) & VENDETTA, 0);
        assert!(!g.treaty(2, 1));
        let _ = c;
    }

    #[test]
    fn a_computer_unit_stops_at_a_treaty_partners_border() {
        use crate::game::{Dir8, MoveOutcome};
        let mut g = game();
        g.set_relation(1, 2, Relation::Peace);
        g.add_city(1, "Mine".into(), 20, 10);
        g.map.get_mut(19, 11).unwrap().set(crate::game::tile_flags::ROAD, true);
        let ai = g.add_unit(2, unit_ids::LEGION, 18, 11);
        assert_eq!(g.try_move(ai, Dir8::E), Ok(MoveOutcome::Blocked));
        assert_eq!(g.unit(ai).moves_left, 0, "its turn is over (0x5a0e)");
        // an undeveloped square is no border
        g.unit_mut(ai).moves_left = 3;
        assert_eq!(g.try_move(ai, Dir8::NE), Ok(MoveOutcome::Moved));
    }

    #[test]
    fn peace_at_chieftain_sends_units_near_human_cities_home() {
        let mut g = game();
        g.setup.difficulty = crate::rules::Difficulty::Chieftain;
        g.add_city(1, "Mine".into(), 20, 10);
        g.add_city(2, "Theirs".into(), 30, 10);
        let near = g.add_unit(2, unit_ids::LEGION, 21, 11);
        let far = g.add_unit(2, unit_ids::LEGION, 25, 10);
        g.sign_treaty_with_human(2);
        assert!(g.treaty(1, 2) && g.treaty(2, 1));
        assert!(g.unit_opt(near).is_none());
        assert!(g.unit_opt(far).is_some());
        assert_eq!(g.player(2).last_contact, g.turn as i32 + 16);
    }

    #[test]
    fn advance_values_follow_the_leader_trait() {
        let g = game();
        // Babylonians are civilized (+1): Monarchy (6, 1) -> 7
        let s = (1..8u8).find(|s| g.player(*s).civ().civilized == 1).unwrap();
        assert_eq!(crate::ai::tech_ai::value(&g, s, crate::rules::advance_ids::MONARCHY), 7);
    }
}
