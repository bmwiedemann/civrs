//! Scripted human actions for the differential test against CIV.EXE
//! (reference/tools/sve_diff.py, `civrs --load-sve F --actions SCRIPT --dump state:N`).
//!
//! The same script drives CIV.EXE in dosbox (the harness turns each action into its keys and
//! clicks) and civrs (`apply` performs it through the game API as the map view and the city
//! screen would), so both play the same human turns. Lines, `#` starts a comment:
//!
//! ```text
//! turn 36                  the actions below run at the start of the human's turn 36
//! unit 10,25 r             keys for the active unit, which must stand at 10,25: k1..k9
//!                          keypad moves, r i m f (Settlers: fortress) s p P(pillage) b(found,
//!                          default name) w space h u D(disband); ret is a Return for a
//!                          message box in CIV.EXE (nothing in civrs)
//! unit 12,28 k2:2          a move that opens a menu (a caravan at one of the human's cities:
//!                          0 keep moving, 1 trade route, 2 help build the wonder; a treaty
//!                          partner's unit or land: 0 cancel, 1 break the treaty) and its row;
//!                          a caravan entering a foreign city sets up its route, a move onto a
//!                          foreign unit attacks
//! select 7,23 Caesarea 9,25   FindCity, a click on the square and Return: its one unit
//!                          of the human's becomes the active unit
//! tax +1 / lux -2          the rate menus ('+' / '-'): rows down from the current rate
//! revolution               GAME menu, REVOLUTION!, Yes
//! city Rome 12,29 change 1 Militia   CHANGE: the menu row (the cursor starts on row 0, as
//!                          choose_production 0xba14 sets no row); the name is checked
//! city Rome 12,29 buy      BUY, Yes
//! city Rome 12,29 sell 0 Barracks   's', the improvement row, "Yes."
//! city Rome 12,29 spec 1   key '1': cycle the first specialist
//! city Rome 12,29 tile 1,-1   'p', the cursor to the square, Return (toggles a worker)
//!                          (the harness opens the city with FindCity and a click on X,Y)
//! ```
//! An action that civrs cannot perform as written (another active unit, a different menu
//! item) is an error: the script and the two games disagree before any state is compared.

use crate::game::turn::CaravanAction;
use crate::game::{AutoStop, Dir8, Game, MoveOutcome, Specialist, UnitOrder};
use crate::rules;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Unit { at: (i32, i32), keys: Vec<String> },
    Select { at: (i32, i32), city: String, city_at: (i32, i32) },
    Rate { lux: bool, rows: i32 },
    Revolution,
    City { name: String, at: (i32, i32), op: CityOp },
}

#[derive(Clone, Debug, PartialEq)]
pub enum CityOp {
    Change { row: usize, expect: Option<String> },
    Buy,
    Sell { row: usize, expect: Option<String> },
    Specialist(usize),
    Tile(i32, i32),
}

/// The script's actions with the turn each runs at.
pub fn parse(text: &str) -> Result<Vec<(u32, Action)>, String> {
    let mut out = Vec::new();
    let mut turn = None;
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let err = |m: &str| format!("line {}: {m}: {line}", n + 1);
        let w: Vec<&str> = line.split_whitespace().collect();
        let pos = |s: &str| -> Result<(i32, i32), String> {
            let (x, y) = s.split_once(',').ok_or_else(|| err("expected X,Y"))?;
            Ok((x.parse().map_err(|_| err("bad x"))?, y.parse().map_err(|_| err("bad y"))?))
        };
        let num = |s: Option<&&str>| -> Result<i32, String> { s.ok_or_else(|| err("missing number"))?.trim_start_matches('+').parse().map_err(|_| err("bad number")) };
        let rest = |i: usize| (w.len() > i).then(|| w[i..].join(" "));
        let action = match w[0] {
            "turn" => {
                turn = Some(num(w.get(1))? as u32);
                continue;
            }
            "unit" => Action::Unit { at: pos(w.get(1).ok_or_else(|| err("missing X,Y"))?)?, keys: w[2..].iter().map(|s| s.to_string()).collect() },
            "tax" | "lux" => Action::Rate { lux: w[0] == "lux", rows: num(w.get(1))? },
            "revolution" => Action::Revolution,
            "select" => Action::Select {
                at: pos(w.get(1).ok_or_else(|| err("missing X,Y"))?)?,
                city: w.get(2).ok_or_else(|| err("missing city"))?.to_string(),
                city_at: pos(w.get(3).ok_or_else(|| err("missing city X,Y"))?)?,
            },
            "city" => {
                let name = w.get(1).ok_or_else(|| err("missing city"))?.to_string();
                let at = pos(w.get(2).ok_or_else(|| err("missing X,Y"))?)?;
                let op = match w.get(3).copied() {
                    Some("change") => CityOp::Change { row: num(w.get(4))? as usize, expect: rest(5) },
                    Some("buy") => CityOp::Buy,
                    Some("sell") => CityOp::Sell { row: num(w.get(4))? as usize, expect: rest(5) },
                    Some("spec") => CityOp::Specialist(num(w.get(4))? as usize),
                    Some("tile") => {
                        let (dx, dy) = pos(w.get(4).ok_or_else(|| err("missing DX,DY"))?)?;
                        CityOp::Tile(dx, dy)
                    }
                    _ => return Err(err("unknown city action")),
                };
                Action::City { name, at, op }
            }
            _ => return Err(err("unknown action")),
        };
        out.push((turn.ok_or_else(|| err("action before the first turn line"))?, action));
    }
    Ok(out)
}

/// The next unit wanting orders, as the map view's `activate_next`; the units the loop meets
/// on its way that move or work by themselves move or work (`run_autos`).
fn activate_next(g: &mut Game) -> Result<(), String> {
    g.active_unit = g.next_active_unit();
    run_autos(g)
}

/// While the loop's unit moves or works by itself (a goto, an aircraft returning to base, a
/// Settlers at work), move it or press its job's key and go on (a refused job leaves the
/// unit waiting for orders), as the map view's `update` does (`MapView::run_goto`); the audiences the
/// units' looks bring get their first answer, as `--dump state` answers the turn's
/// decisions.
fn run_autos(g: &mut Game) -> Result<(), String> {
    loop {
        settle(g);
        let Some(id) = g.active_unit.filter(|id| g.is_auto_unit(*id)) else { return Ok(()) };
        if let AutoStop::Menu(out, _) = g.run_auto_unit(id) {
            return Err(format!("the goto of the unit at {:?} opens a menu: {out:?}", g.unit(id).pos()));
        }
        if g.unit_opt(id).is_some_and(|u| u.order == UnitOrder::None && u.moves_left > 0) {
            g.unit_waits(id);
        } else {
            g.active_unit = g.next_active_unit();
        }
    }
}

/// Answer the decisions among the new events with their first choice; the rest go.
fn settle(g: &mut Game) {
    for e in g.take_events() {
        if let crate::game::GameEvent::NeedsDecision(d) = e {
            g.apply_decision(&d, crate::game::DecisionReply::Choice(0));
        }
    }
}

/// Perform one action for the human, as the map view and the city screen do on its keys.
pub fn apply(g: &mut Game, a: &Action) -> Result<(), String> {
    let me = g.human_slot;
    match a {
        Action::Unit { at, keys } => {
            let id = g.active_unit.ok_or("no active unit")?;
            if g.unit(id).pos() != *at {
                return Err(format!("the active unit stands at {:?}, not {at:?}", g.unit(id).pos()));
            }
            for k in keys {
                // a Return for a message box in CIV.EXE (a trade route, a fight's report)
                if k == "ret" {
                    continue;
                }
                let id = g.active_unit.ok_or_else(|| format!("no active unit for key {k}"))?;
                unit_key(g, id, k)?;
            }
            Ok(())
        }
        Action::Rate { lux, rows } => {
            // o 0x6fd9 / 0x70bf: the rows are the rates 0, 10, ... of the share the other rate
            // leaves, the current one preselected
            let p = g.player(me);
            let (fixed, cur) = if *lux { (p.tax, p.lux) } else { (p.lux, p.tax) };
            let rate = cur as i32 + 10 * rows;
            if rate < 0 || rate + fixed as i32 > 100 {
                return Err(format!("no rate row {rows} from {cur}"));
            }
            let (tax, l) = if *lux { (p.tax, rate as u8) } else { (rate as u8, p.lux) };
            g.set_rates(me, tax, l).map_err(|e| e.message().to_string())
        }
        Action::Select { at, city, city_at } => {
            // FindCity centres the view on the city; a click on a square with one unit of the
            // human's (not a city) lists it, Return makes it the active unit (Game::wake_picked_unit)
            let c = g.city_at(city_at.0, city_at.1).filter(|c| g.city(*c).name == *city).ok_or_else(|| format!("no city {city} at {city_at:?}"))?;
            let view = Game::view_centred_on(city_at.0, city_at.1);
            let (col, row) = (crate::game::Map::wrap_x(at.0 - view.0), at.1 - view.1);
            if col > 14 || !(0..12).contains(&row) {
                return Err(format!("{at:?} is outside the view of {}", g.city(c).name));
            }
            let own: Vec<_> = g.units_at(at.0, at.1).into_iter().filter(|u| g.unit(*u).owner == me).collect();
            if own.len() != 1 || g.city_at(at.0, at.1).is_some() {
                return Err(format!("{at:?} holds {} units of the human or a city", own.len()));
            }
            g.view = Some(view);
            g.wake_picked_unit(own[0]);
            g.active_unit = Some(own[0]);
            Ok(())
        }
        Action::Revolution => {
            g.revolution(me);
            Ok(())
        }
        Action::City { name, at, op } => {
            let c = g.city_at(at.0, at.1).filter(|c| g.city(*c).owner == me && g.city(*c).name == *name).ok_or_else(|| format!("no city {name} at {at:?}"))?;
            // the harness finds the city with FindCity, which centres the view on it
            g.view = Some(Game::view_centred_on(at.0, at.1));
            match op {
                CityOp::Change { row, expect } => {
                    // CHANGE turns AUTO off (0x117ae)
                    g.city_mut(c).auto_build = false;
                    let list = g.available_production(c);
                    if 1 + list.len() > 30 {
                        return Err("the production menu has two pages".into());
                    }
                    let p = *list.get(*row).ok_or_else(|| format!("no production row {row}"))?;
                    check(p.name(), expect)?;
                    g.set_production(c, p).map_err(|e| e.message().to_string())
                }
                CityOp::Buy => g.buy(c).map_err(|e| e.message().to_string()),
                CityOp::Sell { row, expect } => {
                    // the improvements in the order the city screen lists them
                    let ids: Vec<u8> = g.city(c).improvement_list();
                    let id = *ids.get(*row).ok_or_else(|| format!("no improvement row {row}"))?;
                    check(rules::IMPROVEMENTS[id as usize].name, expect)?;
                    g.sell_improvement(c, id).map(|_| ()).map_err(|e| e.message().to_string())
                }
                CityOp::Specialist(i) => {
                    let city = g.city(c);
                    let s = *city.specialists.get(i.wrapping_sub(1)).ok_or_else(|| format!("no specialist {i}"))?;
                    if city.size < 5 {
                        return Err("specialists need size 5".into());
                    }
                    let next = match s {
                        Specialist::Entertainer => Specialist::Taxman,
                        Specialist::Taxman => Specialist::Scientist,
                        Specialist::Scientist => Specialist::Entertainer,
                    };
                    g.set_specialist(c, i - 1, next);
                    Ok(())
                }
                CityOp::Tile(dx, dy) => {
                    if (*dx, *dy) == (0, 0) {
                        g.fix_workers(c);
                    } else {
                        let idx = rules::CITY_RADIUS.iter().position(|&(rx, ry)| (rx as i32, ry as i32) == (*dx, *dy)).ok_or("not a city square")?;
                        g.toggle_tile(c, idx as u8);
                    }
                    Ok(())
                }
            }
        }
    }
}

fn check(name: &str, expect: &Option<String>) -> Result<(), String> {
    match expect {
        Some(e) if e != name => Err(format!("the row holds {name}, not {e}")),
        _ => Ok(()),
    }
}

/// One map-view key for the active unit `id` (MapView::key_order, move_active).
fn unit_key(g: &mut Game, id: crate::game::UnitId, k: &str) -> Result<(), String> {
    let order = |g: &mut Game, o: UnitOrder| -> Result<(), String> {
        g.give_order(id, o).map_err(|e| e.message().to_string())?;
        activate_next(g)
    };
    if let Some(n) = k.strip_prefix('k') {
        // kN:ROW answers the menu the move opens with its row
        let (n, row) = match n.split_once(':') {
            Some((n, r)) => (n, Some(r.parse::<usize>().map_err(|_| format!("bad menu row in {k}"))?)),
            None => (n, None),
        };
        let d = n.parse().ok().and_then(Dir8::from_keypad).ok_or_else(|| format!("bad key {k}"))?;
        let caravan = g.unit(id).kind == rules::unit_ids::CARAVAN;
        match g.try_move(id, d).map_err(|e| e.message().to_string())? {
            MoveOutcome::Moved | MoveOutcome::Attacked { .. } if row.is_none() => {}
            // a caravan in a foreign city sets up the route at once (0x5d15)
            MoveOutcome::EnteredForeignCity(c) if caravan && row.is_none() => g.caravan_action(id, CaravanAction::TradeRoute(c)).map_err(|e| e.message().to_string())?,
            // "Will you?" (0x5cbe): Keep moving, Establish trade route (greyed for a wonder
            // city within 10 squares of home, ds:0xb276 = 2), Help build WONDER.
            MoveOutcome::CaravanArrived(c) => {
                let wonder = matches!(g.city(c).production, crate::game::Production::Wonder(_));
                let near = g.unit(id).home.and_then(|h| g.city_opt(h)).is_some_and(|h| crate::game::Map::dist(h.pos(), g.city(c).pos()) < 10);
                match row {
                    Some(0) => {
                        g.move_on(id, d).map_err(|e| e.message().to_string())?;
                    }
                    Some(1) if !(wonder && near) => g.caravan_action(id, CaravanAction::TradeRoute(c)).map_err(|e| e.message().to_string())?,
                    Some(2) if wonder => g.caravan_action(id, CaravanAction::HelpWonder(c)).map_err(|e| e.message().to_string())?,
                    _ => return Err(format!("move {k}: no caravan menu row {row:?}")),
                }
            }
            // "We have signed a peace treaty with the X!" (0x1a3cf): 0 Cancel action., 1 Break
            // treaty. (no Senate asks), and the move goes on
            MoveOutcome::BreakTreaty(o) => match row {
                Some(0) => {}
                Some(1) => {
                    g.treaty_break_ok = Some(o);
                    let out = g.try_move(id, d).map_err(|e| e.message().to_string());
                    g.treaty_break_ok = None;
                    g.weak_attack_ok = false;
                    match out? {
                        MoveOutcome::Moved | MoveOutcome::Attacked { .. } => {}
                        out => return Err(format!("move {k} after breaking the treaty: {out:?}")),
                    }
                }
                _ => return Err(format!("move {k}: no treaty menu row {row:?}")),
            },
            out => return Err(format!("move {k}: {out:?}")),
        }
        // the map view follows the unit (scroll_to 0x9505, MapView::ensure_visible)
        if let (Some(u), Some(view)) = (g.unit_opt(id), g.view) {
            g.view = Some(Game::view_scrolled_to(view, u.x, u.y));
        }
        if g.unit_opt(id).map(|u| u.moves_left == 0 || u.order != UnitOrder::None).unwrap_or(true) {
            activate_next(g)?;
        }
        return Ok(());
    }
    match k {
        "space" => {
            g.skip_unit(id);
            activate_next(g)
        }
        "w" => {
            g.wait_unit(id);
            if g.active_unit.is_none() {
                g.active_unit = Some(id);
            }
            run_autos(g)
        }
        "f" => order(g, UnitOrder::Fortify),
        "s" => order(g, UnitOrder::Sentry),
        "r" => order(g, UnitOrder::Road),
        "i" => order(g, UnitOrder::Irrigate),
        "m" => order(g, UnitOrder::Mine),
        "p" => order(g, UnitOrder::CleanPollution),
        "P" => order(g, UnitOrder::Pillage),
        "b" => {
            g.found_city(id).map_err(|e| e.message().to_string())?;
            activate_next(g)
        }
        "h" => {
            // an aircraft elsewhere flies home at once (MapView::run_goto)
            if g.press_home(id).is_some() {
                g.run_auto_unit(id);
            }
            if g.unit_opt(id).map(|u| u.moves_left == 0 || u.order != UnitOrder::None).unwrap_or(true) {
                activate_next(g)?;
            }
            Ok(())
        }
        // 'D' deletes the unit at once (0x4c5a)
        "D" => {
            g.disband(id);
            activate_next(g)
        }
        "u" => {
            if let Some(next) = g.unload(id) {
                g.active_unit = g.next_active_unit_at(next);
            }
            run_autos(g)
        }
        _ => Err(format!("unknown unit key {k}")),
    }
}

/// Play `turns` turns from an imported save with the script's actions at the start of the
/// human's turns, the rest as `--dump state:N` plays them (decisions get their first choice).
/// After the actions every unit still waiting gets a Space, as the harness presses it until
/// the turn ends, so the loop goes through the units that move by themselves.
pub fn play(g: &mut Game, ai: &mut crate::ai::Ai, actions: &[(u32, Action)], turns: u32) -> Result<(), String> {
    use crate::game::turn::TurnStep;
    run_autos(g)?;
    for _ in 0..turns {
        let turn = g.turn;
        for (_, a) in actions.iter().filter(|(t, _)| *t == turn) {
            apply(g, a).map_err(|e| format!("turn {}: {a:?}: {e}", g.turn))?;
            settle(g);
        }
        while let Some(id) = g.active_unit {
            g.skip_unit(id);
            activate_next(g)?;
        }
        let mut r = g.end_turn();
        loop {
            match r.step(g, ai) {
                TurnStep::Done => break,
                TurnStep::Decision(_) => r.resolve(g, crate::game::DecisionReply::Choice(0)),
                _ => {}
            }
        }
        // the turn's loop has taken its first unit (`Phase::HumanStart`)
        run_autos(g)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_script_lines() {
        let a = parse("# x\nturn 36\nunit 10,25 r k4  # road\ntax +1\ncity Rome 12,29 change 2 Militia\ncity Rome 12,29 sell 1\nturn 37\nrevolution\ncity Rome 12,29 tile 1,-1\n").unwrap();
        assert_eq!(a[0], (36, Action::Unit { at: (10, 25), keys: vec!["r".into(), "k4".into()] }));
        assert_eq!(a[1], (36, Action::Rate { lux: false, rows: 1 }));
        let rome = |op| Action::City { name: "Rome".into(), at: (12, 29), op };
        assert_eq!(a[2], (36, rome(CityOp::Change { row: 2, expect: Some("Militia".into()) })));
        assert_eq!(a[3], (36, rome(CityOp::Sell { row: 1, expect: None })));
        assert_eq!(a[4], (37, Action::Revolution));
        assert_eq!(a[5], (37, rome(CityOp::Tile(1, -1))));
        assert_eq!(parse("turn 3\nselect 7,23 Caesarea 9,25\n").unwrap()[0], (3, Action::Select { at: (7, 23), city: "Caesarea".into(), city_at: (9, 25) }));
        assert!(parse("unit 1,1 r").is_err());
    }
}
