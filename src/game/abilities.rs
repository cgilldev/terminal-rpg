//! Typed ability definitions, loadout slots, and per-run cooldown state.

use serde::{Deserialize, Serialize};

pub const ABILITY_SLOT_COUNT: usize = 10;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct AbilityId(pub u16);

impl AbilityId {
    pub const CLEAVE: Self = Self(1);
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct AbilitySlot(u8);

impl AbilitySlot {
    pub const CLEAVE: Self = Self(1);

    #[must_use]
    pub const fn new(number: u8) -> Option<Self> {
        if number >= 1 && number <= 10 {
            Some(Self(number))
        } else {
            None
        }
    }

    #[must_use]
    pub const fn number(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn index(self) -> usize {
        (self.0 - 1) as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AbilityTargeting {
    Immediate,
    HostileSingle { range: i32, line_of_sight: bool },
    Unsupported(u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AbilityEffect {
    Cleave,
    GraveBolt { base_damage: i32 },
    Unsupported(u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AbilityAvailability {
    Class(super::ClassId),
    Unsupported(u16),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbilityDefinition {
    pub id: AbilityId,
    pub name: String,
    pub description: String,
    pub mana_cost: i32,
    pub cooldown_turns: u8,
    pub targeting: AbilityTargeting,
    pub effect: AbilityEffect,
    pub availability: AbilityAvailability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AbilityState {
    pub ability_id: AbilityId,
    pub cooldown_remaining: u8,
}
