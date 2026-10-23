//! Civilopedia data: the entries of each category, their BLURB text keys and the picture
//! tables (advance icons, unit / terrain / improvement / wonder pictures).
//!
//! Picture cells were measured on the decoded sheets; the mapping of cells to names was done
//! by looking at the artwork, so a few entries are best guesses (marked below).

use crate::assets::text::paragraphs;
use crate::gfx::Rect;
use crate::rules::{self, AdvanceId, ImprovementId, Terrain, UnitTypeId, WonderId};
use crate::ui::context::UiContext;

/// One Civilopedia entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Advance(AdvanceId),
    Improvement(ImprovementId),
    Wonder(WonderId),
    Unit(UnitTypeId),
    Terrain(Terrain),
    /// Index into [`CONCEPTS`].
    Concept(usize),
}

/// A "Miscellaneous" entry: display name and BLURB4 section key.
pub struct Concept {
    pub name: &'static str,
    pub key: &'static str,
}

/// The 21 game concepts of BLURB4 (governments included), in file order.
pub static CONCEPTS: [Concept; 21] = [
    Concept { name: "Roads", key: "ROADS" },
    Concept { name: "RailRoads", key: "RAILROADS" },
    Concept { name: "Irrigation", key: "IRRIGATION" },
    Concept { name: "Mining", key: "MINING" },
    Concept { name: "Fortify", key: "FORTIFY" },
    Concept { name: "Fortress", key: "FORTRESS" },
    Concept { name: "Pillage", key: "PILLAGE" },
    Concept { name: "Disband", key: "DISBAND" },
    Concept { name: "Veteran Units", key: "VETERAN UNITS" },
    Concept { name: "Anarchy", key: "ANARCHY" },
    Concept { name: "Despotism", key: "DESPOTISM" },
    Concept { name: "Monarchy", key: "MONARCHY" },
    Concept { name: "Communism", key: "COMMUNISM" },
    Concept { name: "Republic", key: "REPUBLIC" },
    Concept { name: "Democracy", key: "DEMOCRACY" },
    Concept { name: "Pollution", key: "POLLUTION" },
    Concept { name: "Sentry", key: "SENTRY" },
    Concept { name: "Luxuries", key: "LUXURIES" },
    Concept { name: "Taxes", key: "TAXES" },
    Concept { name: "Science", key: "SCIENCE" },
    Concept { name: "Trade Routes", key: "TRADE ROUTES" },
];

/// Civilopedia categories as numbered by the CIVILOPEDIA menu.
pub const CAT_COMPLETE: usize = 0;
pub const CAT_ADVANCES: usize = 1;
pub const CAT_IMPROVEMENTS: usize = 2;
pub const CAT_UNITS: usize = 3;
pub const CAT_TERRAIN: usize = 4;
pub const CAT_MISC: usize = 5;

impl Entry {
    pub fn name(self) -> String {
        match self {
            Entry::Advance(a) => rules::ADVANCES[a as usize].name.to_string(),
            Entry::Improvement(i) => rules::IMPROVEMENTS[i as usize].name.to_string(),
            Entry::Wonder(w) => rules::WONDERS[w as usize].name.to_string(),
            Entry::Unit(u) => rules::UNITS[u as usize].name.to_string(),
            Entry::Terrain(t) => t.name().to_string(),
            Entry::Concept(c) => CONCEPTS[c].name.to_string(),
        }
    }

    /// The grey line under the title.
    pub fn subtitle(self) -> &'static str {
        match self {
            Entry::Advance(_) => "Civilization Advance",
            Entry::Improvement(_) => "City Improvement",
            Entry::Wonder(_) => "Wonder of the World",
            Entry::Unit(_) => "Military Unit",
            Entry::Terrain(_) => "Terrain Type",
            Entry::Concept(_) => "Game Concept",
        }
    }


    /// BLURB file index and section key of the history text.
    fn blurb_key(self) -> Option<(usize, String)> {
        match self {
            Entry::Advance(a) => Some((0, rules::advance_pedia_key(a))),
            Entry::Improvement(i) => Some((1, rules::IMPROVEMENTS[i as usize].name.to_string())),
            Entry::Wonder(w) => Some((1, rules::WONDERS[w as usize].name.to_string())),
            Entry::Unit(u) => Some((2, rules::UNITS[u as usize].name.to_string())),
            Entry::Terrain(_) => None,
            Entry::Concept(c) => Some((4, CONCEPTS[c].key.to_string())),
        }
    }

    /// History text (paragraphs) of the entry.
    pub fn blurb(self, ctx: &UiContext) -> Vec<String> {
        match self.blurb_key() {
            Some((file, key)) => ctx.texts.blurb[file].section(&key).map(paragraphs).unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// Game-effect note (the `NAME2` section), if the entry has one.
    pub fn blurb2(self, ctx: &UiContext) -> Vec<String> {
        let text = match self.blurb_key() {
            Some((file, key)) if file != 0 => ctx.texts.blurb[file].section(&format!("{key}2")).map(paragraphs).unwrap_or_default(),
            _ => Vec::new(),
        };
        match self {
            Entry::Wonder(w) => mark_missing_obsolescence(w, text),
            _ => text,
        }
    }
}

/// A wonder whose page says it works "until the development of" an advance, while the rules
/// (CIV.EXE's table) never make it obsolete, gets "not implemented: " in front of that part
/// (Copernicus' Observatory and the Automobile).
fn mark_missing_obsolescence(w: WonderId, text: Vec<String>) -> Vec<String> {
    if rules::WONDERS[w as usize].obsolete_by.is_some() {
        return text;
    }
    text.into_iter().map(|p| p.replacen("until the development", "not implemented: until the development", 1)).collect()
}

fn sort_entries(mut v: Vec<Entry>) -> Vec<Entry> {
    v.sort_by_key(|e| e.name().to_ascii_lowercase());
    v
}

/// Entries of a category, alphabetically (case-insensitive) sorted.
pub fn entries(category: usize) -> Vec<Entry> {
    let advances = || (0..rules::advances::REAL_ADVANCE_COUNT as u8).map(Entry::Advance);
    let improvements = || (1..rules::IMPROVEMENTS.len() as u8).map(Entry::Improvement).chain((1..rules::WONDERS.len() as u8).map(Entry::Wonder));
    let units = || (0..rules::UNITS.len() as u8).map(Entry::Unit);
    let terrain = || Terrain::ALL.iter().copied().map(Entry::Terrain);
    let concepts = || (0..CONCEPTS.len()).map(Entry::Concept);
    let v: Vec<Entry> = match category {
        CAT_ADVANCES => advances().collect(),
        CAT_IMPROVEMENTS => improvements().collect(),
        CAT_UNITS => units().collect(),
        CAT_TERRAIN => terrain().collect(),
        CAT_MISC => concepts().collect(),
        _ => advances().chain(improvements()).chain(units()).chain(terrain()).chain(concepts()).collect(),
    };
    sort_entries(v)
}

// ---------------------------------------------------------------------------------------------
// Picture tables
// ---------------------------------------------------------------------------------------------

/// Cell of the 3x3 advance-icon pages ICONPG1..ICONPG8 (grid lines at x 0/111/222, y 0/69/138).
const fn adv_cell(page: u8, row: i32, col: i32) -> (&'static str, Rect) {
    let name = match page {
        1 => "ICONPG1",
        2 => "ICONPG2",
        3 => "ICONPG3",
        4 => "ICONPG4",
        5 => "ICONPG5",
        6 => "ICONPG6",
        7 => "ICONPG7",
        _ => "ICONPG8",
    };
    (name, Rect::new(1 + 111 * col, 1 + 69 * row, 110, 68))
}

/// Civilopedia icon of an advance: sheet base name and the cell rectangle (110x68, drawn
/// opaque at (7,8) on the entry page; background index 253).
///
/// Mapping by artwork; confidence notes: "?" = plausible but unverified guess.
pub fn advance_icon(id: AdvanceId) -> Option<(&'static str, Rect)> {
    use rules::advance_ids as a;
    let c = match id {
        a::ALPHABET => adv_cell(3, 1, 2),            // letters "ABCDEFG / Aa"
        a::CODE_OF_LAWS => adv_cell(2, 1, 2),        // raised hands over a flag (oath) ?
        a::CURRENCY => adv_cell(5, 0, 0),            // coins and a bank note
        a::ATOMIC_THEORY => adv_cell(5, 1, 2),       // atom
        a::DEMOCRACY => adv_cell(2, 1, 0),           // Statue of Liberty
        a::MONARCHY => adv_cell(2, 0, 2),            // crown
        a::ASTRONOMY => adv_cell(6, 0, 0),           // telescope
        a::MAP_MAKING => adv_cell(7, 2, 1),          // map with compass rose
        a::NAVIGATION => adv_cell(6, 2, 2),          // ship's wheel and chart
        a::MATHEMATICS => adv_cell(7, 1, 1),         // abacus
        a::MEDICINE => adv_cell(3, 2, 1),            // caduceus
        a::PHYSICS => adv_cell(1, 0, 2),             // pulley and weight ?
        a::ENGINEERING => adv_cell(4, 1, 1),         // blueprint, compasses, hammer ?
        a::UNIVERSITY => adv_cell(1, 2, 0),          // mortarboard and diploma
        a::MAGNETISM => adv_cell(6, 1, 0),           // horseshoe magnet
        a::ELECTRONICS => adv_cell(4, 1, 2),         // circuit board
        a::MASONRY => adv_cell(2, 2, 1),             // bricks and trowel
        a::BRONZE_WORKING => adv_cell(5, 1, 1),      // shield, spears and fire ?
        a::IRON_WORKING => adv_cell(7, 1, 2),        // crucible pouring metal ?
        a::BRIDGE_BUILDING => adv_cell(4, 0, 1),     // suspension bridge
        a::INVENTION => adv_cell(6, 1, 2),           // gramophone, light bulb, flask
        a::COMPUTERS => adv_cell(3, 2, 0),           // personal computer
        a::WRITING => adv_cell(3, 1, 1),             // quill and inkwell
        a::STEAM_ENGINE => adv_cell(6, 2, 1),        // boiler and turbine
        a::TRADE => adv_cell(1, 0, 0),               // hands bartering ?
        a::CEREMONIAL_BURIAL => adv_cell(8, 0, 2),   // sarcophagus
        a::MYSTICISM => adv_cell(1, 1, 1),           // meditating figure ?
        a::NUCLEAR_FISSION => adv_cell(5, 1, 0),     // mushroom cloud
        a::PHILOSOPHY => adv_cell(8, 0, 1),          // The Thinker
        a::RELIGION => adv_cell(3, 0, 2),            // stained-glass window
        a::LITERACY => adv_cell(4, 2, 1),            // books
        a::HORSEBACK_RIDING => adv_cell(7, 1, 0),    // horse
        a::FEUDALISM => adv_cell(1, 0, 1),           // castle
        a::THE_WHEEL => adv_cell(3, 2, 2),           // stone wheel on the ground ?
        a::GUNPOWDER => adv_cell(6, 0, 1),           // musket and powder
        a::INDUSTRIALIZATION => adv_cell(2, 2, 0),   // factory
        a::CHEMISTRY => adv_cell(1, 1, 2),           // red vessel with glass dome ?
        a::COMBUSTION => adv_cell(4, 0, 2),          // engine ?
        a::FLIGHT => adv_cell(3, 0, 1),              // Leonardo's wings
        a::ADVANCED_FLIGHT => adv_cell(2, 0, 1),     // stealth bomber (verified)
        a::SPACE_FLIGHT => adv_cell(4, 2, 2),        // Earth through a porthole
        a::MASS_PRODUCTION => adv_cell(5, 0, 1),     // row of Model T cars
        a::POTTERY => adv_cell(7, 2, 0),             // vases
        a::COMMUNISM => adv_cell(4, 2, 0),           // hammer and sickle
        a::THE_REPUBLIC => adv_cell(2, 0, 0),        // eagle on a globe ?
        a::CONSTRUCTION => adv_cell(4, 1, 0),        // hammer and house frame
        a::ROCKETRY => adv_cell(8, 1, 0),            // rocket on a gantry
        a::THE_CORPORATION => adv_cell(7, 2, 2),     // chart and executives ?
        a::METALLURGY => adv_cell(5, 0, 2),          // cauldron over fire ?
        a::RAILROAD => adv_cell(3, 1, 0),            // locomotive
        a::NUCLEAR_POWER => adv_cell(2, 2, 2),       // radiation symbol
        a::THEORY_OF_GRAVITY => adv_cell(2, 1, 1),   // falling balls
        a::STEEL => adv_cell(3, 0, 0),               // steel mill and blast furnace ?
        a::BANKING => adv_cell(1, 2, 2),             // piggy bank
        a::ELECTRICITY => adv_cell(5, 2, 2),         // circuit with bulb and meters
        a::REFINING => adv_cell(6, 0, 2),            // oil pump and barrels ?
        a::EXPLOSIVES => adv_cell(5, 2, 1),          // dynamite
        a::SUPERCONDUCTOR => adv_cell(8, 0, 0),      // sparking red cube ?
        a::AUTOMOBILE => adv_cell(6, 2, 0),          // early automobile
        a::GENETIC_ENGINEERING => adv_cell(1, 2, 1), // embryo and molecules
        a::PLASTICS => adv_cell(4, 0, 0),            // plastic bags and bottles ?
        a::RECYCLING => adv_cell(5, 2, 0),           // recycling arrows
        a::CHIVALRY => adv_cell(6, 1, 1),            // sword and pennant
        a::ROBOTICS => adv_cell(7, 0, 2),            // robot arm
        a::CONSCRIPTION => adv_cell(7, 0, 0),        // row of soldiers
        a::LABOR_UNION => adv_cell(1, 1, 0),         // handshake, hammer and shovel ?
        a::FUSION_POWER => adv_cell(7, 0, 1),        // colliding atoms in a sun
        _ => return None,
    };
    Some(c)
}

/// Picture of a unit type in ICONPGA..ICONPGE (2x3 grid of ~160x60 cells with a cloud
/// background; background index 253). Drawn with key 253 at (9,13) on the entry page.
pub fn unit_picture(kind: UnitTypeId) -> (&'static str, Rect) {
    use rules::unit_ids as u;
    let (page, row, col) = match kind {
        u::MUSKETEERS => ("ICONPGA", 0, 0),
        u::BATTLESHIP => ("ICONPGA", 0, 1),
        u::IRONCLAD => ("ICONPGA", 1, 0),
        u::FIGHTER => ("ICONPGA", 1, 1),
        u::TRANSPORT => ("ICONPGA", 2, 0),
        u::BOMBER => ("ICONPGA", 2, 1),
        u::ARTILLERY => ("ICONPGB", 0, 0),
        u::FRIGATE => ("ICONPGB", 0, 1),
        u::TRIREME => ("ICONPGB", 1, 0),
        u::SAIL => ("ICONPGB", 1, 1),
        u::CATAPULT => ("ICONPGB", 2, 0),
        u::CANNON => ("ICONPGB", 2, 1),
        u::MECH_INF => ("ICONPGC", 0, 0),
        u::DIPLOMAT => ("ICONPGC", 0, 1),
        u::CRUISER => ("ICONPGC", 1, 0),
        u::CAVALRY => ("ICONPGC", 1, 1),
        u::MILITIA => ("ICONPGC", 2, 0),
        u::SUBMARINE => ("ICONPGC", 2, 1),
        u::NUCLEAR => ("ICONPGD", 0, 0),
        u::CARRIER => ("ICONPGD", 0, 1),
        u::ARMOR => ("ICONPGD", 1, 0), // verified against the capture
        u::SETTLERS => ("ICONPGD", 1, 1),
        u::CHARIOT => ("ICONPGD", 2, 0),
        u::RIFLEMEN => ("ICONPGD", 2, 1),
        u::PHALANX => ("ICONPGE", 0, 0),
        u::LEGION => ("ICONPGE", 0, 1),
        u::CARAVAN => ("ICONPGE", 1, 0),
        _ => ("ICONPGE", 1, 1), // Knights
    };
    // Page E's second row sits a little lower and taller than on the other pages.
    let (y, h) = match (page, row) {
        (_, 0) => (1, 60),
        ("ICONPGE", _) => (63, 66),
        (_, 1) => (62, 60),
        _ => (123, 60),
    };
    // column 0 of the sheet is a guide column that is never shown
    (page, Rect::new(1 + col * 160, y, 159, h))
}

/// Picture of a terrain type: the content rectangle inside its ICONPGT1/ICONPGT2 cell
/// (3x2 cells of 106x86 separated by magenta lines; background index 253).
pub fn terrain_picture(t: Terrain) -> (&'static str, Rect) {
    match t {
        Terrain::Desert => ("ICONPGT2", Rect::new(7, 21, 90, 48)),
        Terrain::Plains => ("ICONPGT2", Rect::new(21, 102, 62, 62)),
        Terrain::Grassland => ("ICONPGT2", Rect::new(115, 25, 94, 42)),
        Terrain::Forest => ("ICONPGT2", Rect::new(225, 20, 85, 55)),
        Terrain::Hills => ("ICONPGT2", Rect::new(127, 103, 67, 60)),
        Terrain::Mountains => ("ICONPGT2", Rect::new(232, 103, 68, 60)),
        Terrain::Tundra => ("ICONPGT1", Rect::new(4, 25, 100, 39)),
        Terrain::Arctic => ("ICONPGT1", Rect::new(131, 15, 60, 60)), // verified
        Terrain::Swamp => ("ICONPGT1", Rect::new(225, 21, 91, 45)),
        Terrain::Jungle => ("ICONPGT1", Rect::new(7, 109, 90, 52)),
        Terrain::Ocean => ("ICONPGT1", Rect::new(129, 99, 65, 65)),
        Terrain::River => ("ICONPGT1", Rect::new(224, 111, 92, 40)),
    }
}

/// City-view building of an improvement in CITYPIX2 (6x4 grid of 49x49 cells on black;
/// background index 0). The Palace and the spaceship parts have no building picture.
pub fn improvement_picture(id: ImprovementId) -> Option<(&'static str, Rect)> {
    use rules::improvement_ids as i;
    let (row, col) = match id {
        i::BARRACKS => (0, 0),      // long brown barracks
        i::LIBRARY => (0, 1),       // colonnade with a statue ?
        i::BANK => (0, 2),          // verified against the capture
        i::COLOSSEUM => (0, 3),     // arena
        i::RECYCLING_CNTR => (0, 4), // recycling logo
        i::GRANARY => (1, 0),       // beehive-shaped granary
        i::UNIVERSITY => (1, 1),    // colonnade with a statue on top ?
        i::CATHEDRAL => (1, 2),     // spired church
        i::FACTORY => (1, 3),       // three-stack factory
        i::POWER_PLANT => (1, 4),   // pylon and power lines ?
        i::TEMPLE => (2, 0),        // domed temple
        i::CITY_WALLS => (2, 1),    // wall segment with gate
        i::COURTHOUSE => (2, 2),    // brick hall with clock tower ?
        i::MFG_PLANT => (2, 3),     // plant with silo ?
        i::HYDRO_PLANT => (2, 4),   // building with water channel ?
        i::MARKETPLACE => (3, 0),   // market stalls ?
        i::AQUEDUCT => (3, 1),      // arches with water ?
        i::MASS_TRANSIT => (3, 2),  // bus
        i::SDI_DEFENSE => (3, 3),   // radar on a tower
        i::NUCLEAR_PLANT => (3, 4), // cooling tower
        _ => return None,
    };
    Some(("CITYPIX2", Rect::new(1 + 50 * col, 1 + 50 * row, 49, 49)))
}

/// Picture of a wonder: its sprite in the city view (WONDERS.PIC, or WONDERS2.PIC for the four
/// large-scale wonders).
pub fn wonder_picture(w: WonderId) -> (&'static str, Rect) {
    crate::ui::city_view::wonder_sprite(w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::wonder_ids as won;

    #[test]
    fn copernicus_page_says_its_obsolescence_is_not_implemented() {
        let text = vec!["Doubles knowledge production in city, until the development of the AUTOMOBILE.".to_string()];
        assert_eq!(
            mark_missing_obsolescence(won::COPERNICUS_OBSERVATORY, text.clone()),
            vec!["Doubles knowledge production in city, not implemented: until the development of the AUTOMOBILE.".to_string()]
        );
        // wonders that do become obsolete keep their text
        assert_eq!(mark_missing_obsolescence(won::ORACLE, text.clone()), text);
    }
}
