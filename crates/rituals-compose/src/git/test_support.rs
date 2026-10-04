//! The helpers every test module under [`crate::git`] shares: a `git` that
//! reads nothing from the machine it runs on, and the repositories the tests
//! ask it about.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::test_support::{ScratchDir, TestOutcome};

/// `git` configured to read nothing from the machine it runs on: no
/// global or system configuration, an identity from the environment, and
/// signing off for the commits these tests make alone.
///
/// Set on the `Command` itself rather than the process's environment, so
/// tests running side by side neither interfere nor need `unsafe`.
pub(super) fn isolated_git() -> Command {
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

/// Runs `git <arguments>` in `directory` and fails the test if git does.
pub(super) fn git(directory: &Path, arguments: &[&str]) -> TestOutcome {
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

pub(super) fn commit_everything(directory: &Path) -> TestOutcome {
    git(directory, &["add", "--all"])?;
    git(directory, &["commit", "--message", "fixture"])
}

/// A repository at the root of a scratch directory with `task/`
/// committed whole.
pub(super) fn committed_task(
    tag: &str,
) -> Result<(ScratchDir, PathBuf), Box<dyn std::error::Error>> {
    let scratch = ScratchDir::new(tag)?;
    let task = scratch.path().join("task");
    std::fs::create_dir_all(task.join("src"))?;
    std::fs::write(task.join("src/lib.rs"), "// committed\n")?;
    std::fs::write(task.join(".gitignore"), "ignored.txt\n")?;
    git(scratch.path(), &["init"])?;
    commit_everything(scratch.path())?;
    Ok((scratch, task))
}

/// `isolated_git` that never looks for a repository above `scratch`'s own
/// directory, so the machine's repositories are never found.
///
/// Git does not search the directory named as the ceiling itself, only
/// those below it, so the ceiling is `scratch`'s parent.
pub(super) fn contained_in(scratch: &Path) -> impl Fn() -> Command {
    let ceiling = scratch.parent().map(Path::to_path_buf).unwrap_or_default();
    move || {
        let mut command = isolated_git();
        command.env("GIT_CEILING_DIRECTORIES", &ceiling);
        command
    }
}

/// Adds a repository of its own, made beside `project` and holding one
/// committed file, as a submodule of `project` at `at`, and commits it.
///
/// It is cloned from a local path, which git refuses as a submodule unless
/// the file transport is allowed for that one command.
pub(super) fn add_a_submodule(project: &Path, at: &str) -> TestOutcome {
    let upstream = project.join("upstream");
    std::fs::create_dir_all(&upstream)?;
    std::fs::write(upstream.join("vendored.txt"), "a file of its own\n")?;
    git(&upstream, &["init"])?;
    commit_everything(&upstream)?;
    let upstream = upstream
        .to_str()
        .ok_or("a scratch path that is not UTF-8")?;
    git(
        project,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            upstream,
            at,
        ],
    )?;
    git(project, &["commit", "--message", "add a submodule"])
}
