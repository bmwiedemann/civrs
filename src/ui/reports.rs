//! Advisor reports (F1-F12) and the GAME menu dialogs (tax/luxury rates, FindCity, Options,
//! Save, Load), in the layouts of the original.
//!
//! The reports themselves live in `report_advisors` (F1-F6) and `report_world` (F7-F12); this
//! module holds the entry points used by the map view and the drawing helpers they share:
//! the solid-colour page, the three-line title block, the advisor portrait (GOVTnX.PIC) and
//! the red/green dotted page border of the save/load pages.

use crate::assets::Palette;
use crate::assets::fonts::GLYPH_SPACING;
use crate::gfx::text::{draw_text, ink_width};
use crate::gfx::{Rect, Surface, TextStyle};
use crate::platform::{Event, Key, MouseButton};
use crate::ui::context::UiContext;
use crate::ui::screen::{DialogResult, Screen, Transition};
use crate::ui::widgets::{MenuDialog, MessageBox};

/// Standard 7-px UI font.
pub const FONT: usize = 0;
/// Small caps font.
pub const TINY: usize = 1;
pub const LINE_H: i32 = 8;

/// Font 0 icon glyphs.
pub const GLYPH_LUX: char = '\u{5c}';
pub const GLYPH_CHECK: char = '\u{5e}';
pub const GLYPH_COIN: char = '$';
pub const GLYPH_FOOD: char = '\u{7b}';
pub const GLYPH_SHIELD: char = '\u{7c}';
pub const GLYPH_TRADE: char = '\u{7d}';
pub const GLYPH_BULB: char = '\u{7e}';

// ---- colours ---------------------------------------------------------------------------------

pub const WHITE: [u8; 3] = [235, 235, 235];
pub const BLACK: [u8; 3] = [0, 0, 0];
pub const GREY: [u8; 3] = [138, 138, 142];
pub const DARK_GREY: [u8; 3] = [77, 77, 77];
pub const CYAN: [u8; 3] = [12, 227, 235];
pub const LIGHT_GREEN: [u8; 3] = [97, 227, 101];
pub const PALE_YELLOW: [u8; 3] = [255, 255, 150];
pub const RED: [u8; 3] = [247, 85, 85];
pub const CREAM: [u8; 3] = [255, 255, 223];
pub const TAN: [u8; 3] = [219, 206, 166];
pub const BROWN: [u8; 3] = [162, 125, 97];
/// Page backgrounds.
pub const BG_F1: [u8; 3] = [77, 77, 77];
pub const BG_BLUE: [u8; 3] = [49, 77, 178];
pub const BG_LIGHT_BLUE: [u8; 3] = [121, 142, 255];
pub const BG_GREEN: [u8; 3] = [45, 121, 0];
pub const BG_TEAL: [u8; 3] = [0, 170, 170];

#[inline]
pub fn col(ctx: &UiContext, rgb: [u8; 3]) -> u8 {
    ctx.sp.pal.nearest(rgb)
}

fn spaced_run(fb: &mut Surface, ctx: &UiContext, colour: u8, x: i32, y: i32, s: &str) -> i32 {
    draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, colour), x, y, s) - GLYPH_SPACING
}

/// Width of `s` in the report font.
pub fn width(ctx: &UiContext, s: &str) -> i32 {
    ink_width(&ctx.fonts, FONT, s)
}

/// Spaced report text; returns the x after the last glyph.
pub fn text(fb: &mut Surface, ctx: &UiContext, x: i32, y: i32, s: &str, rgb: [u8; 3]) -> i32 {
    spaced_run(fb, ctx, col(ctx, rgb), x, y, s)
}

/// Spaced text with a shadow at (+1,+1).
pub fn text_sh(fb: &mut Surface, ctx: &UiContext, x: i32, y: i32, s: &str, rgb: [u8; 3], shadow: [u8; 3]) -> i32 {
    spaced_run(fb, ctx, col(ctx, shadow), x + 1, y + 1, s);
    spaced_run(fb, ctx, col(ctx, rgb), x, y, s)
}

pub fn text_c(fb: &mut Surface, ctx: &UiContext, cx: i32, y: i32, s: &str, rgb: [u8; 3]) -> i32 {
    let w = width(ctx, s);
    text(fb, ctx, cx - w / 2, y, s, rgb)
}

pub fn text_c_sh(fb: &mut Surface, ctx: &UiContext, cx: i32, y: i32, s: &str, rgb: [u8; 3], shadow: [u8; 3]) -> i32 {
    let w = width(ctx, s);
    text_sh(fb, ctx, cx - w / 2, y, s, rgb, shadow)
}

/// Right-aligned spaced text.
pub fn text_r(fb: &mut Surface, ctx: &UiContext, rx: i32, y: i32, s: &str, rgb: [u8; 3]) -> i32 {
    let w = width(ctx, s);
    text(fb, ctx, rx - w, y, s, rgb)
}

pub fn tiny(fb: &mut Surface, ctx: &UiContext, x: i32, y: i32, s: &str, rgb: [u8; 3]) -> i32 {
    draw_text(fb, &ctx.fonts, &TextStyle::new(TINY, col(ctx, rgb)), x, y, s)
}

/// Fill the whole screen with a page colour.
pub fn page(fb: &mut Surface, ctx: &UiContext, bg: [u8; 3]) {
    fb.fill(col(ctx, bg));
}

/// The three centred title lines of an advisor report: "CITY STATUS" / "Empire of the Romans" /
/// "Emperor Caesar: 4000 BC" at y 2, 10, 18.
pub fn title_block(fb: &mut Surface, ctx: &UiContext, title: &str) {
    let g = ctx.game();
    let p = g.human();
    text_c(fb, ctx, 160, 2, title, WHITE);
    text_c(fb, ctx, 160, 10, &format!("Empire of the {}", p.plural()), WHITE);
    text_c(fb, ctx, 160, 18, &format!("{} {}: {}", p.title(), p.leader, g.year_text()), WHITE);
}

/// Colour remap table turning indices of palette `from` into the nearest entries of `to`.
pub fn remap_table(from: &Palette, to: &Palette) -> [u8; 256] {
    let mut m = [0u8; 256];
    for (i, c) in from.0.iter().enumerate() {
        m[i] = to.nearest(*c);
    }
    m
}

/// Advisors in GOVTnX.PIC (40x60 cells across the top): 0 military, 1 domestic/trade,
/// 2 foreign, 3 science.
pub fn advisor_pic_name(ctx: &UiContext) -> String {
    let g = ctx.game();
    let p = g.human();
    let group = match p.govt {
        crate::rules::Govt::Anarchy | crate::rules::Govt::Despotism => 0,
        crate::rules::Govt::Monarchy => 1,
        crate::rules::Govt::Republic | crate::rules::Govt::Democracy => 2,
        crate::rules::Govt::Communism => 3,
    };
    let modern = p.knows(crate::rules::advance_ids::INVENTION) && group != 3;
    format!("GOVT{group}{}", if modern { 'M' } else { 'A' })
}

/// Draw advisor `idx` (0..3) in the top-right corner at (278,2), 40x60, remapped to the
/// sprite-sheet palette the report pages use.
pub fn portrait(fb: &mut Surface, ctx: &UiContext, idx: i32) {
    let name = advisor_pic_name(ctx);
    let pic = ctx.pic(&name);
    if pic.surf.w < 160 {
        return;
    }
    let map = remap_table(&pic.pal, &ctx.sp.pal);
    fb.blit_remap(&pic.surf, Rect::new(idx * 40, 0, 40, 60), 278, 2, None, &map);
}

// ---- dotted red/green page border (save/load pages, civilopedia entries) ---------------------

const BORDER_COLS: [[u8; 3]; 6] = [[45, 121, 0], [247, 85, 85], [130, 32, 20], [255, 255, 150], [0, 0, 0], [235, 235, 235]];
const EDGE_H: [[u8; 8]; 8] = [
    [1, 0, 1, 0, 1, 0, 1, 0],
    [1, 1, 1, 1, 1, 1, 1, 1],
    [0, 3, 0, 0, 0, 0, 0, 0],
    [0, 0, 5, 3, 5, 3, 0, 0],
    [0, 0, 3, 5, 3, 5, 0, 0],
    [0, 0, 0, 0, 0, 0, 3, 0],
    [1, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 0, 1, 0, 1, 0, 1],
];
const EDGE_V: [[u8; 8]; 8] = [
    [0, 1, 0, 0, 0, 0, 1, 1],
    [1, 1, 0, 0, 0, 3, 1, 0],
    [0, 1, 0, 3, 5, 0, 1, 1],
    [1, 1, 0, 5, 3, 0, 1, 0],
    [0, 1, 0, 3, 5, 0, 1, 1],
    [1, 1, 0, 5, 3, 0, 1, 0],
    [0, 1, 3, 0, 0, 0, 1, 1],
    [1, 1, 0, 0, 0, 0, 1, 0],
];
const CORNER_TL: [[u8; 8]; 8] = [
    [0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 1, 1, 1],
    [0, 1, 2, 1, 2, 1, 2, 1],
    [0, 1, 1, 0, 3, 4, 5, 1],
    [0, 1, 2, 3, 0, 4, 1, 5],
    [0, 1, 1, 4, 4, 4, 4, 4],
    [0, 1, 2, 1, 5, 4, 0, 0],
    [0, 1, 1, 5, 1, 4, 0, 5],
];
const CORNER_TR: [[u8; 8]; 8] = [
    [0, 0, 0, 0, 0, 0, 0, 0],
    [1, 1, 1, 1, 1, 1, 1, 0],
    [1, 2, 1, 2, 1, 2, 1, 0],
    [5, 1, 4, 3, 0, 1, 1, 0],
    [1, 5, 4, 0, 3, 2, 1, 0],
    [4, 4, 4, 4, 4, 1, 1, 0],
    [0, 0, 4, 1, 5, 2, 1, 0],
    [5, 0, 4, 5, 1, 1, 1, 0],
];
const CORNER_BL: [[u8; 8]; 8] = [
    [0, 1, 1, 1, 5, 4, 0, 5],
    [0, 1, 2, 5, 1, 4, 0, 0],
    [0, 1, 1, 4, 4, 4, 4, 4],
    [0, 1, 2, 3, 0, 4, 5, 1],
    [0, 1, 1, 0, 3, 4, 1, 5],
    [0, 1, 2, 1, 2, 1, 2, 1],
    [0, 1, 1, 1, 1, 1, 1, 1],
    [0, 0, 0, 0, 0, 0, 0, 0],
];
const CORNER_BR: [[u8; 8]; 8] = [
    [5, 0, 4, 1, 5, 1, 1, 0],
    [0, 0, 4, 5, 1, 2, 1, 0],
    [4, 4, 4, 4, 4, 1, 1, 0],
    [5, 1, 4, 0, 3, 2, 1, 0],
    [1, 5, 4, 3, 0, 1, 1, 0],
    [1, 2, 1, 2, 1, 2, 1, 0],
    [1, 1, 1, 1, 1, 1, 1, 0],
    [0, 0, 0, 0, 0, 0, 0, 0],
];

fn blit_tile(fb: &mut Surface, t: &[[u8; 8]; 8], x: i32, y: i32, cols: &[u8; 6]) {
    for (r, row) in t.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            fb.put(x + c as i32, y + r as i32, cols[*v as usize]);
        }
    }
}

/// The 8-px dotted border around a full-screen page.
pub fn dotted_border(fb: &mut Surface, ctx: &UiContext) {
    let mut cols = [0u8; 6];
    for (i, c) in BORDER_COLS.iter().enumerate() {
        cols[i] = col(ctx, *c);
    }
    for x in (8..312).step_by(8) {
        blit_tile(fb, &EDGE_H, x, 0, &cols);
        blit_tile(fb, &EDGE_H, x, 192, &cols);
    }
    for y in (8..192).step_by(8) {
        blit_tile(fb, &EDGE_V, 0, y, &cols);
        blit_tile(fb, &EDGE_V, 312, y, &cols);
    }
    blit_tile(fb, &CORNER_TL, 0, 0, &cols);
    blit_tile(fb, &CORNER_TR, 312, 0, &cols);
    blit_tile(fb, &CORNER_BL, 0, 192, &cols);
    blit_tile(fb, &CORNER_BR, 312, 192, &cols);
}

/// Thousands-separated number ("260,000").
pub fn commas(n: i64) -> String {
    crate::ui::mapview::format_pop(n)
}

// ---- generic full-screen report page -----------------------------------------------------------

/// A full-screen report drawn by `paint`; any key or click returns to the map.
pub struct Page {
    pub bg: [u8; 3],
    pub paint: fn(&mut Surface, &UiContext),
    guard: u32,
}

impl Page {
    pub fn new(bg: [u8; 3], paint: fn(&mut Surface, &UiContext)) -> Page {
        Page { bg, paint, guard: 2 }
    }
}

impl Screen for Page {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }
    fn handle(&mut self, ev: &Event, _ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        match ev {
            Event::Key { down: true, key, .. } if !matches!(key, Key::Unknown) => Transition::Pop(DialogResult::Done),
            Event::MouseButton { down: true, .. } => Transition::Pop(DialogResult::Done),
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        page(fb, ctx, self.bg);
        (self.paint)(fb, ctx);
    }
    fn palette(&self, ctx: &UiContext) -> Palette {
        ctx.sp.pal
    }
    fn draws_parent(&self) -> bool {
        false
    }
    fn wants_cursor(&self) -> bool {
        false
    }
}

// ---- entry points ------------------------------------------------------------------------------

pub fn advisor(ctx: &UiContext, n: u8) -> Box<dyn Screen> {
    use crate::ui::report_advisors as a;
    use crate::ui::report_world as w;
    match n {
        1 => Box::new(Page::new(BG_F1, a::paint_city_status)),
        2 => Box::new(Page::new(BG_BLUE, a::paint_military)),
        3 => Box::new(Page::new(BG_BLUE, a::paint_intelligence)),
        4 => Box::new(a::AttitudeSurvey::new(ctx)),
        5 => Box::new(Page::new(BG_GREEN, a::paint_trade)),
        6 => Box::new(Page::new(BG_BLUE, a::paint_science)),
        7 => Box::new(Page::new(BG_TEAL, w::paint_wonders)),
        8 => Box::new(Page::new(BG_TEAL, w::paint_top_five)),
        9 => Box::new(Page::new(BG_TEAL, w::paint_score)),
        10 => Box::new(Page::new(BLACK, w::paint_world_map)),
        11 => Box::new(Page::new(BG_TEAL, w::paint_demographics)),
        _ => Box::new(Page::new(BG_BLUE, w::paint_spaceships)),
    }
}

/// "Select new Tax rate:" / "Select new Luxuries rate:" at (101,81); applies the rate itself.
pub fn rate_dialog(ctx: &UiContext, lux: bool) -> Box<dyn Screen> {
    Box::new(RateDialog::new(ctx, lux))
}

pub fn find_city_dialog(_ctx: &UiContext) -> Box<dyn Screen> {
    Box::new(FindCity::new())
}

pub fn options_dialog(ctx: &UiContext) -> Box<dyn Screen> {
    Box::new(OptionsDialog::new(ctx))
}

pub fn save_dialog(ctx: &UiContext) -> Box<dyn Screen> {
    Box::new(crate::ui::loadgame::SlotPage::new(ctx, crate::ui::loadgame::SlotMode::Save))
}

pub fn load_dialog(ctx: &UiContext) -> Box<dyn Screen> {
    Box::new(crate::ui::loadgame::SlotPage::new(ctx, crate::ui::loadgame::SlotMode::LoadInGame))
}

/// Load-game chooser used from the title screen (cream page without the border).
pub fn load_game_screen() -> Box<dyn Screen> {
    Box::new(crate::ui::loadgame::LoadGame::new())
}

// ---- tax / luxuries rate ----------------------------------------------------------------------

pub struct RateDialog {
    menu: MenuDialog,
    lux: bool,
    /// Rate (in percent) of each menu row.
    rates: Vec<u8>,
}

impl RateDialog {
    pub fn new(ctx: &UiContext, lux: bool) -> RateDialog {
        let g = ctx.game();
        let p = g.human();
        let max = g.max_rate(g.human_slot);
        let (fixed, cur, word) = if lux { (p.tax, p.lux, "Luxuries") } else { (p.lux, p.tax, "Tax") };
        let mut items = Vec::new();
        let mut disabled = Vec::new();
        let mut rates = Vec::new();
        let mut r = 0u8;
        while r + fixed <= 100 {
            let sci = 100 - r - fixed;
            items.push(format!("{r}% {word}, ({sci}% Science)"));
            disabled.push(r > max || sci > max);
            rates.push(r);
            r += 10;
        }
        let sel = rates.iter().position(|x| *x == cur).unwrap_or(0);
        let title = format!("Select new {word} rate:");
        let menu = MenuDialog::new(vec![title], items).with_pos(101, 81).with_disabled(disabled).with_selection(sel);
        RateDialog { menu, lux, rates }
    }
}

impl Screen for RateDialog {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        self.menu.on_enter(ctx);
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        match self.menu.handle(ev, ctx) {
            Transition::Pop(DialogResult::Choice(i)) => {
                let rate = self.rates[i];
                let slot = ctx.game().human_slot;
                let (tax, lux) = if self.lux { (ctx.game().human().tax, rate) } else { (rate, ctx.game().human().lux) };
                match ctx.game_mut().set_rates(slot, tax, lux) {
                    Ok(()) => Transition::Pop(DialogResult::Done),
                    Err(e) => Transition::Replace(Box::new(MessageBox::text(e.message()))),
                }
            }
            t => t,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        self.menu.draw(fb, ctx);
    }
}

// ---- options -----------------------------------------------------------------------------------

const OPTION_NAMES: [&str; 8] = ["Instant Advice", "AutoSave", "End of Turn", "Animations", "Sound", "Enemy Moves", "Civilopedia Text", "Palace"];

fn option_flags(ctx: &UiContext) -> [bool; 8] {
    let o = &ctx.game().options;
    [o.instant_advice, o.autosave, o.end_turn_confirm, o.animations, o.sound, o.enemy_moves, o.civilopedia_text, o.palace]
}

fn option_items(ctx: &UiContext) -> Vec<String> {
    let f = option_flags(ctx);
    OPTION_NAMES.iter().enumerate().map(|(i, n)| format!("{}{n}", if f[i] { GLYPH_CHECK } else { ' ' })).collect()
}

/// "Options:" check list over the GAME menu; a choice toggles the option and keeps the box open.
pub struct OptionsDialog {
    menu: MenuDialog,
}

impl OptionsDialog {
    pub fn new(ctx: &UiContext) -> OptionsDialog {
        OptionsDialog { menu: MenuDialog::new(vec!["Options:".into()], option_items(ctx)).with_pos(24, 17) }
    }
}

impl Screen for OptionsDialog {
    fn on_enter(&mut self, ctx: &mut UiContext) {
        self.menu.on_enter(ctx);
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        match self.menu.handle(ev, ctx) {
            Transition::Pop(DialogResult::Choice(i)) => {
                let o = &mut ctx.game_mut().options;
                match i {
                    0 => o.instant_advice = !o.instant_advice,
                    1 => o.autosave = !o.autosave,
                    2 => o.end_turn_confirm = !o.end_turn_confirm,
                    3 => o.animations = !o.animations,
                    4 => o.sound = !o.sound,
                    5 => o.enemy_moves = !o.enemy_moves,
                    6 => o.civilopedia_text = !o.civilopedia_text,
                    _ => o.palace = !o.palace,
                }
                self.menu.items = option_items(ctx);
                Transition::None
            }
            Transition::Pop(_) => Transition::Pop(DialogResult::Done),
            t => t,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        self.menu.draw(fb, ctx);
    }
}

// ---- FindCity ----------------------------------------------------------------------------------

const FIND_RECT: Rect = Rect::new(80, 79, 208, 23);
const FIND_FIELD: Rect = Rect::new(80, 88, 123, 14);

/// "Where in the heck is ... (city name)": a light box with an input field; a match centres the
/// map on the city through `UiContext::center_request`.
pub struct FindCity {
    value: String,
    guard: u32,
}

impl Default for FindCity {
    fn default() -> Self {
        Self::new()
    }
}

impl FindCity {
    pub fn new() -> FindCity {
        FindCity { value: String::new(), guard: 1 }
    }

    fn lookup(&self, ctx: &UiContext) -> Option<(i32, i32)> {
        let g = ctx.game();
        let q = self.value.trim().to_ascii_lowercase();
        if q.is_empty() {
            return None;
        }
        let known = |c: &crate::game::City| c.owner == g.human_slot || g.explored(g.human_slot, c.x, c.y);
        if let Some(c) = g.all_cities().filter(|c| known(c)).find(|c| c.name.to_ascii_lowercase() == q) {
            return Some((c.x, c.y));
        }
        g.all_cities().filter(|c| known(c)).find(|c| c.name.to_ascii_lowercase().starts_with(&q)).map(|c| (c.x, c.y))
    }
}

impl Screen for FindCity {
    fn update(&mut self, _ctx: &mut UiContext) -> Transition {
        self.guard = self.guard.saturating_sub(1);
        Transition::None
    }
    fn handle(&mut self, ev: &Event, ctx: &mut UiContext) -> Transition {
        if self.guard > 0 {
            return Transition::None;
        }
        match ev {
            Event::Key { key: Key::Enter, down: true, .. } => match self.lookup(ctx) {
                Some(pos) => {
                    ctx.center_request = Some(pos);
                    Transition::Pop(DialogResult::Done)
                }
                None => Transition::Replace(Box::new(MessageBox::text(&format!("{} not found.", self.value.trim())))),
            },
            Event::Key { key: Key::Esc, down: true, .. } => Transition::Pop(DialogResult::Cancelled),
            Event::Key { key: Key::Backspace, down: true, .. } => {
                self.value.pop();
                Transition::None
            }
            Event::MouseButton { button: MouseButton::Right, down: true, .. } => Transition::Pop(DialogResult::Cancelled),
            Event::Text(c) => {
                if self.value.chars().count() < 13 && (' '..='~').contains(c) {
                    self.value.push(*c);
                }
                Transition::None
            }
            _ => Transition::None,
        }
    }
    fn draw(&mut self, fb: &mut Surface, ctx: &mut UiContext) {
        let light = col(ctx, WHITE);
        let black = col(ctx, BLACK);
        fb.fill_rect(FIND_RECT, light);
        fb.rect_outline(FIND_RECT, black);
        draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, black), FIND_RECT.x + 2, FIND_RECT.y + 1, "Where in the heck is ... (city name)");
        fb.rect_outline(FIND_FIELD, black);
        let end = draw_text(fb, &ctx.fonts, &TextStyle::new(FONT, black), FIND_FIELD.x + 4, FIND_FIELD.y + 3, &self.value);
        if (ctx.frame / 20) % 2 == 0 {
            fb.fill_rect(Rect::new(end + 1, FIND_FIELD.y + 3, 4, 7), black);
        }
    }
}
