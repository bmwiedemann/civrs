//! "Instant advice": the green CIVILIZATION NOTE hints from HELP.TXT. Each hint is shown at
//! most once per game; `Game::hints_shown` remembers which ones the player has seen.

use super::events::{Decision, GameEvent};
use super::map::{tile_flags, Map};
use super::{CityId, Game, Production, UnitId};
use crate::rules::{self, advance_ids, unit_ids, Govt, Terrain};

/// HELP.TXT sections used as in-game hints; the index is the bit in `Game::hints_shown`.
pub const HINTS: [&str; 11] =
    ["FIRSTMOVE", "BUILDCITY", "FIRSTPRODUCT", "FIRSTCIV", "FIRSTUNIT1", "FIRSTUNIT2", "MINING", "IRRIGATE", "ROAD", "MONARCHY", "DISORDER"];

/// Minimum `site_score` for the BUILDCITY hint (the AI's own threshold for a city site).
const GOOD_SITE: i32 = 80;

impl Game {
    /// Mark `section` as shown; true when it had not been shown before.
    fn claim_hint(&mut self, section: &str) -> bool {
        let Some(bit) = HINTS.iter().position(|h| *h == section) else { return false };
        let fresh = self.hints_shown & (1 << bit) == 0;
        self.hints_shown |= 1 << bit;
        fresh
    }

    /// The hint for `section` unless it was already shown (or there is no human player).
    pub fn hint(&mut self, section: &str) -> Option<GameEvent> {
        (self.player(self.human_slot).human && self.claim_hint(section)).then(|| GameEvent::Hint(section.into()))
    }

    /// Hints that belong in front of (`before`) or after an event about to be queued.
    pub(super) fn hints_for_event(&mut self, e: &GameEvent) -> (Vec<GameEvent>, Vec<GameEvent>) {
        let me = self.human_slot;
        let mut before = Vec::new();
        let mut after = Vec::new();
        match e {
            // "Use the following menu to direct their studies:"
            GameEvent::NeedsDecision(Decision::ChooseResearch { slot, .. }) if *slot == me => before.extend(self.hint("FIRSTCIV")),
            GameEvent::CityFounded { city } if self.city(*city).owner == me && self.cities_of(me).len() == 1 => {
                after.extend(self.hint("FIRSTPRODUCT"))
            }
            GameEvent::ItemBuilt { city, item: Production::Unit(k) } if self.city(*city).owner == me => {
                let info = &rules::UNITS[*k as usize];
                if info.att > 0 && *k != unit_ids::SETTLERS && *k != unit_ids::CARAVAN && *k != unit_ids::DIPLOMAT {
                    after.extend(self.hint("FIRSTUNIT1"));
                    after.extend(self.hint("FIRSTUNIT2"));
                }
            }
            GameEvent::AdvanceDiscovered { slot, adv } | GameEvent::AdvanceAcquired { slot, adv, .. }
                if *slot == me && *adv == advance_ids::MONARCHY && matches!(self.player(me).govt, Govt::Despotism | Govt::Anarchy) =>
            {
                after.extend(self.hint("MONARCHY"))
            }
            GameEvent::Disorder(c) if self.city(*c).owner == me => after.extend(self.hint("DISORDER")),
            _ => {}
        }
        (before, after)
    }

    /// Advice for the human's active Settlers on its current tile: BUILDCITY before the first
    /// city, then MINING / IRRIGATE / ROAD inside a city's radius.
    pub fn settler_hint(&mut self, id: UnitId) -> Option<GameEvent> {
        let u = self.unit_opt(id)?;
        if u.kind != unit_ids::SETTLERS || u.owner != self.human_slot || !self.player(u.owner).human {
            return None;
        }
        let (x, y) = u.pos();
        let t = self.map.tile(x, y);
        if !t.is_land() || t.city.is_some() {
            return None;
        }
        if self.cities_of(u.owner).is_empty() {
            if self.good_city_site(x, y) {
                return self.hint("BUILDCITY");
            }
            return None;
        }
        let city = self.radius_city(x, y)?;
        let section = if t.terrain == Terrain::Hills && !t.has(tile_flags::MINE) && !t.has(tile_flags::IRRIGATION) {
            "MINING"
        } else if matches!(t.terrain, Terrain::Grassland | Terrain::Plains | Terrain::Desert | Terrain::River)
            && !t.has(tile_flags::IRRIGATION)
            && !t.has(tile_flags::MINE)
            && self.irrigation_water(x, y)
        {
            "IRRIGATE"
        } else if matches!(t.terrain, Terrain::Grassland | Terrain::Plains | Terrain::Desert) && !t.has(tile_flags::ROAD) {
            "ROAD"
        } else {
            return None;
        };
        if !self.claim_hint(section) {
            return None;
        }
        Some(GameEvent::HintAt { section: section.into(), city })
    }

    /// BUILDCITY's test (CIV.EXE o 0x3824): the square yields at least 2 food and shields,
    /// it is a good site and no neighbour is a better one, and no city works it (the original
    /// reads its site value from a map layer that founding a city overwrites over the radius).
    fn good_city_site(&self, x: i32, y: i32) -> bool {
        let info = self.map.tile(x, y).terrain.info();
        if info.food + info.shields < 2 {
            return false;
        }
        let in_radius = self.all_cities().any(|c| rules::CITY_RADIUS.iter().any(|&(dx, dy)| (Map::wrap_x(c.x + dx as i32), c.y + dy as i32) == (Map::wrap_x(x), y)));
        if in_radius {
            return false;
        }
        let score = super::mapgen::site_score(&self.map, x, y);
        if score < GOOD_SITE {
            return false;
        }
        super::Dir8::ALL.iter().all(|d| {
            let (dx, dy) = d.delta();
            let (nx, ny) = (x + dx, y + dy);
            !Map::in_bounds(nx, ny) || super::mapgen::site_score(&self.map, nx, ny) <= score
        })
    }

    /// The human's nearest city whose radius covers (x,y).
    fn radius_city(&self, x: i32, y: i32) -> Option<CityId> {
        self.all_cities()
            .filter(|c| c.owner == self.human_slot)
            .filter(|c| rules::CITY_RADIUS.iter().any(|(dx, dy)| c.x + *dx as i32 == x && c.y + *dy as i32 == y))
            .min_by_key(|c| Map::dist((c.x, c.y), (x, y)))
            .map(|c| c.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;

    /// A game with the human's units removed and a plains arena around (20,20).
    fn arena() -> Game {
        let mut g = Game::new(GameSetup { seed: 3, ..Default::default() }, None);
        let me = g.human_slot;
        for id in g.units_of(me) {
            g.remove_unit(id);
        }
        for y in 14..27 {
            for x in 14..27 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Plains;
                t.flags = 0;
                t.special = false;
            }
        }
        g.take_events();
        g
    }

    #[test]
    fn each_hint_is_shown_once() {
        let mut g = arena();
        assert_eq!(g.hint("FIRSTCIV"), Some(GameEvent::Hint("FIRSTCIV".into())));
        assert_eq!(g.hint("FIRSTCIV"), None);
        assert_eq!(g.hint("NOSUCHHINT"), None);
    }

    #[test]
    fn first_city_queues_firstproduct_after_founding() {
        let mut g = arena();
        let me = g.human_slot;
        let s = g.add_unit(me, unit_ids::SETTLERS, 20, 20);
        let c = g.found_city(s).unwrap();
        let ev = g.take_events();
        let at = ev.iter().position(|e| *e == GameEvent::CityFounded { city: c }).unwrap();
        assert_eq!(ev[at + 1], GameEvent::Hint("FIRSTPRODUCT".into()));
        let s2 = g.add_unit(me, unit_ids::SETTLERS, 24, 24);
        g.found_city(s2).unwrap();
        assert!(!g.take_events().iter().any(|e| matches!(e, GameEvent::Hint(_))));
    }

    #[test]
    fn no_buildcity_hint_inside_another_city_radius() {
        let mut g = arena();
        let me = g.human_slot;
        let other = (0..8u8).find(|s| *s != me && *s != 0 && g.player(*s).alive).unwrap();
        let s = g.add_unit(other, unit_ids::SETTLERS, 20, 20);
        g.found_city(s).unwrap();
        g.take_events();
        for (x, y) in [(22, 21), (21, 22), (20, 18)] {
            assert!(!g.good_city_site(x, y), "({x},{y}) is worked by the city");
        }
        let mine = g.add_unit(me, unit_ids::SETTLERS, 22, 21);
        assert_eq!(g.settler_hint(mine), None);
    }

    #[test]
    fn research_menu_is_preceded_by_firstciv() {
        let mut g = arena();
        let me = g.human_slot;
        g.player_mut(me).researching = None;
        assert!(g.pick_research(me));
        let ev = g.take_events();
        assert_eq!(ev[0], GameEvent::Hint("FIRSTCIV".into()));
        assert!(matches!(ev[1], GameEvent::NeedsDecision(Decision::ChooseResearch { .. })));
    }

    #[test]
    fn settler_advice_names_the_city() {
        let mut g = arena();
        let me = g.human_slot;
        let s = g.add_unit(me, unit_ids::SETTLERS, 20, 20);
        let c = g.found_city(s).unwrap();
        g.map.get_mut(21, 20).unwrap().terrain = Terrain::Hills;
        let s = g.add_unit(me, unit_ids::SETTLERS, 21, 20);
        assert_eq!(g.settler_hint(s), Some(GameEvent::HintAt { section: "MINING".into(), city: c }));
        assert_eq!(g.settler_hint(s), None);
        // plains next to nothing wet: no irrigation, but a road
        let s = g.add_unit(me, unit_ids::SETTLERS, 22, 21);
        assert_eq!(g.settler_hint(s), Some(GameEvent::HintAt { section: "ROAD".into(), city: c }));
        // outside every city radius: nothing
        g.hints_shown = 0;
        let far = g.add_unit(me, unit_ids::SETTLERS, 26, 26);
        assert_eq!(g.settler_hint(far), None);
    }

    #[test]
    fn hints_survive_save_and_old_saves_load() {
        let mut g = arena();
        g.hint("MONARCHY");
        let dir = std::env::temp_dir().join(format!("civrs-hints-{}", std::process::id()));
        let path = dir.join("h.sav");
        crate::save::save_game(&path, &g).unwrap();
        assert_eq!(crate::save::load_game(&path).unwrap().hints_shown, g.hints_shown);
        // version 1: no trailer
        let mut v1 = b"CIVR".to_vec();
        v1.extend_from_slice(&1u16.to_le_bytes());
        v1.extend_from_slice(&bincode::serialize(&g).unwrap());
        std::fs::write(&path, v1).unwrap();
        assert_eq!(crate::save::load_game(&path).unwrap().hints_shown, 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}
