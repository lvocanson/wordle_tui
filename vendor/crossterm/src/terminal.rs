//! # Terminal
//!
//! This module exposes the two terminal facilities the game uses: the current size, and Unix raw
//! mode.
//!
//! ## Raw mode
//!
//! By default the terminal behaves in a certain way. It buffers input until Enter is pressed,
//! echoes what is typed, and interprets control characters itself. Raw mode turns all of that off
//! so the program sees every keystroke as it happens.
//!
//! On Windows there is nothing to call here: the console input mode that
//! [`event::EnableMouseCapture`](crate::event::EnableMouseCapture) assigns already leaves the
//! console raw.

use std::io;

pub(crate) mod sys;

/// Enables raw mode.
///
/// Please have a look at the [raw mode](./index.html#raw-mode) section.
#[cfg(unix)]
pub fn enable_raw_mode() -> io::Result<()> {
    sys::enable_raw_mode()
}

/// Disables raw mode.
///
/// Please have a look at the [raw mode](./index.html#raw-mode) section.
#[cfg(unix)]
pub fn disable_raw_mode() -> io::Result<()> {
    sys::disable_raw_mode()
}

/// Returns the terminal size `(columns, rows)`.
///
/// The top left cell is represented `(1, 1)`.
pub fn size() -> io::Result<(u16, u16)> {
    sys::size()
}
