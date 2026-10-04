//! What git can say about a project's work tree.
//!
//! Whether it can give back every file in a directory that is about to be
//! deleted or moved, whether the work tree is clean, which directories hold
//! Cargo configuration, which entries are submodules, and which files mention
//! a pattern.
//!
//! Every function runs the `git` on `PATH`, answers from what git itself
//! reports, and is read-only: none of them writes to the project or takes
//! the repository's locks. A failure is an [`Obstacle`], one vocabulary for
//! the facts git can fail to vouch for, so a task words its own refusal from
//! the variant it receives and names the command that suits its own run.
//! Each public function says which variants it can return.
//!
//! Paths git reports are spelled from git's top level, the form the `:/`
//! pathspec takes, so a person can hand one to git from any directory of
//! the project.
//!
//! # Examples
//!
//! Asking whether git can give back everything in a directory before
//! deleting it:
//!
//! ```no_run
//! use std::path::Path;
//!
//! use rituals_compose::git::{self, Obstacle};
//!
//! // Needs a real repository on disk and runs `git`, so this example is
//! // `no_run`.
//! let workspace_root = Path::new(".");
//! match git::ensure_git_can_give_back(Path::new(".rituals/lint"), workspace_root) {
//!     Ok(from_the_top_level) => println!("git can give back {}", from_the_top_level.display()),
//!     Err(Obstacle::NotARepository) => println!("not in a git repository"),
//!     Err(other) => println!("git cannot vouch for it: {other}"),
//! }
//! ```

use std::fmt;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

mod configuration;
mod give_back;
mod index;
mod mentions;
mod status;
#[cfg(test)]
mod test_support;

pub use configuration::directories_holding_cargo_configuration;
pub use give_back::ensure_git_can_give_back;
pub use index::{Flag, Unwatched, submodules_under};
pub use mentions::files_mentioning;
pub use status::ensure_work_tree_is_clean;

/// Why git cannot vouch for a part of the work tree.
///
/// Every path a variant holds is spelled from git's top level, the form the
/// `:/` pathspec takes, so a person can hand it to git from any directory of
/// the project — except [`Obstacle::OwnRepository`]'s, which is the
/// directory's own. The variants state git's facts and no remedy: a caller
/// builds its own refusal from the one it receives, in its own words.
///
/// # Examples
///
/// ```
/// use rituals_compose::git::Obstacle;
///
/// assert_eq!(Obstacle::GitMissing.to_string(), "`git` could not be run");
/// assert_eq!(
///     Obstacle::Dirty(vec!["a.txt".into(), "b.txt".into()]).to_string(),
///     "2 files are untracked, ignored or changed since the last commit"
/// );
/// ```
#[derive(Debug, PartialEq, Eq)]
pub enum Obstacle {
    /// `git` could not be started because there is no such program.
    GitMissing,
    /// The directory is not inside a git repository.
    NotARepository,
    /// The directory is, or contains, a git repository of its own, so the
    /// project's git has no record of what is in it. The path is that
    /// repository's, relative to the directory, and empty when it is the
    /// directory itself.
    ///
    /// A submodule is one: the project records only the commit it points
    /// at, so a clean one shows nothing in `git status`, and `git checkout`
    /// gives back that record, not the submodule's files.
    OwnRepository(PathBuf),
    /// The directory is inside a git repository that is not the project's,
    /// such as one a symbolic link leads into: the path is that repository's
    /// top level. Whatever that repository vouches for, the project's git
    /// cannot give it back.
    OtherRepository(PathBuf),
    /// Git ran and failed for some other reason, or printed something this
    /// module could not read; the text is git's own, or says what was
    /// unreadable.
    Failed(String),
    /// Tracked files git has been told not to look at, so `git status`
    /// calls them clean whatever is on disk.
    Unwatched(Vec<Unwatched>),
    /// Tracked files a `filter` driver other than Git LFS cleans on the way
    /// in, with the driver's name: what git stores can differ from what is
    /// on disk, and `git checkout` gives back what it stored.
    Filtered(Vec<(PathBuf, String)>),
    /// Files git could not give back once they are deleted: untracked,
    /// ignored or changed since the last commit, as far as the asking
    /// function counts them.
    Dirty(Vec<PathBuf>),
}

impl fmt::Display for Obstacle {
    /// Says git's fact in lowercase, with no remedy and no trailing
    /// punctuation, so a caller can put it inside a sentence of its own.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitMissing => formatter.write_str("`git` could not be run"),
            Self::NotARepository => formatter.write_str("the directory is not in a git repository"),
            Self::OwnRepository(repository) if repository.as_os_str().is_empty() => {
                formatter.write_str("the directory is a git repository of its own")
            }
            Self::OwnRepository(repository) => {
                write!(
                    formatter,
                    "{} is a git repository of its own",
                    repository.display()
                )
            }
            Self::OtherRepository(top_level) => write!(
                formatter,
                "the directory is in the git repository at {}, which is not the project's",
                top_level.display()
            ),
            Self::Failed(message) => formatter.write_str(message),
            Self::Unwatched(files) => write!(
                formatter,
                "git has been told not to look at {}",
                counted(files.len(), "tracked file")
            ),
            Self::Filtered(files) => write!(
                formatter,
                "git stores {} through a filter",
                counted(files.len(), "tracked file")
            ),
            Self::Dirty(files) => write!(
                formatter,
                "{} untracked, ignored or changed since the last commit",
                counted_with_verb(files.len())
            ),
        }
    }
}

impl std::error::Error for Obstacle {}

/// `count` of `noun`, pluralised with an `s`: `1 tracked file`, `3 tracked
/// files`.
fn counted(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// `1 file is`, `3 files are`.
fn counted_with_verb(count: usize) -> String {
    if count == 1 {
        format!("{count} file is")
    } else {
        format!("{count} files are")
    }
}

/// `path`, resolved through symbolic links, spelled from `top_level`.
fn from_the_top_level(path: &Path, top_level: &Path) -> Result<PathBuf, Obstacle> {
    path.strip_prefix(top_level)
        .map(Path::to_path_buf)
        .map_err(|_| {
            Obstacle::Failed(format!(
                "git puts {} in the repository at {}, which does not hold it",
                path.display(),
                top_level.display()
            ))
        })
}

/// The top level of the repository `directory` is in, resolved through
/// symbolic links.
fn top_level_of(new_git: &impl Fn() -> Command, directory: &Path) -> Result<PathBuf, Obstacle> {
    let top_level = run_git(new_git, directory, &["rev-parse", "--show-toplevel"])?;
    canonical(Path::new(String::from_utf8_lossy(&top_level).trim_end()))
}

/// Runs `git -C <directory> <arguments>` and returns what it printed to
/// standard output.
fn run_git(
    new_git: &impl Fn() -> Command,
    directory: &Path,
    arguments: &[&str],
) -> Result<Vec<u8>, Obstacle> {
    run_git_with_input(new_git, directory, arguments, &[])
}

/// Runs `git -C <directory> <arguments>` with `input` on its standard input,
/// and returns what it printed to standard output.
///
/// A failure is read as [`failure_of`] reads it.
fn run_git_with_input(
    new_git: &impl Fn() -> Command,
    directory: &Path,
    arguments: &[&str],
    input: &[u8],
) -> Result<Vec<u8>, Obstacle> {
    let output = run_git_for_output(new_git, directory, arguments, input)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(failure_of(&output))
    }
}

/// Runs `git -C <directory> <arguments>` with `input` on its standard input,
/// and returns everything it did, whether or not it succeeded, for a caller
/// that reads an exit status other than zero as an answer.
///
/// Run with `LC_ALL=C`, because the one failure told apart from the rest —
/// "not a git repository" — is told apart by git's own words, and those are
/// translated otherwise. The input is written from a thread of its own, so a
/// command that answers each line as it reads it cannot fill its output pipe
/// while this process is still writing.
fn run_git_for_output(
    new_git: &impl Fn() -> Command,
    directory: &Path,
    arguments: &[&str],
    input: &[u8],
) -> Result<Output, Obstacle> {
    let spawn_failure = |error: std::io::Error| match error.kind() {
        ErrorKind::NotFound => Obstacle::GitMissing,
        _ => Obstacle::Failed(format!("running `git` failed: {error}")),
    };
    let mut child = new_git()
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(spawn_failure)?;

    let stdin = child.stdin.take();
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.map_or(Ok(()), |mut stdin| stdin.write_all(input)));
        let output = child.wait_with_output();
        // A git that exits without reading all of its input closes the pipe,
        // and its own exit status says why; the write's error adds nothing.
        drop(writer.join());
        output
    })
    .map_err(|error| Obstacle::Failed(format!("running `git` failed: {error}")))
}

/// The obstacle a git that exited unsuccessfully stands for: "not a git
/// repository" when it said so, and otherwise whatever it said, in its own
/// words.
fn failure_of(output: &Output) -> Obstacle {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("not a git repository") {
        return Obstacle::NotARepository;
    }
    Obstacle::Failed(stderr.trim_end().to_string())
}

/// Resolves `path` through symbolic links, so two spellings of one
/// directory compare equal.
fn canonical(path: &Path) -> Result<PathBuf, Obstacle> {
    std::fs::canonicalize(path)
        .map_err(|error| Obstacle::Failed(format!("reading {} failed: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Flag, Obstacle, Unwatched};

    /// Every variant renders as git's fact, in lowercase, with no remedy and
    /// no trailing full stop, so a caller can build a sentence around it.
    #[test]
    fn every_obstacle_says_git_s_fact_in_lowercase_with_no_remedy() {
        let unwatched = |flag| Unwatched::new(PathBuf::from("a.txt"), flag);
        let cases = [
            (Obstacle::GitMissing, "`git` could not be run"),
            (
                Obstacle::NotARepository,
                "the directory is not in a git repository",
            ),
            (
                Obstacle::OwnRepository(PathBuf::new()),
                "the directory is a git repository of its own",
            ),
            (
                Obstacle::OwnRepository(PathBuf::from("vendor/upstream")),
                "vendor/upstream is a git repository of its own",
            ),
            (
                Obstacle::OtherRepository(PathBuf::from("/elsewhere")),
                "the directory is in the git repository at /elsewhere, which is not the project's",
            ),
            (
                Obstacle::Failed("fatal: detected dubious ownership".to_string()),
                "fatal: detected dubious ownership",
            ),
            (
                Obstacle::Unwatched(vec![unwatched(Flag::AssumeUnchanged)]),
                "git has been told not to look at 1 tracked file",
            ),
            (
                Obstacle::Unwatched(vec![
                    unwatched(Flag::AssumeUnchanged),
                    unwatched(Flag::SkipWorktree),
                ]),
                "git has been told not to look at 2 tracked files",
            ),
            (
                Obstacle::Filtered(vec![(PathBuf::from("a.cfg"), "strip".to_string())]),
                "git stores 1 tracked file through a filter",
            ),
            (
                Obstacle::Dirty(vec![PathBuf::from("a.txt")]),
                "1 file is untracked, ignored or changed since the last commit",
            ),
            (
                Obstacle::Dirty(vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")]),
                "2 files are untracked, ignored or changed since the last commit",
            ),
        ];
        for (obstacle, expected) in cases {
            assert_eq!(obstacle.to_string(), expected);
            assert!(
                !expected.ends_with('.'),
                "{expected:?} must not end in a full stop"
            );
        }
    }

    fn assert_send_sync_error<T: Send + Sync + std::error::Error>() {}

    #[test]
    fn an_obstacle_is_a_send_and_sync_error() {
        assert_send_sync_error::<Obstacle>();
    }
}
