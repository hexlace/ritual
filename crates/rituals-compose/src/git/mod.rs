//! What git can say about a project's work tree.
//!
//! Whether it can give back every file in a directory that is about to be
//! deleted or moved, whether the work tree is clean, which directories hold
//! Cargo configuration, which entries are submodules, which files mention a
//! pattern, and which files it does not track.
//!
//! Every function runs the `git` on `PATH`, answers from what git itself
//! reports, and is read-only: none of them writes to the project or takes
//! the repository's locks. Each function returns the narrowest of three types
//! that holds every failure it has: [`Unanswered`] when git could not answer
//! at all; [`NotClean`], which adds the files a commit would carry; and
//! [`CannotGiveBack`], which adds everything that stops git giving back a
//! directory. Each states git's facts and no remedy, so a task words its own
//! refusal from the variant it receives and names the command that suits its
//! own run.
//!
//! Paths git reports are spelled from git's top level, the form the `:/`
//! pathspec takes, so a person can hand one to git from any directory of
//! the project. [`top_level_pathspec`] spells one.
//!
//! # Examples
//!
//! Asking whether git can give back everything in a directory before
//! deleting it:
//!
//! ```no_run
//! use std::path::Path;
//!
//! use rituals_compose::git::{self, CannotGiveBack, Unanswered};
//!
//! // Needs a real repository on disk and runs `git`, so this example is
//! // `no_run`.
//! let workspace_root = Path::new(".");
//! match git::ensure_git_can_give_back(Path::new(".rituals/lint"), workspace_root) {
//!     Ok(from_the_top_level) => println!("git can give back {}", from_the_top_level.display()),
//!     Err(CannotGiveBack::Unanswered(Unanswered::NotARepository)) => {
//!         println!("not in a git repository");
//!     }
//!     Err(other) => println!("git cannot vouch for it: {other}"),
//! }
//! ```

use std::fmt;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

mod configuration;
#[cfg(any(test, feature = "test-util"))]
pub mod fixture;
mod give_back;
mod index;
mod mentions;
mod status;
#[cfg(test)]
mod test_support;
mod tracked;

pub use configuration::directories_holding_cargo_configuration;
pub use give_back::{CannotGiveBack, ensure_git_can_give_back};
pub use index::{Flag, Unwatched, submodules_under};
pub use mentions::files_mentioning;
pub use status::{NotClean, ensure_work_tree_is_clean};
pub use tracked::files_git_does_not_track;

// RS-CANONICAL-ERRORS asks for a struct with a private kind and `is_*`
// accessors. These are enums with public variants instead, and no
// `#[non_exhaustive]`, on purpose. Every caller builds its own refusal from
// the variant's data, in its own words, and an exhaustive `match` is what
// makes a new git fact a compile error in each of them, rather than a
// wildcard arm that words it as something else. The crates that match on
// them are released together at one version, so a new variant arrives with
// the callers that word it. There is no backtrace: each value is an answer
// about the person's work tree, reported to them as a refusal, never a fault
// to trace.
/// Why git could not answer a question about the work tree at all.
///
/// The three facts every question here can fail on, and the whole of the
/// error for the questions that fail on nothing else. The variants state
/// git's facts and no remedy: a caller builds its own refusal from the one it
/// receives, in its own words. [`NotClean`] and [`CannotGiveBack`] hold one
/// of these beside the facts of their own questions.
///
/// # Examples
///
/// ```
/// use rituals_compose::git::Unanswered;
///
/// assert_eq!(Unanswered::GitMissing.to_string(), "`git` could not be run");
/// assert_eq!(
///     Unanswered::NotARepository.to_string(),
///     "the directory is not in a git repository"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unanswered {
    /// `git` could not be started because there is no such program.
    GitMissing,
    /// The directory is not inside a git repository.
    NotARepository,
    /// Git ran and failed for some other reason, or printed something this
    /// module could not read; the text is git's own, or says what was
    /// unreadable.
    Failed(String),
}

impl fmt::Display for Unanswered {
    /// Says git's fact in lowercase, with no remedy and no trailing
    /// punctuation, so a caller can put it inside a sentence of its own.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitMissing => formatter.write_str("`git` could not be run"),
            Self::NotARepository => formatter.write_str("the directory is not in a git repository"),
            Self::Failed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for Unanswered {}

/// `path_from_top_level` as the `:/` pathspec, which git reads from the top
/// level whatever directory it is run in.
///
/// # Examples
///
/// Naming a file git reported, so a person can hand it back to git from any
/// directory of the project:
///
/// ```
/// use std::path::Path;
///
/// use rituals_compose::git::top_level_pathspec;
///
/// assert_eq!(
///     top_level_pathspec(Path::new(".rituals/lint/src/lib.rs")),
///     ":/.rituals/lint/src/lib.rs"
/// );
/// ```
#[must_use]
pub fn top_level_pathspec(path_from_top_level: &Path) -> String {
    format!(":/{}", path_from_top_level.display())
}

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
fn from_the_top_level(path: &Path, top_level: &Path) -> Result<PathBuf, Unanswered> {
    path.strip_prefix(top_level)
        .map(Path::to_path_buf)
        .map_err(|_| {
            Unanswered::Failed(format!(
                "git puts {} in the repository at {}, which does not hold it",
                path.display(),
                top_level.display()
            ))
        })
}

/// The top level of the repository `directory` is in, resolved through
/// symbolic links.
fn top_level_of(new_git: &impl Fn() -> Command, directory: &Path) -> Result<PathBuf, Unanswered> {
    let top_level = run_git(new_git, directory, &["rev-parse", "--show-toplevel"])?;
    canonical(Path::new(String::from_utf8_lossy(&top_level).trim_end()))
}

/// Runs `git -C <directory> <arguments>` and returns what it printed to
/// standard output.
fn run_git(
    new_git: &impl Fn() -> Command,
    directory: &Path,
    arguments: &[&str],
) -> Result<Vec<u8>, Unanswered> {
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
) -> Result<Vec<u8>, Unanswered> {
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
) -> Result<Output, Unanswered> {
    let spawn_failure = |error: std::io::Error| match error.kind() {
        ErrorKind::NotFound => Unanswered::GitMissing,
        _ => Unanswered::Failed(format!("running `git` failed: {error}")),
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
    .map_err(|error| Unanswered::Failed(format!("running `git` failed: {error}")))
}

/// The answer a git that exited unsuccessfully stands for: "not a git
/// repository" when it said so, and otherwise whatever it said, in its own
/// words.
fn failure_of(output: &Output) -> Unanswered {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("not a git repository") {
        return Unanswered::NotARepository;
    }
    Unanswered::Failed(stderr.trim_end().to_string())
}

/// Resolves `path` through symbolic links, so two spellings of one
/// directory compare equal.
fn canonical(path: &Path) -> Result<PathBuf, Unanswered> {
    std::fs::canonicalize(path)
        .map_err(|error| Unanswered::Failed(format!("reading {} failed: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{CannotGiveBack, NotClean, Unanswered, top_level_pathspec};

    /// Every variant renders as git's fact, in lowercase, with no remedy and
    /// no trailing full stop, so a caller can build a sentence around it.
    #[test]
    fn every_unanswered_says_git_s_fact_in_lowercase_with_no_remedy() {
        let cases = [
            (Unanswered::GitMissing, "`git` could not be run"),
            (
                Unanswered::NotARepository,
                "the directory is not in a git repository",
            ),
            (
                Unanswered::Failed("fatal: detected dubious ownership".to_string()),
                "fatal: detected dubious ownership",
            ),
        ];
        for (unanswered, expected) in cases {
            assert_eq!(unanswered.to_string(), expected);
            assert!(
                !expected.ends_with('.'),
                "{expected:?} must not end in a full stop"
            );
        }
    }

    #[test]
    fn a_path_from_the_top_level_is_spelled_with_the_slash_colon_magic() {
        assert_eq!(top_level_pathspec(Path::new("a/b.rs")), ":/a/b.rs");
    }

    fn assert_send_sync_error<T: Send + Sync + std::error::Error>() {}

    #[test]
    fn every_git_error_is_a_send_and_sync_error() {
        assert_send_sync_error::<Unanswered>();
        assert_send_sync_error::<NotClean>();
        assert_send_sync_error::<CannotGiveBack>();
    }
}
