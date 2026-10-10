//! Miscellaneous constants: map size, city radius, calendar, name lists
//! and menu texts, as decoded from CIV.EXE.

/// World map width in tiles.
pub const MAP_W: usize = 80;
/// World map height in tiles.
pub const MAP_H: usize = 50;

/// The 21 tile offsets of a city's work radius (the "fat cross"), centre
/// first, then the inner ring, then the outer ring.
pub const CITY_RADIUS: [(i8, i8); 21] = [
    (0, 0),
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -2),
    (1, -2),
    (2, -1),
    (2, 0),
    (2, 1),
    (1, 2),
    (0, 2),
    (-1, 2),
    (-2, 1),
    (-2, 0),
    (-2, -1),
    (-1, -2),
];

/// Calendar year for a game turn (turn 0 = 4000 BC).
///
/// 20 years/turn until 1000 AD, 10 until 1500, 5 until 1750, 2 until 1850,
/// then 1 year per turn.
pub fn year_for_turn(turn: u32) -> i32 {
    let t = turn as i32;
    if t <= 250 {
        20 * t - 4000
    } else if t <= 300 {
        1000 + 10 * (t - 250)
    } else if t <= 350 {
        1500 + 5 * (t - 300)
    } else if t <= 400 {
        1750 + 2 * (t - 350)
    } else {
        1850 + (t - 400)
    }
}

/// Formats a year as "4000 BC" / "1 AD" / "1850 AD".  There is no year 0;
/// it is shown as "1 AD".
pub fn year_text(year: i32) -> String {
    if year < 0 {
        format!("{} BC", -year)
    } else if year == 0 {
        "1 AD".to_string()
    } else {
        format!("{year} AD")
    }
}

/// Score-screen rulers ("your rating exceeds even that of ..."), best first.
pub static RATINGS: [&str; 20] = [
    "Solomon the Wise",
    "Emperor Augustus",
    "King Charlemagne",
    "Thomas Jefferson",
    "Shogun Tokugawa",
    "Franklin Roosevelt",
    "Sulayman the Magnificent",
    "Winston Churchill",
    "Cleopatra",
    "Charles De Gaulle",
    "Vladimir Lenin",
    "Simon Bolivar",
    "Otto von Bismarck",
    "Eric the Red",
    "Kaiser Wilhelm",
    "Louis the XVI",
    "Neville Chamberlain",
    "Ferdinand Marcos",
    "Emperor Nero",
    "Dan Quayle",
];

/// Historians quoted on the score screen.
pub static HISTORIANS: [&str; 4] = ["Herodotus", "Pliny", "Toynbee", "Gibbon"];

/// Rating adjectives, best first.
pub static ADJECTIVES: [&str; 7] = [
    "Glorious", "Great", "Fine", "Mediocre", "Puny", "Pathetic", "Hopeless",
];

/// Trade-route commodities.
pub static COMMODITIES: [&str; 8] = [
    "Silk", "Silver", "Wine", "Copper", "Gems", "Dye", "Salt", "Spice",
];

/// Newspaper name suffixes.
pub static NEWSPAPERS: [&str; 5] = ["Weekly", "Today", "Times", "Tribune", "News"];

/// Demographics report categories.
pub static DEMOGRAPHIC_CATEGORIES: [&str; 5] = [
    "WEALTHIEST",
    "MOST POWERFUL",
    "MOST ADVANCED",
    "HAPPIEST",
    "LARGEST",
];

/// Customize World menus: (title, three choices).
pub static CUSTOM_WORLD_OPTIONS: [(&str, [&str; 3]); 4] = [
    ("Land mass", ["Small", "Normal", "Large"]),
    ("Temperature", ["Cool", "Temperate", "Warm"]),
    ("Climate", ["Arid", "Normal", "Wet"]),
    (
        "Age",
        ["3 billion years", "4 billion years", "5 billion years"],
    ),
];

/// Main menu entries.
pub static MAIN_MENU: [&str; 5] = [
    "Start a New Game",
    "Load a Saved Game",
    "EARTH",
    "Customize World",
    "View Hall of Fame",
];

/// Menu bar titles (hotkey = first letter).
pub static MENU_BAR: [&str; 5] = ["GAME", "ORDERS", "ADVISORS", "WORLD", "CIVILOPEDIA"];

/// Spaceship part names.
pub static SPACESHIP_PARTS: [&str; 6] = [
    "Structure",
    "Propulsion",
    "Fuel",
    "Habitation",
    "Life Support",
    "Solar Panel",
];
