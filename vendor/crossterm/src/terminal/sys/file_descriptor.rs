use std::io;

use rustix::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};

/// A file descriptor wrapper, owning the descriptor or borrowing it from stdin.
pub enum FileDesc<'a> {
    Owned(OwnedFd),
    Borrowed(BorrowedFd<'a>),
}

impl FileDesc<'_> {
    pub fn read(&self, buffer: &mut [u8]) -> io::Result<usize> {
        let result = rustix::io::read(self.as_fd(), buffer)?;
        Ok(result)
    }

    pub fn raw_fd(&self) -> RawFd {
        match self {
            FileDesc::Owned(fd) => fd.as_raw_fd(),
            FileDesc::Borrowed(fd) => fd.as_raw_fd(),
        }
    }
}

impl AsFd for FileDesc<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        match self {
            FileDesc::Owned(fd) => fd.as_fd(),
            FileDesc::Borrowed(fd) => fd.as_fd(),
        }
    }
}

/// Creates a file descriptor pointing to the standard input or `/dev/tty`.
pub fn tty_fd() -> io::Result<FileDesc<'static>> {
    use std::fs::File;

    let stdin = rustix::stdio::stdin();
    let fd = if rustix::termios::isatty(stdin) {
        FileDesc::Borrowed(stdin)
    } else {
        let dev_tty = File::options().read(true).write(true).open("/dev/tty")?;
        FileDesc::Owned(dev_tty.into())
    };
    Ok(fd)
}
