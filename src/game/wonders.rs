//! The wonders the computer civilizations get at the end of every turn without building them
//! (CIV.EXE 0x3ae7 `global_warming_and_ai_wonders`, the part after the warming step).

use super::events::GameEvent;
use super::{Decision, Game, Production};
use crate::rules::{self, advance_ids as adv, wonder_ids as won};

impl Game {
    /// Up to difficulty + 1 tries (0x3c1d..0x3ebe), the first success ends them: a random
    /// wonder 1..21 (random(21) + 1), on Chieftain only one whose prerequisite the human knows;
    /// below Prince one that a city of the human is building becomes the Manhattan Project.
    /// Then a random city slot (random(128)); its owner, not the human, gets the wonder when it
    /// is still unbuilt, the city's size exceeds the cost in rows / 10 and the owner knows the
    /// prerequisite. The Manhattan Project needs Rocketry; on the first try a civ that knows
    /// Rocketry while the human does not is given it instead of the drawn wonder, and once the
    /// human knows Rocketry it goes only to a civ no stronger than the human (ds:0x6e82).
    ///
    /// The city's store is emptied and its owner pays a third of its gold, at most ten times
    /// the cost; Darwin's Voyage then gives two advances (0x3bab); the human's cities that were
    /// building the wonder open their city screens (0x3bcd..0x3c1b).
    ///
    /// (CIV.EXE's Chieftain test reads the prerequisite of improvement w, ds:0xbb6 + 0x1e * w,
    /// at 0x3c4a; the wonder's own is taken here. Owners with the revolution bit [civ][0] & 4
    /// are skipped, 0x3cdb; only the human's is ever set, 0x12820.)
    pub fn ai_wonder_purchases(&mut self) {
        let human = self.human_slot;
        let difficulty = self.setup.difficulty as i32;
        let rocketry = |g: &Game, s| g.player(s).knows(adv::ROCKETRY);
        for attempt in 0..=difficulty {
            let mut w = self.rng.below(21) as u8 + 1;
            if difficulty == 0 && !rules::WONDERS[w as usize].prereq.is_none_or(|a| self.player(human).knows(a)) {
                continue;
            }
            if difficulty < 2 && self.all_cities().any(|c| c.owner == human && c.production == Production::Wonder(w)) {
                w = won::MANHATTAN_PROJECT;
            }
            let slot = self.rng.below(128) as usize;
            let Some(c) = self.cities.get(slot).and_then(|c| c.as_ref()).map(|c| c.id) else { continue };
            let civ = self.city(c).owner;
            if w == won::MANHATTAN_PROJECT && !rocketry(self, civ) {
                continue;
            }
            if attempt == 0 && rocketry(self, civ) && !rocketry(self, human) && self.wonders_built[won::MANHATTAN_PROJECT as usize].is_none() {
                w = won::MANHATTAN_PROJECT;
            }
            if w == won::MANHATTAN_PROJECT && rocketry(self, human) && self.military_strength(civ) > self.military_strength(human) {
                continue;
            }
            // the barbarians know no advance (knows_advance 0xc87b), so never qualify
            if civ == human || civ == 0 || self.wonders_built[w as usize].is_some() {
                continue;
            }
            let info = &rules::WONDERS[w as usize];
            if self.city(c).size as i32 <= info.cost as i32 / 10 || !info.prereq.is_none_or(|a| self.player(civ).knows(a)) {
                continue;
            }
            self.wonders_built[w as usize] = Some(c);
            {
                let cm = self.city_mut(c);
                cm.add_wonder(w);
                cm.shields = 0;
                cm.caravan_shields = 0;
            }
            self.replay_log(super::replay::kind::WONDER, &[civ, w + 1]);
            let gold = self.player(civ).gold;
            self.player_mut(civ).gold -= (gold / 3).clamp(0, 10 * info.cost as i32);
            self.push_event(GameEvent::WonderBuilt { city: c, wonder: w, owner: civ });
            match w {
                won::MANHATTAN_PROJECT => self.nukes_enabled = true,
                won::APOLLO_PROGRAM => self.apollo_built = true,
                // choose_research twice (0x3bb7, 0x3bc5)
                won::DARWINS_VOYAGE => self.darwins_voyage(civ),
                _ => {}
            }
            for hc in self.cities_of(human) {
                if self.city(hc).production == Production::Wonder(w) {
                    self.push_event(GameEvent::NeedsDecision(Decision::ProductionChoice { city: hc, reason: "wonder".into() }));
                }
            }
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;

    /// A world with one computer city of size 12 that knows every advance.
    fn rich_ai_city(difficulty: rules::Difficulty) -> (Game, crate::game::CityId, crate::game::Slot) {
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        g.setup.difficulty = difficulty;
        let ai = g.alive_slots().into_iter().find(|s| *s != g.human_slot).unwrap();
        let settler = g.units_of(ai).into_iter().find(|id| g.unit(*id).kind == rules::unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        g.city_mut(c).size = 12;
        for a in 0..67 {
            g.player_mut(ai).learn(a);
        }
        g.player_mut(ai).gold = 900;
        g.take_events();
        (g, c, ai)
    }

    #[test]
    fn a_computer_city_gets_a_wonder_for_a_third_of_the_gold() {
        // 0x3dd3..0x3e83: only that one city exists, so some turn's draws hit it
        let (mut g, c, ai) = rich_ai_city(rules::Difficulty::Emperor);
        g.city_mut(c).shields = 50;
        let mut turns = 0;
        while !g.wonders_built.iter().any(|w| *w == Some(c)) {
            g.ai_wonder_purchases();
            turns += 1;
            assert!(turns < 10_000);
        }
        let w = g.wonders_built.iter().position(|w| *w == Some(c)).unwrap() as u8;
        assert!(g.city(c).has_wonder(w));
        assert_eq!(g.city(c).shields, 0);
        assert_eq!(g.player(ai).gold, 900 - (300.min(10 * rules::WONDERS[w as usize].cost as i32)));
        assert!(g.take_events().iter().any(|e| matches!(e, GameEvent::WonderBuilt { city, owner, .. } if *city == c && *owner == ai)));
    }

    #[test]
    fn small_cities_and_built_wonders_are_passed_over() {
        // size must exceed cost / 10 (0x3db6); a built wonder is never given again (0x3d89)
        let (mut g, c, _) = rich_ai_city(rules::Difficulty::Emperor);
        g.city_mut(c).size = 1;
        for _ in 0..2000 {
            g.ai_wonder_purchases();
        }
        assert!(g.wonders_built.iter().all(|w| *w != Some(c)));
        g.city_mut(c).size = 12;
        for w in 1..22u8 {
            if w != won::PYRAMIDS {
                g.wonders_built[w as usize] = Some(999);
            }
        }
        for _ in 0..20_000 {
            g.ai_wonder_purchases();
        }
        assert_eq!(g.wonders_built[won::PYRAMIDS as usize], Some(c));
    }
}
