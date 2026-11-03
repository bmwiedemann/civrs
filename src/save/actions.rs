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
//!                          partner's unit or land: 0 cancel, 1 break the treaty, which a
//!                          Senate overrules with a message box: add `ret`) and its row;
//!                          a caravan entering a foreign city sets up its route, a move onto a
//!                          foreign unit attacks
//! unit 13,24 k9:4:1 ret ret esc   a Diplomat entering a foreign city: the action's row
//!                          (0 embassy, 1 investigate, 2 steal, 3 sabotage, 4 incite; Meet with
//!                          King is not scripted) and, for a theft from a treaty partner (0
//!                          cancel, 1 break the treaty) or an incite offer (0 forget it, 1
//!                          incite, 2 subvert), the next menu's row; then a `ret` (Return) for
//!                          each report or popup and an `esc` (Escape) for a city screen. A
//!                          Diplomat onto a lone foreign unit: kN:1 pays the bribe, kN:0 forgets
//!                          it, kN alone meets a price above the treasury (`ret`)
//! menu 1                  answers the menu a unit's goto opened as the loop came to it
//!                          (a treaty partner's land or unit: 0 cancel, 1 break the
//!                          treaty), as kN:ROW does for a move by hand; the next action
//!                          must be this one
//! audience ret 0 ret 1    only in the first turn block: the keys for the audience the loaded
//!                          game opens with (its first looks meet a computer's unit): `ret` a
//!                          Return for a screen without choices, N a menu's row (the harness
//!                          sends them before the options; civrs gives the rows to the
//!                          audience's menus in turn, later menus get row 0)
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
//!
//! Every other menu gets its first row, in CIV.EXE the harness's Return on the cursor's
//! starting row: an audience's *PEACE is "Reject" (o 0x6fdf), so a human Republic or
//! Democracy is overruled by the Senate ("Treaty signed.", o 0x703b) and any other human
//! stays at war.

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
    /// The row for the menu a goto opened (`run_autos`).
    Menu(usize),
    /// The keys for the audience the loaded game opens with (the first looks' emissary):
    /// `None` a Return for a screen without choices, `Some(row)` a menu row.
    Audience(Vec<Option<usize>>),
}

thread_local! {
    /// The rows still to give the audience's menus (`Action::Audience`); others get row 0.
    static ANSWERS: std::cell::RefCell<std::collections::VecDeque<usize>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    /// The treaty question a goto step opened (`run_autos`): the unit, its step, the partner.
    static OPEN: std::cell::Cell<Option<(crate::game::UnitId, Dir8, crate::game::Slot)>> = const { std::cell::Cell::new(None) };
}

fn open_question() -> Result<(), String> {
    match OPEN.get() {
        Some((_, _, o)) => Err(format!("a goto's treaty question (civ {o}) waits for its `menu` answer")),
        None => Ok(()),
    }
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
            "menu" => Action::Menu(num(w.get(1))? as usize),
            "audience" => Action::Audience(w[1..].iter().map(|k| if *k == "ret" { Ok(None) } else { k.parse().map(Some).map_err(|_| err("bad audience key")) }).collect::<Result<_, _>>()?),
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
        let stop = g.run_auto_unit(id);
        // "We have signed a peace treaty" for a goto's step (0x656d, 0x1a3b4): the unit
        // waits for the script's `menu` answer
        if let AutoStop::Menu(MoveOutcome::BreakTreaty(o), d) = stop {
            OPEN.set(Some((id, d, o)));
            return Ok(());
        }
        if let AutoStop::Menu(out, _) = stop {
            return Err(format!("the goto of the unit at {:?} opens a menu: {out:?}", g.unit(id).pos()));
        }
        if g.unit_opt(id).is_some_and(|u| u.order == UnitOrder::None && u.moves_left > 0) {
            g.unit_waits(id);
        } else {
            g.active_unit = g.next_active_unit();
        }
    }
}

/// Answer the decisions among the new events with their first choice (an audience's menus
/// with the rows `Action::Audience` left); the rest go.
fn settle(g: &mut Game) {
    // an answered audience screen queues the next one: until none is left
    loop {
        let mut answered = false;
        for e in g.take_events() {
            if let crate::game::GameEvent::NeedsDecision(d) = e {
                let row = match &d {
                    crate::game::Decision::Diplomacy(st) if st.choices() > 0 => ANSWERS.with_borrow_mut(|a| a.pop_front()).unwrap_or(0),
                    _ => 0,
                };
                g.apply_decision(&d, crate::game::DecisionReply::Choice(row));
                answered = true;
            }
        }
        if !answered {
            return;
        }
    }
}

/// Perform one action for the human, as the map view and the city screen do on its keys.
pub fn apply(g: &mut Game, a: &Action) -> Result<(), String> {
    let me = g.human_slot;
    if !matches!(a, Action::Menu(_)) {
        open_question()?;
    }
    match a {
        // taken before the load's looks (`play`)
        Action::Audience(_) => Ok(()),
        Action::Menu(row) => {
            let (id, d, o) = OPEN.take().ok_or("no menu is open")?;
            answer_treaty(g, id, d, o, Some(*row), "menu")?;
            unit_done(g, id)
        }
        Action::Unit { at, keys } => {
            let id = g.active_unit.ok_or("no active unit")?;
            if g.unit(id).pos() != *at {
                return Err(format!("the active unit stands at {:?}, not {at:?}", g.unit(id).pos()));
            }
            for k in keys {
                // a Return for a message box in CIV.EXE (a trade route, a fight's report)
                if k == "ret" || k == "esc" {
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
                    // CHANGE turns AUTO off (0x117ae); choose_production values the menu for
                    // the human too (0x117ef -> 0xa9fd), the advisors shown or not
                    g.city_mut(c).auto_build = false;
                    let _ = g.change_menu_advice(c);
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
        // kN:ROW answers the menu the move opens with its row, kN:ROW:ROW2 the menu that
        // row opens next (a Diplomat's incite offer or treaty question)
        let mut parts = n.split(':');
        let n = parts.next().unwrap_or("");
        let rows: Vec<usize> = parts.map(|r| r.parse::<usize>().map_err(|_| format!("bad menu row in {k}"))).collect::<Result<_, _>>()?;
        if rows.len() > 2 {
            return Err(format!("too many menu rows in {k}"));
        }
        let (row, row2) = (rows.first().copied(), rows.get(1).copied());
        if row2.is_some() && g.unit(id).kind != rules::unit_ids::DIPLOMAT {
            return Err(format!("move {k}: a second menu row is for a Diplomat"));
        }
        let d = n.parse().ok().and_then(Dir8::from_keypad).ok_or_else(|| format!("bad key {k}"))?;
        let caravan = g.unit(id).kind == rules::unit_ids::CARAVAN;
        match g.try_move(id, d).map_err(|e| e.message().to_string())? {
            MoveOutcome::Moved | MoveOutcome::Attacked { .. } if row.is_none() => {}
            // a city taken (its messages take a `ret` each in CIV.EXE)
            MoveOutcome::CapturedCity(_) | MoveOutcome::DestroyedCity if row.is_none() => {}
            // a caravan in a foreign city sets up the route at once (0x5d15)
            MoveOutcome::EnteredForeignCity(c) if caravan && row.is_none() => g.caravan_action(id, CaravanAction::TradeRoute(c)).map_err(|e| e.message().to_string())?,
            // "ADJ diplomat arrives in CITY..." (o 0x1d575)
            MoveOutcome::EnteredForeignCity(c) if !caravan => diplomat(g, id, c, row, row2, k)?,
            // "ADJ UNIT will desert for N$." (o 0x1dd15): with the price in the treasury 0
            // Forget it., 1 Pay (and the Diplomat moves on); else a message box (`ret`)
            MoveOutcome::BribeOffer(t) if row2.is_none() => {
                let (cost, gold) = (g.bribe_cost(t), g.player(g.human_slot).gold);
                match row {
                    None if cost > gold => {}
                    Some(0) if cost <= gold => {}
                    Some(1) if cost <= gold => {
                        g.bribe_unit(g.human_slot, t);
                        match g.try_move(id, d).map_err(|e| e.message().to_string())? {
                            MoveOutcome::Moved => {}
                            out => return Err(format!("move {k} after the bribe: {out:?}")),
                        }
                    }
                    _ => return Err(format!("move {k}: no bribe menu row {row:?} (price {cost}, treasury {gold})")),
                }
            }
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
            MoveOutcome::BreakTreaty(o) => answer_treaty(g, id, d, o, row, k)?,
            out => return Err(format!("move {k}: {out:?}")),
        }
        return unit_done(g, id);
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

/// A Diplomat's menu at the foreign city `c` (o 0x1d575, rows as `DIPLOMAT_MENU`; a greyed
/// row is an error), as `ui::unit_actions` performs it: Steal Technology from a treaty
/// partner asks first (`row2` 0 Cancel action., 1 Break treaty.; a Senate overrules with a
/// message box), Incite a Revolt makes its offer (`row2` 0 Forget it., 1 Incite revolt, 2
/// Subvert city; a price above the treasury is a message box, no `row2`). The reports,
/// popups and city screens that follow take a `ret` each in CIV.EXE. Meet with King is not
/// scripted.
fn diplomat(g: &mut Game, id: crate::game::UnitId, c: crate::game::CityId, row: Option<usize>, row2: Option<usize>, k: &str) -> Result<(), String> {
    use crate::game::turn::DiplomatAction as A;
    let row = row.ok_or_else(|| format!("move {k}: the Diplomat's menu needs a row"))?;
    let (us, them) = (g.unit(id).owner, g.city(c).owner);
    if g.diplomat_menu_disabled(us, c).get(row).copied().unwrap_or(true) {
        return Err(format!("move {k}: Diplomat menu row {row} is greyed or missing"));
    }
    let action = match A::from_menu(row) {
        Some(A::MeetKing) | None => return Err(format!("move {k}: Meet with King is not scripted")),
        Some(A::StealTech) if g.treaty(us, them) => match row2 {
            Some(0) => return Ok(()),
            Some(1) if g.senate_overrules(us) => return Ok(()),
            Some(1) => A::StealTech,
            _ => return Err(format!("move {k}: no treaty menu row {row2:?}")),
        },
        Some(A::InciteRevolt { .. }) => {
            let cost = g.incite_cost(c).ok_or("a capital cannot be incited")?;
            let gold = g.player(us).gold;
            let subvert = cost * 2 <= gold && g.treaty(us, them);
            match row2 {
                None if cost > gold => return Ok(()),
                Some(0) if cost <= gold => return Ok(()),
                Some(1) if cost <= gold => A::InciteRevolt { subvert: false },
                Some(2) if subvert => A::InciteRevolt { subvert: true },
                _ => return Err(format!("move {k}: no incite menu row {row2:?} (price {cost}, treasury {gold})")),
            }
        }
        Some(a) if row2.is_none() => a,
        Some(_) => return Err(format!("move {k}: Diplomat menu row {row} opens no second menu")),
    };
    g.diplomat_action(id, action, c).map(|_| ()).map_err(|e| e.message().to_string())
}

/// After a move: the map view follows the unit (scroll_to 0x9505, MapView::ensure_visible);
/// a unit with moves left and no order keeps the focus (0x6d7f -> 0x442c), else the loop
/// goes on.
fn unit_done(g: &mut Game, id: crate::game::UnitId) -> Result<(), String> {
    if let (Some(u), Some(view)) = (g.unit_opt(id), g.view) {
        g.view = Some(Game::view_scrolled_to(view, u.x, u.y));
    }
    if g.unit_opt(id).map(|u| u.moves_left == 0 || u.order != UnitOrder::None).unwrap_or(true) {
        activate_next(g)?;
    }
    Ok(())
}

/// "We have signed a peace treaty with the X!" (0x1a3cf): row 0 Cancel action. (a fight's
/// question drops the goto then, 0x19a1a; the territory rule has dropped it before asking,
/// 0x656d), 1 Break treaty., and the move goes on (no Senate asks).
fn answer_treaty(g: &mut Game, id: crate::game::UnitId, d: Dir8, o: crate::game::Slot, row: Option<usize>, k: &str) -> Result<(), String> {
    match row {
        Some(0) => g.clear_goto(id),
        // the Senate of a Republic or Democracy: "Overruled by the Senate. Action canceled."
        // (0x1a42d), a message box the script answers with `ret`
        Some(1) if g.senate_overrules(g.human_slot) => g.clear_goto(id),
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
    }
    Ok(())
}

/// Play `turns` turns from an imported save with the script's actions at the start of the
/// human's turns, the rest as `--dump state:N` plays them (decisions get their first choice).
/// After the actions every unit still waiting gets a Space, as the harness presses it until
/// the turn ends, so the loop goes through the units that move by themselves.
pub fn play(g: &mut Game, ai: &mut crate::ai::Ai, actions: &[(u32, Action)], turns: u32) -> Result<(), String> {
    use crate::game::turn::TurnStep;
    OPEN.set(None);
    // the audience the load's looks open is answered before anything else, as the harness
    // sends its keys before the options (`Action::Audience`)
    let start = g.turn;
    let rows: std::collections::VecDeque<usize> = actions.iter().filter(|(t, _)| *t == start).flat_map(|(_, a)| match a {
        Action::Audience(keys) => keys.iter().flatten().copied().collect(),
        _ => Vec::new(),
    }).collect();
    ANSWERS.set(rows);
    // a new game's turn 0 runs up to the human's first move first, as the map view starts it
    if let Some(mut r) = g.take_opening() {
        loop {
            match r.step(g, ai) {
                TurnStep::Done => break,
                TurnStep::Decision(_) => r.resolve(g, crate::game::DecisionReply::Choice(0)),
                _ => {}
            }
        }
    }
    run_autos(g)?;
    let left = ANSWERS.with_borrow(|a| a.len());
    if left != 0 {
        return Err(format!("{left} audience rows left over: no audience at the load"));
    }
    // the harness's first key after the load is a Space: with no unit of the human waiting
    // for orders it ends the turn at once, before any save at record 0x80 (the options menu
    // before it returns no command, ds:0xd4ca = -1)
    let idle_load = g.active_unit.is_none();
    for i in 0..turns {
        let turn = g.turn;
        for (_, a) in actions.iter().filter(|(t, _)| *t == turn) {
            apply(g, a).map_err(|e| format!("turn {}: {a:?}: {e}", g.turn))?;
            settle(g);
        }
        open_question().map_err(|e| format!("turn {}: {e}", g.turn))?;
        while let Some(id) = g.active_unit {
            g.skip_unit(id);
            activate_next(g)?;
        }
        if !(i == 0 && idle_load) {
            harness_key(g, true);
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
        harness_key(g, false);
    }
    Ok(())
}

/// The harness's save while the human's loop has no unit (sve_diff.py play): at the start of
/// a turn with no unit waiting for orders, and the one before the Space that ends a turn
/// (`end`; the turn's last save comes once its units are done). With no unit to take, the
/// loop waits for it at record 0x80 (0x76c3 after the looks, 0x43f4), and CIV.EXE runs a
/// menu's command (GAME, Save Game) through the unit code for that record (0x73cc -> 0x4bbe,
/// past the record check at 0x4bb4 that a key meets), which is record 0 of the next
/// civilization (0x600 bytes on): without a goto its
/// heading is forgotten (0x4bf8), so its next step is scored without the turn penalty; with
/// one, goto_direction runs on it for the human's civ (0x4bce..0x4bdb), the human's rules
/// and owner tests included (a far goto whose every step leads away is dropped), and only
/// its side effects stay: the key it returns replaces the menu's and matches no command
/// (0x4c1d..0x4c57). With `exe_quirks` only.
fn harness_key(g: &mut Game, end: bool) {
    if !end && g.active_unit.is_some() {
        return;
    }
    harness_save(g);
}

/// `harness_key`'s heading store at 0x4bf8.
fn harness_save(g: &mut Game) {
    if !g.exe_quirks {
        return;
    }
    let next = g.human_slot + 1;
    if next > 7 {
        return;
    }
    let first = g.all_units().find(|u| u.owner == next && u.record == 0).map(|u| (u.id, u.goto));
    match first {
        Some((id, None)) => g.set_heading(id, 0xff),
        Some((id, Some(_))) => {
            let h = g.human_slot;
            g.goto_direction_as(id, h);
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_quirks_the_harness_save_forgets_the_next_civs_first_heading() {
        // 0x73cc -> 0x4bbe with record 0x80 of the human's civ: record 0 of the next one
        use crate::game::GameSetup;
        use crate::rules::unit_ids;
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let next = g.human_slot + 1;
        for id in g.units_of(next) {
            g.remove_unit(id);
        }
        let u = g.add_unit(next, unit_ids::MILITIA, 30, 30);
        g.unit_mut(u).record = 0;
        g.set_heading(u, 3);
        harness_save(&mut g);
        assert_eq!(g.heading(u), 3, "civrs keeps it");
        g.exe_quirks = true;
        harness_save(&mut g);
        assert_eq!(g.heading(u), 0xff);
        // with a goto it is goto_direction for the human's civ (0x4bce): an adjacent target is
        // the human's, taken by the step key and dropped, so the unit keeps no goto
        let (x, y) = g.unit(u).pos();
        g.unit_mut(u).goto = Some((x + 1, y));
        harness_save(&mut g);
        assert_eq!(g.unit(u).goto, None);
        assert_eq!(g.unit(u).pos(), (x, y));
    }

    #[test]
    fn change_values_the_menu_for_the_human_too() {
        // CHANGE 0x117ef -> choose_production 0xa9fd: the valuation runs with the advisors
        // off, and its Diplomat line draws random(5) (0xb3e0) once the human can build one
        use crate::game::GameSetup;
        use crate::rules::{advance_ids as adv, unit_ids};
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let me = g.human_slot;
        g.options.instant_advice = false;
        let settler = g.units_of(me).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        g.player_mut(me).learn(adv::WRITING);
        let (name, at) = (g.city(c).name.clone(), g.city(c).pos());
        let row = g.available_production(c).iter().position(|p| *p == crate::game::Production::Unit(unit_ids::MILITIA)).unwrap();
        crate::game::rng::trace_start();
        apply(&mut g, &Action::City { name, at, op: CityOp::Change { row, expect: Some("Militia".into()) } }).unwrap();
        let draws = crate::game::rng::trace_take();
        assert_eq!(draws.iter().map(|d| (d.at, d.n)).collect::<Vec<_>>(), vec![(0xb3e0, 5)]);
        assert_eq!(g.city(c).production, crate::game::Production::Unit(unit_ids::MILITIA));
    }

    /// A flat arena with a city of civ 2 at (20, 20) (its capital at (8, 20)) and the human's
    /// Diplomat west of it, the active unit.
    fn diplomat_arena() -> (Game, crate::game::CityId, crate::game::UnitId) {
        use crate::game::GameSetup;
        use crate::rules::unit_ids;
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        g.players[2].in_game = true;
        g.players[2].alive = true;
        for y in 5..30 {
            for x in 5..30 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = crate::rules::Terrain::Grassland;
                t.flags = 0;
            }
        }
        let ids: Vec<_> = g.all_units().filter(|u| (5..30).contains(&u.x) && (5..30).contains(&u.y)).map(|u| u.id).collect();
        for id in ids {
            g.remove_unit(id);
        }
        g.set_relation(1, 2, crate::game::Relation::War);
        let cap = g.add_city(2, "Capital".into(), 8, 20);
        g.city_mut(cap).add_improvement(crate::rules::improvement_ids::PALACE);
        let c = g.add_city(2, "Target".into(), 20, 20);
        let d = g.add_unit(1, unit_ids::DIPLOMAT, 19, 20);
        g.active_unit = Some(d);
        assert_eq!(g.human_slot, 1);
        (g, c, d)
    }

    #[test]
    fn a_scripted_diplomat_takes_its_menu_rows() {
        // kN:ROW:ROW2 (o 0x1d575): Incite a Revolt (row 4), then the offer's row
        let unit = |k: &str| Action::Unit { at: (19, 20), keys: vec![k.into()] };
        let (mut g, c, d) = diplomat_arena();
        g.player_mut(1).gold = 5000;
        // the offer's "Forget it." leaves everything as it was
        apply(&mut g, &unit("k6:4:0")).unwrap();
        assert_eq!((g.city(c).owner, g.unit_opt(d).is_some(), g.player(1).gold), (2, true, 5000));
        // no subversion without a treaty, and a theft from an enemy asks nothing more
        assert!(apply(&mut g, &unit("k6:4:2")).is_err());
        assert!(apply(&mut g, &unit("k6:2:1")).is_err());
        // Meet with King is not scripted; a capital's Incite row is greyed
        assert!(apply(&mut g, &unit("k6:5")).is_err());
        apply(&mut g, &unit("k6:4:1")).unwrap();
        assert_eq!(g.city(c).owner, 1);
        assert!(g.unit_opt(d).is_none(), "deleted after the transfer (o 0x1db51)");
        assert!(g.player(1).gold < 5000);
        let (mut g, _, d) = diplomat_arena();
        g.unit_mut(d).x = 9;
        let cap = g.city_at(8, 20).unwrap();
        assert!(diplomat(&mut g, d, cap, Some(4), Some(1), "k4:4:1").is_err(), "greyed");
        // Establish Embassy uses the Diplomat up
        let (mut g, _, d) = diplomat_arena();
        apply(&mut g, &unit("k6:0")).unwrap();
        assert!(g.player(1).has_embassy(2));
        assert!(g.unit_opt(d).is_none());
        assert_eq!(parse("turn 1\nunit 19,20 k6:4:1 ret ret esc\n").unwrap()[0].1, Action::Unit { at: (19, 20), keys: vec!["k6:4:1".into(), "ret".into(), "ret".into(), "esc".into()] });
    }

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
        assert_eq!(parse("turn 3\nmenu 1\n").unwrap()[0], (3, Action::Menu(1)));
        assert!(parse("unit 1,1 r").is_err());
    }
}
