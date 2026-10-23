//! City management for computer players: production choice, purchases and tax rates. Workers
//! and entertainers are the city turn's business (`Game::assign_workers`), as in CIV.EXE.

use super::naval_ai::{self, Plan};
use super::unit_ai::enemy_near;
use crate::game::{CityId, Game, Production, Slot};
use crate::rules::{self, improvement_ids as imp, AiRole, Govt};

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
pub fn manage(g: &mut Game, c: CityId, plan: &Plan) {
    if g.city_opt(c).is_none() {
        return;
    }
    let slot = g.city(c).owner;
    // the advisor's item can no longer be built (a wonder finished elsewhere, an improvement
    // the city got another way): CIV.EXE cancels it on completion and asks again (0xe4f1)
    if g.production_invalid(c) {
        let p = choose_production(g, c);
        g.city_mut(c).production = p;
    }
    // purchases: defenders when threatened, otherwise when rolling in gold
    let gold = g.player(slot).gold;
    let cost = g.buy_cost(c);
    if cost > 0 && !g.city(c).bought && !g.city(c).disorder {
        let prod = g.city(c).production;
        let is_defender = matches!(prod, Production::Unit(k) if rules::UNITS[k as usize].role == AiRole::Defend);
        let threatened = enemy_near(g, slot, g.city(c).pos(), 3);
        let rich = gold > 300 + cost * 5 && !matches!(prod, Production::Wonder(_));
        // the expedition's ship is worth paying for
        let ship_wanted = plan.wants_transport && matches!(prod, Production::Unit(k) if naval_ai::is_transport_kind(k)) && gold >= cost + 50;
        if (threatened && is_defender && gold >= cost * 3) || (threatened && matches!(prod, Production::Improvement(i) if i == imp::CITY_WALLS) && gold >= cost * 2) || rich || ship_wanted {
            let _ = g.buy(c);
        }
    }
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
}
