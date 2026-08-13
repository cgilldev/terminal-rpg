//! Field-of-view discovery and terrain-aware line of sight.

use super::ExplorationGame;
use crate::world::{Map, Position, Tile};

impl ExplorationGame {
    pub(crate) fn recompute_visibility(&mut self) {
        self.visible.clear();
        for position in self.map.positions() {
            if self.player.position.chebyshev_distance(position)
                <= self.effective_visibility_radius()
                && has_line_of_sight(&self.map, self.player.position, position)
            {
                self.visible.insert(position);
                self.explored.insert(position);
            }
        }
    }
}

pub(super) fn has_line_of_sight(map: &Map, from: Position, to: Position) -> bool {
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
