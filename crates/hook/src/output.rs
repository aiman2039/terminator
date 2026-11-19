//! The CLI's only path to stdout and stderr.

pub struct Output;

#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "terminator-hook is a CLI; this is its user-facing output"
)]
impl Output {
    /// Prints one line to stdout.
    pub fn line(text: impl std::fmt::Display) {
        println!("{text}");
    }

    /// Prints one line to stdout, and ignores a stdout that is gone (for
    /// example the closed terminal of a process that got SIGHUP).
    pub fn line_or_drop(text: impl std::fmt::Display) {
        use std::io::Write;
        let _ = writeln!(std::io::stdout().lock(), "{text}");
    }

    /// Prints text to stdout without a trailing newline.
    pub fn text(text: impl std::fmt::Display) {
        print!("{text}");
    }

    /// Prints one line to stderr.
    pub fn warn(text: impl std::fmt::Display) {
        eprintln!("{text}");
    }
}
