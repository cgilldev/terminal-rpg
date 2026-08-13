//! Validated item definitions, deterministic placement, inventory, and effects.

use super::{CommandOutcome, ExplorationGame};
use crate::world::{Position, Tile};
use rand::{SeedableRng, seq::SliceRandom};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

pub const INVENTORY_SLOT_COUNT: usize = 4;
pub const TORCH_VISION_RADIUS: i32 = 12;
pub const POTION_HEAL: i32 = 12;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ItemId(pub u16);

impl ItemId {
    pub const TORCH: Self = Self(1);
    pub const HEALTH_POTION: Self = Self(2);
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ItemInstanceId(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ItemEffect {
    TorchVision { radius: i32 },
    Heal { amount: i32 },
    Unsupported(u16),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ItemDefinition {
    pub id: ItemId,
    pub name: String,
    pub description: String,
    pub effect: ItemEffect,
    pub glyph_ascii: char,
    pub glyph_unicode: char,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ItemInstance {
    pub instance_id: ItemInstanceId,
    pub item_id: ItemId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GroundItem {
    pub item: ItemInstance,
    pub position: Position,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ItemCatalog {
    definitions: Vec<ItemDefinition>,
}

impl ItemCatalog {
    /// Validate a complete immutable item catalog.
    ///
    /// # Errors
    ///
    /// Returns duplicate-ID or unsupported-definition errors.
    pub fn new(definitions: Vec<ItemDefinition>) -> Result<Self, ItemCatalogError> {
        let mut ids = HashSet::new();
        for definition in &definitions {
            if !ids.insert(definition.id) {
                return Err(ItemCatalogError::DuplicateId(definition.id));
            }
            if definition.name.is_empty()
                || definition.description.is_empty()
                || matches!(definition.effect, ItemEffect::Unsupported(_))
                || matches!(definition.effect, ItemEffect::TorchVision { radius } if radius <= 0)
                || matches!(definition.effect, ItemEffect::Heal { amount } if amount <= 0)
            {
                return Err(ItemCatalogError::UnsupportedDefinition(definition.id));
            }
        }
        Ok(Self { definitions })
    }

    #[must_use]
    /// Return the built-in item catalog.
    ///
    /// # Panics
    ///
    /// Panics only when programmer-authored built-in definitions are invalid.
    pub fn builtin() -> Self {
        Self::new(vec![
            ItemDefinition {
                id: ItemId::TORCH,
                name: "Grave Torch".into(),
                description: "A pitch-black brand whose corpse-flame pushes back the dark.".into(),
                glyph_ascii: 't',
                glyph_unicode: '†',
                effect: ItemEffect::TorchVision {
                    radius: TORCH_VISION_RADIUS,
                },
            },
            ItemDefinition {
                id: ItemId::HEALTH_POTION,
                name: "Health Potion".into(),
                description: "A bitter crimson draught that knits wounded flesh.".into(),
                glyph_ascii: 'v',
                glyph_unicode: '¡',
                effect: ItemEffect::Heal {
                    amount: POTION_HEAL,
                },
            },
        ])
        .expect("built-in items are valid")
    }

    #[must_use]
    pub fn item(&self, id: ItemId) -> Option<&ItemDefinition> {
        self.definitions
            .iter()
            .find(|definition| definition.id == id)
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ItemCatalogError {
    #[error("duplicate item id {0:?}")]
    DuplicateId(ItemId),
    #[error("item {0:?} uses unsupported metadata")]
    UnsupportedDefinition(ItemId),
    #[error("item instance {instance_id:?} references unknown item {item_id:?}")]
    UnknownItem {
        instance_id: ItemInstanceId,
        item_id: ItemId,
    },
}

impl ExplorationGame {
    /// Validate every serialized item reference against the attached catalog.
    ///
    /// # Errors
    ///
    /// Returns the first stable instance ID that references an unknown item.
    pub fn validate_item_references(&self) -> Result<(), ItemCatalogError> {
        for item in self
            .inventory
            .iter()
            .flatten()
            .chain(self.ground_items.iter().map(|ground| &ground.item))
        {
            if self.item_definition(item.item_id).is_none() {
                return Err(ItemCatalogError::UnknownItem {
                    instance_id: item.instance_id,
                    item_id: item.item_id,
                });
            }
        }
        Ok(())
    }

    pub(super) fn populate_items(&mut self) {
        let mut positions = self
            .map
            .spawn_candidates
            .iter()
            .copied()
            .filter(|position| {
                self.map.tile(*position) == Some(Tile::Floor)
                    && *position != self.player.position
                    && *position != self.map.exit
                    && !self
                        .hostiles
                        .iter()
                        .any(|actor| actor.position == *position)
            })
            .collect::<Vec<_>>();
        let mut rng = ChaCha8Rng::seed_from_u64(self.map.seed.0 ^ 0x17E0_5EED_1A7E_DA7A);
        positions.shuffle(&mut rng);
        for (item_id, position) in [ItemId::TORCH, ItemId::HEALTH_POTION]
            .into_iter()
            .zip(positions)
        {
            self.spawn_ground_item(item_id, position);
        }
    }

    pub fn spawn_ground_item(&mut self, item_id: ItemId, position: Position) -> ItemInstanceId {
        let instance_id = ItemInstanceId(self.next_item_instance_id);
        self.next_item_instance_id += 1;
        self.ground_items.push(GroundItem {
            item: ItemInstance {
                instance_id,
                item_id,
            },
            position,
        });
        instance_id
    }

    #[must_use]
    pub fn item_definition(&self, id: ItemId) -> Option<&ItemDefinition> {
        self.item_catalog.item(id)
    }

    #[must_use]
    pub fn effective_visibility_radius(&self) -> i32 {
        self.inventory
            .iter()
            .flatten()
            .filter_map(|item| self.item_definition(item.item_id))
            .filter_map(|definition| match definition.effect {
                ItemEffect::TorchVision { radius } => Some(radius),
                _ => None,
            })
            .max()
            .unwrap_or(super::FIELD_OF_VIEW_RADIUS)
    }

    pub(super) fn pickup_item(&mut self) -> CommandOutcome {
        let Some(ground_index) = self
            .ground_items
            .iter()
            .position(|ground| ground.position == self.player.position)
        else {
            self.push_event("There is nothing here to take.");
            return CommandOutcome::Rejected;
        };
        let Some(slot) = self.inventory.iter().position(Option::is_none) else {
            self.push_event("Your four item slots are full.");
            return CommandOutcome::Rejected;
        };
        let ground = self.ground_items.remove(ground_index);
        let name = self
            .item_definition(ground.item.item_id)
            .expect("ground items use catalog definitions")
            .name
            .clone();
        self.inventory[slot] = Some(ground.item);
        self.recompute_visibility();
        self.advance_turn(true);
        self.push_event(format!("You take the {name} into slot {}.", slot + 1));
        CommandOutcome::Advanced
    }

    pub(super) fn use_item_slot(&mut self, slot: u8) -> CommandOutcome {
        let Some(index) = slot
            .checked_sub(1)
            .map(usize::from)
            .filter(|index| *index < INVENTORY_SLOT_COUNT)
        else {
            return CommandOutcome::Rejected;
        };
        let Some(item) = self.inventory[index] else {
            return CommandOutcome::Rejected;
        };
        let definition = self
            .item_definition(item.item_id)
            .expect("inventory items use catalog definitions");
        let (name, amount) = match definition.effect {
            ItemEffect::Heal { amount } if self.player.health < self.player.max_health => {
                (definition.name.clone(), amount)
            }
            ItemEffect::Heal { .. }
            | ItemEffect::TorchVision { .. }
            | ItemEffect::Unsupported(_) => {
                return CommandOutcome::Rejected;
            }
        };
        let before = self.player.health;
        self.player.health = (self.player.health + amount).min(self.player.max_health);
        let restored = self.player.health - before;
        self.inventory[index] = None;
        self.using_item = false;
        self.recompute_visibility();
        self.advance_turn(true);
        self.push_event(format!(
            "You drink the {name} and restore {restored} health."
        ));
        CommandOutcome::Advanced
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Command, GameCatalog, RunStatus};
    use crate::world::{Map, RunSeed};

    fn game(rows: &[&str]) -> ExplorationGame {
        ExplorationGame::from_map(
            Map::from_test_rows(rows, RunSeed(7)),
            Some(RunSeed(7)),
            RunStatus::Active,
            crate::game::ClassId::GRAVE_KNIGHT,
            GameCatalog::builtin(),
        )
        .unwrap()
    }

    #[test]
    fn item_catalog_validates_ids_effects_and_serializes() {
        let catalog = ItemCatalog::builtin();
        assert_eq!(catalog.item(ItemId::TORCH).unwrap().name, "Grave Torch");
        assert_eq!(
            catalog.item(ItemId::HEALTH_POTION).unwrap().effect,
            ItemEffect::Heal { amount: 12 }
        );
        let encoded = serde_json::to_string(&catalog).unwrap();
        assert_eq!(
            serde_json::from_str::<ItemCatalog>(&encoded).unwrap(),
            catalog
        );
        let duplicate = catalog.definitions[0].clone();
        assert_eq!(
            ItemCatalog::new(vec![duplicate.clone(), duplicate]),
            Err(ItemCatalogError::DuplicateId(ItemId::TORCH))
        );
        assert!(matches!(
            ItemCatalog::new(vec![ItemDefinition {
                id: ItemId(9),
                name: "Broken".into(),
                description: "Bad".into(),
                glyph_ascii: '?',
                glyph_unicode: '?',
                effect: ItemEffect::Unsupported(1),
            }]),
            Err(ItemCatalogError::UnsupportedDefinition(ItemId(9)))
        ));
    }

    #[test]
    fn serialized_item_references_are_explicitly_validated() {
        let mut game = ExplorationGame::new(Some(RunSeed(5))).unwrap();
        game.inventory[0] = Some(ItemInstance {
            instance_id: ItemInstanceId(77),
            item_id: ItemId(999),
        });
        let encoded = serde_json::to_string(&game).unwrap();
        let decoded = serde_json::from_str::<ExplorationGame>(&encoded).unwrap();
        assert_eq!(
            decoded.validate_item_references(),
            Err(ItemCatalogError::UnknownItem {
                instance_id: ItemInstanceId(77),
                item_id: ItemId(999),
            })
        );
    }

    #[test]
    fn generated_items_are_deterministic_distinct_and_legal() {
        for seed in 0..100 {
            let left = ExplorationGame::new(Some(RunSeed(seed))).unwrap();
            let right = ExplorationGame::new(Some(RunSeed(seed))).unwrap();
            assert_eq!(left.ground_items, right.ground_items);
            assert_eq!(left.ground_items.len(), 2);
            assert_ne!(left.ground_items[0].position, left.ground_items[1].position);
            assert_eq!(left.ground_items[0].item.item_id, ItemId::TORCH);
            assert_eq!(left.ground_items[1].item.item_id, ItemId::HEALTH_POTION);
            for ground in &left.ground_items {
                assert_eq!(left.map.tile(ground.position), Some(Tile::Floor));
                assert_ne!(ground.position, left.player.position);
                assert_ne!(ground.position, left.map.exit);
                assert!(
                    !left
                        .hostiles
                        .iter()
                        .any(|actor| actor.position == ground.position)
                );
            }
        }
    }

    #[test]
    fn pickup_uses_lowest_slot_costs_one_turn_and_rejects_failures() {
        let mut game = game(&["#####", "#@.>#", "#####"]);
        game.spawn_ground_item(ItemId::TORCH, game.player.position);
        assert_eq!(game.apply(Command::PickupItem), CommandOutcome::Advanced);
        assert_eq!(game.turn, 1);
        assert_eq!(game.inventory[0].unwrap().item_id, ItemId::TORCH);
        assert!(game.ground_items.is_empty());
        let turn = game.turn;
        assert_eq!(game.apply(Command::PickupItem), CommandOutcome::Rejected);
        assert_eq!(game.turn, turn);

        for slot in &mut game.inventory {
            *slot = Some(ItemInstance {
                instance_id: ItemInstanceId(99),
                item_id: ItemId::TORCH,
            });
        }
        game.spawn_ground_item(ItemId::HEALTH_POTION, game.player.position);
        assert_eq!(game.apply(Command::PickupItem), CommandOutcome::Rejected);
        assert_eq!(game.turn, turn);
        assert_eq!(game.ground_items.len(), 1);
    }

    #[test]
    fn torch_extends_occluded_visibility_without_stacking() {
        let mut game = game(&[
            "##################",
            "#@..............>#",
            "##################",
        ]);
        assert_eq!(game.effective_visibility_radius(), 8);
        assert!(!game.is_visible(Position::new(10, 1)));
        let torch = ItemInstance {
            instance_id: ItemInstanceId(1),
            item_id: ItemId::TORCH,
        };
        game.inventory[0] = Some(torch);
        game.inventory[1] = Some(ItemInstance {
            instance_id: ItemInstanceId(2),
            item_id: ItemId::TORCH,
        });
        game.recompute_visibility();
        assert_eq!(game.effective_visibility_radius(), 12);
        assert!(game.is_visible(Position::new(12, 1)));
        assert!(!game.is_visible(Position::new(14, 1)));
    }

    #[test]
    fn potion_heals_caps_consumes_and_only_success_advances() {
        let mut game = game(&["#####", "#@.>#", "#####"]);
        let potion = ItemInstance {
            instance_id: ItemInstanceId(1),
            item_id: ItemId::HEALTH_POTION,
        };
        game.inventory[1] = Some(potion);
        game.using_item = true;
        let before = game.clone();
        assert_eq!(
            game.apply(Command::UseItemSlot(2)),
            CommandOutcome::Rejected
        );
        assert_eq!(game, before);
        game.player.health -= 5;
        assert_eq!(
            game.apply(Command::UseItemSlot(2)),
            CommandOutcome::Advanced
        );
        assert_eq!(game.player.health, game.player.max_health);
        assert_eq!(game.turn, 1);
        assert!(game.inventory[1].is_none());
        assert!(!game.using_item);
        assert!(game.events.last().unwrap().contains("restore 5 health"));
    }

    #[test]
    fn item_mode_is_exclusive_and_seeded_restart_restores_placement() {
        let seed = RunSeed(81);
        let mut game = ExplorationGame::new(Some(seed)).unwrap();
        game.start();
        let initial_ground = game.ground_items.clone();
        game.apply(Command::ToggleItemUse);
        assert!(game.using_item);
        game.apply(Command::ToggleInspect);
        game.apply(Command::UseAbility(crate::game::AbilitySlot::CLEAVE));
        assert!(game.using_item && game.inspecting.is_none() && game.targeting.is_none());
        game.apply(Command::CancelMode);
        game.inventory[0] = Some(ItemInstance {
            instance_id: ItemInstanceId(88),
            item_id: ItemId::TORCH,
        });
        game.restart_with_seed(RunSeed(999)).unwrap();
        assert_eq!(game.ground_items, initial_ground);
        assert_eq!(game.inventory, [None; INVENTORY_SLOT_COUNT]);
        assert!(!game.using_item);
        assert_eq!(
            game.effective_visibility_radius(),
            super::super::FIELD_OF_VIEW_RADIUS
        );
    }

    #[test]
    fn player_inspection_includes_carried_definitions_but_other_tiles_do_not() {
        let mut game = game(&["#####", "#@.>#", "#####"]);
        game.inventory[0] = Some(ItemInstance {
            instance_id: ItemInstanceId(42),
            item_id: ItemId::TORCH,
        });
        let player = game.inspect(game.player.position);
        assert_eq!(player.carried_items.len(), 1);
        assert_eq!(player.carried_items[0].name, "Carried: Grave Torch");
        assert!(game.inspect(Position::new(2, 1)).carried_items.is_empty());
    }
}
