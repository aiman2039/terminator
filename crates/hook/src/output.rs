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

    /// Prints text to stdout without a trailing newline.
    pub fn text(text: impl std::fmt::Display) {
        print!("{text}");
    }

    /// Prints one line to stderr.
    pub fn warn(text: impl std::fmt::Display) {
        eprintln!("{text}");
    }
}
