//! Removing the directories a migration's moves emptied.
//!
//! This runs inside the rollback, after the moves and before Cargo is asked
//! to read the project again: whether a directory is still there decides
//! what a glob in `[workspace] members` matches, so the check that the
//! project still loads has to see the tree the run leaves, and a run that
//! fails afterwards has to put the directories back before it moves anything
//! back into them. A directory that could not be removed is still only a
//! line in the report: if leaving it there breaks nothing, the migration
//! worked, and if it does, that check says so.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use rituals_compose::rollback::Changes;

use crate::places::from_the_root;
use crate::report;

/// Directories a step moved things out of: each moved directory, and the
/// directory above them all that is emptied along with them if nothing else
/// is in it.
pub(crate) struct Vacated {
    pub(crate) boundary: PathBuf,
    pub(crate) sources: Vec<PathBuf>,
}

/// Removes every directory the moves emptied, from each source's parent up
/// to and including the boundary, recording each in `changes` so a failed
/// run creates it again, and reports each as it goes, deepest first.
///
/// A directory that still holds something stops the walk upward and is
/// reported as kept, with what is in it. A directory that could not be
/// removed or listed is reported as a line too, not a failure: whether
/// leaving it matters is for the check that follows to say.
pub(crate) fn tidy(root: &Path, vacated: &Vacated, changes: &mut Changes) -> Vec<String> {
    let mut lines = Vec::new();
    for source in &vacated.sources {
        remove_empty_ancestors(root, &vacated.boundary, source, changes, &mut lines);
    }
    lines.extend(kept(root, &vacated.boundary));
    lines
}

/// Removes each directory above `source`, nearest first, that is empty, up to
/// and including `boundary`, and stops at the first that is not.
///
/// [`Changes::remove_empty_directory`] removes a directory only if it is
/// empty, which is the check that nothing else was in it, made by the one
/// call that deletes it.
fn remove_empty_ancestors(
    root: &Path,
    boundary: &Path,
    source: &Path,
    changes: &mut Changes,
    lines: &mut Vec<String>,
) {
    for directory in source
        .ancestors()
        .skip(1)
        .take_while(|directory| directory.starts_with(boundary))
    {
        match changes.remove_empty_directory(directory) {
            Ok(()) => lines.push(report::deleted(&from_the_root(directory, root))),
            // Either an earlier source's walk already removed it and
            // everything above it that could go, or something else is in it.
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
                ) =>
            {
                return;
            }
            Err(error) => {
                lines.push(report::deletion_failed(
                    &from_the_root(directory, root),
                    error,
                ));
                return;
            }
        }
    }
}

/// What is left in `directory`, as a line, or no line when nothing is: a
/// directory that is gone was deleted and reported, and one that is empty but
/// could not be deleted was reported as that, so neither has anything to keep.
fn kept(root: &Path, directory: &Path) -> Option<String> {
    let shown = from_the_root(directory, root);
    match entries_of(directory) {
        Ok(entries) if entries.is_empty() => None,
        Ok(entries) => Some(report::kept(&shown, &entries)),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => Some(report::kept_unlisted(&shown, &error)),
    }
}

/// The names of what `directory` holds, sorted, each directory's with a
/// trailing `/`.
fn entries_of(directory: &Path) -> std::io::Result<Vec<String>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_dir() {
            entries.push(format!("{name}/"));
        } else {
            entries.push(name);
        }
    }
    entries.sort();
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rituals::Failure;
    use rituals_compose::rollback::{self, Wording};

    use super::{Vacated, tidy};
    use crate::test_support::{ScratchDir, TestOutcome};

    fn vacated(root: &Path, sources: &[&str]) -> Vacated {
        Vacated {
            boundary: root.join("tasks"),
            sources: sources.iter().map(|source| root.join(source)).collect(),
        }
    }

    /// The lines [`tidy`] reports for `sources` under `tasks/`, from a run
    /// that keeps what it removed.
    fn tidied(root: &Path, sources: &[&str]) -> Result<Vec<String>, Failure> {
        rollback::attempt(Wording::project(root, "tidying again"), |changes| {
            Ok(tidy(root, &vacated(root, sources), changes))
        })
    }

    /// A run that tidies and then fails has every directory it removed back,
    /// so the moves can be undone into them.
    #[test]
    fn a_run_that_fails_after_tidying_has_the_directories_back() -> TestOutcome {
        let scratch = ScratchDir::resolved("tidy-undone")?;
        let root = scratch.path();
        std::fs::create_dir_all(root.join("tasks/group"))?;

        let outcome = rollback::attempt(Wording::project(root, "tidying again"), |changes| {
            let lines = tidy(root, &vacated(root, &["tasks/group/deep"]), changes);
            assert_eq!(lines.len(), 2, "{lines:?}");
            Err::<(), _>(Failure::new("the project no longer loads"))
        });

        assert!(outcome.is_err());
        assert!(root.join("tasks/group").is_dir());
        Ok(())
    }

    /// The tasks moved out and nothing else was in `tasks/`: it goes, and one
    /// line says so.
    #[test]
    fn a_directory_the_moves_emptied_is_deleted() -> TestOutcome {
        let scratch = ScratchDir::resolved("tidy-empty")?;
        let root = scratch.path();
        std::fs::create_dir_all(root.join("tasks"))?;

        let lines = tidied(root, &["tasks/greet", "tasks/shout"])?;

        assert_eq!(lines, ["deleted tasks/ (empty once its tasks moved out)"]);
        assert!(!root.join("tasks").exists());
        Ok(())
    }

    #[test]
    fn a_directory_with_something_else_in_it_is_kept_and_what_is_left_is_named() -> TestOutcome {
        let scratch = ScratchDir::resolved("tidy-kept")?;
        let root = scratch.path();
        std::fs::create_dir_all(root.join("tasks/helper"))?;
        std::fs::write(root.join("tasks/notes.md"), "keep\n")?;

        let lines = tidied(root, &["tasks/greet"])?;

        assert_eq!(
            lines,
            ["kept tasks/, which still holds helper/ and notes.md"]
        );
        assert!(root.join("tasks/notes.md").is_file());
        Ok(())
    }

    /// Nested tasks leave nested directories behind; they go deepest first,
    /// and the boundary goes last once it is empty.
    #[test]
    fn nested_directories_go_deepest_first() -> TestOutcome {
        let scratch = ScratchDir::resolved("tidy-nested")?;
        let root = scratch.path();
        std::fs::create_dir_all(root.join("tasks/group"))?;

        let lines = tidied(root, &["tasks/group/deep"])?;

        assert_eq!(
            lines,
            [
                "deleted tasks/group/ (empty once its tasks moved out)",
                "deleted tasks/ (empty once its tasks moved out)",
            ]
        );
        assert!(!root.join("tasks").exists());
        Ok(())
    }

    #[test]
    fn a_nested_directory_still_holding_something_stops_the_walk_there() -> TestOutcome {
        let scratch = ScratchDir::resolved("tidy-nested-kept")?;
        let root = scratch.path();
        std::fs::create_dir_all(root.join("tasks/group/other"))?;

        let lines = tidied(root, &["tasks/group/deep"])?;

        assert_eq!(lines, ["kept tasks/, which still holds group/"]);
        assert!(root.join("tasks/group/other").is_dir());
        Ok(())
    }

    /// Two sources share a parent: it is deleted once, by whichever walk
    /// reaches it empty, and the other walk finds it gone and says nothing.
    #[test]
    fn a_directory_shared_by_two_sources_is_reported_once() -> TestOutcome {
        let scratch = ScratchDir::resolved("tidy-shared")?;
        let root = scratch.path();
        std::fs::create_dir_all(root.join("tasks/group"))?;

        let lines = tidied(root, &["tasks/group/a", "tasks/group/b"])?;

        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("tasks/group/"))
                .count(),
            1,
            "{lines:?}"
        );
        assert!(!root.join("tasks").exists());
        Ok(())
    }

    #[test]
    fn a_boundary_that_is_already_gone_reports_nothing() -> TestOutcome {
        let scratch = ScratchDir::resolved("tidy-gone")?;
        let root = scratch.path();

        let lines = tidied(root, &["tasks/greet"])?;

        assert!(lines.is_empty(), "{lines:?}");
        Ok(())
    }

    /// A directory that cannot be emptied of its entry because its parent is
    /// read-only is reported as what failed, and the run goes on.
    #[test]
    fn a_directory_that_cannot_be_deleted_is_reported_as_a_line() -> TestOutcome {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::resolved("tidy-denied")?;
        let root = scratch.path();
        std::fs::create_dir_all(root.join("tasks/group"))?;
        // `tasks/group` cannot be removed from a `tasks` that cannot be
        // written. Probed, because a process that ignores permission bits
        // (root) cannot make this failure.
        std::fs::set_permissions(root.join("tasks"), std::fs::Permissions::from_mode(0o555))?;
        let enforced = std::fs::File::create(root.join("tasks/.probe")).is_err();

        let lines = tidied(root, &["tasks/group/deep"])?;

        std::fs::set_permissions(root.join("tasks"), std::fs::Permissions::from_mode(0o755))?;
        if !enforced {
            return Ok(());
        }
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].starts_with("deleting tasks/group/ failed after every task had moved"),
            "{lines:?}"
        );
        assert_eq!(lines[1], "kept tasks/, which still holds group/");
        Ok(())
    }

    /// An empty directory that could not be deleted has nothing left in it to
    /// name, so the line that says the deletion failed is the whole report:
    /// a `kept` line with an empty list would read as a sentence cut short.
    #[test]
    fn an_empty_directory_that_cannot_be_deleted_is_reported_once_and_not_as_kept() -> TestOutcome {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::resolved("tidy-denied-empty")?;
        let root = scratch.path();
        std::fs::create_dir_all(root.join("tasks"))?;
        // `tasks` cannot be removed from a root that cannot be written.
        // Probed, because a process that ignores permission bits (root)
        // cannot make this failure.
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o555))?;
        let enforced = std::fs::File::create(root.join(".probe")).is_err();

        let lines = tidied(root, &["tasks/greet"])?;

        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o755))?;
        if !enforced {
            return Ok(());
        }
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("deleting tasks/ failed after every task had moved"),
            "{lines:?}"
        );
        Ok(())
    }
}
