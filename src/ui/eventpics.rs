//! Full-screen event pictures and the small news/notice boxes of the map screen:
//! civil disorder (RIOT/RIOT2), "We love the King day" (LOVE1/2), barbarian landings
//! (INVADERS/INVADER2/INVADER3), the nuclear explosion (NUKE1 frames over the map), the
//! "Travelers report:" news box (grey box with a 48x48 scene from SP299.PIC) and the red
//! "CIVILIZATION NOTE" error box.

use crate::assets::Palette;
use crate::gfx::text::{draw_text, text_width, wrap};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::{Event, Key};
use crate::rules::advance_ids as adv;
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::theme;

/// Dismiss logic shared by informational screens: the first frames ignore input so the key
/// that closed the previous dialog is not reused; afterwards any key or click closes.
pub struct Dismiss {
    guard: u32,
}

impl Default for Dismiss {
    fn default() -> Self {
        Self::new()
    }
}

impl Dismiss {
    pub fn new() -> Dismiss {
        Dismiss { guard: 2 }
    }
    pub fn tick(&mut self) {
        self.guard = self.guard.saturating_sub(1);
    }
    /// True when `ev` closes the screen.
    pub fn closes(&self, ev: &Event) -> bool {
        if self.guard > 0 {
            return false;
        }
        match ev {
            Event::Key { down: true, key, .. } => !matches!(key, Key::Unknown),
            Event::MouseButton { down: true, .. } => true,
            _ => false,
        }
    }
}

/// Whether the human has reached the "modern" era pictures (after Invention).
pub fn modern_era(ctx: &UiContext) -> bool {
    ctx.game.as_ref().map(|g| g.human().knows(adv::INVENTION)).unwrap_or(false)
}

/// Colours of a caption box drawn on top of a picture, chosen from the picture's palette.
pub struct CaptionColours {
    pub fill: u8,
    pub border: u8,
    pub text: u8,
    pub title: u8,
}

pub fn caption_colours(pal: &Palette) -> CaptionColours {
    CaptionColours {
        fill: pal.nearest([235, 235, 235]),
        border: pal.nearest([0, 0, 0]),
        text: pal.nearest([0, 0, 0]),
        title: pal.nearest([130, 32, 20]),
    }
}

/// Draw a light caption box with wrapped text (used on every full-screen picture).
/// Returns the box rectangle.
#[allow(clippy::too_many_arguments)]
pub fn draw_caption(fb: &mut Surface, ctx: &UiContext, pal: &Palette, x: i32, y: i32, width: i32, lines: &[String], font: usize) -> Rect {
    let cols = caption_colours(pal);
    let lh = ctx.fonts.get(font).line_height();
    let mut wrapped = Vec::new();
    for l in lines {
        if l.is_empty() {
            wrapped.push(String::new());
        } else {
            wrapped.extend(wrap(&ctx.fonts, font, width - 12, l));
        }
    }
    let mut w = 40;
    for l in &wrapped {
        w = w.max(text_width(&ctx.fonts, font, l) + 12);
    }
    let w = w.min(width);
    let h = 6 + wrapped.len() as i32 * lh;
    let r = Rect::new(x.clamp(0, 320 - w), y.clamp(0, 200 - h), w, h);
    fb.fill_rect(r, cols.fill);
    fb.rect_outline(r, cols.border);
    fb.rect_outline(r.inset(1), cols.fill);
    let mut ty = r.y + 3;
    for l in &wrapped {
        draw_text(fb, &ctx.fonts, &TextStyle::new(font, cols.text), r.x + 6, ty, l);
        ty += lh;
    }
    r
}

/// Caption box centred horizontally.
pub fn draw_caption_centred(fb: &mut Surface, ctx: &UiContext, pal: &Palette, y: i32, width: i32, lines: &[String], font: usize) -> Rect {
    let mut w = 40;
    for l in lines {
        for part in wrap(&ctx.fonts, font, width - 12, l) {
            w = w.max(text_width(&ctx.fonts, font, &part) + 12);
        }
    }
    let w = w.min(width);
    draw_caption(fb, ctx, pal, (320 - w) / 2, y, w, lines, font)
}

/// A full-screen picture with a caption box; any key dismisses it.
pub struct PicScreen {
    pub pic: String,
    pub lines: Vec<String>,
    /// Caption position: None = centred near the top.
    pub caption_pos: Option<(i32, i32)>,
    pub caption_width: i32,
    pub font: usize,
    dismiss: Dismiss,
}

impl PicScreen {
    pub fn new(pic: &str, lines: Vec<String>) -> PicScreen {
        PicScreen { pic: pic.to_string(), lines, caption_pos: None, caption_width: 220, font: 0, dismiss: Dismiss::new() }
    }
    pub fn with_caption_pos(mut self, x: i32, y: i32) -> Self {
        self.caption_pos = Some((x, y));
        self
    }
    pub fn with_caption_width(mut self, w: i32) -> Self {
        self.caption_width = w;
        self
    }
    pub fn with_font(mut self, f: usize) -> Self {
        self.font = f;
        self
    }
}

impl Screen for PicScreen {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.dismiss.tick();
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.dismiss.closes(ev) {
            Transition::Pop(DialogResult::Done)
        } else {
            Transition::None
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let pic = ctx.pic(&self.pic);
        fb.blit(&pic.surf, 0, 0);
        let pal = ctx.pic_pal(&self.pic);
        match self.caption_pos {
            Some((x, y)) => {
                draw_caption(fb, ctx, &pal, x, y, self.caption_width, &self.lines, self.font);
            }
            None => {
                draw_caption_centred(fb, ctx, &pal, 4, self.caption_width, &self.lines, self.font);
            }
        }
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.pic_pal(&self.pic)
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

// ---- crowd pictures (RIOT / LOVE / INVADERS) ------------------------------------------------

/// Layout of a crowd sprite sheet: `cols x rows` cells of `cell_w x cell_h` starting at
/// `origin`, separated by 1-px grid lines.
#[derive(Clone, Copy)]
pub struct SheetGrid {
    pub origin: (i32, i32),
    pub cell: (i32, i32),
    pub cols: i32,
    pub rows: i32,
}

impl SheetGrid {
    pub fn rect(&self, i: i32) -> Rect {
        let c = i % self.cols;
        let r = i / self.cols;
        Rect::new(self.origin.0 + c * (self.cell.0 + 1), self.origin.1 + r * (self.cell.1 + 1), self.cell.0, self.cell.1)
    }
}

/// Grid of a crowd sheet (measured from the pictures).
pub fn crowd_grid(sheet: &str) -> SheetGrid {
    match sheet {
        "RIOT" => SheetGrid { origin: (1, 1), cell: (78, 63), cols: 4, rows: 3 },
        "RIOT2" => SheetGrid { origin: (1, 1), cell: (74, 65), cols: 4, rows: 3 },
        "INVADERS" | "INVADER2" => SheetGrid { origin: (0, 2), cell: (78, 60), cols: 4, rows: 3 },
        // LOVE1, LOVE2, INVADER3
        _ => SheetGrid { origin: (1, 1), cell: (78, 65), cols: 4, rows: 3 },
    }
}

/// Frames of the sheet that actually contain a figure (non-key pixels).
fn crowd_frames(surf: &Surface, grid: &SheetGrid, key: u8) -> Vec<Rect> {
    let mut out = Vec::new();
    for i in 0..grid.cols * grid.rows {
        let r = grid.rect(i);
        let mut used = false;
        'scan: for y in (r.y..r.bottom()).step_by(3) {
            for x in (r.x..r.right()).step_by(3) {
                if surf.get(x, y) != key {
                    used = true;
                    break 'scan;
                }
            }
        }
        if used {
            out.push(r);
        }
    }
    out
}

/// Crowd marching across the city-view meadow (CBACK.PIC) with a caption.
pub struct CrowdScreen {
    sheet: String,
    lines: Vec<String>,
    t: u64,
    dismiss: Dismiss,
    frames: Vec<Rect>,
    key: u8,
}

pub fn crowd_screen(sheet: &str, lines: Vec<String>) -> Box<dyn Screen> {
    Box::new(CrowdScreen { sheet: sheet.to_string(), lines, t: 0, dismiss: Dismiss::new(), frames: Vec::new(), key: 0 })
}

/// Civil disorder picture: "Civil disorder in CITY! Mayor flees in panic."
pub fn disorder_screen(ctx: &UiContext, city: &str) -> Box<dyn Screen> {
    let sheet = if ctx.frame.is_multiple_of(2) { "RIOT" } else { "RIOT2" };
    crowd_screen(sheet, vec![format!("Civil disorder in {city}!"), "Mayor flees in panic.".into()])
}

/// "We love the King day" celebration picture.
pub fn celebration_screen(ctx: &UiContext, city: &str) -> Box<dyn Screen> {
    let title = ctx.game.as_ref().map(|g| g.human().title()).unwrap_or("King");
    let sheet = if modern_era(ctx) { "LOVE2" } else { "LOVE1" };
    crowd_screen(sheet, vec![format!("We love the {title} day"), format!("celebrated in {city}.")])
}

/// Barbarian landing / uprising picture by era.
pub fn invaders_screen(ctx: &UiContext, lines: Vec<String>) -> Box<dyn Screen> {
    let g = ctx.game.as_ref();
    let sheet = match g {
        Some(g) if g.human().knows(adv::CONSCRIPTION) => "INVADERS",
        Some(g) if g.human().knows(adv::GUNPOWDER) => "INVADER2",
        _ => "INVADER3",
    };
    crowd_screen(sheet, lines)
}

impl Screen for CrowdScreen {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        let pic = ctx.pic(&self.sheet);
        let grid = crowd_grid(&self.sheet);
        let r0 = grid.rect(0);
        self.key = pic.surf.get(r0.x, r0.y);
        self.frames = crowd_frames(&pic.surf, &grid, self.key);
    }
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.t += 1;
        self.dismiss.tick();
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.dismiss.closes(ev) {
            Transition::Pop(DialogResult::Done)
        } else {
            Transition::None
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let back = ctx.pic("CBACK");
        let pic = ctx.pic(&self.sheet);
        // palette: the sheet's own (CBACK is remapped to it)
        let sheet_pal = ctx.pic_pal(&self.sheet);
        let back_pal = ctx.pic_pal("CBACK");
        let mut map = [0u8; 256];
        for (i, m) in map.iter_mut().enumerate() {
            *m = sheet_pal.nearest(back_pal.rgb(i as u8));
        }
        fb.blit_remap(&back.surf, back.surf.bounds(), 0, 0, None, &map);
        if !self.frames.is_empty() {
            let n = self.frames.len() as i32;
            let step = (self.t / 6) as i32;
            let shift = (self.t as i32 * 2) % 60;
            // two rows of marchers, the back row smaller offset
            for row in 0..2 {
                let y = 108 + row * 26;
                let mut i = 0;
                let mut x = -shift - row * 30;
                while x < 320 {
                    let f = self.frames[((i + step + row * 3) % n) as usize];
                    fb.blit_part(&pic.surf, f, x, y, Some(self.key));
                    x += 60;
                    i += 1;
                }
            }
        }
        draw_caption_centred(fb, ctx, &sheet_pal, 6, 240, &self.lines, 0);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.pic_pal(&self.sheet)
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

// ---- nuclear explosion ------------------------------------------------------------------------

/// NUKE1.PIC: 7x4 frames of 44x44 in 45-px cells.
pub const NUKE_GRID: SheetGrid = SheetGrid { origin: (1, 1), cell: (44, 44), cols: 7, rows: 4 };
const NUKE_FRAMES: i32 = 28;

pub struct NukeAnimation {
    cx: i32,
    cy: i32,
    t: u64,
}

/// Explosion animation centred on screen pixel (`cx`, `cy`), drawn over the map.
pub fn nuke_animation(cx: i32, cy: i32) -> Box<dyn Screen> {
    Box::new(NukeAnimation { cx, cy, t: 0 })
}

/// Explosion over map tile (`x`, `y`) given the map view origin (`vx`, `vy`).
pub fn nuke_animation_at_tile(x: i32, y: i32, vx: i32, vy: i32) -> Box<dyn Screen> {
    use crate::ui::mapview::{VIEW_X, VIEW_Y};
    let px = VIEW_X + (x - vx) * crate::ui::tilerender::TILE + 8;
    let py = VIEW_Y + (y - vy) * crate::ui::tilerender::TILE + 8;
    nuke_animation(px, py)
}

impl Screen for NukeAnimation {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.t += 1;
        if self.t > (NUKE_FRAMES as u64) * 3 + 10 {
            return Transition::Pop(DialogResult::Done);
        }
        Transition::None
    }
    fn handle(&mut self, _ev: &Event, _ctx: &mut UiContext) -> Transition {
        Transition::None
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let frame = ((self.t / 3) as i32).min(NUKE_FRAMES - 1);
        let nuke = ctx.pic("NUKE1");
        let r = NUKE_GRID.rect(frame);
        // remap the explosion colours into the active (map) palette
        let src_pal = ctx.pic_pal("NUKE1");
        let dst_pal = ctx.palette;
        let mut map = [0u8; 256];
        for (i, m) in map.iter_mut().enumerate() {
            *m = dst_pal.nearest(src_pal.rgb(i as u8));
        }
        let key = nuke.surf.get(r.x, r.y);
        let clip = Rect::new(crate::ui::mapview::VIEW_X, crate::ui::mapview::VIEW_Y, 240, 192);
        fb.set_clip(Some(clip));
        fb.blit_remap(&nuke.surf, r, self.cx - 22, self.cy - 22, Some(key), &map);
        fb.set_clip(None);
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

// ---- news box ---------------------------------------------------------------------------------

/// Scenes in SP299.PIC used at the left of report boxes (41x50 panels).
pub const SCENE_SPY: Rect = Rect::new(160, 150, 41, 50);
pub const SCENE_EMBASSY: Rect = Rect::new(202, 150, 41, 50);
pub const SCENE_TRAVELERS: Rect = Rect::new(244, 150, 41, 50);

/// Grey "Travelers report:" box over the map: cyan underlined title, white text and a scene
/// picture at the left.
pub struct NewsBox {
    pub title: String,
    pub lines: Vec<String>,
    pub scene: Rect,
    rect: Rect,
    wrapped: Vec<String>,
    dismiss: Dismiss,
}

pub fn news_box(title: &str, lines: Vec<String>) -> Box<dyn Screen> {
    Box::new(NewsBox { title: title.to_string(), lines, scene: SCENE_TRAVELERS, rect: Rect::default(), wrapped: Vec::new(), dismiss: Dismiss::new() })
}

pub fn travelers_report(lines: Vec<String>) -> Box<dyn Screen> {
    news_box("Travelers report:", lines)
}

impl Screen for NewsBox {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        let text_w = 150;
        self.wrapped.clear();
        for l in &self.lines {
            if l.is_empty() {
                self.wrapped.push(String::new());
            } else {
                self.wrapped.extend(wrap(&ctx.fonts, 0, text_w, l));
            }
        }
        let mut w = text_width(&ctx.fonts, 0, &self.title) + 16;
        for l in &self.wrapped {
            w = w.max(text_width(&ctx.fonts, 0, l) + 16);
        }
        let w = w + self.scene.w + 8;
        let h = (12 + 8 + self.wrapped.len() as i32 * 8).max(self.scene.h + 8);
        self.rect = Rect::new(80 + (240 - w) / 2, 8 + (192 - h) / 2, w, h);
    }
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.dismiss.tick();
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.dismiss.closes(ev) {
            Transition::Pop(DialogResult::Done)
        } else {
            Transition::None
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        crate::ui::widgets::draw_panel(fb, ctx, self.rect);
        // scene picture remapped into the map palette
        let sp = ctx.pic("SP299");
        let src_pal = ctx.pic_pal("SP299");
        let mut map = [0u8; 256];
        for (i, m) in map.iter_mut().enumerate() {
            *m = ctx.palette.nearest(src_pal.rgb(i as u8));
        }
        let sx = self.rect.x + 4;
        let sy = self.rect.y + (self.rect.h - self.scene.h) / 2;
        fb.blit_remap(&sp.surf, self.scene, sx, sy, None, &map);
        let tx = sx + self.scene.w + 6;
        let mut y = self.rect.y + 4;
        let end = draw_text(fb, &ctx.fonts, &TextStyle::new(0, theme::CYAN), tx, y, &self.title);
        fb.hline(tx, end, y + 7, theme::CYAN);
        y += 12;
        for l in &self.wrapped {
            draw_text(fb, &ctx.fonts, &TextStyle::new(0, theme::WHITE), tx, y, l);
            y += 8;
        }
    }
}

// ---- red note ---------------------------------------------------------------------------------

/// Red "CIVILIZATION NOTE" box (errors): the green note's layout with border 247,85,85 and
/// fill 130,32,20.
pub struct RedNote {
    pub lines: Vec<String>,
    rect: Rect,
    wrapped: Vec<String>,
    dismiss: Dismiss,
}

pub fn red_note(lines: Vec<String>) -> Box<dyn Screen> {
    Box::new(RedNote { lines, rect: Rect::new(56, 16, 209, 41), wrapped: Vec::new(), dismiss: Dismiss::new() })
}

impl Screen for RedNote {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        (self.rect, self.wrapped) = crate::ui::widgets::note_layout(ctx, &self.lines);
    }
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.dismiss.tick();
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.dismiss.closes(ev) {
            Transition::Pop(DialogResult::Done)
        } else {
            Transition::None
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let pal = &ctx.palette;
        let fill = pal.nearest([130, 32, 20]);
        let border = pal.nearest([247, 85, 85]);
        let white = pal.nearest([235, 235, 235]);
        fb.fill_rect(self.rect, fill);
        fb.rect_outline(self.rect, border);
        crate::ui::widgets::draw_note_text(fb, ctx, self.rect, &self.wrapped, white);
    }
}

// ---- headless preview helper -----------------------------------------------------------------

/// A map view that pushes `child` over itself on its first frame (used by `--dump` to render
/// the map-overlay boxes above a real map).
pub struct OverMap {
    map: crate::ui::mapview::MapView,
    child: Option<Box<dyn Screen>>,
}

pub fn over_map(child: Box<dyn Screen>) -> Box<dyn Screen> {
    Box::new(OverMap { map: crate::ui::mapview::MapView::new(), child: Some(child) })
}

impl Screen for OverMap {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        self.map.on_enter(ctx);
    }
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        match self.child.take() {
            Some(c) => Transition::Push(c),
            None => Transition::None,
        }
    }
    fn handle(&mut self, _ev: &Event, _ctx: &mut UiContext) -> Transition {
        Transition::None
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        self.map.draw(fb, ctx);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        self.map.palette(ctx)
    }
    fn draws_parent(&self) -> bool {
        false
    }
}
