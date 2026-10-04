//! What the index records: gitlinks, and the flags that make `git status`
//! stop looking at a file; and what `.gitattributes` says about each.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Unanswered, canonical, from_the_top_level, run_git, top_level_of};

/// A tracked file git has been told not to look at, and which flag says so.
///
/// Only [`ensure_git_can_give_back`](super::ensure_git_can_give_back) builds
/// one, from what git reports, so every value a caller reads is one git
/// vouched for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unwatched {
    path: PathBuf,
    flag: Flag,
}

impl Unwatched {
    pub(super) const fn new(path: PathBuf, flag: Flag) -> Self {
        Self { path, flag }
    }

    /// The file, spelled from git's top level.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, CannotGiveBack};
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// if let Err(CannotGiveBack::Unwatched(files)) =
    ///     git::ensure_git_can_give_back(Path::new(".rituals/lint"), Path::new("."))
    /// {
    ///     for file in &files {
    ///         println!("git ignores changes to {}", file.path().display());
    ///     }
    /// }
    /// ```
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The flag that makes git stop looking at the file.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, CannotGiveBack, Flag};
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// if let Err(CannotGiveBack::Unwatched(files)) =
    ///     git::ensure_git_can_give_back(Path::new(".rituals/lint"), Path::new("."))
    /// {
    ///     let skipped = files.iter().filter(|file| file.flag() == Flag::SkipWorktree);
    ///     println!("{} files are skip-worktree", skipped.count());
    /// }
    /// ```
    #[must_use]
    pub const fn flag(&self) -> Flag {
        self.flag
    }
}

/// The index flags that make `git status` stop looking at a file.
///
/// # Examples
///
/// ```
/// use rituals_compose::git::Flag;
///
/// let name = |flag| match flag {
///     Flag::AssumeUnchanged => "assume-unchanged",
///     Flag::SkipWorktree => "skip-worktree",
/// };
/// assert_eq!(name(Flag::SkipWorktree), "skip-worktree");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    /// `git update-index --assume-unchanged`: an edit is never seen, and
    /// `git checkout` gives back the committed file without it.
    AssumeUnchanged,
    /// `git update-index --skip-worktree`: an edit is never seen, and `git
    /// checkout` does not give the file back at all.
    SkipWorktree,
}

/// Lists every submodule at `directory` or inside it, spelled from git's top
/// level.
///
/// A submodule is a gitlink in the project's index, so `directory` is one when
/// the project records it as a commit rather than as files, and holds one when
/// something under it is. The project's git is asked, from the directory
/// above `directory`, which is how a `directory` that is itself a submodule
/// is found: asked from inside it, git would be talking about the submodule's
/// own repository.
///
/// Nothing found is an empty list and not a failure.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::git;
///
/// // Needs a real repository on disk and runs `git`, so this example is
/// // `no_run`.
/// for submodule in git::submodules_under(Path::new(".rituals/lint"))? {
///     println!("{} is a git submodule", submodule.display());
/// }
/// # Ok::<(), rituals_compose::git::Unanswered>(())
/// ```
///
/// # Errors
///
/// Returns [`Unanswered::GitMissing`] when `git` cannot be run,
/// [`Unanswered::NotARepository`] when `directory` is not in a git repository,
/// and [`Unanswered::Failed`] for anything else git reports or prints that
/// cannot be read.
pub fn submodules_under(directory: &Path) -> Result<Vec<PathBuf>, Unanswered> {
    submodules_under_with(|| Command::new("git"), directory)
}

/// [`submodules_under`], with `git` started from `new_git`.
fn submodules_under_with(
    new_git: impl Fn() -> Command,
    directory: &Path,
) -> Result<Vec<PathBuf>, Unanswered> {
    let Some(holder) = directory.parent() else {
        return Err(Unanswered::Failed(format!(
            "{} has no directory holding it to ask git from",
            directory.display()
        )));
    };
    let top_level = top_level_of(&new_git, holder)?;
    let from_top_level = from_the_top_level(&canonical(holder)?, &top_level)?;
    let Some(name) = directory.file_name() else {
        return Err(Unanswered::Failed(format!(
            "{} has no name to ask git about",
            directory.display()
        )));
    };

    // Literal, so a directory whose name holds `*` or `?` is not read as a
    // pattern matching others. A directory name as a pathspec matches the
    // directory itself, as a gitlink, and everything under it.
    let pathspec = format!(":(literal){}", name.to_string_lossy());
    let index = parse_index(&run_git(
        &new_git,
        holder,
        &["ls-files", "--stage", "-v", "-z", "--", &pathspec],
    )?)?;
    Ok(index
        .into_iter()
        .filter(|entry| entry.is_gitlink)
        .map(|entry| from_top_level.join(entry.path))
        .collect())
}

/// One entry of `git ls-files --stage -v -z`, as far as this check reads
/// it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct IndexEntry {
    pub(super) path: PathBuf,
    pub(super) is_gitlink: bool,
    pub(super) flag: Option<Flag>,
}

/// Every entry in `git ls-files --stage -v -z` output.
///
/// An entry is a one-letter tag and a space, then the mode, the object name
/// and the stage, separated by spaces, then a tab and the path, ended by a
/// NUL. The tag is lowercase for an entry marked assume-unchanged and `S`
/// for one marked skip-worktree. An entry this cannot read is a failure
/// rather than something to skip, since skipping it could hide a submodule
/// or a flag.
pub(super) fn parse_index(output: &[u8]) -> Result<Vec<IndexEntry>, Unanswered> {
    let text = String::from_utf8_lossy(output);
    let mut entries = Vec::new();
    for entry in text.split('\0').filter(|entry| !entry.is_empty()) {
        let unreadable = || {
            Unanswered::Failed(format!(
                "git ls-files printed an entry this check cannot read: {entry:?}"
            ))
        };
        let (fields, path) = entry.split_once('\t').ok_or_else(unreadable)?;
        let (tag, rest) = fields.split_once(' ').ok_or_else(unreadable)?;
        let mut tag_letters = tag.chars();
        let (Some(letter), None) = (tag_letters.next(), tag_letters.next()) else {
            return Err(unreadable());
        };
        let flag = if letter.is_ascii_lowercase() {
            Some(Flag::AssumeUnchanged)
        } else if letter == 'S' {
            Some(Flag::SkipWorktree)
        } else {
            None
        };
        entries.push(IndexEntry {
            path: PathBuf::from(path),
            is_gitlink: rest.starts_with("160000 "),
            flag,
        });
    }
    Ok(entries)
}

/// Every path and its `filter` value in `git check-attr -z filter` output.
///
/// Each answer is three NUL-ended fields: the path, the attribute's name,
/// and its value — `unspecified`, `unset`, `set`, or the value it was given.
/// Output that does not come in threes is a failure, since misreading it
/// could pair a file with the wrong filter.
pub(super) fn parse_attributes(output: &[u8]) -> Result<Vec<(PathBuf, String)>, Unanswered> {
    let text = String::from_utf8_lossy(output);
    let fields: Vec<&str> = text.split_terminator('\0').collect();
    let answers = fields.chunks_exact(3);
    if !answers.remainder().is_empty() {
        return Err(Unanswered::Failed(format!(
            "git check-attr printed output this check cannot read: {text:?}"
        )));
    }
    Ok(answers
        .map(|answer| (PathBuf::from(answer[0]), answer[2].to_string()))
        .collect())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{Flag, IndexEntry, parse_attributes, parse_index, submodules_under_with};
    use crate::git::Unanswered;
    use crate::git::test_support::{add_a_submodule, commit_everything, contained_in, git};
    use crate::test_support::{ScratchDir, TestOutcome};

    #[test]
    fn the_index_is_read_for_gitlinks_and_flags() -> TestOutcome {
        let output = b"H 100644 0123456789abcdef0123456789abcdef01234567 0\tsrc/lib.rs\0\
            H 160000 89abcdef0123456789abcdef0123456789abcdef 0\tvendor/upstream\0\
            h 100755 0123456789abcdef0123456789abcdef01234567 0\trun.sh\0\
            S 100644 0123456789abcdef0123456789abcdef01234567 0\tlocal.toml\0";
        let entry = |path: &str, is_gitlink, flag| IndexEntry {
            path: PathBuf::from(path),
            is_gitlink,
            flag,
        };
        assert_eq!(
            parse_index(output).map_err(|error| format!("{error:?}"))?,
            [
                entry("src/lib.rs", false, None),
                entry("vendor/upstream", true, None),
                entry("run.sh", false, Some(Flag::AssumeUnchanged)),
                entry("local.toml", false, Some(Flag::SkipWorktree)),
            ]
        );
        Ok(())
    }

    #[test]
    fn an_index_entry_this_cannot_read_is_a_failure_not_a_skip() {
        for output in [
            &b"H 160000 89abcdef 0\0"[..],
            &b"160000 89abcdef 0\tno-tag\0"[..],
            &b"HS 100644 89abcdef 0\ttwo-letter-tag\0"[..],
        ] {
            let result = parse_index(output);
            assert!(
                matches!(
                    &result,
                    Err(Unanswered::Failed(message)) if message.contains("cannot read")
                ),
                "expected {output:?} to be refused, got {result:?}"
            );
        }
    }

    #[test]
    fn attributes_are_read_in_threes() -> TestOutcome {
        let output = b"a.cfg\0filter\0strip\0b.rs\0filter\0unspecified\0";
        assert_eq!(
            parse_attributes(output).map_err(|error| format!("{error:?}"))?,
            [
                (PathBuf::from("a.cfg"), "strip".to_string()),
                (PathBuf::from("b.rs"), "unspecified".to_string()),
            ]
        );
        let result = parse_attributes(b"a.cfg\0filter\0");
        assert!(
            matches!(&result, Err(Unanswered::Failed(message)) if message.contains("cannot read")),
            "expected output not in threes to be refused, got {result:?}"
        );
        Ok(())
    }

    /// A repository with `tasks/greet` and `tasks/shout` committed, and the
    /// scratch directory holding it.
    fn committed_tasks(tag: &str) -> Result<ScratchDir, Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new(tag)?;
        for task in ["greet", "shout"] {
            std::fs::create_dir_all(scratch.path().join("tasks").join(task))?;
            std::fs::write(
                scratch.path().join("tasks").join(task).join("lib.rs"),
                "// c\n",
            )?;
        }
        git(scratch.path(), &["init"])?;
        commit_everything(scratch.path())?;
        Ok(scratch)
    }

    fn submodules(scratch: &Path, directory: &Path) -> Result<Vec<PathBuf>, Unanswered> {
        submodules_under_with(contained_in(scratch), directory)
    }

    #[test]
    fn a_directory_with_no_submodule_has_none() -> TestOutcome {
        let scratch = committed_tasks("submodules-none")?;

        assert_eq!(
            submodules(scratch.path(), &scratch.path().join("tasks/greet")),
            Ok(Vec::new())
        );
        Ok(())
    }

    /// A submodule inside the directory is named from the top level, and one
    /// beside it, in a directory that is not asked about, is not.
    #[test]
    fn a_submodule_inside_the_directory_is_named_from_the_top_level() -> TestOutcome {
        let scratch = committed_tasks("submodules-inside")?;
        add_a_submodule(scratch.path(), "tasks/greet/vendor/upstream")?;

        assert_eq!(
            submodules(scratch.path(), &scratch.path().join("tasks/greet")),
            Ok(vec![PathBuf::from("tasks/greet/vendor/upstream")])
        );
        assert_eq!(
            submodules(scratch.path(), &scratch.path().join("tasks/shout")),
            Ok(Vec::new())
        );
        Ok(())
    }

    /// The directory is itself the submodule: the project's git records only
    /// the commit it points at, so that is the gitlink, named as the
    /// directory, whether or not the submodule has been checked out.
    #[test]
    fn a_directory_that_is_itself_a_submodule_is_named() -> TestOutcome {
        let scratch = committed_tasks("submodules-itself")?;
        add_a_submodule(scratch.path(), "tasks/vendored")?;

        assert_eq!(
            submodules(scratch.path(), &scratch.path().join("tasks/vendored")),
            Ok(vec![PathBuf::from("tasks/vendored")])
        );
        Ok(())
    }

    #[test]
    fn a_directory_outside_any_repository_is_refused_as_no_repository_when_listing_submodules()
    -> TestOutcome {
        let scratch = ScratchDir::new("submodules-no-repository")?;
        std::fs::create_dir(scratch.path().join("tasks"))?;

        assert_eq!(
            submodules(scratch.path(), &scratch.path().join("tasks")),
            Err(Unanswered::NotARepository)
        );
        Ok(())
    }

    #[test]
    fn a_missing_git_binary_is_refused_when_listing_submodules() -> TestOutcome {
        let scratch = ScratchDir::new("submodules-git-missing")?;

        assert_eq!(
            submodules_under_with(|| Command::new("ritual-test-no-such-git"), scratch.path()),
            Err(Unanswered::GitMissing)
        );
        Ok(())
    }
}
