#![deny(unused_imports, unused_must_use)]

//! # Terminal manipulation, trimmed to what this game uses
//!
//! A local copy of [crossterm](https://github.com/crossterm-rs/crossterm) 0.29.0, patched for
//! binary size and stripped of everything the game does not link. See `LOCAL_PATCH.md` next to
//! this directory for the change list and the rationale.
//!
//! What is left:
//!
//! * [`event::next`] — block until the terminal produces a key press, a left click or a resize.
//! * [`event::EnableMouseCapture`] / [`event::DisableMouseCapture`] — the only two [`Command`]s,
//!   run through the [`execute!`] macro.
//! * [`terminal::size`] and, on Unix, [`terminal::enable_raw_mode`] /
//!   [`terminal::disable_raw_mode`].

pub use crate::command::{Command, QueueableCommand};

/// A module to read events.
pub mod event;
/// A module to work with the terminal.
pub mod terminal;

mod command;
pub(crate) mod macros;
