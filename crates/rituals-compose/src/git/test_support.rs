//! The helpers every test module under [`crate::git`] shares: the
//! repositories the tests ask `git` about, built with [`super::fixture`].

use std::path::{Path, PathBuf};
use std::process::Command;

use super::fixture::{commit_everything, git, isolated_git};
use crate::test_support::{ScratchDir, TestOutcome};

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
    git(project, &["commit", "--message", "add a submodule"])?;
    Ok(())
}
