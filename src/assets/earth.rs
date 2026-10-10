//! MAP.PIC: the Earth map, stored as the top-left 80x50 pixels of a 16-colour picture whose
//! EGA colour index encodes the terrain (1 Ocean, 10 Grassland, 6 Plains, 2 Forest, 14 Desert,
//! 15 Arctic, 9 River, 12 Hills, 7 Tundra, 13 Mountains, 11 Jungle, 3 Swamp).

use super::pic::Pic;

pub const EARTH_W: usize = 80;
pub const EARTH_H: usize = 50;

#[derive(Clone, Debug)]
pub struct EarthMap {
    /// Raw terrain codes (EGA colour indices), row-major 80x50.
    pub codes: Vec<u8>,
}

impl EarthMap {
    pub fn code(&self, x: usize, y: usize) -> u8 {
        self.codes[y * EARTH_W + x]
    }
}

pub fn decode_earth(pic: &Pic) -> Result<EarthMap, String> {
    if (pic.width as usize) < EARTH_W || (pic.height as usize) < EARTH_H {
        return Err(format!("MAP.PIC is {}x{}, expected at least 80x50", pic.width, pic.height));
    }
    let mut codes = Vec::with_capacity(EARTH_W * EARTH_H);
    for y in 0..EARTH_H {
        for x in 0..EARTH_W {
            codes.push(pic.get(x, y));
        }
    }
    Ok(EarthMap { codes })
}
