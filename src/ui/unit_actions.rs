//! Caravan and diplomat actions in foreign cities (menu handling for the map view).

use crate::game::{Map, UnitId};
use crate::rules;
use crate::ui::context::UiContext;
use crate::ui::mapview::MapView;
use crate::ui::screen::Transition;
use crate::ui::widgets::MessageBox;

pub fn special_unit_choice(view: &mut MapView, ctx: &mut UiContext, id: UnitId, choice: usize) -> Transition {
    let g = ctx.game();
    let Some(u) = g.unit_opt(id) else { return Transition::None };
    let (ux, uy) = u.pos();
    let target = g.all_cities().filter(|c| c.owner != u.owner && Map::dist((c.x, c.y), (ux, uy)) <= 1).map(|c| c.id).next();
    let Some(c) = target else { return Transition::None };
    let is_caravan = u.kind == rules::unit_ids::CARAVAN;
    let r = perform(ctx, id, c, is_caravan, choice);
    let _ = ctx.game_mut().take_events();
    match r {
        Ok(msg) if !msg.is_empty() => {
            view.set_pending_event();
            Transition::Push(Box::new(MessageBox::text(&msg)))
        }
        Ok(_) => {
            view.finish_unit(ctx);
            Transition::None
        }
        Err(e) => view.show_error(ctx, &e),
    }
}

/// Execute the chosen action; returns a message to display. Wired to the engine's
/// caravan_action / diplomat_action once those exist.
fn perform(ctx: &mut UiContext, id: UnitId, city: crate::game::CityId, caravan: bool, choice: usize) -> Result<String, crate::game::RuleError> {
    crate::ui::unit_actions_impl::perform(ctx, id, city, caravan, choice)
}
