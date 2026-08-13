//! Serializable hostile-target selection and validation.

use super::{
    AbilityEffect, AbilitySlot, AbilityTargeting, ActorId, CommandOutcome, Direction,
    ExplorationGame, visibility::has_line_of_sight,
};
use crate::world::Position;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TargetingState {
    pub ability_slot: AbilitySlot,
    pub cursor: Position,
    pub selected_actor: Option<ActorId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetValidity {
    Valid(ActorId),
    NoHostile,
    OutOfRange,
    Blocked,
}

impl ExplorationGame {
    pub(super) fn begin_targeting(&mut self, slot: AbilitySlot) -> CommandOutcome {
        let candidates = self.valid_targets(slot);
        let Some(id) = candidates.first().copied() else {
            let name = self
                .ability_state(slot)
                .and_then(|state| self.ability_definition(state.ability_id))
                .map_or("ability", |definition| definition.name.as_str())
                .to_owned();
            self.push_event(format!("No valid target answers {name}."));
            return CommandOutcome::Rejected;
        };
        let cursor = self.hostile(id).expect("candidate remains alive").position;
        self.targeting = Some(TargetingState {
            ability_slot: slot,
            cursor,
            selected_actor: Some(id),
        });
        CommandOutcome::Rejected
    }

    pub(super) fn move_target_cursor(&mut self, direction: Direction) -> CommandOutcome {
        let Some(mut targeting) = self.targeting else {
            return CommandOutcome::Rejected;
        };
        let (dx, dy) = direction.delta();
        let candidate = targeting.cursor.offset(dx, dy);
        if self.map.contains(candidate) {
            targeting.cursor = candidate;
            targeting.selected_actor = self
                .hostiles
                .iter()
                .find(|actor| actor.position == candidate)
                .map(|actor| actor.id);
            self.targeting = Some(targeting);
        }
        CommandOutcome::Rejected
    }

    pub(super) fn cycle_target(&mut self, backwards: bool) -> CommandOutcome {
        let Some(mut targeting) = self.targeting else {
            return CommandOutcome::Rejected;
        };
        let candidates = self.valid_targets(targeting.ability_slot);
        if candidates.is_empty() {
            self.targeting = None;
            return CommandOutcome::Rejected;
        }
        let current = targeting
            .selected_actor
            .and_then(|id| candidates.iter().position(|candidate| *candidate == id));
        let index = if backwards {
            current.map_or(candidates.len() - 1, |index| {
                (index + candidates.len() - 1) % candidates.len()
            })
        } else {
            current.map_or(0, |index| (index + 1) % candidates.len())
        };
        let id = candidates[index];
        targeting.selected_actor = Some(id);
        targeting.cursor = self.hostile(id).expect("candidate remains alive").position;
        self.targeting = Some(targeting);
        CommandOutcome::Rejected
    }

    pub(super) fn cancel_targeting(&mut self) -> CommandOutcome {
        self.targeting = None;
        CommandOutcome::Rejected
    }

    pub(super) fn confirm_target(&mut self) -> CommandOutcome {
        let Some(targeting) = self.targeting else {
            return CommandOutcome::Rejected;
        };
        if targeting
            .selected_actor
            .is_some_and(|id| self.hostile(id).is_none())
        {
            self.targeting = None;
            self.push_event("The chosen target is gone.");
            return CommandOutcome::Rejected;
        }
        let id = match self.target_validity(targeting.ability_slot, targeting.cursor) {
            TargetValidity::Valid(id) => id,
            TargetValidity::NoHostile => {
                self.push_event("No living foe stands beneath the cursor.");
                return CommandOutcome::Rejected;
            }
            TargetValidity::OutOfRange => {
                self.push_event("That foe lies beyond the bolt's reach.");
                return CommandOutcome::Rejected;
            }
            TargetValidity::Blocked => {
                self.push_event("Stone and shadow block the Grave Bolt.");
                return CommandOutcome::Rejected;
            }
        };
        let slot = targeting.ability_slot;
        let state = self
            .ability_state(slot)
            .expect("targeted ability is equipped");
        let definition = self
            .ability_definition(state.ability_id)
            .cloned()
            .expect("targeted ability is defined");
        let AbilityEffect::GraveBolt { base_damage } = definition.effect else {
            return CommandOutcome::Rejected;
        };
        if self.player.mana < definition.mana_cost {
            self.push_event(format!("Not enough mana for {}.", definition.name));
            return CommandOutcome::Rejected;
        }
        let index = self
            .hostiles
            .iter()
            .position(|actor| actor.id == id)
            .expect("valid target lives");
        let damage = self.roll_damage(base_damage, self.hostiles[index].armor);
        self.hostiles[index].health -= damage;
        let kind = self.hostiles[index].kind;
        self.push_event(format!(
            "{} strikes {} {} for {damage} damage.",
            definition.name,
            kind.name(),
            id.0
        ));
        if self.hostiles[index].health <= 0 {
            self.hostiles.remove(index);
            self.push_event(format!("The {} is destroyed.", kind.name()));
        }
        self.abilities[slot.index()]
            .as_mut()
            .expect("ability remains equipped")
            .cooldown_remaining = definition.cooldown_turns;
        self.player.mana -= definition.mana_cost;
        self.targeting = None;
        self.advance_turn(false);
        CommandOutcome::Advanced
    }

    #[must_use]
    pub fn target_validity(&self, slot: AbilitySlot, position: Position) -> TargetValidity {
        let Some(state) = self.ability_state(slot) else {
            return TargetValidity::NoHostile;
        };
        let Some(definition) = self.ability_definition(state.ability_id) else {
            return TargetValidity::NoHostile;
        };
        let AbilityTargeting::HostileSingle {
            range,
            line_of_sight,
        } = definition.targeting
        else {
            return TargetValidity::NoHostile;
        };
        let Some(actor) = self
            .hostiles
            .iter()
            .find(|actor| actor.position == position)
        else {
            return TargetValidity::NoHostile;
        };
        if self.player.position.chebyshev_distance(position) > range {
            return TargetValidity::OutOfRange;
        }
        if line_of_sight && !has_line_of_sight(&self.map, self.player.position, position) {
            return TargetValidity::Blocked;
        }
        TargetValidity::Valid(actor.id)
    }

    fn valid_targets(&self, slot: AbilitySlot) -> Vec<ActorId> {
        let mut candidates = self
            .hostiles
            .iter()
            .filter(|actor| {
                matches!(
                    self.target_validity(slot, actor.position),
                    TargetValidity::Valid(_)
                )
            })
            .map(|actor| actor.id)
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|id| {
            let actor = self.hostile(*id).expect("candidate lives");
            (self.player.position.chebyshev_distance(actor.position), *id)
        });
        candidates
    }

    fn hostile(&self, id: ActorId) -> Option<&super::Actor> {
        self.hostiles.iter().find(|actor| actor.id == id)
    }
}
