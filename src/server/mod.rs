//! Application-owned SSH transport for isolated terminal sessions.

mod host_key;
mod terminal;

use crate::{
    app::ServeOptions,
    game::ExplorationGame,
    session::{InputDecoder, apply_game_intent},
    ui::{DisplayProfile, capped_area, draw_game, intent_allowed_at_size},
};
use anyhow::Result;
use host_key::load_or_create_host_key;
use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend, layout::Rect};
use russh::{
    Channel, ChannelId, ChannelOpenFailure, Pty,
    server::{self, Auth, ChannelOpenHandle, Handler, Msg, Server as _, Session},
};
use std::{collections::HashMap, io, net::SocketAddr, path::Path, sync::Arc, time::Duration};
#[cfg(test)]
use terminal::SSH_CURSOR_SHOW;
use terminal::TerminalHandle;
use thiserror::Error;
use tokio::sync::Mutex;
#[cfg(test)]
use tokio::sync::mpsc::channel as mpsc_channel;

pub const DEFAULT_LISTEN: &str = "127.0.0.1:2222";
pub const DEFAULT_HOST_KEY: &str = ".terminal-rpg/host-key";

#[derive(Debug, Error)]
pub enum ServerConfigError {
    #[error("host-key path must name a file")]
    MissingHostKeyName,
}

/// Validate configuration independent of the SSH runtime.
///
/// # Errors
///
/// Returns [`ServerConfigError::MissingHostKeyName`] for a path without a file name.
pub fn validate_host_key_path(path: &Path) -> Result<(), ServerConfigError> {
    if path.file_name().is_none() {
        return Err(ServerConfigError::MissingHostKeyName);
    }
    Ok(())
}

type SshTerminal = Terminal<CrosstermBackend<TerminalHandle>>;

struct Client {
    channel: ChannelId,
    terminal: SshTerminal,
    game: ExplorationGame,
    decoder: InputDecoder,
    has_pty: bool,
    started: bool,
}

fn draw_client(client: &mut Client, display: DisplayProfile) -> io::Result<()> {
    let Client { terminal, game, .. } = client;
    terminal.draw(|frame| draw_game(frame, game, display))?;
    Ok(())
}

type SharedClient = Arc<Mutex<Client>>;

#[derive(Clone)]
struct GameServer {
    clients: Arc<Mutex<HashMap<usize, SharedClient>>>,
    display: DisplayProfile,
    id: usize,
    seed: Option<crate::game::RunSeed>,
    debug_godmode: bool,
}

impl GameServer {
    fn new(
        display: DisplayProfile,
        seed: Option<crate::game::RunSeed>,
        debug_godmode: bool,
    ) -> Self {
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            display,
            id: 0,
            seed,
            debug_godmode,
        }
    }
}

impl server::Server for GameServer {
    type Handler = Self;
    fn new_client(&mut self, peer_addr: Option<SocketAddr>) -> Self {
        self.id += 1;
        tracing::info!(client_id = self.id, ?peer_addr, "SSH client connected");
        Self {
            clients: Arc::clone(&self.clients),
            display: self.display,
            id: self.id,
            seed: self.seed,
            debug_godmode: self.debug_godmode,
        }
    }
}

impl Handler for GameServer {
    type Error = anyhow::Error;

    async fn auth_none(&mut self, user: &str) -> Result<Auth, Self::Error> {
        tracing::info!(
            client_id = self.id,
            user,
            "accepted development none authentication"
        );
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.clients.lock().await.contains_key(&self.id) {
            reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        let backend = CrosstermBackend::new(TerminalHandle::start(session.handle(), channel.id()));
        let terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::default()),
            },
        )?;
        let mut game = ExplorationGame::new(self.seed)?;
        game.set_debug_godmode_enabled(self.debug_godmode);
        game.start();
        self.clients.lock().await.insert(
            self.id,
            Arc::new(Mutex::new(Client {
                channel: channel.id(),
                terminal,
                game,
                decoder: InputDecoder::default(),
                has_pty: false,
                started: false,
            })),
        );
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        columns: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(shared) = self.client().await else {
            session.channel_failure(channel)?;
            return Ok(());
        };
        let mut client = shared.lock().await;
        if client.channel != channel {
            session.channel_failure(channel)?;
            return Ok(());
        }
        client.terminal.resize(capped_area(columns, rows))?;
        client.has_pty = true;
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(shared) = self.client().await else {
            session.channel_failure(channel)?;
            return Ok(());
        };
        let mut client = shared.lock().await;
        if client.channel != channel {
            session.channel_failure(channel)?;
            return Ok(());
        }
        if !client.has_pty {
            session.channel_failure(channel)?;
            return Ok(());
        }
        client.started = true;
        draw_client(&mut client, self.display)?;
        session.channel_success(channel)?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(shared) = self.client().await else {
            return Ok(());
        };
        let mut close = false;
        {
            let mut client = shared.lock().await;
            if client.channel != channel || !client.started {
                return Ok(());
            }
            for intent in client.decoder.feed(data) {
                let area = client.terminal.size()?;
                if intent_allowed_at_size(intent, area)
                    && !apply_game_intent(&mut client.game, intent)?
                {
                    close = true;
                    break;
                }
            }
            if !close {
                draw_client(&mut client, self.display)?;
            }
        }
        if !close {
            let generation = {
                let client = shared.lock().await;
                client.decoder.pending_escape_generation()
            };
            if let Some(generation) = generation {
                let pending_client = Arc::clone(&shared);
                let display = self.display;
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    let mut client = pending_client.lock().await;
                    if let Some(intent) = client.decoder.flush_pending_escape_generation(generation)
                        && apply_game_intent(&mut client.game, intent).is_ok()
                    {
                        let _ = draw_client(&mut client, display);
                    }
                });
            }
        }
        if close {
            {
                let mut client = shared.lock().await;
                let _ = crossterm::execute!(client.terminal.backend_mut(), crossterm::cursor::Show);
            }
            self.remove_client(&shared).await;
            session.exit_status_request(channel, 0)?;
            session.close(channel)?;
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        columns: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(shared) = self.client().await {
            let mut client = shared.lock().await;
            if client.channel != channel {
                return Ok(());
            }
            client.terminal.resize(capped_area(columns, rows))?;
            if client.started {
                draw_client(&mut client, self.display)?;
            }
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.release_channel(channel).await;
        session.close(channel)?;
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.release_channel(channel).await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        _: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)?;
        Ok(())
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        _: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)?;
        Ok(())
    }

    async fn x11_request(
        &mut self,
        channel: ChannelId,
        _: bool,
        _: &str,
        _: &str,
        _: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)?;
        Ok(())
    }
}

impl GameServer {
    async fn client(&self) -> Option<SharedClient> {
        self.clients.lock().await.get(&self.id).cloned()
    }

    async fn remove_client(&self, expected: &SharedClient) {
        let mut clients = self.clients.lock().await;
        if clients
            .get(&self.id)
            .is_some_and(|current| Arc::ptr_eq(current, expected))
        {
            clients.remove(&self.id);
        }
    }

    async fn release_channel(&mut self, channel: ChannelId) {
        let Some(shared) = self.client().await else {
            return;
        };
        {
            let mut client = shared.lock().await;
            if client.channel != channel {
                return;
            }
            if client.started {
                let _ = crossterm::execute!(client.terminal.backend_mut(), crossterm::cursor::Show);
            }
        }
        self.remove_client(&shared).await;
        tracing::info!(
            client_id = self.id,
            channel = u32::from(channel),
            "SSH game channel released"
        );
    }
}

impl Drop for GameServer {
    fn drop(&mut self) {
        if self.id == 0 {
            return;
        }
        let id = self.id;
        let clients = Arc::clone(&self.clients);
        tokio::spawn(async move {
            clients.lock().await.remove(&id);
            tracing::info!(client_id = id, "SSH client session released");
        });
    }
}

/// Run the development SSH game server until shutdown.
///
/// # Errors
///
/// Returns host-key loading/creation, bind, or server runtime failures.
pub async fn serve(options: ServeOptions) -> Result<()> {
    let key = load_or_create_host_key(&options.host_key)?;
    let config = server::Config {
        inactivity_timeout: Some(Duration::from_hours(1)),
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![key],
        nodelay: true,
        ..Default::default()
    };
    tracing::info!(listen = %options.listen, "starting development SSH server");
    GameServer::new(options.display, options.seed, options.debug_godmode)
        .run_on_address(Arc::new(config), options.listen)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write};

    #[test]
    fn terminal_output_saturation_is_not_acknowledged() {
        let (sender, mut receiver) = mpsc_channel(1);
        let mut terminal = TerminalHandle::from_sender(sender);

        terminal.write_all(b"first").unwrap();
        terminal.flush().unwrap();
        terminal.write_all(b"rejected").unwrap();
        assert_eq!(
            terminal.flush().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );

        assert_eq!(receiver.try_recv().unwrap(), b"first\x1b[?25h");
        assert!(receiver.try_recv().is_err());

        drop(receiver);
        terminal.write_all(b"closed").unwrap();
        assert_eq!(
            terminal.flush().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn ratatui_draw_surfaces_output_saturation() {
        use ratatui::widgets::Paragraph;

        let (sender, mut receiver) = mpsc_channel(1);
        let backend = CrosstermBackend::new(TerminalHandle::from_sender(sender));
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 20, 2)),
            },
        )
        .unwrap();

        terminal
            .draw(|frame| frame.render_widget(Paragraph::new("first frame"), frame.area()))
            .unwrap();
        let error = terminal
            .draw(|frame| frame.render_widget(Paragraph::new("second frame"), frame.area()))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        let delivered = receiver.try_recv().unwrap();
        assert!(delivered.ends_with(SSH_CURSOR_SHOW));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn host_key_is_persisted_and_reloaded() {
        let path = std::env::temp_dir().join(format!(
            "terminal-rpg-host-key-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let first = load_or_create_host_key(&path).unwrap();
        let second = load_or_create_host_key(&path).unwrap();
        assert_eq!(first.public_key(), second.public_key());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn ssh_escape_timer_generation_cannot_flush_a_later_sequence() {
        let mut decoder = InputDecoder::default();
        assert!(decoder.feed(b"\x1b").is_empty());
        let stale = decoder.pending_escape_generation().unwrap();
        assert_eq!(
            decoder.feed(b"[A"),
            [crate::session::Intent::Move(crate::game::Direction::North)]
        );
        assert!(decoder.feed(b"\x1b").is_empty());
        let current = decoder.pending_escape_generation().unwrap();
        assert_ne!(stale, current);
        assert_eq!(decoder.flush_pending_escape_generation(stale), None);
        assert_eq!(
            decoder.flush_pending_escape_generation(current),
            Some(crate::session::Intent::CancelMode)
        );
    }
}
