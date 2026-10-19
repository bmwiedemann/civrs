//! The palace: HILL.PIC meadow with the palace composed from CASTLE{level}.PIC parts
//! (row 0 = castle/classical style, row 1 = oriental style) and CBRUSH hedges / statues in
//! the foreground for the higher levels. The view dissolves in (driver duration 20, as in
//! the original).

use crate::assets::Palette;
use crate::gfx::{Rect, Surface};
use crate::platform::Event;
use crate::ui::context::UiContext;
use crate::ui::eventpics::{draw_caption_centred, Dismiss};
use crate::ui::reveal::Reveal;
use crate::ui::screen::{DialogResult, Screen, Transition};

/// Column spans of the CASTLE sheets (x0, width); both rows share them.
const CELLS: [(i32, i32); 8] = [(0, 52), (53, 49), (103, 28), (132, 27), (160, 35), (196, 16), (213, 49), (263, 56)];

/// Part `cell` of style row `style` (0 = castle, 1 = oriental) in a CASTLE sheet.
pub fn castle_part(style: u8, cell: usize) -> Rect {
    let (x, w) = CELLS[cell.min(7)];
    Rect::new(x, if style == 0 { 1 } else { 101 }, w, 99)
}

/// Baseline of the palace on the hill.
const GROUND: i32 = 150;
/// Driver duration of the dissolve (overlay call site: `push 20; push 1; call dissolve`).
const DISSOLVE: u32 = 20;

pub struct PalaceScreen {
    level: u8,
    style: u8,
    lines: Vec<String>,
    dismiss: Dismiss,
    reveal: Reveal,
}

/// The palace of the human player with an optional caption (PalaceAddition text).
pub fn palace_screen(ctx: &UiContext, lines: Vec<String>) -> Box<dyn Screen> {
    let (level, style) = ctx
        .game
        .as_ref()
        .map(|g| {
            let p = g.human();
            (p.palace_level.min(4), p.palace_parts.first().copied().unwrap_or(0) % 2)
        })
        .unwrap_or((0, 0));
    Box::new(PalaceScreen { level, style, lines, dismiss: Dismiss::new(), reveal: Reveal::new(ctx, DISSOLVE) })
}

/// Palace shown after a "palace addition" event, with the KING.TXT PALACE text.
pub fn palace_addition(ctx: &UiContext) -> Box<dyn Screen> {
    let lines = ctx.texts.king.section("PALACE").map(crate::assets::text::paragraphs).unwrap_or_default();
    palace_screen(ctx, lines)
}

/// Draw the palace of `level` in `style` on top of what is already in `fb`.
pub fn draw_palace(fb: &mut Surface, ctx: &UiContext, pal: &Palette, level: u8, style: u8) {
    let sheet = format!("CASTLE{}", level.min(4));
    let pic = ctx.pic(&sheet);
    let sheet_pal = ctx.pic_pal(&sheet);
    let mut map = [0u8; 256];
    for (i, m) in map.iter_mut().enumerate() {
        *m = pal.nearest(sheet_pal.rgb(i as u8));
    }
    let key = pic.surf.get(10, 5);
    // tower, wall, gate, wall, tower from left to right (centred)
    let order: [usize; 5] = [2, 1, 0, 1, 3];
    let total: i32 = order.iter().map(|c| CELLS[*c].1).sum();
    let mut x = (320 - total) / 2;
    for c in order {
        let r = castle_part(style, c);
        fb.blit_remap(&pic.surf, r, x, GROUND - r.h, Some(key), &map);
        x += r.w;
    }
    // foreground gardens
    let brushes: Option<(&str, &str)> = match level {
        0 | 1 => None,
        2 => Some(("CBRUSH0", "CBRUSH1")),
        3 => Some(("CBRUSH2", "CBRUSH3")),
        _ => Some(("CBRUSH4", "CBRUSH5")),
    };
    if let Some((l, r)) = brushes {
        for (name, x) in [(l, 0), (r, 184)] {
            let b = ctx.pic(name);
            let bp = ctx.pic_pal(name);
            let mut bmap = [0u8; 256];
            for (i, m) in bmap.iter_mut().enumerate() {
                *m = pal.nearest(bp.rgb(i as u8));
            }
            let key = b.surf.get(0, 0);
            fb.blit_remap(&b.surf, b.surf.bounds(), x, 200 - b.surf.h, Some(key), &bmap);
        }
    }
}

impl Screen for PalaceScreen {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.dismiss.tick();
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if !self.dismiss.closes(ev) {
            Transition::None
        } else if self.reveal.done() {
            Transition::Pop(DialogResult::Done)
        } else {
            self.reveal.finish();
            Transition::None
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let pal = ctx.pic_pal("HILL");
        let mut content = Surface::screen();
        let hill = ctx.pic("HILL");
        content.blit(&hill.surf, 0, 0);
        draw_palace(&mut content, ctx, &pal, self.level, self.style);
        if !self.lines.is_empty() {
            draw_caption_centred(&mut content, ctx, &pal, 4, 260, &self.lines, 0);
        }
        let old = ctx.palette;
        self.reveal.draw(fb, &content, &old, &pal);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.pic_pal("HILL")
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}
