//! Versioned browser-session wire messages and validation.

use serde::{Deserialize, Serialize};

pub(super) const PROTOCOL_VERSION: u8 = 1;
pub(super) const MAX_CLIENT_MESSAGE_BYTES: usize = 8192;
pub(super) const MAX_INPUT_BYTES: usize = 4096;

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "type", deny_unknown_fields)]
pub(super) enum ClientCommand {
    #[serde(rename = "hello")]
    Hello { v: u8, cols: u32, rows: u32 },
    #[serde(rename = "resize")]
    Resize { v: u8, cols: u32, rows: u32 },
    #[serde(rename = "input")]
    Input { v: u8, data: String },
    #[serde(rename = "quit")]
    Quit { v: u8 },
}

impl ClientCommand {
    pub(super) const fn version(&self) -> u8 {
        match self {
            Self::Hello { v, .. }
            | Self::Resize { v, .. }
            | Self::Input { v, .. }
            | Self::Quit { v } => *v,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub(super) enum ServerCommand<'a> {
    #[serde(rename = "ready")]
    Ready { v: u8, seed: u64 },
    #[serde(rename = "state")]
    State { v: u8, targeting: bool },
    #[serde(rename = "error")]
    Error { v: u8, message: &'a str },
}

pub(super) fn decode_client_command(text: &str) -> Result<ClientCommand, &'static str> {
    if text.len() > MAX_CLIENT_MESSAGE_BYTES {
        return Err("message is too large");
    }
    let command = serde_json::from_str::<ClientCommand>(text).map_err(|_| "malformed message")?;
    if let ClientCommand::Input { data, .. } = &command
        && data.len() > MAX_INPUT_BYTES
    {
        return Err("input is too large");
    }
    Ok(command)
}
