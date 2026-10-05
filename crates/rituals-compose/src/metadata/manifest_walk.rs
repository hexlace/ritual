//! The walk over the manifests Cargo reads, which both asking what Cargo reads
//! and asking who still depends on a directory are answered from.
//
// `redundant_pub_crate` (clippy nursery) wants `pub` here because this
// module is private, but `pub(crate)` is the visibility that is actually
// true: `task_imports` reaches it from outside `metadata`.
#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) reflects this module's actual visibility even though the enclosing \
              module is private; see the note above"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rituals::Failure;

use crate::manifest::{Manifest, ManifestRole};
use crate::paths::{lies_under, normalize};

/// A manifest the walk has read.
pub(crate) struct Reached<'a> {
    /// Where the manifest is, normalised.
    pub(crate) path: &'a Path,
    pub(crate) manifest: &'a Manifest,
    /// The directories the path dependencies Cargo reads in it lead to, as
    /// [`Manifest::path_dependency_directories`] lists them for what it is
    /// to the workspace.
    pub(crate) leads_to: &'a [PathBuf],
}

/// Reads the manifest at each of `starts`, then the manifest of every crate
/// those reach through a path dependency of any kind Cargo reads, however
/// far, and hands each manifest to `visit` once.
///
/// `workspace_root_manifest` is the workspace's root manifest, normalised:
/// it is the one manifest whose `[workspace.dependencies]`, `[patch]` and
/// `[replace]` are followed, since Cargo ignores them in any other.
///
/// A manifest in `already_read` is not read, nor are the crates only it
/// reaches. A crate whose directory lies under `keeping_out_of` is not
/// followed into, when there is one. A path dependency whose directory holds
/// no `Cargo.toml` is left out.
///
/// # Errors
///
/// Returns a [`Failure`] naming the manifest when one exists but cannot be
/// read or does not parse as TOML.
pub(crate) fn walk_manifests(
    workspace_root_manifest: &Path,
    starts: Vec<PathBuf>,
    already_read: BTreeSet<PathBuf>,
    keeping_out_of: Option<&Path>,
    mut visit: impl FnMut(&Reached<'_>),
) -> Result<(), Failure> {
    let mut read = already_read;
    let mut pending = starts;
    // Each manifest is read once, so the walk is bounded by the manifests on
    // disk, however the path dependencies loop.
    while let Some(path) = pending.pop() {
        let path = normalize(&path);
        if !read.insert(path.clone()) {
            continue;
        }
        let manifest = Manifest::read(&path)?;
        let role = if path == workspace_root_manifest {
            ManifestRole::WorkspaceRoot
        } else {
            ManifestRole::Other
        };
        let leads_to = manifest.path_dependency_directories(role);
        visit(&Reached {
            path: &path,
            manifest: &manifest,
            leads_to: &leads_to,
        });
        pending.extend(manifests_at(
            leads_to.iter().map(PathBuf::as_path),
            keeping_out_of,
        ));
    }
    Ok(())
}

/// The manifests of the crates at `directories` that exist, normalised, leaving
/// out a crate under `keeping_out_of`, when there is one.
pub(crate) fn manifests_at<'a>(
    directories: impl Iterator<Item = &'a Path>,
    keeping_out_of: Option<&Path>,
) -> Vec<PathBuf> {
    directories
        .filter(|directory| keeping_out_of.is_none_or(|kept_out| !lies_under(directory, kept_out)))
        .map(|directory| normalize(&directory.join("Cargo.toml")))
        .filter(|manifest_path| manifest_path.is_file())
        .collect()
}
