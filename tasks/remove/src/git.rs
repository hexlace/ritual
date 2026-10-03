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

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The one `filter` driver whose files git gives back byte for byte: Git
/// LFS stores the file itself and puts it back on checkout.
const GIVES_BACK_ITS_BYTES: &str = "lfs";

/// Why a directory cannot be deleted on git's word that it can be given
/// back.
///
/// Every path a variant holds is spelled from git's top level, the form the
/// `:/` pathspec takes, so a person can hand it to git from any directory of
/// the project — except [`Obstacle::OwnRepository`]'s, which is the
/// directory's own.
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
    /// Files in the directory git could not give back once it is deleted:
    /// untracked, ignored, or changed since the last commit.
    Dirty(Vec<PathBuf>),
}

/// A tracked file git has been told not to look at, and which flag says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unwatched {
    pub(crate) path: PathBuf,
    pub(crate) flag: Flag,
}

/// The index flags that make `git status` stop looking at a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flag {
    /// `git update-index --assume-unchanged`: an edit is never seen, and
    /// `git checkout` gives back the committed file without it.
    AssumeUnchanged,
    /// `git update-index --skip-worktree`: an edit is never seen, and `git
    /// checkout` does not give the file back at all.
    SkipWorktree,
}

/// Checks, with the `git` on `PATH`, that every file in `directory` is one
/// git can give back: the directory is in the same repository as
/// `workspace_root`, no part of it is a repository of its own, git is
/// watching every tracked file in it and stores each one as it is on disk,
/// and nothing in it is untracked, ignored or changed since the last commit.
///
/// Returns the directory spelled from git's top level, the path a person
/// gives `git checkout` to get it back.
///
/// When `directory` is itself a symbolic link, deleting it removes only the
/// link, so the link is what git is asked about: it has to be tracked and
/// unchanged, and the path returned is the link's, not its target's.
///
/// Ignored files count against it: a build directory inside the task is
/// files git cannot give back, and a check that let them through would no
/// longer be able to say that git can give back everything it deletes.
pub(crate) fn ensure_git_can_give_back(
    directory: &Path,
    workspace_root: &Path,
) -> Result<PathBuf, Obstacle> {
    check_with(|| Command::new("git"), directory, workspace_root)
}

/// [`ensure_git_can_give_back`], with `git` started from `new_git` each time
/// it is needed.
///
/// A closure rather than a program name so a test can configure the command
/// itself — its environment, its leading `-c` options — without touching
/// this process's own environment.
fn check_with(
    new_git: impl Fn() -> Command,
    directory: &Path,
    workspace_root: &Path,
) -> Result<PathBuf, Obstacle> {
    let is_a_link = directory
        .symlink_metadata()
        .map_err(|error| {
            Obstacle::Failed(format!("reading {} failed: {error}", directory.display()))
        })?
        .file_type()
        .is_symlink();
    if is_a_link {
        return check_link_with(&new_git, directory, workspace_root);
    }

    let top_level = top_level_of(&new_git, directory)?;
    let directory_itself = canonical(directory)?;

    // Git's top level is the directory or lies inside it: what it tracks is
    // that repository's own business, not the project's.
    if top_level.starts_with(&directory_itself) {
        return Err(Obstacle::OwnRepository(PathBuf::new()));
    }
    ensure_the_projects(&new_git, &top_level, workspace_root)?;
    let from_top_level = from_the_top_level(&directory_itself, &top_level)?;

    // One read of the index answers three questions: whether any entry is a
    // submodule, which is tracked as a gitlink, mode 160000, and which
    // `git status` says nothing about while it is clean; which entries carry
    // a flag that stops `git status` looking at them, which `-v` shows; and
    // which files a filter attribute could apply to. Git names each entry
    // from the directory it was asked in.
    let index = parse_index(&run_git(
        &new_git,
        directory,
        &["ls-files", "--stage", "-v", "-z", "--", "."],
    )?)?;
    if let Some(submodule) = index.iter().find(|entry| entry.is_gitlink) {
        return Err(Obstacle::OwnRepository(submodule.path.clone()));
    }

    let unwatched: Vec<Unwatched> = index
        .iter()
        .filter_map(|entry| {
            let flag = entry.flag?;
            // A sparse checkout marks every file outside it skip-worktree
            // and leaves it off disk; with nothing on disk there is nothing
            // to lose.
            let on_disk = directory.join(&entry.path).symlink_metadata().is_ok();
            (flag == Flag::AssumeUnchanged || on_disk).then(|| Unwatched {
                path: from_top_level.join(&entry.path),
                flag,
            })
        })
        .collect();
    if !unwatched.is_empty() {
        return Err(Obstacle::Unwatched(unwatched));
    }

    let mut paths = Vec::new();
    for entry in &index {
        paths.extend_from_slice(entry.path.as_os_str().as_encoded_bytes());
        paths.push(0);
    }
    let filters = parse_attributes(&run_git_with_input(
        &new_git,
        directory,
        &["check-attr", "-z", "--stdin", "filter"],
        &paths,
    )?)?;
    let filtered: Vec<(PathBuf, String)> = filters
        .into_iter()
        .filter(|(_path, value)| {
            !matches!(
                value.as_str(),
                "unspecified" | "unset" | GIVES_BACK_ITS_BYTES
            )
        })
        .map(|(path, value)| (from_top_level.join(path), value))
        .collect();
    if !filtered.is_empty() {
        return Err(Obstacle::Filtered(filtered));
    }

    ensure_nothing_dirty(&new_git, directory, ".")?;
    Ok(from_top_level)
}

/// [`check_with`] for a `link`, a symbolic link standing where the directory
/// is: deleting it removes the link and leaves what it points at, so git is
/// asked about the link itself, from the directory holding it, and never
/// about the files it leads to. Git keeps a link as one entry, and gives it
/// back from that when it is tracked and unchanged.
///
/// A link runs no `filter` driver, so only the index and the status are
/// read.
fn check_link_with(
    new_git: &impl Fn() -> Command,
    link: &Path,
    workspace_root: &Path,
) -> Result<PathBuf, Obstacle> {
    let (Some(holder), Some(name)) = (link.parent(), link.file_name()) else {
        return Err(Obstacle::Failed(format!(
            "{} has no directory holding it to ask git from",
            link.display()
        )));
    };
    let top_level = top_level_of(new_git, holder)?;
    ensure_the_projects(new_git, &top_level, workspace_root)?;
    let from_top_level = from_the_top_level(&canonical(holder)?, &top_level)?.join(name);
    // Literal, so a link whose name holds `*` or `?` is not read as a
    // pattern matching others.
    let pathspec = format!(":(literal){}", name.to_string_lossy());

    let index = parse_index(&run_git(
        new_git,
        holder,
        &["ls-files", "--stage", "-v", "-z", "--", &pathspec],
    )?)?;
    let unwatched: Vec<Unwatched> = index
        .iter()
        .filter_map(|entry| {
            entry.flag.map(|flag| Unwatched {
                path: from_top_level.clone(),
                flag,
            })
        })
        .collect();
    if !unwatched.is_empty() {
        return Err(Obstacle::Unwatched(unwatched));
    }

    ensure_nothing_dirty(new_git, holder, &pathspec)?;
    Ok(from_top_level)
}

/// Refuses when `top_level` is not the repository `workspace_root` is in:
/// whatever that other repository vouches for, the project's git cannot give
/// it back.
fn ensure_the_projects(
    new_git: &impl Fn() -> Command,
    top_level: &Path,
    workspace_root: &Path,
) -> Result<(), Obstacle> {
    match top_level_of(new_git, workspace_root) {
        Ok(projects) if projects == top_level => Ok(()),
        Ok(_) | Err(Obstacle::NotARepository) => {
            Err(Obstacle::OtherRepository(top_level.to_path_buf()))
        }
        Err(obstacle) => Err(obstacle),
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

/// Refuses when anything `pathspec` matches, asked from `asked_in`, is
/// untracked, ignored, or changed since the last commit, naming each.
fn ensure_nothing_dirty(
    new_git: &impl Fn() -> Command,
    asked_in: &Path,
    pathspec: &str,
) -> Result<(), Obstacle> {
    let status = run_git(
        new_git,
        asked_in,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
            "--",
            pathspec,
        ],
    )?;
    // Git names each path in the status from the top level, whichever
    // directory it was asked from.
    let dirty: Vec<PathBuf> = parse_porcelain(&status)?
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if dirty.is_empty() {
        Ok(())
    } else {
        Err(Obstacle::Dirty(dirty))
    }
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
/// Run with `LC_ALL=C`, because the one failure told apart from the rest —
/// "not a git repository" — is told apart by git's own words, and those are
/// translated otherwise. The input is written from a thread of its own, so a
/// command that answers each line as it reads it cannot fill its output pipe
/// while this process is still writing.
fn run_git_with_input(
    new_git: &impl Fn() -> Command,
    directory: &Path,
    arguments: &[&str],
    input: &[u8],
) -> Result<Vec<u8>, Obstacle> {
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
    let output = std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.map_or(Ok(()), |mut stdin| stdin.write_all(input)));
        let output = child.wait_with_output();
        // A git that exits without reading all of its input closes the pipe,
        // and its own exit status says why; the write's error adds nothing.
        drop(writer.join());
        output
    })
    .map_err(|error| Obstacle::Failed(format!("running `git` failed: {error}")))?;

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

/// One entry of `git ls-files --stage -v -z`, as far as this check reads
/// it.
#[derive(Debug, PartialEq, Eq)]
struct IndexEntry {
    path: PathBuf,
    is_gitlink: bool,
    flag: Option<Flag>,
}

/// Every entry in `git ls-files --stage -v -z` output.
///
/// An entry is a one-letter tag and a space, then the mode, the object name
/// and the stage, separated by spaces, then a tab and the path, ended by a
/// NUL. The tag is lowercase for an entry marked assume-unchanged and `S`
/// for one marked skip-worktree. An entry this cannot read is a failure
/// rather than something to skip, since skipping it could hide a submodule
/// or a flag.
fn parse_index(output: &[u8]) -> Result<Vec<IndexEntry>, Obstacle> {
    let text = String::from_utf8_lossy(output);
    let mut entries = Vec::new();
    for entry in text.split('\0').filter(|entry| !entry.is_empty()) {
        let unreadable = || {
            Obstacle::Failed(format!(
                "git ls-files printed an entry this check cannot read: {entry:?}"
            ))
        };
        let (fields, path) = entry.split_once('\t').ok_or_else(unreadable)?;
        let (tag, rest) = fields.split_once(' ').ok_or_else(unreadable)?;
        let mut tag_letters = tag.chars();
        let (Some(letter), None) = (tag_letters.next(), tag_letters.next()) else {
            return Err(unreadable());
        };
        let flag = if letter.is_ascii_lowercase() {
            Some(Flag::AssumeUnchanged)
        } else if letter == 'S' {
            Some(Flag::SkipWorktree)
        } else {
            None
        };
        entries.push(IndexEntry {
            path: PathBuf::from(path),
            is_gitlink: rest.starts_with("160000 "),
            flag,
        });
    }
    Ok(entries)
}

/// Every path and its `filter` value in `git check-attr -z filter` output.
///
/// Each answer is three NUL-ended fields: the path, the attribute's name,
/// and its value — `unspecified`, `unset`, `set`, or the value it was given.
/// Output that does not come in threes is a failure, since misreading it
/// could pair a file with the wrong filter.
fn parse_attributes(output: &[u8]) -> Result<Vec<(PathBuf, String)>, Obstacle> {
    let text = String::from_utf8_lossy(output);
    let fields: Vec<&str> = text.split_terminator('\0').collect();
    let answers = fields.chunks_exact(3);
    if !answers.remainder().is_empty() {
        return Err(Obstacle::Failed(format!(
            "git check-attr printed output this check cannot read: {text:?}"
        )));
    }
    Ok(answers
        .map(|answer| (PathBuf::from(answer[0]), answer[2].to_string()))
        .collect())
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

    use super::{
        Flag, IndexEntry, Obstacle, Unwatched, check_with, parse_attributes, parse_index,
        parse_porcelain,
    };
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

    /// [`check_with`] on `task`, with the scratch directory as the project's
    /// workspace root.
    fn check(scratch: &Path, task: &Path) -> Result<PathBuf, Obstacle> {
        check_with(contained_in(scratch), task, scratch)
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
    fn a_clean_directory_passes_and_is_spelled_from_the_top_level() -> TestOutcome {
        let (scratch, task) = committed_task("git-clean")?;
        assert_eq!(check(scratch.path(), &task), Ok(PathBuf::from("task")));
        Ok(())
    }

    /// One file of each kind git cannot give back — untracked, ignored,
    /// changed since the commit — inside the directory, and one outside it
    /// that must not be named, since it is not deleted.
    #[test]
    fn a_dirty_directory_names_each_file_from_the_top_level() -> TestOutcome {
        let (scratch, task) = committed_task("git-dirty")?;
        std::fs::write(task.join("untracked.txt"), "new\n")?;
        std::fs::write(task.join("ignored.txt"), "ignored\n")?;
        std::fs::write(task.join("src/lib.rs"), "// changed\n")?;
        std::fs::write(scratch.path().join("outside.txt"), "not deleted\n")?;

        let result = check(scratch.path(), &task);

        let Err(Obstacle::Dirty(mut files)) = result else {
            return Err(format!("expected the directory to be dirty, got {result:?}").into());
        };
        files.sort();
        assert_eq!(
            files,
            [
                PathBuf::from("task/ignored.txt"),
                PathBuf::from("task/src/lib.rs"),
                PathBuf::from("task/untracked.txt"),
            ]
        );
        Ok(())
    }

    /// A repository holding `vendor/task`, committed, and `tasks/task`, an
    /// uncommitted symbolic link to it.
    fn a_link_to_a_committed_task(
        tag: &str,
    ) -> Result<(ScratchDir, PathBuf), Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new(tag)?;
        std::fs::create_dir_all(scratch.path().join("vendor/task"))?;
        std::fs::create_dir_all(scratch.path().join("tasks"))?;
        std::fs::write(scratch.path().join("vendor/task/lib.rs"), "// committed\n")?;
        git(scratch.path(), &["init"])?;
        commit_everything(scratch.path())?;
        let link = scratch.path().join("tasks/task");
        std::os::unix::fs::symlink("../vendor/task", &link)?;
        Ok((scratch, link))
    }

    /// Deleting a link removes only the link, so it is the link git has to
    /// give back: an untracked or ignored one is refused, named by its own
    /// path, whatever state its target is in.
    #[test]
    fn an_untracked_or_ignored_link_is_refused_naming_the_link() -> TestOutcome {
        let (scratch, link) = a_link_to_a_committed_task("git-link-untracked")?;
        assert_eq!(
            check(scratch.path(), &link),
            Err(Obstacle::Dirty(vec![PathBuf::from("tasks/task")]))
        );

        std::fs::write(scratch.path().join("tasks/.gitignore"), "task\n")?;
        git(scratch.path(), &["add", "tasks/.gitignore"])?;
        git(scratch.path(), &["commit", "--message", "ignore the link"])?;
        assert_eq!(
            check(scratch.path(), &link),
            Err(Obstacle::Dirty(vec![PathBuf::from("tasks/task")]))
        );
        Ok(())
    }

    /// A committed link passes, spelled as the link, and a change inside its
    /// target is not the link's business, since the target is not deleted;
    /// pointing the link elsewhere, or telling git not to look at it, is.
    #[test]
    fn a_committed_link_passes_as_itself_until_it_changes_or_is_unwatched() -> TestOutcome {
        let (scratch, link) = a_link_to_a_committed_task("git-link-committed")?;
        commit_everything(scratch.path())?;
        std::fs::write(scratch.path().join("vendor/task/lib.rs"), "// changed\n")?;
        assert_eq!(
            check(scratch.path(), &link),
            Ok(PathBuf::from("tasks/task"))
        );

        git(
            scratch.path(),
            &["update-index", "--assume-unchanged", "tasks/task"],
        )?;
        assert_eq!(
            check(scratch.path(), &link),
            Err(Obstacle::Unwatched(vec![Unwatched {
                path: PathBuf::from("tasks/task"),
                flag: Flag::AssumeUnchanged,
            }]))
        );
        git(
            scratch.path(),
            &["update-index", "--no-assume-unchanged", "tasks/task"],
        )?;

        std::fs::remove_file(&link)?;
        std::os::unix::fs::symlink("../vendor", &link)?;
        assert_eq!(
            check(scratch.path(), &link),
            Err(Obstacle::Dirty(vec![PathBuf::from("tasks/task")]))
        );
        Ok(())
    }

    #[test]
    fn a_directory_outside_any_repository_is_refused_as_no_repository() -> TestOutcome {
        let scratch = ScratchDir::new("git-no-repository")?;
        let task = scratch.path().join("task");
        std::fs::create_dir_all(&task)?;

        assert_eq!(check(scratch.path(), &task), Err(Obstacle::NotARepository));
        Ok(())
    }

    /// Spawning a program name that does not exist fails with `NotFound`,
    /// the same error a machine without `git` gives.
    #[test]
    fn a_missing_git_binary_is_refused() -> TestOutcome {
        let scratch = ScratchDir::new("git-missing")?;
        assert_eq!(
            check_with(
                || Command::new("ritual-test-no-such-git"),
                scratch.path(),
                scratch.path()
            ),
            Err(Obstacle::GitMissing)
        );
        Ok(())
    }

    #[test]
    fn a_directory_that_is_its_own_repository_is_refused() -> TestOutcome {
        let (scratch, task) = committed_task("git-own-repository")?;
        git(&task, &["init"])?;

        assert_eq!(
            check(scratch.path(), &task),
            Err(Obstacle::OwnRepository(PathBuf::new()))
        );
        Ok(())
    }

    /// The task sits in a repository of its own one level up, inside the
    /// project but not the project's: that repository is clean, and the
    /// project's git still has no record of the task.
    #[test]
    fn a_directory_in_a_repository_that_is_not_the_projects_is_refused() -> TestOutcome {
        let scratch = ScratchDir::new("git-other-repository")?;
        let tasks = scratch.path().join("tasks");
        std::fs::create_dir_all(tasks.join("lint/src"))?;
        std::fs::write(tasks.join("lint/src/lib.rs"), "// committed elsewhere\n")?;
        git(&tasks, &["init"])?;
        commit_everything(&tasks)?;
        std::fs::write(scratch.path().join("Cargo.toml"), "[workspace]\n")?;
        git(scratch.path(), &["init"])?;
        git(scratch.path(), &["add", "Cargo.toml"])?;
        git(scratch.path(), &["commit", "--message", "fixture"])?;

        assert_eq!(
            check(scratch.path(), &tasks.join("lint")),
            Err(Obstacle::OtherRepository(std::fs::canonicalize(&tasks)?))
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
            check(scratch.path(), &task),
            Err(Obstacle::OwnRepository(PathBuf::from("vendor/upstream")))
        );
        Ok(())
    }

    /// An edit to a file marked assume-unchanged or skip-worktree is
    /// invisible to `git status`, so the flags themselves are refused.
    #[test]
    fn files_git_has_been_told_not_to_look_at_are_refused_naming_each_flag() -> TestOutcome {
        let (scratch, task) = committed_task("git-unwatched")?;
        std::fs::write(task.join("assumed.txt"), "committed\n")?;
        std::fs::write(task.join("skipped.txt"), "committed\n")?;
        commit_everything(scratch.path())?;
        git(
            scratch.path(),
            &["update-index", "--assume-unchanged", "task/assumed.txt"],
        )?;
        git(
            scratch.path(),
            &["update-index", "--skip-worktree", "task/skipped.txt"],
        )?;
        std::fs::write(task.join("assumed.txt"), "edited\n")?;
        std::fs::write(task.join("skipped.txt"), "edited\n")?;

        assert_eq!(
            check(scratch.path(), &task),
            Err(Obstacle::Unwatched(vec![
                Unwatched {
                    path: PathBuf::from("task/assumed.txt"),
                    flag: Flag::AssumeUnchanged,
                },
                Unwatched {
                    path: PathBuf::from("task/skipped.txt"),
                    flag: Flag::SkipWorktree,
                },
            ]))
        );
        Ok(())
    }

    /// A sparse checkout marks every file outside it skip-worktree and
    /// leaves it off disk, so there is nothing of it to lose.
    #[test]
    fn a_skip_worktree_file_that_is_not_on_disk_passes() -> TestOutcome {
        let (scratch, task) = committed_task("git-sparse")?;
        std::fs::write(task.join("elsewhere.txt"), "committed\n")?;
        commit_everything(scratch.path())?;
        git(
            scratch.path(),
            &["update-index", "--skip-worktree", "task/elsewhere.txt"],
        )?;
        std::fs::remove_file(task.join("elsewhere.txt"))?;

        assert_eq!(check(scratch.path(), &task), Ok(PathBuf::from("task")));
        Ok(())
    }

    /// A clean filter that drops a line stores less than is on disk, and
    /// `git status` still calls the file clean; only the attribute shows it.
    /// Git LFS stores the file whole, so its filter passes.
    #[test]
    fn a_file_behind_a_filter_other_than_lfs_is_refused_naming_it() -> TestOutcome {
        let (scratch, task) = committed_task("git-filter")?;
        git(
            scratch.path(),
            &["config", "filter.strip.clean", "sed '/SECRET/d'"],
        )?;
        git(scratch.path(), &["config", "filter.lfs.clean", "cat"])?;
        std::fs::write(
            task.join(".gitattributes"),
            "*.cfg filter=strip\n*.bin filter=lfs\n",
        )?;
        std::fs::write(task.join("local.cfg"), "kept\nSECRET=1\n")?;
        std::fs::write(task.join("large.bin"), "stored whole\n")?;
        commit_everything(scratch.path())?;

        assert_eq!(
            check(scratch.path(), &task),
            Err(Obstacle::Filtered(vec![(
                PathBuf::from("task/local.cfg"),
                "strip".to_string()
            )]))
        );

        std::fs::write(task.join(".gitattributes"), "*.bin filter=lfs\n")?;
        commit_everything(scratch.path())?;
        assert_eq!(check(scratch.path(), &task), Ok(PathBuf::from("task")));
        Ok(())
    }

    #[test]
    fn the_index_is_read_for_gitlinks_and_flags() -> TestOutcome {
        let output = b"H 100644 0123456789abcdef0123456789abcdef01234567 0\tsrc/lib.rs\0\
            H 160000 89abcdef0123456789abcdef0123456789abcdef 0\tvendor/upstream\0\
            h 100755 0123456789abcdef0123456789abcdef01234567 0\trun.sh\0\
            S 100644 0123456789abcdef0123456789abcdef01234567 0\tlocal.toml\0";
        let entry = |path: &str, is_gitlink, flag| IndexEntry {
            path: PathBuf::from(path),
            is_gitlink,
            flag,
        };
        assert_eq!(
            parse_index(output).map_err(|obstacle| format!("{obstacle:?}"))?,
            [
                entry("src/lib.rs", false, None),
                entry("vendor/upstream", true, None),
                entry("run.sh", false, Some(Flag::AssumeUnchanged)),
                entry("local.toml", false, Some(Flag::SkipWorktree)),
            ]
        );
        Ok(())
    }

    #[test]
    fn an_index_entry_this_cannot_read_is_a_failure_not_a_skip() {
        for output in [
            &b"H 160000 89abcdef 0\0"[..],
            &b"160000 89abcdef 0\tno-tag\0"[..],
            &b"HS 100644 89abcdef 0\ttwo-letter-tag\0"[..],
        ] {
            let result = parse_index(output);
            assert!(
                matches!(
                    &result,
                    Err(Obstacle::Failed(message)) if message.contains("cannot read")
                ),
                "expected {output:?} to be refused, got {result:?}"
            );
        }
    }

    #[test]
    fn attributes_are_read_in_threes() -> TestOutcome {
        let output = b"a.cfg\0filter\0strip\0b.rs\0filter\0unspecified\0";
        assert_eq!(
            parse_attributes(output).map_err(|obstacle| format!("{obstacle:?}"))?,
            [
                (PathBuf::from("a.cfg"), "strip".to_string()),
                (PathBuf::from("b.rs"), "unspecified".to_string()),
            ]
        );
        let result = parse_attributes(b"a.cfg\0filter\0");
        assert!(
            matches!(&result, Err(Obstacle::Failed(message)) if message.contains("cannot read")),
            "expected output not in threes to be refused, got {result:?}"
        );
        Ok(())
    }

    /// A failure other than "not a repository" is passed on in git's words:
    /// asking about a directory that does not exist makes git fail without
    /// saying that.
    #[test]
    fn any_other_git_failure_passes_gits_own_words_through() -> TestOutcome {
        let scratch = ScratchDir::new("git-other-failure")?;
        let result = check(scratch.path(), &scratch.path().join("no-such-directory"));
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
