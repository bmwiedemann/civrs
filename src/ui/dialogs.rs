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
use crate::ui::widgets::{MenuDialog, MessageBox};

/// Start the sound CIV.EXE plays for an event (before its dialog opens).
pub fn sound_for_event(ctx: &UiContext, ev: &GameEvent) {
    use crate::audio::Sound;
    let Some(g) = ctx.game.as_ref() else { return };
    let me = g.human_slot;
    let a = &ctx.audio;
    let mine = |c: &crate::game::CityId| g.city_opt(*c).map(|c| c.owner == me).unwrap_or(false);
    match ev {
        GameEvent::Famine(c) | GameEvent::Disorder(c) if mine(c) => a.play(Sound::Disorder),
        GameEvent::FoodExhausted { owner, .. } if *owner == me => a.play(Sound::Disorder),
        GameEvent::Celebration(c) if mine(c) => a.play(Sound::Win),
        GameEvent::CelebrationEnded(c) if mine(c) => a.play(Sound::Lose),
        // completed production has no sound of its own: only the city view plays tune 44 as
        // it adds a new building (CIV.EXE o 0x19fa1)
        GameEvent::Pollution { city, .. } | GameEvent::Meltdown { city } if mine(city) => a.play(Sound::Disorder),
        // "Sensors report a NUCLEAR WEAPONS test"; another civ's wonder gets its jingle, our own
        // only the city view's build-complete sound
        GameEvent::WonderBuilt { wonder, .. } if *wonder == rules::wonder_ids::MANHATTAN_PROJECT => a.play(Sound::Disorder),
        GameEvent::WonderBuilt { owner, .. } if *owner != me => a.national_theme(g.player(*owner).civ, false),
        GameEvent::AdvanceDiscovered { slot, .. } if *slot == me => a.national_theme(g.player(me).civ, false),
        GameEvent::CityCaptured { by, from, .. } if *by == me || *from == me => a.national_theme(g.player(*by).civ, false),
        GameEvent::BarbariansLanded { target, .. } | GameEvent::BarbarianUprising { target, .. } if *target == me => a.play(Sound::Disorder),
        GameEvent::WarDeclared { a: x, b: y } if *x == me || *y == me => a.play(Sound::WarDrums),
        GameEvent::NuclearStrike { .. } => a.play(Sound::Nuke),
        GameEvent::CivDestroyed { by, civ, .. } if *by == me => a.national_theme(*civ, true),
        GameEvent::GameEnded(reason) | GameEvent::GameEndedAgain(reason) => match reason {
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

/// Lines of a disaster message: KING.TXT *D1..*D6, or CIV.EXE's own earthquake and unrest
/// texts (o 0x1b9b9, 0x1bc99).
fn disaster_text(ctx: &UiContext, city: &str, kind: crate::game::events::Disaster) -> Vec<String> {
    use crate::game::events::Disaster;
    match kind {
        Disaster::Earthquake(i) => vec![format!("Earthquake in {city}!"), format!("{} destroyed.", rules::IMPROVEMENTS[i as usize].name)],
        Disaster::Unrest { wording, demand } => {
            let what = ["Rioting in", "Corruption in", "Scandal in"][wording.min(2) as usize];
            let demand = match demand {
                Some(i) => format!("{}.", rules::IMPROVEMENTS[i as usize].name.to_uppercase()),
                None => "lower taxes.".into(),
            };
            vec![format!("{what} {city}!"), format!("Citizens demand {demand}")]
        }
        _ => {
            let lines = kind.section().and_then(|sec| ctx.texts.king.section(sec)).map(|l| l.to_vec()).unwrap_or_default();
            let us = match kind {
                Disaster::Fire(i) => rules::IMPROVEMENTS[i as usize].name.to_string(),
                _ => String::new(),
            };
            let mut text = paragraphs(&substitute(&lines, &Vars { rplc1: city.to_string(), us, ..Default::default() }));
            // the famine text has no demand line of its own (CIV.EXE appends it, o 0x1bb6d)
            if kind == Disaster::Famine {
                text.push("Citizens demand GRANARY.".into());
            }
            text
        }
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
        GameEvent::Famine(c) if g.city_opt(*c).map(|c| c.owner == me).unwrap_or(false) => msg(vec![format!("Famine in {}!", city_name(*c)), "Population decreases.".into()]),
        // CIV.EXE 0xd0a4..0xd0ff
        GameEvent::FoodExhausted { name, owner, settlers_lost, .. } if *owner == me => {
            msg(vec!["Food storage exhausted".into(), format!("in {name}!"), if *settlers_lost { "Settlers lost." } else { "Famine feared." }.into()])
        }
        // CIV.EXE 0xd221..0xd258
        GameEvent::AqueductNeeded(c) if g.city_opt(*c).map(|c| c.owner == me).unwrap_or(false) => {
            msg(vec![format!("{} requires an AQUEDUCT", city_name(*c)), "for further growth.".into()])
        }
        // "CITY can't support\nUNIT.\n Unit Disbanded." (CIV.EXE 0xfef3)
        GameEvent::UnitDisbanded { unit_kind, city, reason } if reason == "shortage of shields" && g.city_opt(*city).map(|c| c.owner == me).unwrap_or(false) => {
            msg(vec![format!("{} can't support", city_name(*city)), format!("{}.", rules::UNITS[*unit_kind as usize].name), " Unit Disbanded.".into()])
        }
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
        // "<adv> cancels\nthe effect of\n<wonder>." (CIV.EXE 0xc796, the human's wonders)
        GameEvent::WonderObsolete { wonder, owner } if *owner == me => {
            let w = &rules::WONDERS[*wonder as usize];
            let adv = w.obsolete_by.map(|a| rules::ADVANCES[a as usize].name).unwrap_or("");
            msg(vec![format!("{adv} cancels"), "the effect of".into(), format!("{}.", w.name)])
        }
        GameEvent::BarracksObsolete { slot, adv } if *slot == me => {
            msg(vec![format!("Development of {}", rules::ADVANCES[*adv as usize].name), "makes existing Barracks".into(), "obsolete.".into()])
        }
        GameEvent::Disorder(c) if g.city(*c).owner == me => Some(eventpics::disorder_screen(ctx, *c)),
        GameEvent::DisorderEnded(c) if g.city(*c).owner == me => msg(vec![format!("Order restored in {}.", city_name(*c))]),
        GameEvent::Celebration(c) if g.city(*c).owner == me => Some(eventpics::celebration_screen(ctx, *c)),
        GameEvent::AdvanceDiscovered { slot, adv } if *slot == me => with_pedia(Some(crate::ui::discovery::discovery_screen(ctx, *adv)), *adv),
        // the hut's own message (HutEntered) announces it and adds the Civilopedia page
        GameEvent::AdvanceAcquired { source, .. } if source == "hut" => None,
        // "<Plural> acquire\n<adv> from\nthe Great Library!" (CIV.EXE 0xc394..0xc42a)
        GameEvent::AdvanceAcquired { slot, adv, source } if *slot == me && source == "Great Library" => {
            with_pedia(msg(vec![format!("{} acquire", g.player(me).plural()), format!("{} from", rules::ADVANCES[*adv as usize].name), "the Great Library!".into()]), *adv)
        }
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
        // civ_destroyed (o5 0x4fe1): "<adjective> civilization / destroyed by <plural>!"
        GameEvent::CivDestroyed { adjective, by, .. } => news(vec![format!("{adjective} civilization"), "destroyed".into(), format!("by {}!", civ(*by))]),
        // CIV.EXE 0x18fb1 (ds:0x2a70), drawn as the Defense Minister's box at (100,32); the
        // minister's portrait is not drawn here
        GameEvent::BarbariansLanded { near_city, target, .. } if *target == me => Some(Box::new(
            MessageBox::new(vec!["Barbarian raiding party".into(), format!("lands near {}!", near_city.map(city_name).unwrap_or_default()), "Citizens are alarmed.".into()]).with_pos(100, 32),
        )),
        // CIV.EXE 0x2fd3 (ds:0x1d12..0x1d6c): the variant follows the first discovery of
        // Industrialization or Gunpowder by anyone
        GameEvent::BarbarianUprising { at, target } if *target == me => {
            let what = if g.discovered_by_anyone(rules::advance_ids::INDUSTRIALIZATION) {
                "Guerrilla uprising"
            } else if g.discovered_by_anyone(rules::advance_ids::GUNPOWDER) {
                "Native unrest"
            } else {
                "Barbarian uprising"
            };
            let near = g.nearest_city_exe(at.0, at.1).0.map(city_name).unwrap_or_default();
            news(vec!["Travellers report:".into(), what.into(), format!("near {near}!")])
        }
        // KING.TXT *LEADER (combat 0x1a04c)
        GameEvent::BarbarianLeaderCaptured { slot, gold } if *slot == me => {
            let lines = ctx.texts.king.section("LEADER").map(paragraphs).unwrap_or_else(|| vec!["Barbarian leader captured!".into(), format!("{gold}$ ransom paid.")]);
            msg(lines)
        }
        GameEvent::Pollution { city, .. } if g.city(*city).owner == me => msg(vec![format!("Pollution near {}!", city_name(*city))]),
        // the explosion over the city, then "Nuclear Catastrophe\nin CITY!\nContamination
        // feared!" (CIV.EXE 0x12fab..0x1301f)
        GameEvent::Meltdown { city } if g.city(*city).owner == me => {
            let at = g.city(*city).pos();
            let vx = crate::game::Map::wrap_x(at.0 - crate::ui::mapview::VIEW_W / 2);
            let vy = (at.1 - crate::ui::mapview::VIEW_H / 2).clamp(0, crate::rules::MAP_H as i32 - crate::ui::mapview::VIEW_H);
            let anim = eventpics::nuke_animation_at_tile(at.0, at.1, vx, vy);
            let text = msg(vec!["Nuclear Catastrophe".into(), format!("in {}!", city_name(*city)), "Contamination feared!".into()]);
            Some(Box::new(Sequence { first: Some(anim), then: text }))
        }
        // ds:0x3b6a and ds:0x1df8, shown by the newspaper (overlay 21) in CIV.EXE
        GameEvent::GlobalWarming => msg(vec!["Global temperature".into(), "rises! Icecaps melt.".into(), "Severe Drought.".into()]),
        GameEvent::GlobalWarmingFeared => msg(vec!["Scientists alarmed by".into(), "rising temperatures.".into(), "Global Warming feared!".into()]),
        // the owner reads it all, anyone else who sees the city only the first line (CIV.EXE
        // o 0x1bf3b)
        GameEvent::Disaster { city, kind } if g.city_opt(*city).is_some_and(|c| c.owner == me || g.visible_to(me, c.x, c.y)) => {
            let lines = disaster_text(ctx, &city_name(*city), *kind);
            if g.city(*city).owner == me {
                msg(lines)
            } else {
                msg(lines.into_iter().take(1).collect())
            }
        }
        GameEvent::CityDefected { city, admired, from } if *from == me || g.city_opt(*city).is_some_and(|c| g.visible_to(me, c.x, c.y)) => {
            msg(vec![format!("Residents of {}", city_name(*city)), format!("admire the prosperity of {}.", city_name(*admired))])
        }
        // "The <Plural> are\nrevolting! Citizens\ndemand new govt." (CIV.EXE 0x6f88, newspaper)
        GameEvent::Revolution { slot } if *slot == me => msg(vec![format!("The {} are", g.player(*slot).plural()), "revolting! Citizens".into(), "demand new govt.".into()]),
        GameEvent::GovernmentCollapsed { slot, city } if *slot == me => msg(vec!["Discontented citizens of".into(), format!("{} revolt:", city_name(*city)), "Government Collapses!".into()]),
        GameEvent::AnarchyEnded { slot } if *slot == me => None,
        GameEvent::GovernmentChanged { slot, govt } if *slot == me => Some(crate::ui::govtscreen::govt_screen(ctx, *govt)),
        GameEvent::LowFunds { slot } if *slot == me => {
            let lines = ctx.texts.king.section("TAXES").map(paragraphs).unwrap_or_else(|| vec!["Treasury is low!".into()]);
            msg(lines)
        }
        // "CITY\ncan't maintain\nBUILDING." (CIV.EXE 0x13154)
        // "ADJ UNIT unit\nbribed by PLURAL!" (CIV.EXE o 0x1dd83)
        GameEvent::UnitBribed { kind, from, by, .. } if *from == me => {
            msg(vec![format!("{} {} unit", g.player(*from).adjective(), rules::UNITS[*kind as usize].name), format!("bribed by {}!", g.player(*by).plural())])
        }
        // CIV.EXE 0xd724..0xd761: "Population decrease\nin " + name + ".\n" in a box at (100, 80)
        GameEvent::PopulationDecrease(city) if g.city_opt(*city).map(|c| c.owner == me).unwrap_or(false) => {
            msg(vec!["Population decrease".into(), format!("in {}.", city_name(*city))])
        }
        GameEvent::CantMaintain { city, improvement } if g.city(*city).owner == me => {
            msg(vec![city_name(*city), "can't maintain".into(), format!("{}.", rules::IMPROVEMENTS[*improvement as usize].name)])
        }
        GameEvent::ImprovementSold { city, improvement, gold } if g.city(*city).owner == me => {
            msg(vec![format!("{} sold in {} for {} gold.", rules::IMPROVEMENTS[*improvement as usize].name, city_name(*city), gold)])
        }
        // "<Plural> LAUNCH space ship!" / "Estimate arrival in <year>.", then the ship
        // (CIV.EXE o 0x18b38)
        GameEvent::SpaceshipLaunched(s) => {
            let (first, then) = crate::ui::spaceship_ui::launch_screens(ctx, *s);
            Some(Box::new(Sequence { first: Some(first), then }))
        }
        GameEvent::SpaceshipArrived(s) => news(vec![format!("The {} spaceship", civ(*s)), "reaches Alpha Centauri!".into()]),
        GameEvent::SpaceshipLost(s) if *s == me => msg(vec!["Your spaceship is lost!".into()]),
        // CIV.EXE o 0x1842b, for every part an AI civ adds
        GameEvent::SpaceshipPartBuilt(s) if *s != me => {
            let (first, then) = crate::ui::spaceship_ui::ai_part_screens(ctx, *s);
            Some(Box::new(Sequence { first: Some(first), then }))
        }
        // "<Title> <Leader>\nplans retirement\nin 20 years." (CIV.EXE 0x2a28, overlay 21)
        GameEvent::RetirementWarning(s) if *s == me => {
            let p = g.player(*s);
            news(vec![format!("{} {}", p.title(), p.leader), "plans retirement".into(), "in 20 years.".into()])
        }
        // CIV.EXE 0x14364
        GameEvent::SpaceshipCancelled { slot, launched } => {
            msg(vec![format!("{} spaceship", g.player(*slot).adjective()), if *launched { "returns to Earth.".into() } else { "construction cancelled.".into() }])
        }
        GameEvent::PalaceAddition(s) if *s == me => Some(crate::ui::palace::palace_addition(ctx)),
        GameEvent::HistorianReport { historian, category, rows } => Some(Box::new(crate::ui::report_world::Historian::new(*historian, *category, rows.clone()))),
        // the computer players' treaties and wars, for the human with an embassy (CIV.EXE
        // 0x15247, 0x152f8, 0x14aa4, at (100, 80))
        GameEvent::PeaceTreaty { a, b } => msg(vec![format!("{} and", civ(*a)), format!("{} sign", civ(*b)), "a peace treaty.".into()]),
        GameEvent::WarDeclared { a, b } => msg(vec![format!("{} and", civ(*a)), format!("{} declare", civ(*b)), "war on each other.".into()]),
        GameEvent::TreatyCancelled { a, b } => msg(vec![format!("{} and", civ(*a)), format!("{} cancel", civ(*b)), "peace treaty.".into()]),
        GameEvent::WarEnded { a, b } => msg(vec!["The war between".into(), format!("the {} and the", civ(*a)), format!("{} has ended.", civ(*b))]),
        // fights (CIV.EXE 0x19e98, 0x19f2c)
        GameEvent::SneakAttack { by, .. } => msg(vec!["Sneak attack by".into(), format!("{} forces!", g.player(*by).adjective())]),
        GameEvent::TreatyBroken { by, victim } => msg(vec![format!("{} cancel", civ(*victim)), "peace treaty".into(), format!("with {}.", civ(*by))]),
        GameEvent::Schism(s) => note(ctx, "SCHISM", &Vars { them: civ(*s).to_string(), ..Default::default() }),
        GameEvent::EmbassyEstablished { slot, with } if *slot == me => news(vec!["Embassy established".into(), format!("with the {}.", civ(*with))]),
        GameEvent::ContactMade { .. } => None,
        // CIV.EXE 0x147f7
        GameEvent::TradeRouteMade { city, with, bonus, goods } if g.city(*city).owner == me => msg(vec![
            format!("{} caravan from {}", crate::game::turn::CARAVAN_GOODS[*goods as usize & 7], city_name(*city)),
            format!("arrives in {}", city_name(*with)),
            "Trade route established".into(),
            format!("Revenue: ${bonus}."),
        ]),
        // the boxes of hut_outcome (CIV.EXE 0x0978d, ds:0x212c..0x21e6)
        GameEvent::HutEntered { slot, outcome, .. } if *slot == me => {
            let found = |what: &[&str]| std::iter::once("You have discovered").chain(what.iter().copied()).map(String::from).collect::<Vec<_>>();
            let text = match outcome {
                HutOutcome::Gold(n) => found(&["valuable metal deposits", &format!("worth {n}$")]),
                HutOutcome::Advance(_) | HutOutcome::EmptyScrolls => found(&["scrolls of ancient wisdom."]),
                HutOutcome::Mercenaries(_) | HutOutcome::Settlers => found(&["a friendly tribe of", "skilled mercenaries."]),
                HutOutcome::Barbarians => vec!["You have unleashed".into(), "a horde of barbarians!".into()],
                HutOutcome::AdvancedTribe(_) | HutOutcome::Empty => found(&["an advanced tribe."]),
            };
            match outcome {
                HutOutcome::Advance(a) => with_pedia(msg(text), *a),
                _ => msg(text),
            }
        }
        GameEvent::CityFounded { .. } => None,
        // CIV.EXE 0x0bfde..0x0c0f2 (ds:0x2327..0x2369): the units and buildings the advance
        // allows, the box at (8, 168 - 8 * their number); the advisor's portrait is not drawn
        GameEvent::ScienceAdvice { slot, adv } if *slot == me => {
            let mut lines = vec!["Science Advisor:".into(), "I recommend we".into(), format!("develop {}.", rules::ADVANCES[*adv as usize].name)];
            let units = rules::UNITS.iter().filter(|u| u.prereq == Some(*adv)).map(|u| u.name);
            // improvements 0..23: the SS Module (24) is left out
            let buildings = rules::IMPROVEMENTS.iter().take(24).filter(|i| i.prereq == Some(*adv)).map(|i| i.name);
            let items: Vec<String> = units.chain(buildings).map(|n| format!("-{n}")).collect();
            let n = items.len() as i32;
            if n > 0 {
                lines.push("So we can build:".into());
                lines.extend(items);
            }
            Some(Box::new(MessageBox::new(lines).with_pos(8, 168 - 8 * n)))
        }
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
            let vx = crate::game::Map::wrap_x(at.0 - crate::ui::mapview::VIEW_W / 2);
            let vy = (at.1 - crate::ui::mapview::VIEW_H / 2).clamp(0, crate::rules::MAP_H as i32 - crate::ui::mapview::VIEW_H);
            let anim = eventpics::nuke_animation_at_tile(at.0, at.1, vx, vy);
            let where_ = city.map(city_name).filter(|n| !n.is_empty()).map(|n| format!("on {n}")).unwrap_or_else(|| format!("at {},{}", at.0, at.1));
            let text = vec![format!("The {} launch", civ(*by)), format!("a nuclear attack {where_}!")];
            Some(Box::new(Sequence { first: Some(anim), then: Some(eventpics::travelers_report(text)) }))
        }
        GameEvent::SdiProtects { city } => Some(Box::new(MessageBox::new(vec![format!("SDI protects {}.", city_name(*city))]).with_pos(80, 100))),
        GameEvent::GameEnded(reason) => Some(crate::ui::endgame::end_screen(ctx, *reason)),
        GameEvent::GameEndedAgain(reason) => Some(crate::ui::endgame::end_again_screen(ctx, *reason)),
        // a computer Diplomat's deeds are announced to the human whoever was involved
        // (CIV.EXE o 0x1d817 / 0x1d9a3, popups at (80,80))
        GameEvent::DiplomatStole { by, adv, .. } => {
            Some(Box::new(MessageBox::new(vec![format!("{} steal", g.player(*by).plural()), format!("{}.", rules::ADVANCES[*adv as usize].name)]).with_pos(80, 80)))
        }
        GameEvent::CitySabotaged { city, improvement, production, .. } => {
            let first = match improvement {
                Some(i) => format!("{} destroyed", rules::IMPROVEMENTS[*i as usize].name),
                None => format!("{production} production sabotaged"),
            };
            Some(Box::new(MessageBox::new(vec![first, format!("in {}.", city_name(*city))]).with_pos(80, 80)))
        }
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
        // choose_research (CIV.EXE 0x0c103..0x0c23b, ds:0x236c..0x23d5) at (80,64)
        Decision::ChooseResearch { slot, options } => {
            let p = g.player(*slot);
            let items: Vec<String> = options
                .iter()
                .map(|a| if *a == rules::advance_ids::FUTURE_TECH { format!("Future Technology {}", p.future_techs + 1) } else { rules::ADVANCES[*a as usize].name.to_string() })
                .collect();
            let who = if p.knows(rules::advance_ids::INVENTION) { "scientists" } else { "wise men" };
            let head = vec!["Which discovery should our".into(), format!("{who} be pursuing, sire?"), "Pick one...".into()];
            Box::new(MenuDialog::new(head, items).with_pos(80, 64).with_help().no_cancel())
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
        // *FAME (popup_menu at (80, 80), 0x2bf9): "No, I'm done." first
        Decision::KeepPlaying { .. } => {
            let lines = ctx.texts.king.section("FAME").map(|l| l.to_vec()).unwrap_or_default();
            let m = crate::assets::text::split_menu(&lines);
            Box::new(MenuDialog::new(m.body, m.choices).with_pos(80, 80).no_cancel())
        }
        // CIV.EXE o 0x1855b / 0x18591 at (100,80) over the ship (Esc there wastes the part;
        // here a type must be chosen)
        Decision::SpaceshipPart { slot, kind } => {
            let items: Vec<String> = if *kind == 1 { vec!["Propulsion".into(), "Fuel".into()] } else { vec!["Habitation".into(), "Life Support".into(), "Solar Panel".into()] };
            let menu = Box::new(MenuDialog::new(vec!["Type?".into()], items).with_pos(100, 80).no_cancel());
            crate::ui::spaceship_ui::part_question(ctx, *slot, menu)
        }
        // the ship with its LAUNCH button (CIV.EXE o 0x18842); confirming answers Choice(1)
        Decision::SpaceshipLaunch { slot } => crate::ui::spaceship_ui::launch_prompt(ctx, *slot),
        // steal_advance (CIV.EXE 0x14550, ds:0x28f6) at (80,32)
        Decision::StealAdvance { options, .. } => {
            let items: Vec<String> = options.iter().map(|a| rules::ADVANCES[*a as usize].name.to_string()).collect();
            Box::new(MenuDialog::new(vec!["Select one...".into()], items).with_pos(80, 32).no_cancel())
        }
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
        (Decision::KeepPlaying { .. }, DialogResult::Yes | DialogResult::Choice(1)) => DecisionReply::Yes,
        (Decision::KeepPlaying { .. }, _) => DecisionReply::No,
        // the audience screen has run the whole audience itself
        (Decision::Diplomacy(_), _) => DecisionReply::Cancel,
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
