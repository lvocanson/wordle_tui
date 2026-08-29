//! This module provides platform related functions.

#[cfg(unix)]
pub(crate) use self::unix::{disable_raw_mode, enable_raw_mode, size};
#[cfg(windows)]
pub(crate) use self::windows::size;

#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub mod file_descriptor;
#[cfg(unix)]
mod unix;
