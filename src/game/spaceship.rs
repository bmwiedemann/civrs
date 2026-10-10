//! Spaceship construction and flight.
//!
//! SS Structural/Component/Module "improvements" are not city improvements: completing one
//! increments the owner's part counters (`add_spaceship_part`, called from production
//! completion). Modules are assigned in rotation: habitation, life support, solar panel.
//!
//! [approx] formulas: structurals needed to carry the parts `ceil((components + modules) * 39 / 28)`;
//! flight time `max(5, 25 - components)` turns; success chance
//! `100 * min(life_support, habitation) / habitation`, scaled by the share of structurals present.

use super::events::{EndReason, GameEvent};
use super::map::Slot;
use super::{Game, RuleError};
use crate::rules::{improvement_ids as imp, ImprovementId};

pub const MAX_STRUCTURALS: u8 = 39;
pub const MAX_COMPONENTS: u8 = 16;
pub const MAX_MODULES: u8 = 12;

/// Modules split by type: (habitation, life support, solar panels).
pub fn module_split(modules: u8) -> (i32, i32, i32) {
    let m = modules as i32;
    ((m + 2) / 3, (m + 1) / 3, m / 3)
}

/// Structurals required to attach all components and modules.
pub fn structurals_needed(components: u8, modules: u8) -> i32 {
    let parts = components as i32 + modules as i32;
    (parts * 39 + 27) / 28
}

/// Turns of flight for a ship with `components` propulsion/fuel parts.
pub fn flight_turns(components: u8) -> u32 {
    (25 - components as i32).max(5) as u32
}

impl Game {
    /// Record a completed spaceship part; false when the limit for that part is reached.
    pub fn add_spaceship_part(&mut self, slot: Slot, part: ImprovementId) -> bool {
        let s = &mut self.player_mut(slot).spaceship;
        if s.launched() {
            return false;
        }
        match part {
            imp::SS_STRUCTURAL if s.structurals < MAX_STRUCTURALS => s.structurals += 1,
            imp::SS_COMPONENT if s.components < MAX_COMPONENTS => s.components += 1,
            imp::SS_MODULE if s.modules < MAX_MODULES => s.modules += 1,
            _ => return false,
        }
        s.lost = false;
        true
    }

    pub fn spaceship_can_launch(&self, slot: Slot) -> bool {
        let s = self.player(slot).spaceship;
        self.apollo_built
            && !s.launched()
            && s.components >= 2
            && s.modules >= 3
            && s.structurals as i32 >= structurals_needed(s.components, s.modules)
            && self.player(slot).capital.and_then(|c| self.city_opt(c)).is_some()
    }

    /// Probability (percent) that the ship reaches Alpha Centauri.
    pub fn success_chance(&self, slot: Slot) -> i32 {
        let s = self.player(slot).spaceship;
        let (hab, life, _solar) = module_split(s.modules);
        if hab == 0 {
            return 0;
        }
        let base = 100 * life.min(hab) / hab;
        let needed = structurals_needed(s.components, s.modules).max(1);
        let structural = (100 * s.structurals as i32 / needed).min(100);
        (base * structural / 100).clamp(0, 100)
    }

    pub fn launch_spaceship(&mut self, slot: Slot) -> Result<(), RuleError> {
        if !self.spaceship_can_launch(slot) {
            return Err(RuleError::Other("The spaceship is not ready for launch."));
        }
        let turn = self.turn;
        let s = &mut self.player_mut(slot).spaceship;
        s.launched_turn = Some(turn);
        s.arrival_turn = Some(turn + flight_turns(s.components));
        s.lost = false;
        self.push_event(GameEvent::SpaceshipLaunched(slot));
        Ok(())
    }

    /// Year in which a launched ship arrives (for the F12 report).
    pub fn spaceship_arrival_year(&self, slot: Slot) -> Option<i32> {
        self.player(slot).spaceship.arrival_turn.map(crate::rules::year_for_turn)
    }

    /// The capital was captured: a ship in flight is lost.
    pub fn on_capital_lost(&mut self, slot: Slot) {
        let s = self.player(slot).spaceship;
        if s.launched() {
            self.lose_spaceship(slot);
        }
    }

    fn lose_spaceship(&mut self, slot: Slot) {
        let s = &mut self.player_mut(slot).spaceship;
        s.structurals = 0;
        s.components = 0;
        s.modules = 0;
        s.launched_turn = None;
        s.arrival_turn = None;
        s.lost = true;
        self.push_event(GameEvent::SpaceshipLost(slot));
    }

    /// Ships whose arrival turn has come: roll for success. Called once per turn from
    /// `process_world` (before the turn counter advances).
    pub fn spaceship_arrivals(&mut self) {
        for slot in self.alive_slots() {
            let s = self.player(slot).spaceship;
            let Some(arrival) = s.arrival_turn else { continue };
            if self.turn + 1 < arrival {
                continue;
            }
            let chance = self.success_chance(slot);
            if self.rng.chance(chance as u32) {
                self.player_mut(slot).spaceship.arrival_turn = None;
                self.push_event(GameEvent::SpaceshipArrived(slot));
                if self.ended.is_none() {
                    self.ended = Some(EndReason::Spaceship(slot));
                    self.push_event(GameEvent::GameEnded(EndReason::Spaceship(slot)));
                }
            } else {
                self.lose_spaceship(slot);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;
    use crate::rules::unit_ids;

    #[test]
    fn build_launch_arrive() {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let slot = g.human_slot;
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        g.found_city(settler).unwrap();
        g.apollo_built = true;
        assert!(!g.spaceship_can_launch(slot));
        for _ in 0..2 {
            assert!(g.add_spaceship_part(slot, imp::SS_COMPONENT));
        }
        for _ in 0..3 {
            assert!(g.add_spaceship_part(slot, imp::SS_MODULE));
        }
        let need = structurals_needed(2, 3);
        for _ in 0..need {
            assert!(g.add_spaceship_part(slot, imp::SS_STRUCTURAL));
        }
        assert!(g.spaceship_can_launch(slot));
        assert_eq!(g.success_chance(slot), 100);
        g.launch_spaceship(slot).unwrap();
        assert!(!g.add_spaceship_part(slot, imp::SS_MODULE));
        let arrival = g.player(slot).spaceship.arrival_turn.unwrap();
        assert_eq!(arrival, flight_turns(2));
        g.turn = arrival - 1;
        g.spaceship_arrivals();
        assert_eq!(g.ended, Some(EndReason::Spaceship(slot)));
        assert_eq!(module_split(12), (4, 4, 4));
        assert_eq!(module_split(4), (2, 1, 1));
    }
}
