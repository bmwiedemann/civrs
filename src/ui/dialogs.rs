//! Mapping of engine events / decisions to dialogs and event screens: discovery, wonder,
//! government, civil disorder, celebration, barbarian landings, nuclear strikes, diplomacy
//! audiences, end of game, "Travelers report" news boxes, disasters and plain message boxes.

use crate::assets::text::{paragraphs, substitute, Vars};
use crate::game::turn::TurnStep;
use crate::game::{Decision, Game, GameEvent, HutOutcome};
use crate::rules;
use crate::ui::context::UiContext;
use crate::ui::eventpics;
use crate::audio::{Sound, Tune};
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::{MenuDialog, MessageBox, SPEAKER_DEFENSE, SPEAKER_DIPLOMATS, SPEAKER_DOMESTIC, SPEAKER_FOREIGN, SPEAKER_SCIENCE, SPEAKER_SPIES, SPEAKER_TRAVELERS};

/// The tune CIV.EXE plays with an event's message, started as the message opens and faded
/// (`PlayTune(1)`) as it closes. Only these call sites react to game events; everything else
/// (unit moves, pollution, barbarians, wars between computer players, the end of a
/// civilization, a computer's wonder from its own production) is silent.
pub fn event_tune(g: &Game, ev: &GameEvent) -> Option<Tune> {
    let me = g.human_slot;
    let mine = |c: &crate::game::CityId| g.city_opt(*c).is_some_and(|c| c.owner == me);
    // no celebration messages under Anarchy (0x129e4, 0x12b4d)
    let ruled = g.player(me).govt != rules::Govt::Anarchy;
    let theme = |s: crate::game::Slot| Tune::Theme { civ: g.player(s).civ, long: false };
    match ev {
        // "Food storage exhausted" (0xd0e3), civil disorder (0x12641), "Government Collapses!"
        // (0x127ce), "Nuclear Catastrophe" after the explosion (0x12fe9)
        GameEvent::FoodExhausted { owner, .. } if *owner == me => Some(Tune::Sound(Sound::Disorder)),
        GameEvent::Disorder(c) | GameEvent::DisorderOngoing { city: c, .. } | GameEvent::Meltdown { city: c } if mine(c) => Some(Tune::Sound(Sound::Disorder)),
        GameEvent::GovernmentCollapsed { slot, .. } if *slot == me => Some(Tune::Sound(Sound::Disorder)),
        GameEvent::Celebration(c) if mine(c) && ruled => Some(Tune::Sound(Sound::Win)),
        GameEvent::CelebrationEnded(c) if mine(c) && ruled => Some(Tune::Sound(Sound::Lose)),
        // "Sensors report a NUCLEAR WEAPONS test", whoever built it (0xe566)
        GameEvent::WonderBuilt { wonder, .. } if *wonder == rules::wonder_ids::MANHATTAN_PROJECT => Some(Tune::Sound(Sound::Disorder)),
        // the owner's jingle (0x3e9e, 0xbe14, 0x140d3)
        GameEvent::WonderGranted { owner, .. } => Some(theme(*owner)),
        GameEvent::AdvanceDiscovered { slot, .. } if *slot == me => Some(theme(me)),
        GameEvent::CityCaptured { by, from, .. } if *by == me || *from == me => Some(theme(*by)),
        _ => None,
    }
}

/// A screen with a tune: it starts as the screen opens and fades out (`PlayTune(1)`) as it
/// closes, as nearly every CIV.EXE call site does.
pub struct Tuned {
    tune: Tune,
    inner: Box<dyn Screen>,
}

pub fn tuned(tune: Tune, inner: Box<dyn Screen>) -> Box<dyn Screen> {
    Box::new(Tuned { tune, inner })
}

/// `s` with the tune, when both are there.
fn with_tune(tune: Option<Tune>, s: Option<Box<dyn Screen>>) -> Option<Box<dyn Screen>> {
    match (tune, s) {
        (Some(t), Some(s)) => Some(tuned(t, s)),
        (_, s) => s,
    }
}

impl Tuned {
    fn forward(&mut self, t: Transition, ctx: &mut UiContext) -> Transition {
        match t {
            Transition::Pop(r) => {
                ctx.audio.fade_out();
                Transition::Pop(r)
            }
            Transition::Replace(mut s) => {
                self.inner.on_exit(ctx);
                s.on_enter(ctx);
                self.inner = s;
                Transition::None
            }
            other => other,
        }
    }
}

impl Screen for Tuned {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        ctx.audio.play_tune(self.tune);
        self.inner.on_enter(ctx);
    }
    fn on_exit(&mut self, ctx: &mut UiContext) {
        self.inner.on_exit(ctx);
    }
    fn handle(&mut self, ev: &crate::platform::Event, ctx: &mut UiContext) -> Transition {
        let t = self.inner.handle(ev, ctx);
        self.forward(t, ctx)
    }
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        let t = self.inner.update(ctx);
        self.forward(t, ctx)
    }
    fn resume(&mut self, result: DialogResult, ctx: &mut UiContext) -> Transition {
        let t = self.inner.resume(result, ctx);
        self.forward(t, ctx)
    }
    fn draw(&mut self, fb: &mut crate::gfx::Surface, ctx: &mut UiContext) {
        self.inner.draw(fb, ctx);
    }
    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        self.inner.palette(ctx)
    }
    fn draws_parent(&self) -> bool {
        self.inner.draws_parent()
    }
    fn wants_cursor(&self) -> bool {
        self.inner.wants_cursor()
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
/// The event's tune (`event_tune`) plays over it.
pub fn screen_for_event(ctx: &mut UiContext, ev: &GameEvent) -> Option<Box<dyn Screen>> {
    let mut tune = event_tune(ctx.game.as_ref()?, ev);
    let s = event_message(ctx, ev, &mut tune);
    with_tune(tune, s)
}

/// The screen of an event; an arm that plays the tune over only a part of it takes `tune`.
fn event_message(ctx: &mut UiContext, ev: &GameEvent, tune: &mut Option<Tune>) -> Option<Box<dyn Screen>> {
    let g = ctx.game.as_ref()?;
    let me = g.human_slot;
    let msg = |lines: Vec<String>| -> Option<Box<dyn Screen>> { Some(Box::new(MessageBox::new(lines))) };
    let news = |lines: Vec<String>| -> Option<Box<dyn Screen>> { Some(eventpics::travelers_report(lines)) };
    // popup_menu(text, 100, 80) with a speaker (ds:0x2f9e)
    let speaker = |s: u8, lines: Vec<String>| -> Option<Box<dyn Screen>> { Some(Box::new(MenuDialog::new(lines, Vec::new()).with_pos(100, 80).with_speaker(s))) };
    // the same at popup_menu(text, x, y)
    let speaker_at = |s: u8, lines: Vec<String>, x: i32, y: i32| -> Option<Box<dyn Screen>> { Some(Box::new(MenuDialog::new(lines, Vec::new()).with_pos(x, y).with_speaker(s))) };
    // overlay 21, named after `city` (None: the human's city, the -1 argument)
    let paper = |lines: Vec<String>, city: Option<crate::game::CityId>| -> Option<Box<dyn Screen>> { Some(crate::ui::newspaper::newspaper(lines, city)) };
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
        // CIV.EXE 0xd0a4..0xd0ff, the newspaper of the city
        GameEvent::FoodExhausted { city, name, owner, settlers_lost } if *owner == me => {
            paper(vec!["Food storage exhausted".into(), format!("in {name}!"), if *settlers_lost { "Settlers lost." } else { "Famine feared." }.into()], Some(*city))
        }
        // the Domestic Advisor at (100, 80) (draw_sidebar_status 0x370a..0x3818)
        GameEvent::PopulationMilestone { figure } => speaker(
            SPEAKER_DOMESTIC,
            vec!["The population of the".into(), format!("fertile {} empire now", g.player(me).adjective()), format!("exceeds {figure}00,000 citizens.")],
        ),
        // the Domestic Advisor at (100, 80) (CIV.EXE 0xd221..0xd258)
        GameEvent::AqueductNeeded(c) if g.city_opt(*c).map(|c| c.owner == me).unwrap_or(false) => {
            speaker(SPEAKER_DOMESTIC, vec![format!("{} requires an AQUEDUCT", city_name(*c)), "for further growth.".into()])
        }
        // "CITY can't support\nUNIT.\n Unit Disbanded." from the Defense Minister at (100, 80);
        // the last line is an item (CIV.EXE 0xfef3..0xff5d)
        GameEvent::UnitDisbanded { unit_kind, city, reason } if reason == "shortage of shields" && g.city_opt(*city).map(|c| c.owner == me).unwrap_or(false) => {
            let lines = vec![format!("{} can't support", city_name(*city)), format!("{}.", rules::UNITS[*unit_kind as usize].name)];
            Some(Box::new(MenuDialog::new(lines, vec![" Unit Disbanded.".into()]).with_pos(100, 80).with_speaker(SPEAKER_DEFENSE)))
        }
        GameEvent::UnitDisbanded { unit_kind, city, reason } if g.city_opt(*city).map(|c| c.owner == me).unwrap_or(false) => {
            msg(vec![format!("{} in {} disbanded", rules::UNITS[*unit_kind as usize].name, city_name(*city)), format!("({reason})")])
        }
        GameEvent::ItemBuilt { city, item } if g.city(*city).owner == me => match item {
            // "CITY builds UNIT." only for Settlers, Diplomats and Caravans, from the Defense
            // Minister at (80, 80) (CIV.EXE 0xe3dc..0xe45e)
            crate::game::Production::Unit(k) if *k == rules::unit_ids::SETTLERS || *k >= rules::unit_ids::DIPLOMAT => {
                speaker_at(SPEAKER_DEFENSE, vec![format!("{} builds {}.", city_name(*city), item.name())], 80, 80)
            }
            crate::game::Production::Unit(_) => None,
            // the city view adds it, else the city's newspaper (CIV.EXE 0xe62f..0xe6e9)
            _ => match built_view_item(g, *city, *item) {
                Some(view) => Some(crate::ui::city_view::built_view(ctx, *city, view)),
                None => paper(vec![format!("{} builds", city_name(*city)), format!("{}.", item.name())], Some(*city)),
            },
        },
        GameEvent::ProductionBlocked { city, item } if g.city(*city).owner == me => {
            msg(vec![format!("{} cannot complete {}.", city_name(*city), item.name())])
        }
        // a wonder goes through the same completion as an improvement (CIV.EXE 0xe5d6..0xe6e9):
        // the human's is added by the city view, else (and every computer wonder) the
        // newspaper of the city says "CITY builds\nWONDER."; the Manhattan Project's
        // "Sensors report a\nNUCLEAR WEAPONS test\nnear CITY!" newspaper comes first (0xe52d)
        GameEvent::WonderBuilt { city, wonder, owner } => {
            let builds = match built_view_item(g, *city, crate::game::Production::Wonder(*wonder)).filter(|_| *owner == me) {
                Some(view) => Some(crate::ui::city_view::built_view(ctx, *city, view)),
                None => paper(vec![format!("{} builds", city_name(*city)), format!("{}.", rules::WONDERS[*wonder as usize].name)], Some(*city)),
            };
            if *wonder == rules::wonder_ids::MANHATTAN_PROJECT {
                // the tune plays over the test's newspaper only (0xe566..0xe57e)
                let test = paper(vec!["Sensors report a".into(), "NUCLEAR WEAPONS test".into(), format!("near {}!", city_name(*city))], None);
                let test = with_tune(tune.take(), test);
                Some(Box::new(Sequence { first: test, then: builds }))
            } else {
                builds
            }
        }
        // "CITY (Adjective)\nbuilds WONDER." at (100, 80) from the Foreign Minister with an
        // embassy, else the Travelers, then the wonder's Civilopedia page (CIV.EXE
        // 0x3e07..0x3b93)
        GameEvent::WonderGranted { city, wonder, owner } => {
            let lines = vec![format!("{} ({})", city_name(*city), g.player(*owner).adjective()), format!("builds {}.", rules::WONDERS[*wonder as usize].name)];
            let who = if g.player(me).has_embassy(*owner) { SPEAKER_FOREIGN } else { SPEAKER_TRAVELERS };
            let page = crate::ui::civilopedia::entry_screen(crate::ui::pedia_data::Entry::Wonder(*wonder), 0);
            Some(Box::new(Sequence { first: speaker(who, lines), then: Some(page) }))
        }
        // "<adv> cancels\nthe effect of\n<wonder>." (CIV.EXE 0xc796, the human's wonders, a
        // newspaper)
        GameEvent::WonderObsolete { wonder, owner } if *owner == me => {
            let w = &rules::WONDERS[*wonder as usize];
            let adv = w.obsolete_by.map(|a| rules::ADVANCES[a as usize].name).unwrap_or("");
            paper(vec![format!("{adv} cancels"), "the effect of".into(), format!("{}.", w.name)], None)
        }
        // the Defense Minister at (80, 100) (CIV.EXE 0xc6ea..0xc732)
        GameEvent::BarracksObsolete { slot, adv } if *slot == me => {
            speaker_at(SPEAKER_DEFENSE, vec![format!("Development of {}", rules::ADVANCES[*adv as usize].name), "makes existing Barracks".into(), "obsolete.".into()], 80, 100)
        }
        GameEvent::Disorder(c) if g.city(*c).owner == me => Some(eventpics::disorder_screen(ctx, *c)),
        // the riot message without its animation, from the Domestic Advisor at (100, 80)
        // (CIV.EXE 0x126a5..0x126b7)
        GameEvent::DisorderOngoing { city, .. } if g.city(*city).owner == me => speaker(SPEAKER_DOMESTIC, eventpics::disorder_lines(ctx, *city)),
        // "Order restored\nin CITY." (CIV.EXE 0x12880..0x128bd)
        GameEvent::DisorderEnded(c) if g.city(*c).owner == me => speaker(SPEAKER_DOMESTIC, vec!["Order restored".into(), format!("in {}.", city_name(*c))]),
        // not under Anarchy (CIV.EXE 0x129e4)
        GameEvent::Celebration(c) if g.city(*c).owner == me && g.human().govt != rules::Govt::Anarchy => Some(eventpics::celebration_screen(ctx, *c)),
        // "'We love the TITLE'\ncelebration canceled\nin CITY." in the city's newspaper, not
        // under Anarchy (CIV.EXE 0x12b23..0x12bbd)
        GameEvent::CelebrationEnded(c) if g.city(*c).owner == me && g.human().govt != rules::Govt::Anarchy => paper(
            vec![format!("'We love the {}'", g.human().title()), "celebration canceled".into(), format!("in {}.", city_name(*c))],
            Some(*c),
        ),
        // the discovery picture with Animations on (and VGA), except for Future Technology;
        // else "ADJ wise men|scientists\ndiscover the secret\nof ADV!" in the newspaper
        // (CIV.EXE 0xbc84..0xbe22 -> 0xbf6b)
        GameEvent::AdvanceDiscovered { slot, adv } if *slot == me => {
            let first = if *adv != rules::advance_ids::FUTURE_TECH && g.options.animations {
                crate::ui::discovery::discovery_screen(ctx, *adv)
            } else {
                let p = g.human();
                let who = if p.knows(rules::advance_ids::INVENTION) { "scientists" } else { "wise men" };
                let name = rules::ADVANCES[*adv as usize].name;
                let what = if *adv == rules::advance_ids::FUTURE_TECH { format!("of {name} {}!", p.future_techs) } else { format!("of {name}!") };
                crate::ui::newspaper::newspaper(vec![format!("{} {who}", p.adjective()), "discover the secret".into(), what], None)
            };
            with_pedia(Some(first), *adv)
        }
        // the hut's own message (HutEntered) announces it and adds the Civilopedia page
        GameEvent::AdvanceAcquired { source, .. } if source == "hut" => None,
        // "<Plural> acquire\n<adv> from\nthe Great Library!" from the Science Advisor at
        // (100, 80) (CIV.EXE 0xc394..0xc42a, 0xc4f0)
        GameEvent::AdvanceAcquired { slot, adv, source } if *slot == me && source == "Great Library" => {
            with_pedia(speaker(SPEAKER_SCIENCE, vec![format!("{} acquire", g.player(me).plural()), format!("{} from", rules::ADVANCES[*adv as usize].name), "the Great Library!".into()]), *adv)
        }
        // grant_advance 0xc394..0xc507 from the Spies at (100,80) (0xc4e8): what the advance
        // allows follows, one line each with a leading space (ds:0x2404, 0x2408)
        GameEvent::ForeignAdvance { slot, adv, source } if *slot != me => {
            let name = rules::ADVANCES[*adv as usize].name;
            let mut l = match *source {
                s if s == 0 || s == *slot as i8 => vec![format!("{} develop", g.player(*slot).plural()), format!("{name}!")],
                s => {
                    let from = if s == -2 { "Great Library".to_string() } else { g.player(s as u8).plural().to_string() };
                    vec![format!("{} acquire", g.player(*slot).plural()), format!("{name} from"), format!("the {from}!")]
                }
            };
            l.extend(rules::IMPROVEMENTS.iter().take(24).filter(|i| i.prereq == Some(*adv)).map(|i| format!(" {}", i.name)));
            l.extend(rules::UNITS.iter().filter(|u| u.prereq == Some(*adv)).map(|u| format!(" {}", u.name)));
            speaker(SPEAKER_SPIES, l)
        }
        GameEvent::AdvanceAcquired { slot, adv, source } if *slot == me => {
            with_pedia(msg(vec![format!("You acquire {}", rules::ADVANCES[*adv as usize].name), format!("({source})")]), *adv)
        }
        GameEvent::UnitLost { slot, kind, reason, .. } if *slot == me => msg(vec![format!("{} {}.", rules::UNITS[*kind as usize].name, reason)]),
        // capture_city (CIV.EXE 0x14014..0x14130): with the human on either side "PLURAL
        // capture\nCITY. N gold\npieces plundered."; with Animations (ds:0x19c0 bit 3) in a
        // box over the captured city's view (overlay 19 +0 with -2) while the captor's
        // invaders march past (overlay 19 +0x167b), else in the city's newspaper. A computer
        // captor's theft follows as "PLURAL take\nADVANCE." (steal_advance 0x145a6). Between
        // computer players only a human embassy with either side tells, as "Spies report:"
        // at (100, 80) (0x1416d..0x1422a).
        GameEvent::CityCaptured { city, by, from, gold, adv } if *by == me || *from == me => {
            let lines = vec![format!("{} capture", g.player(*by).plural()), format!("{}. {} gold", city_name(*city), gold), "pieces plundered.".into()];
            let news = if g.options.animations {
                Some(eventpics::invaders_screen(ctx, *by, g.city_opt(*city).map(|_| *city), lines))
            } else {
                paper(lines, Some(*city))
            };
            // the jingle fades before the theft's message (0x1413c)
            let news = with_tune(tune.take(), news);
            match adv {
                Some(a) if *from == me => Some(Box::new(Sequence {
                    first: news,
                    then: Some(Box::new(MessageBox::new(vec![format!("{} take", g.player(*by).plural()), format!("{}.", rules::ADVANCES[*a as usize].name)]).with_pos(80, 32))),
                })),
                _ => news,
            }
        }
        // the Spies' box over its own "Spies report:" line (0x14190)
        GameEvent::CityCaptured { city, by, from, .. } if g.player(me).has_embassy(*by) || g.player(me).has_embassy(*from) => speaker(
            SPEAKER_SPIES,
            vec!["Spies report:".into(), format!("{} capture", g.player(*by).plural()), format!("the {} city", g.player(*from).adjective()), format!("of {}.", city_name(*city))],
        ),
        GameEvent::CityRebels { city, from, to } if *from == me || *to == me || g.player(me).has_embassy(*from) || g.player(me).has_embassy(*to) => {
            let lines = if *to == 0 {
                vec![format!("{} declare", g.player(*from).plural()), "independence in".into(), city_name(*city)]
            } else {
                vec![format!("{} rebel!", g.player(*from).plural()), "Civil War in".into(), format!("{}.", city_name(*city)), format!("{} influence", g.player(*to).adjective()), "suspected.".into()]
            };
            Some(Box::new(MessageBox::new(lines).with_pos(100, 32)))
        }
        GameEvent::CityDestroyed { name, by, from, .. } => {
            if *by == me {
                msg(vec![format!("{name} destroyed!")])
            } else if *from == me {
                news(vec![format!("{name} destroyed"), format!("by the {}!", civ(*by))])
            } else if g.player(me).has_embassy(*by) || g.player(me).has_embassy(*from) {
                // between computer players: "Spies report:" for a human embassy with either
                // side, from the Spies (CIV.EXE 0x1416d..0x1422a, ds:0x28a1)
                speaker(SPEAKER_SPIES, vec!["Spies report:".into(), format!("{} destroy", g.player(*by).plural()), format!("the {} city", g.player(*from).adjective()), format!("of {name}.")])
            } else {
                None
            }
        }
        // KING.TXT *ESCHISM, shown to everyone (civil_war o 0x14dce: $US / $THEM the two
        // adjectives). For the human's own civil war civil_war picks *SCHISM (o 0x14dd6),
        // which KING.TXT lacks (load_text_entry 0x1ee63 returns -1, the box would be empty),
        // but its one caller never splits the human (0x13db4): dead code, *ESCHISM always.
        GameEvent::CivilWar { slot, new_slot } => {
            let v = Vars { us: g.player(*slot).adjective().to_string(), them: g.player(*new_slot).adjective().to_string(), ..Default::default() };
            let lines = ctx.texts.king.section("ESCHISM").map(|l| paragraphs(&substitute(l, &v))).unwrap_or_else(|| vec!["Civil war!".into()]);
            // the Foreign Minister at (80, 80) (o 0x14de7)
            speaker_at(SPEAKER_FOREIGN, lines, 80, 80)
        }
        // civ_destroyed (o5 0x5029..0x50a7): "<Adjective>\ncivilization\ndestroyed\nby
        // <Plural>!" at (100, 80) from the speaker of the event
        GameEvent::CivDestroyedReport { adjective, by, speaker: s, .. } => {
            speaker(*s, vec![adjective.clone(), "civilization".into(), "destroyed".into(), format!("by {}!", g.player(*by).plural())])
        }
        GameEvent::CivDestroyed { slot, adjective, by, .. } => {
            let s = if *by == me {
                SPEAKER_DEFENSE
            } else if g.player(me).has_embassy(*slot) {
                SPEAKER_FOREIGN
            } else {
                SPEAKER_TRAVELERS
            };
            speaker(s, vec![adjective.clone(), "civilization".into(), "destroyed".into(), format!("by {}!", g.player(*by).plural())])
        }
        // set_government (CIV.EXE 0x14f5e..0x15024), from the Diplomats at (80, 64)
        GameEvent::ForeignGovernment { slot, govt } => {
            let adj = g.player(*slot).adjective();
            let lines = match govt {
                Some(gv) => vec![format!("{adj} government"), format!("changed to {}!", rules::GOVERNMENTS[*gv as usize].adjective)],
                None => vec![format!("The {adj} government"), "has been overthrown!".into()],
            };
            speaker_at(SPEAKER_DIPLOMATS, lines, 80, 64)
        }
        // CIV.EXE 0x18fb1 (ds:0x2a70), the Defense Minister's box at (100, 32)
        GameEvent::BarbariansLanded { near_city, target, .. } if *target == me => Some(Box::new(
            MenuDialog::new(vec!["Barbarian raiding party".into(), format!("lands near {}!", near_city.map(city_name).unwrap_or_default()), "Citizens are alarmed.".into()], Vec::new())
                .with_pos(100, 32)
                .with_speaker(SPEAKER_DEFENSE),
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
            // the Travelers' box at (100, 32) over its own "Travellers report:" line (0x302c)
            speaker_at(SPEAKER_TRAVELERS, vec!["Travellers report:".into(), what.into(), format!("near {near}!")], 100, 32)
        }
        // KING.TXT *LEADER (combat 0x1a04c)
        GameEvent::BarbarianLeaderCaptured { slot, gold } if *slot == me => {
            let lines = ctx.texts.king.section("LEADER").map(paragraphs).unwrap_or_else(|| vec!["Barbarian leader captured!".into(), format!("{gold}$ ransom paid.")]);
            msg(lines)
        }
        // the Science Advisor at (80, 64) (CIV.EXE 0x12f01..0x12f3e)
        GameEvent::Pollution { city, .. } if g.city(*city).owner == me => speaker_at(SPEAKER_SCIENCE, vec![format!("Pollution near {}.", city_name(*city))], 80, 64),
        // the explosion over the city, then "Nuclear Catastrophe\nin CITY!\nContamination
        // feared!" (CIV.EXE 0x12fab..0x1301f)
        GameEvent::Meltdown { city } if g.city(*city).owner == me => {
            let at = g.city(*city).pos();
            let vx = crate::game::Map::wrap_x(at.0 - crate::ui::mapview::VIEW_W / 2);
            let vy = (at.1 - crate::ui::mapview::VIEW_H / 2).clamp(0, crate::rules::MAP_H as i32 - crate::ui::mapview::VIEW_H);
            let anim = eventpics::nuke_animation_at_tile(at.0, at.1, vx, vy);
            let text = paper(vec!["Nuclear Catastrophe".into(), format!("in {}!", city_name(*city)), "Contamination feared!".into()], Some(*city));
            // the explosion's own tune 42 first, then 36 over the newspaper (0x12fe9)
            let text = with_tune(tune.take(), text);
            Some(Box::new(Sequence { first: Some(anim), then: text }))
        }
        // ds:0x3b6a (o 0xa924) and ds:0x1df8 (0x3b2e), in the newspaper
        GameEvent::GlobalWarming => paper(vec!["Global temperature".into(), "rises! Icecaps melt.".into(), "Severe Drought.".into()], None),
        GameEvent::GlobalWarmingFeared => paper(vec!["Scientists alarmed by".into(), "rising temperatures.".into(), "Global Warming feared!".into()], None),
        // the owner reads it all, anyone else who sees the city only the first line (CIV.EXE
        // o 0x1bf3b)
        GameEvent::Disaster { city, kind } if g.city_opt(*city).is_some_and(|c| c.owner == me || g.visible_to(me, c.x, c.y)) => {
            let lines = disaster_text(ctx, &city_name(*city), *kind);
            // the owner's Domestic Advisor at (100, 80), else the Travelers at (100, 64)
            // (o 0x1bf5c, 0x1bfd6)
            if g.city(*city).owner == me {
                speaker(SPEAKER_DOMESTIC, lines)
            } else {
                speaker_at(SPEAKER_TRAVELERS, lines.into_iter().take(1).collect(), 100, 64)
            }
        }
        GameEvent::CityDefected { city, admired, from } if *from == me || g.city_opt(*city).is_some_and(|c| g.visible_to(me, c.x, c.y)) => {
            msg(vec![format!("Residents of {}", city_name(*city)), format!("admire the prosperity of {}.", city_name(*admired))])
        }
        // "The <Plural> are\nrevolting! Citizens\ndemand new govt." (CIV.EXE 0x6f88, newspaper)
        GameEvent::Revolution { slot } if *slot == me => paper(vec![format!("The {} are", g.player(*slot).plural()), "revolting! Citizens".into(), "demand new govt.".into()], None),
        // the city's newspaper (CIV.EXE 0x127d6..0x12810)
        GameEvent::GovernmentCollapsed { slot, city } if *slot == me => {
            paper(vec!["Discontented citizens of".into(), format!("{} revolt:", city_name(*city)), "Government Collapses!".into()], Some(*city))
        }
        GameEvent::AnarchyEnded { slot } if *slot == me => None,
        // government_changed (o 0x5d17, after load_govt_picture at 0x14f2d); nothing under Anarchy
        GameEvent::GovernmentChanged { slot, govt } if *slot == me => crate::ui::govtscreen::govt_screen(ctx, *govt),
        GameEvent::UnitLimit(slot) if *slot == me => {
            let lines = ctx.texts.king.section("UMAX").map(paragraphs).unwrap_or_else(|| vec!["Too many units.".into()]);
            // the Defense Minister at (80, 64) (CIV.EXE 0x8d33)
            speaker_at(SPEAKER_DEFENSE, lines, 80, 64)
        }
        GameEvent::FutureTechnology { slot, n } if *slot == me => {
            let who = if g.player(*slot).knows(rules::advance_ids::INVENTION) { "scientists" } else { "wise men" };
            // the Science Advisor at (100, 80), the last line an item (CIV.EXE 0xc218, 0xc290..0xc2de)
            let head = vec!["Which discovery should our".into(), format!("{who} be pursuing, sire?"), "Pick one...".into()];
            Some(Box::new(MenuDialog::new(head, vec![format!(" Future Technology {n}")]).with_pos(100, 80).with_speaker(SPEAKER_SCIENCE)))
        }
        GameEvent::LowFunds { slot } if *slot == me => {
            let lines = ctx.texts.king.section("TAXES").map(paragraphs).unwrap_or_else(|| vec!["Treasury is low!".into()]);
            // the Domestic Advisor at (80, 80) (CIV.EXE 0x26d7)
            speaker_at(SPEAKER_DOMESTIC, lines, 80, 80)
        }
        // "CITY\ncan't maintain\nBUILDING." (CIV.EXE 0x13154)
        // "ADJ UNIT unit\nbribed by PLURAL!" (CIV.EXE o 0x1dd83)
        // from the Spies at (80, 40) (o 0x1ddf3)
        GameEvent::UnitBribed { kind, from, by, .. } if *from == me => {
            speaker_at(SPEAKER_SPIES, vec![format!("{} {} unit", g.player(*from).adjective(), rules::UNITS[*kind as usize].name), format!("bribed by {}!", g.player(*by).plural())], 80, 40)
        }
        // CIV.EXE 0xd724..0xd761: "Population decrease\nin " + name + ".\n" from the Domestic
        // Advisor at (100, 80)
        GameEvent::PopulationDecrease(city) if g.city_opt(*city).map(|c| c.owner == me).unwrap_or(false) => {
            speaker(SPEAKER_DOMESTIC, vec!["Population decrease".into(), format!("in {}.", city_name(*city))])
        }
        // the Domestic Advisor at (100, 80) (CIV.EXE 0x13154..0x131ad)
        GameEvent::CantMaintain { city, improvement } if g.city(*city).owner == me => {
            speaker(SPEAKER_DOMESTIC, vec![city_name(*city), "can't maintain".into(), format!("{}.", rules::IMPROVEMENTS[*improvement as usize].name)])
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
        // the arrival's newspaper follows its animation in the ending (`endgame`, 0x2b09)
        GameEvent::SpaceshipArrived(_) => None,
        GameEvent::SpaceshipLost(s) if *s == me => msg(vec!["Your spaceship is lost!".into()]),
        // CIV.EXE o 0x1842b, for every part an AI civ adds
        GameEvent::SpaceshipPartBuilt(s) if *s != me => {
            let (first, then) = crate::ui::spaceship_ui::ai_part_screens(ctx, *s);
            Some(Box::new(Sequence { first: Some(first), then }))
        }
        // "<Title> <Leader>\nplans retirement\nin 20 years." (CIV.EXE 0x2a28, overlay 21)
        GameEvent::RetirementWarning(s) if *s == me => {
            let p = g.player(*s);
            paper(vec![format!("{} {}", p.title(), p.leader), "plans retirement".into(), "in 20 years.".into()], None)
        }
        // CIV.EXE 0x14364
        GameEvent::SpaceshipCancelled { slot, launched } => {
            msg(vec![format!("{} spaceship", g.player(*slot).adjective()), if *launched { "returns to Earth.".into() } else { "construction cancelled.".into() }])
        }
        GameEvent::PalaceAddition(s) if *s == me => Some(crate::ui::palace::palace_addition(ctx)),
        GameEvent::HistorianReport { historian, category, rows } => Some(Box::new(crate::ui::report_world::Historian::new(*historian, *category, rows.clone()))),
        // the computer players' treaties and wars, for the human with an embassy, from the
        // Diplomats (CIV.EXE 0x15247, 0x152f8, 0x14aa4, at (100, 80), ds:0x2f9e = 1)
        GameEvent::PeaceTreaty { a, b } => speaker(SPEAKER_DIPLOMATS, vec![format!("{} and", civ(*a)), format!("{} sign", civ(*b)), "a peace treaty.".into()]),
        GameEvent::WarDeclared { a, b } => speaker(SPEAKER_DIPLOMATS, vec![format!("{} and", civ(*a)), format!("{} declare", civ(*b)), "war on each other.".into()]),
        GameEvent::TreatyCancelled { a, b } => speaker(SPEAKER_DIPLOMATS, vec![format!("{} and", civ(*a)), format!("{} cancel", civ(*b)), "peace treaty.".into()]),
        GameEvent::WarEnded { a, b } => speaker(SPEAKER_DIPLOMATS, vec!["The war between".into(), format!("the {} and the", civ(*a)), format!("{} has ended.", civ(*b))]),
        // fights, from the Defense and the Foreign Minister (CIV.EXE 0x19e98, 0x19f2c, at (100, 80))
        GameEvent::SneakAttack { by, .. } => speaker(SPEAKER_DEFENSE, vec!["Sneak attack by".into(), format!("{} forces!", g.player(*by).adjective())]),
        // "Diplomats report:" only for another civilization, then from the Foreign Minister
        // at (80, 80) (CIV.EXE 0xe984..0xe9fe; the speaker 1 set for the line is overwritten)
        GameEvent::CapitalMoved { slot, city } => {
            let mut lines = if *slot == me { Vec::new() } else { vec!["Diplomats report:".to_string()] };
            lines.push(format!("{} capital", g.player(*slot).adjective()));
            lines.push(format!("moved to {}.", city_name(*city)));
            Some(Box::new(MenuDialog::new(lines, Vec::new()).with_pos(80, 80).with_speaker(SPEAKER_FOREIGN)))
        }
        GameEvent::TreatyBroken { by, victim } => speaker(SPEAKER_FOREIGN, vec![format!("{} cancel", civ(*victim)), "peace treaty".into(), format!("with {}.", civ(*by))]),
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
        // nuclear_attack (CIV.EXE 0x1a463): a computer's strike is announced first by the
        // Defense Minister (ds:0x2f9e = 3) at (100, 80), "<Plural> use\nnuclear weapons!"
        // (ds:0x2b77); the explosion follows (o 0x1dedc). The human's own strike has no text.
        // The explosion plays at the viewport centre (the map view centres on the strike first).
        GameEvent::NuclearStrike { at, by, .. } => {
            let vx = crate::game::Map::wrap_x(at.0 - crate::ui::mapview::VIEW_W / 2);
            let vy = (at.1 - crate::ui::mapview::VIEW_H / 2).clamp(0, crate::rules::MAP_H as i32 - crate::ui::mapview::VIEW_H);
            let anim = eventpics::nuke_animation_at_tile(at.0, at.1, vx, vy);
            if *by == me {
                return Some(anim);
            }
            let text: Box<dyn Screen> =
                Box::new(MenuDialog::new(vec![format!("{} use", g.player(*by).plural()), "nuclear weapons!".into()], Vec::new()).with_pos(100, 80).with_speaker(SPEAKER_DEFENSE));
            Some(Box::new(Sequence { first: Some(text), then: Some(anim) }))
        }
        GameEvent::SdiProtects { city } => Some(Box::new(MessageBox::new(vec![format!("SDI protects {}.", city_name(*city))]).with_pos(80, 100))),
        GameEvent::GameEnded(reason) => Some(crate::ui::endgame::end_screen(ctx, *reason)),
        GameEvent::GameEndedAgain(reason) => Some(crate::ui::endgame::end_again_screen(ctx, *reason)),
        // a computer Diplomat's deeds are announced to the human whoever was involved, from
        // the Spies (CIV.EXE o 0x1d811 / 0x1d99d, popups at (80,80))
        GameEvent::DiplomatStole { by, adv, .. } => speaker_at(SPEAKER_SPIES, vec![format!("{} steal", g.player(*by).plural()), format!("{}.", rules::ADVANCES[*adv as usize].name)], 80, 80),
        GameEvent::CitySabotaged { city, improvement, production, .. } => {
            let first = match improvement {
                Some(i) => format!("{} destroyed", rules::IMPROVEMENTS[*i as usize].name),
                None => format!("{production} production sabotaged"),
            };
            speaker_at(SPEAKER_SPIES, vec![first, format!("in {}.", city_name(*city))], 80, 80)
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
        // choose_research (CIV.EXE 0x0c103..0x0c23b, ds:0x236c..0x23d5) at (80,64), from the
        // Science Advisor (0xc218)
        Decision::ChooseResearch { slot, options } => {
            let p = g.player(*slot);
            let items: Vec<String> = options
                .iter()
                .map(|a| if *a == rules::advance_ids::FUTURE_TECH { format!("Future Technology {}", p.future_techs + 1) } else { rules::ADVANCES[*a as usize].name.to_string() })
                .collect();
            let who = if p.knows(rules::advance_ids::INVENTION) { "scientists" } else { "wise men" };
            let head = vec!["Which discovery should our".into(), format!("{who} be pursuing, sire?"), "Pick one...".into()];
            // a right-button (or Alt-H) choice shows the advance's Civilopedia page and asks
            // again (0xc266)
            let pages = options.iter().map(|a| (*a != rules::advance_ids::FUTURE_TECH).then_some(crate::ui::pedia_data::Entry::Advance(*a))).collect();
            Box::new(MenuDialog::new(head, items).with_pos(80, 64).with_help_entries(pages).with_speaker(SPEAKER_SCIENCE).no_cancel())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::GameSetup;
    use crate::rules::{unit_ids, wonder_ids, Govt};

    #[test]
    fn event_tunes_follow_the_call_sites() {
        let mut g = Game::new(GameSetup { seed: 42, ..Default::default() }, None);
        let me = g.human_slot;
        let settler = g.units_of(me).into_iter().find(|id| g.unit(*id).kind == unit_ids::SETTLERS).unwrap();
        let c = g.found_city(settler).unwrap();
        let ai = g.alive_slots().into_iter().find(|s| *s != me && *s != 0).unwrap();
        let mine = Tune::Theme { civ: g.player(me).civ, long: false };
        let theirs = Tune::Theme { civ: g.player(ai).civ, long: false };
        let disorder = Some(Tune::Sound(Sound::Disorder));
        let tune = |g: &Game, ev: GameEvent| event_tune(g, &ev);

        assert_eq!(tune(&g, GameEvent::Disorder(c)), disorder);
        assert_eq!(tune(&g, GameEvent::GovernmentCollapsed { slot: me, city: c }), disorder);
        assert_eq!(tune(&g, GameEvent::Meltdown { city: c }), disorder);
        assert_eq!(tune(&g, GameEvent::AdvanceDiscovered { slot: me, adv: 0 }), Some(mine));
        assert_eq!(tune(&g, GameEvent::AdvanceDiscovered { slot: ai, adv: 0 }), None);
        // the Manhattan Project's test is heard from anyone; other computer wonders only when
        // granted at the end of the turn (0x3e9e)
        assert_eq!(tune(&g, GameEvent::WonderBuilt { city: c, wonder: wonder_ids::MANHATTAN_PROJECT, owner: ai }), disorder);
        assert_eq!(tune(&g, GameEvent::WonderBuilt { city: c, wonder: wonder_ids::PYRAMIDS, owner: ai }), None);
        assert_eq!(tune(&g, GameEvent::WonderGranted { city: c, wonder: wonder_ids::PYRAMIDS, owner: ai }), Some(theirs));
        assert_eq!(tune(&g, GameEvent::CityCaptured { city: c, by: ai, from: me, gold: 0, adv: None }), Some(theirs));
        // no call site at all
        assert_eq!(tune(&g, GameEvent::Pollution { city: c, at: (0, 0) }), None);
        assert_eq!(tune(&g, GameEvent::CivDestroyed { slot: ai, by: me, adjective: String::new(), civ: 1 }), None);
        assert_eq!(tune(&g, GameEvent::NuclearStrike { at: (0, 0), by: ai, city: None }), None);
        // the celebrations are silent under Anarchy (0x129e4, 0x12b4d)
        g.player_mut(me).govt = Govt::Monarchy;
        assert_eq!(tune(&g, GameEvent::Celebration(c)), Some(Tune::Sound(Sound::Win)));
        assert_eq!(tune(&g, GameEvent::CelebrationEnded(c)), Some(Tune::Sound(Sound::Lose)));
        g.player_mut(me).govt = Govt::Anarchy;
        assert_eq!(tune(&g, GameEvent::Celebration(c)), None);
        assert_eq!(tune(&g, GameEvent::CelebrationEnded(c)), None);
    }
}
