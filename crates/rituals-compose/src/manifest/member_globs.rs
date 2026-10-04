//! Reading a `[workspace]` members entry as Cargo does: a path, or a glob.

use std::path::{Path, PathBuf};

use rituals::Failure;

/// The characters that make a `members` entry a glob rather than a path.
const GLOB_CHARACTERS: [char; 3] = ['*', '?', '['];

/// Whether `entry` is a glob, which names every directory it matches, rather
/// than one path.
pub(super) fn is_a_glob(entry: &str) -> bool {
    entry.contains(GLOB_CHARACTERS)
}

/// Every path the glob `entry`, joined to `root`, matches on disk, the way
/// Cargo expands a `members` glob.
///
/// A glob that matches nothing returns nothing, though Cargo then reads the
/// entry as a literal path.
pub(super) fn expand(root: &Path, entry: &str) -> Result<Vec<PathBuf>, Failure> {
    let pattern = root.join(entry);
    let unreadable = |error: &dyn std::fmt::Display| {
        Failure::new(format!(
            "the [workspace] members glob `{entry}` could not be expanded: {error}"
        ))
    };
    let pattern = pattern
        .to_str()
        .ok_or_else(|| unreadable(&"its path is not UTF-8"))?;
    glob::glob(pattern)
        .map_err(|error| unreadable(&error))?
        .map(|matched| matched.map_err(|error| unreadable(&error)))
        .collect()
}
