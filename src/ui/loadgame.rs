//! Save / load slot pages: "Select Save File..." / "Select Load File..." listing the ten
//! slots as "<EMPTY>" or the saved game's name, on the cream page (title screen) or on the
//! light page with the dotted border (GAME menu).

use crate::assets::Palette;
use crate::gfx::{Rect, Surface};
use crate::platform::{Event, Key, MouseButton};
use crate::ui::context::UiContext;
use crate::ui::reports::{col, dotted_border, page, text, BLACK, CREAM, CYAN, LINE_H, RED, TAN, WHITE};
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::MessageBox;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotMode {
    /// GAME menu: save into a slot.
    Save,
    /// GAME menu: load a slot and restart the map view.
    LoadInGame,
    /// Title screen: load a slot (cream page, no border).
    LoadTitle,
}

/// Position of the list on the two page variants.
fn list_origin(mode: SlotMode) -> (i32, i32) {
    match mode {
        SlotMode::LoadTitle => (53, 70),
        _ => (53, 39),
    }
}

pub struct SlotPage {
    mode: SlotMode,
    slots: Vec<Option<crate::save::SlotInfo>>,
    sel: usize,
    guard: u32,
}

impl SlotPage {
    pub fn new(ctx: &UiContext, mode: SlotMode) -> SlotPage {
        let slots = crate::save::list_slots(&ctx.save_dir);
        let sel = match mode {
            SlotMode::Save => 0,
            _ => slots.iter().position(|s| s.is_some()).unwrap_or(0),
        };
        SlotPage { mode, slots, sel, guard: 2 }
    }

    fn label(&self, i: usize) -> String {
        match &self.slots[i] {
            Some(info) => format!("{} of the {}, {}", info.leader, info.civ, crate::rules::year_text(info.year)),
            None => "<EMPTY>".to_string(),
        }
    }

    fn selectable(&self, i: usize) -> bool {
        self.mode == SlotMode::Save || self.slots[i].is_some()
    }

    fn row_at(&self, mx: i32, my: i32) -> Option<usize> {
        let (x, y) = list_origin(self.mode);
        if mx < x - 2 || mx > x + 160 {
            return None;
        }
        let i = (my - (y + LINE_H) + 1).div_euclid(LINE_H);
        if i >= 0 && (i as usize) < self.slots.len() {
            Some(i as usize)
        } else {
            None
        }
    }

    fn choose(&mut self, i: usize, ctx: &mut UiContext) -> Transition {
        if !self.selectable(i) {
            return Transition::None;
        }
        let path = crate::save::slot_path(&ctx.save_dir, i as u8);
        match self.mode {
            SlotMode::Save => match crate::save::save_game(&path, ctx.game()) {
                Ok(()) => Transition::Replace(Box::new(MessageBox::text("Game saved."))),
                Err(e) => Transition::Replace(Box::new(MessageBox::text(&format!("Cannot save: {e}")))),
            },
            SlotMode::LoadInGame | SlotMode::LoadTitle => match crate::save::load_game(&path) {
                Ok(g) => {
                    ctx.game = Some(g);
                    if self.mode == SlotMode::LoadInGame {
                        Transition::PopAll(Box::new(crate::ui::mapview::MapView::new()))
                    } else {
                        // the title-screen `LoadGame` parent starts the map view
                        Transition::Pop(DialogResult::Yes)
                    }
                }
                Err(e) => Transition::Replace(Box::new(MessageBox::text(&format!("Cannot load: {e}")))),
            },
        }
    }

    fn move_sel(&mut self, delta: i32) {
        let n = self.slots.len() as i32;
        let mut s = self.sel as i32;
        for _ in 0..n {
            s = (s + delta).rem_euclid(n);
            if self.selectable(s as usize) {
                break;
            }
        }
        self.sel = s as usize;
    }
}

impl Screen for SlotPage {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        match ev {
            Event::Key { key, down: true, .. } => match key {
                Key::Up | Key::Kp(8) => self.move_sel(-1),
                Key::Down | Key::Kp(2) => self.move_sel(1),
                Key::Home => self.sel = 0,
                Key::End => self.sel = self.slots.len() - 1,
                Key::Enter | Key::Space => return self.choose(self.sel, ctx),
                Key::Esc => return Transition::Pop(DialogResult::Cancelled),
                _ => {}
            },
            Event::MouseMove { x, y } => {
                if let Some(i) = self.row_at(*x, *y) {
                    if self.selectable(i) {
                        self.sel = i;
                    }
                }
            }
            Event::MouseButton { button: MouseButton::Left, x, y, down: true } => {
                if let Some(i) = self.row_at(*x, *y) {
                    return self.choose(i, ctx);
                }
            }
            Event::MouseButton { button: MouseButton::Right, down: true, .. } => return Transition::Pop(DialogResult::Cancelled),
            _ => {}
        }
        Transition::None
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let (bar, title_col) = match self.mode {
            SlotMode::LoadTitle => {
                page(fb, ctx, CREAM);
                (col(ctx, TAN), [255, 85, 85])
            }
            _ => {
                page(fb, ctx, WHITE);
                dotted_border(fb, ctx);
                (col(ctx, CYAN), RED)
            }
        };
        let (x, y) = list_origin(self.mode);
        let title = if self.mode == SlotMode::Save { "Select Save File..." } else { "Select Load File..." };
        text(fb, ctx, x, y, title, title_col);
        for i in 0..self.slots.len() {
            let ry = y + LINE_H * (i as i32 + 1);
            if i == self.sel {
                fb.fill_rect(Rect::new(x - 2, ry - 1, 102, LINE_H), bar);
            }
            let colour = if self.selectable(i) { BLACK } else { [138, 138, 142] };
            text(fb, ctx, x + 6, ry, &self.label(i), colour);
        }
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.sp.pal
    }
    fn draws_parent(&self) -> bool {
        false
    }
}

/// Load-game chooser of the title screen.
pub struct LoadGame {
    started: bool,
}

impl Default for LoadGame {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadGame {
    pub fn new() -> LoadGame {
        LoadGame { started: false }
    }
}

impl Screen for LoadGame {
    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        if !self.started {
            self.started = true;
            return Transition::Push(Box::new(SlotPage::new(ctx, SlotMode::LoadTitle)));
        }
        Transition::None
    }
    fn handle(&mut self, _ev: &Event, _ctx: &mut UiContext) -> Transition {
        Transition::None
    }
    fn resume(&mut self, r: DialogResult, _ctx: &mut UiContext) -> Transition {
        match r {
            DialogResult::Yes => Transition::Replace(Box::new(crate::ui::mapview::MapView::new())),
            _ => Transition::Pop(DialogResult::Done),
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        page(fb, ctx, CREAM);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.sp.pal
    }
    fn draws_parent(&self) -> bool {
        false
    }
}
