//! The one place the framework writes to stdout.

use std::io::{self, Write};

/// Writes one line to stdout, the way a task talks to the person running it.
///
/// A failed write, such as a closed pipe, is ignored rather than panicking.
///
/// # Examples
///
/// ```
/// rituals::report("created tasks/lint/Cargo.toml");
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
