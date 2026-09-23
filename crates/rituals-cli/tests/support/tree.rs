//! Walking a directory tree, and comparing what is in it before and after.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::{Outcome, ResultContext};

/// The names this suite never treats as project content when snapshotting or
/// searching a tree: build output, version control, and the lockfile, which
/// legitimately gains entries on operations this suite does not mean to hold
/// constant.
const EXCLUDED_ENTRIES: [&str; 3] = ["target", ".git", "Cargo.lock"];

/// What a walk found at one path: a directory, or a regular file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Directory,
    File,
}

/// Every directory and regular file under `root` (not `root` itself),
/// skipping [`EXCLUDED_ENTRIES`] and everything beneath them at any depth,
/// as paths that start with `root`. The one walk both [`files_under`] and
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

/// One entry in a [`Snapshot`]: a directory, or a regular file with its
/// exact bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    Directory,
    File(Vec<u8>),
}

/// A tree's directories and regular files, keyed by their path relative to
/// the tree's root.
pub(crate) type Snapshot = BTreeMap<PathBuf, Entry>;

/// Collects every directory and regular file under `root` (not `root`
/// itself), keyed by its path relative to `root`, with each file's exact
/// bytes.
///
/// Directories are recorded, empty ones included, because a directory is
/// something ritual can leave behind: `add` treats an existing
/// `tasks/<name>` as a leftover from an earlier run, so a refused run that
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
            };
            let relative = path
                .strip_prefix(root)
                .context("stripping the root prefix failed")?
                .to_path_buf();
            Ok((relative, entry))
        })
        .collect()
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
