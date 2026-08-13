//! Enemy activation, telegraphs, pursuit, and line-of-sight rules.

use super::{
    ActorId, ActorKind, DIRECTIONS, ExplorationGame, FIELD_OF_VIEW_RADIUS, RunStatus, Telegraph,
    visibility::has_line_of_sight,
};
use crate::world::{Map, Position, Tile};
use std::collections::{HashMap, HashSet, VecDeque};

impl ExplorationGame {
    pub(super) fn run_enemy_turns(&mut self) {
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

pub(super) fn can_step(map: &Map, from: Position, to: Position) -> bool {
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
