//! Whether moving directories changes what git sees of the files in them.
//!
//! Git decides, by a file's path alone, whether to ignore it, which
//! attributes to give it, and whether the sparse checkout includes it. A
//! rename keeps the file and changes its path, so a rule keyed on the old
//! place stops applying and one keyed on the new place starts: a commit after
//! the move can then leave out a file that is committed now, add one that is
//! ignored now, or store one through another filter, and `git status` shows
//! each as an ordinary change.
//!
//! [`ensure_a_move_keeps_what_git_sees`] asks git, before anything moves,
//! about every file under the directories that would: tracked, untracked and
//! ignored. It asks about each at its place now, in the real work tree, and
//! at its place afterwards, in a work tree of copies that holds the
//! `.gitignore` and `.gitattributes` files where they will be, so that the
//! ones that move with a directory are seen at the new place. Git's own
//! reading of the repository's configuration and the index is the same for
//! both. Nothing is written to the project or the repository.
//!
//! Only rules the project carries count. A rule outside the repository, in a
//! global excludes file or `.git/info/exclude`, is not considered, because a
//! project has to work from a fresh clone.
//!
//! # Examples
//!
//! Asking, before moving `tasks/greet` into `.rituals/`, whether git would
//! see its files differently there:
//!
//! ```no_run
//! use std::path::Path;
//!
//! use rituals_compose::git::{self, SeenDifferently};
//! use rituals_compose::relocation::Relocation;
//!
//! // Needs a real repository on disk and runs `git`, so this example is
//! // `no_run`.
//! let root = Path::new("/work/project");
//! let relocation = Relocation::new(
//!     &root.join("tasks"),
//!     &root.join(".rituals"),
//!     [root.join("tasks/greet")],
//! );
//! match git::ensure_a_move_keeps_what_git_sees(&relocation, root) {
//!     Ok(()) => println!("git sees the files the same way after the move"),
//!     Err(SeenDifferently::WouldBeIgnored(files)) => {
//!         for ignored in &files {
//!             println!("git would ignore {}: {}", ignored.file().to().display(), ignored.rule());
//!         }
//!     }
//!     Err(other) => println!("git would see them differently: {other}"),
//! }
//! ```

use std::path::Path;
use std::process::Command;

use crate::relocation::Relocation;

mod attribute;
mod attribute_change;
mod attribute_state;
mod attributes;
mod ignore;
mod ignore_rule;
mod ignored_file;
mod listing;
#[cfg(test)]
mod matrix;
mod moved_file;
mod places;
mod prediction;
mod scratch;
mod seen_differently;
mod sparse;
mod verdict;

pub use attribute::Attribute;
pub use attribute_change::AttributeChange;
pub use attribute_state::AttributeState;
pub use ignore_rule::IgnoreRule;
pub use ignored_file::IgnoredFile;
pub use moved_file::MovedFile;
pub use seen_differently::SeenDifferently;

/// The arguments of a question put to git about `scope`'s tree: `scope`'s
/// options, which choose the tree, ahead of `command`.
fn scoped<'a>(scope: &'a [String], command: &[&'static str]) -> Vec<&'a str> {
    scope
        .iter()
        .map(String::as_str)
        .chain(command.iter().copied())
        .collect()
}

/// Checks, with the `git` on `PATH`, that moving the directories `relocation`
/// moves changes nothing about what git sees of the files in them.
///
/// Asks, for every file under a moved directory, whether git ignores it now
/// and would ignore it afterwards, which attributes git gives it at each
/// place, and whether the checkout's sparse-checkout patterns would include
/// the new one. A tracked file counts as seen whatever a rule says about it
/// now, since a rule does not stop git tracking what it already tracks: a
/// rule that matches both places would still leave the file out of a commit
/// that moves it, which is what [`SeenDifferently::WouldBeIgnored`] says. A
/// tracked file git has been told not to look at is refused too, because a
/// commit after the move could carry an edit `git status` never showed.
///
/// A rule file that moves with a directory counts at its new place, and a
/// symbolic link is not read as one, which is how git treats it. Ignored
/// files are listed and asked about like any other, so a rule that stops
/// ignoring a build directory is found. A tracked file marked skip-worktree
/// that is not on disk is not moved by a rename and is left out.
///
/// Every path in the answer is spelled from git's top level.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::git::{self, SeenDifferently};
/// use rituals_compose::relocation::Relocation;
///
/// // Needs a real repository on disk and runs `git`, so this example is
/// // `no_run`.
/// let root = Path::new("/work/project");
/// let relocation = Relocation::new(
///     &root.join("tasks"),
///     &root.join(".rituals"),
///     [root.join("tasks/greet")],
/// );
/// if let Err(SeenDifferently::AttributesWouldChange(changes)) =
///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
/// {
///     for change in &changes {
///         println!("{} would be given other attributes", change.file().from().display());
///     }
/// }
/// ```
///
/// # Errors
///
/// Returns a [`SeenDifferently`] naming the first kind of difference that
/// holds for any file, with every file it holds for, or
/// [`SeenDifferently::Unanswered`] when git could not answer: when `git`
/// cannot be run, when the project is not in a git repository, when git says
/// anything else went wrong, or when the system's temporary directory cannot
/// hold the copies git is asked about.
pub fn ensure_a_move_keeps_what_git_sees(
    relocation: &Relocation,
    workspace_root: &Path,
) -> Result<(), SeenDifferently> {
    ensure_with(|| Command::new("git"), relocation, workspace_root)
}

/// [`ensure_a_move_keeps_what_git_sees`], with `git` started from `new_git`
/// each time it is needed.
fn ensure_with(
    new_git: impl Fn() -> Command,
    relocation: &Relocation,
    workspace_root: &Path,
) -> Result<(), SeenDifferently> {
    let prediction = prediction::predict(&new_git, relocation, workspace_root)?;
    verdict::judge(&prediction)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::ensure_with;
    use crate::git::fixture::{commit_everything, git};
    use crate::git::test_support::contained_in;
    use crate::git::{SeenDifferently, Unanswered};
    use crate::relocation::Relocation;
    use crate::test_support::{ScratchDir, TestOutcome};

    fn write(root: &Path, file: &str, contents: &str) -> TestOutcome {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().ok_or("a fixture file has a directory")?)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    fn greet_moves(root: &Path) -> Relocation {
        Relocation::new(
            &root.join("tasks"),
            &root.join(".rituals"),
            [root.join("tasks/greet")],
        )
    }

    /// Every file under `root`, `.git` included, with its bytes, so a test
    /// can say that nothing was written anywhere. Walks with a stack of
    /// directories still to read.
    fn everything_under(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, std::io::Error> {
        let mut found = BTreeMap::new();
        let mut unread = vec![root.to_path_buf()];
        while let Some(directory) = unread.pop() {
            for entry in std::fs::read_dir(&directory)? {
                let path = entry?.path();
                if path.is_dir() {
                    unread.push(path);
                } else {
                    let bytes = std::fs::read(&path)?;
                    found.insert(path, bytes);
                }
            }
        }
        Ok(found)
    }

    /// A refusal and an answer that nothing differs both leave the project
    /// and the repository byte for byte as they were: the check runs while a
    /// move is only planned, and a git command that took a lock or refreshed
    /// the index would make the refusal's promise of an untouched project
    /// false.
    #[test]
    fn asking_writes_nothing_to_the_project_or_the_repository() -> TestOutcome {
        let scratch = ScratchDir::new("sight-writes-nothing")?;
        let root = scratch.path();
        write(root, "tasks/greet/Cargo.toml", "[package]\n")?;
        write(root, "tasks/greet/.env", "SECRET=1\n")?;
        write(root, ".gitignore", "/target\ntasks/greet/.env\n")?;
        git(root, &["init", "--quiet"])?;
        commit_everything(root)?;
        let before = everything_under(root)?;

        let refused = ensure_with(contained_in(root), &greet_moves(root), root);
        assert!(
            matches!(refused, Err(SeenDifferently::WouldNoLongerBeIgnored(_))),
            "expected the first ask to be refused, got {refused:?}"
        );
        assert_eq!(
            everything_under(root)?,
            before,
            "a refused ask changed something"
        );

        write(root, ".gitignore", "/target\n")?;
        let before_the_answer = everything_under(root)?;
        let answered = ensure_with(contained_in(root), &greet_moves(root), root);
        assert_eq!(answered, Ok(()));
        assert_eq!(
            everything_under(root)?,
            before_the_answer,
            "an ask that found nothing changed something"
        );
        Ok(())
    }

    #[test]
    fn a_move_of_a_directory_with_no_files_changes_nothing_git_sees() -> TestOutcome {
        let scratch = ScratchDir::new("sight-no-files")?;
        let root = scratch.path();
        write(root, "tasks/other/Cargo.toml", "[package]\n")?;
        std::fs::create_dir_all(root.join("tasks/greet"))?;
        git(root, &["init", "--quiet"])?;
        commit_everything(root)?;

        assert_eq!(
            ensure_with(contained_in(root), &greet_moves(root), root),
            Ok(())
        );
        Ok(())
    }

    #[test]
    fn outside_any_repository_is_refused_as_no_repository() -> TestOutcome {
        let scratch = ScratchDir::new("sight-no-repository")?;
        let root = scratch.path();
        write(root, "tasks/greet/Cargo.toml", "[package]\n")?;

        assert_eq!(
            ensure_with(contained_in(root), &greet_moves(root), root),
            Err(SeenDifferently::Unanswered(Unanswered::NotARepository))
        );
        Ok(())
    }

    #[test]
    fn a_missing_git_binary_is_refused() -> TestOutcome {
        let scratch = ScratchDir::new("sight-git-missing")?;
        let root = scratch.path();
        write(root, "tasks/greet/Cargo.toml", "[package]\n")?;

        assert_eq!(
            ensure_with(
                || Command::new("ritual-test-no-such-git"),
                &greet_moves(root),
                root
            ),
            Err(SeenDifferently::Unanswered(Unanswered::GitMissing))
        );
        Ok(())
    }
}
