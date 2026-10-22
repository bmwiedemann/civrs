//! Static rules data transcribed from CIV.EXE (v474.01).
//!
//! Every table in this module is a verbatim copy of the data decoded from
//! the original executable (names keep their original spelling, e.g.
//! "MapMaking", "RailRoad", "SuperConductor").  Where the game applies an
//! override to a raw table value the override is used and the raw value is
//! kept in a comment next to it.
//!
//! The module is self-contained: it depends only on `std` and `serde`.

pub mod advances;
pub mod city_names;
pub mod civs;
pub mod difficulty;
pub mod governments;
pub mod improvements;
pub mod misc;
pub mod terrain;
pub mod units;
pub mod wonders;

pub use advances::{advance_by_name, advance_ids, advance_pedia_key, is_real, Advance, ADVANCES};
pub use city_names::SPARE_CITY_NAMES;
pub use civs::{civs_for_slot, slot_for_civ, Civ, SlotColour, CIVS, SLOT_COLOURS};
pub use difficulty::{Difficulty, DifficultyInfo, DIFFICULTY};
pub use governments::{Government, Govt, GOVERNMENTS};
pub use improvements::{improvement_by_name, improvement_ids, Improvement, IMPROVEMENTS};
pub use misc::{
    year_for_turn, year_text, ADJECTIVES, CITY_RADIUS, COMMODITIES, CUSTOM_WORLD_OPTIONS,
    DEMOGRAPHIC_CATEGORIES, HISTORIANS, MAIN_MENU, MAP_H, MAP_W, NEWSPAPERS, RATINGS,
};
pub use terrain::{Special, Terraform, Terrain, TerrainInfo, CLEAN_POLLUTION_TURNS, FORTRESS_TURNS, RAILROAD_TURNS, ROAD_TURNS, TERRAIN};
pub use units::{unit_by_name, unit_ids, AiRole, Domain, UnitType, UNITS};
pub use wonders::{wonder_by_name, wonder_ids, Wonder, WONDERS};

/// Index into [`UNITS`].
pub type UnitTypeId = u8;
/// Index into [`ADVANCES`].
pub type AdvanceId = u8;
/// Index into [`IMPROVEMENTS`] (0 = none).
pub type ImprovementId = u8;
/// Index into [`WONDERS`] (0 = unused).
pub type WonderId = u8;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn unit_table() {
        assert_eq!(UNITS.len(), 28);
        for (i, u) in UNITS.iter().enumerate() {
            assert!(!u.name.is_empty(), "unit {i} has no name");
            if let Some(p) = u.prereq {
                assert!(is_real(p), "unit {i} prereq {p} not a real advance");
            }
            if let Some(o) = u.obsolete_by {
                assert!(is_real(o), "unit {i} obsolete_by {o} not a real advance");
            }
        }
        assert_eq!(UNITS[unit_ids::SETTLERS as usize].cost, 4);
        let bs = &UNITS[unit_ids::BATTLESHIP as usize];
        assert_eq!((bs.att, bs.def, bs.moves), (18, 12, 4));
        assert_eq!(UNITS[unit_ids::CARRIER as usize].capacity, 8);
        assert_eq!(UNITS[unit_ids::MECH_INF as usize].name, "Mech. Inf.");
        assert_eq!(unit_by_name("Trireme"), Some(unit_ids::TRIREME));
        assert_eq!(UNITS[unit_ids::CARAVAN as usize].name, "Caravan");
        assert_eq!(unit_ids::CARAVAN, 27);
    }

    #[test]
    fn advance_table() {
        assert_eq!(ADVANCES.len(), 72);
        for (i, a) in ADVANCES.iter().enumerate() {
            for p in a.prereq.iter().flatten() {
                assert!(
                    (*p as usize) < ADVANCES.len(),
                    "advance {i} prereq out of range"
                );
                assert!(is_real(*p), "advance {i} prereq {p} is a placeholder");
            }
            if (67..=70).contains(&i) {
                assert_eq!(a.name, "");
                assert!(!is_real(i as u8));
            } else {
                assert!(!a.name.is_empty(), "advance {i} has no name");
                assert!(is_real(i as u8));
            }
        }
        assert_eq!(
            ADVANCES[advance_ids::FUTURE_TECH as usize].name,
            "Future Tech."
        );
        assert_eq!(advance_ids::FUTURE_TECH, 71);
        let d = &ADVANCES[advance_ids::DEMOCRACY as usize];
        assert_eq!(d.name, "Democracy");
        assert_eq!(
            d.prereq,
            [Some(advance_ids::PHILOSOPHY), Some(advance_ids::LITERACY)]
        );
        let m = &ADVANCES[advance_ids::MONARCHY as usize];
        assert_eq!(
            m.prereq,
            [
                Some(advance_ids::CEREMONIAL_BURIAL),
                Some(advance_ids::CODE_OF_LAWS)
            ]
        );
        assert_eq!(advance_by_name("MapMaking"), Some(advance_ids::MAP_MAKING));
        assert_eq!(advance_by_name("RailRoad"), Some(advance_ids::RAILROAD));
        assert_eq!(
            advance_by_name("SuperConductor"),
            Some(advance_ids::SUPERCONDUCTOR)
        );
        assert_eq!(advance_by_name("Nope"), None);
        assert_eq!(advance_pedia_key(advance_ids::CODE_OF_LAWS), "CODE OF LAWS");
        assert_eq!(advance_pedia_key(advance_ids::FUTURE_TECH), "FUTURE TECH.");
    }

    #[test]
    fn tech_tree_is_acyclic() {
        // Kahn's algorithm over the real advances.
        let n = ADVANCES.len();
        let mut indeg: Vec<usize> = ADVANCES
            .iter()
            .map(|a| a.prereq.iter().flatten().count())
            .collect();
        let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
        let mut order = Vec::new();
        while let Some(x) = queue.pop() {
            order.push(x);
            for (i, a) in ADVANCES.iter().enumerate() {
                if a.prereq.iter().flatten().any(|&p| p as usize == x) {
                    indeg[i] -= 1;
                    if indeg[i] == 0 {
                        queue.push(i);
                    }
                }
            }
        }
        assert_eq!(order.len(), n, "tech tree has a cycle");
    }

    #[test]
    fn improvement_table() {
        assert_eq!(IMPROVEMENTS.len(), 25);
        assert_eq!(IMPROVEMENTS[0].name, "NONE");
        assert_eq!(
            IMPROVEMENTS[improvement_ids::PALACE as usize].name,
            "Palace"
        );
        assert_eq!(IMPROVEMENTS[improvement_ids::PALACE as usize].upkeep, 0);
        assert_eq!(IMPROVEMENTS[improvement_ids::BARRACKS as usize].upkeep, 1);
        assert_eq!(
            IMPROVEMENTS[improvement_ids::SS_MODULE as usize].name,
            "SS Module"
        );
        assert_eq!(improvement_ids::SS_MODULE, 24);
        assert_eq!(
            improvement_by_name("Mfg. Plant"),
            Some(improvement_ids::MFG_PLANT)
        );
        assert_eq!(
            improvement_by_name("Recycling Cntr."),
            Some(improvement_ids::RECYCLING_CNTR)
        );
        for (i, imp) in IMPROVEMENTS.iter().enumerate().skip(1) {
            assert!(!imp.name.is_empty(), "improvement {i} has no name");
            if let Some(p) = imp.prereq {
                assert!(is_real(p));
            }
        }
    }

    #[test]
    fn wonder_table() {
        assert_eq!(WONDERS.len(), 22);
        assert_eq!(WONDERS[0].name, "");
        let gw = &WONDERS[7];
        assert_eq!(gw.name, "Great Wall");
        assert_eq!(gw.obsolete_by, Some(advance_ids::GUNPOWDER));
        assert_eq!(wonder_ids::GREAT_WALL, 7);
        assert_eq!(
            WONDERS[wonder_ids::CURE_FOR_CANCER as usize].name,
            "Cure for Cancer"
        );
        assert_eq!(wonder_ids::CURE_FOR_CANCER, 21);
        assert_eq!(
            wonder_by_name("J.S.Bach's Cathedral"),
            Some(wonder_ids::JS_BACHS_CATHEDRAL)
        );
        for (i, w) in WONDERS.iter().enumerate().skip(1) {
            assert!(!w.name.is_empty(), "wonder {i} has no name");
            assert!(is_real(w.prereq.expect("wonder prereq")));
            if let Some(o) = w.obsolete_by {
                assert!(is_real(o));
            }
        }
    }

    #[test]
    fn terrain_table() {
        assert_eq!(TERRAIN.len(), 12);
        let codes: Vec<u8> = TERRAIN.iter().map(|t| t.earth_code).collect();
        assert_eq!(codes, vec![14, 6, 10, 2, 12, 13, 7, 15, 3, 11, 1, 9]);
        for (i, t) in TERRAIN.iter().enumerate() {
            let terr = Terrain::from_u8(i as u8).unwrap();
            assert_eq!(terr.as_u8(), i as u8);
            assert_eq!(terr.name(), t.name);
            assert_eq!(Terrain::from_earth_code(t.earth_code), Some(terr));
            assert!(std::ptr::eq(terr.info(), t));
        }
        assert_eq!(Terrain::from_u8(12), None);
        assert_eq!(Terrain::from_earth_code(0), None);
        assert!(Terrain::Ocean.is_ocean());
        assert!(!Terrain::River.is_ocean());
        assert_eq!(Terrain::ALL.len(), 12);
        assert_eq!(Terrain::Grassland.info().shields, 0);
        assert_eq!(Terrain::Mountains.info().defense, 6);
        assert_eq!(
            Terrain::Swamp.info().irrigate,
            Terraform::Becomes(Terrain::Grassland)
        );
        assert_eq!(Terrain::Hills.info().mine, Terraform::Bonus(3));
        assert_eq!(
            Terrain::Ocean.info().special.as_ref().map(|s| s.name),
            Some("Fish")
        );
        assert_eq!(Terrain::River.info().special, None);
    }

    #[test]
    fn civ_table() {
        assert_eq!(CIVS.len(), 16);
        assert_eq!(CIVS[0].leader, "Attila");
        assert_eq!(CIVS[0].plural, "Barbarians");
        assert_eq!(CIVS[8].leader, "");
        assert_eq!(CIVS[8].adjective, "");
        for i in (1..=7).chain(9..=15) {
            let c = &CIVS[i];
            assert!(!c.leader.is_empty(), "civ {i} has no leader");
            assert!(!c.adjective.is_empty());
            assert!(!c.plural.is_empty());
            assert_eq!(c.cities.len(), 16);
            for (j, n) in c.cities.iter().enumerate() {
                assert!(!n.is_empty(), "civ {i} city {j} is empty");
            }
        }
        assert!(CIVS[14].female);
        assert_eq!(CIVS[14].leader, "Elizabeth I");
        assert_eq!(CIVS.iter().filter(|c| c.female).count(), 1);
        assert_eq!(CIVS[11].plural, "French");
        assert_eq!(CIVS[1].plural, "Romans");
        assert_eq!(CIVS[1].cities[0], "Rome");
        assert_eq!(CIVS[15].cities[15], "Kerman");
        assert_eq!(SPARE_CITY_NAMES.len(), 32);
        assert_eq!(CIVS[0].cities[0], "Mecca");
        assert_eq!(CIVS[8].cities[15], "Sydney");
        assert_eq!(CIVS[0].cities[..], SPARE_CITY_NAMES[..16]);
        assert_eq!(CIVS[8].cities[..], SPARE_CITY_NAMES[16..]);
        assert_eq!(SLOT_COLOURS.len(), 8);
        assert_eq!((SLOT_COLOURS[0].light, SLOT_COLOURS[0].dark), (12, 4));
        for s in 0..8u8 {
            assert_eq!(civs_for_slot(s), [s, s + 8]);
            assert_eq!(slot_for_civ(s), s);
            assert_eq!(slot_for_civ(s + 8), s);
        }
    }

    #[test]
    fn government_table() {
        assert_eq!(GOVERNMENTS.len(), 6);
        for (i, g) in GOVERNMENTS.iter().enumerate() {
            let govt = Govt::from_u8(i as u8).unwrap();
            assert_eq!(govt.as_u8(), i as u8);
            assert_eq!(govt.name(), g.name);
            assert!(std::ptr::eq(govt.info(), g));
        }
        assert_eq!(
            GOVERNMENTS[Govt::Democracy as usize].prereq,
            Some(advance_ids::DEMOCRACY)
        );
        assert_eq!(
            GOVERNMENTS[Govt::Republic as usize].prereq,
            Some(advance_ids::THE_REPUBLIC)
        );
        assert_eq!(
            GOVERNMENTS[Govt::Monarchy as usize].prereq,
            Some(advance_ids::MONARCHY)
        );
        assert_eq!(
            GOVERNMENTS[Govt::Communism as usize].prereq,
            Some(advance_ids::COMMUNISM)
        );
        assert_eq!(GOVERNMENTS[Govt::Despotism as usize].prereq, None);
        assert_eq!(GOVERNMENTS[Govt::Anarchy as usize].prereq, None);
        assert_eq!(GOVERNMENTS[Govt::Monarchy as usize].title_f, "Queen");
        assert_eq!(GOVERNMENTS[Govt::Despotism as usize].title_f, "Empress");
        assert_eq!(GOVERNMENTS[Govt::Communism as usize].adjective, "Communist");
        assert_eq!(GOVERNMENTS[Govt::Democracy as usize].max_rate, 100);
    }

    #[test]
    fn difficulty_table() {
        assert_eq!(DIFFICULTY.len(), 5);
        let years: Vec<i32> = DIFFICULTY.iter().map(|d| d.end_year).collect();
        assert_eq!(years, vec![2100, 2080, 2060, 2040, 2020]);
        let content: Vec<u8> = DIFFICULTY.iter().map(|d| d.content_base).collect();
        assert_eq!(content, vec![4, 3, 2, 1, 0]);
        let settlers: Vec<u8> = DIFFICULTY.iter().map(|d| d.human_start_settlers).collect();
        assert_eq!(settlers, vec![2, 1, 1, 1, 1]);
        let techs: Vec<u8> = DIFFICULTY.iter().map(|d| d.ai_start_techs).collect();
        assert_eq!(techs, vec![0, 0, 1, 2, 3]);
        for (i, d) in DIFFICULTY.iter().enumerate() {
            let diff = Difficulty::from_u8(i as u8).unwrap();
            assert_eq!(diff.as_u8(), i as u8);
            assert_eq!(diff.name(), d.name);
            assert!(std::ptr::eq(diff.info(), d));
        }
        assert_eq!(DIFFICULTY[1].short, "Lord");
    }

    #[test]
    fn city_radius_offsets() {
        assert_eq!(CITY_RADIUS.len(), 21);
        let set: HashSet<(i8, i8)> = CITY_RADIUS.iter().copied().collect();
        assert_eq!(set.len(), 21);
        assert_eq!(CITY_RADIUS[0], (0, 0));
        for &(dx, dy) in CITY_RADIUS.iter() {
            assert!(dx.abs() <= 2 && dy.abs() <= 2);
            assert!(
                !(dx.abs() == 2 && dy.abs() == 2),
                "corners are not in the fat cross"
            );
        }
    }

    #[test]
    fn years() {
        assert_eq!(year_for_turn(0), -4000);
        assert_eq!(year_for_turn(1), -3980);
        assert_eq!(year_for_turn(250), 1000);
        assert_eq!(year_for_turn(300), 1500);
        assert_eq!(year_for_turn(350), 1750);
        assert_eq!(year_for_turn(400), 1850);
        assert_eq!(year_for_turn(401), 1851);
        assert_eq!(year_text(-4000), "4000 BC");
        assert_eq!(year_text(1), "1 AD");
        assert_eq!(year_text(1850), "1850 AD");
        assert_eq!(year_text(0), "1 AD");
    }

    #[test]
    fn name_lists() {
        assert_eq!(RATINGS.len(), 20);
        assert_eq!(RATINGS[0], "Solomon the Wise");
        assert_eq!(RATINGS[19], "Dan Quayle");
        assert_eq!(HISTORIANS.len(), 4);
        assert_eq!(ADJECTIVES.len(), 7);
        assert_eq!(COMMODITIES.len(), 8);
        assert_eq!(NEWSPAPERS.len(), 5);
        assert_eq!(DEMOGRAPHIC_CATEGORIES.len(), 5);
        assert_eq!(CUSTOM_WORLD_OPTIONS.len(), 4);
        assert_eq!(CUSTOM_WORLD_OPTIONS[3].1[0], "3 billion years");
        assert_eq!(MAIN_MENU.len(), 5);
        assert_eq!(MAIN_MENU[2], "EARTH");
        assert_eq!((MAP_W, MAP_H), (80, 50));
    }

    #[test]
    fn enums_round_trip() {
        for d in Domain::ALL {
            assert_eq!(Domain::from_u8(d.as_u8()), Some(d));
        }
        for r in AiRole::ALL {
            assert_eq!(AiRole::from_u8(r.as_u8()), Some(r));
        }
        assert_eq!(Domain::from_u8(3), None);
        assert_eq!(AiRole::from_u8(7), None);
        assert_eq!(Govt::from_u8(6), None);
        assert_eq!(Difficulty::from_u8(5), None);
    }
}
