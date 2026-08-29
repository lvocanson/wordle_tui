/// Writes a CSI sequence: the escape prefix `ESC [` followed by the given literals.
#[macro_export]
#[doc(hidden)]
macro_rules! csi {
    ($( $l:expr ),*) => { concat!("\x1B[", $( $l ),*) };
}

/// Queues one or more commands on the given writer, to be executed on its next flush.
#[macro_export]
macro_rules! queue {
    ($writer:expr $(, $command:expr)* $(,)?) => {{
        use ::std::io::Write;

        // This allows the macro to take both mut impl Write and &mut impl Write.
        Ok($writer.by_ref())
            $(.and_then(|writer| $crate::QueueableCommand::queue(writer, $command)))*
            .map(|_| ())
    }}
}

/// Queues one or more commands on the given writer, then flushes it.
#[macro_export]
macro_rules! execute {
    ($writer:expr $(, $command:expr)* $(,)? ) => {{
        use ::std::io::Write;

        // Queue each command, then flush
        $crate::queue!($writer $(, $command)*)
            .and_then(|()| {
                ::std::io::Write::flush($writer.by_ref())
            })
    }}
}
