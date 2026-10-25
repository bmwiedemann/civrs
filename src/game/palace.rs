//! The palace the people build for the ruler (CIV.EXE overlay 0x15f76..0x1700a).
//!
//! The palace has seven building sections (1..7, left to right; 4 is the centre) and three
//! garden sections (9 = A left, 10 = B the road in front, 11 = C right). A new game starts
//! with a plain stone building of sections 3, 4 and 5 at level 0 and bare gardens (level 0).
//! Each palace addition raises one section by a level (buildings 1..4, gardens 1..3); a
//! building section also gets one of three styles (0 castle, 1 classical, 2 oriental).
//! Sections next to the building can only be added once a neighbour is above level 0.

/// Section count of the state arrays (index 0 and 8 stay empty, like in the original).
pub const SECTIONS: usize = 12;
/// Highest level of a building section / a garden section.
pub const MAX_BUILDING: i8 = 4;
pub const MAX_GARDEN: i8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palace {
    /// Level per section, -1 = not built.
    pub level: [i8; SECTIONS],
    /// Style per building section (0 castle, 1 classical, 2 oriental).
    pub style: [u8; SECTIONS],
}

/// Where a building section sits in the row, which decides the pieces it is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Leftmost built section: a tower (plus a wall for section 3).
    LeftEnd,
    /// Between other sections: two wall pieces.
    Inner,
    /// Section 4, the centre piece.
    Middle,
    /// Rightmost built section: a tower (plus a wall for section 5).
    RightEnd,
}

impl Default for Palace {
    fn default() -> Self {
        let mut level = [-1i8; SECTIONS];
        for i in 3..=5 {
            level[i] = 0;
            level[i + 6] = 0;
        }
        Palace { level, style: [0; SECTIONS] }
    }
}

impl Palace {
    /// Restore from the bytes kept in `Player::palace_parts` (older saves held a list of
    /// addition numbers there: they start over with the stone building).
    pub fn decode(bytes: &[u8]) -> Palace {
        if bytes.len() != 2 * SECTIONS {
            return Palace::default();
        }
        let mut p = Palace::default();
        for i in 0..SECTIONS {
            p.level[i] = bytes[i] as i8;
            p.style[i] = bytes[SECTIONS + i] % 3;
        }
        p
    }

    pub fn encode(&self) -> Vec<u8> {
        self.level.iter().map(|l| *l as u8).chain(self.style.iter().copied()).collect()
    }

    fn lv(&self, i: usize) -> i8 {
        self.level.get(i).copied().unwrap_or(-1)
    }

    pub fn is_garden(part: usize) -> bool {
        (9..=11).contains(&part)
    }

    /// The section can still be raised.
    pub fn can_improve(&self, part: usize) -> bool {
        match part {
            1..=7 => self.lv(part) < MAX_BUILDING,
            9..=11 => self.lv(part) < MAX_GARDEN,
            _ => false,
        }
    }

    /// The section gets a label on the "Which section shall we improve?" screen.
    pub fn offered(&self, part: usize) -> bool {
        match part {
            1..=7 => self.lv(part) < MAX_BUILDING && (self.lv(part) >= 0 || self.lv(part - 1) > 0 || self.lv(part + 1) > 0),
            9..=11 => self.lv(part) < MAX_GARDEN,
            _ => false,
        }
    }

    pub fn any_offered(&self) -> bool {
        (1..=7).chain(9..=11).any(|p| self.offered(p))
    }

    /// The section a choice really improves: an empty building section away from the palace
    /// moves towards the centre until it reaches one next to the building.
    pub fn pick(&self, part: usize) -> Option<usize> {
        if !self.can_improve(part) {
            return None;
        }
        let mut p = part;
        if p < 9 {
            while p < 4 && self.lv(p + 1) <= 0 && self.lv(p) == -1 {
                p += 1;
            }
            while p > 4 && self.lv(p - 1) <= 0 && self.lv(p) == -1 {
                p -= 1;
            }
        }
        Some(p)
    }

    /// Level the section reaches with the next addition.
    pub fn next_level(&self, part: usize) -> i8 {
        (self.lv(part) + 1).clamp(1, MAX_BUILDING)
    }

    /// Raise a section (the style is ignored for gardens).
    pub fn improve(&mut self, part: usize, style: u8) {
        self.level[part] = self.next_level(part);
        if !Self::is_garden(part) {
            self.style[part] = style % 3;
        }
    }

    pub fn kind(&self, i: usize) -> Kind {
        if i == 4 {
            Kind::Middle
        } else if i < 4 {
            if i == 1 || self.lv(i - 1) == -1 { Kind::LeftEnd } else { Kind::Inner }
        } else if i == 7 || self.lv(i + 1) == -1 {
            Kind::RightEnd
        } else {
            Kind::Inner
        }
    }

    /// Piece types of section `i` (0 centre, 1 and 2 walls, 3 left tower, 4 right tower);
    /// `full` is the complete palace, otherwise the single section shown in the style chooser.
    pub fn pieces(&self, i: usize, full: bool) -> [Option<u8>; 2] {
        let l = self.lv(i);
        match self.kind(i) {
            Kind::LeftEnd => [Some(3), (i == 3 || !full).then_some(if l != 0 { 1 } else { 2 })],
            Kind::Inner => {
                let first = if self.lv(i - 1) > l || i == 5 || self.lv(i - 2) == -1 { 1 } else { 2 };
                [Some(first), Some(2)]
            }
            Kind::Middle => [Some(0), None],
            Kind::RightEnd => [(i == 5 || !full).then_some(if self.lv(i - 1) > l || i == 5 { 1 } else { 2 }), Some(4)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computer_palaces_grow_with_the_level() {
        assert_eq!((tri5_level(-3), tri5_level(5), tri5_level(6), tri5_level(15), tri5_level(16)), (0, 0, 1, 1, 2));
        let small = computer_palace(2, 0, 4763);
        assert_eq!(small.level[3..=5].iter().map(|l| *l as i32).sum::<i32>(), 0, "3 parts, none above level 0");
        // 15 parts, levels up to 3 (36 parts overflow the nine slots: after 999 tries a part
        // goes in anyway)
        let big = computer_palace(2, 12, 4763);
        assert!(big.level.iter().all(|l| *l <= 3));
        assert!(big.level[..9].iter().filter(|l| **l > 0).count() > 3);
        assert!((0..9).filter(|i| big.level[*i] >= 0).all(|i| big.style[i] == 2 % 3));
    }

    #[test]
    fn starts_as_stone_building() {
        let p = Palace::default();
        let offered: Vec<usize> = (1..=11).filter(|i| p.offered(*i)).collect();
        assert_eq!(offered, vec![3, 4, 5, 9, 10, 11]);
        assert_eq!(Palace::decode(&p.encode()), p);
        assert_eq!(Palace::decode(&[1, 2]), p);
    }

    #[test]
    fn outer_sections_need_a_raised_neighbour() {
        let mut p = Palace::default();
        assert_eq!(p.pick(1), Some(3));
        assert_eq!(p.pick(7), Some(5));
        p.improve(3, 2);
        assert!(p.offered(2));
        assert_eq!(p.pick(1), Some(2));
        assert_eq!(p.level[3], 1);
        assert_eq!(p.style[3], 2);
        for _ in 0..5 {
            p.improve(3, 0);
        }
        assert_eq!(p.level[3], MAX_BUILDING);
        assert_eq!(p.pick(3), None);
    }
}

/// `tri5_level(n)` (0x4018): the k reached when 5, 10, 15, .. are taken from `n` until it is no
/// longer above 0 is 5 * (k + 1); 0 for n <= 5.
pub fn tri5_level(n: i32) -> i32 {
    let (mut n, mut k) = (n, 0);
    loop {
        k += 5;
        n -= k;
        if n <= 0 {
            return k / 5 - 1;
        }
    }
}

/// A computer civilization's palace as `draw_palace` (0x3823) makes it up from a palace
/// `level`: slots 3..5 start at level 0, then clamp(level + 3, 3, 36) parts each raise the slot
/// (low byte of seed = seed * 5 + 1) % 9 of the 16-bit generator seeded with civ * map seed,
/// redrawn (up to 999 tries) while the slot would pass its inner neighbour, the mirror slot
/// (7 - slot left, 9 - slot right, as CIV.EXE has it) is not built, or the level would exceed
/// clamp / 5 or 3. Every part takes the style civ % 3.
pub fn computer_palace(civ: u8, level: i32, map_seed: u32) -> Palace {
    let parts = (level + 3).clamp(3, 36);
    let max = parts / 5;
    let mut lv = [-1i32; 9];
    lv[3..=5].fill(0);
    let mut p = Palace { level: [-1; SECTIONS], style: [0; SECTIONS] };
    let mut seed = (civ as u16).wrapping_mul(map_seed as u16);
    for _ in 0..parts {
        let mut tries = 0;
        loop {
            seed = seed.wrapping_mul(5).wrapping_add(1);
            let s = (seed & 0xff) as usize % 9;
            let new = lv[s] + 1;
            let bad = (s < 4 && (lv[s + 1] < new || lv[7 - s] == -1)) || (s > 4 && (lv[s - 1] < new || lv[9 - s] == -1)) || new > max || new > 3;
            tries += 1;
            if !bad || tries >= 999 {
                lv[s] = new;
                p.style[s] = civ % 3;
                break;
            }
        }
    }
    for (i, l) in lv.iter().enumerate() {
        p.level[i] = *l as i8;
    }
    p
}
