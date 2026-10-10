//! New-game setup flow: difficulty, number of civilisations, tribe, leader name, custom world,
//! then world generation and the evolution story.

use crate::game::{GameSetup, WorldParams, WorldSource};
use crate::gfx::text::draw_text;
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::Event;
use crate::rules::{self, Difficulty, CUSTOM_WORLD_OPTIONS};
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::{MenuDialog, PanelStyle, TextInput};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Start,
    Difficulty,
    NumCivs,
    Tribe,
    Name,
    Custom(usize),
    Generate,
    Done,
}

/// Portrait rectangles in DIFFS.PIC, one per difficulty level (Chieftain..Emperor).
pub const PORTRAITS: [Rect; 5] = [Rect::new(21, 5, 53, 50), Rect::new(80, 40, 53, 50), Rect::new(21, 76, 53, 50), Rect::new(80, 111, 53, 50), Rect::new(21, 146, 53, 50)];
const BOX: Rect = Rect::new(155, 28, 132, 140);

pub struct SetupFlow {
    kind: usize,
    step: Step,
    setup: GameSetup,
    params: WorldParams,
    tribes: Vec<u8>,
    custom_choices: [Option<usize>; 4],
}

pub fn start_flow(kind: usize) -> Box<dyn Screen> {
    if kind == 1 {
        return crate::ui::reports::load_game_screen();
    }
    Box::new(SetupFlow {
        kind,
        step: Step::Start,
        setup: GameSetup { seed: seed_from_clock(), ..Default::default() },
        params: WorldParams::normal(),
        tribes: Vec::new(),
        custom_choices: [None; 4],
    })
}

fn seed_from_clock() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
}

impl SetupFlow {
    fn white_menu(title: &str, items: Vec<String>) -> MenuDialog {
        MenuDialog::new(vec![title.to_string()], items).with_pos(BOX.x, BOX.y).with_style(PanelStyle::White).with_size(BOX.w, BOX.h)
    }

    fn next(&mut self, ctx: &mut UiContext) -> Transition {
        match self.step {
            Step::Start => {
                if self.kind == 3 {
                    self.step = Step::Custom(0);
                    return self.custom_menu(0);
                }
                self.difficulty_menu()
            }
            Step::Custom(i) => {
                if i + 1 < 4 {
                    self.step = Step::Custom(i + 1);
                    return self.custom_menu(i + 1);
                }
                self.difficulty_menu()
            }
            Step::Difficulty => {
                self.step = Step::NumCivs;
                let items: Vec<String> = (3..=7).rev().map(|n| format!("{n} Civilizations")).collect();
                Transition::Push(Box::new(Self::white_menu("Level of Competition...", items)))
            }
            Step::NumCivs => {
                self.step = Step::Tribe;
                self.tribes = (1..16u8).filter(|c| *c != 8).collect();
                let items: Vec<String> = self.tribes.iter().map(|c| rules::CIVS[*c as usize].adjective.to_string()).collect();
                Transition::Push(Box::new(Self::white_menu("Pick your tribe...", items)))
            }
            Step::Tribe => {
                self.step = Step::Name;
                let default = rules::CIVS[self.setup.human_civ as usize].leader.to_string();
                Transition::Push(Box::new(TextInput::new(vec!["Your Name...".into()], &default, 14).with_pos(158, 88).with_style(PanelStyle::Cyan)))
            }
            Step::Name => {
                self.step = Step::Generate;
                self.generate(ctx)
            }
            Step::Generate | Step::Done => Transition::None,
        }
    }

    fn difficulty_menu(&mut self) -> Transition {
        self.step = Step::Difficulty;
        let mut items: Vec<String> = rules::DIFFICULTY.iter().map(|d| d.name.to_string()).collect();
        items[0] = format!("{} (easiest)", items[0]);
        items[4] = format!("{} (toughest)", items[4]);
        Transition::Push(Box::new(Self::white_menu("Difficulty Level...", items)))
    }

    fn custom_menu(&self, i: usize) -> Transition {
        let (_, opts) = CUSTOM_WORLD_OPTIONS[i];
        let items: Vec<String> = opts.iter().map(|s| s.to_string()).collect();
        let y = 14 + i as i32 * 49;
        Transition::Push(Box::new(CustomMenu { inner: MenuDialog::new(vec![], items).with_pos(190, y).with_selection(1).no_cancel() }))
    }

    fn generate(&mut self, ctx: &mut UiContext) -> Transition {
        self.setup.world = if self.kind == 2 { WorldSource::Earth } else { WorldSource::Random(self.params) };
        let earth = if self.kind == 2 { ctx.assets.load_earth().ok() } else { None };
        let game = crate::game::Game::new(self.setup.clone(), earth.as_ref());
        ctx.game = Some(game);
        self.step = Step::Done;
        Transition::Replace(crate::ui::story::story_screen())
    }

    fn picture(&self) -> &'static str {
        if matches!(self.step, Step::Custom(_)) {
            "CUSTOM"
        } else {
            "DIFFS"
        }
    }

    fn draw_portrait_stack(&self, fb: &mut Surface, ctx: &UiContext, n: i32) {
        let diffs = ctx.pic("DIFFS");
        let r = PORTRAITS[self.setup.difficulty as usize];
        let pal = ctx.pic_pal("DIFFS");
        let gold = pal.nearest([255, 219, 89]);
        let black = pal.nearest([0, 0, 0]);
        for i in (0..n).rev() {
            let (x, y) = (12 + i * 2, 142 - i * 2);
            if i > 0 {
                fb.fill_rect(Rect::new(x, y, r.w, r.h), gold);
                fb.rect_outline(Rect::new(x, y, r.w, r.h), black);
            } else {
                fb.blit_part(&diffs.surf, r, x, y, None);
            }
        }
    }
}

/// Custom-world option menu drawn in the cyan area without a panel.
struct CustomMenu {
    inner: MenuDialog,
}

impl Screen for CustomMenu {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        self.inner.on_enter(ctx);
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        self.inner.handle(ev, ctx)
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let pal = ctx.pic_pal("CUSTOM");
        let bar = pal.nearest([12, 227, 235]);
        let black = pal.nearest([0, 0, 0]);
        let r = self.inner.rect;
        for (i, it) in self.inner.items.iter().enumerate() {
            let y = r.y + 4 + i as i32 * 8;
            if i == self.inner.sel {
                fb.fill_rect(Rect::new(r.x - 2, y - 1, 60, 8), bar);
            }
            draw_text(fb, &ctx.fonts, &TextStyle::new(0, black), r.x, y, it);
        }
    }
}

impl Screen for SetupFlow {
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        if self.step == Step::Start {
            return self.next(ctx);
        }
        Transition::None
    }

    fn handle(&mut self, _ev: &Event, _ctx: &mut UiContext) -> Transition {
        Transition::None
    }

    fn resume(&mut self, result: DialogResult, ctx: &mut UiContext) -> Transition {
        match (&self.step, result) {
            (_, DialogResult::Cancelled) => Transition::Pop(DialogResult::Done),
            (Step::Difficulty, DialogResult::Choice(i)) => {
                self.setup.difficulty = Difficulty::from_u8(i as u8).unwrap_or(Difficulty::Chieftain);
                self.next(ctx)
            }
            (Step::NumCivs, DialogResult::Choice(i)) => {
                self.setup.num_civs = 7 - i as u8;
                self.next(ctx)
            }
            (Step::Tribe, DialogResult::Choice(i)) => {
                self.setup.human_civ = self.tribes.get(i).copied().unwrap_or(1);
                self.next(ctx)
            }
            (Step::Name, DialogResult::Text(t)) => {
                self.setup.leader_name = t;
                self.next(ctx)
            }
            (Step::Custom(i), DialogResult::Choice(c)) => {
                let i = *i;
                self.custom_choices[i] = Some(c);
                match i {
                    0 => self.params.land_mass = c as u8,
                    1 => self.params.temperature = c as u8,
                    2 => self.params.climate = c as u8,
                    _ => self.params.age = c as u8,
                }
                self.next(ctx)
            }
            _ => Transition::None,
        }
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let p = ctx.pic(self.picture());
        let pal = ctx.pic_pal(self.picture());
        match self.step {
            Step::Custom(cur) => {
                fb.blit(&p.surf, 0, 0);
                let white = pal.nearest([255, 255, 255]);
                let black = pal.nearest([0, 0, 0]);
                let bar = pal.nearest([12, 227, 235]);
                for (i, (title, opts)) in CUSTOM_WORLD_OPTIONS.iter().enumerate() {
                    if i > cur {
                        break;
                    }
                    let y = 6 + i as i32 * 49;
                    draw_text(fb, &ctx.fonts, &TextStyle::new(0, white), 180, y, &format!("{}:", title.to_ascii_uppercase()));
                    if i < cur {
                        for (k, o) in opts.iter().enumerate() {
                            let yy = y + 8 + k as i32 * 8;
                            if self.custom_choices[i] == Some(k) {
                                fb.fill_rect(Rect::new(188, yy - 1, 60, 8), bar);
                            }
                            draw_text(fb, &ctx.fonts, &TextStyle::new(0, black), 190, yy, o);
                        }
                    }
                }
            }
            Step::Difficulty | Step::Start => {
                fb.blit(&p.surf, 0, 0);
            }
            _ => {
                fb.fill(pal.nearest([0, 0, 0]));
                let n = match self.step {
                    Step::NumCivs => 1,
                    _ => 6,
                };
                self.draw_portrait_stack(fb, ctx, n);
                if self.step == Step::Name {
                    let white = pal.nearest([255, 255, 255]);
                    draw_text(fb, &ctx.fonts, &TextStyle::new(0, white), 14, 50, rules::CIVS[self.setup.human_civ as usize].plural);
                }
            }
        }
    }

    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        ctx.pic_pal(self.picture())
    }

    fn draws_parent(&self) -> bool {
        false
    }
}
