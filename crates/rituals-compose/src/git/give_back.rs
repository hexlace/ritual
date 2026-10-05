//! Whether git can give back every file in a directory that is about to be
//! deleted.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::index::{Flag, IndexEntry, Unwatched, parse_attributes, parse_index};
use super::status::{IgnoredFiles, uncommitted};
use super::{
    Unanswered, canonical, counted, counted_with_verb, from_the_top_level, nul_terminated, run_git,
    run_git_with_input, top_level_of,
};

/// The one `filter` driver whose files git gives back byte for byte: Git
/// LFS stores the file itself and puts it back on checkout.
const GIVES_BACK_ITS_BYTES: &str = "lfs";

/// Why git cannot give back what is in a directory.
///
/// Holds [`Unanswered`] for the three facts every question here can fail on,
/// and the facts of its own for the ones that stop git giving a directory
/// back. Every path a variant holds is spelled from git's top level, the form
/// the `:/` pathspec takes, so a person can hand it to git from any directory
/// of the project — except [`CannotGiveBack::OwnRepository`]'s, which is the
/// directory's own. The variants state git's facts and no remedy: a caller
/// builds its own refusal from the one it receives, in its own words. See
/// [`Unanswered`] for why the variants are public and the enum is exhaustive.
///
/// # Examples
///
/// ```
/// use rituals_compose::git::CannotGiveBack;
///
/// assert_eq!(
///     CannotGiveBack::OwnRepository("vendor/upstream".into()).to_string(),
///     "vendor/upstream is a git repository of its own"
/// );
/// assert_eq!(
///     CannotGiveBack::Dirty(vec!["a.txt".into(), "b.txt".into()]).to_string(),
///     "2 files are untracked, ignored or changed since the last commit"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CannotGiveBack {
    /// Git could not answer at all.
    Unanswered(Unanswered),
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
    /// Tracked files git has been told not to look at, so `git status`
    /// calls them clean whatever is on disk.
    Unwatched(Vec<Unwatched>),
    /// Tracked files a `filter` driver other than Git LFS cleans on the way
    /// in, with the driver's name: what git stores can differ from what is
    /// on disk, and `git checkout` gives back what it stored.
    Filtered(Vec<(PathBuf, String)>),
    /// Files git could not give back once they are deleted: untracked,
    /// ignored or changed since the last commit.
    Dirty(Vec<PathBuf>),
}

impl From<Unanswered> for CannotGiveBack {
    fn from(unanswered: Unanswered) -> Self {
        Self::Unanswered(unanswered)
    }
}

impl fmt::Display for CannotGiveBack {
    /// Says git's fact in lowercase, with no remedy and no trailing
    /// punctuation, so a caller can put it inside a sentence of its own.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unanswered(unanswered) => fmt::Display::fmt(unanswered, formatter),
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

impl std::error::Error for CannotGiveBack {}

/// Checks, with the `git` on `PATH`, that every file in `directory` is one
/// git can give back.
///
/// That holds when the directory is in the same repository as
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
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::git::{self, CannotGiveBack};
///
/// // Needs a real repository on disk and runs `git`, so this example is
/// // `no_run`.
/// let workspace_root = Path::new(".");
/// match git::ensure_git_can_give_back(Path::new(".rituals/lint"), workspace_root) {
///     Ok(from_the_top_level) => println!("git can give back {}", from_the_top_level.display()),
///     Err(CannotGiveBack::Dirty(files)) => println!("{} files git cannot give back", files.len()),
///     Err(other) => println!("git cannot vouch for it: {other}"),
/// }
/// ```
///
/// # Errors
///
/// Returns a [`CannotGiveBack`] naming why git cannot vouch for the
/// directory: [`CannotGiveBack::Unanswered`] when git could not answer, one
/// of the variants for a repository, a flag or a filter git cannot see
/// through, or [`CannotGiveBack::Dirty`], which names every file git cannot
/// give back.
pub fn ensure_git_can_give_back(
    directory: &Path,
    workspace_root: &Path,
) -> Result<PathBuf, CannotGiveBack> {
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
) -> Result<PathBuf, CannotGiveBack> {
    let is_a_link = directory
        .symlink_metadata()
        .map_err(|error| {
            Unanswered::Failed(format!("reading {} failed: {error}", directory.display()))
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
        return Err(CannotGiveBack::OwnRepository(PathBuf::new()));
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
        return Err(CannotGiveBack::OwnRepository(submodule.path.clone()));
    }
    ensure_none_unwatched(&index, directory, &from_top_level)?;
    ensure_none_filtered(&new_git, directory, &index, &from_top_level)?;
    ensure_nothing_uncommitted(&new_git, directory, &["."])?;
    Ok(from_top_level)
}

/// Refuses when any entry of `index`, read from `directory`, carries a flag
/// that stops `git status` looking at it, naming each from `from_top_level`,
/// the directory's own spelling.
fn ensure_none_unwatched(
    index: &[IndexEntry],
    directory: &Path,
    from_top_level: &Path,
) -> Result<(), CannotGiveBack> {
    let unwatched: Vec<Unwatched> = index
        .iter()
        .filter_map(|entry| {
            let flag = entry.flag?;
            // A sparse checkout marks every file outside it skip-worktree
            // and leaves it off disk; with nothing on disk there is nothing
            // to lose.
            let on_disk = directory.join(&entry.path).symlink_metadata().is_ok();
            (flag == Flag::AssumeUnchanged || on_disk)
                .then(|| Unwatched::new(from_top_level.join(&entry.path), flag))
        })
        .collect();
    if unwatched.is_empty() {
        Ok(())
    } else {
        Err(CannotGiveBack::Unwatched(unwatched))
    }
}

/// Refuses when a filter driver other than Git LFS applies to any entry of
/// `index`, asked about from `directory`, naming each from `from_top_level`,
/// the directory's own spelling.
fn ensure_none_filtered(
    new_git: &impl Fn() -> Command,
    directory: &Path,
    index: &[IndexEntry],
    from_top_level: &Path,
) -> Result<(), CannotGiveBack> {
    let paths = nul_terminated(
        index
            .iter()
            .map(|entry| entry.path.as_os_str().as_encoded_bytes()),
    );
    let filters = parse_attributes(&run_git_with_input(
        new_git,
        directory,
        &["check-attr", "-z", "--stdin", "filter"],
        &paths,
    )?)?;
    let filtered: Vec<(PathBuf, String)> = filters
        .into_iter()
        .filter(|(_path, _name, value)| {
            !matches!(
                value.as_str(),
                "unspecified" | "unset" | GIVES_BACK_ITS_BYTES
            )
        })
        .map(|(path, _name, value)| (from_top_level.join(path), value))
        .collect();
    if filtered.is_empty() {
        Ok(())
    } else {
        Err(CannotGiveBack::Filtered(filtered))
    }
}

/// Refuses when anything `pathspecs` match, asked from `asked_in`, is
/// untracked, ignored or changed since the last commit, naming each.
fn ensure_nothing_uncommitted(
    new_git: &impl Fn() -> Command,
    asked_in: &Path,
    pathspecs: &[&str],
) -> Result<(), CannotGiveBack> {
    let dirty = uncommitted(new_git, asked_in, pathspecs, IgnoredFiles::Count)?;
    if dirty.is_empty() {
        Ok(())
    } else {
        Err(CannotGiveBack::Dirty(dirty))
    }
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
) -> Result<PathBuf, CannotGiveBack> {
    let (Some(holder), Some(name)) = (link.parent(), link.file_name()) else {
        return Err(Unanswered::Failed(format!(
            "{} has no directory holding it to ask git from",
            link.display()
        ))
        .into());
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
            entry
                .flag
                .map(|flag| Unwatched::new(from_top_level.clone(), flag))
        })
        .collect();
    if !unwatched.is_empty() {
        return Err(CannotGiveBack::Unwatched(unwatched));
    }

    ensure_nothing_uncommitted(new_git, holder, &[&pathspec])?;
    Ok(from_top_level)
}

/// Refuses when `top_level` is not the repository `workspace_root` is in:
/// whatever that other repository vouches for, the project's git cannot give
/// it back.
fn ensure_the_projects(
    new_git: &impl Fn() -> Command,
    top_level: &Path,
    workspace_root: &Path,
) -> Result<(), CannotGiveBack> {
    match top_level_of(new_git, workspace_root) {
        Ok(projects) if projects == top_level => Ok(()),
        Ok(_) | Err(Unanswered::NotARepository) => {
            Err(CannotGiveBack::OtherRepository(top_level.to_path_buf()))
        }
        Err(unanswered) => Err(unanswered.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::check_with;
    use crate::git::fixture::{commit_everything, git};
    use crate::git::test_support::{add_a_submodule, committed_task, contained_in};
    use crate::git::{CannotGiveBack, Flag, Unanswered, Unwatched};
    use crate::test_support::{ScratchDir, TestOutcome};

    /// Every variant renders as git's fact, in lowercase, with no remedy and
    /// no trailing full stop, so a caller can build a sentence around it.
    #[test]
    fn every_cannot_give_back_says_git_s_fact_in_lowercase_with_no_remedy() {
        let unwatched = |flag| Unwatched::new(PathBuf::from("a.txt"), flag);
        let cases = [
            (
                CannotGiveBack::Unanswered(Unanswered::GitMissing),
                "`git` could not be run",
            ),
            (
                CannotGiveBack::OwnRepository(PathBuf::new()),
                "the directory is a git repository of its own",
            ),
            (
                CannotGiveBack::OwnRepository(PathBuf::from("vendor/upstream")),
                "vendor/upstream is a git repository of its own",
            ),
            (
                CannotGiveBack::OtherRepository(PathBuf::from("/elsewhere")),
                "the directory is in the git repository at /elsewhere, which is not the project's",
            ),
            (
                CannotGiveBack::Unwatched(vec![unwatched(Flag::AssumeUnchanged)]),
                "git has been told not to look at 1 tracked file",
            ),
            (
                CannotGiveBack::Unwatched(vec![
                    unwatched(Flag::AssumeUnchanged),
                    unwatched(Flag::SkipWorktree),
                ]),
                "git has been told not to look at 2 tracked files",
            ),
            (
                CannotGiveBack::Filtered(vec![(PathBuf::from("a.cfg"), "strip".to_string())]),
                "git stores 1 tracked file through a filter",
            ),
            (
                CannotGiveBack::Dirty(vec![PathBuf::from("a.txt")]),
                "1 file is untracked, ignored or changed since the last commit",
            ),
            (
                CannotGiveBack::Dirty(vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")]),
                "2 files are untracked, ignored or changed since the last commit",
            ),
        ];
        for (cannot_give_back, expected) in cases {
            assert_eq!(cannot_give_back.to_string(), expected);
            assert!(
                !expected.ends_with('.'),
                "{expected:?} must not end in a full stop"
            );
        }
    }

    /// [`check_with`] on `task`, with the scratch directory as the project's
    /// workspace root.
    fn check(scratch: &Path, task: &Path) -> Result<PathBuf, CannotGiveBack> {
        check_with(contained_in(scratch), task, scratch)
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

        let Err(CannotGiveBack::Dirty(mut files)) = result else {
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
            Err(CannotGiveBack::Dirty(vec![PathBuf::from("tasks/task")]))
        );

        std::fs::write(scratch.path().join("tasks/.gitignore"), "task\n")?;
        git(scratch.path(), &["add", "tasks/.gitignore"])?;
        git(scratch.path(), &["commit", "--message", "ignore the link"])?;
        assert_eq!(
            check(scratch.path(), &link),
            Err(CannotGiveBack::Dirty(vec![PathBuf::from("tasks/task")]))
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
            Err(CannotGiveBack::Unwatched(vec![Unwatched::new(
                PathBuf::from("tasks/task"),
                Flag::AssumeUnchanged
            )]))
        );
        git(
            scratch.path(),
            &["update-index", "--no-assume-unchanged", "tasks/task"],
        )?;

        std::fs::remove_file(&link)?;
        std::os::unix::fs::symlink("../vendor", &link)?;
        assert_eq!(
            check(scratch.path(), &link),
            Err(CannotGiveBack::Dirty(vec![PathBuf::from("tasks/task")]))
        );
        Ok(())
    }

    #[test]
    fn a_directory_outside_any_repository_is_refused_as_no_repository() -> TestOutcome {
        let scratch = ScratchDir::new("git-no-repository")?;
        let task = scratch.path().join("task");
        std::fs::create_dir_all(&task)?;

        assert_eq!(
            check(scratch.path(), &task),
            Err(CannotGiveBack::Unanswered(Unanswered::NotARepository))
        );
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
            Err(CannotGiveBack::Unanswered(Unanswered::GitMissing))
        );
        Ok(())
    }

    #[test]
    fn a_directory_that_is_its_own_repository_is_refused() -> TestOutcome {
        let (scratch, task) = committed_task("git-own-repository")?;
        git(&task, &["init"])?;

        assert_eq!(
            check(scratch.path(), &task),
            Err(CannotGiveBack::OwnRepository(PathBuf::new()))
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
            Err(CannotGiveBack::OtherRepository(std::fs::canonicalize(
                &tasks
            )?))
        );
        Ok(())
    }

    /// A clean submodule leaves nothing in the parent's `git status`, so
    /// only the gitlink in the index shows it.
    #[test]
    fn a_directory_holding_a_clean_submodule_is_refused_naming_it() -> TestOutcome {
        let (scratch, task) = committed_task("git-submodule")?;
        add_a_submodule(scratch.path(), "task/vendor/upstream")?;

        assert_eq!(
            check(scratch.path(), &task),
            Err(CannotGiveBack::OwnRepository(PathBuf::from(
                "vendor/upstream"
            )))
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
            Err(CannotGiveBack::Unwatched(vec![
                Unwatched::new(PathBuf::from("task/assumed.txt"), Flag::AssumeUnchanged),
                Unwatched::new(PathBuf::from("task/skipped.txt"), Flag::SkipWorktree),
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
            Err(CannotGiveBack::Filtered(vec![(
                PathBuf::from("task/local.cfg"),
                "strip".to_string()
            )]))
        );

        std::fs::write(task.join(".gitattributes"), "*.bin filter=lfs\n")?;
        commit_everything(scratch.path())?;
        assert_eq!(check(scratch.path(), &task), Ok(PathBuf::from("task")));
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
                Err(CannotGiveBack::Unanswered(Unanswered::Failed(message)))
                    if message.contains("no-such-directory")
            ),
            "expected git's own message naming the directory, got {result:?}"
        );
        Ok(())
    }
}
