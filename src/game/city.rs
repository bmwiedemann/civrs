use super::map::{CityId, Map, Slot, UnitId};
use super::{Game, UnitOrder};
use crate::rules::{ImprovementId, UnitTypeId, WonderId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Production {
    Unit(UnitTypeId),
    Improvement(ImprovementId),
    Wonder(WonderId),
}

impl Production {
    pub fn name(&self) -> &'static str {
        match self {
            Production::Unit(u) => crate::rules::UNITS[*u as usize].name,
            Production::Improvement(i) => crate::rules::IMPROVEMENTS[*i as usize].name,
            Production::Wonder(w) => crate::rules::WONDERS[*w as usize].name,
        }
    }
    /// Cost in shields.
    pub fn cost(&self) -> i32 {
        10 * match self {
            Production::Unit(u) => crate::rules::UNITS[*u as usize].cost as i32,
            Production::Improvement(i) => crate::rules::IMPROVEMENTS[*i as usize].cost as i32,
            Production::Wonder(w) => crate::rules::WONDERS[*w as usize].cost as i32,
        }
    }
    pub fn class(&self) -> u8 {
        match self {
            Production::Unit(_) => 0,
            Production::Improvement(_) => 1,
            Production::Wonder(_) => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Specialist {
    Entertainer,
    Taxman,
    Scientist,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct City {
    pub id: CityId,
    pub owner: Slot,
    /// Original founder (for score / civil war).
    pub founder: Slot,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub size: u8,
    /// Food stored.
    pub food: i32,
    /// Shields accumulated toward current production.
    pub shields: i32,
    pub production: Production,
    /// Bit per ImprovementId (1..=24).
    pub improvements: u32,
    /// Bit per WonderId (1..=21).
    pub wonders: u32,
    /// Bit per CITY_RADIUS index (1..=20) of worked tiles. Bit 0 (`CENTRE_IDLE`) is set when
    /// the centre is not worked: CIV.EXE drops it from the worked mask (bit 20) while a foreign
    /// unit stands on the city square (0xd454..0xd5b1) and its citizen works another square.
    pub worked: u32,
    pub specialists: Vec<Specialist>,
    /// The three route slots of the record (+0x17..+0x19), in slot order; `NO_ROUTE` marks a
    /// free slot between used ones (a destroyed partner, destroy_city 0xa83a), trailing free
    /// slots are left out. Read them with [`City::routes`].
    pub trade_routes: Vec<CityId>,
    pub disorder: bool,
    pub disorder_turns: u8,
    pub celebrating: bool,
    /// Something was bought this turn. Informational only: CIV.EXE allows further purchases and
    /// changes after a purchase (0x1176c, 0x117e8).
    pub bought: bool,
    /// A Diplomat stole an advance here; no other can (CIV.EXE city status bit 0x20).
    pub tech_stolen: bool,
    /// CIV.EXE city status bit 0x10 (+6). For the human it is the city screen's AUTO toggle
    /// ('A', 0x113d7; CHANGE clears it, 0x117ae). A computer city gets it whenever it is
    /// attacked (0x6141, any attacker, win or lose); its next turn then re-chooses production
    /// and may buy a unit although units are inside, and clears it again (0xec27..0xef74).
    /// A capture clears it (0x13ef0).
    pub auto_build: bool,
    pub founded_turn: u32,
    /// Accumulated pollution chance bookkeeping.
    pub pollution_pending: u8,
    /// Shields contributed by caravans towards a wonder.
    pub caravan_shields: i32,
    /// CIV.EXE city record +0x1a/+0x1b (ds:0x7106/0x7107): fortified units of a human city
    /// taken off the unit table, each its type | `GARRISON_VETERAN`, `NO_GARRISON` when free.
    /// See [`Game::stash_garrison`]. Saved after the game body (save version 6).
    #[serde(skip, default = "no_garrison")]
    pub garrison: [u8; 2],
    /// CIV.EXE city status bit 0x4 (+6): the garrison was let out (0x7f17); the city stashes
    /// it again at the end of its turn or of the city screen (0x131d2). Saved with `garrison`.
    #[serde(skip)]
    pub garrison_out: bool,
    /// CIV.EXE city record +8 (ds:0x70f4): the size the human last saw, 0 while he has not
    /// seen the city. His map shows a foreign city only when it is not 0, and with this size
    /// (0x1b2ad, 0x1b3a3; the minimap 0x1a97c, the World Map o 0x10348). See
    /// [`Game::city_size_shown`]. Saved after the game body (save version 19).
    #[serde(skip)]
    pub known_size: u8,
    /// CIV.EXE city status bit 0x2 (+6), set once at founding (o 0x1b687..0x1b6b9) when one of
    /// the eight squares around is Ocean of a body larger than 256 squares, or any Ocean for
    /// the human's city; a capture keeps it. Only this bit lets a city build sea units
    /// (choose_production 0xaab2, for every civ) and makes it a port for the computer's
    /// transports (0x16f8d). See [`Game::founding_coastal`]. Saved after the game body (save
    /// version 21).
    #[serde(skip)]
    pub coastal: bool,
    /// CIV.EXE city status bit 0x8 (+6), set once at founding (o 0x1b6be..0x1b6d2) when one of
    /// the eight squares around is River or Mountains; a capture keeps it. Only this bit lets
    /// the city build a Hydro Plant (0xad64). See [`Game::founding_hydro_site`]. Saved after
    /// the game body (save version 22).
    #[serde(skip)]
    pub hydro_site: bool,
    /// The city's entry in the 256-name table (CIV.EXE city +0x16), given at founding and kept
    /// by a rename, which CIV.EXE writes over that entry: Find City orders cities by it (o
    /// 0x1ecf4). `NO_NAME_INDEX` for a name from outside the table. Saved after the game body
    /// (save version 23).
    #[serde(skip)]
    pub name_index: u8,
    /// CIV.EXE city record +0xa (ds:0x70f6): the trade of the worked squares less its
    /// corruption, as the city routine last counted it (`process_city` 0xf13d, on every call:
    /// the city's turn, its screen, the advisor's mode -1 and the reports), 0 at founding (o
    /// 0x1b570, 0x1b62a). Trade routes (0xf195), a caravan's revenue (0x14712), the route
    /// weights (0x148af), the computer's caravan target (0xe0df) and the city screen's route
    /// list (0x10f81) read this, not the city as it is now. See [`Game::note_trade`]. Saved
    /// after the game body (save version 27).
    #[serde(skip)]
    pub trade: i32,
}

/// `City::worked` bit 0: the centre is not worked (CIV.EXE's worked bit 20 clear).
pub const CENTRE_IDLE: u32 = 1;
/// `City::name_index` of a city whose name is not in the name table.
pub const NO_NAME_INDEX: u8 = 0xff;
/// A free trade route slot (0xff in the record).
pub const NO_ROUTE: CityId = 0xff;

/// The name-table entry of `name` (the first one that has it), or `NO_NAME_INDEX`.
pub fn name_table_index(name: &str) -> u8 {
    crate::rules::city_names::name_table().iter().position(|t| *t == name).map_or(NO_NAME_INDEX, |i| i as u8)
}

/// A free garrison slot (CIV.EXE writes 0xff at city founding, o 0x1b5cd).
pub const NO_GARRISON: u8 = 0xff;
/// The stashed unit was a veteran (CIV.EXE 0x7f8f).
pub const GARRISON_VETERAN: u8 = 0x40;

pub fn no_garrison() -> [u8; 2] {
    [NO_GARRISON; 2]
}

impl City {
    pub fn pos(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    /// The trade partners in slot order, free slots skipped (as 0xf141 and 0x10f28 go over
    /// them).
    pub fn routes(&self) -> impl Iterator<Item = CityId> + '_ {
        self.trade_routes.iter().copied().filter(|r| *r != NO_ROUTE)
    }

    /// Route slot `i` (0..3), `None` when free.
    pub fn route_slot(&self, i: usize) -> Option<CityId> {
        self.trade_routes.get(i).copied().filter(|r| *r != NO_ROUTE)
    }

    /// Store `r` (or free the slot with `None`) in route slot `i`, keeping the trailing free
    /// slots out of the list.
    pub fn set_route_slot(&mut self, i: usize, r: Option<CityId>) {
        while self.trade_routes.len() <= i {
            self.trade_routes.push(NO_ROUTE);
        }
        self.trade_routes[i] = r.unwrap_or(NO_ROUTE);
        while self.trade_routes.last() == Some(&NO_ROUTE) {
            self.trade_routes.pop();
        }
    }
    pub fn has_improvement(&self, id: ImprovementId) -> bool {
        self.improvements & (1u32 << id) != 0
    }
    pub fn add_improvement(&mut self, id: ImprovementId) {
        self.improvements |= 1u32 << id;
    }
    pub fn remove_improvement(&mut self, id: ImprovementId) {
        self.improvements &= !(1u32 << id);
    }
    pub fn has_wonder(&self, id: WonderId) -> bool {
        self.wonders & (1u32 << id) != 0
    }
    pub fn add_wonder(&mut self, id: WonderId) {
        self.wonders |= 1u32 << id;
    }
    pub fn works(&self, radius_idx: u8) -> bool {
        (radius_idx == 0) != (self.worked & (1u32 << radius_idx) != 0)
    }
    pub fn set_worked(&mut self, radius_idx: u8, on: bool) {
        if radius_idx == 0 {
            if on {
                self.worked &= !CENTRE_IDLE;
            } else {
                self.worked |= CENTRE_IDLE;
            }
        } else if on {
            self.worked |= 1u32 << radius_idx;
        } else {
            self.worked &= !(1u32 << radius_idx);
        }
    }
    /// Worked squares besides the centre.
    pub fn workers(&self) -> u32 {
        (self.worked & !CENTRE_IDLE).count_ones()
    }
    pub fn improvement_list(&self) -> Vec<ImprovementId> {
        (1..=24u8).filter(|i| self.has_improvement(*i)).collect()
    }
    pub fn wonder_list(&self) -> Vec<WonderId> {
        (1..=21u8).filter(|i| self.has_wonder(*i)).collect()
    }
    /// Food needed to grow.
    /// Population display figure (Civ1: sum of 10000 * i for i in 1..=size).
    pub fn population(&self) -> i64 {
        let n = self.size as i64;
        n * (n + 1) / 2 * 10_000
    }
    /// The stashed garrison units: (unit type, veteran).
    pub fn garrison_units(&self) -> impl Iterator<Item = (UnitTypeId, bool)> + '_ {
        self.garrison.iter().filter(|g| **g != NO_GARRISON).map(|g| (g & 0x3f, g & GARRISON_VETERAN != 0))
    }
    pub fn garrison_count(&self) -> i32 {
        self.garrison_units().count() as i32
    }
}

/// The garrison store. CIV.EXE keeps at most 128 units per civilization and saves slots by
/// taking fortified defenders of the human's cities off the unit table: when a unit finishes
/// fortifying in its home city (0x431f), and again at the end of the city's turn or of its city
/// screen (0x1320d), up to two of the city's fortified home units are written into the city
/// record and deleted; the first such unit always stays (0x7ff2). They come back, fortified,
/// as soon as a unit of another civilization stands next to the city (unit_look 0x828f) or the
/// city screen is drawn (process_city mode 1, 0xcda0). Meanwhile they still count: shield
/// support (0xd2ae, 0xfc18), martial law (0xf92d), the production advisor (0xb056), the AI
/// census (0x15aa7) and the map's "city has units" mark (0x1b433). civrs has no unit limit
/// but keeps the store for the rules that see it differently from units on the map.
impl Game {
    /// CIV.EXE 0x7e6:0xc2 `city_stash_garrison`: a human city's fortified units homed there and
    /// standing on it go into the free garrison slots, all but the first found (unit table
    /// order); clears `garrison_out`.
    pub fn stash_garrison(&mut self, c: CityId) {
        let city = self.city(c);
        if city.owner != self.human_slot {
            return;
        }
        let (owner, pos) = (city.owner, city.pos());
        // (the owner's unit table, in record order, 0x7e6:0xc2)
        let mut fortified: Vec<(u8, UnitId)> = self
            .all_units()
            .filter(|u| u.owner == owner && u.pos() == pos && u.home == Some(c) && u.order == UnitOrder::Fortified)
            .map(|u| (u.record, u.id))
            .collect();
        fortified.sort_unstable();
        for (_, id) in fortified.into_iter().skip(1) {
            let Some(slot) = self.city(c).garrison.iter().position(|g| *g == NO_GARRISON) else { break };
            let u = self.unit(id);
            let code = u.kind | if u.veteran { GARRISON_VETERAN } else { 0 };
            self.city_mut(c).garrison[slot] = code;
            self.remove_unit(id);
        }
        self.city_mut(c).garrison_out = false;
    }

    /// CIV.EXE 0x7e6:0x2 `city_unstash_garrison`: the stashed units of a human city become
    /// units again, fortified, homed there (create_unit 0x7e6:0xcf1 homes a unit to the city
    /// it is made in); sets `garrison_out`. CIV.EXE leaves their moves at whatever the reused
    /// unit slot held; civrs gives full moves.
    pub fn unstash_garrison(&mut self, c: CityId) {
        let city = self.city(c);
        if city.owner != self.human_slot {
            return;
        }
        let (owner, (x, y)) = (city.owner, city.pos());
        let mut kept = no_garrison();
        for (i, code) in city.garrison.into_iter().enumerate().filter(|(_, g)| *g != NO_GARRISON) {
            let (kind, veteran) = (code & 0x3f, code & GARRISON_VETERAN != 0);
            // a unit with no free unit slot stays stashed (0x7ec6)
            let Some(id) = self.try_add_unit(owner, kind, x, y) else {
                kept[i] = code;
                continue;
            };
            // a unit let out is no new one
            self.players[owner as usize].stats.units_built -= 1;
            let u = self.unit_mut(id);
            u.order = UnitOrder::Fortified;
            u.veteran = veteran;
            u.home = Some(c);
        }
        let cm = self.city_mut(c);
        cm.garrison = kept;
        cm.garrison_out = true;
    }

    /// Unit_look's city check (CIV.EXE 0x8241..0x8292): a unit of `slot` at (x, y) lets the
    /// garrison out of every city of another civilization next to it, an empty store too:
    /// `city_unstash_garrison` sets the let-out bit whatever it found (0x7f17), so the human
    /// city's fortified home units go into the store at the end of its turn (0x131da).
    pub fn reveal_garrisons_near(&mut self, slot: Slot, x: i32, y: i32) {
        for (nx, ny) in Map::neighbours8(x, y) {
            if let Some(c) = self.city_at(nx, ny) {
                if self.city(c).owner != slot {
                    self.unstash_garrison(c);
                }
            }
        }
    }

    /// The size the human's map shows for city `c`: the size of his own city, the size he
    /// last saw of a foreign one, nothing before he has seen it (CIV.EXE 0x1b2ad..0x1b3a3).
    pub fn city_size_shown(&self, c: CityId) -> Option<u8> {
        let city = self.city_opt(c)?;
        if city.owner == self.human_slot {
            Some(city.size)
        } else {
            (city.known_size != 0).then_some(city.known_size)
        }
    }

    /// The human sees city `c` as it is now (`+8 = +7`, CIV.EXE 0x82a9, 0x6219, 0x96e1).
    pub fn note_city_size(&mut self, c: CityId) {
        if let Some(city) = self.cities.get_mut(c as usize).and_then(|c| c.as_mut()) {
            city.known_size = city.size;
        }
    }

    /// A city changes hands (capture_city 0x13d76, 0x14144): when the human takes it, he knows
    /// it as his own (0); when he loses it, as it is now. Between computer players the old
    /// knowledge stays.
    pub fn city_changed_hands(&mut self, c: CityId, from: Slot, to: Slot) {
        let h = self.human_slot;
        if to == h {
            self.city_mut(c).known_size = 0;
        } else if from == h {
            self.note_city_size(c);
        }
        // and he sees the square's improvements (0x1402a)
        if to == h || from == h {
            let (x, y) = self.city(c).pos();
            self.map.update_seen(x, y);
        }
        // the new owner's number goes on the city radius of the site/owner layer (0x13fbc).
        // CIV.EXE stops at index 19 (`< 0x14`, 0x13fff) and leaves (-1, -2) out, unlike
        // destroy_city's `<= 0x14` (0xa929); civrs takes all 21 squares (but for `exe_quirks`).
        let (x, y) = self.city(c).pos();
        let n = if self.exe_quirks { 20 } else { 21 };
        for (dx, dy) in &super::barbarians::CLAIM[..n] {
            self.map.set_claim(Map::wrap_x(x + dx), y + dy, to);
        }
    }

    /// CIV.EXE 0x7e6:0x18cc `foreign_unit_adjacent(civ, x, y)`: a unit of another civilization
    /// on one of the 8 squares around (x, y).
    pub fn foreign_unit_adjacent(&self, slot: Slot, x: i32, y: i32) -> bool {
        Map::neighbours8(x, y).any(|(nx, ny)| self.all_units().any(|u| u.pos() == (nx, ny) && u.owner != slot))
    }

    /// The end of a city's turn or of its city screen (CIV.EXE 0x131d2): a garrison let out goes
    /// back into the store unless a foreign unit is next to the city. CIV.EXE calls
    /// `foreign_unit_adjacent` here (and at 0x430d) with only (x, y), so the check reads the
    /// wrong squares; civrs checks what was meant.
    pub fn restash_garrison(&mut self, c: CityId) {
        let Some(city) = self.city_opt(c) else { return };
        if city.garrison_out && !self.foreign_unit_adjacent(city.owner, city.x, city.y) {
            self.stash_garrison(c);
        }
    }

    /// `restash_garrison` at the end of a city's turn: with `exe_quirks` the check as CIV.EXE
    /// makes it there (`exe_garrison_check`).
    pub fn restash_garrison_turn(&mut self, c: CityId) {
        if !self.exe_quirks {
            return self.restash_garrison(c);
        }
        let Some(city) = self.city_opt(c) else { return };
        if city.garrison_out && !self.exe_garrison_check(c) {
            self.stash_garrison(c);
        }
    }

    /// `restash_garrison` at the end of a quiet run of the city routine (0x131d2): with
    /// `exe_quirks` and the caller's si known (`Game::quiet_si`) the check as CIV.EXE makes
    /// it (`exe_garrison_check`).
    pub fn restash_garrison_quiet(&mut self, c: CityId) {
        let si = match self.quiet_si {
            Some(super::QuietSi::Index) if self.exe_quirks => 0x1c * c as i32,
            Some(super::QuietSi::Fixed(v)) if self.exe_quirks => v,
            _ => return self.restash_garrison(c),
        };
        let Some(city) = self.city_opt(c) else { return };
        if city.garrison_out && !self.exe_foreign_unit_adjacent(city.x, city.y, si) {
            self.stash_garrison(c);
        }
    }

    /// What `foreign_unit_adjacent` answers at the end of a city's turn (0x131fa): it gets
    /// the city's x as the civ, its y as x, and as y the word above them on the stack,
    /// process_city's saved si, which the city loop (0xa613) left at 0x1c * the city's
    /// index. So it looks at the squares around (y, 28 * index) of map page 2's owner band
    /// (x + 160) for a mark (bit 3) of another "civ" than x: rows up to 49 hold the units' owners,
    /// 50..199 picture data (`exe_page_band`), and rows from 200 on lie past the graphics
    /// driver's row table (cs:0x17d6), in memory that a probe of CIV.EXE showed answering
    /// "adjacent" (Carthage, index 12, row 336).
    pub fn exe_garrison_check(&self, c: CityId) -> bool {
        let city = self.city(c);
        self.exe_foreign_unit_adjacent(city.x, city.y, 0x1c * c as i32)
    }

    /// `foreign_unit_adjacent(civ, x, y)` (0x7e6:0x18cc) as CIV.EXE answers it for any
    /// arguments, on map page 2's owner band (see `exe_garrison_check`).
    fn exe_foreign_unit_adjacent(&self, civ: i32, x0: i32, y0: i32) -> bool {
        for (dx, dy) in [(0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1)] {
            let (x, y) = (Map::wrap_x(x0 + dx), y0 + dy);
            // (row -1: the word before the table, cs:0x17d4, is 0, so row 0)
            let y = y.max(0);
            let owner = if y < 50 {
                self.unit_owner_at(x, y).map(|o| o as i32)
            } else if y < 200 {
                let v = self.exe_page_band.get(((y - 50) * 80 + x) as usize).copied().unwrap_or(0);
                (v & 8 != 0).then_some((v & 7) as i32)
            } else {
                return true;
            };
            if owner.is_some_and(|o| o != civ) {
                return true;
            }
        }
        false
    }

    /// What `foreign_unit_adjacent` answers when a unit fortified in its home city decides
    /// whether the garrison goes back into the store (0x430d, `complete_fortify`): it gets the
    /// unit's x as the civ, its y as x, and as y the word above them, move_units's saved si,
    /// which game_turn left at 2 * civ for the human (0x2572) and at 0xb1d6 + 2 * civ, the
    /// civ's gold, for the others (0x2515). So the human's check looks at rows 1..3 around
    /// column y; for a computer's (rows from 0xb1d5) probes of CIV.EXE answered "not
    /// adjacent" every time (civs 0, 3, 6 and 7).
    pub fn exe_fortify_garrison_check(&self, slot: Slot, x: i32, y: i32) -> bool {
        slot == self.human_slot && self.exe_foreign_unit_adjacent(x, y, 2 * slot as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{GameSetup, MoveOutcome};
    use crate::rules::{unit_ids, Govt, Terrain};

    /// A size-3 city of the human at (20, 20) on cleared grassland and three Phalanxes homed
    /// there that are fortifying in it, the second a veteran.
    fn garrisoned() -> (Game, CityId, Vec<UnitId>) {
        let mut g = Game::new(GameSetup { seed: 7, ..Default::default() }, None);
        for y in 10..30 {
            for x in 10..30 {
                let t = g.map.get_mut(x, y).unwrap();
                t.terrain = Terrain::Grassland;
                t.flags = 0;
                t.special = false;
            }
        }
        let near: Vec<UnitId> = g.all_units().filter(|u| (10..30).contains(&u.x) && (10..30).contains(&u.y)).map(|u| u.id).collect();
        for id in near {
            g.remove_unit(id);
        }
        let h = g.human_slot;
        let c = g.add_city(h, "Fort".into(), 20, 20);
        g.city_mut(c).size = 3;
        g.auto_assign(c);
        let ids: Vec<UnitId> = (0..3)
            .map(|i| {
                let id = g.add_unit(h, unit_ids::PHALANX, 20, 20);
                let u = g.unit_mut(id);
                u.home = Some(c);
                u.order = UnitOrder::Fortify;
                u.veteran = i == 1;
                id
            })
            .collect();
        (g, c, ids)
    }

    fn rival(g: &Game) -> Slot {
        (1..8).find(|s| *s != g.human_slot && g.player(*s).in_game).unwrap()
    }

    #[test]
    fn exe_quirks_the_fortified_unit_s_check_reads_rows_by_its_civ() {
        // 0x430d: (civ x, y, 2 * human) for the human, rows from 0xb1d5 for the others
        let (mut g, c, _) = garrisoned();
        let (x, y) = g.city(c).pos();
        let me = g.human_slot;
        let r = rival(&g);
        assert!(!g.exe_fortify_garrison_check(me, x, y));
        g.add_unit(r, unit_ids::PHALANX, y + 1, 2 * me as i32 + 1);
        assert!(g.exe_fortify_garrison_check(me, x, y), "a unit next to ({y}, {})", 2 * me);
        g.add_unit(me, unit_ids::PHALANX, x + 1, y);
        assert!(!g.exe_fortify_garrison_check(r, x + 1, y), "a computer's never sees one");
    }

    /// A quiet run of the city routine ends at the garrison check (0x125d6 -> 0x131d2): the
    /// garrison let out goes back unless a foreign unit is next to the city.
    #[test]
    fn a_quiet_run_stashes_the_garrison_again() {
        for foreign in [false, true] {
            let (mut g, c, ids) = garrisoned();
            for id in &ids {
                g.unit_mut(*id).order = UnitOrder::Fortified;
            }
            g.city_mut(c).garrison_out = true;
            if foreign {
                let r = rival(&g);
                g.add_unit(r, unit_ids::PHALANX, 21, 20);
            }
            g.city_routine_quiet(c);
            assert_eq!(g.city(c).garrison_count(), if foreign { 0 } else { 2 }, "foreign {foreign}");
        }
    }

    /// With `exe_quirks` a quiet run's garrison check takes the caller's si for y
    /// (`Game::quiet_si`): the unrest's first run has 16 (o 0x1bc64), a far one ("adjacent")
    /// keeps the garrison out; an unknown caller checks as meant.
    #[test]
    fn exe_quirks_a_quiet_run_s_garrison_check_reads_the_caller_s_si() {
        for (si, kept) in [(None, false), (Some(crate::game::QuietSi::Fixed(16)), false), (Some(crate::game::QuietSi::Fixed(0x1c * 20)), true)] {
            let (mut g, c, ids) = garrisoned();
            g.exe_quirks = true;
            for id in &ids {
                g.unit_mut(*id).order = UnitOrder::Fortified;
            }
            g.city_mut(c).garrison_out = true;
            g.quiet_si = si;
            g.city_routine_quiet(c);
            assert_eq!(g.city(c).garrison_count(), if kept { 0 } else { 2 }, "si {si:?}");
        }
    }

    /// The city screen's CHANGE (0x117ef) runs the city routine with process_city's si, 2 * the
    /// unit type in production (0x117d7), 0x1c * the city's index for an improvement (0x111d4):
    /// with `exe_quirks` its garrison check looks around (city y, si) and a unit there keeps
    /// the garrison out.
    #[test]
    fn exe_quirks_change_s_garrison_check_reads_the_screen_s_si() {
        for far in [false, true] {
            let (mut g, c, ids) = garrisoned();
            g.exe_quirks = true;
            for id in &ids {
                g.unit_mut(*id).order = UnitOrder::Fortified;
            }
            g.city_mut(c).production = Production::Unit(unit_ids::PHALANX);
            g.city_mut(c).garrison_out = true;
            let y = g.city(c).y;
            if far {
                let r = rival(&g);
                g.add_unit(r, unit_ids::PHALANX, y + 1, 2 * unit_ids::PHALANX as i32 + 1);
            }
            let _ = g.change_advice(c);
            assert_eq!(g.city(c).garrison_count(), if far { 0 } else { 2 }, "far {far}");
            assert_eq!(g.quiet_si, None);
        }
        // an improvement: the screen's last drawing left 0x1c * the city's index (0x111d4)
        for far in [false, true] {
            let (mut g, c, ids) = garrisoned();
            g.exe_quirks = true;
            for id in &ids {
                g.unit_mut(*id).order = UnitOrder::Fortified;
            }
            g.city_mut(c).production = Production::Improvement(crate::rules::improvement_ids::BARRACKS);
            g.city_mut(c).garrison_out = true;
            let y = g.city(c).y;
            let row = 0x1c * c as i32;
            assert!(row + 1 < 50, "index {c}");
            if far {
                let r = rival(&g);
                g.add_unit(r, unit_ids::PHALANX, y - 1, row + 1);
            }
            let _ = g.change_advice(c);
            assert_eq!(g.city(c).garrison_count(), if far { 0 } else { 2 }, "improvement, far {far}");
        }
    }

    /// CIV.EXE's end-of-turn garrison check (0x131fa) reads the owner band around (city y,
    /// 28 * index): index 1 looks at the squares around (y, 28), a high index past the
    /// driver's row table counts as "adjacent" (`exe_quirks`).
    #[test]
    fn exe_quirks_the_garrison_check_reads_around_y_and_28_times_the_index() {
        let (mut g, c, _) = garrisoned();
        let (x, y) = g.city(c).pos();
        let ids: Vec<CityId> = g.all_cities().map(|o| o.id).collect();
        for o in ids {
            if o != c {
                g.cities[o as usize] = None;
            }
        }
        // move the city to slot 1
        let city = g.cities[c as usize].take();
        while g.cities.len() < 2 {
            g.cities.push(None);
        }
        g.cities[1] = city;
        g.city_mut(1).id = 1;
        for t in g.map.tiles.iter_mut() {
            if t.city == Some(c) {
                t.city = Some(1);
            }
        }
        assert!(!g.exe_garrison_check(1));
        let r = rival(&g);
        g.add_unit(r, unit_ids::PHALANX, y + 1, 28);
        assert!(g.exe_garrison_check(1), "a unit next to ({y}, 28)");
        let _ = x;
        g.cities.resize(13, None);
        g.cities[12] = g.cities[1].take();
        g.city_mut(12).id = 12;
        assert!(g.exe_garrison_check(12), "row 336");
    }

    /// The stash takes the fortified units in the unit table's order (records), whatever
    /// order civrs made them in; the first stays.
    #[test]
    fn the_stash_goes_by_record() {
        let (mut g, c, ids) = garrisoned();
        for (id, rec) in ids.iter().zip([5u8, 1, 3]) {
            g.unit_mut(*id).order = UnitOrder::Fortified;
            g.unit_mut(*id).record = rec;
        }
        g.stash_garrison(c);
        // records 1 (ids[1]) stays, 3 and 5 go
        assert!(g.unit_opt(ids[1]).is_some());
        assert!(g.unit_opt(ids[0]).is_none() && g.unit_opt(ids[2]).is_none());
    }

    #[test]
    fn fortified_home_units_but_the_first_go_into_the_record() {
        let (mut g, c, ids) = garrisoned();
        let h = g.human_slot;
        g.run_human_loop();
        assert_eq!(g.unit(ids[0]).order, UnitOrder::Fortified, "the first stays");
        assert!(g.unit_opt(ids[1]).is_none() && g.unit_opt(ids[2]).is_none());
        assert_eq!(g.city(c).garrison, [unit_ids::PHALANX | GARRISON_VETERAN, unit_ids::PHALANX]);
        assert!(!g.city(c).garrison_out);
        // they still keep order and defend the continent in the census
        assert_eq!(g.martial_law_units(c), 3);
        let k = crate::game::production::continent_index(g.map.tile(20, 20).continent);
        let on_map: i32 = g.all_units().filter(|u| u.owner == h && g.map.tile(u.x, u.y).is_land() && crate::game::production::continent_index(g.map.tile(u.x, u.y).continent) == k).map(|u| u.info().def as i32).sum();
        assert_eq!(g.live_census().defence[h as usize][k], on_map + 2 * 2);
        // a computer city never stashes
        let ai = rival(&g);
        let other = g.add_city(ai, "Other".into(), 26, 26);
        for _ in 0..3 {
            let id = g.add_unit(ai, unit_ids::PHALANX, 26, 26);
            g.unit_mut(id).home = Some(other);
            g.unit_mut(id).order = UnitOrder::Fortified;
        }
        g.stash_garrison(other);
        assert_eq!(g.city(other).garrison, no_garrison());
    }

    #[test]
    fn a_foreign_unit_next_to_the_city_lets_the_garrison_out_until_the_city_turn() {
        let (mut g, c, _) = garrisoned();
        g.run_human_loop();
        let scout = g.add_unit(rival(&g), unit_ids::MILITIA, 22, 20);
        assert_eq!(g.try_move(scout, crate::game::Dir8::W).unwrap(), MoveOutcome::Moved);
        assert_eq!(g.city(c).garrison, no_garrison());
        assert!(g.city(c).garrison_out);
        let inside: Vec<UnitId> = g.units_at(20, 20);
        assert_eq!(inside.len(), 3);
        assert!(inside.iter().all(|id| g.unit(*id).order == UnitOrder::Fortified && g.unit(*id).home == Some(c)));
        assert_eq!(inside.iter().filter(|id| g.unit(**id).veteran).count(), 1);
        assert_eq!(g.martial_law_units(c), 3);
        // the city's turn puts them back only once nobody foreign is next to it
        g.process_city(c);
        assert!(g.city(c).garrison_out);
        assert_eq!(g.units_at(20, 20).len(), 3);
        g.remove_unit(scout);
        g.process_city(c);
        assert!(!g.city(c).garrison_out);
        assert_eq!(g.units_at(20, 20).len(), 1);
        assert_eq!(g.city(c).garrison_count(), 2);
    }

    /// A foreign unit next to a human city with an empty store still sets the let-out bit
    /// (0x7f17), so the city's turn stashes fortified home units that never fortified there
    /// (0x131da -> 0x1320d).
    #[test]
    fn a_foreign_unit_next_to_a_city_with_an_empty_store_sets_the_let_out_bit() {
        let (mut g, c, ids) = garrisoned();
        for id in &ids {
            g.unit_mut(*id).order = UnitOrder::Fortified;
        }
        assert!(!g.city(c).garrison_out);
        let scout = g.add_unit(rival(&g), unit_ids::MILITIA, 22, 20);
        assert_eq!(g.try_move(scout, crate::game::Dir8::W).unwrap(), MoveOutcome::Moved);
        assert!(g.city(c).garrison_out);
        assert_eq!(g.units_at(20, 20).len(), 3);
        g.remove_unit(scout);
        g.process_city(c);
        assert!(!g.city(c).garrison_out);
        assert_eq!(g.units_at(20, 20).len(), 1);
        assert_eq!(g.city(c).garrison_count(), 2);
    }

    #[test]
    fn stashed_units_are_free_up_to_the_city_size_under_any_government() {
        let (mut g, c, _) = garrisoned();
        let h = g.human_slot;
        g.run_human_loop();
        g.player_mut(h).govt = Govt::Monarchy;
        // the unit on the map pays, the two in the record do not (size 3)
        assert_eq!(g.city_yield(c).shield_upkeep, 1);
        g.city_mut(c).size = 1;
        assert_eq!(g.garrison_upkeep(c), 1);
        assert_eq!(g.city_yield(c).shield_upkeep, 2);
        // under Despotism they use up the free quota first
        g.player_mut(h).govt = Govt::Despotism;
        g.city_mut(c).size = 2;
        assert_eq!(g.shield_costs(c), vec![(g.units_at(20, 20)[0], true)]);
    }

    #[test]
    fn a_computer_unit_fortifying_in_a_big_city_is_rehomed_there() {
        let (mut g, _, _) = garrisoned();
        let ai = rival(&g);
        let big = g.add_city(ai, "Big".into(), 14, 14);
        let small = g.add_city(ai, "Small".into(), 26, 14);
        g.city_mut(big).size = 3;
        let a = g.add_unit(ai, unit_ids::PHALANX, 14, 14);
        let b = g.add_unit(ai, unit_ids::PHALANX, 26, 14);
        for id in [a, b] {
            g.unit_mut(id).order = UnitOrder::Fortify;
            g.unit_mut(id).home = None;
        }
        // when the computer's unit loop comes to them (0x4239)
        crate::ai::unit_ai::move_units(&mut g, ai, &mut |_| {});
        assert_eq!((g.unit(a).order, g.unit(a).moves_left), (UnitOrder::Fortified, 0));
        assert_eq!(g.unit(a).home, Some(big));
        assert_eq!(g.unit(b).home, None);
        let _ = small;
    }
}
