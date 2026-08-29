//! WinAPI related logic for terminal manipulation.

use std::io;

use winapi::um::{
    processenv::GetStdHandle,
    winbase::STD_OUTPUT_HANDLE,
    wincon::{GetConsoleScreenBufferInfo, CONSOLE_SCREEN_BUFFER_INFO},
};

// LOCAL PATCH — see …LOCAL_PATCH.md: read the window rect straight off the standard output
// handle. `ScreenBuffer::current()` opens CONOUT$ with `CreateFileW` and wraps it in a refcounted,
// self-closing `Handle` so it stays valid for an arbitrary lifetime; this is one call on a handle
// the process already owns, and the app has already established that it is a console.
pub(crate) fn size() -> io::Result<(u16, u16)> {
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) };
    if ok == 0 {
        return Err(io::Error::from(io::ErrorKind::Other));
    }
    let window = info.srWindow;
    // windows starts counting at 0, unix at 1, add one to replicated unix behaviour.
    Ok((
        (window.Right - window.Left + 1) as u16,
        (window.Bottom - window.Top + 1) as u16,
    ))
}
