//! Unauthenticated, loopback-first browser terminal transport.

mod assets;
mod protocol;
mod terminal;

use crate::{
    app::WebOptions,
    game::ExplorationGame,
    session::{InputDecoder, apply_game_intent},
    ui::{DisplayProfile, capped_area, draw_game, intent_allowed_at_size},
};
use anyhow::Context;
#[cfg(test)]
use assets::CONTENT_SECURITY_POLICY;
use assets::{
    addon_fit_js, app_css, app_js, index, not_found, secured_response, security_headers, xterm_css,
    xterm_js,
};
#[cfg(test)]
use axum::http::HeaderValue;
use axum::{
    Router,
    body::Body,
    extract::{
        State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    middleware,
    response::Response,
    routing::get,
};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
#[cfg(test)]
use protocol::MAX_INPUT_BYTES;
use protocol::{
    ClientCommand, MAX_CLIENT_MESSAGE_BYTES, PROTOCOL_VERSION, ServerCommand, decode_client_command,
};
use ratatui::{Terminal, TerminalOptions, Viewport, backend::Backend, layout::Rect};
use std::{io, time::Duration};
use terminal::{Outbound, WebBackend, WebTerminalHandle};
use tokio::{net::TcpListener, sync::mpsc};

pub const DEFAULT_WEB_LISTEN: &str = "127.0.0.1:8080";
const WEB_OUTPUT_QUEUE_FRAMES: usize = 16;
const WEB_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

type WebTerminal = Terminal<WebBackend>;

#[derive(Clone, Copy)]
struct WebState {
    display: DisplayProfile,
    seed: Option<crate::game::RunSeed>,
    debug_godmode: bool,
}

/// Serve the unauthenticated browser viewport.
///
/// # Errors
///
/// Returns listener bind and HTTP service errors.
pub async fn serve(options: WebOptions) -> anyhow::Result<()> {
    let listener = TcpListener::bind(options.listen)
        .await
        .with_context(|| format!("bind development web listener at {}", options.listen))?;
    tracing::warn!(
        listen = %options.listen,
        "serving unauthenticated development browser access"
    );
    axum::serve(
        listener,
        router_with_seed(options.display, options.seed, options.debug_godmode),
    )
    .await
    .context("serve browser viewport")
}

#[cfg(test)]
fn router(display: DisplayProfile) -> Router {
    router_with_seed(display, None, false)
}

fn router_with_seed(
    display: DisplayProfile,
    seed: Option<crate::game::RunSeed>,
    debug_godmode: bool,
) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.css", get(app_css))
        .route("/app.js", get(app_js))
        .route("/vendor/xterm-6.0.0.css", get(xterm_css))
        .route("/vendor/xterm-6.0.0.mjs", get(xterm_js))
        .route("/vendor/addon-fit-0.11.0.mjs", get(addon_fit_js))
        .route("/ws", get(websocket_upgrade))
        .fallback(not_found)
        .with_state(WebState {
            display,
            seed,
            debug_godmode,
        })
        .layer(middleware::from_fn(security_headers))
}

async fn websocket_upgrade(
    State(state): State<WebState>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    if !same_origin(&headers) {
        return secured_response(
            StatusCode::FORBIDDEN,
            Body::from("same-origin WebSocket required"),
        );
    }
    websocket
        .max_message_size(MAX_CLIENT_MESSAGE_BYTES)
        .max_frame_size(MAX_CLIENT_MESSAGE_BYTES)
        .on_upgrade(move |socket| session(socket, state.display, state.seed, state.debug_godmode))
}

fn same_origin(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    origin == format!("http://{host}") || origin == format!("https://{host}")
}

async fn session(
    socket: WebSocket,
    display: DisplayProfile,
    seed: Option<crate::game::RunSeed>,
    debug_godmode: bool,
) {
    let (websocket_sender, mut websocket_receiver) = socket.split();
    let (output_sender, output_receiver) = mpsc::channel(WEB_OUTPUT_QUEUE_FRAMES);
    let writer = tokio::spawn(output_writer(websocket_sender, output_receiver));
    let Some((mut terminal, mut game)) = initialize_session(
        &mut websocket_receiver,
        &output_sender,
        display,
        seed,
        debug_godmode,
    )
    .await
    else {
        drop(output_sender);
        let _ = writer.await;
        return;
    };

    let mut decoder = InputDecoder::default();
    loop {
        let message = if decoder.has_pending_escape() {
            tokio::select! {
                message = websocket_receiver.next() => message,
                () = tokio::time::sleep(Duration::from_millis(30)) => {
                    if let Some(intent) = decoder.flush_pending_escape()
                        && apply_game_intent(&mut game, intent).is_ok()
                    {
                        let _ = draw(&mut terminal, &game, display);
                    }
                    continue;
                }
            }
        } else {
            websocket_receiver.next().await
        };
        let Some(message) = message else {
            break;
        };
        let command = match message {
            Ok(Message::Text(text)) => match decode_client_command(text.as_str()) {
                Ok(command) => command,
                Err(message) => {
                    send_protocol_error(&output_sender, message).await;
                    break;
                }
            },
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            Ok(Message::Binary(_)) => {
                send_protocol_error(&output_sender, "binary client messages are unsupported").await;
                break;
            }
        };
        if command.version() != PROTOCOL_VERSION {
            send_protocol_error(&output_sender, "unsupported protocol version").await;
            break;
        }
        let mut close = false;
        let result = match command {
            ClientCommand::Hello { .. } => Err("hello is only valid once"),
            ClientCommand::Resize { cols, rows, .. } => {
                resize_terminal(&mut terminal, capped_area(cols, rows))
                    .and_then(|()| draw(&mut terminal, &game, display))
                    .map_err(|_| "terminal output failed")
            }
            ClientCommand::Input { data, .. } => apply_input(
                &mut terminal,
                &mut game,
                &mut decoder,
                data.as_bytes(),
                display,
            )
            .map(|should_close| close = should_close),
            ClientCommand::Quit { .. } => {
                close = true;
                Ok(())
            }
        };
        if let Err(message) = result {
            send_protocol_error(&output_sender, message).await;
            break;
        }
        if close {
            let _ = terminal.backend_mut().show_cursor();
            let _ = output_sender.send(Outbound::Close).await;
            break;
        }
    }
    drop(terminal);
    drop(output_sender);
    let _ = writer.await;
}

async fn output_writer(
    mut websocket_sender: SplitSink<WebSocket, Message>,
    mut output_receiver: mpsc::Receiver<Outbound>,
) {
    while let Some(outbound) = output_receiver.recv().await {
        let closes = matches!(outbound, Outbound::Close);
        let message = match outbound {
            Outbound::Output(bytes) => Message::Binary(bytes.into()),
            Outbound::Text(text) => Message::Text(text.into()),
            Outbound::Close => Message::Close(Some(CloseFrame {
                code: 1000,
                reason: "game closed".into(),
            })),
        };
        let sent = tokio::time::timeout(WEB_WRITE_TIMEOUT, websocket_sender.send(message))
            .await
            .is_ok_and(|result| result.is_ok());
        if !sent || closes {
            break;
        }
    }
}

async fn initialize_session(
    receiver: &mut SplitStream<WebSocket>,
    sender: &mpsc::Sender<Outbound>,
    display: DisplayProfile,
    seed: Option<crate::game::RunSeed>,
    debug_godmode: bool,
) -> Option<(WebTerminal, ExplorationGame)> {
    let Some(Ok(Message::Text(text))) = receiver.next().await else {
        send_protocol_error(sender, "hello must be the first message").await;
        return None;
    };
    let Ok(ClientCommand::Hello { v, cols, rows }) = decode_client_command(text.as_str()) else {
        send_protocol_error(sender, "valid hello must be the first message").await;
        return None;
    };
    if v != PROTOCOL_VERSION {
        send_protocol_error(sender, "unsupported protocol version").await;
        return None;
    }
    let area = capped_area(cols, rows);
    let backend = WebBackend::new(WebTerminalHandle::new(sender.clone()), area.as_size());
    let Ok(mut terminal) = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    ) else {
        send_protocol_error(sender, "could not create terminal").await;
        return None;
    };
    let Ok(mut game) = ExplorationGame::new(seed) else {
        send_protocol_error(sender, "could not create game").await;
        return None;
    };
    game.set_debug_godmode_enabled(debug_godmode);
    game.start();
    let ready = serde_json::to_string(&ServerCommand::Ready {
        v: PROTOCOL_VERSION,
        seed: game.seed().0,
    })
    .expect("server protocol is serializable");
    if sender.send(Outbound::Text(ready)).await.is_err()
        || draw(&mut terminal, &game, display).is_err()
    {
        return None;
    }
    Some((terminal, game))
}

fn resize_terminal(terminal: &mut WebTerminal, area: Rect) -> io::Result<()> {
    terminal.backend_mut().set_size(area.as_size());
    terminal.resize(area)
}

fn apply_input(
    terminal: &mut WebTerminal,
    game: &mut ExplorationGame,
    decoder: &mut InputDecoder,
    bytes: &[u8],
    display: DisplayProfile,
) -> Result<bool, &'static str> {
    for intent in decoder.feed(bytes) {
        let area = terminal.size().map_err(|_| "terminal state unavailable")?;
        if intent_allowed_at_size(intent, area)
            && !apply_game_intent(game, intent).map_err(|_| "game command failed")?
        {
            return Ok(true);
        }
    }
    draw(terminal, game, display).map_err(|_| "terminal output failed")?;
    Ok(false)
}

fn draw(
    terminal: &mut WebTerminal,
    game: &ExplorationGame,
    display: DisplayProfile,
) -> io::Result<()> {
    terminal.draw(|frame| draw_game(frame, game, display))?;
    Ok(())
}

async fn send_protocol_error(sender: &mpsc::Sender<Outbound>, message: &'static str) {
    let serialized = serde_json::to_string(&ServerCommand::Error {
        v: PROTOCOL_VERSION,
        message,
    })
    .expect("server protocol is serializable");
    let _ = sender.send(Outbound::Text(serialized)).await;
    let _ = sender.send(Outbound::Close).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::to_bytes,
        http::{Method, Request},
    };
    use std::io::Write;
    use tower::ServiceExt;

    #[tokio::test]
    async fn http_assets_have_security_and_cache_headers() {
        let app = router(DisplayProfile::default());
        for (path, content_type, cache) in [
            ("/", "text/html; charset=utf-8", "no-store"),
            ("/app.js", "text/javascript; charset=utf-8", "no-store"),
            (
                "/vendor/xterm-6.0.0.mjs",
                "text/javascript; charset=utf-8",
                "public",
            ),
            (
                "/vendor/xterm-6.0.0.css",
                "text/css; charset=utf-8",
                "public",
            ),
            (
                "/vendor/addon-fit-0.11.0.mjs",
                "text/javascript; charset=utf-8",
                "public",
            ),
        ] {
            let response = app
                .clone()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], content_type);
            assert!(
                response.headers()[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .starts_with(cache)
            );
            assert_eq!(
                response.headers()[header::CONTENT_SECURITY_POLICY],
                CONTENT_SECURITY_POLICY
            );
            assert_eq!(
                response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                "nosniff"
            );
            let body = to_bytes(response.into_body(), 1_000_000).await.unwrap();
            assert!(!body.is_empty());
        }
    }

    #[tokio::test]
    async fn routes_reject_unknown_methods_paths_and_invalid_upgrades() {
        let app = router(DisplayProfile::default());
        let unknown = app
            .clone()
            .oneshot(Request::get("/missing").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        assert_eq!(unknown.headers()[header::CACHE_CONTROL], "no-store");

        let method = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(method.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            method.headers()[header::CONTENT_SECURITY_POLICY],
            CONTENT_SECURITY_POLICY
        );

        let upgrade = app
            .oneshot(Request::get("/ws").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(upgrade.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn protocol_validates_versions_shapes_sizes_and_dimensions() {
        assert_eq!(
            decode_client_command(r#"{"type":"hello","v":1,"cols":80,"rows":24}"#),
            Ok(ClientCommand::Hello {
                v: 1,
                cols: 80,
                rows: 24,
            })
        );
        assert!(decode_client_command(r#"{"type":"bogus","v":1}"#).is_err());
        assert!(decode_client_command(r#"{"type":"input","v":1,"data":"s","extra":1}"#).is_err());
        let oversized = format!(
            r#"{{"type":"input","v":1,"data":"{}"}}"#,
            "s".repeat(MAX_INPUT_BYTES + 1)
        );
        assert_eq!(decode_client_command(&oversized), Err("input is too large"));
        assert_eq!(capped_area(u32::MAX, u32::MAX), Rect::new(0, 0, 300, 120));
    }

    #[test]
    fn websocket_origin_must_exactly_match_the_http_host() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8080"));
        assert!(!same_origin(&headers));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://example.invalid"),
        );
        assert!(!same_origin(&headers));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:8080"),
        );
        assert!(same_origin(&headers));
    }

    #[tokio::test]
    async fn output_saturation_is_reported_without_advancing_terminal_state() {
        let (sender, _receiver) = mpsc::channel(1);
        let mut handle = WebTerminalHandle::new(sender);
        handle.write_all(b"first").unwrap();
        handle.flush().unwrap();
        handle.write_all(b"second").unwrap();
        assert_eq!(
            handle.flush().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn browser_terminal_resizes_without_a_host_tty() {
        let (sender, _receiver) = mpsc::channel(WEB_OUTPUT_QUEUE_FRAMES);
        let initial = Rect::new(0, 0, 80, 24);
        let backend = WebBackend::new(WebTerminalHandle::new(sender), initial.as_size());
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(initial),
            },
        )
        .unwrap();

        let resized = Rect::new(0, 0, 100, 30);
        resize_terminal(&mut terminal, resized).unwrap();

        assert_eq!(terminal.size().unwrap(), resized.as_size());
    }

    #[test]
    fn browser_adapter_preserves_fragmented_targeting_input_and_session_isolation() {
        let mut game = (0..500)
            .find_map(|seed| {
                let mut game = ExplorationGame::new(Some(crate::game::RunSeed(seed))).unwrap();
                game.start();
                game.hostiles
                    .iter()
                    .any(|actor| {
                        matches!(
                            game.target_validity(
                                crate::game::AbilitySlot::new(2).unwrap(),
                                actor.position
                            ),
                            crate::game::TargetValidity::Valid(_)
                        )
                    })
                    .then_some(game)
            })
            .expect("representative seeds contain a valid Grave Bolt target");
        let isolated = game.clone();
        let (sender, _receiver) = mpsc::channel(128);
        let area = Rect::new(0, 0, 80, 24);
        let backend = WebBackend::new(WebTerminalHandle::new(sender), area.as_size());
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )
        .unwrap();
        let mut decoder = InputDecoder::default();
        apply_input(
            &mut terminal,
            &mut game,
            &mut decoder,
            b"2",
            DisplayProfile::default(),
        )
        .unwrap();
        assert!(game.targeting.is_some());
        apply_input(
            &mut terminal,
            &mut game,
            &mut decoder,
            b"\x1b",
            DisplayProfile::default(),
        )
        .unwrap();
        apply_input(
            &mut terminal,
            &mut game,
            &mut decoder,
            b"[C",
            DisplayProfile::default(),
        )
        .unwrap();
        assert!(game.targeting.is_some());
        apply_input(
            &mut terminal,
            &mut game,
            &mut decoder,
            b"\x1b[",
            DisplayProfile::default(),
        )
        .unwrap();
        apply_input(
            &mut terminal,
            &mut game,
            &mut decoder,
            b"Z",
            DisplayProfile::default(),
        )
        .unwrap();
        assert!(game.targeting.is_some());
        let generation = decoder.pending_escape_generation();
        assert!(generation.is_none());
        apply_input(
            &mut terminal,
            &mut game,
            &mut decoder,
            b"\x1b",
            DisplayProfile::default(),
        )
        .unwrap();
        let intent = decoder.flush_pending_escape().unwrap();
        apply_game_intent(&mut game, intent).unwrap();
        assert!(game.targeting.is_none());
        assert_eq!(isolated.targeting, None);
        assert_eq!(isolated.turn, 0);
    }
}
