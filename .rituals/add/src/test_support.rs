//! A scratch directory helper for this crate's own tests.
//!
//! `rituals-compose` already has one, but a different crate is a genuine
//! boundary that module cannot cross — `tasks/new` and `tasks/create` each
//! keep their own copy for the same reason, and this is this crate's.
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
/// needs to be unpredictable, only distinct within one test run.
static SCRATCH_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory under the system temp root that removes itself on drop.
pub(crate) struct ScratchDir(PathBuf);

impl ScratchDir {
    /// Creates a fresh, empty directory named `ritual-add-<tag>-<pid>-<counter>`
    /// under the system temp root.
    pub(crate) fn new(tag: &str) -> Result<Self, Box<dyn Error>> {
        let unique = SCRATCH_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("ritual-add-{tag}-{}-{unique}", std::process::id()));
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
