//! The daemon's only path to stderr. Native fixtures and the
//! integration harness read these lines, so they stay plain text.

pub struct Console;

#[expect(
    clippy::print_stderr,
    reason = "Diagnostics and fixture markers are plain stdout/stderr lines"
)]
impl Console {
    /// Prints one line to stderr.
    pub fn err(args: std::fmt::Arguments<'_>) {
        eprintln!("{args}");
    }
}
