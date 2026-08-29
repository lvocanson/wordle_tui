//! UNIX related logic for terminal manipulation.

use crate::terminal::sys::file_descriptor::{tty_fd, FileDesc};
use rustix::{fd::AsFd, termios::Termios};

use std::cell::UnsafeCell;
use std::{fs::File, io};

// Some(Termios) -> we're in the raw mode and this is the previous mode
// None -> we're not in the raw mode
//
// LOCAL PATCH — see vendor/crossterm/LOCAL_PATCH.md. Upstream guards this behind a
// `parking_lot::Mutex`; this app is single-threaded (one event-loop thread), so a bare cell drops
// `parking_lot`/`parking_lot_core` — the analogue of the event-reader patch, for the Linux side.
// `lock()` hands back the `&mut` directly instead of a guard, so the call sites below bind it
// without `mut`; those one-word bindings are the only other difference from upstream in this file.
struct RawModeCell(UnsafeCell<Option<Termios>>);
// SAFETY: touched only from the single thread that enables/disables raw mode; `Sync` is required
// only to place it in a `static`.
unsafe impl Sync for RawModeCell {}
impl RawModeCell {
    // SAFETY: single-threaded, non-reentrant access — the returned `&mut` is used and dropped
    // within one raw-mode call, never aliased.
    #[allow(clippy::mut_from_ref)]
    fn lock(&self) -> &mut Option<Termios> {
        unsafe { &mut *self.0.get() }
    }
}
static TERMINAL_MODE_PRIOR_RAW_MODE: RawModeCell = RawModeCell(UnsafeCell::new(None));

/// Returns the terminal size `(columns, rows)`, read from `/dev/tty` when it is there and from
/// standard output otherwise.
///
/// LOCAL PATCH — see vendor/crossterm/LOCAL_PATCH.md. Upstream falls back to spawning `tput` when
/// the ioctl fails; we drop it (ioctl-only) to shed the Command/env-map/Debug subtree that pulls
/// in, and return the OS error instead.
pub(crate) fn size() -> io::Result<(u16, u16)> {
    let file = File::open("/dev/tty").map(|file| FileDesc::Owned(file.into()));
    let fd = if let Ok(file) = &file {
        file.as_fd()
    } else {
        // Fallback to stdout if /dev/tty is missing
        rustix::stdio::stdout()
    };
    let size = rustix::termios::tcgetwinsize(fd)?;
    Ok((size.ws_col, size.ws_row))
}

pub(crate) fn enable_raw_mode() -> io::Result<()> {
    let original_mode = TERMINAL_MODE_PRIOR_RAW_MODE.lock();
    if original_mode.is_some() {
        return Ok(());
    }

    let tty = tty_fd()?;
    let mut ios = get_terminal_attr(&tty)?;
    let original_mode_ios = ios.clone();
    ios.make_raw();
    set_terminal_attr(&tty, &ios)?;
    // Keep it last - set the original mode only if we were able to switch to the raw mode
    *original_mode = Some(original_mode_ios);
    Ok(())
}

/// Reset the raw mode.
///
/// More precisely, reset the whole termios mode to what it was before the first call
/// to [enable_raw_mode]. If you don't mess with termios outside of crossterm, it's
/// effectively disabling the raw mode and doing nothing else.
pub(crate) fn disable_raw_mode() -> io::Result<()> {
    let original_mode = TERMINAL_MODE_PRIOR_RAW_MODE.lock();
    if let Some(original_mode_ios) = original_mode.as_ref() {
        let tty = tty_fd()?;
        set_terminal_attr(&tty, original_mode_ios)?;
        // Keep it last - remove the original mode only if we were able to switch back
        *original_mode = None;
    }
    Ok(())
}

fn get_terminal_attr(fd: impl AsFd) -> io::Result<Termios> {
    let result = rustix::termios::tcgetattr(fd)?;
    Ok(result)
}

fn set_terminal_attr(fd: impl AsFd, termios: &Termios) -> io::Result<()> {
    rustix::termios::tcsetattr(fd, rustix::termios::OptionalActions::Now, termios)?;
    Ok(())
}
