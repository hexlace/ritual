//! Test fixtures for a crate that tests against `git`: a `git` that reads
//! nothing from the machine it runs on, and the two commands every fixture
//! repository is built with.
//!
//! Only built with the `test-util` feature, which a crate that tests against
//! `git` turns on for its own tests and a build never needs. Running `git`
//! on the machine's own configuration makes a test depend on whose machine
//! it runs on: a signing key, a different default branch or a global ignore
//! file changes what a fixture repository holds. So every command here sets
//! its own, on the [`Command`] itself rather than in the process's
//! environment, which lets tests running side by side neither interfere nor
//! need `unsafe`.

use std::io;
use std::path::Path;
use std::process::Command;

/// A `git` command that reads nothing from the machine it runs on.
///
/// There is no global or system configuration, an identity comes from the
/// environment, signing is off for the commits a test makes alone, and `main`
/// is the default branch.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::git::fixture::isolated_git;
///
/// // Runs `git`, so this example is `no_run`.
/// let output = isolated_git()
///     .arg("-C")
///     .arg(Path::new("."))
///     .args(["status", "--porcelain"])
///     .output()?;
/// assert!(output.status.success());
/// # Ok::<(), std::io::Error>(())
/// ```
#[must_use]
pub fn isolated_git() -> Command {
    let mut command = Command::new("git");
    command
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    command
}

/// Runs [`isolated_git`] with `arguments` in `directory`.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::git::fixture::git;
///
/// // Runs `git`, so this example is `no_run`.
/// git(Path::new("."), &["init", "--quiet"])?;
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// # Errors
///
/// Returns the error when `git` cannot be started.
///
/// # Panics
///
/// Panics when git exits non-zero, with its standard error: a fixture
/// repository that could not be built means the test has nothing to test.
pub fn git(directory: &Path, arguments: &[&str]) -> io::Result<()> {
    let output = isolated_git()
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()?;
    assert!(
        output.status.success(),
        "`git {}` failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

/// Stages everything in `directory`'s work tree, ignored files aside, and
/// commits it.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::git::fixture::{commit_everything, git};
///
/// // Runs `git`, so this example is `no_run`.
/// let directory = Path::new(".");
/// git(directory, &["init", "--quiet"])?;
/// commit_everything(directory)?;
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// # Errors
///
/// Returns the error when `git` cannot be started.
///
/// # Panics
///
/// Panics when git exits non-zero, with its standard error, as [`git`] does.
pub fn commit_everything(directory: &Path) -> io::Result<()> {
    git(directory, &["add", "--all"])?;
    git(directory, &["commit", "--message", "fixture"])
}
