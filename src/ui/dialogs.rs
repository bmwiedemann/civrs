//! Mapping of engine events / decisions to dialogs and event screens: discovery, wonder,
//! government, civil disorder, celebration, barbarian landings, nuclear strikes, diplomacy
//! audiences, end of game, "Travelers report" news boxes, disasters and plain message boxes.

use crate::assets::text::{paragraphs, substitute, Vars};
use crate::game::turn::TurnStep;
use crate::game::{Decision, Game, GameEvent, HutOutcome};
use crate::rules;
use crate::ui::context::UiContext;
use crate::ui::eventpics;
use crate::ui::screen::Screen;
use crate::ui::widgets::{MenuDialog, MessageBox, YesNo};

/// Start the sound CIV.EXE plays for an event (before its dialog opens).
pub fn sound_for_event(ctx: &UiContext, ev: &GameEvent) {
    use crate::audio::Sound;
    let Some(g) = ctx.game.as_ref() else { return };
    let me = g.human_slot;
    let a = &ctx.audio;
    let mine = |c: &crate::game::CityId| g.city_opt(*c).map(|c| c.owner == me).unwrap_or(false);
    match ev {
        GameEvent::Famine(c) | GameEvent::Disorder(c) if mine(c) => a.play(Sound::Disorder),
        GameEvent::Celebration(c) if mine(c) => a.play(Sound::Win),
        GameEvent::CelebrationEnded(c) if mine(c) => a.play(Sound::Lose),
        // completed production has no sound of its own: only the city view plays tune 44 as
        // it adds a new building (CIV.EXE o 0x19fa1)
        GameEvent::Pollution { city, .. } if mine(city) => a.play(Sound::Disorder),
        // "Sensors report a NUCLEAR WEAPONS test"; another civ's wonder gets its jingle, our own
        // only the city view's build-complete sound
        GameEvent::WonderBuilt { wonder, .. } if *wonder == rules::wonder_ids::MANHATTAN_PROJECT => a.play(Sound::Disorder),
        GameEvent::WonderBuilt { owner, .. } if *owner != me => a.national_theme(g.player(*owner).civ, false),
        GameEvent::AdvanceDiscovered { slot, .. } if *slot == me => a.national_theme(g.player(me).civ, false),
        GameEvent::CityCaptured { by, from, .. } if *by == me || *from == me => a.national_theme(g.player(*by).civ, false),
        GameEvent::BarbariansLanded { target, .. } | GameEvent::BarbarianUprising { target, .. } if *target == me => a.play(Sound::Disorder),
        GameEvent::WarDeclared { a: x, b: y } if *x == me || *y == me => a.play(Sound::WarDrums),
        GameEvent::NuclearStrike { .. } => a.play(Sound::Nuke),
        GameEvent::CivDestroyed { slot, by } if *by == me => a.national_theme(g.player(*slot).civ, true),
        GameEvent::GameEnded(reason) => match reason {
            crate::game::EndReason::Destroyed => a.play(Sound::Lose),
            crate::game::EndReason::Spaceship(_) => a.play(Sound::MainTheme),
            // conquest and retirement show their plaque over the full anthem
            _ => a.national_theme(g.player(me).civ, true),
        },
        _ => {}
    }
}

/// The improvement a completed `item` adds to the city view, when the original shows it there
/// (CIV.EXE 0xe676: Animations on, not the Palace, no spaceship part, no auto-build; wonders
/// included).
fn built_view_item(g: &Game, city: crate::game::CityId, item: crate::game::Production) -> Option<crate::ui::city_view::ViewItem> {
    use rules::improvement_ids as imp;
    let c = g.city_opt(city)?;
    match item {
        crate::game::Production::Improvement(i) if g.options.animations && !c.auto_build && i != imp::PALACE && i < imp::SS_STRUCTURAL => {
            Some(crate::ui::city_view::improvement_item(i))
        }
        crate::game::Production::Wonder(w) if g.options.animations && !c.auto_build => Some(crate::ui::city_view::wonder_item(w)),
        _ => None,
    }
}

/// Produce a dialog for a game event, if it concerns the human player and is worth showing.
pub fn screen_for_event(ctx: &mut UiContext, ev: &GameEvent) -> Option<Box<dyn Screen>> {
    sound_for_event(ctx, ev);
    let g = ctx.game.as_ref()?;
    let me = g.human_slot;
    let msg = |lines: Vec<String>| -> Option<Box<dyn Screen>> { Some(Box::new(MessageBox::new(lines))) };
    let news = |lines: Vec<String>| -> Option<Box<dyn Screen>> { Some(eventpics::travelers_report(lines)) };
    let city_name = |id| g.city_opt(id).map(|c| c.name.clone()).unwrap_or_default();
    let civ = |s: u8| -> String {
        if s == 0 {
            "barbarians".to_string()
        } else {
            g.civ_name(s).to_string()
        }
    };
    match ev {
        GameEvent::CityGrew(_) | GameEvent::CityShrank(_) | GameEvent::TurnStarted(_) | GameEvent::UnitVeteran(_) => None,
        GameEvent::Famine(c) if g.city(*c).owner == me => msg(vec![format!("Famine in {}!", city_name(*c)), "Population decreases.".into()]),
        GameEvent::UnitDisbanded { unit_kind, city, reason } if g.city_opt(*city).map(|c| c.owner == me).unwrap_or(false) => {
            msg(vec![format!("{} in {} disbanded", rules::UNITS[*unit_kind as usize].name, city_name(*city)), format!("({reason})")])
        }
        GameEvent::ItemBuilt { city, item } if g.city(*city).owner == me => match item {
            crate::game::Production::Unit(_) => None,
            _ => match built_view_item(g, *city, *item) {
                Some(view) => Some(crate::ui::city_view::built_view(ctx, *city, view)),
                None => msg(vec![format!("{} builds", city_name(*city)), format!("{}.", item.name())]),
            },
        },
        GameEvent::ProductionBlocked { city, item } if g.city(*city).owner == me => {
            msg(vec![format!("{} cannot complete {}.", city_name(*city), item.name())])
        }
        // the human's own wonder goes through the same completion as an improvement: the city
        // view adds it (CIV.EXE 0xe676), or a plain note without animations
        GameEvent::WonderBuilt { city, wonder, owner } if *owner == me => match built_view_item(g, *city, crate::game::Production::Wonder(*wonder)) {
            Some(view) => Some(crate::ui::city_view::built_view(ctx, *city, view)),
            None => msg(vec![format!("{} builds", city_name(*city)), format!("{}.", rules::WONDERS[*wonder as usize].name)]),
        },
        GameEvent::WonderBuilt { city, wonder, owner } => Some(crate::ui::wonderscreen::wonder_screen(ctx, *wonder, &city_name(*city), *owner)),
        GameEvent::WonderObsolete { wonder, owner } if *owner == me => msg(vec![format!("{} is now obsolete.", rules::WONDERS[*wonder as usize].name)]),
        GameEvent::Disorder(c) if g.city(*c).owner == me => Some(eventpics::disorder_screen(ctx, *c)),
        GameEvent::DisorderEnded(c) if g.city(*c).owner == me => msg(vec![format!("Order restored in {}.", city_name(*c))]),
        GameEvent::Celebration(c) if g.city(*c).owner == me => Some(eventpics::celebration_screen(ctx, *c)),
        GameEvent::AdvanceDiscovered { slot, adv } if *slot == me => with_pedia(Some(crate::ui::discovery::discovery_screen(ctx, *adv)), *adv),
        // the hut's own message (HutEntered) announces it and adds the Civilopedia page
        GameEvent::AdvanceAcquired { source, .. } if source == "hut" => None,
        GameEvent::AdvanceAcquired { slot, adv, source } if *slot == me => {
            with_pedia(msg(vec![format!("You acquire {}", rules::ADVANCES[*adv as usize].name), format!("({source})")]), *adv)
        }
        GameEvent::UnitLost { slot, kind, reason, .. } if *slot == me => msg(vec![format!("{} {}.", rules::UNITS[*kind as usize].name, reason)]),
        GameEvent::CityCaptured { city, by, from, gold, adv } => {
            if *by == me {
                let mut l = vec![format!("{} captured!", city_name(*city)), format!("{} gold plundered.", gold)];
                if let Some(a) = adv {
                    l.push(format!("You acquire {}.", rules::ADVANCES[*a as usize].name));
                }
                match adv {
                    Some(a) => with_pedia(msg(l), *a),
                    None => msg(l),
                }
            } else if *from == me {
                news(vec![format!("{} captured", city_name(*city)), format!("by the {}!", civ(*by))])
            } else {
                news(vec![format!("{} capture", civ(*by)), format!("{} city of {}!", g.player(*from).adjective(), city_name(*city))])
            }
        }
        GameEvent::CityDestroyed { name, by, from, .. } => {
            if *by == me {
                msg(vec![format!("{name} destroyed!")])
            } else {
                news(vec![format!("{name} destroyed"), format!("by the {}!", civ(*by)), if *from == me { String::new() } else { format!("({} city)", g.player(*from).adjective()) }])
            }
        }
        GameEvent::CivilWar { slot, new_slot } if *slot == me || *new_slot == me => msg(vec!["Civil war!".into()]),
        GameEvent::CivDestroyed { slot, by } => news(vec![format!("{} civilization", g.civ_name(*slot)), "destroyed".into(), format!("by {}!", civ(*by))]),
        GameEvent::BarbariansLanded { near_city, target, .. } if *target == me => {
            let near = near_city.map(city_name).unwrap_or_default();
            let lines = if near.is_empty() { vec!["Barbarians land!".into()] } else { vec!["Barbarians land".into(), format!("near {near}!")] };
            Some(eventpics::invaders_screen(ctx, *near_city, lines))
        }
        GameEvent::BarbarianUprising { target, .. } if *target == me => Some(eventpics::invaders_screen(ctx, None, vec!["Barbarian uprising!".into()])),
        GameEvent::BarbarianLeaderCaptured { slot, gold } if *slot == me => msg(vec!["Barbarian leader captured!".into(), format!("{gold} gold ransom paid.")]),
        GameEvent::Pollution { city, .. } if g.city(*city).owner == me => msg(vec![format!("Pollution near {}!", city_name(*city))]),
        GameEvent::GlobalWarming => msg(vec!["Global warming!".into(), "Polar ice caps melt, coastal areas flood.".into()]),
        GameEvent::Disaster { city, kind } if g.city(*city).owner == me => {
            let lines = ctx.texts.king.section(kind.section()).map(|l| l.to_vec()).unwrap_or_default();
            let v = Vars { rplc1: city_name(*city), us: "Buildings".into(), ..Default::default() };
            msg(paragraphs(&substitute(&lines, &v)))
        }
        GameEvent::Revolution { slot } if *slot == me => msg(vec!["Revolution!".into(), "Your government falls into anarchy.".into()]),
        GameEvent::AnarchyEnded { slot } if *slot == me => None,
        GameEvent::GovernmentChanged { slot, govt } if *slot == me => Some(crate::ui::govtscreen::govt_screen(ctx, *govt)),
        GameEvent::LowFunds { slot } if *slot == me => {
            let lines = ctx.texts.king.section("TAXES").map(paragraphs).unwrap_or_else(|| vec!["Treasury is low!".into()]);
            msg(lines)
        }
        GameEvent::ImprovementSold { city, improvement, gold } if g.city(*city).owner == me => {
            msg(vec![format!("{} sold in {} for {} gold.", rules::IMPROVEMENTS[*improvement as usize].name, city_name(*city), gold)])
        }
        GameEvent::SpaceshipLaunched(s) => news(vec![format!("The {}", civ(*s)), "launch a spaceship!".into()]),
        GameEvent::SpaceshipArrived(s) => news(vec![format!("The {} spaceship", civ(*s)), "reaches Alpha Centauri!".into()]),
        GameEvent::SpaceshipLost(s) if *s == me => msg(vec!["Your spaceship is lost!".into()]),
        GameEvent::PalaceAddition(s) if *s == me => Some(crate::ui::palace::palace_addition(ctx)),
        GameEvent::PeaceTreaty { a, b } if *a == me || *b == me => {
            let other = if *a == me { *b } else { *a };
            news(vec!["Peace treaty signed".into(), format!("with the {}.", civ(other))])
        }
        GameEvent::PeaceTreaty { a, b } => news(vec![format!("The {} and the {}", civ(*a), civ(*b)), "sign a peace treaty.".into()]),
        GameEvent::WarDeclared { a, b } if *a == me || *b == me => {
            let other = if *a == me { *b } else { *a };
            news(vec![format!("The {} declare", civ(other)), "WAR on us!".into()])
        }
        GameEvent::WarDeclared { a, b } => news(vec![format!("The {} declare war", civ(*a)), format!("on the {}!", civ(*b))]),
        GameEvent::EmbassyEstablished { slot, with } if *slot == me => news(vec!["Embassy established".into(), format!("with the {}.", civ(*with))]),
        GameEvent::ContactMade { .. } => None,
        GameEvent::TradeRouteMade { city, with, bonus } if g.city(*city).owner == me => {
            msg(vec![format!("Trade route established between {} and {}.", city_name(*city), city_name(*with)), format!("Bonus: {bonus} gold.")])
        }
        GameEvent::HutEntered { slot, outcome, .. } if *slot == me => {
            let text = match outcome {
                HutOutcome::Gold(n) => vec![format!("You have discovered {n}$ of valuable metals.")],
                HutOutcome::Advance(a) => vec!["You have discovered".into(), "scrolls of ancient wisdom.".into(), format!("({})", rules::ADVANCES[*a as usize].name)],
                HutOutcome::Mercenaries(k) => vec![format!("A band of {} mercenaries", rules::UNITS[*k as usize].name), "joins your civilization.".into()],
                HutOutcome::Barbarians => vec!["You have unleashed".into(), "a horde of barbarians!".into()],
                HutOutcome::Empty => vec!["The tribe is friendly,".into(), "but has nothing to offer.".into()],
                HutOutcome::Settlers => vec!["A friendly tribe".into(), "joins your civilization.".into()],
            };
            match outcome {
                HutOutcome::Advance(a) => with_pedia(msg(text), *a),
                _ => msg(text),
            }
        }
        GameEvent::CityFounded { .. } => None,
        GameEvent::Hint(section) => {
            if !g.options.instant_advice {
                return None;
            }
            note(ctx, section, &Vars::default())
        }
        GameEvent::HintAt { section, city } => {
            if !g.options.instant_advice {
                return None;
            }
            let us = g.city_opt(*city).map(|c| c.name.clone()).unwrap_or_default();
            note(ctx, section, &Vars { us, ..Default::default() })
        }
        GameEvent::News { section, rplc1, rplc2 } => {
            let lines = ctx.texts.king.section(section).map(|l| l.to_vec()).unwrap_or_default();
            let v = Vars { rplc1: rplc1.clone(), rplc2: rplc2.clone(), us: g.human().plural().to_string(), ..Default::default() };
            news(paragraphs(&substitute(&lines, &v)))
        }
        GameEvent::UnitExpired { .. } => None,
        GameEvent::NuclearStrike { at, by, city } => {
            // The explosion plays at the viewport centre (the map view centres on the strike
            // first); the report box follows.
            let vx = (at.0 - crate::ui::mapview::VIEW_W / 2).clamp(0, crate::rules::MAP_W as i32 - crate::ui::mapview::VIEW_W);
            let vy = (at.1 - crate::ui::mapview::VIEW_H / 2).clamp(0, crate::rules::MAP_H as i32 - crate::ui::mapview::VIEW_H);
            let anim = eventpics::nuke_animation_at_tile(at.0, at.1, vx, vy);
            let where_ = city.map(city_name).filter(|n| !n.is_empty()).map(|n| format!("on {n}")).unwrap_or_else(|| format!("at {},{}", at.0, at.1));
            let text = vec![format!("The {} launch", civ(*by)), format!("a nuclear attack {where_}!")];
            Some(Box::new(Sequence { first: Some(anim), then: Some(eventpics::travelers_report(text)) }))
        }
        GameEvent::GameEnded(reason) => Some(crate::ui::endgame::end_screen(ctx, *reason)),
        _ => None,
    }
}

/// Follows the message about a new advance of the human player with the advance's
/// Civilopedia page, as CIV.EXE's grant routine does (0xc5d9; not for Future Tech).
fn with_pedia(first: Option<Box<dyn Screen>>, adv: rules::AdvanceId) -> Option<Box<dyn Screen>> {
    if adv == rules::advance_ids::FUTURE_TECH {
        return first;
    }
    let page = crate::ui::civilopedia::entry_screen(crate::ui::pedia_data::Entry::Advance(adv), 0);
    Some(match first {
        Some(f) => Box::new(Sequence { first: Some(f), then: Some(page) }),
        None => page,
    })
}

/// Plays `first` then `then` (both drawn over the parent), popping `Done` at the end.
struct Sequence {
    first: Option<Box<dyn Screen>>,
    then: Option<Box<dyn Screen>>,
}

impl Screen for Sequence {
    fn update(&mut self, _ctx: &mut UiContext) -> crate::ui::screen::Transition {
        use crate::ui::screen::Transition;
        if let Some(s) = self.first.take() {
            return Transition::Push(s);
        }
        if let Some(s) = self.then.take() {
            return Transition::Push(s);
        }
        Transition::Pop(crate::ui::screen::DialogResult::Done)
    }
    fn handle(&mut self, _ev: &crate::platform::Event, _ctx: &mut UiContext) -> crate::ui::screen::Transition {
        crate::ui::screen::Transition::None
    }
    fn draw(&mut self, _fb: &mut crate::gfx::Surface, _ctx: &mut UiContext) {}
    fn wants_cursor(&self) -> bool {
        false
    }
}

/// Produce the dialog for a decision. The dialog's result is mapped back by `reply_for`.
pub fn screen_for_decision(ctx: &mut UiContext, d: &Decision) -> Box<dyn Screen> {
    let g: &Game = ctx.game.as_ref().expect("game");
    match d {
        Decision::ChooseResearch { options, .. } => {
            let items: Vec<String> = options.iter().map(|a| rules::ADVANCES[*a as usize].name.to_string()).collect();
            Box::new(MenuDialog::new(vec!["Which discovery should".into(), "our wise men pursue?".into()], items).no_cancel())
        }
        Decision::ChooseGovernment { options, .. } => {
            let items: Vec<String> = options.iter().map(|gv| rules::GOVERNMENTS[*gv as usize].name.to_string()).collect();
            Box::new(MenuDialog::new(vec!["Select type of government:".into()], items).no_cancel())
        }
        // the map view opens the city screen once this is answered
        Decision::ProductionChoice { .. } => Box::new(Proceed),
        Decision::SellForUpkeep { city, candidates, .. } => {
            let name = g.city_opt(*city).map(|c| c.name.clone()).unwrap_or_default();
            let items: Vec<String> = candidates.iter().map(|i| rules::IMPROVEMENTS[*i as usize].name.to_string()).collect();
            Box::new(MenuDialog::new(vec![format!("Treasury empty! {name} must sell..."), "Pick one...".into()], items).no_cancel())
        }
        Decision::Diplomacy(state) => crate::ui::diplomacy_ui::audience_screen(state.clone()),
        Decision::KeepPlaying { .. } => Box::new(YesNo::new(vec!["Do you wish to".into(), "continue playing?".into()], "Yes, keep playing.", "No, I'm done.")),
    }
}

/// A decision that needs no question: closes on its first frame.
struct Proceed;

impl Screen for Proceed {
    fn update(&mut self, _ctx: &mut UiContext) -> crate::ui::screen::Transition {
        crate::ui::screen::Transition::Pop(crate::ui::screen::DialogResult::Done)
    }
    fn handle(&mut self, _ev: &crate::platform::Event, _ctx: &mut UiContext) -> crate::ui::screen::Transition {
        crate::ui::screen::Transition::None
    }
    fn draw(&mut self, _fb: &mut crate::gfx::Surface, _ctx: &mut UiContext) {}
}

/// Translate a dialog result into a decision reply.
pub fn reply_for(d: &Decision, r: &crate::ui::screen::DialogResult) -> crate::game::DecisionReply {
    use crate::game::DecisionReply;
    use crate::ui::screen::DialogResult;
    match (d, r) {
        (Decision::KeepPlaying { .. }, DialogResult::Yes | DialogResult::Choice(0)) => DecisionReply::Yes,
        (Decision::KeepPlaying { .. }, _) => DecisionReply::No,
        (_, DialogResult::Choice(i)) => DecisionReply::Choice(*i),
        (_, DialogResult::Yes) => DecisionReply::Yes,
        (_, DialogResult::No) => DecisionReply::No,
        _ => DecisionReply::Choice(0),
    }
}

/// Green CIVILIZATION NOTE for a HELP.TXT section.
fn note(ctx: &UiContext, section: &str, v: &Vars) -> Option<Box<dyn Screen>> {
    let lines = ctx.texts.help.section(section).map(|l| l.to_vec()).unwrap_or_default();
    if lines.is_empty() {
        return None;
    }
    Some(Box::new(crate::ui::widgets::NoteBox::new(paragraphs(&substitute(&lines, v)))))
}

/// Dialog for a rule error (ERROR.TXT section or plain message): red "CIVILIZATION NOTE".
pub fn error_screen(ctx: &UiContext, e: &crate::game::RuleError) -> Option<Box<dyn Screen>> {
    if let Some(sec) = e.section() {
        if let Some(lines) = ctx.texts.error.section(sec) {
            return Some(eventpics::red_note(paragraphs(lines)));
        }
    }
    let m = e.message();
    if m.is_empty() {
        None
    } else {
        Some(eventpics::red_note(vec![m.to_string()]))
    }
}

/// Whether a turn step needs the map redrawn with a pause (enemy move animation).
pub fn step_is_visible_move(step: &TurnStep) -> bool {
    matches!(step, TurnStep::AiMove { visible: true, .. } | TurnStep::Combat { visible: true, .. })
}
