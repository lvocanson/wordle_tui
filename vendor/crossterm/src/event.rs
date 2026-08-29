//! # Event
//!
//! The `event` module provides the input this game reads: key presses, left mouse clicks and
//! terminal resizes.
//!
//! [`next`] blocks until one of them arrives. Mouse events only reach it once
//! [`EnableMouseCapture`] has been executed on the terminal.
//!
//! LOCAL PATCH — see vendor/crossterm/LOCAL_PATCH.md. Upstream also carries focus changes,
//! bracketed paste, key releases and repeats, the kitty keyboard protocol, function and
//! navigation keys, and every other mouse kind; none of them is decoded here, so none of the
//! types describing them exist either.

pub(crate) mod source;
pub(crate) mod sys;

use crate::{csi, Command};
use std::cell::UnsafeCell;
use std::fmt;

use bitflags::bitflags;

/// The platform's event source, named concretely rather than held behind `Box<dyn EventSource>`.
/// Windows' source is two words and lives in the static; Unix's carries the parser's buffers, so
/// it stays boxed — inline it would put ~3 KB of `.bss` behind a pointer-sized win.
#[cfg(windows)]
type PlatformSource = crate::event::source::windows::WindowsEventSource;
#[cfg(unix)]
type PlatformSource = Box<crate::event::source::unix::UnixInternalEventSource>;

struct EventSourceCell(UnsafeCell<Option<PlatformSource>>);
// SAFETY: this app only ever touches the source from its single event-loop thread. Making the
// cell `Sync` is required to place it in a `static`; the single-thread invariant is what keeps the
// unsynchronized access sound.
unsafe impl Sync for EventSourceCell {}
static EVENT_SOURCE: EventSourceCell = EventSourceCell(UnsafeCell::new(None));

fn event_source() -> std::io::Result<&'static mut PlatformSource> {
    // SAFETY: single-threaded access with no reentrancy — `next` is the only caller and the
    // source's read never re-enters it, so no `&mut` alias is ever live at once.
    let slot = unsafe { &mut *EVENT_SOURCE.0.get() };
    if slot.is_none() {
        #[cfg(windows)]
        let source = crate::event::source::windows::WindowsEventSource::new()?;
        #[cfg(unix)]
        let source = Box::new(crate::event::source::unix::UnixInternalEventSource::new()?);
        *slot = Some(source);
    }

    // LOCAL PATCH — see …LOCAL_PATCH.md: no message payload.
    slot.as_mut()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::Other))
}

/// Blocks until the next [`Event`] arrives.
///
/// LOCAL PATCH — see vendor/crossterm/LOCAL_PATCH.md. Upstream splits this into `poll` (which
/// decodes an event and queues it) and `read` (which takes it back out); the queue, the `Filter`
/// trait and the `Box<dyn EventSource>` exist only to carry the event between the two. The event
/// source already blocks and returns one event, so this hands its result straight to the caller.
/// There is no timeout because the app has nothing to do between events: it renders on change,
/// and only an event changes anything.
pub fn next() -> std::io::Result<Event> {
    event_source()?.read_blocking()
}

/// A command that enables mouse event capturing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnableMouseCapture;

impl Command for EnableMouseCapture {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str(concat!(
            // Normal tracking: Send mouse X & Y on button press and release
            csi!("?1000h"),
            // Button-event tracking: Report button motion events (dragging)
            csi!("?1002h"),
            // Any-event tracking: Report all motion events
            csi!("?1003h"),
            // RXVT mouse mode: Allows mouse coordinates of >223
            csi!("?1015h"),
            // SGR mouse mode: Allows mouse coordinates of >223, preferred over RXVT mode
            csi!("?1006h"),
        ))
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        sys::windows::enable_mouse_capture()
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        false
    }
}

/// A command that disables mouse event capturing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisableMouseCapture;

impl Command for DisableMouseCapture {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str(concat!(
            // The inverse commands of EnableMouseCapture, in reverse order.
            csi!("?1006l"),
            csi!("?1015l"),
            csi!("?1003l"),
            csi!("?1002l"),
            csi!("?1000l"),
        ))
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        sys::windows::disable_mouse_capture()
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        false
    }
}

/// Represents an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A single key event with additional pressed modifiers.
    Key(KeyEvent),
    /// A left mouse button press, at the cell it landed on.
    Mouse(MouseEvent),
    /// An resize event with new dimensions after resize (columns, rows).
    /// **Note** that resize events can occur in batches.
    Resize(u16, u16),
}

/// A left mouse button press, at the cell it landed on.
///
/// LOCAL PATCH — see …LOCAL_PATCH.md: a left press is the only mouse event either parser emits,
/// so there is no button and no kind to carry, and the modifier bits held during the click are
/// not decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    /// The column that the event occurred on.
    pub column: u16,
    /// The row that the event occurred on.
    pub row: u16,
}

bitflags! {
    /// Represents key modifiers (shift, control, alt).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct KeyModifiers: u8 {
        const SHIFT = 0b0000_0001;
        const CONTROL = 0b0000_0010;
        const ALT = 0b0000_0100;
        const NONE = 0b0000_0000;
    }
}

/// Represents a key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// The key itself.
    pub code: KeyCode,
    /// Additional key modifiers.
    pub modifiers: KeyModifiers,
}

impl KeyEvent {
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers }
    }
}

impl From<KeyCode> for KeyEvent {
    fn from(code: KeyCode) -> Self {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
        }
    }
}

/// Represents a key.
///
/// LOCAL PATCH — see …LOCAL_PATCH.md: the four keys the game reads. Function and navigation keys,
/// the modifier and media keys, and the kitty protocol's extras are not decoded by either parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    /// Backspace key (called "Delete" on macOS, and "Backspace" on Windows)
    Backspace,
    /// Enter key.
    Enter,
    /// Escape key.
    Esc,
    /// A character.
    ///
    /// `KeyCode::Char('c')` represents `c` character, etc.
    Char(char),
}
