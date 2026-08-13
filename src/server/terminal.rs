//! Bounded SSH terminal output and cursor-safety policy.

use russh::{ChannelId, server};
use std::io::{self, Write};
use tokio::sync::mpsc::{Sender, channel as mpsc_channel, error::TrySendError};

const SSH_OUTPUT_QUEUE_FRAMES: usize = 64;
pub(super) const SSH_CURSOR_SHOW: &[u8] = b"\x1b[?25h";

pub(super) struct TerminalHandle {
    sender: Sender<Vec<u8>>,
    sink: Vec<u8>,
}

impl TerminalHandle {
    pub(super) fn start(handle: server::Handle, channel: ChannelId) -> Self {
        let (sender, mut receiver) = mpsc_channel::<Vec<u8>>(SSH_OUTPUT_QUEUE_FRAMES);
        tokio::spawn(async move {
            while let Some(data) = receiver.recv().await {
                if handle.data(channel, data).await.is_err() {
                    break;
                }
            }
        });
        Self::from_sender(sender)
    }

    pub(super) fn from_sender(sender: Sender<Vec<u8>>) -> Self {
        Self {
            sender,
            sink: Vec::new(),
        }
    }
}

impl Write for TerminalHandle {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.sink.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.sink.is_empty() {
            return Ok(());
        }
        // Every deliverable SSH frame is independently cursor-safe. If the
        // following enqueue fails, the rejected bytes never reached the remote
        // and its last successful frame already left the cursor visible.
        self.sink.extend_from_slice(SSH_CURSOR_SHOW);
        match self.sender.try_send(std::mem::take(&mut self.sink)) {
            Ok(()) => Ok(()),
            // Ratatui emits incremental terminal diffs. Reporting success after
            // dropping one would advance its buffer beyond what the client saw,
            // so saturation deliberately fails only this SSH session.
            Err(TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "SSH terminal output queue is full",
            )),
            Err(TrySendError::Closed(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "SSH terminal output channel is closed",
            )),
        }
    }
}
