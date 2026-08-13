//! Reusable game, world, presentation, and server-facing types.
//!
//! The synchronous game and world modules deliberately do not depend on
//! terminal, SSH, or async runtime libraries.

pub mod app;
pub mod game;
pub mod server;
pub mod session;
pub mod ui;
pub mod web;
pub mod world;
