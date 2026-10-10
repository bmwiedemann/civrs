//! Research: costs, discovery, free advances (Great Library, Darwin's Voyage) and the
//! bookkeeping that follows learning an advance (wonder obsolescence).

use super::events::GameEvent;
use super::map::Slot;
use super::Game;
use crate::rules::{self, wonder_ids as won, AdvanceId};

/// Beakers for the `n+1`-th advance at difficulty `d` [approx]: `(n + 1) * (8 + 2 * d)`.
pub fn research_cost_for(n: i32, difficulty: i32) -> i32 {
    (n + 1) * (8 + 2 * difficulty)
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

    /// Beakers needed for the next advance (AI pays `ai_cost_pct`).
    pub fn research_cost(&self, slot: Slot) -> i32 {
        let p = self.player(slot);
        let n = p.advance_count() as i32 + p.future_techs as i32;
        let d = self.setup.difficulty as i32;
        let mut cost = research_cost_for(n, d);
        if !p.human {
            cost = cost * rules::DIFFICULTY[self.setup.difficulty as usize].ai_cost_pct as i32 / 100;
        }
        cost.max(1)
    }

    pub fn set_research(&mut self, slot: Slot, a: AdvanceId) {
        self.player_mut(slot).researching = Some(a);
    }

    /// Give an advance to a player (no research bookkeeping); returns false if already known.
    /// Also marks wonders of the player obsolete when their obsoleting advance is learned.
    pub fn grant_advance(&mut self, slot: Slot, a: AdvanceId) -> bool {
        if a == rules::advance_ids::FUTURE_TECH {
            self.player_mut(slot).future_techs += 1;
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
        self.player_mut(slot).learn(a);
        if self.player(slot).researching == Some(a) {
            self.player_mut(slot).researching = None;
        }
        // wonders made obsolete by this advance
        for w in 1..rules::WONDERS.len() as u8 {
            if rules::WONDERS[w as usize].obsolete_by != Some(a) {
                continue;
            }
            let owned = self.wonders_built[w as usize].and_then(|c| self.city_opt(c)).map(|c| c.owner == slot).unwrap_or(false);
            if owned {
                self.push_event(GameEvent::WonderObsolete { wonder: w, owner: slot });
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
        self.grant_advance(slot, a);
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
            if self.grant_advance(slot, a) {
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
        let c0 = g.research_cost(slot);
        assert!(c0 > 0);
        assert!(g.grant_advance(slot, rules::advance_ids::ALPHABET));
        assert!(!g.grant_advance(slot, rules::advance_ids::ALPHABET));
        assert!(g.research_cost(slot) > c0);
        assert!(g.researchable(slot).contains(&rules::advance_ids::WRITING));
        // future tech after everything
        for a in 0..67u8 {
            g.grant_advance(slot, a);
        }
        assert_eq!(g.researchable(slot), vec![rules::advance_ids::FUTURE_TECH]);
        g.grant_advance(slot, rules::advance_ids::FUTURE_TECH);
        assert_eq!(g.player(slot).future_techs, 1);
    }
}
