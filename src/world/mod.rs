//! Deterministic dungeon terrain and generation.

mod generation;

pub use generation::{DungeonGenerator, GenerationError, GeneratorConfig};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

pub const GENERATOR_VERSION: u32 = 1;

/// A seed that fully identifies a deterministic dungeon run.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct RunSeed(pub u64);

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TileDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub glyph_ascii: char,
    pub glyph_unicode: char,
    pub walkable: bool,
    pub opaque: bool,
}

impl Tile {
    #[must_use]
    pub const fn definition(self) -> TileDefinition {
        match self {
            Self::Wall => TileDefinition {
                name: "Ossuary Wall",
                description: "Mortared bone and lightless stone.",
                glyph_ascii: '#',
                glyph_unicode: '▓',
                walkable: false,
                opaque: true,
            },
            Self::Floor => TileDefinition {
                name: "Crypt Floor",
                description: "Dust lies thick across worn flagstones.",
                glyph_ascii: '.',
                glyph_unicode: '·',
                walkable: true,
                opaque: false,
            },
            Self::ClosedDoor => TileDefinition {
                name: "Sealed Door",
                description: "An ancient door swollen shut with damp.",
                glyph_ascii: '+',
                glyph_unicode: '╬',
                walkable: false,
                opaque: true,
            },
            Self::OpenDoor => TileDefinition {
                name: "Open Door",
                description: "A forced doorway yawns into the dark.",
                glyph_ascii: '/',
                glyph_unicode: '/',
                walkable: true,
                opaque: false,
            },
            Self::Exit => TileDefinition {
                name: "Stair to Dying Light",
                description: "The only path out of the ossuary.",
                glyph_ascii: '>',
                glyph_unicode: '>',
                walkable: true,
                opaque: false,
            },
        }
    }

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
