//! What the first step is about to do, decided before anything is written,
//! and the check that it did it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rituals::{Failure, Outcome};
use rituals_compose::manifest::{Manifest, PathChange};
use rituals_compose::metadata::Metadata;
use rituals_compose::relocation::Relocation;
use rituals_compose::rollback::Changes;
use rituals_compose::sentence::join_with_and;

use super::Candidates;
use super::refusals::{self, Member};
use crate::places::from_the_root;
use crate::report;
use crate::step::Migrating;

/// A manifest with the edits `repoint` made to it in memory, in the order
/// they will be written.
struct Edited {
    manifest: Manifest,
    changes: Vec<PathChange>,
}

/// Everything the step will write, captured before the first write.
pub(super) struct Planned {
    root: PathBuf,
    relocation: Relocation,
    edited: Vec<Edited>,
    moves: Vec<(PathBuf, PathBuf)>,
}

impl Planned {
    /// Writes every manifest that has edits, in place, then moves each task
    /// directory, so a task's own manifest is edited where it is and carried
    /// by its move. The undo runs the other way: directories go back first,
    /// then each manifest gets its bytes back at its original path.
    pub(super) fn write(&self, changes: &mut Changes) -> Outcome {
        for edited in &self.edited {
            edited.manifest.write(changes)?;
        }
        for (from, to) in &self.moves {
            changes.rename(from, to)?;
        }
        Ok(())
    }

    /// What to report: every move, then every edit grouped by file in the
    /// order the files were written, each file named where it is after the
    /// move, so every path printed is one a person can open once the run is
    /// done.
    pub(super) fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .moves
            .iter()
            .map(|(from, to)| {
                report::moved(
                    &from_the_root(from, &self.root),
                    &from_the_root(to, &self.root),
                )
            })
            .collect();
        for edited in &self.edited {
            let path = self
                .relocation
                .destination(edited.manifest.path())
                .unwrap_or_else(|| edited.manifest.path().to_path_buf());
            let file = from_the_root(&path, &self.root);
            lines.extend(
                edited
                    .changes
                    .iter()
                    .map(|change| report::updated(&file, change)),
            );
        }
        lines
    }

    /// Checks the project after the move: the task list still resolves, and
    /// Cargo reads the members it read before, each where it now is.
    pub(super) fn verify(&self, before: &Metadata, after: &Metadata, package: &str) -> Outcome {
        after.resolve_task_list(package)?;

        let expected: BTreeSet<PathBuf> = before
            .member_directories()
            .into_iter()
            .map(|directory| {
                self.relocation
                    .destination(directory)
                    .unwrap_or_else(|| directory.to_path_buf())
            })
            .collect();
        let actual: BTreeSet<PathBuf> = after
            .member_directories()
            .into_iter()
            .map(Path::to_path_buf)
            .collect();
        let shown = |directories: &BTreeSet<PathBuf>| -> Vec<String> {
            directories
                .iter()
                .map(|directory| from_the_root(directory, &self.root))
                .collect()
        };
        membership_failure(
            &shown(&expected.difference(&actual).cloned().collect()),
            &shown(&actual.difference(&expected).cloned().collect()),
        )
        .map_or(Ok(()), Err)
    }
}

/// Reads the project and decides what the step would write, refusing when
/// that would be unsafe. Everything here only reads.
///
/// The refusals come in the order of how much each one rules out: a task
/// that holds other members, a destination that is taken, a submodule, files
/// git would see differently at their new place, and last a manifest that
/// reaches a task in a way that cannot be repointed.
pub(super) fn plan(
    candidates: &Candidates,
    migrating: &Migrating<'_>,
    before: &Metadata,
) -> Result<Planned, Failure> {
    let Candidates {
        from_directory,
        to_directory,
        directories,
    } = candidates;
    let relocation = Relocation::new(from_directory, to_directory, directories.iter().cloned());
    let moves: Vec<(PathBuf, PathBuf)> = directories
        .iter()
        .map(|directory| {
            let Some(destination) = relocation.destination(directory) else {
                unreachable!(
                    "{} was given to the relocation as a directory that moves",
                    directory.display()
                )
            };
            (directory.clone(), destination)
        })
        .collect();

    let members: Vec<Member> = before
        .workspace_members()
        .iter()
        .map(|member| Member {
            directory: member.directory().to_path_buf(),
            package: member.package_name().to_string(),
        })
        .collect();
    refusals::ensure_none_holds_other_members(
        directories,
        &members,
        migrating.root,
        migrating.migrate_command,
    )?;
    refusals::ensure_destinations_are_free(
        &moves,
        to_directory,
        migrating.root,
        migrating.migrate_command,
    )?;
    refusals::ensure_none_holds_a_submodule(
        directories,
        migrating.repository,
        migrating.root,
        migrating.migrate_command,
    )?;

    refusals::ensure_the_moves_keep_what_git_sees(
        &relocation,
        to_directory,
        migrating.repository,
        migrating.root,
        migrating.migrate_command,
    )?;

    Ok(Planned {
        root: migrating.root.to_path_buf(),
        edited: repointed_manifests(
            before,
            migrating.root,
            &relocation,
            migrating.migrate_command,
        )?,
        relocation,
        moves,
    })
}

/// Every manifest Cargo reads, each read once and repointed in memory: the
/// workspace's own first, then every other in path order, so the order they
/// are written and reported in is the same on every run. A manifest with
/// nothing to change is left out.
///
/// A task can depend on another and any member can depend on a task, so the
/// command line crate's manifest is not the only one that names a moved
/// directory. Nor is a member: a crate outside the workspace that Cargo
/// still reads can reach a task, and so can one that crate reaches.
///
/// Git has to be able to give every manifest this edits back, so one that
/// git does not track and that needs an edit is refused before anything is
/// written.
fn repointed_manifests(
    before: &Metadata,
    root: &Path,
    relocation: &Relocation,
    migrate_command: &str,
) -> Result<Vec<Edited>, Failure> {
    let workspace_manifest = root.join("Cargo.toml");
    let mut paths = vec![workspace_manifest.clone()];
    paths.extend(
        before
            .manifests_cargo_reads()?
            .into_iter()
            .filter(|path| *path != workspace_manifest),
    );

    let mut edited = Vec::new();
    for path in paths {
        let mut manifest = Manifest::read(&path)?;
        let changes = manifest.repoint(relocation)?;
        if !changes.is_empty() {
            edited.push(Edited { manifest, changes });
        }
    }
    let first_edits: Vec<(PathBuf, String)> = edited
        .iter()
        .filter_map(|edit| {
            let change = edit.changes.first()?;
            Some((edit.manifest.path().to_path_buf(), change.to_string()))
        })
        .collect();
    refusals::ensure_git_tracks_every_edited_manifest(&first_edits, root, migrate_command)?;
    Ok(edited)
}

/// The failure for members Cargo lost or gained in the move, each spelled
/// from the project's root, or `None` when it read the members it should.
fn membership_failure(missing: &[String], gained: &[String]) -> Option<Failure> {
    let lost_clause = (!missing.is_empty()).then(|| {
        format!(
            "no longer reads {} as {}",
            join_with_and(missing),
            a_workspace_member(missing.len())
        )
    });
    let gained_clause = (!gained.is_empty()).then(|| {
        format!(
            "also reads {} as {}",
            join_with_and(gained),
            a_workspace_member(gained.len())
        )
    });
    let clauses = match (lost_clause, gained_clause) {
        (None, None) => return None,
        (Some(lost), None) => format!("Cargo {lost}"),
        (None, Some(gained)) => format!("Cargo {gained}"),
        (Some(lost), Some(gained)) => format!("Cargo {lost}, and {gained}"),
    };
    Some(Failure::new(format!("after the move, {clauses}")))
}

/// `a workspace member`, or `workspace members` for more than one.
const fn a_workspace_member(count: usize) -> &'static str {
    if count == 1 {
        "a workspace member"
    } else {
        "workspace members"
    }
}

#[cfg(test)]
mod tests {
    use super::membership_failure;

    fn words(failure: Option<rituals::Failure>) -> Option<String> {
        failure.map(|failure| failure.to_string())
    }

    #[test]
    fn the_members_it_should_read_are_no_failure() {
        assert!(membership_failure(&[], &[]).is_none());
    }

    #[test]
    fn a_member_lost_is_named() {
        assert_eq!(
            words(membership_failure(&[".rituals/x".to_string()], &[])).as_deref(),
            Some("after the move, Cargo no longer reads .rituals/x as a workspace member")
        );
    }

    #[test]
    fn two_members_lost_are_named_in_the_plural() {
        assert_eq!(
            words(membership_failure(
                &[".rituals/x".to_string(), ".rituals/y".to_string()],
                &[]
            ))
            .as_deref(),
            Some(
                "after the move, Cargo no longer reads .rituals/x and .rituals/y as workspace \
                 members"
            )
        );
    }

    #[test]
    fn a_member_gained_is_named() {
        assert_eq!(
            words(membership_failure(&[], &["tools/z".to_string()])).as_deref(),
            Some("after the move, Cargo also reads tools/z as a workspace member")
        );
    }

    #[test]
    fn two_members_gained_are_named_in_the_plural() {
        assert_eq!(
            words(membership_failure(&[], &["a".to_string(), "b".to_string()])).as_deref(),
            Some("after the move, Cargo also reads a and b as workspace members")
        );
    }

    #[test]
    fn lost_and_gained_are_joined_by_and() {
        assert_eq!(
            words(membership_failure(
                &[".rituals/x".to_string()],
                &["tools/z".to_string()]
            ))
            .as_deref(),
            Some(
                "after the move, Cargo no longer reads .rituals/x as a workspace member, and \
                 also reads tools/z as a workspace member"
            )
        );
    }
}
