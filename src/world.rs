//! Deterministic dungeon terrain and generation.

use crate::game::RunSeed;
use rand::{Rng, SeedableRng, seq::SliceRandom};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use thiserror::Error;

pub const GENERATOR_VERSION: u32 = 1;
const MAX_ATTEMPTS: usize = 32;
const MAX_WIDTH: u16 = 200;
const MAX_HEIGHT: u16 = 120;
const MAX_ROOMS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

impl Position {
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
    #[must_use]
    pub const fn offset(self, dx: i32, dy: i32) -> Self {
        Self::new(self.x + dx, self.y + dy)
    }
    #[must_use]
    pub const fn chebyshev_distance(self, other: Self) -> i32 {
        let dx = (self.x - other.x).abs();
        let dy = (self.y - other.y).abs();
        if dx > dy { dx } else { dy }
    }
    #[must_use]
    pub const fn cardinal_neighbors(self) -> [Self; 4] {
        [
            self.offset(0, -1),
            self.offset(1, 0),
            self.offset(0, 1),
            self.offset(-1, 0),
        ]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Tile {
    Wall,
    Floor,
    ClosedDoor,
    OpenDoor,
    Exit,
}

impl Tile {
    #[must_use]
    pub const fn is_walkable(self) -> bool {
        matches!(self, Self::Floor | Self::OpenDoor | Self::Exit)
    }
    #[must_use]
    pub const fn is_traversable(self) -> bool {
        self.is_walkable() || matches!(self, Self::ClosedDoor)
    }
    #[must_use]
    pub const fn is_opaque(self) -> bool {
        matches!(self, Self::Wall | Self::ClosedDoor)
    }
    #[must_use]
    pub const fn glyph(self) -> char {
        match self {
            Self::Wall => '#',
            Self::Floor => '.',
            Self::ClosedDoor => '+',
            Self::OpenDoor => '/',
            Self::Exit => '>',
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Map {
    width: u16,
    height: u16,
    tiles: Vec<Tile>,
    pub player_start: Position,
    pub exit: Position,
    pub spawn_candidates: Vec<Position>,
    pub seed: RunSeed,
    pub generator_version: u32,
}

impl Map {
    #[must_use]
    pub const fn width(&self) -> u16 {
        self.width
    }
    #[must_use]
    pub const fn height(&self) -> u16 {
        self.height
    }
    #[must_use]
    pub fn contains(&self, p: Position) -> bool {
        p.x >= 0 && p.y >= 0 && p.x < i32::from(self.width) && p.y < i32::from(self.height)
    }
    #[must_use]
    pub fn tile(&self, p: Position) -> Option<Tile> {
        self.index(p).map(|index| self.tiles[index])
    }
    /// Open a closed door at `position`.
    ///
    /// Returns whether a door changed state.
    pub fn open_door(&mut self, position: Position) -> bool {
        if self.tile(position) != Some(Tile::ClosedDoor) {
            return false;
        }
        self.set_tile(position, Tile::OpenDoor);
        true
    }
    pub fn positions(&self) -> impl Iterator<Item = Position> + '_ {
        (0..i32::from(self.height))
            .flat_map(move |y| (0..i32::from(self.width)).map(move |x| Position::new(x, y)))
    }
    #[must_use]
    pub fn traversable_positions(&self) -> HashSet<Position> {
        let mut visited = HashSet::from([self.player_start]);
        let mut queue = VecDeque::from([self.player_start]);
        while let Some(position) = queue.pop_front() {
            for neighbor in position.cardinal_neighbors() {
                if !visited.contains(&neighbor)
                    && self.tile(neighbor).is_some_and(Tile::is_traversable)
                {
                    visited.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        visited
    }
    /// Validate structural invariants.
    ///
    /// # Errors
    ///
    /// Returns an invariant error for invalid storage, special positions,
    /// connectivity, doors, or spawn capacity.
    pub fn validate(&self) -> Result<(), GenerationError> {
        if self.tiles.len() != usize::from(self.width) * usize::from(self.height) {
            return Err(GenerationError::Invariant("tile storage size mismatch"));
        }
        if self.player_start == self.exit {
            return Err(GenerationError::Invariant("start and exit overlap"));
        }
        if self.tile(self.player_start) != Some(Tile::Floor)
            || self.tile(self.exit) != Some(Tile::Exit)
        {
            return Err(GenerationError::Invariant(
                "start or exit has invalid terrain",
            ));
        }
        if self.player_start.chebyshev_distance(self.exit) < 12 {
            return Err(GenerationError::Invariant("start and exit are too close"));
        }
        if !self.tiles.contains(&Tile::ClosedDoor) {
            return Err(GenerationError::Invariant("dungeon has no closed door"));
        }
        let reachable = self.traversable_positions();
        let count = self
            .tiles
            .iter()
            .filter(|tile| tile.is_traversable())
            .count();
        if reachable.len() != count || !reachable.contains(&self.exit) {
            return Err(GenerationError::Invariant(
                "traversable terrain is disconnected",
            ));
        }
        if self.spawn_candidates.len() < 15 {
            return Err(GenerationError::Invariant("fewer than 15 spawn candidates"));
        }
        let mut unique = HashSet::new();
        for p in &self.spawn_candidates {
            if !unique.insert(*p)
                || self.tile(*p) != Some(Tile::Floor)
                || *p == self.player_start
                || *p == self.exit
            {
                return Err(GenerationError::Invariant("invalid spawn candidate"));
            }
        }
        Ok(())
    }
    #[must_use]
    pub fn to_ascii(&self) -> String {
        let mut out =
            String::with_capacity((usize::from(self.width) + 1) * usize::from(self.height));
        for y in 0..i32::from(self.height) {
            for x in 0..i32::from(self.width) {
                let p = Position::new(x, y);
                out.push(if p == self.player_start {
                    '@'
                } else {
                    self.tile(p).map_or(' ', Tile::glyph)
                });
            }
            if y + 1 < i32::from(self.height) {
                out.push('\n');
            }
        }
        out
    }
    fn index(&self, p: Position) -> Option<usize> {
        if !self.contains(p) {
            return None;
        }
        Some(usize::try_from(p.y).ok()? * usize::from(self.width) + usize::try_from(p.x).ok()?)
    }
    fn set_tile(&mut self, p: Position, tile: Tile) {
        if let Some(index) = self.index(p) {
            self.tiles[index] = tile;
        }
    }

    #[cfg(test)]
    pub(crate) fn from_test_rows(rows: &[&str], seed: RunSeed) -> Self {
        assert!(!rows.is_empty());
        let width = rows[0].chars().count();
        assert!(rows.iter().all(|row| row.chars().count() == width));
        let mut player_start = None;
        let mut exit = None;
        let mut tiles = Vec::with_capacity(width * rows.len());
        for (y, row) in rows.iter().enumerate() {
            for (x, glyph) in row.chars().enumerate() {
                let position = Position::new(
                    i32::try_from(x).expect("test map width fits i32"),
                    i32::try_from(y).expect("test map height fits i32"),
                );
                tiles.push(match glyph {
                    '#' => Tile::Wall,
                    '.' => Tile::Floor,
                    '+' => Tile::ClosedDoor,
                    '/' => Tile::OpenDoor,
                    '>' => {
                        exit = Some(position);
                        Tile::Exit
                    }
                    '@' => {
                        player_start = Some(position);
                        Tile::Floor
                    }
                    other => panic!("unsupported test-map glyph: {other}"),
                });
            }
        }
        Self {
            width: u16::try_from(width).expect("test map width fits u16"),
            height: u16::try_from(rows.len()).expect("test map height fits u16"),
            tiles,
            player_start: player_start.expect("test map needs a player"),
            exit: exit.expect("test map needs an exit"),
            spawn_candidates: Vec::new(),
            seed,
            generator_version: GENERATOR_VERSION,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GeneratorConfig {
    pub width: u16,
    pub height: u16,
    pub min_rooms: usize,
    pub max_rooms: usize,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            width: 60,
            height: 28,
            min_rooms: 7,
            max_rooms: 14,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DungeonGenerator {
    config: GeneratorConfig,
}

impl DungeonGenerator {
    /// Construct a validated generator.
    ///
    /// # Errors
    ///
    /// Returns [`GenerationError::InvalidConfig`] for unsupported parameters.
    pub fn new(config: GeneratorConfig) -> Result<Self, GenerationError> {
        if !(20..=MAX_WIDTH).contains(&config.width)
            || !(14..=MAX_HEIGHT).contains(&config.height)
            || config.min_rooms < 2
            || config.max_rooms < config.min_rooms
            || config.max_rooms > MAX_ROOMS
        {
            return Err(GenerationError::InvalidConfig);
        }
        Ok(Self { config })
    }
    /// Generate one validated dungeon from an explicit seed.
    ///
    /// # Errors
    ///
    /// Returns [`GenerationError::Exhausted`] after bounded failed attempts.
    pub fn generate(&self, seed: RunSeed) -> Result<Map, GenerationError> {
        let mut rng = ChaCha8Rng::seed_from_u64(seed.0);
        for _ in 0..MAX_ATTEMPTS {
            if let Ok(map) = self.generate_once(seed, &mut rng) {
                return Ok(map);
            }
        }
        Err(GenerationError::Exhausted {
            seed,
            attempts: MAX_ATTEMPTS,
        })
    }
    fn generate_once(&self, seed: RunSeed, rng: &mut ChaCha8Rng) -> Result<Map, GenerationError> {
        let c = self.config;
        let mut map = Map {
            width: c.width,
            height: c.height,
            tiles: vec![Tile::Wall; usize::from(c.width) * usize::from(c.height)],
            player_start: Position::new(0, 0),
            exit: Position::new(0, 0),
            spawn_candidates: Vec::new(),
            seed,
            generator_version: GENERATOR_VERSION,
        };
        let mut rooms = Vec::with_capacity(c.max_rooms);
        for _ in 0..300 {
            if rooms.len() == c.max_rooms {
                break;
            }
            let width = rng.random_range(5..=10);
            let height = rng.random_range(4..=8);
            let room = Rect::new(
                rng.random_range(1..i32::from(c.width) - width - 1),
                rng.random_range(1..i32::from(c.height) - height - 1),
                width,
                height,
            );
            if rooms.iter().any(|existing| room.overlaps(*existing, 1)) {
                continue;
            }
            carve_room(&mut map, room);
            if let Some(previous) = rooms.last().copied() {
                carve_corridor(&mut map, previous.center(), room.center(), rng);
            }
            rooms.push(room);
        }
        if rooms.len() < c.min_rooms {
            return Err(GenerationError::Invariant("too few rooms"));
        }
        map.player_start = rooms[0].center();
        map.exit = rooms
            .iter()
            .map(|room| room.center())
            .max_by_key(|p| map.player_start.chebyshev_distance(*p))
            .ok_or(GenerationError::Invariant("no exit room"))?;
        map.set_tile(map.exit, Tile::Exit);

        let mut doors = door_candidates(&map);
        doors.retain(|p| {
            *p != map.player_start && *p != map.exit && p.chebyshev_distance(map.player_start) > 2
        });
        doors.shuffle(rng);
        for p in doors
            .into_iter()
            .take(rooms.len().saturating_sub(1).clamp(1, 6))
        {
            map.set_tile(p, Tile::ClosedDoor);
        }
        map.spawn_candidates = map
            .positions()
            .filter(|p| {
                map.tile(*p) == Some(Tile::Floor)
                    && *p != map.player_start
                    && p.chebyshev_distance(map.player_start) >= 6
                    && p.chebyshev_distance(map.exit) >= 2
            })
            .collect();
        map.spawn_candidates.shuffle(rng);
        map.validate()?;
        Ok(map)
    }
}

impl Default for DungeonGenerator {
    fn default() -> Self {
        Self::new(GeneratorConfig::default()).expect("default generator config must be valid")
    }
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}
impl Rect {
    const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    const fn center(self) -> Position {
        Position::new(self.x + self.width / 2, self.y + self.height / 2)
    }
    fn overlaps(self, other: Self, margin: i32) -> bool {
        self.x - margin < other.x + other.width
            && self.x + self.width + margin > other.x
            && self.y - margin < other.y + other.height
            && self.y + self.height + margin > other.y
    }
}

fn carve_room(map: &mut Map, room: Rect) {
    for y in room.y..room.y + room.height {
        for x in room.x..room.x + room.width {
            map.set_tile(Position::new(x, y), Tile::Floor);
        }
    }
}
fn carve_corridor(map: &mut Map, from: Position, to: Position, rng: &mut ChaCha8Rng) {
    if rng.random_bool(0.5) {
        carve_horizontal(map, from.x, to.x, from.y);
        carve_vertical(map, from.y, to.y, to.x);
    } else {
        carve_vertical(map, from.y, to.y, from.x);
        carve_horizontal(map, from.x, to.x, to.y);
    }
}
fn carve_horizontal(map: &mut Map, a: i32, b: i32, y: i32) {
    for x in a.min(b)..=a.max(b) {
        map.set_tile(Position::new(x, y), Tile::Floor);
    }
}
fn carve_vertical(map: &mut Map, a: i32, b: i32, x: i32) {
    for y in a.min(b)..=a.max(b) {
        map.set_tile(Position::new(x, y), Tile::Floor);
    }
}
fn door_candidates(map: &Map) -> Vec<Position> {
    map.positions()
        .filter(|p| {
            if map.tile(*p) != Some(Tile::Floor) {
                return false;
            }
            let n = map.tile(p.offset(0, -1)) == Some(Tile::Floor);
            let e = map.tile(p.offset(1, 0)) == Some(Tile::Floor);
            let s = map.tile(p.offset(0, 1)) == Some(Tile::Floor);
            let w = map.tile(p.offset(-1, 0)) == Some(Tile::Floor);
            (n && s && !e && !w) || (e && w && !n && !s)
        })
        .collect()
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum GenerationError {
    #[error("generator configuration cannot support a valid dungeon")]
    InvalidConfig,
    #[error("dungeon invariant failed: {0}")]
    Invariant(&'static str),
    #[error("generation for seed {seed:?} failed after {attempts} attempts")]
    Exhausted { seed: RunSeed, attempts: usize },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tile_semantics_distinguish_closed_doors() {
        assert!(!Tile::ClosedDoor.is_walkable());
        assert!(Tile::ClosedDoor.is_traversable());
        assert!(Tile::ClosedDoor.is_opaque());
        assert!(Tile::OpenDoor.is_walkable());
        assert!(!Tile::OpenDoor.is_opaque());
    }
    #[test]
    fn invalid_configs_are_rejected() {
        assert_eq!(
            DungeonGenerator::new(GeneratorConfig {
                width: 10,
                ..GeneratorConfig::default()
            })
            .unwrap_err(),
            GenerationError::InvalidConfig
        );
    }

    #[test]
    fn extreme_configs_are_rejected_before_allocation() {
        for config in [
            GeneratorConfig {
                max_rooms: usize::MAX,
                ..GeneratorConfig::default()
            },
            GeneratorConfig {
                width: u16::MAX,
                ..GeneratorConfig::default()
            },
            GeneratorConfig {
                height: u16::MAX,
                ..GeneratorConfig::default()
            },
        ] {
            assert_eq!(
                DungeonGenerator::new(config).unwrap_err(),
                GenerationError::InvalidConfig
            );
        }
    }
}
