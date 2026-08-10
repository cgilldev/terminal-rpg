//! Application configuration shared by the executable and future transports.

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
}

/// Options for a local game session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlayOptions {
    pub seed: Option<RunSeed>,
    pub display: DisplayProfile,
}

/// Options for the SSH service.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServeOptions {
    pub listen: SocketAddr,
    pub host_key: PathBuf,
    pub display: DisplayProfile,
}
