//! Player attacks, ability cooldowns, armor, and damage outcomes.

use super::{
    AbilityEffect, AbilitySlot, AbilityTargeting, CommandOutcome, ExplorationGame, RunStatus,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

const COMBAT_RNG_DOMAIN: u64 = 0xC04B_A700_DA6A_6E55;

impl ExplorationGame {
    pub(super) fn basic_attack(&mut self, index: usize) {
        let damage = self.roll_damage(self.player.damage, self.hostiles[index].armor);
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

    pub(super) fn use_ability(&mut self, slot: AbilitySlot) -> CommandOutcome {
        let Some(state) = self.ability_state(slot) else {
            return CommandOutcome::Rejected;
        };
        if state.cooldown_remaining != 0 {
            return CommandOutcome::Rejected;
        }
        let Some(definition) = self.ability_definition(state.ability_id).cloned() else {
            return CommandOutcome::Rejected;
        };
        if self.player.mana < definition.mana_cost {
            self.push_event(format!("Not enough mana for {}.", definition.name));
            return CommandOutcome::Rejected;
        }
        let outcome = match definition.targeting {
            AbilityTargeting::Immediate => match definition.effect {
                AbilityEffect::Cleave => self.use_cleave(&definition.name),
                AbilityEffect::GraveBolt { .. } | AbilityEffect::Unsupported(_) => {
                    CommandOutcome::Rejected
                }
            },
            AbilityTargeting::HostileSingle { .. } => return self.begin_targeting(slot),
            AbilityTargeting::Unsupported(_) => CommandOutcome::Rejected,
        };
        if outcome == CommandOutcome::Advanced {
            self.player.mana -= definition.mana_cost;
            self.abilities[slot.index()]
                .as_mut()
                .expect("executed ability remains equipped")
                .cooldown_remaining = definition.cooldown_turns;
        }
        outcome
    }

    fn use_cleave(&mut self, ability_name: &str) -> CommandOutcome {
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
        self.push_event(format!("You unleash {ability_name}."));
        let mut defeated = Vec::new();
        for (_, index) in targets {
            let armor = self.hostiles[index].armor;
            let damage = self.roll_damage(self.player.damage, armor);
            let actor = &mut self.hostiles[index];
            actor.health -= damage;
            self.events.push(format!(
                "{ability_name} hits {} {} for {damage} damage.",
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
        self.advance_turn(false);
        CommandOutcome::Advanced
    }

    /// Apply an incoming variable-damage hit and return damage after armor.
    pub fn damage_player(&mut self, base_damage: i32, source: &str) -> i32 {
        let damage = self.roll_damage(base_damage, self.player.armor);
        self.player.health = (self.player.health - damage).max(i32::from(self.godmode));
        self.push_event(format!("{source} hits you for {damage} damage."));
        if self.player.health == 0 {
            self.status = RunStatus::Death;
            let class_name = self.class_definition().name.clone();
            self.push_event(format!("The {class_name} falls."));
        }
        damage
    }

    pub(super) fn roll_damage(&mut self, base_damage: i32, armor: i32) -> i32 {
        let mut rng = ChaCha8Rng::seed_from_u64(self.seed().0 ^ COMBAT_RNG_DOMAIN);
        rng.set_word_pos(u128::from(self.combat_roll_count));
        let spread: i32 = rng.random_range(-2..=2);
        self.combat_roll_count += 1;
        mitigated_damage(base_damage.saturating_add(spread).max(1), armor)
    }
}

#[must_use]
pub const fn mitigated_damage(raw_damage: i32, armor: i32) -> i32 {
    let reduced = raw_damage - armor;
    if reduced < 1 { 1 } else { reduced }
}
