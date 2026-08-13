//! Transport-neutral input decoding and game-session commands.

use crate::{
    game::{AbilitySlot, Command, Direction, ExplorationGame},
    world::GenerationError,
};
use std::collections::VecDeque;

pub const MAX_INPUT_BYTES_PER_FEED: usize = 4096;
pub const MAX_INTENTS_PER_FEED: usize = 4;

pub use crate::game::AbilitySlot as SkillSlot;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Intent {
    Start,
    Move(Direction),
    Wait,
    UseSkill(SkillSlot),
    CycleTarget { backwards: bool },
    Confirm,
    CancelMode,
    ToggleInspect,
    PickupItem,
    ToggleItemUse,
    ToggleHelp,
    ToggleGodmode,
    ToggleCharacterInfo,
    Restart,
    Quit,
}

#[derive(Clone, Debug, Default)]
pub struct InputDecoder {
    pending: VecDeque<u8>,
    escape_generation: u64,
}

impl InputDecoder {
    #[must_use]
    pub fn has_pending_escape(&self) -> bool {
        self.pending.len() == 1 && self.pending[0] == 0x1b
    }

    #[must_use]
    pub fn pending_escape_generation(&self) -> Option<u64> {
        self.has_pending_escape().then_some(self.escape_generation)
    }

    pub fn flush_pending_escape(&mut self) -> Option<Intent> {
        if self.has_pending_escape() {
            self.pending.pop_front();
            Some(Intent::CancelMode)
        } else {
            None
        }
    }

    pub fn flush_pending_escape_generation(&mut self, generation: u64) -> Option<Intent> {
        if self.pending_escape_generation() == Some(generation) {
            self.flush_pending_escape()
        } else {
            None
        }
    }

    #[must_use]
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Intent> {
        let mut intents = Vec::new();
        for &byte in bytes.iter().take(MAX_INPUT_BYTES_PER_FEED) {
            if byte == 0x1b {
                self.escape_generation = self.escape_generation.wrapping_add(1);
            }
            self.pending.push_back(byte);
            self.decode_available(&mut intents);
            if intents.len() == MAX_INTENTS_PER_FEED {
                self.pending.clear();
                break;
            }
        }
        intents
    }

    fn decode_available(&mut self, intents: &mut Vec<Intent>) {
        loop {
            if self.pending.is_empty() {
                break;
            }
            if self.pending[0] == 0x1b {
                if self.pending.len() == 1 || (self.pending.len() == 2 && self.pending[1] == b'[') {
                    break;
                }
                if self.pending.len() >= 3 && self.pending[1] == b'[' {
                    let intent = match self.pending[2] {
                        b'A' => Some(Intent::Move(Direction::North)),
                        b'B' => Some(Intent::Move(Direction::South)),
                        b'C' => Some(Intent::Move(Direction::East)),
                        b'D' => Some(Intent::Move(Direction::West)),
                        b'Z' => Some(Intent::CycleTarget { backwards: true }),
                        _ => None,
                    };
                    self.pending.drain(..3);
                    if let Some(intent) = intent {
                        intents.push(intent);
                    }
                    continue;
                }
                self.pending.pop_front();
                continue;
            }
            let byte = self.pending.pop_front().expect("pending input is nonempty");
            if let Some(intent) = intent_from_byte(byte) {
                intents.push(intent);
                if intents.len() == MAX_INTENTS_PER_FEED {
                    break;
                }
            }
        }
    }
}

/// Apply a transport-neutral intent to a game session.
///
/// Returns `false` when the transport should close.
///
/// # Errors
///
/// Returns dungeon generation errors from restart.
pub fn apply_game_intent(
    game: &mut ExplorationGame,
    intent: Intent,
) -> Result<bool, GenerationError> {
    match intent {
        Intent::Quit => return Ok(false),
        Intent::ToggleHelp if !game.using_item => game.toggle_help(),
        Intent::ToggleGodmode if !game.help => game.toggle_godmode(),
        Intent::ToggleCharacterInfo if !game.help => game.toggle_character_info(),
        Intent::Restart => game.restart()?,
        Intent::Start | Intent::Confirm if game.targeting.is_some() && !game.help => {
            game.apply(Command::ConfirmTarget);
        }
        Intent::Start | Intent::Confirm => game.start(),
        Intent::Move(direction) if !game.help => {
            game.apply(if game.targeting.is_some() || game.inspecting.is_some() {
                Command::MoveCursor(direction)
            } else {
                Command::Move(direction)
            });
        }
        Intent::Wait if !game.help => {
            game.apply(Command::Wait);
        }
        Intent::UseSkill(slot) if !game.help => {
            game.apply(if game.using_item {
                Command::UseItemSlot(slot.number())
            } else {
                Command::UseAbility(slot)
            });
        }
        Intent::CycleTarget { backwards } if !game.help => {
            if game.targeting.is_some() {
                game.apply(Command::CycleTarget { backwards });
            } else {
                game.toggle_character_info();
            }
        }
        Intent::CancelMode if !game.help => {
            game.apply(Command::CancelMode);
        }
        Intent::ToggleInspect if !game.help => {
            game.apply(Command::ToggleInspect);
        }
        Intent::PickupItem if !game.help => {
            game.apply(Command::PickupItem);
        }
        Intent::ToggleItemUse if !game.help => {
            game.apply(Command::ToggleItemUse);
        }
        _ => {}
    }
    Ok(true)
}

pub(crate) fn intent_from_byte(byte: u8) -> Option<Intent> {
    Some(match byte {
        b'\r' | b'\n' => Intent::Confirm,
        b'\t' => Intent::CycleTarget { backwards: false },
        b'q' => Intent::Move(Direction::NorthWest),
        b'w' => Intent::Move(Direction::North),
        b'e' => Intent::Move(Direction::NorthEast),
        b'a' => Intent::Move(Direction::West),
        b's' => Intent::Wait,
        b'd' => Intent::Move(Direction::East),
        b'z' => Intent::Move(Direction::SouthWest),
        b'x' => Intent::Move(Direction::South),
        b'c' => Intent::Move(Direction::SouthEast),
        b'1'..=b'9' => Intent::UseSkill(AbilitySlot::new(byte - b'0')?),
        b'0' => Intent::UseSkill(AbilitySlot::new(10)?),
        b'i' => Intent::ToggleInspect,
        b'g' => Intent::PickupItem,
        b'u' => Intent::ToggleItemUse,
        b'?' => Intent::ToggleHelp,
        b'G' => Intent::ToggleGodmode,
        b'r' => Intent::Restart,
        b'Q' => Intent::Quit,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::RunSeed;

    #[test]
    fn decoder_maps_spatial_keys_arrows_and_skill_slots() {
        let mut decoder = InputDecoder::default();
        assert_eq!(
            decoder.feed(b"qwea"),
            [
                Intent::Move(Direction::NorthWest),
                Intent::Move(Direction::North),
                Intent::Move(Direction::NorthEast),
                Intent::Move(Direction::West),
            ]
        );
        assert_eq!(
            decoder.feed(b"sdzx"),
            [
                Intent::Wait,
                Intent::Move(Direction::East),
                Intent::Move(Direction::SouthWest),
                Intent::Move(Direction::South),
            ]
        );
        assert_eq!(decoder.feed(b"c"), [Intent::Move(Direction::SouthEast)]);
        assert!(decoder.feed(b"\x1b[").is_empty());
        assert_eq!(decoder.feed(b"A"), [Intent::Move(Direction::North)]);
        assert_eq!(
            decoder.feed(b"\t\x1b[Z"),
            [
                Intent::CycleTarget { backwards: false },
                Intent::CycleTarget { backwards: true }
            ]
        );
        assert!(decoder.feed(b"\x1b").is_empty());
        assert_eq!(decoder.flush_pending_escape(), Some(Intent::CancelMode));
        assert!(decoder.feed(b"\x1b").is_empty());
        let old_generation = decoder.pending_escape_generation().unwrap();
        assert_eq!(decoder.feed(b"[A"), [Intent::Move(Direction::North)]);
        assert!(decoder.feed(b"\x1b").is_empty());
        assert_eq!(
            decoder.flush_pending_escape_generation(old_generation),
            None,
            "an old transport timer must not flush a newer escape"
        );
        let current_generation = decoder.pending_escape_generation().unwrap();
        assert_eq!(
            decoder.flush_pending_escape_generation(current_generation),
            Some(Intent::CancelMode)
        );
        assert_eq!(
            decoder.feed(b"10"),
            [
                Intent::UseSkill(SkillSlot::CLEAVE),
                Intent::UseSkill(SkillSlot::new(10).unwrap()),
            ]
        );
        assert_eq!(decoder.feed(b"i"), [Intent::ToggleInspect]);
    }

    #[test]
    fn dispatch_keeps_sessions_independent_and_empty_skills_free() {
        let mut waiting = ExplorationGame::new(Some(RunSeed(11))).unwrap();
        let mut helping = ExplorationGame::new(Some(RunSeed(22))).unwrap();
        waiting.start();
        helping.start();

        apply_game_intent(&mut waiting, Intent::Wait).unwrap();
        apply_game_intent(&mut helping, Intent::ToggleHelp).unwrap();
        let before_empty_skill = helping.clone();
        apply_game_intent(&mut helping, Intent::UseSkill(SkillSlot::new(2).unwrap())).unwrap();

        assert_eq!(waiting.turn, 1);
        assert!(!waiting.help);
        assert_eq!(helping, before_empty_skill);
        assert_ne!(waiting.seed(), helping.seed());
    }

    #[test]
    fn inspect_dispatch_preserves_cursor_across_help_and_is_session_local() {
        let mut inspecting = ExplorationGame::new(Some(RunSeed(31))).unwrap();
        let mut ordinary = ExplorationGame::new(Some(RunSeed(32))).unwrap();
        inspecting.start();
        ordinary.start();
        apply_game_intent(&mut inspecting, Intent::ToggleInspect).unwrap();
        let initial = inspecting.inspecting.unwrap().cursor;
        apply_game_intent(&mut inspecting, Intent::Move(Direction::East)).unwrap();
        let moved = inspecting.inspecting.unwrap().cursor;
        assert_eq!(moved, initial.offset(1, 0));
        apply_game_intent(&mut inspecting, Intent::ToggleHelp).unwrap();
        apply_game_intent(&mut inspecting, Intent::Move(Direction::South)).unwrap();
        assert_eq!(inspecting.inspecting.unwrap().cursor, moved);
        apply_game_intent(&mut inspecting, Intent::ToggleHelp).unwrap();
        apply_game_intent(&mut inspecting, Intent::CancelMode).unwrap();
        assert!(inspecting.inspecting.is_none());
        assert!(ordinary.inspecting.is_none());
        assert_eq!(inspecting.turn, 0);
    }

    #[test]
    fn item_intents_are_modal_turn_free_until_a_valid_slot_is_used() {
        use crate::game::{ItemId, ItemInstance, ItemInstanceId};
        let mut game = ExplorationGame::new(Some(RunSeed(44))).unwrap();
        game.start();
        game.inventory[0] = Some(ItemInstance {
            instance_id: ItemInstanceId(90),
            item_id: ItemId::HEALTH_POTION,
        });
        game.player.health -= 12;
        apply_game_intent(&mut game, Intent::ToggleItemUse).unwrap();
        assert!(game.using_item);
        assert_eq!(game.turn, 0);
        apply_game_intent(&mut game, Intent::ToggleHelp).unwrap();
        assert!(!game.help, "help cannot coexist with item-use mode");
        apply_game_intent(&mut game, Intent::UseSkill(SkillSlot::new(1).unwrap())).unwrap();
        assert!(game.inventory[0].is_none());
        assert_eq!(game.turn, 1);
        assert!(!game.using_item);
    }
}
