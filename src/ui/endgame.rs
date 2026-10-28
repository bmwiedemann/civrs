//! The end of the game as `game_turn` runs it (CIV.EXE 0x2a98..0x2c1f) and the screens it
//! calls:
//! - a conquest: the newspaper "Your civilization\nhas conquered\nthe entire planet!" and
//!   `conquest_victory` (o 0xd953, see `conquest`);
//! - a spaceship's arrival: `spaceship_arrival` (overlay 2, see `spaceship_ui`), then the
//!   newspaper "<Adjective> spaceship\narrives at Alpha Centauri.";
//! - the year 2100 - 20 * difficulty: the newspaper "<Leader> dynasty\nends after
//!   glorious\n6000 year reign!" over the national anthem;
//! - then, the first time only (ds:0x4cd8 bit 0x100), `end_game_scores` (0x2c20): the power
//!   graph, the replay (not after a retirement), the rating (`rating_screen` o 0x2dd2) and the
//!   Hall of Fame with the new entry. A conquest and a retirement end the program on
//!   ADSCREEN.PIC (`AdScreen`, 0x2c8a); the other endings ask *FAME whether to keep playing.
//! - The human destroyed (no cities, no Settlers): only ARCH.PIC with the KING.TXT ARCH text
//!   (`archaeology_screen` o 0x209f), then the program ends (ds:0xdc48 = -1, 0x258f).

use crate::assets::text::{paragraphs, substitute, Vars};
use crate::assets::Palette;
use crate::audio::{Sound, Tune};
use crate::game::EndReason;
use crate::gfx::text::draw_text_centered;
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::Event;
use crate::rules;
use crate::ui::context::UiContext;
use crate::ui::eventpics::PicScreen;
use crate::ui::newspaper::newspaper;
use crate::ui::replay::{PowerGraph, Replay};
use crate::ui::screen::{DialogResult, Screen, Transition};

/// The screen (chain) for a game end reason, the first time the game ends.
pub fn end_screen(ctx: &UiContext, reason: EndReason) -> Box<dyn Screen> {
    match reason {
        EndReason::Destroyed => chain(vec![arch_screen(ctx), Box::new(Exit)]),
        EndReason::Retired => {
            let mut s = end_game_scores(ctx, true);
            s.push(Box::new(AdScreen));
            chain(s)
        }
        EndReason::Conquest => {
            let mut s = ending_news(ctx, reason);
            s.extend(end_game_scores(ctx, false));
            // no *FAME question: a conquest ends the program (ds:0xdc48 = 1, 0x2c09)
            s.push(Box::new(AdScreen));
            chain(s)
        }
        EndReason::Spaceship(_) | EndReason::TimeLimit => {
            let mut s = ending_news(ctx, reason);
            s.extend(end_game_scores(ctx, false));
            chain(s)
        }
    }
}

/// An ending after the human kept playing (ds:0x4cd8 bit 0x100): the same ending without
/// its scores (0x2bc1); a retirement still shows the power graph (0x2c20 tests the bit only
/// after it), a conquest still quits.
pub fn end_again_screen(ctx: &UiContext, reason: EndReason) -> Box<dyn Screen> {
    match reason {
        EndReason::Destroyed => end_screen(ctx, reason),
        EndReason::Retired => chain(vec![Box::new(PowerGraph), Box::new(AdScreen)]),
        EndReason::Conquest => {
            let mut s = ending_news(ctx, reason);
            s.push(Box::new(AdScreen));
            chain(s)
        }
        EndReason::Spaceship(_) | EndReason::TimeLimit => chain(ending_news(ctx, reason)),
    }
}

/// What game_turn shows for an ending before the scores (0x2abb..0x2ba7).
fn ending_news(ctx: &UiContext, reason: EndReason) -> Vec<Box<dyn Screen>> {
    let Some(g) = ctx.game.as_ref() else { return Vec::new() };
    match reason {
        EndReason::Conquest => vec![
            newspaper(vec!["Your civilization".into(), "has conquered".into(), "the entire planet!".into()], None),
            Box::new(crate::ui::conquest::Conquest::new(ctx)),
        ],
        EndReason::Spaceship(slot) => vec![
            crate::ui::spaceship_ui::arrival_screen(ctx, slot),
            newspaper(vec![format!("{} spaceship", g.player(slot).adjective()), "arrives at Alpha Centauri.".into()], None),
        ],
        // over the full anthem, faded as the newspaper closes (0x2b96, 0x2bae); the other
        // endings bring their own music
        EndReason::TimeLimit => {
            let paper = newspaper(vec![format!("{} dynasty", g.human().leader), "ends after glorious".into(), "6000 year reign!".into()], None);
            vec![crate::ui::dialogs::tuned(Tune::Theme { civ: g.human().civ, long: true }, paper)]
        }
        _ => Vec::new(),
    }
}

/// `end_game_scores` (0x2c20): the power graph, the replay unless the human retired
/// (ds:0xdc48 = 2), the rating of the final score and the Hall of Fame.
fn end_game_scores(ctx: &UiContext, retired: bool) -> Vec<Box<dyn Screen>> {
    let mut s: Vec<Box<dyn Screen>> = vec![Box::new(PowerGraph)];
    if !retired {
        s.push(Box::new(Replay::new()));
    }
    if let Some(g) = ctx.game.as_ref() {
        let me = g.human_slot;
        let score = g.player(me).final_score.unwrap_or_else(|| g.score(me).total);
        s.extend(rating_screen_for(ctx, score));
        let e = crate::save::FameEntry {
            leader: g.human().leader.clone(),
            civ_plural: g.civ_name(me).to_string(),
            score,
            percent: score * (g.setup.difficulty as i32 + 1) / 50,
            year: g.year(),
            difficulty: g.setup.difficulty.as_u8(),
            title: g.human().title().trim().to_string(),
            population: g.total_population(me),
        };
        s.push(crate::ui::halloffame::hall_of_fame_with(e));
    }
    s
}

/// `show_ad_screen_and_exit` (0x2c8a): the music stops, ADSCREEN.PIC until a key, and the
/// program ends.
pub struct AdScreen;

impl Screen for AdScreen {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        ctx.audio.stop();
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        match ev {
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } => Transition::Quit,
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        fb.blit(&ctx.pic("ADSCREEN").surf, 0, 0);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.pic_pal("ADSCREEN")
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

/// The screens one after the other.
fn chain(screens: Vec<Box<dyn Screen>>) -> Box<dyn Screen> {
    let mut next: Option<Box<dyn Screen>> = None;
    for screen in screens.into_iter().rev() {
        next = Some(match next {
            None => screen,
            Some(n) => Box::new(Chain { screen, next: Some(n) }),
        });
    }
    next.expect("an empty chain")
}

/// ARCH.PIC with the "Centuries later, archeologists discover..." text (`archaeology_screen`
/// o 0x209f): $US is the leader's name in capitals, $RPLC1 the last advance in table order the
/// human knows (`knows_advance`, so never Future Technology), "Irrigation" when none.
pub fn arch_screen(ctx: &UiContext) -> Box<dyn Screen> {
    let g = ctx.game.as_ref();
    let evidence = g
        .and_then(|g| (0..72u8).rev().find(|a| g.knows_advance(g.human_slot, *a)).map(|a| rules::ADVANCES[a as usize].name.to_string()))
        .unwrap_or_else(|| "Irrigation".to_string());
    let v = Vars { rplc1: evidence, us: g.map(|g| g.human().leader.to_ascii_uppercase()).unwrap_or_default(), ..Default::default() };
    let lines = ctx.texts.king.section("ARCH").map(|l| l.to_vec()).unwrap_or_default();
    let text = paragraphs(&substitute(&lines, &v));
    let pic = PicScreen::new("ARCH", text).with_caption_pos(6, 4).with_caption_width(200);
    Box::new(Arch { pic: Box::new(pic), coda: None })
}

/// Ticks the human's jingle plays at the end of `archaeology_screen` (o 0x2309).
const ARCH_CODA: u32 = 300;

/// The music of `archaeology_screen`: tune 35 as the screen clears (o 0x20db); after the
/// text, a fade, the human's jingle for 300 ticks and a fade again (o 0x22e6..0x2319).
struct Arch {
    pic: Box<dyn Screen>,
    coda: Option<u32>,
}

impl Screen for Arch {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        ctx.audio.play(Sound::Lose);
        self.pic.on_enter(ctx);
    }
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        match self.coda.as_mut() {
            Some(t) => {
                *t = t.saturating_sub(1);
                if *t == 0 || ctx.instant {
                    ctx.audio.fade_out();
                    return Transition::Pop(DialogResult::Done);
                }
                Transition::None
            }
            None => {
                let t = self.pic.update(ctx);
                self.forward(t, ctx)
            }
        }
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if self.coda.is_some() {
            return Transition::None;
        }
        let t = self.pic.handle(ev, ctx);
        self.forward(t, ctx)
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        self.pic.draw(fb, ctx);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        self.pic.palette(ctx)
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

impl Arch {
    fn forward(&mut self, t: Transition, ctx: &mut UiContext) -> Transition {
        match t {
            Transition::Pop(_) => {
                ctx.audio.fade_out();
                if let Some(g) = ctx.game.as_ref() {
                    ctx.audio.play_tune(Tune::Theme { civ: g.human().civ, long: false });
                }
                self.coda = Some(ARCH_CODA);
                Transition::None
            }
            other => other,
        }
    }
}

/// Shows `screen`, then `next` (if any) in its place.
struct Chain {
    screen: Box<dyn Screen>,
    next: Option<Box<dyn Screen>>,
}

impl Screen for Chain {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        self.screen.on_enter(ctx);
    }
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        let t = self.screen.update(ctx);
        self.forward(t, ctx)
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        let t = self.screen.handle(ev, ctx);
        self.forward(t, ctx)
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        self.screen.draw(fb, ctx);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        self.screen.palette(ctx)
    }
    fn draws_parent(&self) -> bool {
        self.screen.draws_parent()
    }
    fn wants_cursor(&self) -> bool {
        self.screen.wants_cursor()
    }
}

impl Chain {
    fn forward(&mut self, t: Transition, ctx: &mut UiContext) -> Transition {
        match t {
            Transition::Pop(_) => match self.next.take() {
                Some(n) => Transition::Replace(n),
                None => Transition::Pop(DialogResult::Done),
            },
            // a screen of the chain that hands over to another keeps the rest of the chain
            Transition::Replace(mut s) => {
                s.on_enter(ctx);
                self.screen = s;
                Transition::None
            }
            other => other,
        }
    }
}

/// `rating_screen(level, score)` (ovl 3 +0x65d, o 0x2dd2) for the final `score`: the rating
/// is score * (difficulty + 1) / 50 percent and the level the largest i - 1 of i in 1..=20
/// with i * i / 3 below it (`score::rating_index`); with no score the level is the palace's
/// (ds:0x1c20 / 2 - 1, the argument of 0x2c5a). Nothing below level 0.
pub fn rating_screen_for(ctx: &UiContext, score: i32) -> Option<Box<dyn Screen>> {
    let g = ctx.game.as_ref()?;
    let (level, percent) = if score != 0 {
        let percent = score * (g.setup.difficulty as i32 + 1) / 50;
        ((1..=20).filter(|i| i * i / 3 < percent).max().map(|i| i - 1)?, Some(percent))
    } else {
        (g.human().palace_level as i32 / 2 - 1, None)
    };
    (level >= 0).then(|| Box::new(RatingScreen { level: level.min(19) as usize, percent, border: (ctx.frame.wrapping_mul(0x9e37_79b9) >> 13) as u8 & 1, guard: 2 }) as Box<dyn Screen>)
}

/// The rating screen of a score (for the screen dump).
pub fn rating_screen(ctx: &UiContext, score: i32) -> Box<dyn Screen> {
    rating_screen_for(ctx, score).unwrap_or_else(|| Box::new(Exit))
}

/// The page of `rating_screen`: tune 0x22, colour 15 with the random SP299 page border, the
/// SP299 pictures (200, 140) and (240, 140) (40x60) at (10, 10) and (270, 10), a stone panel
/// (80, 4, 160, 27) with a 11/8 bevel and three lines in font 7, colour 15, centred at y 6, 14
/// and 22: "Sire, your Civilization" / "Rating of N% exceeds even" / "<ruler>!" (with no
/// score "Sire, your fame now" / "exceeds even that of" / "<ruler>!"); then the rulers of
/// levels 0..=level (far 0x2653:(19 - i) * 25, `rules::RATINGS`) centred at y 184 - 8 i, in 7
/// and the reached one in 0 with its colour 15 turned 14 over (80, 183 - 8 level, 160, 8).
struct RatingScreen {
    level: usize,
    percent: Option<i32>,
    border: u8,
    guard: u32,
}

impl Screen for RatingScreen {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        ctx.audio.play(Sound::Win);
    }
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        match ev {
            // the key fades the tune (o 0x3102)
            Event::Key { down: true, .. } | Event::MouseButton { down: true, .. } if self.guard == 0 => {
                ctx.audio.fade_out();
                Transition::Pop(DialogResult::Done)
            }
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        const FONT7: usize = 6;
        fb.fill(15);
        crate::ui::pedia_entry::draw_border(fb, ctx, self.border);
        let sp = &ctx.pic("SP299").surf;
        fb.blit_part(sp, Rect::new(200, 140, 40, 60), 10, 10, None);
        fb.blit_part(sp, Rect::new(240, 140, 40, 60), 270, 10, None);
        crate::ui::widgets::fill_texture(fb, ctx, Rect::new(80, 4, 160, 27));
        crate::ui::widgets::bevel(fb, 80, 4, 159, 26, 11, 8);
        let ruler = |i: usize| rules::RATINGS[19 - i];
        let head = match self.percent {
            Some(p) => ["Sire, your Civilization".to_string(), format!("Rating of {p}% exceeds even"), format!("{}!", ruler(self.level))],
            None => ["Sire, your fame now".to_string(), "exceeds even that of".to_string(), format!("{}!", ruler(self.level))],
        };
        for (l, y) in head.iter().zip([6, 14, 22]) {
            draw_text_centered(fb, &ctx.fonts, &TextStyle::new(FONT7, 15), 160, y, l);
        }
        for i in 0..=self.level {
            let col = if i == self.level { 0 } else { 7 };
            draw_text_centered(fb, &ctx.fonts, &TextStyle::new(FONT7, col), 160, 184 - 8 * i as i32, ruler(i));
        }
        fb.replace_colour(Rect::new(80, 183 - 8 * self.level as i32, 160, 8), 15, 14);
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

/// The program ends (after the human's destruction: 0x258f -> main's exit 0x1c0d).
struct Exit;

impl Screen for Exit {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        Transition::Quit
    }
    fn handle(&mut self, _ev: &Event, _ctx: &mut UiContext) -> Transition {
        Transition::Quit
    }
    fn draw(&mut self, _fb: &mut Surface, _ctx: &mut UiContext) {}
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.game_palette()
    }
}
