//! Stable actor allocation and deterministic floor population.

use super::{Actor, ActorId, ActorKind, ExplorationGame};
use crate::world::Position;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MonsterDefinition {
    pub kind: ActorKind,
    pub name: &'static str,
    pub description: &'static str,
    pub glyph: char,
    pub health: i32,
    pub armor: i32,
    pub damage: i32,
    pub population_order: u8,
}

impl ActorKind {
    #[must_use]
    pub const fn definition(self) -> MonsterDefinition {
        match self {
            Self::Skeleton => MonsterDefinition {
                kind: self,
                name: "Skeleton",
                description: "A restless soldier held together by old malice.",
                glyph: 's',
                health: 10,
                armor: 1,
                damage: 5,
                population_order: 0,
            },
            Self::Ghoul => MonsterDefinition {
                kind: self,
                name: "Ghoul",
                description: "A ravenous corpse poised to lunge.",
                glyph: 'g',
                health: 14,
                armor: 0,
                damage: 7,
                population_order: 1,
            },
            Self::Cultist => MonsterDefinition {
                kind: self,
                name: "Cultist",
                description: "A grave-priest weaving distant hexes.",
                glyph: 'c',
                health: 9,
                armor: 0,
                damage: 6,
                population_order: 2,
            },
        }
    }
}
use rand::{Rng, SeedableRng, seq::SliceRandom};
use rand_chacha::ChaCha8Rng;

impl ExplorationGame {
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

    pub(super) fn populate_enemies(&mut self) {
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
            let definition = kind.definition();
            self.spawn_hostile(
                kind,
                position,
                definition.health,
                definition.armor,
                definition.damage,
            );
        }
    }
}
