//! Local Crossterm lifecycle adapter.

use super::{DisplayProfile, draw_game, intent_allowed_at_size};
use crate::{
    game::{Direction, ExplorationGame, RunSeed},
    session::{Intent, apply_game_intent, intent_from_byte},
};
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io::{self, Write},
    time::Duration,
};

/// Run the shared game locally.
///
/// # Errors
///
/// Returns terminal I/O errors after restoring terminal state.
pub fn run_local(
    seed: Option<RunSeed>,
    profile: DisplayProfile,
    debug_godmode: bool,
) -> io::Result<()> {
    let mut guard = LocalTerminalGuard::default();
    let mut stdout = io::stdout();
    guard.setup_with(&mut stdout, enable_raw_mode)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let mut game = ExplorationGame::new(seed).map_err(io::Error::other)?;
    game.set_debug_godmode_enabled(debug_godmode);
    loop {
        terminal.draw(|frame| draw_game(frame, &game, profile))?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if let Some(intent) = intent_from_key(key.code) {
                    let area = terminal.size()?;
                    if intent_allowed_at_size(intent, area)
                        && !apply_game_intent(&mut game, intent).map_err(io::Error::other)?
                    {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Debug, Default, Eq, PartialEq)]
pub(super) struct LocalTerminalGuard {
    pub(super) raw_mode: bool,
    pub(super) entered_alternate_screen: bool,
    pub(super) cursor_hidden: bool,
}

impl LocalTerminalGuard {
    pub(super) fn setup_with<W, F>(&mut self, writer: &mut W, enable_raw: F) -> io::Result<()>
    where
        W: Write,
        F: FnOnce() -> io::Result<()>,
    {
        enable_raw()?;
        self.raw_mode = true;

        self.entered_alternate_screen = true;
        execute!(writer, EnterAlternateScreen)?;
        self.cursor_hidden = true;
        execute!(writer, Hide)?;
        Ok(())
    }

    pub(super) fn restore_with<W, F>(&mut self, writer: &mut W, disable_raw: F)
    where
        W: Write,
        F: FnOnce() -> io::Result<()>,
    {
        if self.cursor_hidden {
            let _ = execute!(writer, Show);
            self.cursor_hidden = false;
        }
        if self.entered_alternate_screen {
            let _ = execute!(writer, LeaveAlternateScreen);
            self.entered_alternate_screen = false;
        }
        if self.raw_mode {
            let _ = disable_raw();
            self.raw_mode = false;
        }
    }
}

impl Drop for LocalTerminalGuard {
    fn drop(&mut self) {
        self.restore_with(&mut io::stdout(), disable_raw_mode);
    }
}

pub(super) fn intent_from_key(code: KeyCode) -> Option<Intent> {
    match code {
        KeyCode::Up => Some(Intent::Move(Direction::North)),
        KeyCode::Down => Some(Intent::Move(Direction::South)),
        KeyCode::Left => Some(Intent::Move(Direction::West)),
        KeyCode::Right => Some(Intent::Move(Direction::East)),
        KeyCode::Enter => Some(Intent::Confirm),
        KeyCode::Tab => Some(Intent::CycleTarget { backwards: false }),
        KeyCode::BackTab => Some(Intent::CycleTarget { backwards: true }),
        KeyCode::Esc => Some(Intent::CancelMode),
        KeyCode::Char(character) if character.is_ascii() => intent_from_byte(character as u8),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_terminal_maps_targeting_control_keys() {
        assert_eq!(intent_from_key(KeyCode::Enter), Some(Intent::Confirm));
        assert_eq!(
            intent_from_key(KeyCode::Tab),
            Some(Intent::CycleTarget { backwards: false })
        );
        assert_eq!(
            intent_from_key(KeyCode::BackTab),
            Some(Intent::CycleTarget { backwards: true })
        );
        assert_eq!(intent_from_key(KeyCode::Esc), Some(Intent::CancelMode));
    }
}
