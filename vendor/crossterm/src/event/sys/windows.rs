//! This is a WINDOWS specific implementation for input related action.
//!
//! LOCAL PATCH — see …LOCAL_PATCH.md: the console API is called directly here rather than through
//! `crossterm_winapi`. That crate's `Handle` is a refcounted, self-closing wrapper whose `Arc` and
//! two `Drop` flavours this app has no use for, it spells `CONIN$` as an `OsStr` run through the
//! UTF-16 encoder and collected into a `Vec<u16>`, and every one of its fallible calls builds an
//! `io::Error::last_os_error()` — which anchors the whole OS-error `Display` machinery (see
//! change 4 for what a formatted error costs). Nothing here reads an error beyond its existence.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) mod parse;

/// A Win32 `HANDLE`.
pub(crate) type Handle = isize;

const INVALID_HANDLE_VALUE: Handle = -1;

const ENABLE_MOUSE_MODE: u32 = 0x0010 | 0x0080 | 0x0008;

// Console input record kinds (`INPUT_RECORD::EventType`).
pub(crate) const KEY_EVENT: u16 = 0x0001;
pub(crate) const MOUSE_EVENT: u16 = 0x0002;
pub(crate) const WINDOW_BUFFER_SIZE_EVENT: u16 = 0x0004;

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Coord {
    pub x: i16,
    pub y: i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct KeyEventRecord {
    /// `BOOL`: non-zero for a press, zero for a release.
    pub key_down: i32,
    pub repeat_count: u16,
    pub virtual_key_code: u16,
    pub virtual_scan_code: u16,
    /// The `uChar` union, read as its `WCHAR` member.
    pub u_char: u16,
    pub control_key_state: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct MouseEventRecord {
    pub mouse_position: Coord,
    pub button_state: u32,
    pub control_key_state: u32,
    pub event_flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct WindowBufferSizeRecord {
    pub size: Coord,
}

/// The `Event` union of `INPUT_RECORD`. Which member is live is told by `InputRecord::event_type`;
/// the menu and focus records are not declared because nothing here reads them.
#[repr(C)]
pub(crate) union EventRecord {
    pub key: KeyEventRecord,
    pub mouse: MouseEventRecord,
    pub window: WindowBufferSizeRecord,
}

/// `INPUT_RECORD`. `repr(C)` inserts the same two bytes of padding after `event_type` that the C
/// header's alignment does.
#[repr(C)]
pub(crate) struct InputRecord {
    pub event_type: u16,
    pub event: EventRecord,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share_mode: u32,
        security: *mut u8,
        creation: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn ReadConsoleInputW(
        handle: Handle,
        buffer: *mut InputRecord,
        length: u32,
        read: *mut u32,
    ) -> i32;
    fn GetConsoleMode(handle: Handle, mode: *mut u32) -> i32;
    fn SetConsoleMode(handle: Handle, mode: u32) -> i32;
}

/// Opens a handle on the console's input buffer.
///
/// `CONIN$` rather than `GetStdHandle(STD_INPUT_HANDLE)`: a redirected stdin is a file or a pipe,
/// and the keys still come from the console. The name is a literal UTF-16 array — spelling it as a
/// Rust string would drag in the UTF-16 encoder and a `Vec` to collect it into.
pub(crate) fn console_input() -> io::Result<Handle> {
    const CONIN: [u16; 7] = [
        b'C' as u16,
        b'O' as u16,
        b'N' as u16,
        b'I' as u16,
        b'N' as u16,
        b'$' as u16,
        0,
    ];
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ_WRITE: u32 = 0x0000_0003;
    const OPEN_EXISTING: u32 = 3;

    let handle = unsafe {
        CreateFileW(
            CONIN.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            0,
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::from(io::ErrorKind::Other));
    }
    Ok(handle)
}

/// Blocks until the console input buffer yields one record.
pub(crate) fn read_input_record(handle: Handle) -> io::Result<InputRecord> {
    // SAFETY: every field is a plain integer, so the zeroed value is valid whichever union member
    // the read then fills in; `event_type` decides which one is read back.
    let mut record: InputRecord = unsafe { std::mem::zeroed() };
    let mut read = 0;

    if unsafe { ReadConsoleInputW(handle, &mut record, 1, &mut read) } == 0 || read != 1 {
        return Err(io::Error::from(io::ErrorKind::Other));
    }
    Ok(record)
}

fn console_mode(handle: Handle) -> io::Result<u32> {
    let mut mode = 0;
    if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
        return Err(io::Error::from(io::ErrorKind::Other));
    }
    Ok(mode)
}

fn set_console_mode(handle: Handle, mode: u32) -> io::Result<()> {
    if unsafe { SetConsoleMode(handle, mode) } == 0 {
        return Err(io::Error::from(io::ErrorKind::Other));
    }
    Ok(())
}

/// This is a either `u64::MAX` if it's uninitialized or a valid `u32` that stores the original
/// console mode if it's initialized.
static ORIGINAL_CONSOLE_MODE: AtomicU64 = AtomicU64::new(u64::MAX);

/// Initializes the default console mode. It will be skipped if it has already been initialized.
fn init_original_console_mode(original_mode: u32) {
    let _ = ORIGINAL_CONSOLE_MODE.compare_exchange(
        u64::MAX,
        u64::from(original_mode),
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
}

/// Returns the original console mode, make sure to call `init_original_console_mode` before
/// calling this function.
fn original_console_mode() -> io::Result<u32> {
    // LOCAL PATCH — see …LOCAL_PATCH.md: no message payload.
    u32::try_from(ORIGINAL_CONSOLE_MODE.load(Ordering::Relaxed))
        .map_err(|_| io::Error::from(io::ErrorKind::Other))
}

pub(crate) fn enable_mouse_capture() -> io::Result<()> {
    let handle = console_input()?;
    let result = console_mode(handle).and_then(|mode| {
        init_original_console_mode(mode);
        set_console_mode(handle, ENABLE_MOUSE_MODE)
    });
    unsafe { CloseHandle(handle) };
    result
}

pub(crate) fn disable_mouse_capture() -> io::Result<()> {
    let handle = console_input()?;
    let result = original_console_mode().and_then(|mode| set_console_mode(handle, mode));
    unsafe { CloseHandle(handle) };
    result
}
