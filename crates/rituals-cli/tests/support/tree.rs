//! Walking a directory tree, and comparing what is in it before and after.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use super::{Outcome, ResultContext};

/// The names this suite never treats as project content when snapshotting or
/// searching a tree: build output, version control, and the lockfile, which
/// legitimately gains entries on operations this suite does not mean to hold
/// constant.
const EXCLUDED_ENTRIES: [&str; 3] = ["target", ".git", "Cargo.lock"];

/// What a walk found at one path: a directory, a regular file, or a
/// symbolic link, by the entry's own type and never its target's.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Directory,
    File,
    Link,
}

/// Every directory, regular file and symbolic link under `root` (not `root`
/// itself), skipping [`EXCLUDED_ENTRIES`] and everything beneath them at any
/// depth, as paths that start with `root`. A link is never followed, so
/// nothing is found through one. The one walk both [`files_under`] and
/// [`snapshot_tree`] read, so the two cannot disagree about what a tree
/// holds.
fn walk(root: &Path) -> Outcome<Vec<(PathBuf, Kind)>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        let entries =
            fs::read_dir(&directory).context(&format!("reading {} failed", directory.display()))?;

        for entry in entries {
            let entry = entry.context("reading a directory entry failed")?;
            if EXCLUDED_ENTRIES
                .iter()
                .any(|excluded| entry.file_name() == *excluded)
            {
                continue;
            }

            let path = entry.path();
            let file_type = entry
                .file_type()
                .context(&format!("stat-ing {} failed", path.display()))?;
            if file_type.is_dir() {
                pending.push(path.clone());
                found.push((path, Kind::Directory));
            } else if file_type.is_file() {
                found.push((path, Kind::File));
            } else if file_type.is_symlink() {
                found.push((path, Kind::Link));
            }
        }
    }

    Ok(found)
}

/// Every regular file under `root`, skipping [`EXCLUDED_ENTRIES`] at any
/// depth, as paths that start with `root`.
pub(crate) fn files_under(root: &Path) -> Outcome<Vec<PathBuf>> {
    Ok(walk(root)?
        .into_iter()
        .filter(|(_, kind)| *kind == Kind::File)
        .map(|(path, _)| path)
        .collect())
}

/// One entry in a [`Snapshot`]: a directory, a regular file with its exact
/// bytes, or a symbolic link with the target it holds, as written: the
/// bytes, not a `PathBuf`, because paths compare by their components, which
/// read `first/` and `./first` as `first`, while a link to `first/` no
/// longer resolves when `first` is a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    Directory,
    File(Vec<u8>),
    Link(OsString),
}

/// A tree's directories, regular files and symbolic links, keyed by their
/// path relative to the tree's root.
pub(crate) type Snapshot = BTreeMap<PathBuf, Entry>;

/// Collects every directory, regular file and symbolic link under `root`
/// (not `root` itself), keyed by its path relative to `root`, with each
/// file's exact bytes and each link's target.
///
/// A link is recorded as a link and never followed. So a link that is added,
/// removed or pointed somewhere else changes the snapshot even when what it
/// leads to reads the same, and a link that leads nowhere is recorded rather
/// than failing the walk.
///
/// Directories are recorded, empty ones included, because a directory is
/// something ritual can leave behind: `create` treats an existing
/// `.rituals/<name>` as a leftover from an earlier run, so a refused run that
/// created one has changed what the next run does, whether or not it wrote
/// a file into it.
///
/// What ritual claims to leave alone, or to write exactly once, is the
/// project's own tree — never its build output or version control — so
/// [`EXCLUDED_ENTRIES`] is skipped.
pub(crate) fn snapshot_tree(root: &Path) -> Outcome<Snapshot> {
    walk(root)?
        .into_iter()
        .map(|(path, kind)| {
            let entry = match kind {
                Kind::Directory => Entry::Directory,
                Kind::File => Entry::File(
                    fs::read(&path).context(&format!("reading {} failed", path.display()))?,
                ),
                Kind::Link => Entry::Link(
                    fs::read_link(&path)
                        .context(&format!("reading the link {} failed", path.display()))?
                        .into_os_string(),
                ),
            };
            let relative = path
                .strip_prefix(root)
                .context("stripping the root prefix failed")?
                .to_path_buf();
            Ok((relative, entry))
        })
        .collect()
}

/// The bytes of the lockfile at the root of `root`, or `None` when there is
/// none. [`snapshot_tree`] leaves the lockfile out, because other stories
/// let it change; a story about a run that must leave the project as it found
/// it, a lockfile it did not have included, reads it here.
pub(crate) fn lockfile(root: &Path) -> Outcome<Option<Vec<u8>>> {
    let path = root.join("Cargo.lock");
    match fs::read(&path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context(&format!("reading {} failed", path.display())),
    }
}

/// The paths whose presence or content differs between two snapshots, in
/// path order.
pub(crate) fn changed_paths(before: &Snapshot, after: &Snapshot) -> Vec<PathBuf> {
    let every_path: BTreeSet<&PathBuf> = before.keys().chain(after.keys()).collect();
    every_path
        .into_iter()
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect()
}

/// Asserts that two [`snapshot_tree`] results are identical, and if not,
/// panics with the specific paths that differ rather than a byte dump.
#[track_caller]
pub(crate) fn assert_trees_identical(context: &str, before: &Snapshot, after: &Snapshot) {
    let changed = changed_paths(before, after);
    assert!(
        changed.is_empty(),
        "{context}: these paths were added, removed or changed: {changed:?}"
    );
}
