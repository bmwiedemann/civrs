//! City management for computer players: production choice, purchases and tax rates. Workers
//! and entertainers are the city turn's business (`Game::assign_workers`), as in CIV.EXE.

use crate::game::{CityId, Game, Production, Slot};
use crate::rules::{self, improvement_ids as imp, unit_ids};

/// What the city should build next: CIV.EXE's production advisor (0xa5e:0x41d), see
/// `game::production`. The computer asks it when a city is founded, after every unit,
/// improvement or wonder it completes, when an item becomes impossible, and on every city of a
/// continent whose policy changed (0x15f26 -> 0x18bef); never otherwise.
pub fn choose_production(g: &mut Game, c: CityId) -> Production {
    g.ai_production(c)
}

/// Re-choose on every city of `slot` on continent index `k` (`None`: all of them),
/// CIV.EXE 0x18bef `ai_rechoose_production`.
pub fn rechoose(g: &mut Game, slot: Slot, k: Option<usize>) {
    let cities = g
        .cities_of(slot)
        .into_iter()
        .filter(|c| {
            let city = g.city(*c);
            k.map(|k| crate::game::production::continent_index(g.map.tile(city.x, city.y).continent) == k).unwrap_or(true)
        })
        .collect();
    g.rechoose_loop(cities);
}

/// Per-turn management of one AI city.
pub fn manage(g: &mut Game, c: CityId) {
    if g.city_opt(c).is_none() {
        return;
    }
    // the advisor's item can no longer be built (a wonder finished elsewhere, an improvement
    // the city got another way). Deviation: CIV.EXE has no such rule; the city keeps the item
    // (completion does nothing, 0xe4e8 -> 0xeb15) until an alert (0xec34), a change of
    // disorder (0x1275a), a capture (0x1462f) or a free advance (0xbc5d) asks the advisor;
    // civrs asks at the city's next turn.
    if g.production_invalid(c) {
        let p = choose_production(g, c);
        g.city_mut(c).production = p;
    }
}

/// The end of a computer city's turn in CIV.EXE's city routine (0xec0d..0xef79, after the
/// production): a city on alert (attacked since its last turn, `City::auto_build`) asks the
/// advisor again (0x1579:0x34bc); then the treasury pays for shields, two gold each, never
/// through the price of the BUY button. Each rule that applies replaces the amount of the one
/// before:
/// - gold/64 for a unit whose AI role is the continent's policy (hostile 1 = attack, threatened
///   2 = defend, offensive 5 = transport) while the city makes shields (`net_shields` != 0);
/// - gold/128 for a spaceship part while the human's ship is under way;
/// - in disorder, for an improvement or wonder: what is missing, at most gold/8;
/// - for a unit while nobody stands in the city square, or on alert: what is missing, at most
///   gold/3;
/// - for the Palace (production -1): what is missing of the "unit cost" at 0x1142 + 0x22 * -1 =
///   ds:0x1120, a word of the table before the unit table (127, wonder obsolescence), times the
///   cost factor (0xee8f), at most gold/3;
/// - when the city status byte is exactly 0x19 (disorder, hydro site, alert and nothing else)
///   and the civilization has no Nuclear unit: what is missing of the unit, at most gold/4
///   (0xeeab). CIV.EXE does not test for a unit there: for an improvement or wonder it reads
///   the "unit cost" word at 0x1142 + 0x22 * production from the tables before the unit table
///   (`EXE_COST_BEFORE_UNITS`), times the cost factor; only with `exe_quirks` (K t390:
///   Tenochtitlan's Cathedral reads 40, the cost of improvement-table entry 35, and buys 276
///   shields).
///
/// Then gold/512 more above 2000 gold. The alert is cleared.
///
/// Shield costs carry the owner's cost factor ([bp-0x4a], `Game::cost_factor`).
pub fn city_turn_purchase(g: &mut Game, c: CityId, net_shields: i32) {
    let slot = g.city(c).owner;
    if g.city(c).auto_build {
        let p = g.ai_rechoose(c);
        g.city_mut(c).production = p;
    }
    let amount = purchase_amount(g, c, net_shields);
    let cm = g.city_mut(c);
    cm.shields += amount;
    cm.auto_build = false;
    g.player_mut(slot).gold -= 2 * amount;
}

/// The shields `city_turn_purchase` buys (after the alert's re-choice).
fn purchase_amount(g: &Game, c: CityId, net_shields: i32) -> i32 {
    let slot = g.city(c).owner;
    let city = g.city(c).clone();
    let gold = g.player(slot).gold;
    let missing = g.production_cost(c, city.production) - city.shields;
    let mut amount = 0;
    if let Production::Unit(u) = city.production {
        if net_shields != 0 {
            // the policy of the civ's last AI turn (0xec5e)
            let k = crate::game::production::continent_index(g.map.tile(city.x, city.y).continent);
            let policy = g.stored_policy(slot, k);
            if matches!(policy, 1 | 2 | 5) && rules::UNITS[u as usize].role as u8 == policy {
                amount = gold / 64;
            }
        }
    }
    if g.player(g.human_slot).spaceship.launched() && matches!(city.production, Production::Improvement(i) if (imp::SS_STRUCTURAL..=imp::SS_MODULE).contains(&i)) {
        amount = gold / 128;
    }
    let unit = matches!(city.production, Production::Unit(_));
    if city.disorder && !unit && city.shields != 0 {
        amount = missing.max(0).min(gold / 8);
    }
    // (unit_owner_at 0x1a68:0x14e6 at 0xedc1: the map page's unit bit, `exe_unit_bit_lost`)
    if (g.unit_owner_at(city.x, city.y).is_none() || city.auto_build) && unit && city.shields != 0 {
        amount = missing.max(0).min(gold / 3);
    }
    let factor = g.cost_factor(slot);
    if city.production == Production::Improvement(imp::PALACE) && city.shields != 0 {
        amount = (EXE_COST_BEFORE_UNITS[1] as i32 * factor - city.shields).max(0).min(gold / 3);
    }
    if exe_status(g, c) == 0x19 && !g.all_units().any(|u| u.owner == slot && u.kind == unit_ids::NUCLEAR) && city.shields != 0 {
        let byte = match city.production {
            Production::Unit(_) => None,
            Production::Improvement(i) => Some(i as usize),
            Production::Wonder(w) => Some(24 + w as usize),
        };
        match byte {
            None => amount = missing.max(0).min(gold / 4),
            Some(k) if g.exe_quirks => {
                // 16-bit imul and sub (0xef0b, 0xef0e)
                let v = (EXE_COST_BEFORE_UNITS[k] as i32 * factor) as i16 as i32 - city.shields;
                amount = (v as i16 as i32).max(0).min(gold / 4);
            }
            Some(_) => {}
        }
    }
    if gold > 2000 {
        amount += gold / 512;
    }
    amount
}

/// The words CIV.EXE's purchase reads as a unit's cost (ds:0x1142 + 0x22 * production) for
/// minus an improvement (1..24) or minus 24 plus a wonder (25..45): the cost field of the unit
/// table read backwards into the improvement table and its names (data segment of civ.bin,
/// constant). Entry 1 is the Palace's 127 (0xee8b); entry 0 is the Settlers' cost.
const EXE_COST_BEFORE_UNITS: [i16; 46] = [
    4, 127, -1, 0, 0, 0, 25445, 26214, 17440, 30578, 51, 40, 29295, 24936, 28792, 27756, 25964, 29255, 0, 0, 0, 0, 29806, 25461,
    27747, 55, 20, 0, 0, 0, 109, 21536, 28245, 3, 0, 0, 0, 0, 27728, 28781, -1, 20, 0, -252, -506, -252,
];

/// CIV.EXE's city status byte (+6) as far as civrs keeps its bits: 0x1 disorder, 0x2 coastal,
/// 0x4 garrison out, 0x8 Hydro Plant site, 0x10 AUTO / alert, 0x20 advance stolen, 0x40
/// celebration, 0x80 improvement sold this turn. CIV.EXE sets the coastal and hydro bits when
/// the city is founded (o 0x1b6b9, 0x1b6d2): `City::coastal` and `City::hydro_site`.
fn exe_status(g: &Game, c: CityId) -> u8 {
    let city = g.city(c);
    let bits = [
        city.disorder,
        city.coastal,
        city.garrison_out,
        city.hydro_site,
        city.auto_build,
        city.tech_stolen,
        city.celebrating,
        g.player(city.owner).sold_cities.contains(&c),
    ];
    bits.iter().enumerate().fold(0, |s, (i, b)| s | ((*b as u8) << i))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;

    /// A new computer city building Militia with 4 shields stored and nobody inside.
    fn empty_ai_city() -> (Game, CityId) {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        // King: the computer's cost factor is 16 - 2 * 3 = 10, as for the human, so a Militia
        // costs 10 shields
        g.setup.difficulty = crate::rules::Difficulty::King;
        let slot = (1..8).find(|s| *s != g.human_slot && g.player(*s).alive && !g.units_of(*s).is_empty()).unwrap();
        let settler = g.units_of(slot).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        for id in g.units_at(g.city(c).x, g.city(c).y) {
            g.remove_unit(id);
        }
        let cm = g.city_mut(c);
        cm.production = Production::Unit(unit_ids::MILITIA);
        cm.shields = 4;
        (g, c)
    }

    #[test]
    fn an_empty_computer_city_buys_its_unit_at_two_gold_a_shield() {
        let (mut g, c) = empty_ai_city();
        let slot = g.city(c).owner;
        // what is missing (6), at most gold / 3
        g.player_mut(slot).gold = 300;
        city_turn_purchase(&mut g, c, 0);
        assert_eq!((g.city(c).shields, g.player(slot).gold), (10, 288));
        g.city_mut(c).shields = 4;
        g.player_mut(slot).gold = 9;
        city_turn_purchase(&mut g, c, 0);
        assert_eq!((g.city(c).shields, g.player(slot).gold), (7, 3));
        // nothing stored yet: nothing bought
        g.city_mut(c).shields = 0;
        city_turn_purchase(&mut g, c, 0);
        assert_eq!((g.city(c).shields, g.player(slot).gold), (0, 3));
        // a rich treasury adds gold / 512 to whatever the rules give
        g.player_mut(slot).gold = 5120;
        city_turn_purchase(&mut g, c, 0);
        assert_eq!((g.city(c).shields, g.player(slot).gold), (10, 5100));
    }

    #[test]
    fn an_attacked_computer_city_rechooses_and_buys_with_units_inside() {
        let (mut g, c) = empty_ai_city();
        let slot = g.city(c).owner;
        let (x, y) = g.city(c).pos();
        g.add_unit(slot, unit_ids::PHALANX, x, y);
        g.player_mut(slot).gold = 300;
        // garrisoned and not on alert: nothing
        city_turn_purchase(&mut g, c, 0);
        assert_eq!(g.city(c).shields, 4);
        // attacked: the advisor chooses again and the unit is bought, then the alert ends
        g.city_mut(c).auto_build = true;
        let advice = g.ai_rechoose(c);
        city_turn_purchase(&mut g, c, 0);
        assert_eq!(g.city(c).production, advice);
        assert!(!g.city(c).auto_build);
        if matches!(advice, Production::Unit(_)) {
            assert_eq!(g.city(c).shields, g.production_cost(c, advice));
        }
    }

    #[test]
    fn exe_quirks_a_city_square_without_the_unit_bit_counts_as_empty() {
        // 0xedc1 asks unit_owner_at (0x1a68:0x14e6), the map page's unit bit: a square whose
        // bit a dead record's deletion cleared looks empty though a unit stands there
        let (mut g, c) = empty_ai_city();
        let slot = g.city(c).owner;
        let (x, y) = g.city(c).pos();
        g.add_unit(slot, unit_ids::PHALANX, x, y);
        g.player_mut(slot).gold = 300;
        g.exe_quirks = true;
        g.exe_unit_bit_lost = vec![false; crate::rules::MAP_W * crate::rules::MAP_H];
        g.exe_unit_bit_lost[crate::game::Map::idx(x, y)] = true;
        city_turn_purchase(&mut g, c, 0);
        assert_eq!((g.city(c).shields, g.player(slot).gold), (10, 288));
    }

    #[test]
    fn a_status_0x19_city_buys_improvements_by_the_word_before_the_unit_table_only_with_exe_quirks() {
        // K t390: Tenochtitlan in disorder (status 0x19 with its hydro site and the alert),
        // Cathedral (11, 160 shields) with 124 stored, 1807 gold
        let (mut g, c) = empty_ai_city();
        let slot = g.city(c).owner;
        let cm = g.city_mut(c);
        cm.production = Production::Improvement(imp::CATHEDRAL);
        cm.shields = 124;
        cm.disorder = true;
        cm.hydro_site = true;
        cm.auto_build = true;
        cm.coastal = false;
        cm.garrison_out = false;
        g.player_mut(slot).gold = 1807;
        assert_eq!(g.cost_factor(slot), 10);
        // the disorder rule: what is missing, at most gold/8
        assert_eq!(purchase_amount(&g, c, 0), 36);
        // CIV.EXE reads the word 40 at ds:0xfcc: 400 - 124, at most gold/4
        g.exe_quirks = true;
        assert_eq!(purchase_amount(&g, c, 0), 276);
        g.player_mut(slot).gold = 400;
        assert_eq!(purchase_amount(&g, c, 0), 100);
        // a negative word buys nothing (Barracks reads -1)
        g.city_mut(c).production = Production::Improvement(imp::BARRACKS);
        assert_eq!(purchase_amount(&g, c, 0), 0);
    }

    #[test]
    fn attacks_put_computer_cities_on_alert_but_not_the_humans() {
        let (mut g, c) = empty_ai_city();
        let slot = g.city(c).owner;
        let (x, y) = g.city(c).pos();
        let h = g.human_slot;
        g.set_relation(h, slot, crate::game::Relation::War);
        // big enough to survive the loss of its defender (CIV.EXE 0x618e)
        g.city_mut(c).size = 3;
        // only an attacker that survives puts it on alert (0x60cd jumps past 0x6141)
        let d = g.add_unit(slot, unit_ids::PHALANX, x, y);
        g.unit_mut(d).veteran = true;
        let a = g.add_unit(h, unit_ids::MILITIA, x + 1, y);
        let out = g.attack(a, x, y);
        assert!(matches!(out, crate::game::MoveOutcome::Attacked { won: false, .. }), "{out:?}");
        assert!(!g.city(c).auto_build, "not after a beaten attacker");
        for _ in 0..50 {
            let a = g.add_unit(h, unit_ids::CATAPULT, x + 1, y);
            if matches!(g.attack(a, x, y), crate::game::MoveOutcome::Attacked { won: true, .. }) {
                break;
            }
            assert!(!g.city(c).auto_build);
        }
        assert!(g.city(c).auto_build, "set when the defender loses");
        // the human's own city keeps its AUTO setting
        let hc = g.add_city(h, "Mine".into(), x + 6, y);
        g.city_mut(hc).size = 3;
        g.add_unit(h, unit_ids::PHALANX, x + 6, y);
        let b = g.add_unit(slot, unit_ids::LEGION, x + 5, y);
        g.attack(b, x + 6, y);
        assert!(!g.city(hc).auto_build);
    }
}
