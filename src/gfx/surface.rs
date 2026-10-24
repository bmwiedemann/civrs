//! Indexed-colour surface with clipping, blitting and PPM output.

use crate::assets::{Palette, Pic};

pub const SCREEN_W: i32 = 320;
pub const SCREEN_H: i32 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.right() && py < self.bottom()
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        if x1 <= x0 || y1 <= y0 {
            Rect::new(0, 0, 0, 0)
        } else {
            Rect::new(x0, y0, x1 - x0, y1 - y0)
        }
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    pub fn inset(&self, d: i32) -> Rect {
        Rect::new(self.x + d, self.y + d, self.w - 2 * d, self.h - 2 * d)
    }
    pub fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }
}

#[derive(Clone, Debug)]
pub struct Surface {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>,
    clip: Rect,
}

impl Surface {
    pub fn new(w: i32, h: i32) -> Surface {
        Surface { w, h, px: vec![0; (w.max(0) * h.max(0)) as usize], clip: Rect::new(0, 0, w, h) }
    }

    pub fn screen() -> Surface {
        Surface::new(SCREEN_W, SCREEN_H)
    }

    pub fn from_pic(p: &Pic) -> Surface {
        let mut s = Surface::new(p.width as i32, p.height as i32);
        s.px.copy_from_slice(&p.pixels);
        s
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    pub fn set_clip(&mut self, r: Option<Rect>) {
        self.clip = match r {
            Some(r) => r.intersect(&self.bounds()),
            None => self.bounds(),
        };
    }

    pub fn clip(&self) -> Rect {
        self.clip
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            0
        } else {
            self.px[(y * self.w + x) as usize]
        }
    }

    #[inline]
    pub fn put(&mut self, x: i32, y: i32, c: u8) {
        if self.clip.contains(x, y) {
            self.px[(y * self.w + x) as usize] = c;
        }
    }

    pub fn fill(&mut self, c: u8) {
        let r = self.clip;
        self.fill_rect(r, c);
    }

    pub fn fill_rect(&mut self, r: Rect, c: u8) {
        let r = r.intersect(&self.clip);
        if r.is_empty() {
            return;
        }
        for y in r.y..r.bottom() {
            let row = (y * self.w) as usize;
            self.px[row + r.x as usize..row + r.right() as usize].fill(c);
        }
    }

    pub fn hline(&mut self, x0: i32, x1: i32, y: i32, c: u8) {
        let (a, b) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        self.fill_rect(Rect::new(a, y, b - a + 1, 1), c);
    }

    pub fn vline(&mut self, x: i32, y0: i32, y1: i32, c: u8) {
        let (a, b) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        self.fill_rect(Rect::new(x, a, 1, b - a + 1), c);
    }

    /// A line as the graphics driver draws it (entry 42): the major axis steps every pixel,
    /// the minor one when an error that starts at half the major length drops below zero, so a
    /// half-way point rounds toward the start (checked against the Chieftain screen-layout
    /// tutorial capture).
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: u8) {
        let (dx, dy) = ((x1 - x0).abs(), (y1 - y0).abs());
        let (sx, sy) = ((x1 - x0).signum(), (y1 - y0).signum());
        let (mut x, mut y) = (x0, y0);
        if dx >= dy {
            let mut err = dx / 2;
            for _ in 0..=dx {
                self.put(x, y, c);
                err -= dy;
                if err < 0 {
                    err += dx;
                    y += sy;
                }
                x += sx;
            }
        } else {
            let mut err = dy / 2;
            for _ in 0..=dy {
                self.put(x, y, c);
                err -= dx;
                if err < 0 {
                    err += dy;
                    x += sx;
                }
                y += sy;
            }
        }
    }

    pub fn rect_outline(&mut self, r: Rect, c: u8) {
        if r.is_empty() {
            return;
        }
        self.hline(r.x, r.right() - 1, r.y, c);
        self.hline(r.x, r.right() - 1, r.bottom() - 1, c);
        self.vline(r.x, r.y, r.bottom() - 1, c);
        self.vline(r.right() - 1, r.y, r.bottom() - 1, c);
    }

    /// Opaque blit of a whole surface.
    pub fn blit(&mut self, src: &Surface, x: i32, y: i32) {
        self.blit_part(src, src.bounds(), x, y, None);
    }

    /// Blit a sub-rectangle of `src` to (x,y); pixels equal to `key` are skipped.
    pub fn blit_part(&mut self, src: &Surface, sr: Rect, x: i32, y: i32, key: Option<u8>) {
        let sr = sr.intersect(&src.bounds());
        if sr.is_empty() {
            return;
        }
        let dst = Rect::new(x, y, sr.w, sr.h).intersect(&self.clip);
        if dst.is_empty() {
            return;
        }
        let ox = dst.x - x;
        let oy = dst.y - y;
        for row in 0..dst.h {
            let sy = sr.y + oy + row;
            let dy = dst.y + row;
            let srow = (sy * src.w) as usize;
            let drow = (dy * self.w) as usize;
            let sx0 = (sr.x + ox) as usize;
            let dx0 = dst.x as usize;
            match key {
                None => {
                    self.px[drow + dx0..drow + dx0 + dst.w as usize]
                        .copy_from_slice(&src.px[srow + sx0..srow + sx0 + dst.w as usize]);
                }
                Some(k) => {
                    for i in 0..dst.w as usize {
                        let v = src.px[srow + sx0 + i];
                        if v != k {
                            self.px[drow + dx0 + i] = v;
                        }
                    }
                }
            }
        }
    }

    /// Blit with a colour remap table (e.g. civilisation colours); `key` skipped before remap.
    pub fn blit_remap(&mut self, src: &Surface, sr: Rect, x: i32, y: i32, key: Option<u8>, map: &[u8; 256]) {
        let sr = sr.intersect(&src.bounds());
        if sr.is_empty() {
            return;
        }
        let dst = Rect::new(x, y, sr.w, sr.h).intersect(&self.clip);
        if dst.is_empty() {
            return;
        }
        let ox = dst.x - x;
        let oy = dst.y - y;
        for row in 0..dst.h {
            let sy = sr.y + oy + row;
            let dy = dst.y + row;
            let srow = (sy * src.w) as usize;
            let drow = (dy * self.w) as usize;
            let sx0 = (sr.x + ox) as usize;
            let dx0 = dst.x as usize;
            for i in 0..dst.w as usize {
                let v = src.px[srow + sx0 + i];
                if Some(v) != key {
                    self.px[drow + dx0 + i] = map[v as usize];
                }
            }
        }
    }

    /// Blit where every non-key source pixel is drawn in a single colour (silhouettes/shadows).
    pub fn blit_solid(&mut self, src: &Surface, sr: Rect, x: i32, y: i32, key: u8, colour: u8) {
        let sr = sr.intersect(&src.bounds());
        if sr.is_empty() {
            return;
        }
        for row in 0..sr.h {
            for col in 0..sr.w {
                let v = src.px[((sr.y + row) * src.w + sr.x + col) as usize];
                if v != key {
                    self.put(x + col, y + row, colour);
                }
            }
        }
    }

    /// Fill `r` by tiling the pattern `pat` (a sub-rectangle of `src`), aligned to screen origin.
    pub fn fill_pattern(&mut self, r: Rect, src: &Surface, pat: Rect) {
        let r = r.intersect(&self.clip);
        if r.is_empty() || pat.is_empty() {
            return;
        }
        for y in r.y..r.bottom() {
            let py = pat.y + (y.rem_euclid(pat.h));
            for x in r.x..r.right() {
                let px = pat.x + (x.rem_euclid(pat.w));
                let v = src.px[(py * src.w + px) as usize];
                self.px[(y * self.w + x) as usize] = v;
            }
        }
    }

    /// Fill `r` with a 2-colour checkerboard dither.
    pub fn fill_dither(&mut self, r: Rect, a: u8, b: u8) {
        let r = r.intersect(&self.clip);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                self.px[(y * self.w + x) as usize] = if (x + y) & 1 == 0 { a } else { b };
            }
        }
    }

    /// Replace colour `from` with `to` inside `r`.
    pub fn replace_colour(&mut self, r: Rect, from: u8, to: u8) {
        let r = r.intersect(&self.clip);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let i = (y * self.w + x) as usize;
                if self.px[i] == from {
                    self.px[i] = to;
                }
            }
        }
    }

    /// Copy a rectangle out into a new surface.
    pub fn sub(&self, r: Rect) -> Surface {
        let r = r.intersect(&self.bounds());
        let mut s = Surface::new(r.w.max(0), r.h.max(0));
        s.blit_part(self, r, 0, 0, None);
        s
    }

    /// Convert to RGBA8888 (`out.len() == w*h*4`).
    pub fn to_rgba(&self, pal: &Palette, out: &mut [u8]) {
        for (i, &v) in self.px.iter().enumerate() {
            let c = pal.0[v as usize];
            out[i * 4] = c[0];
            out[i * 4 + 1] = c[1];
            out[i * 4 + 2] = c[2];
            out[i * 4 + 3] = 255;
        }
    }

    pub fn write_ppm(&self, pal: &Palette, path: &std::path::Path) -> std::io::Result<()> {
        let mut data = Vec::with_capacity(self.px.len() * 3 + 32);
        data.extend_from_slice(format!("P6\n{} {}\n255\n", self.w, self.h).as_bytes());
        for &v in &self.px {
            data.extend_from_slice(&pal.0[v as usize]);
        }
        std::fs::write(path, data)
    }

    /// Write a scaled PPM (integer factor, nearest neighbour).
    pub fn write_ppm_scaled(&self, pal: &Palette, path: &std::path::Path, scale: usize) -> std::io::Result<()> {
        let scale = scale.max(1);
        let (w, h) = (self.w as usize, self.h as usize);
        let mut data = Vec::with_capacity(w * h * 3 * scale * scale + 32);
        data.extend_from_slice(format!("P6\n{} {}\n255\n", w * scale, h * scale).as_bytes());
        for y in 0..h * scale {
            for x in 0..w * scale {
                let v = self.px[(y / scale) * w + x / scale];
                data.extend_from_slice(&pal.0[v as usize]);
            }
        }
        std::fs::write(path, data)
    }
}
