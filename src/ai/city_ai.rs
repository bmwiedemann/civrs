//! City management for computer players: production choice, purchases and tax rates. Workers
//! and entertainers are the city turn's business (`Game::assign_workers`), as in CIV.EXE.

use crate::game::{CityId, Game, Map, Production, Slot};
use crate::rules::{self, improvement_ids as imp, unit_ids, Govt, Terrain};

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
    for c in g.cities_of(slot) {
        let city = g.city(c);
        if k.map(|k| crate::game::production::continent_index(g.map.tile(city.x, city.y).continent) == k).unwrap_or(true) {
            let p = choose_production(g, c);
            g.city_mut(c).production = p;
        }
    }
}

/// Per-turn management of one AI city.
pub fn manage(g: &mut Game, c: CityId) {
    if g.city_opt(c).is_none() {
        return;
    }
    // the advisor's item can no longer be built (a wonder finished elsewhere, an improvement
    // the city got another way): CIV.EXE cancels it on completion and asks again (0xe4f1)
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
///   ds:0x1120, a word of the table before the unit table (127, wonder obsolescence), at most
///   gold/3;
/// - when the city status byte is exactly 0x19 (disorder, hydro site, alert and nothing else)
///   and the civilization has no Nuclear unit: what is missing of the unit, at most gold/4
///   (0xeeab; with an improvement CIV.EXE reads its "cost" from unrelated data, left out).
///
/// Then gold/512 more above 2000 gold. The alert is cleared.
///
/// Not modelled: CIV.EXE reads the policy stored at the start of the civilization's turn
/// (ds:0xe698), civrs computes it now; its shield costs carry the computer's cost factor
/// (16 - 2 * difficulty, [bp-0x4a] at 0xccf7) that civrs does not have.
pub fn city_turn_purchase(g: &mut Game, c: CityId, net_shields: i32) {
    let slot = g.city(c).owner;
    if g.city(c).auto_build {
        let p = choose_production(g, c);
        g.city_mut(c).production = p;
    }
    let city = g.city(c).clone();
    let gold = g.player(slot).gold;
    let missing = g.production_cost(c, city.production) - city.shields;
    let mut amount = 0;
    if let Production::Unit(u) = city.production {
        if net_shields != 0 {
            let census = g.census();
            let k = crate::game::production::continent_index(g.map.tile(city.x, city.y).continent);
            let policy = g.continent_policy(&census, slot, k);
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
    if (g.units_at(city.x, city.y).is_empty() || city.auto_build) && unit && city.shields != 0 {
        amount = missing.max(0).min(gold / 3);
    }
    if city.production == Production::Improvement(imp::PALACE) && city.shields != 0 {
        amount = (127 * 10 - city.shields).max(0).min(gold / 3);
    }
    if unit && exe_status(g, c) == 0x19 && !g.all_units().any(|u| u.owner == slot && u.kind == unit_ids::NUCLEAR) && city.shields != 0 {
        amount = missing.max(0).min(gold / 4);
    }
    if gold > 2000 {
        amount += gold / 512;
    }
    let cm = g.city_mut(c);
    cm.shields += amount;
    cm.auto_build = false;
    g.player_mut(slot).gold -= 2 * amount;
}

/// CIV.EXE's city status byte (+6) as far as civrs keeps its bits: 0x1 disorder, 0x2 coastal,
/// 0x4 garrison out, 0x8 Hydro Plant site, 0x10 AUTO / alert, 0x20 advance stolen, 0x40
/// celebration, 0x80 improvement sold this turn. CIV.EXE sets the coastal and hydro bits when
/// the city is founded (o 0x1b6b9, 0x1b6d2); civrs derives them from the map (unverified that
/// the tests are the same).
fn exe_status(g: &Game, c: CityId) -> u8 {
    let city = g.city(c);
    let hydro = Map::neighbours8(city.x, city.y).chain(std::iter::once((city.x, city.y))).any(|(x, y)| matches!(g.map.terrain(x, y), Terrain::River | Terrain::Mountains));
    let bits = [
        city.disorder,
        g.map.is_coastal(city.x, city.y),
        city.garrison_out,
        hydro,
        city.auto_build,
        city.tech_stolen,
        city.celebrating,
        g.player(city.owner).sold_cities.contains(&c),
    ];
    bits.iter().enumerate().fold(0, |s, (i, b)| s | ((*b as u8) << i))
}

/// Tax 50 / lux 0 / sci 50 within the government's limit; luxuries rise while cities riot.
pub fn set_rates(g: &mut Game, slot: Slot) {
    let p = g.player(slot);
    if p.govt == Govt::Anarchy {
        let _ = g.set_rates(slot, 0, 100);
        return;
    }
    let disorder = g.all_cities().any(|c| c.owner == slot && c.disorder);
    let cur_lux = p.lux as i32;
    let mut lux = if disorder { (cur_lux + 10).min(30) } else { (cur_lux - 10).max(0) };
    let broke = p.gold < 30 && g.income(slot) < 0;
    let mut tax = if broke { 70 } else { 50 };
    let max = g.max_rate(slot) as i32;
    lux = lux.min(max);
    // leaving Anarchy (0/100/0) the luxuries are still high: tax + lux must not exceed 100 or
    // the request is refused and the rates never come down
    tax = tax.min(max).min(100 - lux);
    // science is the remainder and must respect the cap too
    let mut sci = 100 - tax - lux;
    while sci > max {
        if tax < max {
            tax += 10;
        } else {
            lux += 10;
        }
        sci = 100 - tax - lux;
    }
    let _ = g.set_rates(slot, tax as u8, lux as u8);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;

    #[test]
    fn rates_recover_after_anarchy() {
        let mut g = Game::new(GameSetup { seed: 5, ..Default::default() }, None);
        let slot = 2;
        g.player_mut(slot).govt = Govt::Anarchy;
        set_rates(&mut g, slot);
        assert_eq!((g.player(slot).tax, g.player(slot).lux), (0, 100));
        // what clamp_rates leaves when Anarchy gives way to Monarchy (max 70)
        g.player_mut(slot).govt = Govt::Monarchy;
        g.clamp_rates(slot);
        assert_eq!((g.player(slot).tax, g.player(slot).lux), (30, 70));
        for _ in 0..10 {
            set_rates(&mut g, slot);
        }
        assert_eq!((g.player(slot).tax, g.player(slot).lux), (50, 0));
    }

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
        let advice = g.ai_production(c);
        city_turn_purchase(&mut g, c, 0);
        assert_eq!(g.city(c).production, advice);
        assert!(!g.city(c).auto_build);
        if matches!(advice, Production::Unit(_)) {
            assert_eq!(g.city(c).shields, g.production_cost(c, advice));
        }
    }

    #[test]
    fn attacks_put_computer_cities_on_alert_but_not_the_humans() {
        let (mut g, c) = empty_ai_city();
        let slot = g.city(c).owner;
        let (x, y) = g.city(c).pos();
        let h = g.human_slot;
        g.set_relation(h, slot, crate::game::Relation::War);
        g.add_unit(slot, unit_ids::PHALANX, x, y);
        let a = g.add_unit(h, unit_ids::LEGION, x + 1, y);
        g.attack(a, x, y);
        assert!(g.city(c).auto_build, "set win or lose");
        // the human's own city keeps its AUTO setting
        let hc = g.add_city(h, "Mine".into(), x + 6, y);
        g.add_unit(h, unit_ids::PHALANX, x + 6, y);
        let b = g.add_unit(slot, unit_ids::LEGION, x + 5, y);
        g.attack(b, x + 6, y);
        assert!(!g.city(hc).auto_build);
    }
}
