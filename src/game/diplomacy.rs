//! Diplomacy state shared by the engine, the AI and the diplomacy screen: relations, contact
//! detection, war/peace changes and the audience description (`DiplomacyState`) whose fields
//! substitute `$RPLC1 $RPLC2 $BUCKS $US $THEM` in the KING.TXT section named by `Proposal`.
//!
//! The AI's judgement (what to propose, how to react) lives in `crate::ai::diplo_ai`.

use super::map::{Map, Slot};
use super::{Game, GameEvent, Relation};
use crate::rules::{wonder_ids as won, Govt};
use serde::{Deserialize, Serialize};

/// What an audience is about; names mirror KING.TXT sections.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Proposal {
    /// AI offers to exchange knowledge (`rplc1` = advance they offer; `advance`).
    Exchange,
    /// AI demands we cancel our treaty with `other` (`rplc1` = "demand"/"insist").
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
    /// Human-initiated audience menu: welcome peace / military proposal / demand tribute.
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
    /// We are already at war with them (no choices).
    AtWar,
    /// No contact yet (no choices).
    NoContact,
    /// AI offers to fight our enemy `other` for `bucks` (Never mind / Yes, pay).
    Mercenary,
    /// Our treasury cannot cover a promise (no choices).
    Unfortunate,
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
            | Proposal::Mercenary => 2,
            _ => 0,
        }
    }

    /// The audience ends with a declaration of war by the AI.
    pub fn is_war_declaration(self) -> bool {
        matches!(self, Proposal::Provoke | Proposal::Reject | Proposal::Rid)
    }
}

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
    /// Whether this audience was requested by the human.
    pub human_initiated: bool,
    /// Steps already taken (for multi-step audiences).
    pub stage: u8,
}

impl DiplomacyState {
    pub fn new(ai: Slot, human: Slot, proposal: Proposal) -> DiplomacyState {
        DiplomacyState { ai, human, proposal, rplc1: String::new(), rplc2: String::new(), bucks: 0, advance: None, other: None, human_initiated: false, stage: 0 }
    }

    /// `$US` substitution: the human civilisation's plural name.
    pub fn us<'a>(&self, g: &'a Game) -> &'a str {
        g.civ_name(self.human)
    }

    /// `$THEM` substitution: the third party when there is one, else the AI civilisation.
    pub fn them<'a>(&self, g: &'a Game) -> &'a str {
        g.civ_name(self.other.unwrap_or(self.ai))
    }

    /// Number of menu choices (0 = message only).
    pub fn choices(&self) -> usize {
        self.proposal.choices()
    }
}

impl Game {
    pub fn relation(&self, a: Slot, b: Slot) -> Relation {
        self.player(a).relation(b)
    }

    /// Set the relation symmetrically.
    pub fn set_relation(&mut self, a: Slot, b: Slot, r: Relation) {
        self.players[a as usize].relations[b as usize] = r;
        self.players[b as usize].relations[a as usize] = r;
    }

    /// First contact between two civilisations: both are at peace and may hold audiences.
    pub fn make_contact(&mut self, a: Slot, b: Slot) {
        if a == b || a == 0 || b == 0 || self.relation(a, b) != Relation::NoContact {
            return;
        }
        self.set_relation(a, b, Relation::Peace);
        self.push_event(GameEvent::ContactMade { a, b });
    }

    /// `a` declares war on `b` (no-op if already at war or no contact).
    pub fn declare_war(&mut self, a: Slot, b: Slot) {
        if a == b || a == 0 || b == 0 || self.relation(a, b) == Relation::War {
            return;
        }
        self.set_relation(a, b, Relation::War);
        self.players[a as usize].peace_turns = 0;
        self.players[b as usize].peace_turns = 0;
        let att = &mut self.players[b as usize].attitude[a as usize];
        *att = (*att as i32 - 30).clamp(-100, 100) as i8;
        self.push_event(GameEvent::WarDeclared { a, b });
    }

    /// Peace treaty between `a` and `b`.
    pub fn make_peace(&mut self, a: Slot, b: Slot) {
        if a == b || a == 0 || b == 0 || self.relation(a, b) == Relation::Peace {
            return;
        }
        self.set_relation(a, b, Relation::Peace);
        for (x, y) in [(a, b), (b, a)] {
            let att = &mut self.players[x as usize].attitude[y as usize];
            *att = (*att as i32 + 20).clamp(-100, 100) as i8;
        }
        self.push_event(GameEvent::PeaceTreaty { a, b });
    }

    /// Senate rule: under Republic/Democracy a civilisation at peace cannot declare war.
    pub fn senate_blocks_war(&self, slot: Slot, on: Slot) -> bool {
        matches!(self.player(slot).govt, Govt::Republic | Govt::Democracy) && self.relation(slot, on) == Relation::Peace
    }

    /// "Other civilizations always offer to make peace with you": the owner of the Great Wall
    /// or the United Nations (CIV.EXE o 0x66d1, and 0x151b2 between computer players).
    pub fn offered_peace(&self, slot: Slot) -> bool {
        self.owner_has_wonder(slot, won::GREAT_WALL) || self.owner_has_wonder(slot, won::UNITED_NATIONS)
    }

    /// Power ratio of `a` relative to `b` in percent (100 = equal); uses `score::power`.
    pub fn power_ratio(&self, a: Slot, b: Slot) -> i32 {
        let pa = self.power(a).max(1);
        let pb = self.power(b).max(1);
        (pa * 100 / pb).min(1000)
    }

    /// Civilisations `slot` is at war with (excluding barbarians).
    pub fn enemies_of(&self, slot: Slot) -> Vec<Slot> {
        self.civ_slots().into_iter().filter(|s| *s != slot && self.player(*s).alive && self.relation(slot, *s) == Relation::War).collect()
    }

    /// Civilisations `slot` has contact with and is alive.
    pub fn contacts_of(&self, slot: Slot) -> Vec<Slot> {
        self.civ_slots().into_iter().filter(|s| *s != slot && self.player(*s).alive && self.relation(slot, *s) != Relation::NoContact).collect()
    }

    /// Detect first contacts: any pair of living civilisations with units or cities within 2
    /// tiles of each other. Returns the newly contacted pairs (a < b).
    pub fn update_contacts(&mut self) -> Vec<(Slot, Slot)> {
        let n = crate::rules::MAP_W * crate::rules::MAP_H;
        let mut presence = vec![0u8; n];
        for u in self.all_units() {
            if u.owner != 0 && Map::in_bounds(u.x, u.y) {
                presence[Map::idx(u.x, u.y)] |= 1 << u.owner;
            }
        }
        for c in self.all_cities() {
            if c.owner != 0 {
                presence[Map::idx(c.x, c.y)] |= 1 << c.owner;
            }
        }
        let mut meets = [[false; 8]; 8];
        for (i, bits) in presence.iter().enumerate() {
            if *bits == 0 {
                continue;
            }
            let (x, y) = ((i % crate::rules::MAP_W) as i32, (i / crate::rules::MAP_W) as i32);
            for yy in (y - 2)..=(y + 2) {
                for xx in (x - 2)..=(x + 2) {
                    if !Map::in_bounds(xx, yy) {
                        continue;
                    }
                    let other = presence[Map::idx(xx, yy)];
                    if other == 0 {
                        continue;
                    }
                    for a in 1..8u8 {
                        if bits & (1 << a) == 0 {
                            continue;
                        }
                        for b in 1..8u8 {
                            if a != b && other & (1 << b) != 0 {
                                meets[a as usize][b as usize] = true;
                            }
                        }
                    }
                }
            }
        }
        let mut new_pairs = Vec::new();
        for a in 1..8u8 {
            for b in (a + 1)..8u8 {
                if !(meets[a as usize][b as usize] || meets[b as usize][a as usize]) {
                    continue;
                }
                let pa = self.player(a);
                let pb = self.player(b);
                if !(pa.in_game && pa.alive && pb.in_game && pb.alive) {
                    continue;
                }
                if self.relation(a, b) == Relation::NoContact {
                    self.make_contact(a, b);
                    new_pairs.push((a, b));
                }
            }
        }
        new_pairs
    }

    /// An advance `from` knows and `to` does not (random pick).
    pub fn advance_to_share(&mut self, from: Slot, to: Slot) -> Option<u8> {
        let known: Vec<u8> = self.player(from).known_advances().into_iter().filter(|a| !self.player(to).knows(*a)).collect();
        self.rng.pick(&known).copied()
    }

    /// Give an advance from one civilisation to another through diplomacy.
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

    /// Establish an embassy of `slot` with `with`.
    pub fn establish_embassy(&mut self, slot: Slot, with: Slot) {
        if slot == with || slot == 0 || with == 0 {
            return;
        }
        if !self.player(slot).has_embassy(with) {
            self.player_mut(slot).embassies |= 1 << with;
            self.push_event(GameEvent::EmbassyEstablished { slot, with });
        }
    }
}
