//! Application configuration shared by the executable and its transports.

use std::{net::SocketAddr, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{game::RunSeed, ui::DisplayProfile};

/// Complete startup configuration after command-line parsing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AppMode {
    /// Run directly in the current terminal.
    Play(PlayOptions),
    /// Listen for SSH game sessions.
    Serve(ServeOptions),
    /// Serve independent game sessions in browser terminal viewports.
    Web(WebOptions),
}

/// Options for a local game session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlayOptions {
    pub seed: Option<RunSeed>,
    pub display: DisplayProfile,
    pub debug_godmode: bool,
}

/// Options for the SSH service.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServeOptions {
    pub listen: SocketAddr,
    pub host_key: PathBuf,
    pub seed: Option<RunSeed>,
    pub display: DisplayProfile,
    pub debug_godmode: bool,
}

/// Options for the unauthenticated development web service.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WebOptions {
    pub listen: SocketAddr,
    pub seed: Option<RunSeed>,
    pub display: DisplayProfile,
    pub debug_godmode: bool,
}
