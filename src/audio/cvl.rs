//! `ASOUND.CVL`: the AdLib sound driver overlay. A small DOS MZ executable whose data
//! segment holds every tune and effect as per-voice bytecode (see `driver.rs`). This module
//! only loads the image and the tables the interpreter needs.
//!
//! Image offset `x` = file offset `x + header`; the data segment starts at image `+0x1E20`.
//! Offsets are those of the driver shipped with v474 (`RLND Cvlzatn12-03-91`); the stream
//! table in `driver.rs` was read out of that driver's `PlayTune` handlers, so other versions
//! (the 1994 patched drivers) are rejected rather than played wrongly.

use std::fmt;

/// Data segment start inside the MZ image.
pub const DS_OFF: usize = 0x1e20;
/// DS offset of the 57 instrument patches, 44 bytes each.
pub const PATCH_TABLE: usize = 0x695a;
pub const PATCH_SIZE: usize = 0x2c;
pub const PATCH_COUNT: usize = 57;
/// ID string at image `+0x10`.
pub const ID_STRING: &[u8; 20] = b"RLND Cvlzatn12-03-91";

#[derive(Debug)]
pub enum CvlError {
    TooShort,
    NotMz,
    /// The image header's ID string is not the one of the supported driver.
    WrongVersion(String),
}

impl fmt::Display for CvlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CvlError::TooShort => write!(f, "file too short for a sound driver"),
            CvlError::NotMz => write!(f, "not an MZ executable"),
            CvlError::WrongVersion(s) => write!(f, "unsupported sound driver version {s:?} (expected {:?})", String::from_utf8_lossy(ID_STRING)),
        }
    }
}

impl std::error::Error for CvlError {}

/// The driver's data segment plus the lookup tables the interpreter reads from it.
pub struct Image {
    /// The whole 64 KiB data segment (file contents followed by zeroed BSS).
    pub ds: Vec<u8>,
    /// F-numbers of the 12 notes of an octave (DS `0x73A8`).
    pub fnum: [u16; 12],
    /// Voice -> (modulator, carrier) operator index (DS `0x73C6`).
    pub voice_ops: [(u8, u8); 9],
    /// Operator index -> register offset (DS `0x73D8`).
    pub op_reg: [u8; 18],
    /// Random number seed (DS `0x73FD`).
    pub rng_seed: u16,
    /// Deep tremolo / deep vibrato / note select flags (DS `0x73C0..`).
    pub flags: (u8, u8, u8),
    /// Index of the last war-drum pattern played (DS `0x73F5`).
    pub id33_index: u16,
}

impl Image {
    pub fn parse(file: &[u8]) -> Result<Image, CvlError> {
        if file.len() < 0x40 {
            return Err(CvlError::TooShort);
        }
        if &file[0..2] != b"MZ" {
            return Err(CvlError::NotMz);
        }
        let hdr = u16::from_le_bytes([file[8], file[9]]) as usize * 16;
        let img = file.get(hdr..).ok_or(CvlError::TooShort)?;
        let id = img.get(0x10..0x24).ok_or(CvlError::TooShort)?;
        if id != ID_STRING {
            return Err(CvlError::WrongVersion(String::from_utf8_lossy(id).into_owned()));
        }
        let ds_src = img.get(DS_OFF..).ok_or(CvlError::TooShort)?;
        if ds_src.len() < 0x7403 {
            return Err(CvlError::TooShort);
        }
        let mut ds = vec![0u8; 0x10000];
        ds[..ds_src.len()].copy_from_slice(ds_src);
        let word = |a: usize| u16::from_le_bytes([ds[a], ds[a + 1]]);
        let mut fnum = [0u16; 12];
        for (i, f) in fnum.iter_mut().enumerate() {
            *f = word(0x73a8 + 2 * i);
        }
        let mut voice_ops = [(0u8, 0u8); 9];
        for (i, v) in voice_ops.iter_mut().enumerate() {
            *v = (ds[0x73c6 + 2 * i], ds[0x73c7 + 2 * i]);
        }
        let mut op_reg = [0u8; 18];
        op_reg.copy_from_slice(&ds[0x73d8..0x73d8 + 18]);
        Ok(Image {
            fnum,
            voice_ops,
            op_reg,
            rng_seed: word(0x73fd),
            flags: (ds[0x73c0], ds[0x73c2], ds[0x73c4]),
            id33_index: word(0x73f5),
            ds,
        })
    }

    /// Instrument patch `n`: (modulator record, carrier record), 22 bytes each.
    pub fn patch(&self, n: usize) -> (&[u8], &[u8]) {
        let b = PATCH_TABLE + n * PATCH_SIZE;
        (&self.ds[b..b + 0x16], &self.ds[b + 0x16..b + 0x2c])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load() -> Option<Image> {
        let dir = crate::assets::AssetDir::locate(None).ok()?;
        let bytes = dir.read("ASOUND.CVL").ok()?;
        Some(Image::parse(&bytes).expect("ASOUND.CVL parses"))
    }

    #[test]
    fn tables() {
        let Some(im) = load() else { return };
        assert_eq!(im.fnum, [512, 542, 575, 609, 645, 683, 724, 767, 813, 861, 912, 967]);
        assert_eq!(im.voice_ops[0], (0, 3));
        assert_eq!(im.voice_ops[8], (14, 17));
        assert_eq!(im.op_reg, [0, 1, 2, 3, 4, 5, 8, 9, 10, 11, 12, 13, 16, 17, 18, 19, 20, 21]);
        assert_eq!(im.rng_seed, 0x04d2);
        assert_eq!(im.flags, (1, 1, 1));
        assert_eq!(im.id33_index, 1);
        assert_eq!(&im.ds[0x10..0x1a], b"Copyright ");
    }
}
