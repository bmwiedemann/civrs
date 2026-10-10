//! MicroProse `.PIC` container: a sequence of tagged blocks (`E0` EGA map, `M0` palette,
//! `X0` 256-colour picture, `X1` 16-colour picture). Pictures are LZW compressed and then RLE
//! encoded (escape byte 0x90).

use super::pal::Palette;
use std::fmt;

#[derive(Debug)]
pub enum PicError {
    Truncated,
    BadBlock(String),
    NoPicture,
}

impl fmt::Display for PicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PicError::Truncated => write!(f, "truncated block"),
            PicError::BadBlock(s) => write!(f, "bad block: {s}"),
            PicError::NoPicture => write!(f, "no picture block"),
        }
    }
}

impl std::error::Error for PicError {}

pub struct Block<'a> {
    pub tag: [u8; 2],
    pub payload: &'a [u8],
}

/// Split a file into its blocks: `tag[2] len:u16le payload[len]`.
pub fn blocks(bytes: &[u8]) -> Result<Vec<Block<'_>>, PicError> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 4 <= bytes.len() {
        let tag = [bytes[pos], bytes[pos + 1]];
        let len = u16::from_le_bytes([bytes[pos + 2], bytes[pos + 3]]) as usize;
        let start = pos + 4;
        let end = start + len;
        if end > bytes.len() {
            // Tolerate a truncated final block.
            out.push(Block { tag, payload: &bytes[start..] });
            break;
        }
        out.push(Block { tag, payload: &bytes[start..end] });
        pos = end;
    }
    Ok(out)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Depth {
    Indexed8,
    Indexed4,
}

/// A decoded picture. `pixels` always holds one byte per pixel (nibbles expanded for 16-colour
/// pictures), row-major, `width * height` entries.
#[derive(Clone, Debug)]
pub struct Pic {
    pub width: u16,
    pub height: u16,
    pub depth: Depth,
    pub pixels: Vec<u8>,
    pub palette: Option<Palette>,
    pub ega_map: Option<[u8; 256]>,
}

impl Pic {
    pub fn get(&self, x: usize, y: usize) -> u8 {
        self.pixels[y * self.width as usize + x]
    }
}

/// LZW decoder as used by MicroProse PIC files: LSB-first codes starting at 9 bits, code 256
/// reserved (never emitted), new codes from 257, and an implicit reset when the dictionary
/// reaches `1 << max_bits` entries.
pub fn lzw_decode(src: &[u8], max_bits: u8) -> Vec<u8> {
    let max_bits = max_bits.clamp(9, 16) as u32;
    #[derive(Clone, Copy)]
    struct Entry {
        prefix: u16, // 0xFFFF for a root entry
        suffix: u8,
        first: u8,
        len: u32,
    }
    let mut dict: Vec<Entry> = Vec::with_capacity(1 << max_bits);
    let reset = |dict: &mut Vec<Entry>| {
        dict.clear();
        for i in 0..256u16 {
            dict.push(Entry { prefix: 0xFFFF, suffix: i as u8, first: i as u8, len: 1 });
        }
        // code 256: reserved, occupies a slot
        dict.push(Entry { prefix: 0xFFFF, suffix: 0, first: 0, len: 0 });
    };
    reset(&mut dict);

    let mut out: Vec<u8> = Vec::with_capacity(src.len() * 3);
    let mut bitbuf: u32 = 0;
    let mut nbits: u32 = 0;
    let mut pos = 0usize;
    let mut width: u32 = 9;
    let mut prev: Option<u16> = None;
    let mut stack: Vec<u8> = Vec::with_capacity(4096);

    loop {
        while nbits < width && pos < src.len() {
            bitbuf |= (src[pos] as u32) << nbits;
            pos += 1;
            nbits += 8;
        }
        if nbits < width {
            break;
        }
        let code = (bitbuf & ((1u32 << width) - 1)) as u16;
        bitbuf >>= width;
        nbits -= width;

        let dict_len = dict.len() as u16;
        let (entry_first, entry_len): (u8, u32);
        if code < dict_len && code != 256 {
            // Emit dict[code] by walking the prefix chain.
            stack.clear();
            let mut c = code;
            loop {
                let e = dict[c as usize];
                stack.push(e.suffix);
                if e.prefix == 0xFFFF {
                    break;
                }
                c = e.prefix;
            }
            entry_first = *stack.last().unwrap();
            entry_len = stack.len() as u32;
            out.extend(stack.iter().rev());
        } else if code == dict_len {
            // KwKwK: entry = prev + first(prev)
            let Some(p) = prev else { break };
            stack.clear();
            let mut c = p;
            loop {
                let e = dict[c as usize];
                stack.push(e.suffix);
                if e.prefix == 0xFFFF {
                    break;
                }
                c = e.prefix;
            }
            let first = *stack.last().unwrap();
            entry_first = first;
            entry_len = stack.len() as u32 + 1;
            out.extend(stack.iter().rev());
            out.push(first);
        } else {
            // Corrupt stream (or reserved code); stop decoding.
            break;
        }
        if let Some(p) = prev {
            let pf = dict[p as usize].first;
            let pl = dict[p as usize].len;
            dict.push(Entry { prefix: p, suffix: entry_first, first: pf, len: pl + 1 });
        }
        let _ = entry_len;
        prev = Some(code);
        if dict.len() >= (1usize << width) {
            if width < max_bits {
                width += 1;
            } else {
                reset(&mut dict);
                width = 9;
                prev = None;
            }
        }
    }
    out
}

/// RLE decoder: `0x90 0x00` is a literal 0x90; `0x90 N` repeats the previous byte N-1 more times.
pub fn rle_decode(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() * 2);
    let mut last = 0u8;
    let mut i = 0;
    while i < src.len() {
        let b = src[i];
        if b == 0x90 {
            if i + 1 >= src.len() {
                break;
            }
            let n = src[i + 1];
            i += 2;
            if n == 0 {
                out.push(0x90);
                last = 0x90;
            } else {
                for _ in 1..n {
                    out.push(last);
                }
            }
        } else {
            out.push(b);
            last = b;
            i += 1;
        }
    }
    out
}

/// Decode a PIC (or PAL) file. Returns the first non-empty picture block together with the
/// palette (`M0`) and EGA map (`E0`) found anywhere in the file. A file with only a palette
/// (a `.PAL`) yields a 0x0 picture.
pub fn decode_pic(bytes: &[u8]) -> Result<Pic, PicError> {
    let mut palette = None;
    let mut ega_map = None;
    let mut picture: Option<(u16, u16, Depth, Vec<u8>)> = None;
    for b in blocks(bytes)? {
        match &b.tag {
            b"M0" => {
                palette = Some(Palette::from_m0(b.payload).map_err(PicError::BadBlock)?);
            }
            b"E0" => {
                if b.payload.len() >= 2 {
                    let first = b.payload[0] as usize;
                    let mut map = [0u8; 256];
                    for (i, v) in b.payload[2..].iter().enumerate() {
                        if first + i < 256 {
                            map[first + i] = *v;
                        }
                    }
                    ega_map = Some(map);
                }
            }
            b"X0" | b"X1" => {
                if picture.is_some() {
                    continue;
                }
                if b.payload.len() < 5 {
                    continue; // empty picture block (w=h=0) as found at the end of .PAL files
                }
                let w = u16::from_le_bytes([b.payload[0], b.payload[1]]);
                let h = u16::from_le_bytes([b.payload[2], b.payload[3]]);
                let max_bits = b.payload[4];
                if w == 0 || h == 0 {
                    continue;
                }
                let lz = lzw_decode(&b.payload[5..], max_bits);
                let raw = rle_decode(&lz);
                let depth = if &b.tag == b"X0" { Depth::Indexed8 } else { Depth::Indexed4 };
                let (wu, hu) = (w as usize, h as usize);
                let mut pixels = vec![0u8; wu * hu];
                match depth {
                    Depth::Indexed8 => {
                        let n = raw.len().min(pixels.len());
                        pixels[..n].copy_from_slice(&raw[..n]);
                    }
                    Depth::Indexed4 => {
                        let stride = wu.div_ceil(2);
                        for y in 0..hu {
                            for x in 0..wu {
                                let idx = y * stride + x / 2;
                                if idx >= raw.len() {
                                    break;
                                }
                                let byte = raw[idx];
                                pixels[y * wu + x] = if x % 2 == 0 { byte & 0x0F } else { byte >> 4 };
                            }
                        }
                    }
                }
                picture = Some((w, h, depth, pixels));
            }
            _ => {}
        }
    }
    match picture {
        Some((width, height, depth, pixels)) => Ok(Pic { width, height, depth, pixels, palette, ega_map }),
        None => {
            if palette.is_some() {
                Ok(Pic { width: 0, height: 0, depth: Depth::Indexed8, pixels: Vec::new(), palette, ega_map })
            } else {
                Err(PicError::NoPicture)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_basics() {
        assert_eq!(rle_decode(&[0x41, 0x90, 0x03]), vec![0x41, 0x41, 0x41]);
        assert_eq!(rle_decode(&[0x90, 0x00]), vec![0x90]);
        assert_eq!(rle_decode(&[0x01, 0x02]), vec![1, 2]);
    }

    #[test]
    fn lzw_roots_only() {
        // codes 65, 66 as 9-bit LSB-first: 65 = 0b001000001, 66 = 0b001000010
        // bitstream: bits 0..8 = 65, bits 9..17 = 66
        let v: u32 = 65 | (66 << 9);
        let bytes = [(v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, ((v >> 16) & 0xFF) as u8];
        let out = lzw_decode(&bytes, 11);
        assert_eq!(out, vec![65, 66]);
    }

    #[test]
    fn lzw_new_code() {
        // 65, 66, 257 -> "AB" + dict[257] = "AB"  => "ABAB"
        let v: u32 = 65 | (66 << 9) | (257 << 18);
        let bytes = [(v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, ((v >> 16) & 0xFF) as u8, ((v >> 24) & 0xFF) as u8];
        let out = lzw_decode(&bytes, 11);
        assert_eq!(out, vec![65, 66, 65, 66]);
    }
}
