//! Validated deterministic room-and-corridor generation.

use super::{GENERATOR_VERSION, Map, Position, RunSeed, Tile};
use rand::{Rng, SeedableRng, seq::SliceRandom};
use rand_chacha::ChaCha8Rng;
use thiserror::Error;

const MAX_ATTEMPTS: usize = 32;
const MAX_WIDTH: u16 = 200;
const MAX_HEIGHT: u16 = 120;
const MAX_ROOMS: usize = 64;

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
