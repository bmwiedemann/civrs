//! FONTS.CV: nine 1-bit bitmap fonts.
//!
//! Layout: `u16 count`, `u16 offset[count]` (offset of each font's bitmap). Immediately before
//! the bitmap at `o`: `width[n]` at `o-8-n`, `first` at `o-8`, `last` at `o-7`, `bytes_per_row`
//! at `o-6`, `height` at `o-4`. The bitmap holds `height+1` rows, each row being the glyph
//! strip for all characters (`n * bpr` bytes). Bits are MSB-first (leftmost pixel).

/// Blank columns the original draws after every glyph; the width table holds ink widths only.
pub const GLYPH_SPACING: i32 = 1;

#[derive(Clone, Debug)]
pub struct Glyph {
    /// Horizontal advance in pixels (may exceed the cell width).
    pub advance: u8,
    /// One u16 per row, MSB = leftmost pixel of the 8- or 16-pixel-wide cell.
    pub rows: Vec<u16>,
}

#[derive(Clone, Debug)]
pub struct Font {
    pub first: u8,
    pub last: u8,
    pub height: u8,
    pub bytes_per_row: u8,
    pub glyphs: Vec<Glyph>,
}

impl Font {
    pub fn glyph(&self, c: u8) -> Option<&Glyph> {
        if c < self.first || c > self.last {
            return None;
        }
        self.glyphs.get((c - self.first) as usize)
    }

    /// Width of a string in pixels: sum of advances (unknown chars use the advance of '?'),
    /// each followed by a `GLYPH_SPACING` column.
    pub fn width(&self, s: &str) -> i32 {
        s.chars().map(|ch| self.advance_of(ch) as i32 + GLYPH_SPACING).sum()
    }

    pub fn advance_of(&self, ch: char) -> u8 {
        let c = char_to_byte(ch);
        match self.glyph(c) {
            Some(g) => g.advance,
            None => self.glyph(b'?').map(|g| g.advance).unwrap_or(0),
        }
    }

    pub fn cell_width(&self) -> i32 {
        self.bytes_per_row as i32 * 8
    }

    /// Number of stored rows (height + 1).
    pub fn rows(&self) -> i32 {
        self.height as i32 + 1
    }

    /// Recommended line spacing.
    pub fn line_height(&self) -> i32 {
        self.height as i32 + 1
    }
}

/// Map a char to the Latin-1 byte used by the font; non-Latin-1 characters become '?'.
pub fn char_to_byte(ch: char) -> u8 {
    let c = ch as u32;
    if c < 256 { c as u8 } else { b'?' }
}

#[derive(Clone, Debug)]
pub struct FontSet(pub Vec<Font>);

impl FontSet {
    pub fn get(&self, id: usize) -> &Font {
        self.0.get(id).unwrap_or(&self.0[0])
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Font ids.
pub const FONT_DEFAULT: usize = 0; // small bold, 7 px
pub const FONT_TINY: usize = 1; // 5 px
pub const FONT_SERIF: usize = 2; // 10 px serif with descenders
pub const FONT_TITLE: usize = 3; // 16 px title
pub const FONT_THIN: usize = 4; // 16 px thin serif
pub const FONT_BOLDSERIF: usize = 5; // 14 px bold serif
pub const FONT_BOLD8: usize = 6; // 8 px bold
pub const FONT_SANS: usize = 7; // 9 px sans
pub const FONT_SCRIPT: usize = 8; // 9 px script

pub fn decode_fonts(bytes: &[u8]) -> Result<FontSet, String> {
    let rd16 = |o: usize| -> Result<usize, String> {
        if o + 2 > bytes.len() {
            return Err(format!("offset {o} out of range"));
        }
        Ok(u16::from_le_bytes([bytes[o], bytes[o + 1]]) as usize)
    };
    let count = rd16(0)?;
    if count == 0 || count > 32 {
        return Err(format!("implausible font count {count}"));
    }
    let mut fonts = Vec::with_capacity(count);
    for i in 0..count {
        let o = rd16(2 + i * 2)?;
        if o < 8 || o > bytes.len() {
            return Err(format!("font {i}: bad bitmap offset {o}"));
        }
        let first = bytes[o - 8];
        let last = bytes[o - 7];
        let bpr = bytes[o - 6];
        let height = bytes[o - 4];
        if last < first || !(1..=2).contains(&bpr) {
            return Err(format!("font {i}: bad header first={first} last={last} bpr={bpr}"));
        }
        let n = (last - first) as usize + 1;
        if o < 8 + n {
            return Err(format!("font {i}: width table out of range"));
        }
        let widths = &bytes[o - 8 - n..o - 8];
        let rows = height as usize + 1;
        let stride = n * bpr as usize;
        if o + rows * stride > bytes.len() {
            return Err(format!("font {i}: bitmap out of range"));
        }
        let mut glyphs = Vec::with_capacity(n);
        for c in 0..n {
            let mut grows = Vec::with_capacity(rows);
            for r in 0..rows {
                let base = o + r * stride + c * bpr as usize;
                let v: u16 = if bpr == 1 {
                    (bytes[base] as u16) << 8
                } else {
                    u16::from_be_bytes([bytes[base], bytes[base + 1]])
                };
                grows.push(v);
            }
            glyphs.push(Glyph { advance: widths[c], rows: grows });
        }
        fonts.push(Font { first, last, height, bytes_per_row: bpr, glyphs });
    }
    Ok(FontSet(fonts))
}
