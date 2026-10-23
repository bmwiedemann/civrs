//! Research: costs, discovery, free advances (Great Library, Darwin's Voyage) and the
//! bookkeeping that follows learning an advance (wonder obsolescence).

use super::events::GameEvent;
use super::map::Slot;
use super::Game;
use crate::rules::{self, wonder_ids as won, AdvanceId};


/// Where a newly gained advance came from, as the Civilopedia reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvanceSource {
    /// Researched, from a hut or Darwin's Voyage: "(Discovered)".
    Discovered,
    /// "(from Great Library)".
    GreatLibrary,
    /// Traded, stolen or captured: "(Taken from CIV)".
    From(Slot),
}

impl AdvanceSource {
    /// The byte CIV.EXE keeps per advance (0, -2 or the slot).
    pub fn code(self) -> i8 {
        match self {
            AdvanceSource::Discovered => 0,
            AdvanceSource::GreatLibrary => -2,
            AdvanceSource::From(s) => s as i8,
        }
    }
}

impl Game {
    /// Advances the slot could research now.
    pub fn researchable(&self, slot: Slot) -> Vec<AdvanceId> {
        let p = self.player(slot);
        let mut v: Vec<AdvanceId> = (0..67u8)
            .filter(|a| !p.knows(*a) && rules::ADVANCES[*a as usize].prereq.iter().all(|pr| pr.map(|x| p.knows(x)).unwrap_or(true)))
            .collect();
        if v.is_empty() && p.advance_count() >= 67 {
            v.push(rules::advance_ids::FUTURE_TECH);
        }
        v
    }

    /// Catch-up bonus to the beakers per advance (CIV.EXE 0x27e0, ds:0xd2de): from 1 AD, the
    /// leading civ's research count less turn/9, clamped to 0..6.
    pub fn research_catch_up(&self) -> i32 {
        if self.year() < 0 {
            return 0;
        }
        let lead = self.players.iter().skip(1).map(|p| p.research_count as i32).max().unwrap_or(0);
        (lead - self.turn as i32 / 9).clamp(0, 6)
    }

    /// Beakers per research step (CIV.EXE 0x12cf7): `2 * difficulty + 6` for the human,
    /// `14 - difficulty` for the AI, plus the catch-up bonus. With `minimum` the human's early
    /// advances cost at least `11 - research count` per step (not applied by the sidebar bulb).
    pub fn research_multiplier(&self, slot: Slot, minimum: bool) -> i32 {
        let p = self.player(slot);
        let d = self.setup.difficulty as i32;
        let m = if p.human { 2 * d + 6 } else { 14 - d } + self.research_catch_up();
        if p.human && minimum {
            m.max(11 - p.research_count as i32)
        } else {
            m
        }
    }

    /// Beakers shown as the cost of the next advance: research count * multiplier, doubled
    /// from 1 AD.
    pub fn research_cost(&self, slot: Slot) -> i32 {
        let k = if self.year() < 0 { 1 } else { 2 };
        (self.player(slot).research_count as i32 * self.research_multiplier(slot, true) * k).max(1)
    }

    /// The advance is found once the beakers (halved from 1 AD) exceed count * multiplier.
    pub fn research_complete(&self, slot: Slot) -> bool {
        let k = if self.year() < 0 { 1 } else { 2 };
        let p = self.player(slot);
        p.beakers / k > p.research_count as i32 * self.research_multiplier(slot, true)
    }

    pub fn set_research(&mut self, slot: Slot, a: AdvanceId) {
        self.player_mut(slot).researching = Some(a);
    }

    /// Give an advance to a player (no research bookkeeping); returns false if already known.
    /// Also marks wonders of the player obsolete when their obsoleting advance is learned.
    pub fn grant_advance(&mut self, slot: Slot, a: AdvanceId, source: AdvanceSource) -> bool {
        if a == rules::advance_ids::FUTURE_TECH {
            self.player_mut(slot).future_techs += 1;
            self.player_mut(slot).research_count += 1;
            if self.player(slot).researching == Some(a) {
                self.player_mut(slot).researching = None;
            }
            return true;
        }
        if a as usize >= rules::ADVANCES.len() || !rules::is_real(a) {
            return false;
        }
        if self.player(slot).knows(a) {
            return false;
        }
        // the source is kept for the Civilopedia (CIV.EXE 0xc35f); advances given at the start
        // do not raise the research count (0xc364)
        if let Some(s) = self.player_mut(slot).advance_source.get_mut(a as usize) {
            *s = source.code();
        }
        if self.turn > 0 {
            self.player_mut(slot).research_count += 1;
        }
        let new_to_world = !(1..8).any(|s| self.player(s).knows(a));
        self.player_mut(slot).learn(a);
        if self.player(slot).researching == Some(a) {
            self.player_mut(slot).researching = None;
        }
        // wonders made obsolete by the first civilisation to learn this advance (CIV.EXE 0xc774)
        for w in 1..rules::WONDERS.len() as u8 {
            if !new_to_world || rules::WONDERS[w as usize].obsolete_by != Some(a) {
                continue;
            }
            if let Some(owner) = self.wonders_built[w as usize].and_then(|c| self.city_opt(c)).map(|c| c.owner) {
                self.push_event(GameEvent::WonderObsolete { wonder: w, owner });
            }
        }
        true
    }

    /// Pick what a player researches next: the human is asked (a decision is queued), the AI
    /// chooses immediately. Returns true when a decision was queued.
    pub fn pick_research(&mut self, slot: Slot) -> bool {
        if self.player(slot).researching.is_some() {
            return false;
        }
        let opts = self.researchable(slot);
        if opts.is_empty() {
            return false;
        }
        if self.player(slot).human {
            self.push_event(GameEvent::NeedsDecision(super::Decision::ChooseResearch { slot, options: opts }));
            true
        } else {
            let a = crate::ai::tech_ai::choose(self, slot).or_else(|| opts.first().copied());
            self.player_mut(slot).researching = a;
            false
        }
    }

    /// Complete the current research (or the first researchable advance) for free.
    fn free_advance(&mut self, slot: Slot) -> Option<AdvanceId> {
        let a = match self.player(slot).researching {
            Some(a) => a,
            None => {
                let opts = self.researchable(slot);
                if self.player(slot).human {
                    *opts.first()?
                } else {
                    crate::ai::tech_ai::choose(self, slot).or_else(|| opts.first().copied())?
                }
            }
        };
        self.grant_advance(slot, a, AdvanceSource::Discovered);
        self.player_mut(slot).researching = None;
        Some(a)
    }

    /// Darwin's Voyage: two immediate advances for the owner.
    pub fn darwins_voyage(&mut self, slot: Slot) {
        for _ in 0..2 {
            if let Some(a) = self.free_advance(slot) {
                self.push_event(GameEvent::AdvanceDiscovered { slot, adv: a });
            }
        }
    }

    /// Great Library: its owner learns every advance known by at least two other civilisations.
    pub fn great_library_check(&mut self, slot: Slot) {
        if !self.owner_has_wonder(slot, won::GREAT_LIBRARY) {
            return;
        }
        let others: Vec<Slot> = self.alive_slots().into_iter().filter(|s| *s != slot).collect();
        let mut gained = Vec::new();
        for a in 0..67u8 {
            if self.player(slot).knows(a) {
                continue;
            }
            let n = others.iter().filter(|s| self.player(**s).knows(a)).count();
            if n >= 2 {
                gained.push(a);
            }
        }
        for a in gained {
            if self.grant_advance(slot, a, AdvanceSource::GreatLibrary) {
                self.push_event(GameEvent::AdvanceAcquired { slot, adv: a, source: "Great Library".into() });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;

    #[test]
    fn research_cost_positive_and_growing() {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        g.turn = 1;
        let c0 = g.research_cost(slot);
        assert!(c0 > 0);
        assert!(g.grant_advance(slot, rules::advance_ids::ALPHABET, AdvanceSource::Discovered));
        assert!(!g.grant_advance(slot, rules::advance_ids::ALPHABET, AdvanceSource::Discovered));
        assert!(g.research_cost(slot) > c0);
        assert!(g.researchable(slot).contains(&rules::advance_ids::WRITING));
        // future tech after everything
        for a in 0..67u8 {
            g.grant_advance(slot, a, AdvanceSource::Discovered);
        }
        assert_eq!(g.researchable(slot), vec![rules::advance_ids::FUTURE_TECH]);
        g.grant_advance(slot, rules::advance_ids::FUTURE_TECH, AdvanceSource::Discovered);
        assert_eq!(g.player(slot).future_techs, 1);
    }
}
