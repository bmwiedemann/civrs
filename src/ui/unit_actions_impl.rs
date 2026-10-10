//! Placeholder until the engine exposes caravan_action / diplomat_action.
use crate::ui::context::UiContext;
pub fn perform(_ctx: &mut UiContext, _id: crate::game::UnitId, _city: crate::game::CityId, caravan: bool, choice: usize) -> Result<String, crate::game::RuleError> {
    let _ = (caravan, choice);
    Ok(String::new())
}
