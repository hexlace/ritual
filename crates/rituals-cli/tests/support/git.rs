//! A throwaway git repository around a fixture project, isolated from the
//! machine it runs on.
//!
//! A story that needs git as the way back — `remove` refuses to delete files
//! git cannot give back — makes a repository of its own inside its temporary
//! directory. Nothing here reads the developer's git configuration: no
//! global or system file is consulted, the identity comes from the
//! environment, and signing is off for these commits alone, so the suite
//! neither prompts for a key nor depends on how the machine is set up.

use std::path::Path;
use std::process::{Command, Stdio};

use super::{Outcome, ResultContext, RunOutput, TestOutcome};

/// Runs `git <arguments…>` in `directory` with the machine's git
/// configuration and any inherited repository location out of the way.
pub(crate) fn git(directory: &Path, arguments: &[&str]) -> Outcome<RunOutput> {
    let output = Command::new("git")
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .context(&format!("spawning `git {}` failed", arguments.join(" ")))?;
    Ok(RunOutput {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Makes `directory` a git repository and commits everything in it.
pub(crate) fn init_and_commit_everything(directory: &Path) -> TestOutcome {
    git(directory, &["init"])?.expect_success("`git init` in a fixture project");
    commit_everything(directory)
}

/// Stages and commits every change under `directory`, so the working tree is
/// clean afterwards.
pub(crate) fn commit_everything(directory: &Path) -> TestOutcome {
    git(directory, &["add", "--all"])?.expect_success("`git add --all` in a fixture project");
    git(directory, &["commit", "--message", "fixture"])?
        .expect_success("`git commit` in a fixture project");
    Ok(())
}
