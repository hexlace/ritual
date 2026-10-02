//! Whether git can give back every file in a directory `remove` is about to
//! delete.
//
// `redundant_pub_crate` (clippy nursery) wants `pub` here because this
// module is private, but `pub(crate)` is the visibility that is actually
// true — it stays correct if this module is ever re-exported at a different
// level — so the nursery lint gives way.
#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) reflects this module's actual visibility even though the enclosing \
              module is private; see the note above"
)]

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Why a directory cannot be deleted on git's word that it can be given
/// back.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Obstacle {
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
    /// Git ran and failed for some other reason, or printed something this
    /// module could not read; the text is git's own, or says what was
    /// unreadable.
    Failed(String),
    /// Files in the directory git could not give back once it is deleted,
    /// each relative to the directory itself.
    Dirty(Vec<PathBuf>),
}

/// Checks, with the `git` on `PATH`, that every file in `directory` is one
/// git can give back: nothing in it is untracked, ignored or changed since
/// the last commit, and no part of it is a repository of its own.
///
/// Ignored files count against it: a build directory inside the task is
/// files git cannot give back, and a check that let them through would no
/// longer be able to say that git can give back everything it deletes.
pub(crate) fn ensure_git_can_give_back(directory: &Path) -> Result<(), Obstacle> {
    check_with(|| Command::new("git"), directory)
}

/// [`ensure_git_can_give_back`], with `git` started from `new_git` each time
/// it is needed.
///
/// A closure rather than a program name so a test can configure the command
/// itself — its environment, its leading `-c` options — without touching
/// this process's own environment.
fn check_with(new_git: impl Fn() -> Command, directory: &Path) -> Result<(), Obstacle> {
    let top_level = run_git(&new_git, directory, &["rev-parse", "--show-toplevel"])?;
    let top_level = canonical(Path::new(String::from_utf8_lossy(&top_level).trim_end()))?;
    let directory_itself = canonical(directory)?;

    // Git's top level is the directory or lies inside it: what it tracks is
    // that repository's own business, not the project's.
    if top_level.starts_with(&directory_itself) {
        return Err(Obstacle::OwnRepository(PathBuf::new()));
    }

    // A submodule is tracked as a gitlink, mode 160000, and is checked
    // before the status, which says nothing about a clean one and only
    // "modified" about a dirty one. Git names each entry from the directory
    // it was asked in.
    let staged = run_git(
        &new_git,
        directory,
        &["ls-files", "--stage", "-z", "--", "."],
    )?;
    if let Some(submodule) = parse_gitlinks(&staged)?.into_iter().next() {
        return Err(Obstacle::OwnRepository(submodule));
    }

    let status = run_git(
        &new_git,
        directory,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
            "--",
            ".",
        ],
    )?;

    // Git names each path from the top level, whichever directory it was
    // asked from, so each is rejoined to it and then shown from the
    // directory being deleted.
    let dirty: Vec<PathBuf> = parse_porcelain(&status)?
        .into_iter()
        .map(|path| {
            let from_top_level = top_level.join(path);
            from_top_level
                .strip_prefix(&directory_itself)
                .map_or_else(|_| from_top_level.clone(), Path::to_path_buf)
        })
        .collect();

    if dirty.is_empty() {
        Ok(())
    } else {
        Err(Obstacle::Dirty(dirty))
    }
}

/// Runs `git -C <directory> <arguments>` and returns what it printed to
/// standard output.
///
/// Run with `LC_ALL=C`, because the one failure told apart from the rest —
/// "not a git repository" — is told apart by git's own words, and those are
/// translated otherwise.
fn run_git(
    new_git: &impl Fn() -> Command,
    directory: &Path,
    arguments: &[&str],
) -> Result<Vec<u8>, Obstacle> {
    let output = new_git()
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| match error.kind() {
            ErrorKind::NotFound => Obstacle::GitMissing,
            _ => Obstacle::Failed(format!("running `git` failed: {error}")),
        })?;

    if output.status.success() {
        return Ok(output.stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("not a git repository") {
        return Err(Obstacle::NotARepository);
    }
    Err(Obstacle::Failed(stderr.trim_end().to_string()))
}

/// Resolves `path` through symbolic links, so two spellings of one
/// directory compare equal.
fn canonical(path: &Path) -> Result<PathBuf, Obstacle> {
    std::fs::canonicalize(path)
        .map_err(|error| Obstacle::Failed(format!("reading {} failed: {error}", path.display())))
}

/// The path of every gitlink — a submodule — in `git ls-files --stage -z`
/// output.
///
/// An entry is the mode, the object name and the stage, separated by
/// spaces, then a tab and the path, ended by a NUL. An entry with no tab is
/// a failure rather than something to skip, since skipping it could hide a
/// submodule.
fn parse_gitlinks(output: &[u8]) -> Result<Vec<PathBuf>, Obstacle> {
    let text = String::from_utf8_lossy(output);
    let mut gitlinks = Vec::new();
    for entry in text.split('\0').filter(|entry| !entry.is_empty()) {
        let Some((fields, path)) = entry.split_once('\t') else {
            return Err(Obstacle::Failed(format!(
                "git ls-files printed an entry this check cannot read: {entry:?}"
            )));
        };
        if fields.starts_with("160000 ") {
            gitlinks.push(PathBuf::from(path));
        }
    }
    Ok(gitlinks)
}

/// The path of every entry in `git status --porcelain=v1 -z` output.
///
/// An entry is two status letters, a space, then the path, ended by a NUL.
/// A rename or a copy is followed by one more NUL-ended field, the path it
/// came from, which is not an entry of its own. An entry too short to hold
/// its status is a failure rather than something to skip, since skipping it
/// could hide a file git cannot give back.
fn parse_porcelain(output: &[u8]) -> Result<Vec<String>, Obstacle> {
    let text = String::from_utf8_lossy(output);
    let mut fields = text.split('\0');
    let mut paths = Vec::new();
    while let Some(field) = fields.next() {
        if field.is_empty() {
            // The output ends with a NUL, which leaves one empty field.
            continue;
        }
        let Some((status, path)) = field.split_at_checked(3) else {
            return Err(Obstacle::Failed(format!(
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

    use super::{Obstacle, check_with, parse_gitlinks, parse_porcelain};
    use crate::test_support::{ScratchDir, TestOutcome};

    /// `git` configured to read nothing from the machine it runs on: no
    /// global or system configuration, an identity from the environment, and
    /// signing off for the commits these tests make alone.
    ///
    /// Set on the `Command` itself rather than the process's environment, so
    /// tests running side by side neither interfere nor need `unsafe`.
    fn isolated_git() -> Command {
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
    fn git(directory: &Path, arguments: &[&str]) -> TestOutcome {
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

    fn commit_everything(directory: &Path) -> TestOutcome {
        git(directory, &["add", "--all"])?;
        git(directory, &["commit", "--message", "fixture"])
    }

    /// A repository at the root of a scratch directory with `task/`
    /// committed whole.
    fn committed_task(tag: &str) -> Result<(ScratchDir, PathBuf), Box<dyn std::error::Error>> {
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
    fn contained_in(scratch: &Path) -> impl Fn() -> Command {
        let ceiling = scratch.parent().map(Path::to_path_buf).unwrap_or_default();
        move || {
            let mut command = isolated_git();
            command.env("GIT_CEILING_DIRECTORIES", &ceiling);
            command
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
            parse_porcelain(output).map_err(|obstacle| format!("{obstacle:?}"))?,
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
            parse_porcelain(output).map_err(|obstacle| format!("{obstacle:?}"))?,
            ["new.rs", "copy.rs", "other.txt"]
        );
        Ok(())
    }

    #[test]
    fn an_entry_too_short_to_hold_a_status_is_a_failure_not_a_skip() {
        let result = parse_porcelain(b"?? fine.txt\0x\0");
        assert!(
            matches!(&result, Err(Obstacle::Failed(message)) if message.contains("\"x\"")),
            "expected the unreadable entry to be named, got {result:?}"
        );
    }

    #[test]
    fn a_clean_directory_passes() -> TestOutcome {
        let (scratch, task) = committed_task("git-clean")?;
        assert_eq!(check_with(contained_in(scratch.path()), &task), Ok(()));
        Ok(())
    }

    /// One file of each kind git cannot give back — untracked, ignored,
    /// changed since the commit — inside the directory, and one outside it
    /// that must not be named, since it is not deleted.
    #[test]
    fn a_dirty_directory_names_each_file() -> TestOutcome {
        let (scratch, task) = committed_task("git-dirty")?;
        std::fs::write(task.join("untracked.txt"), "new\n")?;
        std::fs::write(task.join("ignored.txt"), "ignored\n")?;
        std::fs::write(task.join("src/lib.rs"), "// changed\n")?;
        std::fs::write(scratch.path().join("outside.txt"), "not deleted\n")?;

        let result = check_with(contained_in(scratch.path()), &task);

        let Err(Obstacle::Dirty(mut files)) = result else {
            return Err(format!("expected the directory to be dirty, got {result:?}").into());
        };
        files.sort();
        assert_eq!(
            files,
            [
                PathBuf::from("ignored.txt"),
                PathBuf::from("src/lib.rs"),
                PathBuf::from("untracked.txt"),
            ]
        );
        Ok(())
    }

    #[test]
    fn a_directory_outside_any_repository_is_refused_as_no_repository() -> TestOutcome {
        let scratch = ScratchDir::new("git-no-repository")?;
        let task = scratch.path().join("task");
        std::fs::create_dir_all(&task)?;

        assert_eq!(
            check_with(contained_in(scratch.path()), &task),
            Err(Obstacle::NotARepository)
        );
        Ok(())
    }

    /// Spawning a program name that does not exist fails with `NotFound`,
    /// the same error a machine without `git` gives.
    #[test]
    fn a_missing_git_binary_is_refused() -> TestOutcome {
        let scratch = ScratchDir::new("git-missing")?;
        assert_eq!(
            check_with(|| Command::new("ritual-test-no-such-git"), scratch.path()),
            Err(Obstacle::GitMissing)
        );
        Ok(())
    }

    #[test]
    fn a_directory_that_is_its_own_repository_is_refused() -> TestOutcome {
        let (scratch, task) = committed_task("git-own-repository")?;
        git(&task, &["init"])?;

        assert_eq!(
            check_with(contained_in(scratch.path()), &task),
            Err(Obstacle::OwnRepository(PathBuf::new()))
        );
        Ok(())
    }

    /// A clean submodule leaves nothing in the parent's `git status`, so
    /// only the gitlink in the index shows it. It is made from a local
    /// repository, which git refuses to clone as a submodule unless the
    /// file transport is allowed for that one command.
    #[test]
    fn a_directory_holding_a_clean_submodule_is_refused_naming_it() -> TestOutcome {
        let (scratch, task) = committed_task("git-submodule")?;
        let upstream = scratch.path().join("upstream");
        std::fs::create_dir_all(&upstream)?;
        std::fs::write(upstream.join("vendored.txt"), "a file of its own\n")?;
        git(&upstream, &["init"])?;
        commit_everything(&upstream)?;
        let upstream = upstream
            .to_str()
            .ok_or("a scratch path that is not UTF-8")?;
        git(
            &task,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                upstream,
                "vendor/upstream",
            ],
        )?;
        git(scratch.path(), &["commit", "--message", "add a submodule"])?;

        assert_eq!(
            check_with(contained_in(scratch.path()), &task),
            Err(Obstacle::OwnRepository(PathBuf::from("vendor/upstream")))
        );
        Ok(())
    }

    #[test]
    fn only_gitlinks_are_read_as_submodules() -> TestOutcome {
        let output = b"100644 0123456789abcdef0123456789abcdef01234567 0\tsrc/lib.rs\0\
            160000 89abcdef0123456789abcdef0123456789abcdef 0\tvendor/upstream\0\
            100755 0123456789abcdef0123456789abcdef01234567 0\trun.sh\0";
        assert_eq!(
            parse_gitlinks(output).map_err(|obstacle| format!("{obstacle:?}"))?,
            [PathBuf::from("vendor/upstream")]
        );
        Ok(())
    }

    #[test]
    fn an_index_entry_with_no_path_is_a_failure_not_a_skip() {
        let result = parse_gitlinks(b"160000 89abcdef 0\0");
        assert!(
            matches!(&result, Err(Obstacle::Failed(message)) if message.contains("160000")),
            "expected the unreadable entry to be named, got {result:?}"
        );
    }

    /// A failure other than "not a repository" is passed on in git's words:
    /// asking about a directory that does not exist makes git fail without
    /// saying that.
    #[test]
    fn any_other_git_failure_passes_gits_own_words_through() -> TestOutcome {
        let scratch = ScratchDir::new("git-other-failure")?;
        let result = check_with(
            contained_in(scratch.path()),
            &scratch.path().join("no-such-directory"),
        );
        assert!(
            matches!(
                &result,
                Err(Obstacle::Failed(message)) if message.contains("no-such-directory")
            ),
            "expected git's own message naming the directory, got {result:?}"
        );
        Ok(())
    }
}
