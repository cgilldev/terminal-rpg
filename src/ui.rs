//! Shared Ratatui game presentation and terminal input semantics.

use crate::{
    game::{Command, Direction, ExplorationGame, RunSeed, RunStatus},
    world::{GenerationError, Position, Tile},
};
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction as LayoutDirection, Layout, Rect, Size},
    style::{Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::{self, Write},
    time::Duration,
};

pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;
pub const MAX_WIDTH: u16 = 300;
pub const MAX_HEIGHT: u16 = 120;
pub const MAX_INPUT_BYTES_PER_FEED: usize = 4096;
pub const MAX_INTENTS_PER_FEED: usize = 4;

const ASCII_BORDER: border::Set<'static> = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DisplayProfile {
    pub ascii: bool,
    pub no_color: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Intent {
    Start,
    Move(Direction),
    Wait,
    Cleave,
    ToggleHelp,
    Restart,
    Quit,
}

#[derive(Clone, Debug, Default)]
pub struct InputDecoder {
    pending: VecDeque<u8>,
}

impl InputDecoder {
    #[must_use]
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Intent> {
        let mut intents = Vec::new();
        for &byte in bytes.iter().take(MAX_INPUT_BYTES_PER_FEED) {
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

pub fn draw_game(frame: &mut Frame<'_>, game: &ExplorationGame, profile: DisplayProfile) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        draw_small_terminal(frame, area, profile);
        return;
    }
    if game.status == RunStatus::Title {
        draw_title(frame, area, game, profile);
        return;
    }

    let vertical = Layout::default()
        .direction(LayoutDirection::Vertical)
        .constraints([
            Constraint::Min(17),
            Constraint::Length(4),
            Constraint::Length(1),
        ])
        .split(area);
    let top = Layout::default()
        .direction(LayoutDirection::Horizontal)
        .constraints([Constraint::Min(52), Constraint::Length(26)])
        .split(vertical[0]);
    let map_block = panel(" The Ossuary ", profile);
    let map_inner = map_block.inner(top[0]);
    let map_lines = exploration_map_lines(game, map_inner.width, map_inner.height, profile);
    frame.render_widget(Paragraph::new(map_lines).block(map_block), top[0]);

    let status = vec![
        Line::from(Span::styled(
            "GRAVE KNIGHT",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "HP      {}/{}",
            game.player.health, game.player.max_health
        )),
        Line::from(format!("Armor   {}", game.player.armor)),
        Line::from(if game.cleave_cooldown == 0 {
            "Cleave  Ready".to_owned()
        } else {
            format!("Cleave  {} turns", game.cleave_cooldown)
        }),
        Line::from(""),
        Line::from(format!("Turn    {}", game.turn)),
        Line::from(format!("Seed    {}", game.seed().0)),
        Line::from(format!("Gen     v{}", game.generator_version())),
        Line::from(format!("Enemies {}", game.hostiles.len())),
        Line::from(threat_summary(game)),
    ];
    frame.render_widget(
        Paragraph::new(status).block(panel(" Status ", profile)),
        top[1],
    );
    let events = game
        .events
        .iter()
        .rev()
        .take(2)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    frame.render_widget(
        Paragraph::new(events).block(panel(" Events ", profile)),
        vertical[1],
    );
    frame.render_widget(
        Paragraph::new(
            "Move/attack: arrows/hjklyubn  Wait: .  Cleave: a  Help: ?  Restart: r  Quit: q",
        ),
        vertical[2],
    );

    if game.help {
        let popup = Rect::new(area.width / 2 - 23, area.height / 2 - 5, 46, 10);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(
                "Explore the Ossuary and reach >.\n\nMovement and waiting consume turns. Bumping + opens a door. s Skeleton, g Ghoul, c Cultist. ! lunge, * hex.\n\nPress ? to return.",
            )
            .block(panel(" Help ", profile))
            .wrap(Wrap { trim: true }),
            popup,
        );
    }
    draw_run_outcome(frame, area, game.status, profile);
}

fn draw_run_outcome(frame: &mut Frame<'_>, area: Rect, status: RunStatus, profile: DisplayProfile) {
    if status == RunStatus::Victory {
        let popup = Rect::new(area.width / 2 - 22, area.height / 2 - 4, 44, 8);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(
                "YOU ESCAPED THE OSSUARY\n\nThe Grave Knight returns to the dying light.\n\nPress r for another descent or q to quit.",
            )
            .block(panel(" Victory ", profile))
            .wrap(Wrap { trim: true }),
            popup,
        );
    } else if status == RunStatus::Death {
        let popup = Rect::new(area.width / 2 - 22, area.height / 2 - 4, 44, 8);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(
                "YOU DIED\n\nThe Ossuary claims another oath-bound soul.\n\nPress r for another descent or q to quit.",
            )
            .block(panel(" Death ", profile))
            .wrap(Wrap { trim: true }),
            popup,
        );
    }
}

fn draw_title(frame: &mut Frame<'_>, area: Rect, game: &ExplorationGame, profile: DisplayProfile) {
    let vertical = Layout::vertical([
        Constraint::Percentage(35),
        Constraint::Length(9),
        Constraint::Percentage(35),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("TERMINAL RPG"),
            Line::from(""),
            Line::from("A dark-fantasy descent into the Ossuary."),
            Line::from(""),
            Line::from(format!(
                "Seed {}  Generator v{}",
                game.seed().0,
                game.generator_version()
            )),
            Line::from(""),
            Line::from("Press Enter to descend. Press q to quit."),
        ])
        .block(panel(" The Grave Knight ", profile))
        .wrap(Wrap { trim: true }),
        vertical[1],
    );
}

fn draw_small_terminal(frame: &mut Frame<'_>, area: Rect, profile: DisplayProfile) {
    frame.render_widget(
        Paragraph::new(format!(
            "Terminal too small: {}x{}. Resize to at least {MIN_WIDTH}x{MIN_HEIGHT}. No turns pass while this message is shown.",
            area.width, area.height
        ))
        .block(panel(" Terminal RPG ", profile))
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn exploration_map_lines(
    game: &ExplorationGame,
    width: u16,
    height: u16,
    profile: DisplayProfile,
) -> Vec<Line<'static>> {
    let view_width = i32::from(width);
    let view_height = i32::from(height);
    let max_x = (i32::from(game.map.width()) - view_width).max(0);
    let max_y = (i32::from(game.map.height()) - view_height).max(0);
    let origin_x = (game.player.position.x - view_width / 2).clamp(0, max_x);
    let origin_y = (game.player.position.y - view_height / 2).clamp(0, max_y);
    (0..view_height)
        .map(|screen_y| {
            let mut row = String::with_capacity(usize::from(width));
            for screen_x in 0..view_width {
                let position = Position::new(origin_x + screen_x, origin_y + screen_y);
                row.push(exploration_glyph(game, position, profile));
            }
            Line::from(row)
        })
        .collect()
}

fn exploration_glyph(game: &ExplorationGame, position: Position, profile: DisplayProfile) -> char {
    if game.is_visible(position) {
        if position == game.player.position {
            return '@';
        }
        if let Some(telegraph) = game.telegraph_at(position) {
            return match telegraph {
                crate::game::Telegraph::GhoulLunge { .. } => '!',
                crate::game::Telegraph::CultistHex { .. } => '*',
            };
        }
        if let Some(actor) = game
            .hostiles
            .iter()
            .find(|actor| actor.position == position)
        {
            return match (actor.kind, actor.telegraph.is_some()) {
                (crate::game::ActorKind::Skeleton, _) => 's',
                (crate::game::ActorKind::Ghoul, false) => 'g',
                (crate::game::ActorKind::Ghoul, true) => 'G',
                (crate::game::ActorKind::Cultist, false) => 'c',
                (crate::game::ActorKind::Cultist, true) => 'C',
                (crate::game::ActorKind::GraveKnight, _) => '@',
            };
        }
        return match game.map.tile(position) {
            Some(Tile::Wall) if profile.ascii => '#',
            Some(Tile::Wall) => '▓',
            Some(Tile::Floor) if profile.ascii => '.',
            Some(Tile::Floor) => '·',
            Some(Tile::ClosedDoor) if profile.ascii => '+',
            Some(Tile::ClosedDoor) => '╬',
            Some(Tile::OpenDoor) => '/',
            Some(Tile::Exit) => '>',
            None => ' ',
        };
    }
    if game.is_explored(position) {
        return match game.map.tile(position) {
            Some(Tile::Wall) if profile.ascii => '%',
            Some(Tile::Wall) => '░',
            Some(_) => ',',
            None => ' ',
        };
    }
    ' '
}

fn threat_summary(game: &ExplorationGame) -> String {
    let lunges = game
        .hostiles
        .iter()
        .filter(|actor| {
            matches!(
                actor.telegraph,
                Some(crate::game::Telegraph::GhoulLunge { .. })
            )
        })
        .count();
    let hexes = game
        .hostiles
        .iter()
        .filter(|actor| {
            matches!(
                actor.telegraph,
                Some(crate::game::Telegraph::CultistHex { .. })
            )
        })
        .count();
    if lunges == 0 && hexes == 0 {
        "Threat  None".into()
    } else {
        format!("Threat  !{lunges} *{hexes} on @")
    }
}

#[must_use]
pub fn capped_area(width: u32, height: u32) -> Rect {
    Rect::new(
        0,
        0,
        u16::try_from(width)
            .unwrap_or(MAX_WIDTH)
            .clamp(1, MAX_WIDTH),
        u16::try_from(height)
            .unwrap_or(MAX_HEIGHT)
            .clamp(1, MAX_HEIGHT),
    )
}

#[must_use]
pub const fn intent_allowed_at_size(intent: Intent, area: Size) -> bool {
    matches!(intent, Intent::Quit) || (area.width >= MIN_WIDTH && area.height >= MIN_HEIGHT)
}

/// Apply a transport-neutral UI intent to a game session.
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
        Intent::ToggleHelp => game.toggle_help(),
        Intent::Restart => game.restart()?,
        Intent::Start => game.start(),
        Intent::Move(direction) if !game.help => {
            game.apply(Command::Move(direction));
        }
        Intent::Wait if !game.help => {
            game.apply(Command::Wait);
        }
        Intent::Cleave if !game.help => {
            game.apply(Command::UseCleave);
        }
        _ => {}
    }
    Ok(true)
}

fn panel(title: &'static str, profile: DisplayProfile) -> Block<'static> {
    let block = Block::default().title(title).borders(Borders::ALL);
    if profile.ascii {
        block.border_set(ASCII_BORDER)
    } else {
        block
    }
}

/// Run the shared game locally.
///
/// # Errors
///
/// Returns terminal I/O errors after restoring terminal state.
pub fn run_local(seed: Option<RunSeed>, profile: DisplayProfile) -> io::Result<()> {
    let mut guard = LocalTerminalGuard::default();
    let mut stdout = io::stdout();
    guard.setup_with(&mut stdout, enable_raw_mode)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let mut game = ExplorationGame::new(seed).map_err(io::Error::other)?;
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
struct LocalTerminalGuard {
    raw_mode: bool,
    entered_alternate_screen: bool,
    cursor_hidden: bool,
}

impl LocalTerminalGuard {
    fn setup_with<W, F>(&mut self, writer: &mut W, enable_raw: F) -> io::Result<()>
    where
        W: Write,
        F: FnOnce() -> io::Result<()>,
    {
        enable_raw()?;
        self.raw_mode = true;

        // Record cleanup obligations before writing: an escape sequence can be
        // accepted by the terminal even when the following flush reports an
        // error.
        self.entered_alternate_screen = true;
        execute!(writer, EnterAlternateScreen)?;
        self.cursor_hidden = true;
        execute!(writer, Hide)?;
        Ok(())
    }

    fn restore_with<W, F>(&mut self, writer: &mut W, disable_raw: F)
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

fn intent_from_key(code: KeyCode) -> Option<Intent> {
    match code {
        KeyCode::Up => Some(Intent::Move(Direction::North)),
        KeyCode::Down => Some(Intent::Move(Direction::South)),
        KeyCode::Left => Some(Intent::Move(Direction::West)),
        KeyCode::Right => Some(Intent::Move(Direction::East)),
        KeyCode::Enter => Some(Intent::Start),
        KeyCode::Char(character) if character.is_ascii() => intent_from_byte(character as u8),
        _ => None,
    }
}

fn intent_from_byte(byte: u8) -> Option<Intent> {
    Some(match byte {
        b'\r' | b'\n' => Intent::Start,
        b'k' => Intent::Move(Direction::North),
        b'u' => Intent::Move(Direction::NorthEast),
        b'l' => Intent::Move(Direction::East),
        b'n' => Intent::Move(Direction::SouthEast),
        b'j' => Intent::Move(Direction::South),
        b'b' => Intent::Move(Direction::SouthWest),
        b'h' => Intent::Move(Direction::West),
        b'y' => Intent::Move(Direction::NorthWest),
        b'.' => Intent::Wait,
        b'a' => Intent::Cleave,
        b'?' => Intent::ToggleHelp,
        b'r' => Intent::Restart,
        b'q' => Intent::Quit,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn rendered_game(profile: DisplayProfile, status: RunStatus) -> String {
        rendered_game_at(80, 24, profile, status, false)
    }

    fn rendered_game_at(
        width: u16,
        height: u16,
        profile: DisplayProfile,
        status: RunStatus,
        help: bool,
    ) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut game = ExplorationGame::new(Some(RunSeed(0xD4_4B))).unwrap();
        game.start();
        game.status = status;
        game.help = help;
        if status == RunStatus::Death {
            game.player.health = 0;
            game.events.push("The Grave Knight falls.".into());
        }
        terminal
            .draw(|frame| draw_game(frame, &game, profile))
            .unwrap();
        terminal_text(&terminal, width, height)
    }

    fn rendered_enemy_showcase(profile: DisplayProfile) -> String {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut game = ExplorationGame::new(Some(RunSeed(0xD4_4B))).unwrap();
        game.start();
        let mut positions = game
            .visible
            .iter()
            .copied()
            .filter(|position| {
                *position != game.player.position && game.map.tile(*position) == Some(Tile::Floor)
            })
            .collect::<Vec<_>>();
        positions.sort_unstable();
        assert!(positions.len() >= 3);
        game.hostiles.truncate(3);
        for (actor, (kind, position)) in game.hostiles.iter_mut().zip([
            (crate::game::ActorKind::Skeleton, positions[0]),
            (crate::game::ActorKind::Ghoul, positions[1]),
            (crate::game::ActorKind::Cultist, positions[2]),
        ]) {
            actor.kind = kind;
            actor.position = position;
            actor.active = true;
            actor.telegraph = None;
        }
        game.hostiles[1].telegraph = Some(crate::game::Telegraph::GhoulLunge {
            target: game.player.position,
        });
        game.hostiles[2].telegraph = Some(crate::game::Telegraph::CultistHex {
            target: game.player.position,
        });
        game.events.push("Ghoul 2 marks a lunge.".into());
        game.events.push("Cultist 3 marks a hex.".into());
        terminal
            .draw(|frame| draw_game(frame, &game, profile))
            .unwrap();
        terminal_text(&terminal, 80, 24)
    }

    fn terminal_text(terminal: &Terminal<TestBackend>, width: u16, height: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                let mut row = String::new();
                for x in 0..width {
                    row.push_str(buffer[(x, y)].symbol());
                }
                row.trim_end().to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn decoder_handles_partial_arrows_and_ignores_unknown_input() {
        let mut decoder = InputDecoder::default();
        assert!(decoder.feed(b"\x1b[").is_empty());
        assert_eq!(decoder.feed(b"A"), vec![Intent::Move(Direction::North)]);
        assert_eq!(decoder.feed(b"x.a"), vec![Intent::Wait, Intent::Cleave]);
    }

    #[test]
    fn decoder_bounds_spam_linearly_and_recovers_for_later_input() {
        let mut decoder = InputDecoder::default();
        let spam = vec![b'.'; MAX_INPUT_BYTES_PER_FEED * 8];
        let intents = decoder.feed(&spam);
        assert_eq!(intents.len(), MAX_INTENTS_PER_FEED);
        assert!(intents.iter().all(|intent| *intent == Intent::Wait));
        assert_eq!(decoder.feed(b"q"), [Intent::Quit]);

        let unknown = vec![b'x'; MAX_INPUT_BYTES_PER_FEED * 8];
        assert!(decoder.feed(&unknown).is_empty());
        assert!(decoder.feed(b"\x1b[").is_empty());
        assert_eq!(decoder.feed(b"A"), [Intent::Move(Direction::North)]);
    }

    #[test]
    fn exploration_and_victory_profiles_are_snapshot_tested() {
        let ascii = DisplayProfile {
            ascii: true,
            no_color: true,
        };
        insta::assert_snapshot!(
            "exploration_active_unicode",
            rendered_game(DisplayProfile::default(), RunStatus::Active)
        );
        insta::assert_snapshot!(
            "exploration_active_ascii",
            rendered_game(ascii, RunStatus::Active)
        );
        insta::assert_snapshot!(
            "exploration_victory_unicode",
            rendered_game(DisplayProfile::default(), RunStatus::Victory)
        );
        insta::assert_snapshot!(
            "exploration_victory_ascii",
            rendered_game(ascii, RunStatus::Victory)
        );
        insta::assert_snapshot!(
            "combat_death_unicode",
            rendered_game(DisplayProfile::default(), RunStatus::Death)
        );
        insta::assert_snapshot!("combat_death_ascii", rendered_game(ascii, RunStatus::Death));
        insta::assert_snapshot!(
            "enemy_showcase_unicode",
            rendered_enemy_showcase(DisplayProfile::default())
        );
        insta::assert_snapshot!("enemy_showcase_ascii", rendered_enemy_showcase(ascii));
        insta::assert_snapshot!(
            "exploration_help_unicode",
            rendered_game_at(80, 24, DisplayProfile::default(), RunStatus::Active, true)
        );
        insta::assert_snapshot!(
            "exploration_help_ascii",
            rendered_game_at(80, 24, ascii, RunStatus::Active, true)
        );
        insta::assert_snapshot!(
            "exploration_small_unicode",
            rendered_game_at(60, 20, DisplayProfile::default(), RunStatus::Active, false)
        );
        insta::assert_snapshot!(
            "exploration_small_ascii",
            rendered_game_at(60, 20, ascii, RunStatus::Active, false)
        );
    }

    #[test]
    fn ascii_profile_never_renders_non_ascii_symbols() {
        let profile = DisplayProfile {
            ascii: true,
            no_color: true,
        };
        for output in [
            rendered_game(profile, RunStatus::Active),
            rendered_game(profile, RunStatus::Victory),
            rendered_game(profile, RunStatus::Death),
            rendered_enemy_showcase(profile),
            rendered_game_at(80, 24, profile, RunStatus::Active, true),
            rendered_game_at(60, 20, profile, RunStatus::Active, false),
        ] {
            assert!(output.is_ascii(), "strict ASCII output contained Unicode");
        }
    }

    #[test]
    fn unseen_features_are_hidden_and_explored_terrain_uses_memory_glyphs() {
        let mut game = ExplorationGame::new(Some(RunSeed(9))).unwrap();
        let exit = game.map.exit;
        game.visible.remove(&exit);
        game.explored.remove(&exit);
        assert_eq!(
            exploration_glyph(&game, exit, DisplayProfile::default()),
            ' '
        );
        game.explored.insert(exit);
        assert_eq!(
            exploration_glyph(&game, exit, DisplayProfile::default()),
            ','
        );
    }

    #[test]
    fn partial_terminal_setup_restores_every_possible_acquisition_once() {
        use std::{cell::Cell, rc::Rc};

        #[derive(Default)]
        struct InjectedFailureWriter {
            bytes: Vec<u8>,
            fail_flush: Option<usize>,
            flushes: usize,
            fail_write_after: Option<usize>,
        }

        impl Write for InjectedFailureWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if let Some(limit) = self.fail_write_after {
                    if self.bytes.len() >= limit {
                        self.fail_write_after = None;
                        return Err(io::Error::other("injected write failure"));
                    }
                    let accepted = bytes.len().min(limit - self.bytes.len());
                    self.bytes.extend_from_slice(&bytes[..accepted]);
                    return Ok(accepted);
                }
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                self.flushes += 1;
                if self.fail_flush == Some(self.flushes) {
                    Err(io::Error::other("injected flush failure"))
                } else {
                    Ok(())
                }
            }
        }

        let mut raw_failure = LocalTerminalGuard::default();
        let mut raw_output = InjectedFailureWriter::default();
        assert!(
            raw_failure
                .setup_with(&mut raw_output, || Err(io::Error::other("raw mode")))
                .is_err()
        );
        assert_eq!(raw_failure, LocalTerminalGuard::default());
        raw_failure.restore_with(&mut raw_output, || panic!("raw mode was never acquired"));
        assert!(raw_output.bytes.is_empty());

        for (failing_flush, expected_cleanup) in [
            (1, b"\x1b[?1049l".as_slice()),
            (2, b"\x1b[?25h\x1b[?1049l".as_slice()),
        ] {
            let disabled = Rc::new(Cell::new(0));
            let disabled_for_restore = Rc::clone(&disabled);
            let mut output = InjectedFailureWriter {
                fail_flush: Some(failing_flush),
                ..InjectedFailureWriter::default()
            };
            let mut guard = LocalTerminalGuard::default();
            assert!(guard.setup_with(&mut output, || Ok(())).is_err());
            let setup_len = output.bytes.len();

            guard.restore_with(&mut output, move || {
                disabled_for_restore.set(disabled_for_restore.get() + 1);
                Ok(())
            });

            assert_eq!(disabled.get(), 1);
            assert_eq!(&output.bytes[setup_len..], expected_cleanup);
            assert_eq!(guard, LocalTerminalGuard::default());

            guard.restore_with(&mut output, || panic!("restoration must be idempotent"));
            assert_eq!(disabled.get(), 1);
        }

        let enter_sequence_len = b"\x1b[?1049h".len();
        for (fail_after, expected_cleanup) in [
            (3, b"\x1b[?1049l".as_slice()),
            (enter_sequence_len + 3, b"\x1b[?25h\x1b[?1049l".as_slice()),
        ] {
            let disabled = Rc::new(Cell::new(0));
            let disabled_for_restore = Rc::clone(&disabled);
            let mut output = InjectedFailureWriter {
                fail_write_after: Some(fail_after),
                ..InjectedFailureWriter::default()
            };
            let mut guard = LocalTerminalGuard::default();
            assert!(guard.setup_with(&mut output, || Ok(())).is_err());
            let setup_len = output.bytes.len();

            guard.restore_with(&mut output, move || {
                disabled_for_restore.set(disabled_for_restore.get() + 1);
                Ok(())
            });

            assert_eq!(disabled.get(), 1);
            assert_eq!(&output.bytes[setup_len..], expected_cleanup);
            assert_eq!(guard, LocalTerminalGuard::default());
            guard.restore_with(&mut output, || panic!("restoration must be idempotent"));
            assert_eq!(disabled.get(), 1);
        }
    }

    #[test]
    fn dimensions_are_capped() {
        assert_eq!(
            capped_area(u32::MAX, u32::MAX),
            Rect::new(0, 0, MAX_WIDTH, MAX_HEIGHT)
        );
        assert_eq!(capped_area(0, 0), Rect::new(0, 0, 1, 1));
    }

    #[test]
    fn gameplay_intents_are_ignored_below_minimum_size() {
        let small = Size::new(60, 20);
        assert!(!intent_allowed_at_size(Intent::Wait, small));
        assert!(intent_allowed_at_size(Intent::Quit, small));
    }

    #[test]
    fn shared_game_intents_keep_sessions_independent() {
        let mut waiting = ExplorationGame::new(Some(RunSeed(11))).unwrap();
        let mut helping = ExplorationGame::new(Some(RunSeed(22))).unwrap();
        waiting.start();
        helping.start();

        assert!(apply_game_intent(&mut waiting, Intent::Wait).unwrap());
        assert!(apply_game_intent(&mut helping, Intent::ToggleHelp).unwrap());

        assert_eq!(waiting.turn, 1);
        assert!(!waiting.help);
        assert_eq!(helping.turn, 0);
        assert!(helping.help);
        assert_ne!(waiting.seed(), helping.seed());
    }
}
