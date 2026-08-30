//! WinAPI related logic for terminal manipulation.

use std::io;

const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;

#[repr(C)]
struct Coord {
    x: i16,
    y: i16,
}

#[repr(C)]
struct SmallRect {
    left: i16,
    top: i16,
    right: i16,
    bottom: i16,
}

#[repr(C)]
struct ConsoleScreenBufferInfo {
    size: Coord,
    cursor_position: Coord,
    attributes: u16,
    window: SmallRect,
    maximum_window_size: Coord,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetStdHandle(which: u32) -> isize;
    fn GetConsoleScreenBufferInfo(handle: isize, info: *mut ConsoleScreenBufferInfo) -> i32;
}

// LOCAL PATCH — see …LOCAL_PATCH.md: read the window rect straight off the standard output
// handle. `ScreenBuffer::current()` opens CONOUT$ with `CreateFileW` and wraps it in a refcounted,
// self-closing `Handle` so it stays valid for an arbitrary lifetime; this is one call on a handle
// the process already owns, and the app has already established that it is a console.
pub(crate) fn size() -> io::Result<(u16, u16)> {
    // SAFETY: every field is a plain integer, so the zeroed value is valid; the call fills it in
    // before it reports success.
    let mut info: ConsoleScreenBufferInfo = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) };
    if ok == 0 {
        return Err(io::Error::from(io::ErrorKind::Other));
    }
    let window = info.window;
    // windows starts counting at 0, unix at 1, add one to replicated unix behaviour.
    Ok((
        (window.right - window.left + 1) as u16,
        (window.bottom - window.top + 1) as u16,
    ))
}
