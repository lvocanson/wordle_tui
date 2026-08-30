//! The game's only output: one buffer over the standard output handle.
//!
//! `io::stdout()` is not that. On Windows it decides at *run time* whether to write the bytes as
//! given or convert them to UTF-16 for `WriteConsoleW` (`GetConsoleMode`, then
//! `GetConsoleOutputCP`), so both halves are compiled in whatever the process writes — the
//! conversion, its UTF-8 validation and the `EncodeUtf16` iterator included. Above that sits a
//! `OnceLock<ReentrantLock<RefCell<LineWriter>>>`, whose locking is meaningless in a
//! single-threaded process and whose 1 KB line buffer never sees a newline: this renderer moves
//! the cursor with CSI E.
//!
//! The bytes go to the handle instead, and the buffer below is sized for a whole frame rather
//! than a line. That suits the game's output exactly, it being ASCII and escape sequences.

use std::io::{self, Write};

/// Large enough that a frame on an ordinary terminal leaves in a single write; a bigger frame
/// simply flushes more than once, so correctness does not depend on the value. It is also the
/// ceiling std uses for its own console writes, comfortably under the size at which a write to a
/// Windows console starts failing for want of memory.
const CAPACITY: usize = 8 * 1024;

/// The process's standard output handle.
///
/// `enable_vt` probes this same handle: a stdout redirected to a file or a pipe has no screen to
/// draw on, and fails there before anything is written.
#[cfg(windows)]
pub fn handle() -> isize {
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(which: u32) -> isize;
    }
    unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }
}

/// Where the buffer empties into.
///
/// The two platforms take different routes on purpose. On Windows `WriteFile` is called directly,
/// because every fallible call in `std::fs` reports failure through `io::Error::last_os_error()`,
/// and that one call anchors ~2.5 KB: `from_raw_os_error` references a table of function pointers
/// std fills in, which drags in the OS-error `Display` and the `format!` under it, whether or not
/// an error is ever printed — and this program never reads one. On Unix a `File` on the standard
/// descriptor costs nothing extra (`rustix` and `mio` anchor the same machinery anyway) and buys
/// back `write_all`'s retry on an interrupted write, which the signal-driven resize path wants.
#[cfg(windows)]
type Sink = isize;
#[cfg(unix)]
type Sink = std::mem::ManuallyDrop<std::fs::File>;

#[cfg(windows)]
fn write_sink(sink: &mut Sink, mut data: &[u8]) -> io::Result<()> {
    #[link(name = "kernel32")]
    extern "system" {
        fn WriteFile(
            handle: isize,
            buffer: *const u8,
            length: u32,
            written: *mut u32,
            overlapped: *mut core::ffi::c_void,
        ) -> i32;
    }

    while !data.is_empty() {
        // Chunked at the buffer size, which keeps every call well under the length at which a
        // console write starts to fail.
        let chunk = data.len().min(CAPACITY) as u32;
        let mut written = 0u32;
        let ok = unsafe {
            WriteFile(
                *sink,
                data.as_ptr(),
                chunk,
                &mut written,
                core::ptr::null_mut(),
            )
        };
        if ok == 0 || written == 0 {
            return Err(io::Error::from(io::ErrorKind::Other));
        }
        data = &data[written as usize..];
    }
    Ok(())
}

#[cfg(unix)]
fn write_sink(sink: &mut Sink, data: &[u8]) -> io::Result<()> {
    sink.write_all(data)
}

pub struct Out {
    sink: Sink,
    buf: [u8; CAPACITY],
    len: usize,
}

impl Out {
    pub fn new() -> Out {
        #[cfg(windows)]
        let sink = handle();
        // SAFETY: the standard output descriptor is open for the whole life of the process, and
        // `ManuallyDrop` keeps this `File` from closing it.
        #[cfg(unix)]
        let sink = unsafe {
            use std::os::fd::FromRawFd;
            std::mem::ManuallyDrop::new(std::fs::File::from_raw_fd(1))
        };

        Out {
            sink,
            buf: [0; CAPACITY],
            len: 0,
        }
    }
}

impl Write for Out {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.write_all(data)?;
        Ok(data.len())
    }

    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        if data.len() > CAPACITY - self.len {
            self.flush()?;
        }
        if data.len() > CAPACITY {
            // Longer than the buffer itself: hand it straight to the handle.
            return write_sink(&mut self.sink, data);
        }
        self.buf[self.len..self.len + data.len()].copy_from_slice(data);
        self.len += data.len();
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.len == 0 {
            return Ok(());
        }
        // Emptied before the write, so a failure cannot leave the bytes queued for a second one.
        let len = std::mem::take(&mut self.len);
        write_sink(&mut self.sink, &self.buf[..len])
    }
}
