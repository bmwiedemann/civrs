//! VGA palettes (256 entries of 8-bit RGB), decoded from the `M0` block of PIC/PAL files.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Palette(pub [[u8; 3]; 256]);

impl Default for Palette {
    fn default() -> Self {
        Palette::black()
    }
}

impl Palette {
    pub fn black() -> Self {
        Palette([[0, 0, 0]; 256])
    }

    /// Standard 16 EGA colours in 0..15, rest black (used when a picture has no palette).
    pub fn ega() -> Self {
        let mut p = Palette::black();
        const EGA: [[u8; 3]; 16] = [
            [0, 0, 0], [0, 0, 170], [0, 170, 0], [0, 170, 170],
            [170, 0, 0], [170, 0, 170], [170, 85, 0], [170, 170, 170],
            [85, 85, 85], [85, 85, 255], [85, 255, 85], [85, 255, 255],
            [255, 85, 85], [255, 85, 255], [255, 255, 85], [255, 255, 255],
        ];
        p.0[..16].copy_from_slice(&EGA);
        p
    }

    /// Decode an `M0` block payload: first index, last index, then 6-bit RGB triples.
    pub fn from_m0(payload: &[u8]) -> Result<Self, String> {
        if payload.len() < 2 {
            return Err("M0 block too short".into());
        }
        let first = payload[0] as usize;
        let last = payload[1] as usize;
        if last < first {
            return Err("M0 last < first".into());
        }
        let n = last - first + 1;
        if payload.len() < 2 + 3 * n {
            return Err(format!("M0 block truncated: need {} bytes, have {}", 2 + 3 * n, payload.len()));
        }
        let mut p = Palette::black();
        for i in 0..n {
            let r = payload[2 + i * 3];
            let g = payload[3 + i * 3];
            let b = payload[4 + i * 3];
            p.0[first + i] = [scale6(r), scale6(g), scale6(b)];
        }
        Ok(p)
    }

    /// Copy the entries in `range` from `other` into a copy of `self`.
    pub fn merged(&self, other: &Palette, range: std::ops::RangeInclusive<u8>) -> Palette {
        let mut p = *self;
        for i in range {
            p.0[i as usize] = other.0[i as usize];
        }
        p
    }

    /// Linear interpolation towards `to`; `t` in 0..=255.
    pub fn lerp(&self, to: &Palette, t: u8) -> Palette {
        let mut p = Palette::black();
        let t = t as u32;
        for i in 0..256 {
            for c in 0..3 {
                let a = self.0[i][c] as u32;
                let b = to.0[i][c] as u32;
                p.0[i][c] = ((a * (255 - t) + b * t + 127) / 255) as u8;
            }
        }
        p
    }

    /// Palette with all entries scaled by `level` (0 = black, 255 = unchanged).
    pub fn dimmed(&self, level: u8) -> Palette {
        Palette::black().lerp(self, level)
    }

    /// Index of the palette entry closest to `rgb`.
    pub fn nearest(&self, rgb: [u8; 3]) -> u8 {
        let mut best = 0usize;
        let mut bd = u32::MAX;
        for (i, c) in self.0.iter().enumerate() {
            let d = (0..3).map(|k| (c[k] as i32 - rgb[k] as i32).pow(2) as u32).sum::<u32>();
            if d < bd {
                bd = d;
                best = i;
            }
        }
        best as u8
    }

    pub fn rgb(&self, idx: u8) -> [u8; 3] {
        self.0[idx as usize]
    }
}

#[inline]
fn scale6(c: u8) -> u8 {
    let c = (c & 0x3F) as u32;
    ((c * 255 + 31) / 63) as u8
}
