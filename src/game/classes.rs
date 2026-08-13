//! Validated player-class and ability catalogs.

use super::{
    ABILITY_SLOT_COUNT, AbilityAvailability, AbilityDefinition, AbilityEffect, AbilityId,
    AbilitySlot, AbilityState, AbilityTargeting,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ClassId(pub u16);

impl ClassId {
    pub const GRAVE_KNIGHT: Self = Self(1);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbilityBinding {
    pub slot: u8,
    pub ability_id: AbilityId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClassDefinition {
    pub id: ClassId,
    pub name: String,
    pub description: String,
    pub max_health: i32,
    pub max_mana: i32,
    pub mana_regeneration: i32,
    pub armor: i32,
    pub base_damage: i32,
    pub starting_abilities: Vec<AbilityBinding>,
}

impl ClassDefinition {
    /// Materialize the validated ten-slot starting loadout.
    #[must_use]
    pub fn loadout(&self) -> [Option<AbilityId>; ABILITY_SLOT_COUNT] {
        let mut loadout = [None; ABILITY_SLOT_COUNT];
        for binding in &self.starting_abilities {
            if let Some(slot) = AbilitySlot::new(binding.slot) {
                loadout[slot.index()] = Some(binding.ability_id);
            }
        }
        loadout
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GameCatalog {
    abilities: Vec<AbilityDefinition>,
    classes: Vec<ClassDefinition>,
}

impl GameCatalog {
    /// Validate definitions before they can be attached to a run.
    ///
    /// # Errors
    ///
    /// Returns a precise catalog error for duplicate IDs, bad slots, invalid
    /// references, unavailable abilities, or unsupported metadata.
    pub fn new(
        abilities: Vec<AbilityDefinition>,
        classes: Vec<ClassDefinition>,
    ) -> Result<Self, CatalogError> {
        let mut ability_ids = HashSet::new();
        for ability in &abilities {
            if !ability_ids.insert(ability.id) {
                return Err(CatalogError::DuplicateAbility(ability.id));
            }
            if ability.description.is_empty()
                || matches!(ability.targeting, AbilityTargeting::Unsupported(_))
                || matches!(ability.effect, AbilityEffect::Unsupported(_))
                || matches!(ability.availability, AbilityAvailability::Unsupported(_))
                || matches!(ability.targeting, AbilityTargeting::HostileSingle { range, .. } if range <= 0)
                || matches!(ability.effect, AbilityEffect::GraveBolt { base_damage } if base_damage <= 0)
                || ability.mana_cost < 0
            {
                return Err(CatalogError::UnsupportedAbility(ability.id));
            }
        }

        let mut class_ids = HashSet::new();
        for class in &classes {
            if !class_ids.insert(class.id) {
                return Err(CatalogError::DuplicateClass(class.id));
            }
            if class.max_health <= 0
                || class.max_mana < 0
                || class.mana_regeneration < 0
                || class.armor < 0
                || class.base_damage <= 0
            {
                return Err(CatalogError::InvalidClassStats {
                    class_id: class.id,
                    max_health: class.max_health,
                    max_mana: class.max_mana,
                    mana_regeneration: class.mana_regeneration,
                    armor: class.armor,
                    base_damage: class.base_damage,
                });
            }
            let mut slots = HashSet::new();
            for binding in &class.starting_abilities {
                if AbilitySlot::new(binding.slot).is_none() {
                    return Err(CatalogError::InvalidSlot {
                        class_id: class.id,
                        slot: binding.slot,
                    });
                }
                if !slots.insert(binding.slot) {
                    return Err(CatalogError::DuplicateSlot {
                        class_id: class.id,
                        slot: binding.slot,
                    });
                }
                let Some(ability) = abilities
                    .iter()
                    .find(|ability| ability.id == binding.ability_id)
                else {
                    return Err(CatalogError::UnknownAbility {
                        class_id: class.id,
                        ability_id: binding.ability_id,
                    });
                };
                if ability.availability != AbilityAvailability::Class(class.id) {
                    return Err(CatalogError::UnavailableAbility {
                        class_id: class.id,
                        ability_id: binding.ability_id,
                    });
                }
                if ability.mana_cost > class.max_mana {
                    return Err(CatalogError::AbilityCostExceedsClassMana {
                        class_id: class.id,
                        ability_id: ability.id,
                        mana_cost: ability.mana_cost,
                        max_mana: class.max_mana,
                    });
                }
            }
        }
        for ability in &abilities {
            if let AbilityAvailability::Class(class_id) = ability.availability
                && !class_ids.contains(&class_id)
            {
                return Err(CatalogError::UnknownAvailabilityClass {
                    ability_id: ability.id,
                    class_id,
                });
            }
        }
        Ok(Self { abilities, classes })
    }

    #[must_use]
    /// Return the built-in, statically valid prototype content.
    ///
    /// # Panics
    ///
    /// Panics only if a programmer makes the built-in definitions inconsistent.
    pub fn builtin() -> Self {
        Self::new(
            vec![
                AbilityDefinition {
                    id: AbilityId::CLEAVE,
                    name: "Cleave".into(),
                    description: "Strike every adjacent foe.".into(),
                    mana_cost: 3,
                    cooldown_turns: super::CLEAVE_COOLDOWN_TURNS,
                    targeting: AbilityTargeting::Immediate,
                    effect: AbilityEffect::Cleave,
                    availability: AbilityAvailability::Class(ClassId::GRAVE_KNIGHT),
                },
                AbilityDefinition {
                    id: AbilityId(2),
                    name: "Grave Bolt".into(),
                    description: "Hurl a bolt through unobstructed darkness.".into(),
                    mana_cost: 4,
                    cooldown_turns: 3,
                    targeting: AbilityTargeting::HostileSingle {
                        range: 6,
                        line_of_sight: true,
                    },
                    effect: AbilityEffect::GraveBolt { base_damage: 8 },
                    availability: AbilityAvailability::Class(ClassId::GRAVE_KNIGHT),
                },
            ],
            vec![ClassDefinition {
                id: ClassId::GRAVE_KNIGHT,
                name: "Grave Knight".into(),
                description: "An oath-bound warrior armored against the grave.".into(),
                max_health: super::PLAYER_MAX_HEALTH,
                max_mana: 10,
                mana_regeneration: 1,
                armor: super::PLAYER_ARMOR,
                base_damage: super::PLAYER_DAMAGE,
                starting_abilities: vec![
                    AbilityBinding {
                        slot: 1,
                        ability_id: AbilityId::CLEAVE,
                    },
                    AbilityBinding {
                        slot: 2,
                        ability_id: AbilityId(2),
                    },
                ],
            }],
        )
        .expect("built-in class and ability catalog is valid")
    }

    #[must_use]
    pub fn ability(&self, id: AbilityId) -> Option<&AbilityDefinition> {
        self.abilities.iter().find(|ability| ability.id == id)
    }

    #[must_use]
    pub fn class(&self, id: ClassId) -> Option<&ClassDefinition> {
        self.classes.iter().find(|class| class.id == id)
    }

    pub(super) fn initial_ability_state(
        &self,
        class_id: ClassId,
    ) -> Option<[Option<AbilityState>; ABILITY_SLOT_COUNT]> {
        self.class(class_id).map(|class| {
            class.loadout().map(|ability_id| {
                ability_id.map(|ability_id| AbilityState {
                    ability_id,
                    cooldown_remaining: 0,
                })
            })
        })
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CatalogError {
    #[error("duplicate ability id {0:?}")]
    DuplicateAbility(AbilityId),
    #[error("duplicate class id {0:?}")]
    DuplicateClass(ClassId),
    #[error(
        "class {class_id:?} has invalid stats: health {max_health}, mana {max_mana}, regeneration {mana_regeneration}, armor {armor}, damage {base_damage}"
    )]
    InvalidClassStats {
        class_id: ClassId,
        max_health: i32,
        max_mana: i32,
        mana_regeneration: i32,
        armor: i32,
        base_damage: i32,
    },
    #[error(
        "ability {ability_id:?} costs {mana_cost} mana but class {class_id:?} has maximum {max_mana}"
    )]
    AbilityCostExceedsClassMana {
        class_id: ClassId,
        ability_id: AbilityId,
        mana_cost: i32,
        max_mana: i32,
    },
    #[error("class {class_id:?} has invalid slot {slot}")]
    InvalidSlot { class_id: ClassId, slot: u8 },
    #[error("class {class_id:?} binds slot {slot} more than once")]
    DuplicateSlot { class_id: ClassId, slot: u8 },
    #[error("class {class_id:?} references unknown ability {ability_id:?}")]
    UnknownAbility {
        class_id: ClassId,
        ability_id: AbilityId,
    },
    #[error("ability {ability_id:?} is not available to class {class_id:?}")]
    UnavailableAbility {
        class_id: ClassId,
        ability_id: AbilityId,
    },
    #[error("ability {ability_id:?} references unknown class {class_id:?}")]
    UnknownAvailabilityClass {
        ability_id: AbilityId,
        class_id: ClassId,
    },
    #[error("ability {0:?} uses unsupported metadata")]
    UnsupportedAbility(AbilityId),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ability(id: u16, class_id: ClassId) -> AbilityDefinition {
        AbilityDefinition {
            id: AbilityId(id),
            name: format!("Ability {id}"),
            description: format!("Test ability {id}"),
            mana_cost: 1,
            cooldown_turns: 2,
            targeting: AbilityTargeting::Immediate,
            effect: AbilityEffect::Cleave,
            availability: AbilityAvailability::Class(class_id),
        }
    }

    fn class(id: u16, bindings: &[(u8, u16)]) -> ClassDefinition {
        ClassDefinition {
            id: ClassId(id),
            name: format!("Class {id}"),
            description: format!("Description for class {id}"),
            max_health: 20,
            max_mana: 10,
            mana_regeneration: 1,
            armor: 1,
            base_damage: 4,
            starting_abilities: bindings
                .iter()
                .map(|(slot, ability)| AbilityBinding {
                    slot: *slot,
                    ability_id: AbilityId(*ability),
                })
                .collect(),
        }
    }

    #[test]
    fn catalogs_validate_multiple_classes_loadouts_and_serialization() {
        let catalog = GameCatalog::new(
            vec![ability(1, ClassId(1)), ability(2, ClassId(2))],
            vec![class(1, &[(1, 1)]), class(2, &[(10, 2)])],
        )
        .unwrap();
        assert_eq!(
            catalog.class(ClassId(1)).unwrap().loadout()[0],
            Some(AbilityId(1))
        );
        assert_eq!(
            catalog.class(ClassId(2)).unwrap().loadout()[9],
            Some(AbilityId(2))
        );
        assert_eq!(catalog.class(ClassId(2)).unwrap().loadout()[0], None);
        let encoded = serde_json::to_string(&catalog).unwrap();
        assert_eq!(
            serde_json::from_str::<GameCatalog>(&encoded).unwrap(),
            catalog
        );
    }

    #[test]
    fn catalogs_reject_duplicate_invalid_and_unsupported_definitions() {
        assert_eq!(
            GameCatalog::new(
                vec![ability(1, ClassId(1)), ability(1, ClassId(1))],
                vec![class(1, &[(1, 1)])]
            ),
            Err(CatalogError::DuplicateAbility(AbilityId(1)))
        );
        assert_eq!(
            GameCatalog::new(
                vec![ability(1, ClassId(1))],
                vec![class(1, &[]), class(1, &[])]
            ),
            Err(CatalogError::DuplicateClass(ClassId(1)))
        );
        let mut invalid_stats = class(1, &[(1, 1)]);
        invalid_stats.max_health = 0;
        assert_eq!(
            GameCatalog::new(vec![ability(1, ClassId(1))], vec![invalid_stats]),
            Err(CatalogError::InvalidClassStats {
                class_id: ClassId(1),
                max_health: 0,
                max_mana: 10,
                mana_regeneration: 1,
                armor: 1,
                base_damage: 4
            })
        );
        let mut extreme_stats = class(1, &[(1, 1)]);
        extreme_stats.max_health = i32::MAX;
        extreme_stats.armor = i32::MAX;
        extreme_stats.base_damage = i32::MAX;
        assert!(GameCatalog::new(vec![ability(1, ClassId(1))], vec![extreme_stats]).is_ok());
        assert_eq!(
            GameCatalog::new(vec![ability(1, ClassId(1))], vec![class(1, &[(0, 1)])]),
            Err(CatalogError::InvalidSlot {
                class_id: ClassId(1),
                slot: 0
            })
        );
        assert_eq!(
            GameCatalog::new(
                vec![ability(1, ClassId(1))],
                vec![class(1, &[(1, 1), (1, 1)])]
            ),
            Err(CatalogError::DuplicateSlot {
                class_id: ClassId(1),
                slot: 1
            })
        );
        assert_eq!(
            GameCatalog::new(vec![ability(1, ClassId(1))], vec![class(1, &[(1, 9)])]),
            Err(CatalogError::UnknownAbility {
                class_id: ClassId(1),
                ability_id: AbilityId(9)
            })
        );
        assert_eq!(
            GameCatalog::new(
                vec![ability(1, ClassId(2))],
                vec![class(1, &[(1, 1)]), class(2, &[])]
            ),
            Err(CatalogError::UnavailableAbility {
                class_id: ClassId(1),
                ability_id: AbilityId(1)
            })
        );
        let mut unsupported = ability(1, ClassId(1));
        unsupported.effect = AbilityEffect::Unsupported(7);
        assert_eq!(
            GameCatalog::new(vec![unsupported], vec![class(1, &[(1, 1)])]),
            Err(CatalogError::UnsupportedAbility(AbilityId(1)))
        );
        assert_eq!(
            GameCatalog::new(vec![ability(1, ClassId(9))], vec![class(1, &[])]),
            Err(CatalogError::UnknownAvailabilityClass {
                ability_id: AbilityId(1),
                class_id: ClassId(9)
            })
        );
    }

    #[test]
    fn mana_metadata_is_validated() {
        let mut negative_cost = ability(1, ClassId(1));
        negative_cost.mana_cost = -1;
        assert_eq!(
            GameCatalog::new(vec![negative_cost], vec![class(1, &[(1, 1)])]),
            Err(CatalogError::UnsupportedAbility(AbilityId(1)))
        );

        let mut negative_max = class(1, &[(1, 1)]);
        negative_max.max_mana = -1;
        assert!(matches!(
            GameCatalog::new(vec![ability(1, ClassId(1))], vec![negative_max]),
            Err(CatalogError::InvalidClassStats { .. })
        ));

        let mut negative_regeneration = class(1, &[(1, 1)]);
        negative_regeneration.mana_regeneration = -1;
        assert!(matches!(
            GameCatalog::new(vec![ability(1, ClassId(1))], vec![negative_regeneration]),
            Err(CatalogError::InvalidClassStats { .. })
        ));

        let mut expensive = ability(1, ClassId(1));
        expensive.mana_cost = 11;
        assert!(matches!(
            GameCatalog::new(vec![expensive], vec![class(1, &[(1, 1)])]),
            Err(CatalogError::AbilityCostExceedsClassMana { .. })
        ));
    }
}
