use std::fmt;
use std::io::{self, Write};

/// An interface for a command that performs an action on the terminal.
pub trait Command {
    /// Write an ANSI representation of this command to the given writer.
    /// An ANSI code can manipulate the terminal by writing it to the terminal buffer.
    /// However, only Windows 10 and UNIX systems support this.
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result;

    /// Execute this command.
    ///
    /// Windows versions lower than Windows 10 do not support ANSI escape codes, therefore a
    /// direct WinAPI call is made.
    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()>;

    /// Whether ANSI escape codes should be used to execute this command on Windows.
    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool;
}

/// An interface for types that can queue commands for further execution.
pub trait QueueableCommand {
    /// Queues the given command for further execution.
    fn queue(&mut self, command: impl Command) -> io::Result<&mut Self>;
}

impl<T: Write + ?Sized> QueueableCommand for T {
    /// Queues the given command for further execution.
    ///
    /// Queued commands will be executed in the following cases:
    ///
    /// * When `flush` is called manually on the given type implementing `io::Write`.
    /// * The terminal will `flush` automatically if the buffer is full.
    /// * Each line is flushed in case of `stdout`, because it is line buffered.
    fn queue(&mut self, command: impl Command) -> io::Result<&mut Self> {
        #[cfg(windows)]
        if !command.is_ansi_code_supported() {
            // There may be queued commands in this writer, but `execute_winapi` will execute the
            // command immediately. To prevent commands being executed out of order we flush the
            // writer now.
            self.flush()?;
            command.execute_winapi()?;
            return Ok(self);
        }

        write_command_ansi(self, command)?;
        Ok(self)
    }
}

/// Writes the ANSI representation of a command to the given writer.
fn write_command_ansi<C: Command>(
    io: &mut (impl io::Write + ?Sized),
    command: C,
) -> io::Result<()> {
    struct Adapter<T> {
        inner: T,
        res: io::Result<()>,
    }

    impl<T: Write> fmt::Write for Adapter<T> {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            self.inner.write_all(s.as_bytes()).map_err(|e| {
                self.res = Err(e);
                fmt::Error
            })
        }
    }

    let mut adapter = Adapter {
        inner: io,
        res: Ok(()),
    };

    command
        .write_ansi(&mut adapter)
        .map_err(|fmt::Error| match adapter.res {
            Ok(()) => panic!(
                "<{}>::write_ansi incorrectly errored",
                std::any::type_name::<C>()
            ),
            Err(e) => e,
        })
}
