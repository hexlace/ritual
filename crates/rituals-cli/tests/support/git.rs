//! A throwaway git repository around a fixture project, isolated from the
//! machine it runs on.
//!
//! A story that needs git as the way back — `remove` refuses to delete files
//! git cannot give back — makes a repository of its own inside its temporary
//! directory. Nothing here reads the developer's git configuration or
//! ignore and attribute files: no global or system file is consulted, the
//! identity comes from the environment, and signing is off for these commits
//! alone, so the suite neither prompts for a key nor depends on how the
//! machine is set up.

use std::path::Path;
use std::process::{Command, Stdio};

use super::{Outcome, ResultContext, RunOutput, TestOutcome};

// HC-ONE-WAY: `rituals-compose`'s `test-util` feature exports the same
// isolated git (`rituals_compose::git::fixture::isolated_git`), and this
// suite keeps its own on purpose. These stories judge ritual's shipped binary
// from outside, the way a person would, and they import none of ritual's
// library crates. An instrument taken from a crate under test changes whenever
// that crate does, so a story could pass because its own fixture moved. This
// module is the one place in the suite that says how git is isolated.

/// Points `command` away from the machine it runs on: no global or system git
/// configuration, no global ignore or attributes file, an identity from the
/// environment, and no inherited repository location.
///
/// `GIT_CONFIG_GLOBAL` replaces the user's configuration files but not the
/// global ignore and attributes files, which git finds under
/// `$XDG_CONFIG_HOME/git`, or `$HOME/.config/git` when that is unset.
/// Pointing `XDG_CONFIG_HOME` at `/dev/null`, which holds no `git`
/// directory, leaves git nothing to read there and no other place to look.
///
/// Set on the command itself, so stories running side by side neither
/// interfere with each other nor change the process's environment.
pub(crate) fn isolate_from_the_machine(command: &mut Command) -> &mut Command {
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("XDG_CONFIG_HOME", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
}

/// Runs `git <arguments…>` in `directory` with the machine's git
/// configuration, ignore and attributes files, and any inherited repository
/// location out of the way, and
/// signing off for the commits a story makes alone.
pub(crate) fn git(directory: &Path, arguments: &[&str]) -> Outcome<RunOutput> {
    let output = isolate_from_the_machine(
        Command::new("git")
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(arguments)
            .current_dir(directory)
            .stdin(Stdio::null()),
    )
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
