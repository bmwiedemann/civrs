//! Computer players: city management (`city_ai`), unit behaviour (`unit_ai`), everything
//! across water (`naval_ai`), diplomacy judgement (`diplo_ai`), research choice (`tech_ai`)
//! and pathfinding (`path`).
//!
//! All randomness comes from `Game::rng` so AI-only runs are reproducible from the seed.

pub mod city_ai;
pub mod diplo_ai;
pub mod naval_ai;
pub mod path;
pub mod tech_ai;
pub mod unit_ai;

use crate::game::turn::TurnStep;
use crate::game::{Decision, Game, Slot};
use crate::rules::Govt;

pub trait AiDriver {
    /// Play the whole turn for `slot`: city management, unit moves, diplomacy.
    fn play_turn(&mut self, g: &mut Game, slot: Slot, sink: &mut dyn FnMut(TurnStep));
}

pub struct Ai {
    pub seed: u64,
    /// Standing orders of transport ships (see `naval_ai`).
    pub missions: naval_ai::Missions,
}

impl Ai {
    pub fn new(seed: u64) -> Ai {
        Ai { seed, missions: naval_ai::Missions::new() }
    }
}

impl AiDriver for Ai {
    fn play_turn(&mut self, g: &mut Game, slot: Slot, sink: &mut dyn FnMut(TurnStep)) {
        if slot == 0 {
            g.expire_barbarians();
            unit_ai::play_barbarians(g, sink);
            return;
        }
        let p = g.player(slot);
        if !p.in_game || !p.alive {
            return;
        }
        // first contacts made by anyone this turn: the AI side opens an audience with the human
        let human = g.human_slot;
        for (a, b) in g.update_contacts() {
            let ai_side = if a == human { Some(b) } else if b == human { Some(a) } else { None };
            if let Some(ai_side) = ai_side {
                if g.player(human).alive && !g.player(ai_side).human {
                    if let Some(st) = diplo_ai::on_contact(g, ai_side, human) {
                        sink(TurnStep::Decision(Decision::Diplomacy(st)));
                    }
                }
            }
        }
        govern(g, slot);
        city_ai::set_rates(g, slot);
        let plan = naval_ai::plan(g, slot);
        for c in g.cities_of(slot) {
            if g.city_opt(c).is_some() {
                city_ai::manage(g, c, &plan);
            }
        }
        // ships first: units they land may still act this turn
        naval_ai::play_transports(g, slot, &plan, &mut self.missions, sink);
        unit_ai::play_units(g, slot, sink);
        if g.player(human).alive {
            if let Some(st) = diplo_ai::turn(g, slot) {
                sink(TurnStep::Decision(Decision::Diplomacy(st)));
            }
        }
    }
}

/// Government the civilisation would like to have given what it knows and its traits.
pub fn desired_government(g: &Game, slot: Slot) -> Govt {
    let p = g.player(slot);
    let civ = p.civ();
    let avail = g.governments_available(slot);
    let at_war = !g.enemies_of(slot).is_empty();
    let has = |gv: Govt| avail.contains(&gv);
    if civ.aggressive > 0 && has(Govt::Communism) {
        return Govt::Communism;
    }
    if civ.civilized > 0 && !at_war {
        if has(Govt::Democracy) && civ.aggressive < 0 {
            return Govt::Democracy;
        }
        if has(Govt::Republic) {
            return Govt::Republic;
        }
    }
    if has(Govt::Monarchy) {
        return Govt::Monarchy;
    }
    if has(Govt::Communism) {
        return Govt::Communism;
    }
    if has(Govt::Republic) && !at_war {
        return Govt::Republic;
    }
    Govt::Despotism
}

/// Rank used to decide whether a revolution is worth the anarchy.
fn govt_rank(gv: Govt) -> i32 {
    match gv {
        Govt::Anarchy => 0,
        Govt::Despotism => 1,
        Govt::Monarchy => 2,
        Govt::Communism => 3,
        Govt::Republic => 3,
        Govt::Democracy => 4,
    }
}

/// Government management for an AI: run through anarchy, pick a government, start a
/// revolution when a clearly better government is known. Uses `Game::revolution` /
/// `choose_government` semantics locally (those engine entry points are not available yet).
pub fn govern(g: &mut Game, slot: Slot) {
    let p = g.player(slot).clone();
    if p.govt == Govt::Anarchy {
        // the engine's start-of-turn processing counts down anarchy and picks the government
        return;
    }
    if g.turn < 10 {
        return;
    }
    let want = desired_government(g, slot);
    // only move "up" (Despotism -> anything, Monarchy -> Republic/Democracy...) so the AI does
    // not flip-flop through anarchy every few turns
    if govt_rank(want) <= govt_rank(p.govt) {
        return;
    }
    // no revolution while a city is in disorder or an enemy is at the gates
    if g.all_cities().any(|c| c.owner == slot && c.disorder) {
        return;
    }
    g.revolution(slot);
    if g.player(slot).govt != Govt::Anarchy {
        // Pyramids: immediate change
        g.choose_government(slot, want);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{DecisionReply, GameSetup};

    fn soak(seed: u64, turns: usize) -> Game {
        soak_setup(GameSetup { seed, num_civs: 7, ..Default::default() }, turns, &mut |_| {})
    }

    /// AI-only game from `setup`; `each_turn` is called with the state after every turn.
    fn soak_setup(setup: GameSetup, turns: usize, each_turn: &mut dyn FnMut(&Game)) -> Game {
        let seed = setup.seed;
        let mut g = Game::new(setup, None);
        let mut ai = Ai::new(seed);
        for _ in 0..turns {
            let mut r = g.end_turn();
            let mut guard = 0;
            loop {
                match r.step(&mut g, &mut ai) {
                    TurnStep::Done => break,
                    TurnStep::Decision(_) => r.resolve(&mut g, DecisionReply::Choice(0)),
                    _ => {}
                }
                guard += 1;
                assert!(guard < 100_000, "turn runner does not terminate");
            }
            each_turn(&g);
        }
        g
    }

    /// Landmass each civilisation started on (from its first Settlers).
    fn start_continents(g: &Game) -> [u8; 8] {
        let mut c = [0u8; 8];
        for u in g.all_units() {
            if c[u.owner as usize] == 0 {
                c[u.owner as usize] = g.map.tile(u.x, u.y).continent;
            }
        }
        c
    }

    /// Cities of AI civilisations on a landmass other than the one their civilisation started on.
    fn overseas_cities(g: &Game, start: &[u8; 8]) -> usize {
        g.all_cities().filter(|c| c.owner != 0 && c.owner != g.human_slot && g.map.tile(c.x, c.y).continent != start[c.owner as usize]).count()
    }

    fn dump(g: &Game) {
        eprintln!(
            "soak: turn {} year {} cities {} units {} barbarians {} ended {:?}",
            g.turn,
            g.year_text(),
            g.all_cities().count(),
            g.all_units().count(),
            g.units_of(0).len(),
            g.ended
        );
        for s in g.civ_slots() {
            let p = g.player(s);
            eprintln!(
                "  slot {s} {:<12} alive {} cities {} units {} advances {} govt {:?} gold {} wars {:?}",
                p.plural(),
                p.alive,
                g.cities_of(s).len(),
                g.units_of(s).len(),
                p.advance_count(),
                p.govt,
                p.gold,
                g.enemies_of(s)
            );
            for c in g.cities_of(s) {
                let city = g.city(c);
                let site = unit_ai::best_city_site(g, s, city.pos(), 8);
                let cont = g.map.tile(city.x, city.y).continent;
                let land = g.map.tiles.iter().filter(|t| t.continent == cont).count();
                let y = g.city_yield(c);
                eprintln!("     {} size {} prod {:?} shields {} food {}/{} surplus {} site {:?} continent {} ({} tiles) defenders {}", city.name, city.size, city.production, city.shields, city.food, city.granary_size(), y.food_surplus(), site, cont, land, unit_ai::defenders_in(g, c));
            }
            eprintln!("     relations {:?} embassies {:#x}", p.relations, p.embassies);
            eprintln!("     overseas plan {:?} transport kind {:?}", naval_ai::plan(g, s), naval_ai::transport_kind(g, s));
        }
    }

    #[test]
    fn ai_only_game_runs_150_turns_and_expands() {
        let g = soak(42, 150);
        dump(&g);
        let human = g.human_slot;
        let ai_cities = g.all_cities().filter(|c| c.owner != human && c.owner != 0).count();
        assert!(ai_cities >= 6, "AI founded {ai_cities} cities");
        let ai_units = g.all_units().filter(|u| u.owner != human).count();
        assert!(ai_units > 0);
        // somebody learned something
        assert!(g.civ_slots().iter().any(|s| *s != human && g.player(*s).advance_count() > 0));
    }

    /// Long soak with statistics: `cargo test --lib ai::tests::long_soak -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn long_soak() {
        let g = soak(1234, 300);
        dump(&g);
        assert!(g.all_cities().count() >= 6);
    }

    /// Small land mass = many islands: the AI must cross water to keep expanding.
    fn archipelago(seed: u64, turns: usize) -> (Game, Option<u32>) {
        use crate::game::{WorldParams, WorldSource};
        let setup = GameSetup { seed, num_civs: 7, world: WorldSource::Random(WorldParams { land_mass: 0, ..WorldParams::normal() }), ..Default::default() };
        let start = start_continents(&Game::new(setup.clone(), None));
        let mut first: Option<u32> = None;
        let g = soak_setup(setup, turns, &mut |g| {
            if first.is_none() && overseas_cities(g, &start) > 0 {
                first = Some(g.turn);
            }
        });
        (g, first)
    }

    /// Debug trace of transport ships: `cargo test --lib ai::tests::trace_transports -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn trace_transports() {
        use crate::game::{WorldParams, WorldSource};
        let seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(11u64);
        let setup = GameSetup { seed, num_civs: 7, world: WorldSource::Random(WorldParams { land_mass: 0, ..WorldParams::normal() }), ..Default::default() };
        let mut known: std::collections::BTreeMap<u16, String> = Default::default();
        let mut last_plan: [bool; 8] = [false; 8];
        soak_setup(setup, 250, &mut |g| {
            for s in g.civ_slots() {
                let pl = naval_ai::plan(g, s);
                if pl.wants_transport != last_plan[s as usize] {
                    eprintln!("turn {} slot {s}: wants_transport -> {} ({:?}) cities {} coastal prods {:?}", g.turn, pl.wants_transport, pl, g.cities_of(s).len(),
                        g.cities_of(s).iter().filter(|c| g.map.is_coastal(g.city(**c).x, g.city(**c).y)).map(|c| (g.city(*c).name.clone(), g.city(*c).size, g.city(*c).production)).collect::<Vec<_>>());
                    last_plan[s as usize] = pl.wants_transport;
                }
                if pl.wants_transport && g.turn % 5 == 0 {
                    for c in g.cities_of(s) {
                        let city = g.city(c);
                        if g.map.is_coastal(city.x, city.y) {
                            let avail = g.available_production(c);
                            let ships: Vec<u8> = avail.iter().filter_map(|p| match p { crate::game::Production::Unit(k) if naval_ai::is_transport_kind(*k) => Some(*k), _ => None }).collect();
                            eprintln!("turn {} slot {s} {} size {} prod {:?} shields {} -> choose {:?} ships avail {:?} defenders {}/{}", g.turn, city.name, city.size, city.production, city.shields, city_ai::choose_production(g, c, &pl), ships, unit_ai::defenders_in(g, c), unit_ai::defenders_needed(g, c));
                        }
                    }
                }
            }
            let watch: u8 = std::env::var("WATCH").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
            if watch != 0 {
                for u in g.all_units().filter(|u| u.owner == watch && naval_ai::is_transport(u)) {
                    eprintln!("turn {} watch transport {} at {:?} order {:?} moves {} cargo {:?} home {:?}", g.turn, u.id, u.pos(), u.order, u.moves_left, g.cargo_of(u.id).iter().map(|c| g.unit(*c).info().name).collect::<Vec<_>>(), u.home.and_then(|h| g.city_opt(h)).map(|c| c.name.clone()));
                }
                for u in g.all_units().filter(|u| u.owner == watch && u.kind == crate::rules::unit_ids::SETTLERS) {
                    eprintln!("turn {} watch settlers {} at {:?} order {:?} turns {} goto {:?} carried {:?} in city {:?}", g.turn, u.id, u.pos(), u.order, u.order_turns, u.goto, u.carried_by, g.city_at(u.x, u.y).map(|c| g.city(c).name.clone()));
                }
            }
            let mut now: std::collections::BTreeMap<u16, String> = Default::default();
            for u in g.all_units().filter(|u| u.owner != 0 && naval_ai::is_transport(u)) {
                let desc = format!("slot {} {} at {:?} order {:?} cargo {:?} home {:?}", u.owner, u.info().name, u.pos(), u.order, g.cargo_of(u.id).iter().map(|c| g.unit(*c).info().name).collect::<Vec<_>>(), u.home.map(|h| g.city_opt(h).map(|c| c.name.clone())));
                if g.cargo_of(u.id).iter().any(|c| g.unit(*c).info().role == crate::rules::AiRole::Attack) {
                    eprintln!("turn {}: TROOPS aboard {} {desc} plan {:?}", g.turn, u.id, naval_ai::plan(g, u.owner));
                }
                if !known.contains_key(&u.id) {
                    eprintln!("turn {}: NEW transport {} {desc}", g.turn, u.id);
                }
                now.insert(u.id, desc);
            }
            for (id, desc) in &known {
                if !now.contains_key(id) {
                    eprintln!("turn {}: LOST transport {id}, was {desc}", g.turn);
                }
            }
            for c in g.all_cities() {
                if c.founded_turn + 1 >= g.turn && c.owner != 0 {
                    let overseas = !g.all_cities().any(|o| o.owner == c.owner && o.id != c.id && g.map.tile(o.x, o.y).continent == g.map.tile(c.x, c.y).continent);
                    if overseas { eprintln!("turn {}: OVERSEAS city {} slot {} at {:?} continent {}", g.turn, c.name, c.owner, c.pos(), g.map.tile(c.x, c.y).continent); }
                }
            }
            known = now;
        });
    }

    #[test]
    fn archipelago_ai_settles_other_continents() {
        let mut total = 0;
        for seed in [11u64, 23, 41] {
            let (g, first) = archipelago(seed, 250);
            let start = start_continents(&Game::new(GameSetup { seed, num_civs: 7, world: g.setup.world, ..Default::default() }, None));
            let overseas = overseas_cities(&g, &start);
            let ships = g.all_units().filter(|u| u.owner != 0 && naval_ai::is_transport(u)).count();
            eprintln!("archipelago seed {seed}: turn {} cities {} overseas {} transports {} first overseas city at turn {:?}", g.turn, g.all_cities().count(), overseas, ships, first);
            dump(&g);
            total += overseas;
            assert!(overseas > 0, "seed {seed}: no AI city on a continent other than its starting one");
        }
        assert!(total > 0);
    }

    #[test]
    fn ai_only_game_is_deterministic() {
        let a = soak(9, 40);
        let b = soak(9, 40);
        assert_eq!(a.turn, b.turn);
        assert_eq!(a.rng, b.rng);
        assert_eq!(a.all_cities().count(), b.all_cities().count());
    }
}
