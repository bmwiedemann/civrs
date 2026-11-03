//! "Instant advice": the green CIVILIZATION NOTE hints from HELP.TXT (`civ_note`, CIV.EXE
//! o 0x3a48). CIV.EXE keeps no memory of the notes it has shown: each caller has its own
//! condition (a turn, a count, a latch of the unit loop), and civ_note itself speaks only with
//! Instant Advice on (checked when the note is shown) and in BC years.

use super::events::{Decision, GameEvent};
use super::map::tile_flags;
use super::{Game, Production, UnitId};
use crate::rules::{advance_ids, unit_ids, Terrain};

impl Game {
    /// The note `section` for the human, or nothing: civ_note (CIV.EXE o 0x3a48) only speaks
    /// in BC years. `hints_shown == u32::MAX` mutes the notes (the headless previews).
    pub fn hint(&mut self, section: &str) -> Option<GameEvent> {
        (self.player(self.human_slot).human && self.year() < 0 && self.hints_shown != u32::MAX).then(|| GameEvent::Hint(section.into()))
    }

    /// The GENERAL PRINCIPLES notes at the start of the human's unit phase: HELP1 on turns 20
    /// and 60, HELP2 on turns 40 and 80 (`move_units` CIV.EXE 0x4070..0x40a0).
    pub fn principles_hints(&mut self) {
        let section = match self.turn {
            20 | 60 => "HELP1",
            40 | 80 => "HELP2",
            _ => return,
        };
        if let Some(e) = self.hint(section) {
            self.events.push_back(e);
        }
    }

    /// Hints that belong in front of (`before`) or after an event about to be queued.
    pub(super) fn hints_for_event(&mut self, e: &GameEvent) -> (Vec<GameEvent>, Vec<GameEvent>) {
        let me = self.human_slot;
        let mut before = Vec::new();
        let mut after = Vec::new();
        match e {
            // "Use the following menu to direct their studies:" while the human knows at most
            // one advance (ds:0xe898 <= 1, CIV.EXE 0xbf9f)
            GameEvent::NeedsDecision(Decision::ChooseResearch { slot, .. }) if *slot == me && self.player(me).research_count <= 1 => {
                before.extend(self.hint("FIRSTCIV"))
            }
            // found_city (o 0x1b5d5): the human's only city
            GameEvent::CityFounded { city } if self.city(*city).owner == me && self.cities_of(me).len() == 1 => {
                after.extend(self.hint("FIRSTPRODUCT"))
            }
            // create_unit (CIV.EXE 0x8cd9): any unit of the human after turn 0, FIRSTUNIT1 while
            // its unit count ds:0x6b76 is 0, FIRSTUNIT2 while it is 1. That count is the census
            // of ai_civ_turn (0x15810: Settlers and Nuclear left out), as his turn began.
            GameEvent::ItemBuilt { city, item: Production::Unit(_) } if self.city(*city).owner == me && self.turn != 0 => {
                match self.ai_memory.census.units[me as usize & 7] {
                    0 => after.extend(self.hint("FIRSTUNIT1")),
                    1 => after.extend(self.hint("FIRSTUNIT2")),
                    _ => {}
                }
            }
            // grant_advance (CIV.EXE 0xc605): Monarchy for the human, unless it came in a
            // diplomatic audience (ds:0x3934 != -1)
            GameEvent::AdvanceDiscovered { slot, adv } if *slot == me && *adv == advance_ids::MONARCHY => after.extend(self.hint("MONARCHY")),
            GameEvent::AdvanceAcquired { slot, adv, source }
                if *slot == me && *adv == advance_ids::MONARCHY && !matches!(source.as_str(), "exchange" | "tribute" | "treaty") =>
            {
                after.extend(self.hint("MONARCHY"))
            }
            GameEvent::Disorder(c) | GameEvent::DisorderOngoing { city: c, first: true } if self.city(*c).owner == me => after.extend(self.hint("DISORDER")),
            _ => {}
        }
        (before, after)
    }

    /// Advice for the human's Settlers on its current tile (`settler_advice`, CIV.EXE
    /// o 0x3824): BUILDCITY on a good site, any time; else, next to one of the human's cities
    /// (the nearest city at distance 1), MINING on Hills, IRRIGATE on Plains with water, ROAD
    /// on a square of move cost 1 other than River while the human has fewer than two Settlers
    /// (ds:0xb3be); none without improvements there. `move_units` asks for it for the first
    /// Settlers that becomes active each turn (the latch [bp-0x8], 0x45ae); the caller keeps
    /// that latch.
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
        if self.good_city_site(x, y) {
            return self.hint("BUILDCITY");
        }
        let (city, d) = self.nearest_city_exe(x, y);
        let city = city.filter(|c| self.city(*c).owner == self.human_slot && d == 1)?;
        let bare = !t.has(tile_flags::MINE) && !t.has(tile_flags::IRRIGATION);
        let settlers = self.player(self.human_slot).unit_counter[unit_ids::SETTLERS as usize];
        let section = if t.terrain == Terrain::Hills && bare {
            "MINING"
        } else if t.terrain == Terrain::Plains && bare && self.irrigation_water(x, y) {
            "IRRIGATE"
        } else if t.terrain.info().move_cost == 1 && t.terrain != Terrain::River && !t.has(tile_flags::ROAD) && settlers < 2 {
            "ROAD"
        } else {
            return None;
        };
        let GameEvent::Hint(section) = self.hint(section)? else { return None };
        Some(GameEvent::HintAt { section, city })
    }

    /// BUILDCITY's test (CIV.EXE o 0x3824): the terrain yields at least 2 food and shields,
    /// the square's byte of map layer (x+80, y) is above 9 and no neighbour's is higher. The
    /// layer holds the generator's site values (8..15) where no city has claimed the square
    /// and owner numbers (or 8) where one has, so no note comes inside a city's radius.
    fn good_city_site(&self, x: i32, y: i32) -> bool {
        let info = self.map.tile(x, y).terrain.info();
        let value = self.site_layer(x, y);
        info.food + info.shields >= 2
            && value > 9
            && super::Dir8::ALL.iter().all(|d| {
                let (dx, dy) = d.delta();
                self.site_layer(x + dx, y + dy) <= value
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{GameSetup, Map};
    use crate::rules;

    /// A game with the human's units removed and a plains arena around (20,20) whose site
    /// layer is 8 everywhere.
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
        g.map.site = vec![8; rules::MAP_W * rules::MAP_H];
        g.take_events();
        g
    }

    #[test]
    fn notes_repeat_and_end_with_the_bc_years() {
        let mut g = arena();
        // civ_note keeps no memory of what it has shown
        assert_eq!(g.hint("FIRSTCIV"), Some(GameEvent::Hint("FIRSTCIV".into())));
        assert_eq!(g.hint("FIRSTCIV"), Some(GameEvent::Hint("FIRSTCIV".into())));
        // civ_note (o 0x3a48) is silent from 1 AD
        g.set_turn(250);
        assert_eq!(g.hint("MONARCHY"), None);
    }

    #[test]
    fn general_principles_on_turns_20_40_60_80() {
        let mut g = arena();
        for (turn, want) in [(19, None), (20, Some("HELP1")), (40, Some("HELP2")), (60, Some("HELP1")), (80, Some("HELP2")), (100, None)] {
            g.set_turn(turn);
            g.principles_hints();
            let ev = g.take_events();
            assert_eq!(ev, want.map(|s| GameEvent::Hint(s.into())).into_iter().collect::<Vec<_>>(), "turn {turn}");
        }
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
    fn first_units_by_unit_count() {
        let mut g = arena();
        let me = g.human_slot;
        g.set_turn(5);
        let s = g.add_unit(me, unit_ids::SETTLERS, 20, 20);
        let c = g.found_city(s).unwrap();
        g.take_events();
        let built = |g: &mut Game, k| {
            g.add_unit(me, k, 20, 20);
            g.push_event(GameEvent::ItemBuilt { city: c, item: Production::Unit(k) });
            g.take_events().into_iter().filter_map(|e| if let GameEvent::Hint(s) = e { Some(s) } else { None }).collect::<Vec<_>>()
        };
        // any unit built is noted (create_unit 0x8cd9) by the census's count ds:0x6b76, which
        // only the human's next turn takes again and which leaves out the Settlers
        g.take_census(me);
        assert_eq!(built(&mut g, unit_ids::SETTLERS), vec!["FIRSTUNIT1".to_string()]);
        assert_eq!(built(&mut g, unit_ids::MILITIA), vec!["FIRSTUNIT1".to_string()], "the same turn");
        g.take_census(me);
        assert_eq!(built(&mut g, unit_ids::MILITIA), vec!["FIRSTUNIT2".to_string()], "one Militia, the Settlers left out");
        g.take_census(me);
        assert!(built(&mut g, unit_ids::MILITIA).is_empty());
    }

    #[test]
    fn monarchy_note_unless_from_an_audience() {
        let mut g = arena();
        let me = g.human_slot;
        g.player_mut(me).govt = crate::rules::Govt::Monarchy;
        g.push_event(GameEvent::AdvanceDiscovered { slot: me, adv: advance_ids::MONARCHY });
        assert!(g.take_events().contains(&GameEvent::Hint("MONARCHY".into())));
        g.push_event(GameEvent::AdvanceAcquired { slot: me, adv: advance_ids::MONARCHY, source: "exchange".into() });
        assert!(!g.take_events().contains(&GameEvent::Hint("MONARCHY".into())));
        g.push_event(GameEvent::AdvanceAcquired { slot: me, adv: advance_ids::MONARCHY, source: "conquest".into() });
        assert!(g.take_events().contains(&GameEvent::Hint("MONARCHY".into())));
    }

    #[test]
    fn buildcity_on_the_best_site_layer_square() {
        let mut g = arena();
        let me = g.human_slot;
        g.map.site[Map::idx(20, 20)] = 12;
        g.map.site[Map::idx(21, 20)] = 11;
        assert!(g.good_city_site(20, 20));
        assert!(!g.good_city_site(21, 20), "a better neighbour");
        g.map.site[Map::idx(22, 22)] = 9;
        assert!(!g.good_city_site(22, 22), "a value of 9 is not enough");
        let s = g.add_unit(me, unit_ids::SETTLERS, 20, 20);
        assert_eq!(g.settler_hint(s), Some(GameEvent::Hint("BUILDCITY".into())));
    }

    #[test]
    fn no_buildcity_hint_inside_another_city_radius() {
        let mut g = arena();
        let me = g.human_slot;
        for (x, y) in [(22, 21), (21, 22), (20, 18)] {
            g.map.site[Map::idx(x, y)] = 14;
        }
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
        g.player_mut(me).research_count = 1;
        assert!(g.pick_research(me));
        let ev = g.take_events();
        assert_eq!(ev[0], GameEvent::Hint("FIRSTCIV".into()));
        assert!(matches!(ev[1], GameEvent::NeedsDecision(Decision::ChooseResearch { .. })));
        // not once the human knows two advances (ds:0xe898 > 1, 0xbf9f)
        g.player_mut(me).researching = None;
        g.player_mut(me).research_count = 2;
        assert!(g.pick_research(me));
        assert!(!g.take_events().iter().any(|e| matches!(e, GameEvent::Hint(_))));
    }

    #[test]
    fn settler_advice_names_the_city() {
        let mut g = arena();
        let me = g.human_slot;
        // the advice counts Settlers by ds:0xb3be, which starts at 0 here as after civ_init
        g.player_mut(me).unit_counter = [0; 28];
        let s = g.add_unit(me, unit_ids::SETTLERS, 20, 20);
        let c = g.found_city(s).unwrap();
        g.map.get_mut(21, 20).unwrap().terrain = Terrain::Hills;
        let s = g.add_unit(me, unit_ids::SETTLERS, 21, 20);
        assert_eq!(g.settler_hint(s), Some(GameEvent::HintAt { section: "MINING".into(), city: c }));
        // two squares away: nothing (only next to the city, distance 1)
        let s2 = g.add_unit(me, unit_ids::SETTLERS, 22, 21);
        assert_eq!(g.settler_hint(s2), None);
        // plains next to the city with nothing wet: no irrigation, and a road only while
        // the human has fewer than two Settlers
        g.remove_unit(s2);
        g.remove_unit(s);
        let s = g.add_unit(me, unit_ids::SETTLERS, 19, 19);
        assert_eq!(g.settler_hint(s), Some(GameEvent::HintAt { section: "ROAD".into(), city: c }));
        g.add_unit(me, unit_ids::SETTLERS, 26, 26);
        assert_eq!(g.settler_hint(s), None);
    }
}
