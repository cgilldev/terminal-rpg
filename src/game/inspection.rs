//! Fog-aware, read-only inspection facts for presentation adapters.

use super::{ActorId, ActorKind, ExplorationGame, Telegraph};
use crate::world::{Position, Tile};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InspectState {
    pub cursor: Position,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum InspectionVisibility {
    Unknown,
    Remembered,
    Visible,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Inspection {
    pub position: Position,
    pub visibility: InspectionVisibility,
    pub terrain: Option<InspectionDetail>,
    pub entities: Vec<EntityInspection>,
    pub markers: Vec<InspectionDetail>,
    pub items: Vec<InspectionDetail>,
    pub carried_items: Vec<InspectionDetail>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InspectionDetail {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EntityInspection {
    pub actor_id: ActorId,
    pub name: String,
    pub description: String,
    pub health: i32,
    pub max_health: i32,
    pub armor: i32,
    pub active: bool,
}

impl ExplorationGame {
    #[must_use]
    pub fn inspect(&self, position: Position) -> Inspection {
        if !self.is_explored(position) {
            return Inspection {
                position,
                visibility: InspectionVisibility::Unknown,
                terrain: None,
                entities: Vec::new(),
                markers: Vec::new(),
                items: Vec::new(),
                carried_items: Vec::new(),
            };
        }
        let terrain = self.map.tile(position).map(tile_detail);
        if !self.is_visible(position) {
            return Inspection {
                position,
                visibility: InspectionVisibility::Remembered,
                terrain,
                entities: Vec::new(),
                markers: Vec::new(),
                items: Vec::new(),
                carried_items: Vec::new(),
            };
        }
        let mut entities = Vec::new();
        if position == self.player.position {
            entities.push(EntityInspection {
                actor_id: ActorId(0),
                name: self.class_definition().name.clone(),
                description: self.class_definition().description.clone(),
                health: self.player.health,
                max_health: self.player.max_health,
                armor: self.player.armor,
                active: true,
            });
        }
        let mut hostiles = self
            .hostiles
            .iter()
            .filter(|actor| actor.position == position)
            .collect::<Vec<_>>();
        hostiles.sort_unstable_by_key(|actor| actor.id);
        entities.extend(hostiles.into_iter().map(|actor| EntityInspection {
            actor_id: actor.id,
            name: actor.kind.name().into(),
            description: actor.kind.description().into(),
            health: actor.health,
            max_health: actor.max_health,
            armor: actor.armor,
            active: actor.active,
        }));
        let mut markers = Vec::new();
        for actor in &self.hostiles {
            if actor.position == position
                && let Some(telegraph) = actor.telegraph
            {
                markers.push(telegraph_detail(telegraph));
            }
            if let Some(telegraph) = actor
                .telegraph
                .filter(|telegraph| telegraph.target() == position)
            {
                markers.push(telegraph_target_detail(telegraph));
            }
        }
        let items = self
            .ground_items
            .iter()
            .filter(|ground| ground.position == position)
            .filter_map(|ground| self.item_definition(ground.item.item_id))
            .map(|definition| InspectionDetail {
                name: definition.name.clone(),
                description: definition.description.clone(),
            })
            .collect();
        let carried_items = if position == self.player.position {
            self.inventory
                .iter()
                .flatten()
                .filter_map(|item| self.item_definition(item.item_id))
                .map(|definition| InspectionDetail {
                    name: format!("Carried: {}", definition.name),
                    description: definition.description.clone(),
                })
                .collect()
        } else {
            Vec::new()
        };
        Inspection {
            position,
            visibility: InspectionVisibility::Visible,
            terrain,
            entities,
            markers,
            items,
            carried_items,
        }
    }
}

fn tile_detail(tile: Tile) -> InspectionDetail {
    let definition = tile.definition();
    InspectionDetail {
        name: definition.name.into(),
        description: definition.description.into(),
    }
}

fn telegraph_detail(telegraph: Telegraph) -> InspectionDetail {
    match telegraph {
        Telegraph::GhoulLunge { .. } => InspectionDetail {
            name: "Ghoul Lunge".into(),
            description: "The ghoul gathers itself to cross the marked ground.".into(),
        },
        Telegraph::CultistHex { .. } => InspectionDetail {
            name: "Cultist Hex".into(),
            description: "A death-hex coils toward its chosen tile.".into(),
        },
    }
}

fn telegraph_target_detail(telegraph: Telegraph) -> InspectionDetail {
    match telegraph {
        Telegraph::GhoulLunge { .. } => InspectionDetail {
            name: "Targeted by Ghoul Lunge".into(),
            description: "A ravenous trajectory converges upon this ground.".into(),
        },
        Telegraph::CultistHex { .. } => InspectionDetail {
            name: "Targeted by Cultist Hex".into(),
            description: "A grave-priest's death-hex is fixed upon this ground.".into(),
        },
    }
}

impl ActorKind {
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Skeleton => "A restless soldier held together by old malice.",
            Self::Ghoul => "A ravenous corpse poised to lunge.",
            Self::Cultist => "A grave-priest weaving distant hexes.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_typed_inspectable_variant_has_specific_metadata() {
        for tile in [
            Tile::Wall,
            Tile::Floor,
            Tile::ClosedDoor,
            Tile::OpenDoor,
            Tile::Exit,
        ] {
            let detail = tile_detail(tile);
            assert!(!detail.name.is_empty() && !detail.description.is_empty());
        }
        for kind in [ActorKind::Skeleton, ActorKind::Ghoul, ActorKind::Cultist] {
            assert!(!kind.name().is_empty() && !kind.description().is_empty());
        }
        for telegraph in [
            Telegraph::GhoulLunge {
                target: Position::new(1, 1),
            },
            Telegraph::CultistHex {
                target: Position::new(1, 1),
            },
        ] {
            let origin = telegraph_detail(telegraph);
            let target = telegraph_target_detail(telegraph);
            assert!(!origin.name.is_empty() && !origin.description.is_empty());
            assert!(!target.name.is_empty() && !target.description.is_empty());
            assert_ne!(origin.name, target.name);
        }
        assert_ne!(
            telegraph_target_detail(Telegraph::GhoulLunge {
                target: Position::new(0, 0),
            })
            .name,
            telegraph_target_detail(Telegraph::CultistHex {
                target: Position::new(0, 0),
            })
            .name
        );
    }
}
