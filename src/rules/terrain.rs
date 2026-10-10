//! Terrain types (12), special resources and the terraforming table, as
//! decoded from CIV.EXE.

/// Terrain type; the discriminant is the in-game terrain index.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Terrain {
    Desert = 0,
    Plains = 1,
    Grassland = 2,
    Forest = 3,
    Hills = 4,
    Mountains = 5,
    Tundra = 6,
    Arctic = 7,
    Swamp = 8,
    Jungle = 9,
    Ocean = 10,
    River = 11,
}

impl Terrain {
    pub const ALL: [Terrain; 12] = [
        Terrain::Desert,
        Terrain::Plains,
        Terrain::Grassland,
        Terrain::Forest,
        Terrain::Hills,
        Terrain::Mountains,
        Terrain::Tundra,
        Terrain::Arctic,
        Terrain::Swamp,
        Terrain::Jungle,
        Terrain::Ocean,
        Terrain::River,
    ];

    pub fn from_u8(v: u8) -> Option<Terrain> {
        Self::ALL.get(v as usize).copied()
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    /// Terrain for a MAP.PIC (Earth) terrain code.
    pub fn from_earth_code(code: u8) -> Option<Terrain> {
        Self::ALL
            .iter()
            .copied()
            .find(|t| t.info().earth_code == code)
    }

    pub fn info(self) -> &'static TerrainInfo {
        &TERRAIN[self as usize]
    }

    pub fn name(self) -> &'static str {
        self.info().name
    }

    pub fn is_ocean(self) -> bool {
        self == Terrain::Ocean
    }
}

/// Result of irrigating or mining a terrain type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Terraform {
    /// Not possible on this terrain.
    No,
    /// Adds this many food (irrigate) or shields (mine) to the tile.
    Bonus(u8),
    /// Transforms the tile into another terrain type.
    Becomes(Terrain),
}

/// Special resource that may appear on a terrain type; yields replace the
/// base terrain yields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Special {
    pub name: &'static str,
    pub food: u8,
    pub shields: u8,
    pub trade: u8,
}

/// Static description of one terrain type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TerrainInfo {
    pub name: &'static str,
    pub move_cost: u8,
    /// Defence multiplier in halves (2 = x1, 3 = x1.5, 4 = x2, 6 = x3).
    pub defense: u8,
    pub food: u8,
    pub shields: u8,
    pub trade: u8,
    pub special: Option<Special>,
    pub irrigate: Terraform,
    pub irrigate_turns: u8,
    pub mine: Terraform,
    pub mine_turns: u8,
    /// Terrain code used by MAP.PIC (the Earth map).
    pub earth_code: u8,
}

const fn special(name: &'static str, food: u8, shields: u8, trade: u8) -> Option<Special> {
    Some(Special {
        name,
        food,
        shields,
        trade,
    })
}

#[allow(clippy::too_many_arguments)]
const fn terrain(
    name: &'static str,
    move_cost: u8,
    defense: u8,
    food: u8,
    shields: u8,
    trade: u8,
    earth_code: u8,
    special: Option<Special>,
    irrigate: Terraform,
    irrigate_turns: u8,
    mine: Terraform,
    mine_turns: u8,
) -> TerrainInfo {
    TerrainInfo {
        name,
        move_cost,
        defense,
        food,
        shields,
        trade,
        special,
        irrigate,
        irrigate_turns,
        mine,
        mine_turns,
        earth_code,
    }
}

use Terraform::{Becomes, Bonus, No};

/// The 12 terrain types, in in-game order.
///
/// Terraforming table codes from the executable: -2 = +1 bonus, -4 = +3
/// bonus, -1 = not available, >= 0 = becomes that terrain.
#[rustfmt::skip]
pub static TERRAIN: [TerrainInfo; 12] = [
    //      name         move def food shld trd earth special                 irrigate         turns mine                 turns
    terrain("Desert", 1, 2, 0, 1, 0, 14, special("Oasis", 3, 1, 0), Bonus(1), 5, Bonus(1), 5),
    terrain("Plains", 1, 2, 1, 1, 0, 6, special("Horses", 1, 3, 0), Bonus(1), 5, Becomes(Terrain::Forest), 15),
    // Raw table shields is 1; only "shield" grassland tiles (the resource
    // pattern) yield it, so the base value here is 0.
    terrain("Grassland", 1, 2, 2, 0, 0, 10, None, Bonus(1), 5, Becomes(Terrain::Forest), 10),
    terrain("Forest", 2, 3, 1, 2, 0, 2, special("Game", 3, 2, 0), Becomes(Terrain::Plains), 5, No, 0),
    terrain("Hills", 2, 4, 1, 0, 0, 12, special("Coal", 1, 2, 0), Bonus(1), 10, Bonus(3), 10),
    terrain("Mountains", 3, 6, 0, 1, 0, 13, special("Gold", 0, 1, 6), No, 0, Bonus(1), 10),
    terrain("Tundra", 1, 2, 1, 0, 0, 7, special("Game", 3, 0, 0), No, 0, No, 0),
    terrain("Arctic", 2, 2, 0, 0, 0, 15, special("Seals", 2, 0, 0), No, 0, No, 0),
    terrain("Swamp", 2, 3, 1, 0, 0, 3, special("Oil", 1, 4, 0), Becomes(Terrain::Grassland), 15, Becomes(Terrain::Forest), 15),
    terrain("Jungle", 2, 3, 1, 0, 0, 11, special("Gems", 1, 0, 4), Becomes(Terrain::Grassland), 15, Becomes(Terrain::Forest), 15),
    terrain("Ocean", 1, 2, 1, 0, 2, 1, special("Fish", 3, 0, 2), No, 0, No, 0),
    terrain("River", 1, 3, 2, 1, 1, 9, None, Bonus(1), 5, No, 0),
];

/// Turns for a Settlers unit to build a road (manual).
pub const ROAD_TURNS: u8 = 2;
/// Turns for a Settlers unit to build a railroad (manual).
pub const RAILROAD_TURNS: u8 = 6;
/// Turns for a Settlers unit to build a fortress.
pub const FORTRESS_TURNS: u8 = 3;
/// Turns for a Settlers unit to clean pollution (manual).
pub const CLEAN_POLLUTION_TURNS: u8 = 3;
