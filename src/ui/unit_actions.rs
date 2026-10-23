//! Diplomat actions in foreign cities (menu handling for the map view), CIV.EXE o 0x1d575.

use crate::game::turn::{DiplomatAction, DiplomatResult, DIPLOMAT_MENU};
use crate::game::{CityId, UnitId};
use crate::rules;
use crate::ui::context::UiContext;
use crate::ui::mapview::{MapView, Pending};
use crate::ui::screen::{Screen, Transition};
use crate::ui::widgets::{MenuDialog, MessageBox};

/// "<Adjective> diplomat arrives in <city>..." with the six actions, the impossible ones greyed.
pub fn diplomat_menu(view: &mut MapView, ctx: &mut UiContext, id: UnitId, c: CityId) -> Transition {
    let g = ctx.game();
    let slot = g.unit(id).owner;
    let disabled = g.diplomat_menu_disabled(slot, c).to_vec();
    let title = vec![format!("{} diplomat arrives", g.player(slot).adjective()), format!("in {}...", g.city(c).name)];
    view.set_pending(Pending::Diplomat(id, c));
    Transition::Push(Box::new(MenuDialog::new(title, DIPLOMAT_MENU.iter().map(|s| s.to_string()).collect()).with_pos(80, 80).with_disabled(disabled)))
}

/// A line of the diplomat's menu was chosen.
pub fn diplomat_choice(view: &mut MapView, ctx: &mut UiContext, id: UnitId, c: CityId, choice: usize) -> Transition {
    let g = ctx.game();
    if g.unit_opt(id).is_none() || g.city_opt(c).is_none() {
        return Transition::None;
    }
    let (us, them) = (g.unit(id).owner, g.city(c).owner);
    match DiplomatAction::from_menu(choice) {
        // stealing from a treaty partner needs the treaty broken first (CIV.EXE 0x1a3b4)
        Some(DiplomatAction::StealTech) if g.treaty(us, them) => {
            let lines = vec!["We have signed a".into(), "peace treaty with".into(), format!("the {}!", g.player(them).plural())];
            view.set_pending(Pending::TreatyBreak(id, c));
            Transition::Push(Box::new(MenuDialog::new(lines, vec!["Cancel action.".into(), "Break treaty.".into()]).with_pos(80, 80)))
        }
        Some(DiplomatAction::InciteRevolt { .. }) => incite_offer(view, ctx, id, c),
        Some(action) => perform(view, ctx, id, c, action),
        None => Transition::None,
    }
}

/// "Break treaty." was chosen; under the Republic and Democracy the Senate overrules it.
pub fn treaty_broken(view: &mut MapView, ctx: &mut UiContext, id: UnitId, c: CityId) -> Transition {
    let g = ctx.game();
    if g.unit_opt(id).is_none() {
        return Transition::None;
    }
    if g.senate_overrules(g.unit(id).owner) {
        return Transition::Push(Box::new(MessageBox::new(vec!["Overruled by".into(), "the Senate.".into(), "Action canceled.".into()]).with_pos(80, 80)));
    }
    perform(view, ctx, id, c, DiplomatAction::StealTech)
}

/// "Dissidents in <city> will revolt for $N." with the choices the treasury allows; a capital
/// cannot be incited at all (the diplomat simply stays).
fn incite_offer(view: &mut MapView, ctx: &mut UiContext, id: UnitId, c: CityId) -> Transition {
    let g = ctx.game();
    let Some(cost) = g.incite_cost(c) else { return Transition::None };
    let us = g.unit(id).owner;
    let gold = g.player(us).gold;
    let lines = vec![format!("Dissidents in {}", g.city(c).name), format!("will revolt for ${cost}.")];
    if cost > gold {
        return Transition::Push(Box::new(MessageBox::new(lines).with_pos(100, 80)));
    }
    let mut items = vec!["Forget it.".to_string(), "Incite revolt".to_string()];
    if cost * 2 <= gold && g.treaty(us, g.city(c).owner) {
        items.push("Subvert city ($x2)".into());
    }
    view.set_pending(Pending::Incite(id, c));
    Transition::Push(Box::new(MenuDialog::new(lines, items).with_pos(100, 80)))
}

/// Line `choice` of the incite offer: 1 incite, 2 subvert.
pub fn incite_choice(view: &mut MapView, ctx: &mut UiContext, id: UnitId, c: CityId, choice: usize) -> Transition {
    match choice {
        1 | 2 => perform(view, ctx, id, c, DiplomatAction::InciteRevolt { subvert: choice == 2 }),
        _ => Transition::None,
    }
}

fn perform(view: &mut MapView, ctx: &mut UiContext, id: UnitId, c: CityId, action: DiplomatAction) -> Transition {
    let owner = ctx.game().city(c).owner;
    let (name, production) = {
        let city = ctx.game().city(c);
        (city.name.clone(), city.production.name())
    };
    let result = match ctx.game_mut().diplomat_action(id, action, c) {
        Ok(r) => r,
        Err(e) => return view.show_error(ctx, &e),
    };
    let g = ctx.game();
    let screen: Box<dyn Screen> = match result {
        DiplomatResult::Embassy => crate::ui::reports::advisor(ctx, 3),
        DiplomatResult::Investigated => crate::ui::cityscreen::city_screen_view(c),
        DiplomatResult::Stole(a) => {
            let lines = vec![format!("{} steal", g.player(g.human_slot).plural()), format!("{}.", rules::ADVANCES[a as usize].name)];
            Box::new(MessageBox::new(lines).with_pos(80, 80))
        }
        DiplomatResult::NoTechnology => Box::new(MessageBox::new(vec!["No new technology found.".into()]).with_pos(80, 80)),
        DiplomatResult::Sabotaged(what) => {
            let first = match what {
                Some(i) => format!("{} destroyed", rules::IMPROVEMENTS[i as usize].name),
                None => format!("{production} production sabotaged"),
            };
            Box::new(MessageBox::new(vec![first, format!("in {name}.")]).with_pos(80, 80))
        }
        // the capture message follows from the game's events
        DiplomatResult::Incited { .. } => return view.show_events(ctx),
        DiplomatResult::MetKing => crate::ui::diplomacy_ui::human_audience(owner, ctx.game().city(c).pos()),
    };
    view.set_pending(Pending::Event);
    Transition::Push(screen)
}
