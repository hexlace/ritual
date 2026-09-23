//! A scratch directory helper shared by every test module in this crate
//! that needs one.
//!
//! One implementation of "a self-cleaning scratch directory for a
//! filesystem test", used by every test module in this crate that needs
//! one: two copies of the same type in one crate can drift apart without
//! anything noticing. Every task crate that
//! also needs one — `new`, `create`, `add` — keeps its own copy instead: a
//! different crate is a genuine boundary this module cannot cross.
//!
//! Declared behind `#[cfg(test)]` at the `mod test_support;` site in
//! `lib.rs`, not inside this file — the whole point of a shared module is
//! one thing to read, and a second `#![cfg(test)]` here would say the same
//! thing twice.
//
// `redundant_pub_crate` (clippy nursery) wants `pub` here because this
// module is private, but `pub(crate)` is the visibility that is actually
// true — it stays correct if this module is ever re-exported at a different
// level — so the nursery lint gives way.
#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) reflects this module's actual visibility even though the enclosing \
              module is private; see the note above"
)]

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// What a test in this crate returns — the error path carries only a setup
/// failure (a filesystem operation, a TOML fixture that would not parse),
/// never the property under test, which is always carried by an
/// `assert!`/`assert_eq!` instead.
pub(crate) type TestOutcome = Result<(), Box<dyn Error>>;

/// A counter for [`ScratchDir::new`], so two scratch directories created in
/// the same test process never collide.
///
/// This is test-fixture uniqueness, not a production seed: nothing here
/// needs to be unpredictable, only distinct within one test run. A counter
/// rather than the clock: two clock readings are not guaranteed distinct on
/// every platform this workspace targets, and `cargo test`'s default
/// parallelism means two tests can call `ScratchDir::new` close enough
/// together to collide.
static SCRATCH_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory under the system temp root that removes itself on drop.
///
/// Every fixture a unit test in this crate needs — a scratch manifest, a
/// scratch project's workspace and CLI crates, a directory a permissions
/// change makes undeletable — lives inside one of these.
pub(crate) struct ScratchDir(PathBuf);

impl ScratchDir {
    /// Creates a fresh, empty directory named
    /// `rituals-compose-<tag>-<pid>-<counter>` under the system temp root.
    pub(crate) fn new(tag: &str) -> Result<Self, Box<dyn Error>> {
        let unique = SCRATCH_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rituals-compose-{tag}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Says, on the test process's own stderr, that a check was skipped and
/// why. Written straight to the stream rather than through `eprintln!`,
/// which the test harness captures and discards for a passing test — where
/// a skip would read exactly like a pass.
pub(crate) fn report_skip(message: &str) {
    use std::io::Write as _;
    // Best effort: a notice that cannot be written changes nothing about the
    // result, and a failed write to stderr has nowhere better to go.
    drop(writeln!(std::io::stderr(), "SKIPPED {message}"));
}
