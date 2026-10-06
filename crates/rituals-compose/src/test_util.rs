//! The scratch directory every crate's unit tests share.
//!
//! Only built for this crate's own tests and with the `test-util` feature,
//! which a crate turns on for its tests and a build never needs. One copy,
//! because copies of a fixture drift: a fix made to one does not reach the
//! others, and a test moved between crates could change meaning without
//! anyone noticing.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A counter for [`ScratchDir`]'s constructors, so two scratch directories
/// created in the same test process never collide.
///
/// Test-fixture uniqueness, not a production seed: nothing here needs to be
/// unpredictable, only distinct within one process, and the process id in
/// the name makes it distinct across processes. A counter rather than the
/// clock, because two clock readings are not guaranteed distinct and tests
/// run in parallel.
static SCRATCH_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh, empty directory under the system temp root that removes itself,
/// and everything in it, on drop.
///
/// There are two kinds, and a test asks for the one it needs by name. The
/// system temp root can be reached through a symbolic link (on macOS `/tmp`
/// and `/var` lead to `/private`), so a path under it can be spelled two
/// ways:
///
/// - [`ScratchDir::new`] spells the directory as the temp root does, which
///   may be through a link. It is what a test gets when the spelling makes no
///   difference to it, and what a test needs when it checks that code given
///   a path through a link answers as it would for the resolved one.
/// - [`ScratchDir::resolved`] spells it with every link resolved, the way
///   Cargo reports every path and a task's working directory is spelled. A
///   test needs it when it compares paths it built against ones Cargo or the
///   code under test resolved.
///
/// # Examples
///
/// ```
/// use rituals_compose::test_util::ScratchDir;
///
/// let scratch = ScratchDir::resolved("example")?;
/// assert_eq!(std::fs::canonicalize(scratch.path())?, scratch.path());
/// # Ok::<(), std::io::Error>(())
/// ```
#[derive(Debug)]
pub struct ScratchDir(PathBuf);

impl ScratchDir {
    /// Creates a directory named `ritual-test-<tag>-<pid>-<counter>` under
    /// the system temp root, spelled as the temp root spells it.
    pub fn new(tag: &str) -> io::Result<Self> {
        Ok(Self(create(tag)?))
    }

    /// Creates a directory as [`ScratchDir::new`] does, spelled with every
    /// symbolic link above it resolved.
    pub fn resolved(tag: &str) -> io::Result<Self> {
        let path = create(tag)?;
        match std::fs::canonicalize(&path) {
            Ok(resolved) => Ok(Self(resolved)),
            Err(error) => {
                drop(std::fs::remove_dir_all(&path));
                Err(error)
            }
        }
    }

    /// The directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// Creates the directory `ritual-test-<tag>-<pid>-<counter>` under the
/// system temp root and returns it, spelled as the temp root spells it.
fn create(tag: &str) -> io::Result<PathBuf> {
    let unique = SCRATCH_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("ritual-test-{tag}-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        // Best effort: a leftover scratch directory costs disk space, not
        // correctness, and a test that already failed should not fail a
        // second time over cleanup.
        drop(std::fs::remove_dir_all(&self.0));
    }
}

/// Says, on the test process's own stderr, that a check was skipped and why.
///
/// Written straight to the stream rather than through `eprintln!`, which the
/// test harness captures and discards for a passing test, where a skip would
/// read exactly like a pass.
pub fn report_skip(message: &str) {
    use std::io::Write as _;
    // Best effort: a notice that cannot be written changes nothing about the
    // result, and a failed write to stderr has nowhere better to go.
    drop(writeln!(std::io::stderr(), "SKIPPED {message}"));
}

#[cfg(test)]
mod tests {
    use super::ScratchDir;

    type TestOutcome = Result<(), Box<dyn std::error::Error>>;

    /// `resolved` spells the directory with no link left in it, and `new`
    /// spells it as the temp root does, which may go through one.
    #[test]
    fn resolved_is_spelled_with_every_link_resolved_and_new_as_the_temp_root_is() -> TestOutcome {
        let resolved = ScratchDir::resolved("scratch-resolved")?;
        let plain = ScratchDir::new("scratch-plain")?;

        assert_eq!(std::fs::canonicalize(resolved.path())?, resolved.path());
        assert!(plain.path().starts_with(std::env::temp_dir()));
        Ok(())
    }
}
