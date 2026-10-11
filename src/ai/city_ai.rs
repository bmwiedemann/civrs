//! City management for computer players: production choice, disorder control, purchases and
//! tax rates.

use super::naval_ai::{self, Plan};
use super::unit_ai::{best_city_site, city_target, defenders_in, defenders_needed, enemy_near, garrison_in, SITE_RADIUS};
use crate::game::{CityId, Game, Map, Production, Slot};
use crate::rules::{self, improvement_ids as imp, unit_ids, AiRole, Domain, Govt};

/// Best defensive land unit the city can build (highest defence, then cheapest).
fn best_defender(avail: &[Production]) -> Option<Production> {
    avail
        .iter()
        .filter_map(|p| match p {
            Production::Unit(k) => {
                let u = &rules::UNITS[*k as usize];
                if u.domain == Domain::Land && u.def > 0 && *k != unit_ids::SETTLERS && *k != unit_ids::CARAVAN && *k != unit_ids::DIPLOMAT {
                    Some((u.def as i32 * 10 - u.cost as i32, *p))
                } else {
                    None
                }
            }
            _ => None,
        })
        .max_by_key(|(s, _)| *s)
        .map(|(_, p)| p)
}

/// Best offensive land unit the city can build (attack and mobility per cost).
fn best_attacker(avail: &[Production]) -> Option<Production> {
    avail
        .iter()
        .filter_map(|p| match p {
            Production::Unit(k) => {
                let u = &rules::UNITS[*k as usize];
                if u.domain == Domain::Land && u.role == AiRole::Attack && u.att > 0 {
                    Some(((u.att as i32 * 2 + u.moves as i32 + u.def as i32) * 10 - u.cost as i32, *p))
                } else {
                    None
                }
            }
            _ => None,
        })
        .max_by_key(|(s, _)| *s)
        .map(|(_, p)| p)
}

/// Best transport ship the city can build (largest capacity, then cheapest).
fn best_transport(avail: &[Production]) -> Option<Production> {
    avail
        .iter()
        .filter_map(|p| match p {
            Production::Unit(k) if naval_ai::is_transport_kind(*k) => {
                let u = &rules::UNITS[*k as usize];
                Some((u.capacity as i32 * 10 - u.cost as i32, *p))
            }
            _ => None,
        })
        .max_by_key(|(s, _)| *s)
        .map(|(_, p)| p)
}

/// The coastal city that should build the civilisation's ships: the one with the largest
/// shield surplus (a port with no surplus never finishes a hull).
pub fn shipyard(g: &Game, slot: Slot) -> Option<CityId> {
    g.all_cities()
        .filter(|c| c.owner == slot && c.size >= 2 && g.map.is_coastal(c.x, c.y))
        .max_by_key(|c| (g.city_yield(c.id).shield_surplus(), c.size, std::cmp::Reverse(c.id)))
        .map(|c| c.id)
}

fn has_imp(avail: &[Production], i: u8) -> bool {
    avail.contains(&Production::Improvement(i))
}

fn has_unit(avail: &[Production], k: u8) -> bool {
    avail.contains(&Production::Unit(k))
}

/// The civilisation's most productive city (shields), where wonders are built.
pub fn best_city(g: &Game, slot: Slot) -> Option<CityId> {
    g.all_cities().filter(|c| c.owner == slot).max_by_key(|c| g.city_yield(c.id).shield_surplus() * 4 + c.size as i32).map(|c| c.id)
}

/// Pick what the city should build next, following the plan's priority list. `plan` is the
/// civilisation's overseas plan (transport ships, colonists and invasion troops).
pub fn choose_production(g: &Game, c: CityId, plan: &Plan) -> Production {
    let city = g.city(c);
    let slot = city.owner;
    let p = g.player(slot);
    let civ = p.civ();
    let avail = g.available_production(c);
    let fallback = || g.default_production(c);
    let defender = best_defender(&avail);
    let size = city.size as usize;

    // 1. a minimum garrison
    let have = defenders_in(g, c);
    let need = defenders_needed(g, c);
    if have < need {
        if let Some(d) = defender {
            return d;
        }
    }
    let enemies = g.enemies_of(slot);
    let barbarians_about = g.all_units().any(|u| u.owner == 0 && Map::dist(u.pos(), city.pos()) <= 8);
    let at_war = !enemies.is_empty() || barbarians_about;
    let threatened = enemy_near(g, slot, city.pos(), 6);

    // 2. walls when the enemy is near
    if threatened && !city.has_improvement(imp::CITY_WALLS) && has_imp(&avail, imp::CITY_WALLS) && size >= 3 {
        return Production::Improvement(imp::CITY_WALLS);
    }

    // a city that cannot feed another Settlers loses it to famine right away
    let y = g.city_yield(c);
    let settler_food = rules::GOVERNMENTS[p.govt as usize].settler_food as i32;
    let can_feed_settlers = y.food_surplus() > settler_food || city.food >= city.granary_size() / 2;

    // 3. overseas: a transport ship when the plan wants one and none exists (or is being
    //    built elsewhere), then exactly the cargo the ship waiting in this port still lacks
    let n_cities = g.cities_of(slot).len();
    let coastal = g.map.is_coastal(city.x, city.y);
    if plan.wants_transport && coastal {
        let transports = g.all_units().filter(|u| u.owner == slot && naval_ai::is_transport(u)).count();
        let wanted = if plan.invasion.is_some() { 2 } else { 1 };
        let building = g.all_cities().any(|cc| cc.owner == slot && cc.id != c && matches!(cc.production, Production::Unit(k) if naval_ai::is_transport_kind(k)));
        if transports < wanted && !building && shipyard(g, slot) == Some(c) {
            if let Some(t) = best_transport(&avail) {
                return t;
            }
        }
        if let Some((settlers, escort, attackers)) = naval_ai::port_needs(g, c, plan) {
            let here = |g: &Game, f: &dyn Fn(&crate::game::Unit) -> bool| g.all_units().filter(|u| u.owner == slot && u.pos() == city.pos() && u.carried_by.is_none() && f(u)).count();
            let cont = g.map.tile(city.x, city.y).continent;
            let free_settlers = g.all_units().filter(|u| u.owner == slot && u.kind == unit_ids::SETTLERS && u.carried_by.is_none() && g.map.tile(u.x, u.y).continent == cont).count();
            if settlers > 0 && free_settlers == 0 && size >= 3 && can_feed_settlers && has_unit(&avail, unit_ids::SETTLERS) {
                return Production::Unit(unit_ids::SETTLERS);
            }
            if escort && here(g, &|u| u.info().role == AiRole::Defend && u.is_military()) <= need {
                if let Some(d) = defender {
                    return d;
                }
            }
            if attackers > 0 && here(g, &|u| u.is_land() && u.info().role == AiRole::Attack) == 0 && size >= 2 {
                if let Some(a) = best_attacker(&avail) {
                    return a;
                }
            }
        }
    }

    // 4. expansion
    let settlers_out = g.all_units().filter(|u| u.owner == slot && u.kind == unit_ids::SETTLERS).count();
    let site_near = || best_city_site(g, slot, city.pos(), SITE_RADIUS).is_some();
    if size >= 3 && n_cities < city_target(g, slot) && settlers_out < n_cities / 3 + 2 && can_feed_settlers && has_unit(&avail, unit_ids::SETTLERS) && site_near() {
        return Production::Unit(unit_ids::SETTLERS);
    }

    // 5. happiness
    let h = g.city_happiness(c);
    if h.unhappy >= h.content || city.disorder {
        if !city.has_improvement(imp::TEMPLE) && has_imp(&avail, imp::TEMPLE) {
            return Production::Improvement(imp::TEMPLE);
        }
        if !city.has_improvement(imp::MARKETPLACE) && has_imp(&avail, imp::MARKETPLACE) && size >= 5 {
            return Production::Improvement(imp::MARKETPLACE);
        }
        if !city.has_improvement(imp::COLOSSEUM) && has_imp(&avail, imp::COLOSSEUM) && size >= 6 {
            return Production::Improvement(imp::COLOSSEUM);
        }
        if !city.has_improvement(imp::CATHEDRAL) && has_imp(&avail, imp::CATHEDRAL) && size >= 8 {
            return Production::Improvement(imp::CATHEDRAL);
        }
    }

    // 6. an army when at war (attackers in groups: up to one per city, two for aggressive civs)
    if at_war {
        let attackers = g.all_units().filter(|u| u.owner == slot && u.is_land() && u.info().role == AiRole::Attack).count();
        let wanted = n_cities * if civ.aggressive > 0 { 2 } else { 1 } + 1;
        if attackers < wanted && size >= 2 {
            if let Some(a) = best_attacker(&avail) {
                return a;
            }
        }
        if !city.has_improvement(imp::BARRACKS) && has_imp(&avail, imp::BARRACKS) && civ.aggressive > 0 && size >= 5 && peek(g, 10) < 3 {
            return Production::Improvement(imp::BARRACKS);
        }
    }

    // 7. infrastructure by city state
    let order: Vec<(u8, bool)> = vec![
        (imp::TEMPLE, size >= 3),
        (imp::LIBRARY, size >= 3),
        (imp::MARKETPLACE, size >= 4),
        (imp::GRANARY, size >= 5),
        (imp::AQUEDUCT, size >= 8),
        (imp::COURTHOUSE, size >= 4 && y.corruption * 4 > y.trade.max(1)),
        (imp::UNIVERSITY, size >= 6 && city.has_improvement(imp::LIBRARY)),
        (imp::BANK, size >= 7 && city.has_improvement(imp::MARKETPLACE)),
        (imp::COLOSSEUM, size >= 8),
        (imp::FACTORY, size >= 8),
        (imp::CITY_WALLS, size >= 6 && civ.aggressive <= 0 && (at_war || g.turn > 200)),
        (imp::POWER_PLANT, city.has_improvement(imp::FACTORY) && !city.has_improvement(imp::HYDRO_PLANT) && !city.has_improvement(imp::NUCLEAR_PLANT)),
        (imp::MASS_TRANSIT, size >= 10 && city.has_improvement(imp::FACTORY)),
        (imp::SDI_DEFENSE, g.nukes_enabled && size >= 8),
        (imp::MFG_PLANT, size >= 12 && city.has_improvement(imp::FACTORY)),
        (imp::RECYCLING_CNTR, size >= 10 && city.has_improvement(imp::FACTORY)),
    ];
    for (i, cond) in order {
        if cond && !city.has_improvement(i) && has_imp(&avail, i) {
            return Production::Improvement(i);
        }
    }

    // 8. spaceship parts in the biggest cities once Apollo is built
    if g.apollo_built && !p.spaceship.launched() && size >= 8 {
        for part in [imp::SS_STRUCTURAL, imp::SS_COMPONENT, imp::SS_MODULE] {
            if has_imp(&avail, part) {
                return Production::Improvement(part);
            }
        }
    }

    // 9. wonders in the best city; caravans elsewhere to help
    let best = best_city(g, slot);
    let wonder_in_progress = g.all_cities().any(|cc| cc.owner == slot && matches!(cc.production, Production::Wonder(_)));
    if best == Some(c) && size >= 4 {
        let cheapest = avail
            .iter()
            .filter_map(|pr| match pr {
                Production::Wonder(w) => Some((rules::WONDERS[*w as usize].cost, *pr)),
                _ => None,
            })
            .min_by_key(|(cost, _)| *cost)
            .map(|(_, pr)| pr);
        if let Some(w) = cheapest {
            return w;
        }
    } else if wonder_in_progress && size >= 5 && has_unit(&avail, unit_ids::CARAVAN) {
        let caravans = g.all_units().filter(|u| u.owner == slot && u.kind == unit_ids::CARAVAN).count();
        if caravans < 3 {
            return Production::Unit(unit_ids::CARAVAN);
        }
    }

    // 10. extra garrison / explorers / diplomats, else the engine default
    if have < need + 1 && size >= 5 && garrison_in(g, c) < need + 2 {
        if let Some(d) = defender {
            return d;
        }
    }
    let diplomats = g.all_units().filter(|u| u.owner == slot && u.kind == unit_ids::DIPLOMAT).count();
    if diplomats == 0 && has_unit(&avail, unit_ids::DIPLOMAT) && size >= 4 && g.contacts_of(slot).iter().any(|o| !p.has_embassy(*o)) {
        return Production::Unit(unit_ids::DIPLOMAT);
    }
    if size >= 5 && has_unit(&avail, unit_ids::SETTLERS) && settlers_out < 2 && site_near() {
        return Production::Unit(unit_ids::SETTLERS);
    }
    // a crowded garrison means there is nothing useful left: grow the city instead of the army
    let crowded = garrison_in(g, c) >= need + 3;
    if !crowded && (civ.aggressive > 0 || at_war) {
        if let Some(a) = best_attacker(&avail) {
            return a;
        }
    }
    if crowded {
        if size >= 3 && has_unit(&avail, unit_ids::SETTLERS) {
            return Production::Unit(unit_ids::SETTLERS);
        }
        if has_unit(&avail, unit_ids::CARAVAN) {
            return Production::Unit(unit_ids::CARAVAN);
        }
    }
    fallback()
}

/// Swap workers from the hungriest-poor tiles onto food while the city is starving.
fn feed(g: &mut Game, c: CityId) {
    let slot = g.city(c).owner;
    for _ in 0..8 {
        if g.city_yield(c).food_surplus() >= 0 {
            return;
        }
        let city = g.city(c);
        let food_at = |g: &Game, idx: u8| g.radius_tile(c, idx).map(|(x, y)| g.tile_yield(slot, x, y, Some(c)).0).unwrap_or(0);
        let worst = (1..21u8).filter(|i| city.works(*i)).min_by_key(|i| food_at(g, *i));
        let best = (1..21u8).filter(|i| !city.works(*i) && g.tile_workable(c, *i)).max_by_key(|i| food_at(g, *i) * 4 + g.radius_tile(c, *i).map(|(x, y)| g.tile_yield(slot, x, y, Some(c)).1).unwrap_or(0));
        match (worst, best) {
            (Some(w), Some(b)) if food_at(g, b) > food_at(g, w) => {
                g.toggle_tile(c, w);
                g.toggle_tile(c, b);
            }
            (None, Some(b)) if !g.city(c).specialists.is_empty() && !g.city(c).disorder => {
                g.toggle_tile(c, b);
            }
            _ => return,
        }
    }
}

/// Per-turn management of one AI city.
pub fn manage(g: &mut Game, c: CityId, plan: &Plan) {
    if g.city_opt(c).is_none() {
        return;
    }
    let slot = g.city(c).owner;
    feed(g, c);
    // disorder: turn the least valuable workers into entertainers
    for _ in 0..4 {
        let h = g.city_happiness(c);
        if !h.disorder {
            break;
        }
        let city = g.city(c);
        let worked: Vec<u8> = (1..21u8).filter(|i| city.works(*i)).collect();
        if worked.is_empty() {
            break;
        }
        let worst = worked
            .iter()
            .copied()
            .min_by_key(|idx| {
                let (x, y) = g.radius_tile(c, *idx).unwrap_or((0, 0));
                let (f, s, t) = g.tile_yield(slot, x, y, Some(c));
                f * 4 + s * 3 + t * 2
            })
            .unwrap();
        g.toggle_tile(c, worst);
    }
    // production
    let city = g.city(c).clone();
    let avail = g.available_production(c);
    let current_ok = avail.contains(&city.production) && !matches!(city.production, Production::Wonder(w) if g.wonders_built[w as usize].is_some());
    let fresh = city.shields <= 0 || city.production == Production::Unit(unit_ids::MILITIA);
    let desired = choose_production(g, c, plan);
    if desired != city.production && (!current_ok || fresh || desired.class() == city.production.class() || g.turn % 10 == 0) {
        if g.set_production(c, desired).is_err() && !current_ok {
            let d = g.default_production(c);
            let _ = g.set_production(c, d);
        }
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

/// Deterministic pseudo-random value in `0..n` that does not advance the game RNG (used by
/// read-only heuristics).
fn peek(g: &Game, n: u32) -> u32 {
    let mut r = g.rng.clone();
    r.below(n)
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
