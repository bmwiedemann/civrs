//! Load / save screens as overlay 11 draws them (`load_game` o 0xed75, `save_game` o 0xf0df,
//! `ask_save_drive` o 0xf36d, `read_save_header` o 0xee78).
//!
//! Both start with the drive question on a page filled with colour 15 (saving adds the random
//! SP299 page border): "Which drive contains your saved game files?" (load) or "... Save Game
//! disk?" (save) as a text block at (80, 72). A..F pick the drive letter, Enter goes on, Esc
//! cancels. A: and B: are floppies, which civrs does not have: "No Disk in Drive A.". Then the
//! slots are listed by a frameless `popup_menu3` with the title in colour 12 (the "\x8c" code):
//! loading lists CIVIL0..9 at (48, 65), saving CIVIL0..3 at (48, 33) with the last slot used
//! preselected. A slot reads " <difficulty> <leader>, <civ>/<year> BC" or " (EMPTY)". Saving ends
//! with " Game has been saved." / " Press key to continue." at (64, 127).
//!
//! Loading from the GAME menu does not exist in CIV.EXE; civrs offers it with the save screens'
//! look.

use crate::assets::Palette;
use crate::gfx::text::draw_text;
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::{Event, Key};
use crate::save::SlotInfo;
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::{MenuDialog, MessageBox, PanelStyle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotMode {
    /// GAME menu: save into a slot.
    Save,
    /// GAME menu: load a slot and restart the map view (a civrs addition).
    LoadInGame,
    /// Title screen: load a slot.
    LoadTitle,
}

/// Window font 1 of the title screen and the game.
const FONT: usize = 0;
/// Page colour (15) and text colour (0).
const PAGE: u8 = 15;
const INK: u8 = 0;
/// "Game NOT saved." sits on a colour-12 box (o 0xf1cd).
const ERROR_BOX: Rect = Rect::new(64, 127, 192, 34);

/// The slot menu line of a save (o 0xef06..0xf049).
pub fn slot_line(info: &Option<SlotInfo>) -> String {
    match info {
        Some(i) => {
            let era = if i.year < 0 { "BC" } else { "AD" };
            format!("{} {}, {}/{} {era}", crate::rules::DIFFICULTY[i.difficulty as usize].short, i.leader, i.civ, i.year.abs())
        }
        None => "(EMPTY)".to_string(),
    }
}

/// Drive question text (ds:0x4295 / 0x42d5, whose last blank the drive letter replaces, then
/// ds:0x4314 + ds:0x4351).
fn drive_text(saving: bool, drive: char) -> Vec<String> {
    let head = if saving { "  Which drive contains your\n     Save Game disk?\n\n           " } else { "  Which drive contains your\n    saved game files?\n\n           " };
    format!("{head}{drive}:\n\n    Press drive letter and\nReturn when disk is inserted.\n    Press Escape to cancel.").lines().map(str::to_string).collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Drive,
    Slots,
    NoDisk,
    /// write_save (o 0xf264) shows its box, then writes slot n.
    Writing(usize),
    Result,
}

/// The whole load or save sequence.
pub struct SlotPage {
    mode: SlotMode,
    step: Step,
    drive: char,
    border: u8,
    slots: Vec<Option<SlotInfo>>,
    /// Save result lines and whether it failed.
    result: Option<(Vec<String>, bool)>,
    started: bool,
}

impl SlotPage {
    pub fn new(mode: SlotMode) -> SlotPage {
        SlotPage { mode, step: Step::Drive, drive: 'C', border: 0, slots: Vec::new(), result: None, started: false }
    }

    fn saving(&self) -> bool {
        self.mode == SlotMode::Save
    }

    fn bordered(&self) -> bool {
        self.mode != SlotMode::LoadTitle
    }

    fn slot_menu(&self, ctx: &UiContext) -> Transition {
        let title = if self.saving() { "Select Save File..." } else { "Select Load File..." };
        let items: Vec<String> = self.slots.iter().map(slot_line).collect();
        let (y, sel) = if self.saving() { (33, ctx.last_slot.unwrap_or(0).min(3)) } else { (65, 0) };
        let m = MenuDialog::new(vec![title.to_string()], items).with_pos(48, y).with_style(PanelStyle::Bare).with_font(FONT).with_top_pad(5).with_title_colour(12).with_selection(sel);
        Transition::Push(Box::new(m))
    }

    fn chosen(&mut self, i: usize, ctx: &mut UiContext) -> Transition {
        let path = crate::save::slot_path(&ctx.save_dir, i as u8);
        if self.saving() {
            ctx.last_slot = Some(i);
            self.step = Step::Writing(i);
            return Transition::None;
        }
        // an empty slot ends the load like Esc (o 0xee1b)
        if self.slots.get(i).is_none_or(|s| s.is_none()) {
            return Transition::Pop(DialogResult::Cancelled);
        }
        match crate::save::load_game(&path) {
            Ok(g) => {
                ctx.game = Some(g);
                ctx.last_slot = Some(i);
                if self.mode == SlotMode::LoadInGame {
                    Transition::PopAll(Box::new(crate::ui::mapview::MapView::new()))
                } else {
                    Transition::Replace(Box::new(crate::ui::mapview::MapView::new()))
                }
            }
            Err(e) => Transition::Replace(Box::new(MessageBox::text(&format!("File Error: {e}")))),
        }
    }
}

impl SlotPage {
    /// write_save's box (o 0xf264), drawn without input (ds:0xdb38) at (64, 86) and left on
    /// the page: " <drive>:CIVILn.SVE" (the drive letter in lower case, o 0xf541), then
    /// " <difficulty title> <leader>", " <tribe>/<year>" and " ... save in progress.".
    fn draw_writing(&self, fb: &mut Surface, ctx: &mut UiContext) {
        let Some(n) = ctx.last_slot else { return };
        let Some(g) = ctx.game.as_ref() else { return };
        let h = g.human();
        let title = format!(" {}:CIVIL{n}.SVE", self.drive.to_ascii_lowercase());
        let items = vec![
            format!(" {} {}", crate::rules::DIFFICULTY[g.setup.difficulty as usize].short, h.leader),
            format!(" {}/{}", h.plural(), crate::rules::year_text(g.year())),
            " ... save in progress.".to_string(),
        ];
        MenuDialog::new(vec![title], items).with_pos(64, 86).with_font(FONT).paint(fb, ctx);
    }
}

impl Screen for SlotPage {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        self.slots = crate::save::list_slots(&ctx.save_dir);
        if self.saving() {
            self.slots.truncate(4);
        }
        // random(2) in the original; the frame counter keeps headless renders repeatable
        self.border = (ctx.frame.wrapping_mul(0x9e37_79b9) >> 13) as u8 & 1;
    }

    fn update(&mut self, ctx: &mut UiContext) -> Transition {
        self.started = true;
        // the box has been on the screen for a frame: write the file
        if let Step::Writing(i) = self.step {
            let path = crate::save::slot_path(&ctx.save_dir, i as u8);
            let (lines, failed) = match crate::save::save_game(&path, ctx.game()) {
                Ok(()) => (vec![" Game has been saved.".to_string(), " Press key to continue.".to_string()], false),
                Err(e) => (vec![" Game NOT saved.".to_string(), format!(" {e}"), " Press key to continue.".to_string()], true),
            };
            self.result = Some((lines, failed));
            self.step = Step::Result;
        }
        Transition::None
    }

    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if !self.started {
            return Transition::None;
        }
        let Event::Key { key, down: true, .. } = ev else {
            if matches!(ev, Event::MouseButton { down: true, .. }) && self.step == Step::Result {
                return Transition::Pop(DialogResult::Done);
            }
            return Transition::None;
        };
        match self.step {
            Step::Drive => match key {
                Key::Char(c @ 'a'..='f') => self.drive = c.to_ascii_uppercase(),
                Key::Esc => return Transition::Pop(DialogResult::Cancelled),
                Key::Enter if matches!(self.drive, 'A' | 'B') => {
                    self.step = Step::NoDisk;
                    let msg = format!("No Disk in Drive {}.", self.drive);
                    return Transition::Push(Box::new(MessageBox::text(&msg).with_pos(100, 80)));
                }
                Key::Enter => {
                    self.step = Step::Slots;
                    return self.slot_menu(ctx);
                }
                _ => {}
            },
            Step::Result => return Transition::Pop(DialogResult::Done),
            Step::Slots | Step::NoDisk | Step::Writing(_) => {}
        }
        Transition::None
    }

    fn resume(&mut self, r: DialogResult, ctx: &mut UiContext) -> Transition {
        match (self.step, r) {
            (Step::Slots, DialogResult::Choice(i)) => self.chosen(i, ctx),
            _ => Transition::Pop(DialogResult::Cancelled),
        }
    }

    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        fb.fill(PAGE);
        if self.bordered() {
            crate::ui::pedia_entry::draw_border(fb, ctx, self.border);
        }
        let pitch = ctx.fonts.get(FONT).line_height();
        match self.step {
            Step::Drive | Step::NoDisk => {
                for (i, l) in drive_text(self.saving(), self.drive).iter().enumerate() {
                    draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, INK), 80, 72 + i as i32 * pitch, l);
                }
            }
            Step::Writing(_) | Step::Result => {
                self.draw_writing(fb, ctx);
                if let Some((lines, failed)) = &self.result {
                    if *failed {
                        fb.fill_rect(ERROR_BOX, 12);
                    }
                    // a frameless popup at (64, 127): title line, then the rest
                    for (i, l) in lines.iter().enumerate() {
                        let colour = if i == 0 && !*failed { 3 } else { INK };
                        draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, colour), 69, 132 + i as i32 * pitch, l);
                    }
                }
            }
            Step::Slots => {}
        }
    }

    fn palette(&self, ctx: &UiContext) -> Palette {
        // loading from the title keeps the title's LOGO.PIC palette
        if self.mode == SlotMode::LoadTitle {
            ctx.pic("LOGO").pal
        } else {
            ctx.game_palette()
        }
    }

    fn draws_parent(&self) -> bool {
        false
    }

    fn wants_cursor(&self) -> bool {
        self.step == Step::Slots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_lines_as_the_original_builds_them() {
        let info = SlotInfo { slot: 0, leader: "Caesar".into(), civ: "Romans".into(), year: -3340, difficulty: crate::rules::Difficulty::Chieftain };
        assert_eq!(slot_line(&Some(info)), "Chief Caesar, Romans/3340 BC");
        assert_eq!(slot_line(&None), "(EMPTY)");
    }
}
