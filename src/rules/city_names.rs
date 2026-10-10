//! City name lists as decoded from CIV.EXE: 16 names per civilization
//! (in civ order 1..7, 9..15) followed by a spare pool of 32 names.

#[rustfmt::skip]
pub static ROMAN: [&str; 16] = [
    "Rome", "Caesarea", "Carthage", "Nicopolis", "Byzantium", "Brundisium", "Syracuse", "Antioch",
    "Palmyra", "Cyrene", "Gordion", "Tyrus", "Jerusalem", "Seleucia", "Ravenna", "Artaxata",
];
#[rustfmt::skip]
pub static BABYLONIAN: [&str; 16] = [
    "Babylon", "Sumer", "Uruk", "Nineveh", "Ashur", "Ellipi", "Akkad", "Eridu",
    "Kish", "Nippur", "Shuruppak", "Zariqum", "Izibia", "Nimrud", "Arbela", "Zamua",
];
#[rustfmt::skip]
pub static GERMAN: [&str; 16] = [
    "Berlin", "Leipzig", "Hamburg", "Bremen", "Frankfurt", "Bonn", "Nuremberg", "Cologne",
    "Hannover", "Munich", "Stuttgart", "Heidelburg", "Salzburg", "Konigsberg", "Dortmund", "Brandenburg",
];
#[rustfmt::skip]
pub static EGYPTIAN: [&str; 16] = [
    "Thebes", "Memphis", "Oryx", "Heliopolis", "Gaza", "Alexandria", "Byblos", "Cairo",
    "Coptos", "Edfu", "Pithom", "Busiris", "Athribis", "Mendes", "Tanis", "Abydos",
];
#[rustfmt::skip]
pub static AMERICAN: [&str; 16] = [
    "Washington", "New York", "Boston", "Philadelphia", "Atlanta", "Chicago", "Buffalo", "St.Louis",
    "Detroit", "New Orleans", "Baltimore", "Denver", "Cincinnati", "Dallas", "Los Angeles", "Las Vegas",
];
#[rustfmt::skip]
pub static GREEK: [&str; 16] = [
    "Athens", "Sparta", "Corinth", "Delphi", "Eretria", "Pharsalos", "Argos", "Mycenae",
    "Herakleia", "Antioch", "Ephesos", "Rhodes", "Knossos", "Troy", "Pergamon", "Miletos",
];
#[rustfmt::skip]
pub static INDIAN: [&str; 16] = [
    "Delhi", "Bombay", "Madras", "Bangalore", "Calcutta", "Lahore", "Karachi", "Kolhapur",
    "Jaipur", "Hyderabad", "Bengal", "Chittagong", "Punjab", "Dacca", "Indus", "Ganges",
];
#[rustfmt::skip]
pub static RUSSIAN: [&str; 16] = [
    "Moscow", "Leningrad", "Kiev", "Minsk", "Smolensk", "Odessa", "Sevastopol", "Tblisi",
    "Sverdlovsk", "Yakutsk", "Vladivostok", "Novograd", "Krasnoyarsk", "Riga", "Rostov", "Astrakhan",
];
#[rustfmt::skip]
pub static ZULU: [&str; 16] = [
    "Zimbabwe", "Ulundi", "Bapedi", "Hlobane", "Isandhlwana", "Intombe", "Mpondo", "Ngome",
    "Swazi", "Tugela", "Umtata", "Umfolozi", "Ibabanago", "Isipezi", "Amatikulu", "Zunguin",
];
#[rustfmt::skip]
pub static FRENCH: [&str; 16] = [
    "Paris", "Orleans", "Lyons", "Tours", "Chartres", "Bordeaux", "Rouen", "Avignon",
    "Marseilles", "Grenoble", "Dijon", "Amiens", "Cherbourg", "Poitiers", "Toulouse", "Bayonne",
];
#[rustfmt::skip]
pub static AZTEC: [&str; 16] = [
    "Tenochtitlan", "Chiauhtia", "Chapultepec", "Coatepec", "Ayotzinco", "Itzapalapa", "Iztapam", "Mitxcoac",
    "Tacubaya", "Tecamac", "Tepezinco", "Ticoman", "Tlaxcala", "Xaltocan", "Xicalango", "Zumpanco",
];
#[rustfmt::skip]
pub static CHINESE: [&str; 16] = [
    "Peking", "Shanghai", "Canton", "Nanking", "Tsingtao", "Hangchow", "Tientsin", "Tatung",
    "Macao", "Anyang", "Shantung", "Chinan", "Kaifeng", "Ningpo", "Paoting", "Yangchow",
];
#[rustfmt::skip]
pub static ENGLISH: [&str; 16] = [
    "London", "Coventry", "Birmingham", "Dover", "Nottingham", "York", "Liverpool", "Brighton",
    "Oxford", "Reading", "Exeter", "Cambridge", "Hastings", "Canterbury", "Banbury", "Newcastle",
];
#[rustfmt::skip]
pub static MONGOL: [&str; 16] = [
    "Samarkand", "Bokhara", "Nishapur", "Karakorum", "Kashgar", "Tabriz", "Aleppo", "Kabul",
    "Ormuz", "Basra", "Khanbalyk", "Khorasan", "Shangtu", "Kazan", "Quinsay", "Kerman",
];

/// Spare pool of 32 names used once a civilization's own list is exhausted
/// (and by the Barbarians / unused civ slot).
#[rustfmt::skip]
pub static SPARE_CITY_NAMES: [&str; 32] = [
    "Mecca", "Naples", "Sidon", "Tyre", "Tarsus", "Issus", "Cunaxa", "Cremona",
    "Cannae", "Capua", "Turin", "Genoa", "Utica", "Crete", "Damascus", "Verona",
    "Salamis", "Lisbon", "Hamburg", "Prague", "Salzburg", "Bergen", "Venice", "Milan",
    "Ghent", "Pisa", "Cordoba", "Seville", "Dublin", "Toronto", "Melbourne", "Sydney",
];

/// First half of the spare pool (assigned to civ 0, the Barbarians).
#[rustfmt::skip]
pub static SPARE_FIRST: [&str; 16] = [
    "Mecca", "Naples", "Sidon", "Tyre", "Tarsus", "Issus", "Cunaxa", "Cremona",
    "Cannae", "Capua", "Turin", "Genoa", "Utica", "Crete", "Damascus", "Verona",
];
/// Second half of the spare pool (assigned to the unused civ slot 8).
#[rustfmt::skip]
pub static SPARE_SECOND: [&str; 16] = [
    "Salamis", "Lisbon", "Hamburg", "Prague", "Salzburg", "Bergen", "Venice", "Milan",
    "Ghent", "Pisa", "Cordoba", "Seville", "Dublin", "Toronto", "Melbourne", "Sydney",
];
