//! Bitmap text rendering using the FONTS.CV fonts.

use super::surface::{Rect, Surface};
use crate::assets::fonts::{char_to_byte, Font, FontSet, GLYPH_SPACING};

#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub font: usize,
    pub colour: u8,
    /// Optional shadow colour drawn at (+1,+1).
    pub shadow: Option<u8>,
}

impl TextStyle {
    pub const fn new(font: usize, colour: u8) -> TextStyle {
        TextStyle { font, colour, shadow: None }
    }
    pub const fn shadowed(font: usize, colour: u8, shadow: u8) -> TextStyle {
        TextStyle { font, colour, shadow: Some(shadow) }
    }
}

fn draw_glyph_run(dst: &mut Surface, font: &Font, colour: u8, x: i32, y: i32, s: &str) -> i32 {
    let mut cx = x;
    let cell_w = font.cell_width();
    let n = s.chars().count();
    for (i, ch) in s.chars().enumerate() {
        let c = char_to_byte(ch);
        if let Some(g) = font.glyph(c) {
            // The original blanks the spacing column in front of every glyph, which wipes the
            // pixels a few glyphs ('t', 'u', '-') carry beyond their advance width, except
            // after the last glyph of a string.
            let w = if i + 1 < n { cell_w.min(g.advance as i32) } else { cell_w };
            for (r, bits) in g.rows.iter().enumerate() {
                if *bits == 0 {
                    continue;
                }
                for col in 0..w {
                    if bits & (0x8000 >> col) != 0 {
                        dst.put(cx + col, y + r as i32, colour);
                    }
                }
            }
            cx += g.advance as i32 + GLYPH_SPACING;
        } else {
            cx += font.advance_of('?') as i32 + GLYPH_SPACING;
        }
    }
    cx
}

/// Draw text; returns the x position after the last glyph (including its trailing spacing
/// column, so runs can be chained).
pub fn draw_text(dst: &mut Surface, fonts: &FontSet, st: &TextStyle, x: i32, y: i32, s: &str) -> i32 {
    let font = fonts.get(st.font);
    if let Some(sh) = st.shadow {
        draw_glyph_run(dst, font, sh, x + 1, y + 1, s);
    }
    draw_glyph_run(dst, font, st.colour, x, y, s)
}

/// Advance width of `s`, including the spacing column after the last glyph.
pub fn text_width(fonts: &FontSet, font: usize, s: &str) -> i32 {
    fonts.get(font).width(s)
}

/// Width of `s` without the trailing spacing column.
pub fn ink_width(fonts: &FontSet, font: usize, s: &str) -> i32 {
    (text_width(fonts, font, s) - GLYPH_SPACING).max(0)
}

pub fn draw_text_centered(dst: &mut Surface, fonts: &FontSet, st: &TextStyle, cx: i32, y: i32, s: &str) -> i32 {
    // the original centres on the full advance width, trailing spacing column included
    let w = text_width(fonts, st.font, s);
    draw_text(dst, fonts, st, cx - w / 2, y, s)
}

pub fn draw_text_right(dst: &mut Surface, fonts: &FontSet, st: &TextStyle, right_x: i32, y: i32, s: &str) -> i32 {
    let w = ink_width(fonts, st.font, s);
    draw_text(dst, fonts, st, right_x - w, y, s)
}

/// Word-wrap `s` to at most `max_w` pixels in the given font.
pub fn wrap(fonts: &FontSet, font: usize, max_w: i32, s: &str) -> Vec<String> {
    let f = fonts.get(font);
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split(' ') {
        if word.is_empty() {
            continue;
        }
        if cur.is_empty() {
            cur = word.to_string();
        } else {
            let candidate = format!("{cur} {word}");
            if f.width(&candidate) - GLYPH_SPACING <= max_w {
                cur = candidate;
            } else {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
            }
        }
        // hard-split overlong words
        while f.width(&cur) - GLYPH_SPACING > max_w && cur.chars().count() > 1 {
            let mut split_at = cur.chars().count() - 1;
            while split_at > 1 && f.width(&cur.chars().take(split_at).collect::<String>()) - GLYPH_SPACING > max_w {
                split_at -= 1;
            }
            let head: String = cur.chars().take(split_at).collect();
            let tail: String = cur.chars().skip(split_at).collect();
            lines.push(head);
            cur = tail;
        }
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Wrap each paragraph (empty strings = blank lines) and draw inside `r`; returns the y after
/// the last line drawn.
pub fn draw_paragraph(dst: &mut Surface, fonts: &FontSet, st: &TextStyle, r: Rect, paras: &[String], spacing: i32) -> i32 {
    let lh = fonts.get(st.font).line_height() + spacing;
    let mut y = r.y;
    for p in paras {
        if p.is_empty() {
            y += lh;
            continue;
        }
        for line in wrap(fonts, st.font, r.w, p) {
            if y + lh > r.bottom() + lh {
                return y;
            }
            draw_text(dst, fonts, st, r.x, y, &line);
            y += lh;
        }
    }
    y
}
