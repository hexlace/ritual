//! The one place the framework writes to stdout or stderr.

use std::io::{self, Write};

/// Writes one line to stdout, the way a task talks to the person running it.
///
/// A failed write, such as a closed pipe, is ignored rather than panicking.
/// For something the person should heed that is not the run's result, see
/// [`warn()`].
///
/// # Examples
///
/// ```
/// rituals::report("created .rituals/lint/Cargo.toml");
/// rituals::report(format!("updated {} (tasks: {})", "src/main.rs", "ritual, lint"));
/// ```
//
// `impl AsRef<str>` rather than `&str`, so a task can report a `String` it
// just built without borrowing a temporary at the call site. The write goes
// through `writeln!` on a locked handle rather than `println!`, because
// `println!` panics on a broken pipe, and a caller who closed the pipe
// already knows it did: there is nobody left to tell and no reason to stop
// the program over it.
pub fn report(message: impl AsRef<str>) {
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{}", message.as_ref());
}

/// Writes one line to stderr, for something the person should heed that is
/// not the run's result, such as a command that is going away.
///
/// [`report()`] says what the run did, on stdout, where a caller reads
/// results; `warn` says what to heed, on stderr, where a caller reading
/// results never sees it. The line is written as given, with no prefix. A
/// failed write is ignored rather than panicking.
///
/// # Examples
///
/// ```
/// rituals::warn("add is now create, and will be removed in ritual 0.3.0");
/// ```
pub fn warn(message: impl AsRef<str>) {
    write_to_stderr(message.as_ref());
}

/// Writes one line to stderr: the one place the framework does, shared by
/// [`warn()`] and the dispatch's refusals.
///
/// Uses a locked `writeln!` rather than `eprintln!` for the same reason
/// [`report()`] avoids `println!`: a failed write is discarded rather than
/// panicking.
#[expect(
    clippy::redundant_pub_crate,
    reason = "plain `pub` in this private module trips `unreachable_pub`, and the dispatch \
              module needs the writer, so `pub(crate)` is the one spelling both lints accept"
)]
pub(crate) fn write_to_stderr(message: &str) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "{message}");
}
