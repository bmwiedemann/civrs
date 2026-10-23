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
    /// Bit per CITY_RADIUS index (1..=20) of worked tiles (index 0, the centre, is implicit).
    pub worked: u32,
    pub specialists: Vec<Specialist>,
    pub trade_routes: Vec<CityId>,
    pub disorder: bool,
    pub disorder_turns: u8,
    pub celebrating: bool,
    /// Something was bought this turn (no further changes allowed).
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
        radius_idx == 0 || self.worked & (1u32 << radius_idx) != 0
    }
    pub fn set_worked(&mut self, radius_idx: u8, on: bool) {
        if radius_idx == 0 {
            return;
        }
        if on {
            self.worked |= 1u32 << radius_idx;
        } else {
            self.worked &= !(1u32 << radius_idx);
        }
    }
    pub fn workers(&self) -> u32 {
        self.worked.count_ones()
    }
    pub fn improvement_list(&self) -> Vec<ImprovementId> {
        (1..=24u8).filter(|i| self.has_improvement(*i)).collect()
    }
    pub fn wonder_list(&self) -> Vec<WonderId> {
        (1..=21u8).filter(|i| self.has_wonder(*i)).collect()
    }
    /// Food needed to grow.
    pub fn granary_size(&self) -> i32 {
        (self.size as i32 + 1) * 10
    }
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
        let fortified: Vec<UnitId> = self
            .all_units()
            .filter(|u| u.owner == owner && u.pos() == pos && u.home == Some(c) && u.order == UnitOrder::Fortified && u.carried_by.is_none())
            .map(|u| u.id)
            .collect();
        for id in fortified.into_iter().skip(1) {
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
        for (kind, veteran) in city.garrison_units().collect::<Vec<_>>() {
            let id = self.add_unit(owner, kind, x, y);
            // a unit let out is no new one
            self.players[owner as usize].stats.units_built -= 1;
            let u = self.unit_mut(id);
            u.order = UnitOrder::Fortified;
            u.veteran = veteran;
            u.home = Some(c);
        }
        let cm = self.city_mut(c);
        cm.garrison = no_garrison();
        cm.garrison_out = true;
    }

    /// Unit_look's city check (CIV.EXE 0x8241..0x8292): a unit of `slot` at (x, y) lets the
    /// garrison out of every city of another civilization next to it.
    pub fn reveal_garrisons_near(&mut self, slot: Slot, x: i32, y: i32) {
        for (nx, ny) in Map::neighbours8(x, y) {
            if let Some(c) = self.city_at(nx, ny) {
                if self.city(c).owner != slot && self.city(c).garrison_count() > 0 {
                    self.unstash_garrison(c);
                }
            }
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
    fn fortified_home_units_but_the_first_go_into_the_record() {
        let (mut g, c, ids) = garrisoned();
        let h = g.human_slot;
        g.reset_moves(h);
        assert_eq!(g.unit(ids[0]).order, UnitOrder::Fortified, "the first stays");
        assert!(g.unit_opt(ids[1]).is_none() && g.unit_opt(ids[2]).is_none());
        assert_eq!(g.city(c).garrison, [unit_ids::PHALANX | GARRISON_VETERAN, unit_ids::PHALANX]);
        assert!(!g.city(c).garrison_out);
        // they still keep order and defend the continent in the census
        assert_eq!(g.martial_law_units(c), 3);
        let k = crate::game::production::continent_index(g.map.tile(20, 20).continent);
        let on_map: i32 = g.all_units().filter(|u| u.owner == h && g.map.tile(u.x, u.y).is_land() && crate::game::production::continent_index(g.map.tile(u.x, u.y).continent) == k).map(|u| u.info().def as i32).sum();
        assert_eq!(g.census().defence[h as usize][k], on_map + 2 * 2);
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
        let h = g.human_slot;
        g.reset_moves(h);
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

    #[test]
    fn stashed_units_are_free_up_to_the_city_size_under_any_government() {
        let (mut g, c, _) = garrisoned();
        let h = g.human_slot;
        g.reset_moves(h);
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
        }
        g.reset_moves(ai);
        assert_eq!(g.unit(a).home, Some(big));
        assert_eq!(g.unit(b).home, None);
        let _ = small;
    }
}
