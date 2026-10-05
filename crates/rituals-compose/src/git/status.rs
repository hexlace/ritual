//! What `git status` says is untracked, ignored or changed.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Unanswered, counted_with_verb, run_git, top_level_of};

/// Why the work tree could not be called clean.
///
/// Holds [`Unanswered`] for the three facts every question here can fail on,
/// and the files a commit would have to carry for the one this question adds.
/// It states git's facts and no remedy, so a caller words its own refusal from
/// the variant it receives. See [`Unanswered`] for why the variants are public
/// and the enum is exhaustive.
///
/// # Examples
///
/// ```
/// use rituals_compose::git::NotClean;
///
/// assert_eq!(
///     NotClean::Dirty(vec!["a.txt".into(), "b.txt".into()]).to_string(),
///     "2 files are untracked, ignored or changed since the last commit"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotClean {
    /// Git could not answer at all.
    Unanswered(Unanswered),
    /// Files a commit would have to carry: untracked, or changed since the
    /// last commit, spelled from git's top level.
    Dirty(Vec<PathBuf>),
}

impl From<Unanswered> for NotClean {
    fn from(unanswered: Unanswered) -> Self {
        Self::Unanswered(unanswered)
    }
}

impl fmt::Display for NotClean {
    /// Says git's fact in lowercase, with no remedy and no trailing
    /// punctuation, so a caller can put it inside a sentence of its own.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unanswered(unanswered) => fmt::Display::fmt(unanswered, formatter),
            Self::Dirty(files) => write!(
                formatter,
                "{} untracked, ignored or changed since the last commit",
                counted_with_verb(files.len())
            ),
        }
    }
}

impl std::error::Error for NotClean {}

/// Checks that the work tree of the repository `directory` is in has nothing
/// that is not committed, and returns the repository's top level.
///
/// Anything a commit would have to carry makes it dirty: a tracked file
/// changed in the work tree or the index, and a file that is not tracked and
/// not ignored. Ignored files do not, since a commit would not carry them
/// either. The whole work tree is asked about, whichever directory of it
/// `directory` is.
///
/// The top level is resolved through symbolic links, the form every other
/// check here compares paths in.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::git::{self, NotClean};
///
/// // Needs a real repository on disk and runs `git`, so this example is
/// // `no_run`.
/// match git::ensure_work_tree_is_clean(Path::new(".")) {
///     Ok(top_level) => println!("everything in {} is committed", top_level.display()),
///     Err(NotClean::Dirty(files)) => println!("{} paths are not committed", files.len()),
///     Err(other) => println!("git could not say: {other}"),
/// }
/// ```
///
/// # Errors
///
/// Returns [`NotClean::Unanswered`] when git could not answer, and
/// [`NotClean::Dirty`] naming every path that is not committed, from the top
/// level.
pub fn ensure_work_tree_is_clean(directory: &Path) -> Result<PathBuf, NotClean> {
    ensure_work_tree_is_clean_with(|| Command::new("git"), directory)
}

/// [`ensure_work_tree_is_clean`], with `git` started from `new_git`.
fn ensure_work_tree_is_clean_with(
    new_git: impl Fn() -> Command,
    directory: &Path,
) -> Result<PathBuf, NotClean> {
    let top_level = top_level_of(&new_git, directory)?;
    let dirty = uncommitted(&new_git, directory, &[], IgnoredFiles::Skip)?;
    if dirty.is_empty() {
        Ok(top_level)
    } else {
        Err(NotClean::Dirty(dirty))
    }
}

/// Whether the files git ignores are among the ones that count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IgnoredFiles {
    /// An ignored file is one git cannot give back, so it counts.
    Count,
    /// An ignored file is not something a commit carries, so it does not.
    Skip,
}

/// Every path `pathspecs` match, asked from `asked_in`, that is untracked,
/// changed since the last commit, or, when `ignored` says so, ignored, from
/// git's top level. No pathspec at all is the whole work tree, and nothing
/// dirty is an empty list.
pub(super) fn uncommitted(
    new_git: &impl Fn() -> Command,
    asked_in: &Path,
    pathspecs: &[&str],
    ignored: IgnoredFiles,
) -> Result<Vec<PathBuf>, Unanswered> {
    let mut arguments = vec![
        "--no-optional-locks",
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
    ];
    match ignored {
        IgnoredFiles::Count => arguments.push("--ignored=matching"),
        IgnoredFiles::Skip => {}
    }
    arguments.push("--");
    arguments.extend_from_slice(pathspecs);
    let status = run_git(new_git, asked_in, &arguments)?;
    // Git names each path in the status from the top level, whichever
    // directory it was asked from.
    Ok(parse_porcelain(&status)?
        .into_iter()
        .map(PathBuf::from)
        .collect())
}

/// The path of every entry in `git status --porcelain=v1 -z` output.
///
/// An entry is two status letters, a space, then the path, ended by a NUL.
/// A rename or a copy is followed by one more NUL-ended field, the path it
/// came from, which is not an entry of its own. An entry too short to hold
/// its status is a failure rather than something to skip, since skipping it
/// could hide a file git cannot give back.
pub(super) fn parse_porcelain(output: &[u8]) -> Result<Vec<String>, Unanswered> {
    let text = String::from_utf8_lossy(output);
    let mut fields = text.split('\0');
    let mut paths = Vec::new();
    while let Some(field) = fields.next() {
        if field.is_empty() {
            // The output ends with a NUL, which leaves one empty field.
            continue;
        }
        let Some((status, path)) = field.split_at_checked(3) else {
            return Err(Unanswered::Failed(format!(
                "git status printed an entry this check cannot read: {field:?}"
            )));
        };
        if status.contains(['R', 'C']) {
            fields.next();
        }
        paths.push(path.to_string());
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{ensure_work_tree_is_clean_with, parse_porcelain};
    use crate::git::fixture::{commit_everything, git};
    use crate::git::test_support::contained_in;
    use crate::git::{NotClean, Unanswered};
    use crate::test_support::{ScratchDir, TestOutcome};

    /// Every variant renders as git's fact, in lowercase, with no remedy and
    /// no trailing full stop, so a caller can build a sentence around it.
    #[test]
    fn every_not_clean_says_git_s_fact_in_lowercase_with_no_remedy() {
        let cases = [
            (
                NotClean::Unanswered(Unanswered::NotARepository),
                "the directory is not in a git repository",
            ),
            (
                NotClean::Dirty(vec![PathBuf::from("a.txt")]),
                "1 file is untracked, ignored or changed since the last commit",
            ),
            (
                NotClean::Dirty(vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")]),
                "2 files are untracked, ignored or changed since the last commit",
            ),
        ];
        for (not_clean, expected) in cases {
            assert_eq!(not_clean.to_string(), expected);
            assert!(
                !expected.ends_with('.'),
                "{expected:?} must not end in a full stop"
            );
        }
    }

    /// Exercises the parser on one entry of each kind `--porcelain=v1 -z`
    /// prints that `remove` has to refuse over: untracked, ignored, modified
    /// in the work tree, modified in the index, added, and deleted. Each
    /// path is named exactly once and in the order git printed it.
    #[test]
    fn porcelain_names_every_untracked_ignored_and_changed_path() -> TestOutcome {
        // The continuation strips the next line's leading whitespace, so the
        // space that starts ` D` is written as an escape.
        let output = b"?? untracked.txt\0!! target/\0 M worktree.rs\0M  staged.rs\0A  added.rs\0\
            \x20D gone.rs\0";
        assert_eq!(
            parse_porcelain(output).map_err(|error| format!("{error:?}"))?,
            [
                "untracked.txt",
                "target/",
                "worktree.rs",
                "staged.rs",
                "added.rs",
                "gone.rs"
            ]
        );
        Ok(())
    }

    /// In `-z` mode a rename or a copy is followed by the path it came from
    /// as a field with no status of its own. Reading that field as an entry
    /// would name a path that is not a file in the directory, or fail on it.
    #[test]
    fn a_rename_entry_consumes_its_original_path() -> TestOutcome {
        let output = b"R  new.rs\0old.rs\0C  copy.rs\0source.rs\0?? other.txt\0";
        assert_eq!(
            parse_porcelain(output).map_err(|error| format!("{error:?}"))?,
            ["new.rs", "copy.rs", "other.txt"]
        );
        Ok(())
    }

    #[test]
    fn an_entry_too_short_to_hold_a_status_is_a_failure_not_a_skip() {
        let result = parse_porcelain(b"?? fine.txt\0x\0");
        assert!(
            matches!(&result, Err(Unanswered::Failed(message)) if message.contains("\"x\"")),
            "expected the unreadable entry to be named, got {result:?}"
        );
    }

    /// A repository at the root of a scratch directory with `tasks/greet`
    /// committed, and `ignored.txt` and `target/` ignored.
    fn committed_project(tag: &str) -> Result<ScratchDir, Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new(tag)?;
        std::fs::create_dir_all(scratch.path().join("tasks/greet/src"))?;
        std::fs::write(
            scratch.path().join("tasks/greet/src/lib.rs"),
            "// committed\n",
        )?;
        std::fs::write(scratch.path().join(".gitignore"), "ignored.txt\ntarget/\n")?;
        git(scratch.path(), &["init"])?;
        commit_everything(scratch.path())?;
        Ok(scratch)
    }

    fn clean(scratch: &Path, directory: &Path) -> Result<PathBuf, NotClean> {
        ensure_work_tree_is_clean_with(contained_in(scratch), directory)
    }

    #[test]
    fn a_work_tree_with_nothing_to_commit_is_clean_and_names_its_top_level() -> TestOutcome {
        let scratch = committed_project("status-clean")?;

        assert_eq!(
            clean(scratch.path(), scratch.path()),
            Ok(std::fs::canonicalize(scratch.path())?)
        );
        Ok(())
    }

    /// One file of each kind a commit would have to deal with: a tracked
    /// file edited, one staged and not committed, and one untracked. Each is
    /// named from the top level.
    #[test]
    fn changed_staged_and_untracked_files_make_the_work_tree_dirty() -> TestOutcome {
        let scratch = committed_project("status-dirty")?;
        std::fs::write(scratch.path().join("tasks/greet/src/lib.rs"), "// edited\n")?;
        std::fs::write(scratch.path().join("staged.txt"), "staged\n")?;
        git(scratch.path(), &["add", "staged.txt"])?;
        std::fs::write(scratch.path().join("untracked.txt"), "new\n")?;

        let result = clean(scratch.path(), scratch.path());

        let Err(NotClean::Dirty(mut files)) = result else {
            return Err(format!("expected the work tree to be dirty, got {result:?}").into());
        };
        files.sort();
        assert_eq!(
            files,
            [
                PathBuf::from("staged.txt"),
                PathBuf::from("tasks/greet/src/lib.rs"),
                PathBuf::from("untracked.txt"),
            ]
        );
        Ok(())
    }

    /// Ignored files are the project's own business: a build directory is
    /// not something a commit would carry.
    #[test]
    fn ignored_files_do_not_make_the_work_tree_dirty() -> TestOutcome {
        let scratch = committed_project("status-ignored")?;
        std::fs::write(scratch.path().join("ignored.txt"), "ignored\n")?;
        std::fs::create_dir_all(scratch.path().join("target/debug"))?;
        std::fs::write(scratch.path().join("target/debug/greet.d"), "built\n")?;

        assert_eq!(
            clean(scratch.path(), scratch.path()),
            Ok(std::fs::canonicalize(scratch.path())?)
        );
        Ok(())
    }

    /// The whole work tree is asked about, not the directory the question is
    /// put from: a file outside it is named, from the top level.
    #[test]
    fn asked_from_a_subdirectory_it_still_covers_the_whole_work_tree() -> TestOutcome {
        let scratch = committed_project("status-subdirectory")?;
        std::fs::write(scratch.path().join("outside.txt"), "outside\n")?;
        let subdirectory = scratch.path().join("tasks/greet");

        assert_eq!(
            clean(scratch.path(), &subdirectory),
            Err(NotClean::Dirty(vec![PathBuf::from("outside.txt")]))
        );

        std::fs::remove_file(scratch.path().join("outside.txt"))?;
        assert_eq!(
            clean(scratch.path(), &subdirectory),
            Ok(std::fs::canonicalize(scratch.path())?),
            "the top level is the repository's, not the directory asked from"
        );
        Ok(())
    }

    #[test]
    fn outside_any_repository_is_refused_as_no_repository() -> TestOutcome {
        let scratch = ScratchDir::new("status-no-repository")?;

        assert_eq!(
            clean(scratch.path(), scratch.path()),
            Err(NotClean::Unanswered(Unanswered::NotARepository))
        );
        Ok(())
    }

    #[test]
    fn a_missing_git_binary_is_refused_when_asking_whether_the_work_tree_is_clean() -> TestOutcome {
        let scratch = ScratchDir::new("status-git-missing")?;

        assert_eq!(
            ensure_work_tree_is_clean_with(
                || Command::new("ritual-test-no-such-git"),
                scratch.path()
            ),
            Err(NotClean::Unanswered(Unanswered::GitMissing))
        );
        Ok(())
    }
}
