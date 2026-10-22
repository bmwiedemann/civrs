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
