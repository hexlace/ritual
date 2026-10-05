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
/// There is no global or system configuration and no global ignore or
/// attributes file, an identity comes from the environment, signing is off
/// for the commits a test makes alone, and `main` is the default branch.
///
/// `GIT_CONFIG_GLOBAL` replaces the user's configuration files but not the
/// global ignore and attributes files, which git finds under
/// `$XDG_CONFIG_HOME/git`, or `$HOME/.config/git` when that is unset.
/// `XDG_CONFIG_HOME` is `/dev/null`, which holds no `git` directory, so git
/// has nothing to read there and no other place to look.
///
/// Configuration a parent hands git in the environment is cleared as well:
/// `GIT_CONFIG_COUNT` says how many `GIT_CONFIG_KEY_<n>` and
/// `GIT_CONFIG_VALUE_<n>` pairs to read, and none are read without it, and
/// `GIT_CONFIG_PARAMETERS` is removed too.
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
        .env("XDG_CONFIG_HOME", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{ScratchDir, TestOutcome};

    /// A developer's global ignore file sits at `$HOME/.config/git/ignore`
    /// on a machine that never set `XDG_CONFIG_HOME`, and git reads it even
    /// when `GIT_CONFIG_GLOBAL` is `/dev/null`. The test stands a home
    /// directory holding such a file in for the developer's, points `HOME`
    /// at it on the isolated command, and asks git whether it ignores a
    /// file only that file's rule names.
    #[test]
    fn isolated_git_does_not_read_a_global_ignore_file_under_the_home_directory() -> TestOutcome {
        let home = ScratchDir::new("fixture-home")?;
        std::fs::create_dir_all(home.path().join(".config/git"))?;
        std::fs::write(home.path().join(".config/git/ignore"), "machine-only.txt\n")?;
        let repository = ScratchDir::new("fixture-repository")?;
        git(repository.path(), &["init", "--quiet"])?;
        std::fs::write(repository.path().join("machine-only.txt"), "x\n")?;

        let output = isolated_git()
            .env("HOME", home.path())
            .arg("-C")
            .arg(repository.path())
            .args([
                "check-ignore",
                "--verbose",
                "--no-index",
                "machine-only.txt",
            ])
            .output()?;

        assert!(
            output.stdout.is_empty(),
            "an isolated git must not read $HOME/.config/git/ignore; it said:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            output.stderr.is_empty(),
            "isolating git must not make it warn; it said:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    /// The environment variable that tells this test binary, started again
    /// by the test below, to run as the git-asking child and which git to
    /// ask: `control` for a plain `git`, `isolated` for [`isolated_git`].
    const CHILD_MODE: &str = "FIXTURE_INJECTED_CONFIGURATION_CHILD";
    /// The repository the child asks, as a path.
    const CHILD_REPOSITORY: &str = "FIXTURE_INJECTED_CONFIGURATION_REPOSITORY";
    /// The file the child writes `ignored=true` or `ignored=false` to, which
    /// the parent reads; a child that never ran leaves no file to read.
    const CHILD_ANSWER: &str = "FIXTURE_INJECTED_CONFIGURATION_ANSWER";
    const CHILD_TEST: &str =
        "isolated_git_does_not_read_configuration_injected_through_the_environment";

    /// Git also reads configuration a parent process hands it in the
    /// environment, through `GIT_CONFIG_COUNT` with its key and value pairs
    /// or through `GIT_CONFIG_PARAMETERS`. An isolated command has to ignore
    /// what it inherits, and this crate forbids `unsafe`, so the test cannot
    /// change its own process's environment: it starts this same test binary
    /// again, as a child whose environment carries an injected
    /// `core.excludesFile`, and the child asks git whether it ignores a file
    /// only that file's rule names. A plain `git` is the positive control;
    /// `isolated_git` must not.
    #[test]
    fn isolated_git_does_not_read_configuration_injected_through_the_environment() -> TestOutcome {
        if let Ok(mode) = std::env::var(CHILD_MODE) {
            return ask_whether_git_ignores_the_file(&mode);
        }
        let home = ScratchDir::new("fixture-injected-home")?;
        let ignore_file = home.path().join("ignore");
        std::fs::write(&ignore_file, "machine-only.txt\n")?;
        let repository = ScratchDir::new("fixture-injected-repository")?;
        git(repository.path(), &["init", "--quiet"])?;
        std::fs::write(repository.path().join("machine-only.txt"), "x\n")?;

        let answers = ScratchDir::new("fixture-injected-answers")?;
        let by_count = |command: &mut Command| {
            command
                .env("GIT_CONFIG_COUNT", "1")
                .env("GIT_CONFIG_KEY_0", "core.excludesFile")
                .env("GIT_CONFIG_VALUE_0", &ignore_file);
        };
        let by_parameters = |command: &mut Command| {
            command.env(
                "GIT_CONFIG_PARAMETERS",
                format!("'core.excludesFile'='{}'", ignore_file.display()),
            );
        };
        for (way, inject) in [
            ("GIT_CONFIG_COUNT", &by_count as &dyn Fn(&mut Command)),
            ("GIT_CONFIG_PARAMETERS", &by_parameters),
        ] {
            let control = answer_of(&answers, repository.path(), "control", inject)?;
            assert_eq!(
                control, "ignored=true",
                "expected an unisolated git to read an excludesFile injected through {way}"
            );

            let isolated = answer_of(&answers, repository.path(), "isolated", inject)?;
            assert_eq!(
                isolated, "ignored=false",
                "an isolated git must not read configuration injected through {way}"
            );
        }
        Ok(())
    }

    /// Starts this test binary to run only the test above in `mode`, with
    /// `inject` setting the configuration the child inherits, and returns
    /// what the child wrote as its answer.
    fn answer_of(
        answers: &ScratchDir,
        repository: &Path,
        mode: &str,
        inject: &dyn Fn(&mut Command),
    ) -> Result<String, Box<dyn std::error::Error>> {
        let answer = answers.path().join(mode);
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args([CHILD_TEST, "--test-threads=1"])
            .env(CHILD_MODE, mode)
            .env(CHILD_REPOSITORY, repository)
            .env(CHILD_ANSWER, &answer);
        inject(&mut command);
        let output = command.output()?;
        assert!(
            output.status.success(),
            "the child test run failed:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        Ok(std::fs::read_to_string(&answer)?)
    }

    /// The child's half: runs the git `mode` names against the repository
    /// and writes `ignored=true` or `ignored=false` for the parent to read.
    fn ask_whether_git_ignores_the_file(mode: &str) -> TestOutcome {
        let repository = std::env::var(CHILD_REPOSITORY)
            .map_err(|error| io::Error::other(format!("{CHILD_REPOSITORY}: {error}")))?;
        let mut command = match mode {
            "control" => Command::new("git"),
            "isolated" => isolated_git(),
            other => return Err(io::Error::other(format!("unknown child mode {other}")).into()),
        };
        let output = command
            .arg("-C")
            .arg(repository)
            .args(["check-ignore", "--no-index", "machine-only.txt"])
            .output()?;
        let answer = std::env::var(CHILD_ANSWER)
            .map_err(|error| io::Error::other(format!("{CHILD_ANSWER}: {error}")))?;
        std::fs::write(answer, format!("ignored={}", !output.stdout.is_empty()))?;
        Ok(())
    }
}
