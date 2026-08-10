//! Synchronous, deterministic game-domain state.

use crate::world::{DungeonGenerator, GENERATOR_VERSION, GenerationError, Map, Position, Tile};
use rand::{Rng, SeedableRng, seq::SliceRandom};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use thiserror::Error;

pub const FIELD_OF_VIEW_RADIUS: i32 = 8;
pub const PLAYER_MAX_HEALTH: i32 = 32;
pub const PLAYER_ARMOR: i32 = 2;
pub const PLAYER_DAMAGE: i32 = 6;
pub const CLEAVE_COOLDOWN_TURNS: u8 = 4;
const DIRECTIONS: [Direction; 8] = [
    Direction::North,
    Direction::NorthEast,
    Direction::East,
    Direction::SouthEast,
    Direction::South,
    Direction::SouthWest,
    Direction::West,
    Direction::NorthWest,
];

/// A seed that fully identifies a prototype run's random stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct RunSeed(pub u64);

/// Semantic commands accepted by the future deterministic engine.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Command {
    Move(Direction),
    Wait,
    UseCleave,
}

/// Eight-direction movement independent of any keyboard representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Direction {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

impl Direction {
    #[must_use]
    pub const fn delta(self) -> (i32, i32) {
        match self {
            Self::North => (0, -1),
            Self::NorthEast => (1, -1),
            Self::East => (1, 0),
            Self::SouthEast => (1, 1),
            Self::South => (0, 1),
            Self::SouthWest => (-1, 1),
            Self::West => (-1, 0),
            Self::NorthWest => (-1, -1),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RunStatus {
    Title,
    Active,
    Victory,
    Death,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandOutcome {
    Advanced,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ActorId(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ActorKind {
    GraveKnight,
    Skeleton,
    Ghoul,
    Cultist,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Telegraph {
    GhoulLunge { target: Position },
    CultistHex { target: Position },
}

impl Telegraph {
    #[must_use]
    pub const fn target(self) -> Position {
        match self {
            Self::GhoulLunge { target } | Self::CultistHex { target } => target,
        }
    }
}

impl ActorKind {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::GraveKnight => "Grave Knight",
            Self::Skeleton => "Skeleton",
            Self::Ghoul => "Ghoul",
            Self::Cultist => "Cultist",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    pub id: ActorId,
    pub kind: ActorKind,
    pub position: Position,
    pub health: i32,
    pub max_health: i32,
    pub armor: i32,
    pub damage: i32,
    pub active: bool,
    pub telegraph: Option<Telegraph>,
}

impl Actor {
    fn player(position: Position) -> Self {
        Self {
            id: ActorId(0),
            kind: ActorKind::GraveKnight,
            position,
            health: PLAYER_MAX_HEALTH,
            max_health: PLAYER_MAX_HEALTH,
            armor: PLAYER_ARMOR,
            damage: PLAYER_DAMAGE,
            active: true,
            telegraph: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExplorationGame {
    pub map: Map,
    pub player: Actor,
    pub hostiles: Vec<Actor>,
    pub cleave_cooldown: u8,
    pub turn: u64,
    pub status: RunStatus,
    pub help: bool,
    pub visible: HashSet<Position>,
    pub explored: HashSet<Position>,
    pub events: Vec<String>,
    fixed_seed: Option<RunSeed>,
    next_actor_id: u32,
    enemy_turns_enabled: bool,
}

impl ExplorationGame {
    /// Create a title-screen run from an explicit or freshly generated seed.
    ///
    /// # Errors
    ///
    /// Returns dungeon generation errors for the effective seed.
    pub fn new(seed: Option<RunSeed>) -> Result<Self, GenerationError> {
        let effective = seed.unwrap_or_else(|| RunSeed(rand::random()));
        Self::with_effective_seed(seed, effective)
    }

    fn with_effective_seed(
        fixed_seed: Option<RunSeed>,
        effective: RunSeed,
    ) -> Result<Self, GenerationError> {
        let map = DungeonGenerator::default().generate(effective)?;
        let mut game = Self::from_map(map, fixed_seed, RunStatus::Title);
        game.populate_enemies();
        game.enemy_turns_enabled = true;
        Ok(game)
    }

    fn from_map(map: Map, fixed_seed: Option<RunSeed>, status: RunStatus) -> Self {
        let player = Actor::player(map.player_start);
        let mut game = Self {
            map,
            player,
            hostiles: Vec::new(),
            cleave_cooldown: 0,
            turn: 0,
            status,
            help: false,
            visible: HashSet::new(),
            explored: HashSet::new(),
            events: vec!["The crypt waits in silence.".into()],
            fixed_seed,
            next_actor_id: 1,
            enemy_turns_enabled: false,
        };
        game.recompute_visibility();
        game
    }

    #[must_use]
    pub const fn seed(&self) -> RunSeed {
        self.map.seed
    }

    #[must_use]
    pub const fn generator_version(&self) -> u32 {
        GENERATOR_VERSION
    }

    pub fn start(&mut self) {
        if self.status == RunStatus::Title {
            self.status = RunStatus::Active;
            self.push_event("You descend into the ossuary.");
        }
    }

    pub fn toggle_help(&mut self) {
        self.help = !self.help;
    }

    /// Restart, using `fresh_seed` only for an originally unseeded session.
    ///
    /// # Errors
    ///
    /// Returns dungeon generation errors for the selected seed.
    pub fn restart_with_seed(&mut self, fresh_seed: RunSeed) -> Result<(), GenerationError> {
        let effective = self.fixed_seed.unwrap_or(fresh_seed);
        let map = DungeonGenerator::default().generate(effective)?;
        let mut replacement = Self::from_map(map, self.fixed_seed, RunStatus::Active);
        replacement.populate_enemies();
        replacement.enemy_turns_enabled = true;
        *self = replacement;
        self.push_event("The dungeon reforms around you.");
        Ok(())
    }

    /// Restart using a fresh random seed when the session was not explicit.
    ///
    /// # Errors
    ///
    /// Returns dungeon generation errors for the selected seed.
    pub fn restart(&mut self) -> Result<(), GenerationError> {
        self.restart_with_seed(RunSeed(rand::random()))
    }

    pub fn apply(&mut self, command: Command) -> CommandOutcome {
        if self.status != RunStatus::Active {
            return CommandOutcome::Rejected;
        }
        let outcome = match command {
            Command::Move(direction) => self.move_player(direction),
            Command::Wait => {
                self.advance_turn(true);
                self.push_event("You wait and listen.");
                CommandOutcome::Advanced
            }
            Command::UseCleave => self.use_cleave(),
        };
        if outcome == CommandOutcome::Advanced
            && self.status == RunStatus::Active
            && self.enemy_turns_enabled
        {
            self.run_enemy_turns();
        }
        outcome
    }

    #[must_use]
    pub fn is_visible(&self, position: Position) -> bool {
        self.visible.contains(&position)
    }

    #[must_use]
    pub fn is_explored(&self, position: Position) -> bool {
        self.explored.contains(&position)
    }

    #[must_use]
    pub fn telegraph_at(&self, position: Position) -> Option<Telegraph> {
        self.hostiles
            .iter()
            .filter_map(|actor| actor.telegraph)
            .find(|telegraph| telegraph.target() == position)
    }

    fn move_player(&mut self, direction: Direction) -> CommandOutcome {
        let (dx, dy) = direction.delta();
        let destination = self.player.position.offset(dx, dy);
        let Some(tile) = self.map.tile(destination) else {
            return CommandOutcome::Rejected;
        };
        if dx != 0 && dy != 0 {
            let horizontal = self.map.tile(self.player.position.offset(dx, 0));
            let vertical = self.map.tile(self.player.position.offset(0, dy));
            if !horizontal.is_some_and(Tile::is_walkable)
                || !vertical.is_some_and(Tile::is_walkable)
            {
                return CommandOutcome::Rejected;
            }
        }
        if let Some(index) = self
            .hostiles
            .iter()
            .position(|actor| actor.position == destination)
        {
            self.basic_attack(index);
            self.advance_turn(true);
            return CommandOutcome::Advanced;
        }
        if tile == Tile::ClosedDoor {
            let opened = self.map.open_door(destination);
            debug_assert!(opened);
            self.advance_turn(true);
            self.recompute_visibility();
            self.push_event("You force open the ancient door.");
            return CommandOutcome::Advanced;
        }
        if !tile.is_walkable() {
            return CommandOutcome::Rejected;
        }
        self.player.position = destination;
        self.advance_turn(true);
        self.recompute_visibility();
        if destination == self.map.exit {
            self.status = RunStatus::Victory;
            self.push_event("You escape the ossuary alive.");
        } else {
            self.push_event("You advance through the dark.");
        }
        CommandOutcome::Advanced
    }

    fn advance_turn(&mut self, tick_cooldown: bool) {
        self.turn += 1;
        if tick_cooldown {
            self.cleave_cooldown = self.cleave_cooldown.saturating_sub(1);
        }
    }

    fn basic_attack(&mut self, index: usize) {
        let damage = mitigated_damage(self.player.damage, self.hostiles[index].armor);
        self.hostiles[index].health -= damage;
        let kind = self.hostiles[index].kind;
        self.push_event(format!(
            "You strike the {} for {damage} damage.",
            kind.name()
        ));
        if self.hostiles[index].health <= 0 {
            let defeated = self.hostiles.remove(index);
            self.push_event(format!("The {} is destroyed.", defeated.kind.name()));
        }
    }

    fn use_cleave(&mut self) -> CommandOutcome {
        if self.cleave_cooldown != 0 {
            return CommandOutcome::Rejected;
        }
        let mut targets = self
            .hostiles
            .iter()
            .enumerate()
            .filter(|(_, actor)| self.player.position.chebyshev_distance(actor.position) == 1)
            .map(|(index, actor)| (actor.id, index))
            .collect::<Vec<_>>();
        if targets.is_empty() {
            return CommandOutcome::Rejected;
        }
        targets.sort_unstable_by_key(|(id, _)| *id);
        self.push_event("You unleash Cleave.");
        let mut defeated = Vec::new();
        for (_, index) in targets {
            let actor = &mut self.hostiles[index];
            let damage = mitigated_damage(self.player.damage, actor.armor);
            actor.health -= damage;
            self.events.push(format!(
                "Cleave hits {} {} for {damage} damage.",
                actor.kind.name(),
                actor.id.0
            ));
            if actor.health <= 0 {
                defeated.push(actor.id);
            }
        }
        self.hostiles.retain(|actor| !defeated.contains(&actor.id));
        while self.events.len() > 5 {
            self.events.remove(0);
        }
        self.cleave_cooldown = CLEAVE_COOLDOWN_TURNS;
        self.advance_turn(false);
        CommandOutcome::Advanced
    }

    /// Add a hostile actor with an allocated stable ID.
    pub fn spawn_hostile(
        &mut self,
        kind: ActorKind,
        position: Position,
        health: i32,
        armor: i32,
        damage: i32,
    ) -> ActorId {
        let id = ActorId(self.next_actor_id);
        self.next_actor_id += 1;
        self.hostiles.push(Actor {
            id,
            kind,
            position,
            health,
            max_health: health,
            armor,
            damage,
            active: false,
            telegraph: None,
        });
        id
    }

    /// Apply an incoming fixed-damage hit and return damage after armor.
    pub fn damage_player(&mut self, raw_damage: i32, source: &str) -> i32 {
        let damage = mitigated_damage(raw_damage, self.player.armor);
        self.player.health = (self.player.health - damage).max(0);
        self.push_event(format!("{source} hits you for {damage} damage."));
        if self.player.health == 0 {
            self.status = RunStatus::Death;
            self.push_event("The Grave Knight falls.");
        }
        damage
    }

    fn populate_enemies(&mut self) {
        let mut rng = ChaCha8Rng::seed_from_u64(self.map.seed.0 ^ 0x000A_11CE_5EED_DA7A);
        let count = rng
            .random_range(8..=15)
            .min(self.map.spawn_candidates.len());
        let mut positions = self.map.spawn_candidates.clone();
        positions.shuffle(&mut rng);
        for (index, position) in positions.into_iter().take(count).enumerate() {
            let kind = match index % 3 {
                0 => ActorKind::Skeleton,
                1 => ActorKind::Ghoul,
                _ => ActorKind::Cultist,
            };
            let (health, armor, damage) = match kind {
                ActorKind::Skeleton => (10, 1, 5),
                ActorKind::Ghoul => (14, 0, 7),
                ActorKind::Cultist => (9, 0, 6),
                ActorKind::GraveKnight => unreachable!(),
            };
            self.spawn_hostile(kind, position, health, armor, damage);
        }
    }

    fn run_enemy_turns(&mut self) {
        let mut ids = self
            .hostiles
            .iter()
            .map(|actor| actor.id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        for id in ids {
            let Some(index) = self.hostiles.iter().position(|actor| actor.id == id) else {
                continue;
            };
            if !self.hostiles[index].active
                && self.hostiles[index]
                    .position
                    .chebyshev_distance(self.player.position)
                    <= FIELD_OF_VIEW_RADIUS
                && has_line_of_sight(
                    &self.map,
                    self.hostiles[index].position,
                    self.player.position,
                )
            {
                self.hostiles[index].active = true;
                let kind = self.hostiles[index].kind;
                self.push_event(format!("{} {} awakens.", kind.name(), id.0));
            }
            if self.hostiles[index].active {
                self.run_enemy_action(id);
            }
            if self.status == RunStatus::Death {
                break;
            }
        }
        self.recompute_visibility();
    }

    fn run_enemy_action(&mut self, id: ActorId) {
        let Some(index) = self.hostiles.iter().position(|actor| actor.id == id) else {
            return;
        };
        if let Some(telegraph) = self.hostiles[index].telegraph.take() {
            self.resolve_telegraph(id, telegraph);
            return;
        }
        let kind = self.hostiles[index].kind;
        let distance = self.hostiles[index]
            .position
            .chebyshev_distance(self.player.position);
        match kind {
            ActorKind::Skeleton => {
                if distance == 1 {
                    self.enemy_melee(id);
                } else {
                    self.pursue(id);
                }
            }
            ActorKind::Ghoul => {
                if distance == 1 {
                    self.enemy_melee(id);
                } else if distance == 2
                    && has_line_of_sight(
                        &self.map,
                        self.hostiles[index].position,
                        self.player.position,
                    )
                {
                    let target = self.player.position;
                    self.hostiles[index].telegraph = Some(Telegraph::GhoulLunge { target });
                    self.push_event(format!(
                        "Ghoul {} marks ({}, {}) for a lunge.",
                        id.0, target.x, target.y
                    ));
                } else {
                    self.pursue(id);
                }
            }
            ActorKind::Cultist => {
                if distance <= 2 {
                    self.retreat(id);
                } else if distance <= FIELD_OF_VIEW_RADIUS
                    && has_line_of_sight(
                        &self.map,
                        self.hostiles[index].position,
                        self.player.position,
                    )
                {
                    let target = self.player.position;
                    self.hostiles[index].telegraph = Some(Telegraph::CultistHex { target });
                    self.push_event(format!(
                        "Cultist {} marks ({}, {}) with a hex.",
                        id.0, target.x, target.y
                    ));
                } else {
                    self.pursue(id);
                }
            }
            ActorKind::GraveKnight => {}
        }
    }

    fn resolve_telegraph(&mut self, id: ActorId, telegraph: Telegraph) {
        let Some(index) = self.hostiles.iter().position(|actor| actor.id == id) else {
            return;
        };
        let target = telegraph.target();
        let still_targeted = self.player.position == target
            && has_line_of_sight(&self.map, self.hostiles[index].position, target);
        match telegraph {
            Telegraph::GhoulLunge { .. } => {
                if still_targeted && self.hostiles[index].position.chebyshev_distance(target) <= 2 {
                    let damage = self.hostiles[index].damage;
                    self.damage_player(damage, &format!("Ghoul {} lunges and", id.0));
                } else {
                    if self.map.tile(target).is_some_and(Tile::is_walkable)
                        && !self.position_occupied(target, Some(id))
                    {
                        self.hostiles[index].position = target;
                    }
                    self.push_event(format!("Ghoul {} lunges through empty darkness.", id.0));
                }
            }
            Telegraph::CultistHex { .. } => {
                if still_targeted
                    && self.hostiles[index].position.chebyshev_distance(target)
                        <= FIELD_OF_VIEW_RADIUS
                {
                    let damage = self.hostiles[index].damage;
                    self.damage_player(damage, &format!("Cultist {}'s hex", id.0));
                } else {
                    self.push_event(format!("Cultist {}'s hex strikes only dust.", id.0));
                }
            }
        }
    }

    fn enemy_melee(&mut self, id: ActorId) {
        if let Some((kind, damage)) = self
            .hostiles
            .iter()
            .find(|actor| actor.id == id)
            .map(|actor| (actor.kind, actor.damage))
        {
            self.damage_player(damage, &format!("{} {}", kind.name(), id.0));
        }
    }

    fn pursue(&mut self, id: ActorId) {
        let Some(index) = self.hostiles.iter().position(|actor| actor.id == id) else {
            return;
        };
        let start = self.hostiles[index].position;
        let occupied = self
            .hostiles
            .iter()
            .filter(|actor| actor.id != id)
            .map(|actor| actor.position)
            .collect::<HashSet<_>>();
        let Some(next) = next_step_toward(&self.map, start, self.player.position, &occupied) else {
            return;
        };
        if next == self.player.position {
            self.enemy_melee(id);
        } else if self.map.tile(next) == Some(Tile::ClosedDoor) {
            self.map.open_door(next);
            self.push_event(format!(
                "{} {} opens a door.",
                self.hostiles[index].kind.name(),
                id.0
            ));
        } else {
            self.hostiles[index].position = next;
        }
    }

    fn retreat(&mut self, id: ActorId) {
        let Some(index) = self.hostiles.iter().position(|actor| actor.id == id) else {
            return;
        };
        let start = self.hostiles[index].position;
        let mut best = None;
        let mut best_distance = start.chebyshev_distance(self.player.position);
        for direction in DIRECTIONS {
            let (dx, dy) = direction.delta();
            let candidate = start.offset(dx, dy);
            let distance = candidate.chebyshev_distance(self.player.position);
            if distance > best_distance
                && can_step(&self.map, start, candidate)
                && !self.position_occupied(candidate, Some(id))
            {
                best = Some(candidate);
                best_distance = distance;
            }
        }
        if let Some(position) = best {
            self.hostiles[index].position = position;
            self.push_event(format!("Cultist {} retreats.", id.0));
        }
    }

    fn position_occupied(&self, position: Position, except: Option<ActorId>) -> bool {
        position == self.player.position
            || self
                .hostiles
                .iter()
                .any(|actor| except != Some(actor.id) && actor.position == position)
    }

    fn push_event(&mut self, event: impl Into<String>) {
        self.events.push(event.into());
        if self.events.len() > 5 {
            self.events.remove(0);
        }
    }

    fn recompute_visibility(&mut self) {
        self.visible.clear();
        for position in self.map.positions() {
            if self.player.position.chebyshev_distance(position) <= FIELD_OF_VIEW_RADIUS
                && has_line_of_sight(&self.map, self.player.position, position)
            {
                self.visible.insert(position);
                self.explored.insert(position);
            }
        }
    }
}

#[must_use]
pub const fn mitigated_damage(raw_damage: i32, armor: i32) -> i32 {
    let reduced = raw_damage - armor;
    if reduced < 1 { 1 } else { reduced }
}

fn next_step_toward(
    map: &Map,
    start: Position,
    target: Position,
    occupied: &HashSet<Position>,
) -> Option<Position> {
    let mut queue = VecDeque::from([start]);
    let mut previous = HashMap::from([(start, start)]);
    while let Some(position) = queue.pop_front() {
        if position == target {
            break;
        }
        for direction in DIRECTIONS {
            let (dx, dy) = direction.delta();
            let candidate = position.offset(dx, dy);
            if previous.contains_key(&candidate)
                || !can_step(map, position, candidate)
                || (candidate != target && occupied.contains(&candidate))
            {
                continue;
            }
            previous.insert(candidate, position);
            queue.push_back(candidate);
        }
    }
    if !previous.contains_key(&target) {
        return None;
    }
    let mut step = target;
    while previous[&step] != start {
        step = previous[&step];
    }
    Some(step)
}

fn can_step(map: &Map, from: Position, to: Position) -> bool {
    if !map.tile(to).is_some_and(Tile::is_traversable) {
        return false;
    }
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    if dx != 0 && dy != 0 {
        map.tile(from.offset(dx, 0)).is_some_and(Tile::is_walkable)
            && map.tile(from.offset(0, dy)).is_some_and(Tile::is_walkable)
    } else {
        true
    }
}

fn has_line_of_sight(map: &Map, from: Position, to: Position) -> bool {
    if from == to {
        return true;
    }
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let steps = dx.abs().max(dy.abs());
    let mut previous = from;
    for step in 1..=steps {
        let x = from.x + (dx * step + steps / 2 * dx.signum()) / steps;
        let y = from.y + (dy * step + steps / 2 * dy.signum()) / steps;
        let position = Position::new(x, y);
        if position.x != previous.x && position.y != previous.y {
            let side_x = Position::new(position.x, previous.y);
            let side_y = Position::new(previous.x, position.y);
            if map.tile(side_x).is_none_or(Tile::is_opaque)
                || map.tile(side_y).is_none_or(Tile::is_opaque)
            {
                return false;
            }
        }
        if position != to && map.tile(position).is_none_or(Tile::is_opaque) {
            return false;
        }
        previous = position;
    }
    true
}

/// Structured failures produced by game-domain validation.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum GameError {
    #[error("the requested command is not valid in the current game state")]
    InvalidCommand,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(rows: &[&str]) -> ExplorationGame {
        ExplorationGame::from_map(
            Map::from_test_rows(rows, RunSeed(7)),
            Some(RunSeed(7)),
            RunStatus::Active,
        )
    }

    fn game_with_ai(rows: &[&str]) -> ExplorationGame {
        let mut game = game(rows);
        game.enemy_turns_enabled = true;
        game
    }

    fn activate(game: &mut ExplorationGame, id: ActorId) {
        game.hostiles
            .iter_mut()
            .find(|actor| actor.id == id)
            .unwrap()
            .active = true;
    }

    fn assert_clean_populated_restart(game: &ExplorationGame, expected_seed: RunSeed) {
        let pristine = ExplorationGame::new(Some(expected_seed)).unwrap();
        assert_eq!(game.seed(), expected_seed);
        assert_eq!(game.status, RunStatus::Active);
        assert_eq!(game.turn, 0);
        assert_eq!(game.player, pristine.player);
        assert_eq!(game.player.health, PLAYER_MAX_HEALTH);
        assert_eq!(game.cleave_cooldown, 0);
        assert!(!game.help);
        assert_eq!(game.map, pristine.map);
        assert_eq!(game.hostiles, pristine.hostiles);
        assert_eq!(game.visible, pristine.visible);
        assert_eq!(game.explored, pristine.explored);
        assert_eq!(
            game.next_actor_id,
            u32::try_from(game.hostiles.len()).unwrap() + 1
        );
        assert!(game.enemy_turns_enabled);
        assert!(
            game.hostiles
                .iter()
                .all(|actor| !actor.active && actor.telegraph.is_none())
        );
        assert_eq!(
            game.events,
            [
                "The crypt waits in silence.".to_owned(),
                "The dungeon reforms around you.".to_owned()
            ]
        );
    }

    #[test]
    fn movement_wait_walls_and_turn_rules_are_authoritative() {
        let mut game = game(&["#######", "#@....#", "#....>#", "#######"]);
        assert_eq!(
            game.apply(Command::Move(Direction::North)),
            CommandOutcome::Rejected
        );
        assert_eq!(game.turn, 0);
        assert_eq!(
            game.apply(Command::Move(Direction::East)),
            CommandOutcome::Advanced
        );
        assert_eq!(game.player.position, Position::new(2, 1));
        assert_eq!(game.turn, 1);
        assert_eq!(game.apply(Command::Wait), CommandOutcome::Advanced);
        assert_eq!(game.turn, 2);
        assert_eq!(game.apply(Command::UseCleave), CommandOutcome::Rejected);
        assert_eq!(game.turn, 2);
    }

    #[test]
    fn title_and_help_are_free_state_changes() {
        let seed = RunSeed(42);
        let mut game = ExplorationGame::with_effective_seed(Some(seed), seed).unwrap();
        assert_eq!(game.status, RunStatus::Title);
        game.toggle_help();
        assert!(game.help);
        assert_eq!(game.turn, 0);
        game.start();
        assert_eq!(game.status, RunStatus::Active);
        assert_eq!(game.turn, 0);
    }

    #[test]
    fn diagonals_cannot_cut_either_blocked_corner() {
        for rows in [
            ["#####", "#@#.#", "#...#", "#..>#", "#####"],
            ["#####", "#@..#", "##..#", "#..>#", "#####"],
        ] {
            let mut game = game(&rows);
            assert_eq!(
                game.apply(Command::Move(Direction::SouthEast)),
                CommandOutcome::Rejected
            );
            assert_eq!(game.player.position, Position::new(1, 1));
            assert_eq!(game.turn, 0);
        }

        let mut open = game(&["#####", "#@..#", "#...#", "#..>#", "#####"]);
        assert_eq!(
            open.apply(Command::Move(Direction::SouthEast)),
            CommandOutcome::Advanced
        );
        assert_eq!(open.player.position, Position::new(2, 2));
        assert_eq!(open.turn, 1);
    }

    #[test]
    fn movement_cannot_leave_map_bounds() {
        let mut game = game(&["@..", "..>"]);
        assert_eq!(
            game.apply(Command::Move(Direction::NorthWest)),
            CommandOutcome::Rejected
        );
        assert_eq!(game.player.position, Position::new(0, 0));
        assert_eq!(game.turn, 0);
    }

    #[test]
    fn bumping_a_door_opens_it_for_one_turn_without_moving() {
        let mut game = game(&["######", "#@+>##", "######"]);
        assert_eq!(
            game.apply(Command::Move(Direction::East)),
            CommandOutcome::Advanced
        );
        assert_eq!(game.player.position, Position::new(1, 1));
        assert_eq!(game.map.tile(Position::new(2, 1)), Some(Tile::OpenDoor));
        assert_eq!(game.turn, 1);
        assert_eq!(
            game.apply(Command::Move(Direction::East)),
            CommandOutcome::Advanced
        );
        assert_eq!(game.player.position, Position::new(2, 1));
        assert_eq!(game.turn, 2);
    }

    #[test]
    fn walls_doors_and_corners_occlude_field_of_view() {
        let wall = game(&["########", "#@.#..>#", "########"]);
        assert!(wall.is_visible(Position::new(3, 1)));
        assert!(!wall.is_visible(Position::new(4, 1)));

        let mut door = game(&["########", "#@+...>#", "########"]);
        assert!(door.is_visible(Position::new(2, 1)));
        assert!(!door.is_visible(Position::new(3, 1)));
        door.apply(Command::Move(Direction::East));
        assert!(door.is_visible(Position::new(3, 1)));

        let corner = game(&["#####", "#@#>#", "##..#", "#####"]);
        assert!(!corner.is_visible(Position::new(2, 2)));
    }

    #[test]
    fn explored_memory_survives_after_visibility_moves_away() {
        let mut game = game(&[
            "########################",
            "#@....................>#",
            "########################",
        ]);
        let origin = game.player.position;
        for _ in 0..10 {
            assert_eq!(
                game.apply(Command::Move(Direction::East)),
                CommandOutcome::Advanced
            );
        }
        assert!(!game.is_visible(origin));
        assert!(game.is_explored(origin));
    }

    #[test]
    fn entering_the_exit_wins_immediately() {
        let mut game = game(&["#####", "#@>##", "#####"]);
        assert_eq!(
            game.apply(Command::Move(Direction::East)),
            CommandOutcome::Advanced
        );
        assert_eq!(game.status, RunStatus::Victory);
        assert_eq!(game.turn, 1);
        assert_eq!(game.apply(Command::Wait), CommandOutcome::Rejected);
        assert_eq!(game.turn, 1);
    }

    #[test]
    fn explicit_restart_repeats_and_unseeded_restart_uses_candidate() {
        let seed_a = RunSeed(0xA11CE);
        let seed_b = RunSeed(0xB0B);
        let mut explicit = ExplorationGame::with_effective_seed(Some(seed_a), seed_a).unwrap();
        let first_map = explicit.map.clone();
        explicit.restart_with_seed(seed_b).unwrap();
        assert_eq!(explicit.seed(), seed_a);
        assert_eq!(explicit.map, first_map);

        let mut fresh = ExplorationGame::with_effective_seed(None, seed_a).unwrap();
        fresh.restart_with_seed(seed_b).unwrap();
        assert_eq!(fresh.seed(), seed_b);
        assert_ne!(fresh.map, first_map);
    }

    #[test]
    fn identical_seed_and_commands_reproduce_complete_exploration_state() {
        let seed = RunSeed(0xD4_4B);
        let mut left = ExplorationGame::with_effective_seed(Some(seed), seed).unwrap();
        let mut right = left.clone();
        left.start();
        right.start();
        let commands = [
            Command::Wait,
            Command::Move(Direction::North),
            Command::Move(Direction::East),
            Command::Move(Direction::SouthEast),
        ];
        for command in commands {
            left.apply(command);
            right.apply(command);
        }
        assert_eq!(left, right);
    }

    #[test]
    fn bump_attacks_use_fixed_damage_and_hold_position() {
        let mut game = game(&["######", "#@.>##", "######"]);
        let enemy = game.spawn_hostile(ActorKind::Skeleton, Position::new(2, 1), 10, 2, 4);
        assert_eq!(
            game.apply(Command::Move(Direction::East)),
            CommandOutcome::Advanced
        );
        assert_eq!(game.player.position, Position::new(1, 1));
        assert_eq!(game.turn, 1);
        assert_eq!(
            game.hostiles
                .iter()
                .find(|actor| actor.id == enemy)
                .unwrap()
                .health,
            6
        );
        assert_eq!(
            game.events.last().unwrap(),
            "You strike the Skeleton for 4 damage."
        );
    }

    #[test]
    fn armor_never_reduces_a_successful_hit_below_one_and_defeat_removes_actor() {
        let mut game = game(&["######", "#@.>##", "######"]);
        let enemy = game.spawn_hostile(ActorKind::Skeleton, Position::new(2, 1), 2, 99, 4);
        game.apply(Command::Move(Direction::East));
        assert_eq!(game.hostiles[0].health, 1);
        game.apply(Command::Move(Direction::East));
        assert!(game.hostiles.iter().all(|actor| actor.id != enemy));
        assert_eq!(game.turn, 2);
        assert_eq!(game.events.last().unwrap(), "The Skeleton is destroyed.");
    }

    #[test]
    fn cleave_hits_all_adjacent_targets_in_stable_id_order() {
        let mut game = game(&["#######", "#.....#", "#..@..#", "#....>#", "#######"]);
        let first = game.spawn_hostile(ActorKind::Skeleton, Position::new(2, 1), 9, 1, 3);
        let second = game.spawn_hostile(ActorKind::Ghoul, Position::new(4, 2), 9, 1, 3);
        let distant = game.spawn_hostile(ActorKind::Cultist, Position::new(1, 1), 9, 1, 3);

        assert_eq!(game.apply(Command::UseCleave), CommandOutcome::Advanced);
        assert_eq!(game.turn, 1);
        assert_eq!(game.cleave_cooldown, CLEAVE_COOLDOWN_TURNS);
        assert_eq!(
            game.hostiles.iter().find(|a| a.id == first).unwrap().health,
            4
        );
        assert_eq!(
            game.hostiles
                .iter()
                .find(|a| a.id == second)
                .unwrap()
                .health,
            4
        );
        assert_eq!(
            game.hostiles
                .iter()
                .find(|a| a.id == distant)
                .unwrap()
                .health,
            9
        );
        assert_eq!(
            game.events[game.events.len() - 2],
            "Cleave hits Skeleton 1 for 5 damage."
        );
        assert_eq!(
            game.events[game.events.len() - 1],
            "Cleave hits Ghoul 2 for 5 damage."
        );
    }

    #[test]
    fn cleave_rejections_are_free_and_four_later_turns_restore_it() {
        let mut game = game(&["######", "#@..>#", "######"]);
        assert_eq!(game.apply(Command::UseCleave), CommandOutcome::Rejected);
        assert_eq!(game.turn, 0);
        game.spawn_hostile(ActorKind::Skeleton, Position::new(2, 1), 30, 0, 3);
        assert_eq!(game.apply(Command::UseCleave), CommandOutcome::Advanced);
        assert_eq!(game.cleave_cooldown, 4);
        assert_eq!(game.apply(Command::UseCleave), CommandOutcome::Rejected);
        assert_eq!(game.turn, 1);
        for expected in [3, 2, 1, 0] {
            game.apply(Command::Wait);
            assert_eq!(game.cleave_cooldown, expected);
        }
        assert_eq!(game.turn, 5);
        assert_eq!(game.apply(Command::UseCleave), CommandOutcome::Advanced);
    }

    #[test]
    fn player_damage_uses_armor_and_death_is_immediate() {
        let mut game = game(&["#####", "#@.>#", "#####"]);
        assert_eq!(game.damage_player(3, "Skeleton"), 1);
        assert_eq!(game.player.health, 31);
        assert_eq!(game.status, RunStatus::Active);
        assert_eq!(game.damage_player(99, "Ghoul"), 97);
        assert_eq!(game.player.health, 0);
        assert_eq!(game.status, RunStatus::Death);
        assert_eq!(game.events.last().unwrap(), "The Grave Knight falls.");
        assert_eq!(game.apply(Command::Wait), CommandOutcome::Rejected);
    }

    #[test]
    fn one_thousand_runs_have_legal_deterministic_enemy_populations() {
        for seed in 0..1_000 {
            let game = ExplorationGame::new(Some(RunSeed(seed))).unwrap();
            assert!((8..=15).contains(&game.hostiles.len()));
            let positions = game
                .hostiles
                .iter()
                .map(|actor| actor.position)
                .collect::<HashSet<_>>();
            assert_eq!(positions.len(), game.hostiles.len());
            assert!(game.hostiles.iter().all(|actor| {
                game.map.tile(actor.position) == Some(Tile::Floor)
                    && actor.position != game.player.position
                    && actor.position != game.map.exit
            }));
            assert!(
                game.hostiles
                    .iter()
                    .any(|actor| actor.kind == ActorKind::Skeleton)
            );
            assert!(
                game.hostiles
                    .iter()
                    .any(|actor| actor.kind == ActorKind::Ghoul)
            );
            assert!(
                game.hostiles
                    .iter()
                    .any(|actor| actor.kind == ActorKind::Cultist)
            );
            let repeated = ExplorationGame::new(Some(RunSeed(seed))).unwrap();
            assert_eq!(game.hostiles, repeated.hostiles);
        }
    }

    #[test]
    fn detection_uses_independent_los_radius_and_activation_persists() {
        let mut game = game_with_ai(&["##############", "#@..........>#", "##############"]);
        let id = game.spawn_hostile(ActorKind::Skeleton, Position::new(10, 1), 10, 1, 5);
        game.apply(Command::Wait);
        assert!(!game.hostiles[0].active);
        game.player.position = Position::new(2, 1);
        game.apply(Command::Wait);
        assert!(game.hostiles[0].active);
        game.player.position = Position::new(1, 1);
        game.apply(Command::Wait);
        assert!(
            game.hostiles
                .iter()
                .find(|actor| actor.id == id)
                .unwrap()
                .active
        );

        let mut blocked = game_with_ai(&["########", "#@.#..>#", "########"]);
        blocked.spawn_hostile(ActorKind::Skeleton, Position::new(5, 1), 10, 1, 5);
        blocked.apply(Command::Wait);
        assert!(!blocked.hostiles[0].active);
    }

    #[test]
    fn skeletons_pursue_open_doors_and_attack() {
        let mut pursuit = game_with_ai(&["########", "#@....>#", "########"]);
        let id = pursuit.spawn_hostile(ActorKind::Skeleton, Position::new(4, 1), 10, 1, 5);
        activate(&mut pursuit, id);
        pursuit.apply(Command::Wait);
        assert_eq!(pursuit.hostiles[0].position, Position::new(3, 1));
        pursuit.apply(Command::Wait);
        assert_eq!(pursuit.hostiles[0].position, Position::new(2, 1));
        pursuit.apply(Command::Wait);
        assert_eq!(pursuit.player.health, PLAYER_MAX_HEALTH - 3);

        let mut door = game_with_ai(&["########", "#@.+..>#", "########"]);
        let id = door.spawn_hostile(ActorKind::Skeleton, Position::new(4, 1), 10, 1, 5);
        activate(&mut door, id);
        door.apply(Command::Wait);
        assert_eq!(door.map.tile(Position::new(3, 1)), Some(Tile::OpenDoor));
        assert_eq!(door.hostiles[0].position, Position::new(4, 1));
    }

    #[test]
    fn ghoul_lunge_telegraphs_then_hits_or_moves_to_an_evaded_mark() {
        let mut hit = game_with_ai(&["#######", "#@...>#", "#.....#", "#######"]);
        let id = hit.spawn_hostile(ActorKind::Ghoul, Position::new(3, 1), 14, 0, 7);
        activate(&mut hit, id);
        hit.apply(Command::Wait);
        assert_eq!(
            hit.hostiles[0].telegraph,
            Some(Telegraph::GhoulLunge {
                target: Position::new(1, 1)
            })
        );
        hit.apply(Command::Wait);
        assert_eq!(hit.player.health, PLAYER_MAX_HEALTH - 5);
        assert!(hit.hostiles[0].telegraph.is_none());

        let mut evade = game_with_ai(&["#######", "#@...>#", "#.....#", "#######"]);
        let id = evade.spawn_hostile(ActorKind::Ghoul, Position::new(3, 1), 14, 0, 7);
        activate(&mut evade, id);
        evade.apply(Command::Wait);
        evade.apply(Command::Move(Direction::South));
        assert_eq!(evade.player.health, PLAYER_MAX_HEALTH);
        assert_eq!(evade.hostiles[0].position, Position::new(1, 1));
    }

    #[test]
    fn cultist_hex_telegraphs_and_can_be_evaded_while_cultists_keep_distance() {
        let mut hit = game_with_ai(&["#########", "#@.....>#", "#.......#", "#########"]);
        let id = hit.spawn_hostile(ActorKind::Cultist, Position::new(5, 1), 9, 0, 6);
        activate(&mut hit, id);
        hit.apply(Command::Wait);
        assert!(matches!(
            hit.hostiles[0].telegraph,
            Some(Telegraph::CultistHex { .. })
        ));
        hit.apply(Command::Wait);
        assert_eq!(hit.player.health, PLAYER_MAX_HEALTH - 4);

        let mut evade = game_with_ai(&["#########", "#@.....>#", "#.......#", "#########"]);
        let id = evade.spawn_hostile(ActorKind::Cultist, Position::new(5, 1), 9, 0, 6);
        activate(&mut evade, id);
        evade.apply(Command::Wait);
        evade.apply(Command::Move(Direction::South));
        assert_eq!(evade.player.health, PLAYER_MAX_HEALTH);

        let mut retreat = game_with_ai(&["########", "#@....>#", "#......#", "########"]);
        let id = retreat.spawn_hostile(ActorKind::Cultist, Position::new(3, 1), 9, 0, 6);
        activate(&mut retreat, id);
        retreat.apply(Command::Wait);
        assert!(
            retreat.hostiles[0]
                .position
                .chebyshev_distance(retreat.player.position)
                > 2
        );
    }

    #[test]
    fn enemies_act_in_stable_id_order_and_can_kill_the_player() {
        let mut game = game_with_ai(&["#######", "#..@.>#", "#.....#", "#######"]);
        game.player.health = 3;
        let first = game.spawn_hostile(ActorKind::Skeleton, Position::new(2, 1), 10, 1, 3);
        let second = game.spawn_hostile(ActorKind::Ghoul, Position::new(4, 1), 14, 0, 3);
        activate(&mut game, first);
        activate(&mut game, second);
        game.apply(Command::Wait);
        assert_eq!(game.status, RunStatus::Active);
        assert_eq!(game.player.health, 1);
        assert!(game.events[game.events.len() - 2].starts_with("Skeleton 1"));
        assert!(game.events[game.events.len() - 1].starts_with("Ghoul 2"));
        game.apply(Command::Wait);
        assert_eq!(game.status, RunStatus::Death);
    }

    #[test]
    fn exit_victory_precedes_enemy_response_with_living_hostiles() {
        let mut game = game_with_ai(&["######", "#@>..#", "######"]);
        let id = game.spawn_hostile(ActorKind::Skeleton, Position::new(3, 1), 10, 1, 99);
        activate(&mut game, id);
        assert_eq!(
            game.apply(Command::Move(Direction::East)),
            CommandOutcome::Advanced
        );
        assert_eq!(game.status, RunStatus::Victory);
        assert_eq!(game.player.health, PLAYER_MAX_HEALTH);
        assert_eq!(game.hostiles.len(), 1);
    }

    #[test]
    fn populated_victory_and_death_restart_without_state_leakage() {
        let fixed_seed = RunSeed(0xD4_4B);
        let mut game = ExplorationGame::new(Some(fixed_seed)).unwrap();
        game.start();
        game.help = true;
        game.cleave_cooldown = 3;
        game.hostiles[0].active = true;
        game.hostiles[0].telegraph = Some(Telegraph::CultistHex {
            target: game.player.position,
        });
        game.damage_player(999, "the dark");
        assert_eq!(game.status, RunStatus::Death);
        game.restart_with_seed(RunSeed(1)).unwrap();
        assert_clean_populated_restart(&game, fixed_seed);

        let exit = game.map.exit;
        let (direction, staging) = DIRECTIONS
            .iter()
            .copied()
            .find_map(|direction| {
                let (dx, dy) = direction.delta();
                let staging = exit.offset(-dx, -dy);
                (can_step(&game.map, staging, exit)
                    && !game.hostiles.iter().any(|actor| actor.position == staging))
                .then_some((direction, staging))
            })
            .expect("generated exit has an unoccupied approach");
        game.player.position = staging;
        game.recompute_visibility();
        assert_eq!(
            game.apply(Command::Move(direction)),
            CommandOutcome::Advanced
        );
        assert_eq!(game.status, RunStatus::Victory);
        assert!(!game.hostiles.is_empty());
        game.help = true;
        game.cleave_cooldown = 2;
        game.hostiles[0].active = true;
        game.hostiles[0].telegraph = Some(Telegraph::GhoulLunge {
            target: game.player.position,
        });
        game.restart_with_seed(RunSeed(2)).unwrap();
        assert_clean_populated_restart(&game, fixed_seed);

        let fresh_a = RunSeed(0xAA);
        let fresh_b = RunSeed(0xBB);
        let mut unseeded = ExplorationGame::with_effective_seed(None, fresh_a).unwrap();
        unseeded.start();
        unseeded.damage_player(999, "the dark");
        unseeded.restart_with_seed(fresh_b).unwrap();
        assert_clean_populated_restart(&unseeded, fresh_b);
    }
}
