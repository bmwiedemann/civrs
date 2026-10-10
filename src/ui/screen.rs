//! The screen stack: every full-screen view and every modal dialog implements `Screen`.
//! Dialogs are pushed on top of their parent and pop with a `DialogResult` which the parent
//! receives through `resume`.

use crate::assets::Palette;
use crate::gfx::Surface;
use crate::platform::Event;

#[derive(Clone, Debug, PartialEq)]
pub enum DialogResult {
    Cancelled,
    Choice(usize),
    Text(String),
    Yes,
    No,
    /// Generic "done" result for informational screens.
    Done,
}

pub enum Transition {
    None,
    Push(Box<dyn Screen>),
    Pop(DialogResult),
    Replace(Box<dyn Screen>),
    /// Pop everything and start fresh with this screen.
    PopAll(Box<dyn Screen>),
    Quit,
}

pub trait Screen {
    fn on_enter(&mut self, _ctx: &mut crate::ui::context::UiContext) {}
    fn handle(&mut self, ev: &Event, ctx: &mut crate::ui::context::UiContext) -> Transition;
    /// Called once per frame (60 Hz) before drawing.
    fn update(&mut self, _ctx: &mut crate::ui::context::UiContext) -> Transition {
        Transition::None
    }
    /// Called when a child dialog popped with `result`.
    fn resume(&mut self, _result: DialogResult, _ctx: &mut crate::ui::context::UiContext) -> Transition {
        Transition::None
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut crate::ui::context::UiContext);
    /// The palette this screen wants active; dialogs normally return the parent's.
    fn palette(&self, ctx: &crate::ui::context::UiContext) -> Palette {
        ctx.palette
    }
    /// Dialogs draw over their parent; full screens do not need the parent drawn first.
    fn draws_parent(&self) -> bool {
        true
    }
    /// Whether the mouse cursor should be drawn over this screen.
    fn wants_cursor(&self) -> bool {
        true
    }
}

pub struct ScreenStack {
    stack: Vec<Box<dyn Screen>>,
    quit: bool,
}

impl Default for ScreenStack {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenStack {
    pub fn new() -> Self {
        ScreenStack { stack: Vec::new(), quit: false }
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn push(&mut self, s: Box<dyn Screen>, ctx: &mut crate::ui::context::UiContext) {
        self.stack.push(s);
        if let Some(top) = self.stack.last_mut() {
            top.on_enter(ctx);
        }
    }

    pub fn apply(&mut self, t: Transition, ctx: &mut crate::ui::context::UiContext) {
        match t {
            Transition::None => {}
            Transition::Push(s) => self.push(s, ctx),
            Transition::Pop(r) => {
                self.stack.pop();
                if let Some(top) = self.stack.last_mut() {
                    let t2 = top.resume(r, ctx);
                    self.apply(t2, ctx);
                }
            }
            Transition::Replace(s) => {
                self.stack.pop();
                self.push(s, ctx);
            }
            Transition::PopAll(s) => {
                self.stack.clear();
                self.push(s, ctx);
            }
            Transition::Quit => self.quit = true,
        }
    }

    pub fn handle(&mut self, ev: &Event, ctx: &mut crate::ui::context::UiContext) {
        if let Some(top) = self.stack.last_mut() {
            let t = top.handle(ev, ctx);
            self.apply(t, ctx);
        }
    }

    pub fn update(&mut self, ctx: &mut crate::ui::context::UiContext) {
        if let Some(top) = self.stack.last_mut() {
            let t = top.update(ctx);
            self.apply(t, ctx);
        }
    }

    /// Draw the visible part of the stack (parents first when the top draws over them).
    pub fn draw(&mut self, fb: &mut Surface, ctx: &mut crate::ui::context::UiContext) {
        // find the lowest screen that must be drawn
        let mut start = self.stack.len().saturating_sub(1);
        while start > 0 && self.stack[start].draws_parent() {
            start -= 1;
        }
        for s in self.stack[start..].iter_mut() {
            s.draw(fb, ctx);
        }
    }

    /// The palette of the lowest screen that is actually drawn (dialogs inherit it).
    pub fn palette(&self, ctx: &crate::ui::context::UiContext) -> Palette {
        if self.stack.is_empty() {
            return ctx.palette;
        }
        let mut start = self.stack.len() - 1;
        while start > 0 && self.stack[start].draws_parent() {
            start -= 1;
        }
        self.stack[start].palette(ctx)
    }

    pub fn wants_cursor(&self) -> bool {
        self.stack.last().map(|s| s.wants_cursor()).unwrap_or(false)
    }
}
