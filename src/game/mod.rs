//! Synchronous, deterministic game-domain state.

mod abilities;
mod ai;
mod classes;
mod combat;
mod inspection;
mod items;
mod population;
mod targeting;
mod visibility;

pub use crate::world::RunSeed;
use crate::world::{DungeonGenerator, GENERATOR_VERSION, GenerationError, Map, Position, Tile};
pub use abilities::{
    ABILITY_SLOT_COUNT, AbilityAvailability, AbilityDefinition, AbilityEffect, AbilityId,
    AbilitySlot, AbilityState, AbilityTargeting,
};
#[cfg(test)]
use ai::can_step;
pub use classes::{AbilityBinding, CatalogError, ClassDefinition, ClassId, GameCatalog};
pub use combat::mitigated_damage;
pub use inspection::{
    EntityInspection, InspectState, Inspection, InspectionDetail, InspectionVisibility,
};
pub use items::{
    GroundItem, INVENTORY_SLOT_COUNT, ItemCatalog, ItemCatalogError, ItemDefinition, ItemEffect,
    ItemId, ItemInstance, ItemInstanceId, POTION_HEAL, TORCH_VISION_RADIUS,
};
pub use population::MonsterDefinition;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
pub use targeting::{TargetValidity, TargetingState};
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

/// Semantic commands accepted by the future deterministic engine.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Command {
    Move(Direction),
    Wait,
    UseAbility(AbilitySlot),
    MoveCursor(Direction),
    CycleTarget { backwards: bool },
    ConfirmTarget,
    CancelMode,
    ToggleInspect,
    PickupItem,
    ToggleItemUse,
    UseItemSlot(u8),
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
            Self::Skeleton => "Skeleton",
            Self::Ghoul => "Ghoul",
            Self::Cultist => "Cultist",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Player {
    pub position: Position,
    pub health: i32,
    pub max_health: i32,
    pub mana: i32,
    pub max_mana: i32,
    pub armor: i32,
    pub damage: i32,
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

impl Player {
    fn from_class(position: Position, class: &ClassDefinition) -> Self {
        Self {
            position,
            health: class.max_health,
            max_health: class.max_health,
            mana: class.max_mana,
            max_mana: class.max_mana,
            armor: class.armor,
            damage: class.base_damage,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct ExplorationGame {
    pub map: Map,
    pub player: Player,
    pub hostiles: Vec<Actor>,
    pub class_id: ClassId,
    pub abilities: [Option<AbilityState>; ABILITY_SLOT_COUNT],
    pub turn: u64,
    pub status: RunStatus,
    pub help: bool,
    pub targeting: Option<TargetingState>,
    pub inspecting: Option<InspectState>,
    pub using_item: bool,
    pub inventory: [Option<ItemInstance>; INVENTORY_SLOT_COUNT],
    pub ground_items: Vec<GroundItem>,
    pub visible: HashSet<Position>,
    pub explored: HashSet<Position>,
    pub events: Vec<String>,
    fixed_seed: Option<RunSeed>,
    next_actor_id: u32,
    enemy_turns_enabled: bool,
    pub debug_godmode_enabled: bool,
    pub godmode: bool,
    pub character_info: bool,
    combat_roll_count: u64,
    next_item_instance_id: u32,
    catalog: GameCatalog,
    item_catalog: ItemCatalog,
}

impl ExplorationGame {
    /// Create a title-screen run from an explicit or freshly generated seed.
    ///
    /// # Errors
    ///
    /// Returns dungeon generation errors for the effective seed.
    pub fn new(seed: Option<RunSeed>) -> Result<Self, GenerationError> {
        let effective = seed.unwrap_or_else(|| RunSeed(rand::random()));
        Self::with_effective_seed_and_debug(seed, effective, false)
    }

    /// Create a run for a built-in class.
    ///
    /// # Errors
    ///
    /// Returns an unknown-class error or dungeon generation failure.
    pub fn new_with_class(seed: Option<RunSeed>, class_id: ClassId) -> Result<Self, GameError> {
        Self::new_with_catalog(seed, class_id, GameCatalog::builtin())
    }

    /// Create a run from a prevalidated immutable content catalog.
    ///
    /// # Errors
    ///
    /// Returns an unknown-class error or dungeon generation failure.
    pub fn new_with_catalog(
        seed: Option<RunSeed>,
        class_id: ClassId,
        catalog: GameCatalog,
    ) -> Result<Self, GameError> {
        let effective = seed.unwrap_or_else(|| RunSeed(rand::random()));
        Self::with_effective_seed_and_catalog(seed, effective, class_id, catalog)
    }

    #[allow(dead_code)]
    fn with_effective_seed(
        fixed_seed: Option<RunSeed>,
        effective: RunSeed,
    ) -> Result<Self, GenerationError> {
        Self::with_effective_seed_and_debug(fixed_seed, effective, false)
    }

    /// # Errors
    ///
    /// Returns dungeon generation errors for the selected seed.
    pub fn with_effective_seed_and_debug(
        fixed_seed: Option<RunSeed>,
        effective: RunSeed,
        debug_godmode_enabled: bool,
    ) -> Result<Self, GenerationError> {
        match Self::with_effective_seed_and_catalog(
            fixed_seed,
            effective,
            ClassId::GRAVE_KNIGHT,
            GameCatalog::builtin(),
        ) {
            Ok(mut game) => {
                game.debug_godmode_enabled = debug_godmode_enabled;
                Ok(game)
            }
            Err(GameError::Generation(error)) => Err(error),
            Err(error) => unreachable!("built-in content must remain valid: {error}"),
        }
    }

    fn with_effective_seed_and_catalog(
        fixed_seed: Option<RunSeed>,
        effective: RunSeed,
        class_id: ClassId,
        catalog: GameCatalog,
    ) -> Result<Self, GameError> {
        let map = DungeonGenerator::default().generate(effective)?;
        let mut game = Self::from_map(map, fixed_seed, RunStatus::Title, class_id, catalog)?;
        game.populate_enemies();
        game.populate_items();
        game.enemy_turns_enabled = true;
        Ok(game)
    }

    fn from_map(
        map: Map,
        fixed_seed: Option<RunSeed>,
        status: RunStatus,
        class_id: ClassId,
        catalog: GameCatalog,
    ) -> Result<Self, GameError> {
        let class = catalog
            .class(class_id)
            .ok_or(GameError::UnknownClass(class_id))?;
        let player = Player::from_class(map.player_start, class);
        let abilities = catalog
            .initial_ability_state(class_id)
            .ok_or(GameError::UnknownClass(class_id))?;
        let mut game = Self {
            map,
            player,
            hostiles: Vec::new(),
            class_id,
            abilities,
            turn: 0,
            status,
            help: false,
            targeting: None,
            inspecting: None,
            using_item: false,
            inventory: [None; INVENTORY_SLOT_COUNT],
            ground_items: Vec::new(),
            visible: HashSet::new(),
            explored: HashSet::new(),
            events: vec!["The crypt waits in silence.".into()],
            fixed_seed,
            next_actor_id: 1,
            enemy_turns_enabled: false,
            debug_godmode_enabled: false,
            godmode: false,
            character_info: false,
            combat_roll_count: 0,
            next_item_instance_id: 1,
            catalog,
            item_catalog: ItemCatalog::builtin(),
        };
        game.recompute_visibility();
        Ok(game)
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

    pub fn set_debug_godmode_enabled(&mut self, enabled: bool) {
        self.debug_godmode_enabled = enabled;
        if !enabled {
            self.godmode = false;
        }
    }

    pub fn toggle_godmode(&mut self) {
        if self.debug_godmode_enabled {
            self.godmode = !self.godmode;
            self.push_event(if self.godmode {
                "GODMODE enabled."
            } else {
                "GODMODE disabled."
            });
        }
    }

    pub fn toggle_character_info(&mut self) {
        self.character_info = !self.character_info;
    }

    pub fn toggle_help(&mut self) {
        self.help = !self.help;
    }

    /// Restart, using `fresh_seed` only for an originally unseeded session.
    ///
    /// # Errors
    ///
    /// Returns dungeon generation errors for the selected seed.
    ///
    /// # Panics
    ///
    /// Panics only if an already-running game's serialized catalog no longer
    /// contains its selected class.
    pub fn restart_with_seed(&mut self, fresh_seed: RunSeed) -> Result<(), GenerationError> {
        let effective = self.fixed_seed.unwrap_or(fresh_seed);
        let map = DungeonGenerator::default().generate(effective)?;
        let mut replacement = Self::from_map(
            map,
            self.fixed_seed,
            RunStatus::Active,
            self.class_id,
            self.catalog.clone(),
        )
        .expect("the current run always retains a valid class catalog");
        replacement.populate_enemies();
        replacement.populate_items();
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
        if let Some(targeting) = self.targeting {
            let outcome = match command {
                Command::MoveCursor(direction) | Command::Move(direction) => {
                    self.move_target_cursor(direction)
                }
                Command::CycleTarget { backwards } => self.cycle_target(backwards),
                Command::ConfirmTarget => self.confirm_target(),
                Command::CancelMode => self.cancel_targeting(),
                Command::UseAbility(slot) if slot == targeting.ability_slot => {
                    self.cancel_targeting()
                }
                _ => CommandOutcome::Rejected,
            };
            if outcome == CommandOutcome::Advanced
                && self.status == RunStatus::Active
                && self.enemy_turns_enabled
            {
                self.run_enemy_turns();
            }
            return outcome;
        }
        if let Some(inspecting) = self.inspecting {
            return match command {
                Command::MoveCursor(direction) | Command::Move(direction) => {
                    let (dx, dy) = direction.delta();
                    let cursor = inspecting.cursor.offset(dx, dy);
                    if self.map.tile(cursor).is_some() {
                        self.inspecting = Some(InspectState { cursor });
                    }
                    CommandOutcome::Rejected
                }
                Command::ToggleInspect | Command::CancelMode => {
                    self.inspecting = None;
                    CommandOutcome::Rejected
                }
                _ => CommandOutcome::Rejected,
            };
        }
        if self.using_item {
            let outcome = match command {
                Command::UseItemSlot(slot) => self.use_item_slot(slot),
                Command::ToggleItemUse | Command::CancelMode => {
                    self.using_item = false;
                    CommandOutcome::Rejected
                }
                _ => CommandOutcome::Rejected,
            };
            if outcome == CommandOutcome::Advanced && self.enemy_turns_enabled {
                self.run_enemy_turns();
            }
            return outcome;
        }
        let outcome = match command {
            Command::Move(direction) => self.move_player(direction),
            Command::Wait => {
                self.advance_turn(true);
                self.push_event("You wait and listen.");
                CommandOutcome::Advanced
            }
            Command::UseAbility(slot) => self.use_ability(slot),
            Command::ToggleInspect => {
                self.inspecting = Some(InspectState {
                    cursor: self.player.position,
                });
                CommandOutcome::Rejected
            }
            Command::PickupItem => self.pickup_item(),
            Command::ToggleItemUse => {
                self.using_item = true;
                CommandOutcome::Rejected
            }
            Command::MoveCursor(_)
            | Command::CycleTarget { .. }
            | Command::ConfirmTarget
            | Command::CancelMode
            | Command::UseItemSlot(_) => CommandOutcome::Rejected,
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
        self.player.mana = self
            .player
            .mana
            .saturating_add(self.class_definition().mana_regeneration)
            .clamp(0, self.player.max_mana);
        if tick_cooldown {
            for ability in self.abilities.iter_mut().flatten() {
                ability.cooldown_remaining = ability.cooldown_remaining.saturating_sub(1);
            }
        }
    }

    #[must_use]
    pub fn ability_state(&self, slot: AbilitySlot) -> Option<AbilityState> {
        self.abilities[slot.index()]
    }

    #[must_use]
    pub fn ability_definition(&self, id: AbilityId) -> Option<&AbilityDefinition> {
        self.catalog.ability(id)
    }

    #[must_use]
    pub fn can_afford_ability(&self, slot: AbilitySlot) -> bool {
        self.ability_state(slot)
            .and_then(|state| self.ability_definition(state.ability_id))
            .is_some_and(|definition| self.player.mana >= definition.mana_cost)
    }

    #[must_use]
    /// Return the selected class definition for this validated run.
    ///
    /// # Panics
    ///
    /// Panics only if externally supplied serialized state violates the catalog
    /// invariant established during construction.
    pub fn class_definition(&self) -> &ClassDefinition {
        self.catalog
            .class(self.class_id)
            .expect("run class remains present in its immutable catalog")
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
}

/// Structured failures produced by game-domain validation.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum GameError {
    #[error("the requested command is not valid in the current game state")]
    InvalidCommand,
    #[error("unknown player class {0:?}")]
    UnknownClass(ClassId),
    #[error(transparent)]
    Generation(#[from] GenerationError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(rows: &[&str]) -> ExplorationGame {
        ExplorationGame::from_map(
            Map::from_test_rows(rows, RunSeed(7)),
            Some(RunSeed(7)),
            RunStatus::Active,
            ClassId::GRAVE_KNIGHT,
            GameCatalog::builtin(),
        )
        .unwrap()
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
        assert_eq!(game.abilities[0].as_ref().unwrap().cooldown_remaining, 0);
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
        assert_eq!(
            game.apply(Command::UseAbility(AbilitySlot::CLEAVE)),
            CommandOutcome::Rejected
        );
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
            ["#####", "#@.>#", "##..#", "#..>#", "#####"],
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
    fn inspection_respects_fog_and_orders_live_facts_stably() {
        let mut game = game(&[
            "########################",
            "#@+...................>#",
            "########################",
        ]);
        let player = game.inspect(game.player.position);
        assert_eq!(player.visibility, InspectionVisibility::Visible);
        assert_eq!(player.entities[0].actor_id, ActorId(0));
        assert!(player.entities[0].description.contains("oath-bound"));

        let occupied = Position::new(2, 1);
        let skeleton = game.spawn_hostile(ActorKind::Skeleton, occupied, 10, 2, 3);
        let ghoul = game.spawn_hostile(ActorKind::Ghoul, occupied, 12, 1, 4);
        game.hostiles[0].telegraph = Some(Telegraph::CultistHex { target: occupied });
        game.hostiles[1].telegraph = Some(Telegraph::GhoulLunge {
            target: Position::new(4, 1),
        });
        let visible = game.inspect(occupied);
        assert_eq!(visible.visibility, InspectionVisibility::Visible);
        assert_eq!(
            visible
                .entities
                .iter()
                .map(|entity| entity.actor_id)
                .collect::<Vec<_>>(),
            [skeleton, ghoul]
        );
        assert_eq!(visible.markers.len(), 3);
        assert!(
            visible
                .entities
                .iter()
                .all(|entity| !entity.description.is_empty())
        );

        game.hostiles.push(Actor {
            id: ActorId(99),
            kind: ActorKind::Cultist,
            position: occupied,
            health: 8,
            max_health: 8,
            armor: 0,
            damage: 1,
            active: false,
            telegraph: None,
        });
        assert!(game.inspect(occupied).entities.iter().any(|entity| {
            entity.actor_id == ActorId(99) && entity.description.contains("grave-priest")
        }));

        game.hostiles.clear();
        for _ in 0..11 {
            game.apply(Command::Move(Direction::East));
        }
        let remembered = game.inspect(Position::new(2, 1));
        assert_eq!(remembered.visibility, InspectionVisibility::Remembered);
        assert!(remembered.terrain.is_some());
        assert!(remembered.entities.is_empty() && remembered.markers.is_empty());
        let unknown = game.inspect(Position::new(22, 1));
        assert_eq!(unknown.visibility, InspectionVisibility::Unknown);
        assert!(unknown.terrain.is_none());
        assert!(unknown.entities.is_empty() && unknown.markers.is_empty());
    }

    #[test]
    fn inspect_cursor_is_bounded_modal_and_entirely_turn_free() {
        let mut game = game(&["@+>", "..."]);
        let before = game.clone();
        assert_eq!(game.apply(Command::ToggleInspect), CommandOutcome::Rejected);
        assert_eq!(game.inspecting.unwrap().cursor, Position::new(0, 0));
        game.apply(Command::Move(Direction::NorthWest));
        assert_eq!(game.inspecting.unwrap().cursor, Position::new(0, 0));
        game.apply(Command::MoveCursor(Direction::East));
        assert_eq!(game.inspecting.unwrap().cursor, Position::new(1, 0));
        assert_eq!(game.turn, before.turn);
        assert_eq!(game.player, before.player);
        assert_eq!(game.hostiles, before.hostiles);
        assert_eq!(game.abilities, before.abilities);
        assert_eq!(game.events, before.events);
        assert_eq!(game.combat_roll_count, before.combat_roll_count);
        assert!(game.targeting.is_none());
        game.apply(Command::CancelMode);
        assert!(game.inspecting.is_none());
        game.status = RunStatus::Victory;
        game.apply(Command::ToggleInspect);
        assert!(game.inspecting.is_none());
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
    fn default_and_explicit_grave_knight_construction_are_identical() {
        let seed = RunSeed(0x0C1A_55E5);
        let default = ExplorationGame::new(Some(seed)).unwrap();
        let explicit = ExplorationGame::new_with_class(Some(seed), ClassId::GRAVE_KNIGHT).unwrap();
        assert_eq!(default, explicit);
        assert_eq!(default.player.max_health, PLAYER_MAX_HEALTH);
        assert_eq!(default.player.armor, PLAYER_ARMOR);
        assert_eq!(default.player.damage, PLAYER_DAMAGE);
        assert_eq!(
            default.ability_state(AbilitySlot::CLEAVE),
            Some(AbilityState {
                ability_id: AbilityId::CLEAVE,
                cooldown_remaining: 0
            })
        );
        let encoded = serde_json::to_string(&default).unwrap();
        assert_eq!(
            serde_json::from_str::<ExplorationGame>(&encoded).unwrap(),
            default
        );
        assert!(matches!(
            ExplorationGame::new_with_class(Some(seed), ClassId(999)),
            Err(GameError::UnknownClass(ClassId(999)))
        ));
    }

    #[test]
    fn alternate_catalog_class_uses_generic_construction_execution_and_restart() {
        let class_id = ClassId(99);
        let ability_id = AbilityId(77);
        let catalog = GameCatalog::new(
            vec![AbilityDefinition {
                id: ability_id,
                name: "Test Sweep".into(),
                description: "Sweep nearby foes.".into(),
                mana_cost: 1,
                cooldown_turns: 2,
                targeting: AbilityTargeting::Immediate,
                effect: AbilityEffect::Cleave,
                availability: AbilityAvailability::Class(class_id),
            }],
            vec![ClassDefinition {
                id: class_id,
                name: "Test Warden".into(),
                description: "A sentinel raised against the dark.".into(),
                max_health: 41,
                max_mana: 10,
                mana_regeneration: 1,
                armor: 3,
                base_damage: 8,
                starting_abilities: vec![AbilityBinding {
                    slot: 3,
                    ability_id,
                }],
            }],
        )
        .unwrap();
        let seed = RunSeed(0xA17C_1A55);
        let mut game = ExplorationGame::new_with_catalog(Some(seed), class_id, catalog).unwrap();
        game.start();
        assert_eq!(
            game.inspect(game.player.position).entities[0].description,
            "A sentinel raised against the dark."
        );
        assert_eq!(
            (
                game.player.max_health,
                game.player.armor,
                game.player.damage
            ),
            (41, 3, 8)
        );
        assert_eq!(game.class_id, class_id);
        assert_eq!(game.ability_state(AbilitySlot::CLEAVE), None);
        let slot = AbilitySlot::new(3).unwrap();
        assert_eq!(game.ability_state(slot).unwrap().ability_id, ability_id);

        game.hostiles.clear();
        let adjacent = DIRECTIONS
            .iter()
            .map(|direction| {
                let (dx, dy) = direction.delta();
                game.player.position.offset(dx, dy)
            })
            .find(|position| game.map.tile(*position).is_some_and(Tile::is_walkable))
            .expect("generated player start has an adjacent walkable tile");
        game.spawn_hostile(ActorKind::Skeleton, adjacent, 30, 0, 1);
        assert_eq!(
            game.apply(Command::UseAbility(slot)),
            CommandOutcome::Advanced
        );
        assert_eq!(game.ability_state(slot).unwrap().cooldown_remaining, 2);
        assert!(
            game.events
                .iter()
                .any(|event| event == "You unleash Test Sweep.")
        );
        assert!(
            game.events
                .iter()
                .any(|event| event.starts_with("Test Sweep hits Skeleton"))
        );
        game.restart_with_seed(RunSeed(1)).unwrap();
        assert_eq!(game.class_id, class_id);
        assert_eq!(
            (
                game.player.max_health,
                game.player.armor,
                game.player.damage
            ),
            (41, 3, 8)
        );
        assert_eq!(game.ability_state(slot).unwrap().cooldown_remaining, 0);
        game.damage_player(999, "the dark");
        assert_eq!(game.events.last().unwrap(), "The Test Warden falls.");
    }

    #[test]
    fn bump_attacks_use_seeded_variable_damage_and_hold_position() {
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
    fn combat_rolls_are_bounded_variable_and_count_only_successful_hits() {
        let mut outcomes = HashSet::new();
        for seed in 0..64 {
            let mut rolled = game(&["#####", "#@.>#", "#####"]);
            rolled.map.seed = RunSeed(seed);
            let damage = rolled.damage_player(3, "test");
            assert!((1..=5).contains(&damage));
            assert_eq!(rolled.combat_roll_count, 1);
            outcomes.insert(damage);
        }
        assert!(outcomes.len() > 1);

        let mut clamped = game(&["#####", "#@.>#", "#####"]);
        assert_eq!(clamped.roll_damage(0, 99), 1);
        assert_eq!(clamped.combat_roll_count, 1);

        let mut rejected = game(&["#####", "#@.>#", "#####"]);
        assert_eq!(
            rejected.apply(Command::UseAbility(AbilitySlot::CLEAVE)),
            CommandOutcome::Rejected
        );
        assert_eq!(
            rejected.apply(Command::Move(Direction::North)),
            CommandOutcome::Rejected
        );
        assert_eq!(rejected.combat_roll_count, 0);

        let mut extreme = game(&["#####", "#@.>#", "#####"]);
        assert!(extreme.roll_damage(i32::MAX, 0) > 0);
        assert_eq!(extreme.combat_roll_count, 1);
    }

    #[test]
    fn combat_stream_replays_and_restarts_without_changing_world_generation() {
        let seed = RunSeed(0xD4A6_E001);
        let mut left = ExplorationGame::new(Some(seed)).unwrap();
        let mut right = ExplorationGame::new(Some(seed)).unwrap();
        let original_map = left.map.clone();
        let original_hostiles = left.hostiles.clone();

        let left_rolls = (0..12).map(|_| left.roll_damage(6, 2)).collect::<Vec<_>>();
        let right_rolls = (0..12).map(|_| right.roll_damage(6, 2)).collect::<Vec<_>>();
        assert_eq!(left_rolls, right_rolls);
        assert_eq!(left.map, original_map);
        assert_eq!(left.hostiles, original_hostiles);

        left.restart_with_seed(RunSeed(999)).unwrap();
        assert_eq!(left.combat_roll_count, 0);
        assert_eq!(left.roll_damage(6, 2), left_rolls[0]);
    }

    #[test]
    fn cleave_hits_all_adjacent_targets_in_stable_id_order() {
        let mut game = game(&["#######", "#.....#", "#..@..#", "#....>#", "#######"]);
        let seed = (0..100)
            .find(|seed| {
                let mut probe = game.clone();
                probe.map.seed = RunSeed(*seed);
                probe.roll_damage(PLAYER_DAMAGE, 1) != probe.roll_damage(PLAYER_DAMAGE, 1)
            })
            .expect("representative seeds contain unequal consecutive rolls");
        game.map.seed = RunSeed(seed);
        let mut expected = game.clone();
        let first_damage = expected.roll_damage(PLAYER_DAMAGE, 1);
        let second_damage = expected.roll_damage(PLAYER_DAMAGE, 1);
        assert_ne!(first_damage, second_damage);

        let first = game.spawn_hostile(ActorKind::Skeleton, Position::new(2, 1), 20, 1, 3);
        let second = game.spawn_hostile(ActorKind::Ghoul, Position::new(4, 2), 20, 1, 3);
        let distant = game.spawn_hostile(ActorKind::Cultist, Position::new(1, 1), 20, 1, 3);
        game.hostiles.reverse();

        assert_eq!(
            game.apply(Command::UseAbility(AbilitySlot::CLEAVE)),
            CommandOutcome::Advanced
        );
        assert_eq!(game.turn, 1);
        assert_eq!(
            game.abilities[0].as_ref().unwrap().cooldown_remaining,
            CLEAVE_COOLDOWN_TURNS
        );
        assert_eq!(
            game.hostiles.iter().find(|a| a.id == first).unwrap().health,
            20 - first_damage
        );
        assert_eq!(
            game.hostiles
                .iter()
                .find(|a| a.id == second)
                .unwrap()
                .health,
            20 - second_damage
        );
        assert_eq!(
            game.hostiles
                .iter()
                .find(|a| a.id == distant)
                .unwrap()
                .health,
            20
        );
        assert_eq!(game.combat_roll_count, 2);
        assert_eq!(
            game.events[game.events.len() - 2],
            format!("Cleave hits Skeleton 1 for {first_damage} damage.")
        );
        assert_eq!(
            game.events[game.events.len() - 1],
            format!("Cleave hits Ghoul 2 for {second_damage} damage.")
        );
    }

    #[test]
    fn cleave_rejections_are_free_and_four_later_turns_restore_it() {
        let mut game = game(&["######", "#@..>#", "######"]);
        assert_eq!(
            game.apply(Command::UseAbility(AbilitySlot::CLEAVE)),
            CommandOutcome::Rejected
        );
        assert_eq!(game.turn, 0);
        game.spawn_hostile(ActorKind::Skeleton, Position::new(2, 1), 30, 0, 3);
        assert_eq!(
            game.apply(Command::UseAbility(AbilitySlot::CLEAVE)),
            CommandOutcome::Advanced
        );
        assert_eq!(game.abilities[0].as_ref().unwrap().cooldown_remaining, 4);
        assert_eq!(
            game.apply(Command::UseAbility(AbilitySlot::CLEAVE)),
            CommandOutcome::Rejected
        );
        assert_eq!(game.turn, 1);
        for expected in [3, 2, 1, 0] {
            game.apply(Command::Wait);
            assert_eq!(
                game.abilities[0].as_ref().unwrap().cooldown_remaining,
                expected
            );
        }
        assert_eq!(game.turn, 5);
        assert_eq!(
            game.apply(Command::UseAbility(AbilitySlot::CLEAVE)),
            CommandOutcome::Advanced
        );
    }

    #[test]
    fn grave_bolt_targeting_is_deterministic_free_until_confirmed_and_serializable() {
        let mut game = game(&["##########", "#@......>#", "#........#", "##########"]);
        let farther = game.spawn_hostile(ActorKind::Ghoul, Position::new(5, 1), 20, 0, 1);
        let nearer_high_id = game.spawn_hostile(ActorKind::Cultist, Position::new(3, 2), 20, 0, 1);
        let nearer_low_id = game.spawn_hostile(ActorKind::Skeleton, Position::new(3, 1), 20, 0, 1);
        game.hostiles.reverse();
        let slot = AbilitySlot::new(2).unwrap();

        assert_eq!(
            game.apply(Command::UseAbility(slot)),
            CommandOutcome::Rejected
        );
        assert_eq!(game.turn, 0);
        assert_eq!(game.combat_roll_count, 0);
        assert_eq!(game.targeting.unwrap().selected_actor, Some(nearer_high_id));
        let encoded = serde_json::to_string(&game).unwrap();
        assert_eq!(
            serde_json::from_str::<ExplorationGame>(&encoded).unwrap(),
            game
        );

        assert_eq!(
            game.apply(Command::CycleTarget { backwards: false }),
            CommandOutcome::Rejected
        );
        assert_eq!(game.targeting.unwrap().selected_actor, Some(nearer_low_id));
        assert_eq!(game.apply(Command::ConfirmTarget), CommandOutcome::Advanced);
        assert!(game.targeting.is_none());
        assert_eq!(game.turn, 1);
        assert_eq!(game.combat_roll_count, 1);
        assert_eq!(game.ability_state(slot).unwrap().cooldown_remaining, 3);
        assert!(
            game.hostiles
                .iter()
                .find(|actor| actor.id == farther)
                .is_some()
        );
        assert!(
            game.events
                .iter()
                .any(|event| event.starts_with("Grave Bolt strikes Skeleton"))
        );
    }

    #[test]
    fn target_validation_cursor_bounds_cancel_and_invalid_confirm_are_free() {
        let mut game = game(&["##########", "#@.#....>#", "#........#", "##########"]);
        let slot = AbilitySlot::new(2).unwrap();
        let blocked = game.spawn_hostile(ActorKind::Skeleton, Position::new(5, 1), 20, 0, 1);
        let out_of_range = game.spawn_hostile(ActorKind::Ghoul, Position::new(8, 2), 20, 0, 1);
        assert_eq!(
            game.target_validity(slot, Position::new(5, 1)),
            TargetValidity::Blocked
        );
        assert_eq!(
            game.target_validity(slot, Position::new(8, 2)),
            TargetValidity::OutOfRange
        );
        assert_eq!(
            game.apply(Command::UseAbility(slot)),
            CommandOutcome::Rejected
        );
        assert!(game.targeting.is_none());
        assert_eq!(game.combat_roll_count, 0);

        game.hostiles
            .retain(|actor| actor.id != blocked && actor.id != out_of_range);
        game.spawn_hostile(ActorKind::Cultist, Position::new(2, 2), 20, 0, 1);
        game.apply(Command::UseAbility(slot));
        game.targeting.as_mut().unwrap().cursor = Position::new(0, 0);
        game.apply(Command::MoveCursor(Direction::NorthWest));
        assert_eq!(game.targeting.unwrap().cursor, Position::new(0, 0));
        game.targeting.as_mut().unwrap().selected_actor = None;
        assert_eq!(game.apply(Command::ConfirmTarget), CommandOutcome::Rejected);
        assert!(game.targeting.is_some());
        assert_eq!(game.turn, 0);
        assert_eq!(game.combat_roll_count, 0);
        game.apply(Command::UseAbility(slot));
        assert!(game.targeting.is_none());
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
        game.abilities[0].as_mut().unwrap().cooldown_remaining = 3;
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
        game.abilities[0].as_mut().unwrap().cooldown_remaining = 2;
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

    #[test]
    fn mana_costs_regenerate_only_on_advanced_turns_and_restart_full() {
        let mut game = game(&["#####", "#@.>#", "#####"]);
        game.spawn_hostile(ActorKind::Skeleton, Position::new(2, 1), 10, 1, 5);
        assert_eq!((game.player.mana, game.player.max_mana), (10, 10));
        game.player.mana = 5;
        assert_eq!(
            game.apply(Command::UseAbility(AbilitySlot::CLEAVE)),
            CommandOutcome::Advanced
        );
        assert_eq!(game.player.mana, 3);
        let turn = game.turn;
        assert_eq!(
            game.apply(Command::UseAbility(AbilitySlot::CLEAVE)),
            CommandOutcome::Rejected
        );
        assert_eq!((game.player.mana, game.turn), (3, turn));
        assert_eq!(game.apply(Command::Wait), CommandOutcome::Advanced);
        assert_eq!(game.player.mana, 4);
        game.player.mana = 0;
        game.abilities[0].as_mut().unwrap().cooldown_remaining = 0;
        assert_eq!(
            game.apply(Command::UseAbility(AbilitySlot::CLEAVE)),
            CommandOutcome::Rejected
        );
        assert_eq!((game.player.mana, game.turn), (0, turn + 1));

        let seed = game.seed();
        game.restart_with_seed(seed).unwrap();
        assert_eq!(game.player.mana, game.player.max_mana);
    }
}
