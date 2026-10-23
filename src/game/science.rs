//! Research: costs, discovery, free advances (Great Library, Darwin's Voyage) and the
//! bookkeeping that follows learning an advance (wonder obsolescence, Barracks).
//!
//! CIV.EXE does research inside the city loop (0x12bd0..0x12dbe): each city adds its science
//! to the civ's beakers (ds:0x6dec) and the advance is found as soon as the beakers are full,
//! so the cities after it already work on the next one.

use super::events::GameEvent;
use super::map::{CityId, Slot};
use super::Game;
use crate::rules::{self, improvement_ids as imp, wonder_ids as won, AdvanceId};


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

    /// Catch-up bonus to the beakers per advance (CIV.EXE 0x27e0, ds:0xd2de), set once a turn
    /// right after the year advances (`update_research_catch_up`): from 1 AD, the highest
    /// research count of civs 1..7 (ds:0x804c, dead ones too) less turn/9, clamped to 0..6.
    pub fn research_catch_up(&self) -> i32 {
        self.catch_up
    }

    /// Recompute ds:0xd2de (0x27b9..0x280c) for the turn that just began.
    pub fn update_research_catch_up(&mut self) {
        let lead = self.players.iter().skip(1).take(7).map(|p| p.research_count as i32).max().unwrap_or(0);
        self.catch_up = if self.year() < 0 { 0 } else { (lead - self.turn as i32 / 9).clamp(0, 6) };
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

    /// A research menu for `slot` is queued and not yet answered (the human's goal is being
    /// chosen; CIV.EXE asks at once, civrs between the turn's steps).
    fn research_menu_queued(&self, slot: Slot) -> bool {
        self.events.iter().any(|e| matches!(e, GameEvent::NeedsDecision(super::Decision::ChooseResearch { slot: s, .. }) if *s == slot))
    }

    /// The research step of one city (CIV.EXE 0x12bde..0x12dbe), with the city's science
    /// `sci` (0 under Anarchy). For the human: a goal learned some other way, or no goal while
    /// beakers are stored, brings up the research menu and the beakers are lost (0x12bf1..
    /// 0x12c5d). Then the beakers grow by `sci`, again by `sci` on Chieftain when someone
    /// else already found the human's goal (0x12c93, es:0x4b0 first-discoverer table), and by
    /// `sci / 2` with the SETI Program (0x12cc0). Full beakers are spent at once on the
    /// advance (`choose_research`).
    pub fn city_science(&mut self, c: CityId, sci: i32) {
        let slot = self.city(c).owner;
        let sci = if self.player(slot).govt == rules::Govt::Anarchy { 0 } else { sci };
        let human = self.player(slot).human;
        if human && !self.research_menu_queued(slot) {
            let goal = self.player(slot).researching;
            let learned = goal.is_some_and(|a| a != rules::advance_ids::FUTURE_TECH && self.player(slot).knows(a));
            if learned || (goal.is_none() && self.player(slot).beakers != 0) {
                self.player_mut(slot).researching = None;
                self.pick_research(slot);
                self.player_mut(slot).beakers = 0;
            }
        }
        self.player_mut(slot).beakers += sci;
        if human && self.setup.difficulty == rules::Difficulty::Chieftain {
            // CIV.EXE marks an advance's first discoverer after turn 0; civrs takes any other
            // civ that knows it
            let goal = self.player(slot).researching;
            if goal.is_some_and(|a| (1..8).any(|s| s != slot && self.player(s).knows(a))) {
                self.player_mut(slot).beakers += sci;
            }
        }
        if self.owner_has_wonder(slot, won::SETI_PROGRAM) {
            self.player_mut(slot).beakers += sci / 2;
        }
        if self.research_complete(slot) {
            self.player_mut(slot).beakers = 0;
            self.choose_research(slot);
        }
    }

    /// `choose_research` (CIV.EXE 0x0bb4a) when an advance is due: a computer player is
    /// granted the advance `tech_ai::choose` draws (none on Chieftain half of the time); the
    /// human is granted the goal, if any, with the discovery screen, and asked for the next.
    pub fn choose_research(&mut self, slot: Slot) {
        if slot == 0 || !self.player(slot).alive {
            return;
        }
        if !self.player(slot).human {
            self.player_mut(slot).researching = None;
            if let Some(a) = crate::ai::tech_ai::choose(self, slot) {
                self.grant_advance(slot, a, AdvanceSource::Discovered);
                self.push_event(GameEvent::AdvanceDiscovered { slot, adv: a });
            }
            return;
        }
        if self.research_menu_queued(slot) {
            return;
        }
        if let Some(a) = self.player(slot).researching {
            // the science advisor's pick is drawn like a computer's before the advance is
            // granted, leaving out the goal itself (choose_research 0x0bb5d, 0x0bfc1)
            let advice = (self.setup.difficulty == rules::Difficulty::Chieftain && self.options.instant_advice).then(|| crate::ai::tech_ai::draw(self, slot, Some(a))).flatten();
            self.grant_advance(slot, a, AdvanceSource::Discovered);
            self.player_mut(slot).researching = None;
            self.push_event(GameEvent::AdvanceDiscovered { slot, adv: a });
            if let Some(adv) = advice {
                self.push_event(GameEvent::ScienceAdvice { slot, adv });
            }
        }
        self.pick_research(slot);
    }

    /// Give an advance to a player (no research bookkeeping); returns false if already known.
    /// CIV.EXE `grant_advance` 0x0c2f4: the advance bit and its source byte (also at turn 0,
    /// where nothing else happens); then the research count (ds:0xe898) rises, a wonder made
    /// obsolete by the first civilization to learn it is announced (0xc774), Gunpowder and
    /// Combustion make the civ's Barracks obsolete (0xc69a: all are sold off, the human is
    /// told), and the Great Library's owner gets the advance free once two civs 1..7 know it
    /// (0xc7fa, a recursive grant from -2).
    pub fn grant_advance(&mut self, slot: Slot, a: AdvanceId, source: AdvanceSource) -> bool {
        if a == rules::advance_ids::FUTURE_TECH {
            self.player_mut(slot).future_techs += 1;
            if self.turn > 0 {
                self.player_mut(slot).research_count += 1;
            }
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
        // the source is kept for the Civilopedia (CIV.EXE 0xc35f)
        if let Some(s) = self.player_mut(slot).advance_source.get_mut(a as usize) {
            *s = source.code();
        }
        let new_to_world = !(1..8).any(|s| self.player(s).knows(a));
        self.player_mut(slot).learn(a);
        if self.player(slot).researching == Some(a) {
            self.player_mut(slot).researching = None;
        }
        // advances given at the start change nothing else (0xc364)
        if self.turn == 0 {
            return true;
        }
        self.player_mut(slot).research_count += 1;
        // wonders made obsolete by the first civilisation to learn this advance (CIV.EXE 0xc774)
        for w in 1..rules::WONDERS.len() as u8 {
            if !new_to_world || rules::WONDERS[w as usize].obsolete_by != Some(a) {
                continue;
            }
            if let Some(owner) = self.wonders_built[w as usize].and_then(|c| self.city_opt(c)).map(|c| c.owner) {
                self.push_event(GameEvent::WonderObsolete { wonder: w, owner });
            }
        }
        if a == rules::advance_ids::GUNPOWDER || a == rules::advance_ids::COMBUSTION {
            for c in self.cities_of(slot) {
                self.city_mut(c).remove_improvement(imp::BARRACKS);
            }
            if self.player(slot).human {
                self.push_event(GameEvent::BarracksObsolete { slot, adv: a });
            }
        }
        self.great_library_check(a);
        true
    }

    /// The Great Library (CIV.EXE 0xc7fa, at the end of every `grant_advance`): the owner of
    /// its city, while the wonder works for it, gets `a` when two or more of civs 1..7 know it.
    /// Advances known by two civs when the Library is built only come with the next civ to
    /// learn them.
    fn great_library_check(&mut self, a: AdvanceId) {
        let Some(owner) = self.wonders_built[won::GREAT_LIBRARY as usize].and_then(|c| self.city_opt(c)).map(|c| c.owner) else { return };
        if self.player(owner).knows(a) || !self.owner_has_wonder(owner, won::GREAT_LIBRARY) {
            return;
        }
        if (1..8).filter(|s| self.player(*s).knows(a)).count() >= 2 && self.grant_advance(owner, a, AdvanceSource::GreatLibrary) {
            self.push_event(GameEvent::AdvanceAcquired { slot: owner, adv: a, source: "Great Library".into() });
        }
    }

    /// Pick what a player researches next: the human is asked (a decision is queued);
    /// computer players keep no goal in CIV.EXE (their advance is drawn when the beakers are
    /// full, `tech_ai::choose`). Returns true when a decision was queued.
    ///
    /// The human's menu (`choose_research` 0x0bb4a, 0x0c150..0x0c20c) lists the reachable
    /// advances in index order; above Chieftain, after the first two entries, those whose
    /// index less the research count is a multiple of 3 are left out.
    pub fn pick_research(&mut self, slot: Slot) -> bool {
        if self.player(slot).researching.is_some() || !self.player(slot).human {
            return false;
        }
        if self.research_menu_queued(slot) {
            return true;
        }
        let opts = self.researchable(slot);
        if opts.is_empty() {
            return false;
        }
        let count = self.player(slot).research_count as i32;
        let all = self.setup.difficulty == rules::Difficulty::Chieftain;
        let mut shown: Vec<AdvanceId> = Vec::new();
        for a in opts {
            if shown.len() <= 1 || (a as i32 - count) % 3 != 0 || all || a == rules::advance_ids::FUTURE_TECH {
                shown.push(a);
            }
        }
        self.push_event(GameEvent::NeedsDecision(super::Decision::ChooseResearch { slot, options: shown }));
        true
    }

    /// Darwin's Voyage (CIV.EXE 0xea32): `choose_research` twice. A computer player draws two
    /// advances; the human gets the goal, picks the next one in the menu and gets that too
    /// (`free_research`, granted when the menu is answered). The beakers stay.
    pub fn darwins_voyage(&mut self, slot: Slot) {
        self.choose_research(slot);
        if self.research_menu_queued(slot) {
            self.free_research += 1;
        } else {
            self.choose_research(slot);
        }
    }

    /// The human answered the research menu: a pending free advance of Darwin's Voyage is
    /// the new goal at once.
    pub fn research_chosen(&mut self, slot: Slot) {
        if self.free_research > 0 && self.player(slot).human && self.player(slot).researching.is_some() {
            self.free_research -= 1;
            self.choose_research(slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;

    #[test]
    fn human_menu_hides_every_third_advance_above_chieftain() {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let me = g.human_slot;
        g.player_mut(me).advances = 0;
        g.player_mut(me).researching = None;
        g.player_mut(me).research_count = 4;
        g.setup.difficulty = rules::Difficulty::King;
        let all = g.researchable(me);
        g.take_events();
        assert!(g.pick_research(me));
        let shown = g.take_events().into_iter().find_map(|e| match e {
            GameEvent::NeedsDecision(super::super::Decision::ChooseResearch { options, .. }) => Some(options),
            _ => None,
        });
        let shown = shown.expect("menu");
        // the first two always, then only those with (a - 4) % 3 != 0 (choose_research 0x0c19c)
        let want: Vec<AdvanceId> = all.iter().enumerate().filter(|(i, a)| *i < 2 || (**a as i32 - 4) % 3 != 0).map(|(_, a)| *a).collect();
        assert_eq!(shown, want);
        assert!(shown.len() < all.len(), "{all:?}");
        // Chieftain lists everything; computer players keep no goal
        g.setup.difficulty = rules::Difficulty::Chieftain;
        assert!(g.pick_research(me));
        let ai = g.alive_slots().into_iter().find(|s| *s != me).unwrap();
        assert!(!g.pick_research(ai));
        assert_eq!(g.player(ai).researching, None);
    }

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

    /// A human city and a computer player that is not the human.
    fn fixture() -> (Game, crate::game::CityId, Slot) {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let me = g.human_slot;
        let settler = g.units_of(me).into_iter().find(|id| g.unit(*id).kind == crate::rules::unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        g.take_events();
        let ai = g.alive_slots().into_iter().find(|s| *s != me).unwrap();
        g.turn = 10;
        (g, c, ai)
    }

    #[test]
    fn great_library_takes_an_advance_when_a_second_civ_learns_it() {
        // 0xc7fa: checked by every grant, not when the Library is built
        let (mut g, c, ai) = fixture();
        let me = g.human_slot;
        let others: Vec<Slot> = (1..8).filter(|s| *s != me && *s != ai).collect();
        let a = rules::advance_ids::CURRENCY;
        for s in 1..8 {
            g.player_mut(s).forget(a);
        }
        g.city_mut(c).add_wonder(won::GREAT_LIBRARY);
        g.wonders_built[won::GREAT_LIBRARY as usize] = Some(c);
        g.grant_advance(ai, a, AdvanceSource::Discovered);
        assert!(!g.player(me).knows(a));
        g.grant_advance(others[0], a, AdvanceSource::Discovered);
        assert!(g.player(me).knows(a));
        assert_eq!(g.player(me).advance_source(a), -2);
        assert!(g.take_events().iter().any(|e| matches!(e, GameEvent::AdvanceAcquired { slot, adv, .. } if *slot == me && *adv == a)));
    }

    #[test]
    fn gunpowder_makes_the_barracks_obsolete() {
        let (mut g, c, _) = fixture();
        let me = g.human_slot;
        g.city_mut(c).add_improvement(imp::BARRACKS);
        g.player_mut(me).forget(rules::advance_ids::GUNPOWDER);
        g.grant_advance(me, rules::advance_ids::GUNPOWDER, AdvanceSource::Discovered);
        assert!(!g.city(c).has_improvement(imp::BARRACKS));
        assert!(g.take_events().contains(&GameEvent::BarracksObsolete { slot: me, adv: rules::advance_ids::GUNPOWDER }));
    }

    #[test]
    fn catch_up_is_set_once_a_turn_from_the_leader() {
        // 0x27e0: from 1 AD, clamp(max research count - turn / 9, 0, 6)
        let (mut g, _, ai) = fixture();
        g.set_turn(200);
        g.player_mut(ai).research_count = 25;
        assert_eq!(g.research_catch_up(), 0, "not until the turn begins");
        g.update_research_catch_up();
        assert_eq!(g.research_catch_up(), (25 - 200 / 9).clamp(0, 6));
        g.set_turn(10);
        g.update_research_catch_up();
        assert_eq!(g.research_catch_up(), 0, "BC");
    }

    #[test]
    fn a_goal_learned_otherwise_costs_the_beakers() {
        // 0x12bf1: the goal is known (traded, hut) -> menu, beakers lost
        let (mut g, c, ai) = fixture();
        let me = g.human_slot;
        let goal = g.researchable(me)[0];
        g.set_research(me, goal);
        g.player_mut(me).beakers = 7;
        g.grant_advance(me, goal, AdvanceSource::From(ai));
        g.city_science(c, 3);
        assert_eq!(g.player(me).beakers, 3);
        assert!(g.take_events().iter().any(|e| matches!(e, GameEvent::NeedsDecision(crate::game::Decision::ChooseResearch { .. }))));
        // while the menu waits, the next city keeps adding
        g.player_mut(me).researching = None;
        g.events.push_back(GameEvent::NeedsDecision(crate::game::Decision::ChooseResearch { slot: me, options: vec![goal] }));
        g.city_science(c, 2);
        assert_eq!(g.player(me).beakers, 5);
    }

    #[test]
    fn full_beakers_are_spent_inside_the_city_loop() {
        let (mut g, c, _) = fixture();
        let me = g.human_slot;
        let goal = g.researchable(me)[0];
        g.set_research(me, goal);
        let need = g.player(me).research_count as i32 * g.research_multiplier(me, true);
        g.player_mut(me).beakers = need;
        g.city_science(c, 4);
        assert!(g.player(me).knows(goal));
        assert_eq!(g.player(me).beakers, 0);
        // the next city's science goes to the next advance
        g.events.clear();
        g.set_research(me, g.researchable(me)[0]);
        g.city_science(c, 4);
        assert_eq!(g.player(me).beakers, 4);
    }

    #[test]
    fn darwin_gives_the_human_the_goal_and_the_next_pick() {
        let (mut g, _, _) = fixture();
        let me = g.human_slot;
        let goal = g.researchable(me)[0];
        g.set_research(me, goal);
        let before = g.player(me).advance_count();
        g.darwins_voyage(me);
        assert!(g.player(me).knows(goal));
        assert_eq!(g.free_research, 1);
        let next = g.researchable(me)[0];
        let d = g.take_events().into_iter().find_map(|e| match e {
            GameEvent::NeedsDecision(d @ crate::game::Decision::ChooseResearch { .. }) => Some(d),
            _ => None,
        });
        let crate::game::Decision::ChooseResearch { options, .. } = d.clone().unwrap() else { unreachable!() };
        let i = options.iter().position(|a| *a == next).unwrap();
        g.apply_decision(&d.unwrap(), crate::game::DecisionReply::Choice(i));
        assert!(g.player(me).knows(next));
        assert_eq!((g.player(me).advance_count(), g.free_research), (before + 2, 0));
    }
}

