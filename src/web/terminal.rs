//! ANSI output backend backed by a bounded WebSocket queue.

use ratatui::{
    backend::{Backend, ClearType, CrosstermBackend, WindowSize},
    buffer::Cell,
    layout::{Position, Size},
};
use std::io::{self, Write};
use tokio::sync::mpsc::{self, error::TrySendError};

const MAX_OUTPUT_FRAME_BYTES: usize = 1_048_576;

#[derive(Debug)]
pub(super) enum Outbound {
    Output(Vec<u8>),
    Text(String),
    Close,
}

pub(super) struct WebTerminalHandle {
    sender: mpsc::Sender<Outbound>,
    sink: Vec<u8>,
}

impl WebTerminalHandle {
    pub(super) fn new(sender: mpsc::Sender<Outbound>) -> Self {
        Self {
            sender,
            sink: Vec::new(),
        }
    }
}

impl Write for WebTerminalHandle {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.sink.len().saturating_add(bytes.len()) > MAX_OUTPUT_FRAME_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "web terminal output frame is too large",
            ));
        }
        self.sink.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.sink.is_empty() {
            return Ok(());
        }
        self.sink.extend_from_slice(b"\x1b[?25h");
        match self
            .sender
            .try_send(Outbound::Output(std::mem::take(&mut self.sink)))
        {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "web terminal output queue is full",
            )),
            Err(TrySendError::Closed(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "web terminal output channel is closed",
            )),
        }
    }
}

/// Crossterm normally discovers its dimensions from the process TTY. Browser
/// sessions have no TTY, so retain the dimensions reported by xterm.js while
/// delegating ANSI rendering to `CrosstermBackend`.
pub(super) struct WebBackend {
    inner: CrosstermBackend<WebTerminalHandle>,
    size: Size,
    cursor: Position,
}

impl WebBackend {
    pub(super) fn new(writer: WebTerminalHandle, size: Size) -> Self {
        Self {
            inner: CrosstermBackend::new(writer),
            size,
            cursor: Position::ORIGIN,
        }
    }

    pub(super) const fn set_size(&mut self, size: Size) {
        self.size = size;
    }
}

impl Backend for WebBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.inner.draw(content)
    }

    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        let position = position.into();
        self.inner.set_cursor_position(position)?;
        self.cursor = position;
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        Ok(self.size)
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        Ok(WindowSize {
            columns_rows: self.size,
            pixels: Size::ZERO,
        })
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Backend::flush(&mut self.inner)
    }
}
