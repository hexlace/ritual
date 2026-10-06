//! The scratch directory and tree snapshot every crate's unit tests share.
//!
//! Only built for this crate's own tests and with the `test-util` feature,
//! which a crate turns on for its tests and a build never needs. One copy,
//! because copies of a fixture drift: a fix made to one does not reach the
//! others, and a test moved between crates could change meaning without
//! anyone noticing.

use std::collections::BTreeMap;
use std::ffi::OsString;
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

/// One entry in a [`Snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A directory, empty or not.
    Directory,
    /// A regular file, with its exact bytes.
    File(Vec<u8>),
    /// A symbolic link, with the target it holds, as written and not
    /// followed. The bytes, not a `PathBuf`, because paths compare by their
    /// components, which read `first/` and `./first` as `first`, while a
    /// link to `first/` no longer resolves when `first` is a file.
    Link(OsString),
}

/// What a tree holds, keyed by each entry's path relative to the tree's
/// root.
pub type Snapshot = BTreeMap<PathBuf, Entry>;

/// Every directory, regular file and symbolic link under `root`, keyed by
/// its path relative to `root`.
///
/// `root` itself is not an entry. A test compares one taken before a run
/// with one taken after, when it claims the run left the tree exactly as it
/// was.
///
/// A link is recorded as a link, with its target, and never followed. So a
/// link that is added, removed or pointed somewhere else changes the
/// snapshot even when what it leads to reads the same, and a link that leads
/// nowhere is recorded rather than failing the walk. Directories are
/// recorded, empty ones included, because a directory left behind is a
/// change too. Anything else, such as a socket or a pipe, is not something a
/// project's tree holds, and is skipped.
///
/// Keyed relative to `root`, so the snapshot does not depend on whether
/// `root` was spelled through a link: that choice is the test's, made when
/// it asked for its [`ScratchDir`].
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// use rituals_compose::test_util::{Entry, ScratchDir, snapshot};
///
/// let scratch = ScratchDir::new("snapshot-example")?;
/// std::os::unix::fs::symlink("nowhere", scratch.path().join("link"))?;
///
/// assert_eq!(
///     snapshot(scratch.path())?.get(Path::new("link")),
///     Some(&Entry::Link("nowhere".into()))
/// );
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn snapshot(root: &Path) -> io::Result<Snapshot> {
    let mut found = Snapshot::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            // The entry's own type, not its target's: a link is not followed.
            let file_type = entry.file_type()?;
            let recorded = if file_type.is_dir() {
                pending.push(path.clone());
                Entry::Directory
            } else if file_type.is_symlink() {
                Entry::Link(std::fs::read_link(&path)?.into_os_string())
            } else if file_type.is_file() {
                Entry::File(std::fs::read(&path)?)
            } else {
                continue;
            };
            found.insert(relative(root, &path)?, recorded);
        }
    }
    Ok(found)
}

/// `path`, which a walk of `root` found, relative to `root`.
fn relative(root: &Path, path: &Path) -> io::Result<PathBuf> {
    path.strip_prefix(root)
        .map(Path::to_path_buf)
        .map_err(|error| io::Error::other(format!("{}: {error}", path.display())))
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
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};

    use super::{Entry, ScratchDir, snapshot};

    type TestOutcome = Result<(), Box<dyn std::error::Error>>;

    /// A link pointed at a second file with the same bytes as the first
    /// reads the same through the link, so only a snapshot that records the
    /// link itself can see that it was retargeted.
    #[test]
    fn a_retargeted_link_changes_the_snapshot() -> TestOutcome {
        let scratch = ScratchDir::new("snapshot-retargeted-link")?;
        let root = scratch.path();
        std::fs::write(root.join("first"), "same\n")?;
        std::fs::write(root.join("second"), "same\n")?;
        symlink("first", root.join("link"))?;
        let before = snapshot(root)?;

        std::fs::remove_file(root.join("link"))?;
        symlink("second", root.join("link"))?;
        let after = snapshot(root)?;

        assert_ne!(before, after, "a retargeted link must be seen");
        assert_eq!(
            after.get(Path::new("link")),
            Some(&Entry::Link("second".into()))
        );
        Ok(())
    }

    /// A link respelled `first/` resolves no more, because `first` is a
    /// file, yet a path compared by its components reads it as `first`. Only
    /// a target compared as written sees the change.
    #[test]
    fn a_link_respelled_with_a_trailing_slash_changes_the_snapshot() -> TestOutcome {
        let scratch = ScratchDir::new("snapshot-respelled-link")?;
        let root = scratch.path();
        std::fs::write(root.join("first"), "same\n")?;
        symlink("first", root.join("link"))?;
        let before = snapshot(root)?;

        std::fs::remove_file(root.join("link"))?;
        symlink("first/", root.join("link"))?;
        let after = snapshot(root)?;

        assert_ne!(before, after, "a link respelled `first/` must be seen");
        Ok(())
    }

    /// A link to a directory is recorded as a link and not walked into, and
    /// a link that leads nowhere is recorded rather than failing the walk.
    #[test]
    fn a_link_is_recorded_and_never_followed() -> TestOutcome {
        let scratch = ScratchDir::new("snapshot-link-not-followed")?;
        let root = scratch.path();
        std::fs::create_dir(root.join("target-directory"))?;
        std::fs::write(root.join("target-directory/file"), "inside\n")?;
        symlink("target-directory", root.join("to-a-directory"))?;
        symlink("missing", root.join("dangling"))?;

        let taken = snapshot(root)?;

        assert_eq!(
            taken.keys().map(PathBuf::as_path).collect::<Vec<_>>(),
            [
                Path::new("dangling"),
                Path::new("target-directory"),
                Path::new("target-directory/file"),
                Path::new("to-a-directory"),
            ]
        );
        assert_eq!(
            taken.get(Path::new("dangling")),
            Some(&Entry::Link("missing".into()))
        );
        Ok(())
    }

    /// An empty directory is part of the tree: creating one is a change.
    #[test]
    fn an_empty_directory_changes_the_snapshot() -> TestOutcome {
        let scratch = ScratchDir::new("snapshot-empty-directory")?;
        let before = snapshot(scratch.path())?;

        std::fs::create_dir(scratch.path().join("leftover"))?;
        let after = snapshot(scratch.path())?;

        assert_eq!(
            after.get(Path::new("leftover")),
            Some(&Entry::Directory),
            "before was {before:?}"
        );
        Ok(())
    }

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
