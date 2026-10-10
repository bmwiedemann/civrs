//! Opening sequence: "A MICROPROSE PRESENTATION", the logo over the galaxy, the credits one line
//! at a time, then the main menu (logo on black with the brown menu box).

use crate::gfx::text::draw_text_centered;
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::Event;
use crate::rules::MAIN_MENU;
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::{MenuDialog, PanelStyle};

/// Logo lettering area inside LOGO.PIC and where it is drawn.
const LOGO_SRC: Rect = Rect::new(0, 56, 320, 90);
const LOGO_Y: i32 = 56;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Presentation,
    Logo,
    Credits,
    Menu,
}

pub struct TitleScreen {
    phase: Phase,
    t: u64,
    credit: usize,
    menu_up: bool,
    pending_shortcut: Option<usize>,
    skip_to_menu: bool,
}

impl Default for TitleScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl TitleScreen {
    pub fn new() -> TitleScreen {
        TitleScreen { phase: Phase::Presentation, t: 0, credit: 0, menu_up: false, pending_shortcut: None, skip_to_menu: false }
    }

    pub fn at_menu() -> TitleScreen {
        TitleScreen { phase: Phase::Menu, t: 0, credit: 0, menu_up: false, pending_shortcut: None, skip_to_menu: false }
    }

    fn credit_lines(ctx: &UiContext) -> Vec<String> {
        ctx.texts.credits.iter().flat_map(|(a, b)| vec![a.clone(), b.clone()]).filter(|l| !l.is_empty()).collect()
    }

    fn open_menu(&mut self) -> Transition {
        self.phase = Phase::Menu;
        self.menu_up = true;
        let items: Vec<String> = MAIN_MENU.iter().map(|s| s.to_string()).collect();
        Transition::Push(Box::new(MenuDialog::new(vec![], items).with_pos(86, 135).with_style(PanelStyle::Brown).with_size(154, 53).with_top_pad(7).no_cancel()))
    }

    /// Palette: galaxy picture plus the logo's own colours for the indices the logo uses.
    fn merged_palette(ctx: &UiContext, galaxy: bool) -> crate::assets::Palette {
        let logo = ctx.pic("LOGO");
        if !galaxy {
            return logo.pal;
        }
        let bg = ctx.pic("BIRTH1");
        let mut pal = bg.pal;
        let mut used = [false; 256];
        for &v in &logo.surf.px {
            used[v as usize] = true;
        }
        for (i, u) in used.iter().enumerate() {
            if *u && i != 0 {
                pal.0[i] = logo.pal.0[i];
            }
        }
        pal
    }
}

impl Screen for TitleScreen {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        ctx.audio.music(0);
    }

    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        self.t += 1;
        if let Some(s) = self.pending_shortcut.take() {
            return Transition::Push(crate::ui::setup::start_flow(s));
        }
        if self.skip_to_menu {
            self.skip_to_menu = false;
            return self.open_menu();
        }
        match self.phase {
            Phase::Presentation => {
                if self.t > 150 {
                    self.phase = Phase::Logo;
                    self.t = 0;
                }
            }
            Phase::Logo => {
                if self.t > 240 {
                    self.phase = Phase::Credits;
                    self.t = 0;
                    self.credit = 0;
                }
            }
            Phase::Credits => {
                let n = Self::credit_lines(ctx).len().max(1);
                if self.t > 75 {
                    self.t = 0;
                    self.credit += 1;
                    if self.credit >= n {
                        return self.open_menu();
                    }
                }
            }
            Phase::Menu => {
                if !self.menu_up {
                    return self.open_menu();
                }
            }
        }
        Transition::None
    }

    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        match ev {
            Event::Key { key, down: true, .. } => {
                use crate::platform::Key;
                let shortcut = match key {
                    Key::Char('n') => Some(0),
                    Key::Char('l') => Some(1),
                    Key::Char('e') => Some(2),
                    Key::Char('c') => Some(3),
                    _ => None,
                };
                if let Some(s) = shortcut {
                    self.phase = Phase::Menu;
                    self.pending_shortcut = Some(s);
                } else {
                    self.skip_to_menu = true;
                }
                Transition::None
            }
            Event::MouseButton { down: true, .. } => {
                self.skip_to_menu = true;
                Transition::None
            }
            _ => Transition::None,
        }
    }

    fn resume(&mut self, result: DialogResult, _ctx: &mut UiContext) -> Transition {
        self.menu_up = false;
        match result {
            DialogResult::Choice(4) => Transition::Push(crate::ui::halloffame::hall_of_fame()),
            DialogResult::Choice(i) => Transition::Push(crate::ui::setup::start_flow(i)),
            _ => Transition::None,
        }
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        fb.fill(0);
        let pal = self.palette(ctx);
        match self.phase {
            Phase::Presentation => {
                let c = pal.nearest([219, 190, 130]);
                draw_text_centered(fb, &ctx.fonts, &TextStyle::new(5, c), 160, 44, "A MICROPROSE PRESENTATION");
            }
            Phase::Logo | Phase::Credits => {
                let galaxy = ctx.pic("BIRTH1");
                fb.blit(&galaxy.surf, 0, 0);
                if self.phase == Phase::Logo {
                    let logo = ctx.pic("LOGO");
                    fb.blit_part(&logo.surf, LOGO_SRC, 0, LOGO_Y, Some(0));
                } else {
                    let lines = Self::credit_lines(ctx);
                    if let Some(l) = lines.get(self.credit) {
                        let c = pal.nearest([219, 190, 130]);
                        draw_text_centered(fb, &ctx.fonts, &TextStyle::new(5, c), 160, 150, l);
                    }
                }
            }
            Phase::Menu => {
                let logo = ctx.pic("LOGO");
                fb.blit_part(&logo.surf, LOGO_SRC, 0, LOGO_Y, Some(0));
            }
        }
    }

    fn palette(&self, ctx: &UiContext) -> crate::assets::Palette {
        match self.phase {
            Phase::Presentation | Phase::Menu => Self::merged_palette(ctx, false),
            _ => Self::merged_palette(ctx, true),
        }
    }

    fn draws_parent(&self) -> bool {
        false
    }

    fn wants_cursor(&self) -> bool {
        self.phase == Phase::Menu
    }
}

/// Start-up entry: goes straight into the title sequence.
pub struct Splash;

impl Splash {
    pub fn new() -> TitleScreen {
        TitleScreen::new()
    }
}
