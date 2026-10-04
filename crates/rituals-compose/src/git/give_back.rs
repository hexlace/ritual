//! Whether git can give back every file in a directory that is about to be
//! deleted.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::index::{Flag, Unwatched, parse_attributes, parse_index};
use super::status::{IgnoredFiles, ensure_nothing_dirty};
use super::{Obstacle, canonical, from_the_top_level, run_git, run_git_with_input, top_level_of};

/// The one `filter` driver whose files git gives back byte for byte: Git
/// LFS stores the file itself and puts it back on checkout.
const GIVES_BACK_ITS_BYTES: &str = "lfs";

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
/// use rituals_compose::git::{self, Obstacle};
///
/// // Needs a real repository on disk and runs `git`, so this example is
/// // `no_run`.
/// let workspace_root = Path::new(".");
/// match git::ensure_git_can_give_back(Path::new(".rituals/lint"), workspace_root) {
///     Ok(from_the_top_level) => println!("git can give back {}", from_the_top_level.display()),
///     Err(Obstacle::Dirty(files)) => println!("{} files git cannot give back", files.len()),
///     Err(other) => println!("git cannot vouch for it: {other}"),
/// }
/// ```
///
/// # Errors
///
/// Returns an [`Obstacle`] naming why git cannot vouch for the directory:
/// any of its variants, [`Obstacle::GitMissing`], [`Obstacle::NotARepository`],
/// [`Obstacle::OwnRepository`], [`Obstacle::OtherRepository`],
/// [`Obstacle::Failed`], [`Obstacle::Unwatched`], [`Obstacle::Filtered`] and
/// [`Obstacle::Dirty`], the last one naming every file git cannot give back.
pub fn ensure_git_can_give_back(
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
            (flag == Flag::AssumeUnchanged || on_disk)
                .then(|| Unwatched::new(from_top_level.join(&entry.path), flag))
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

    ensure_nothing_dirty(&new_git, directory, &["."], IgnoredFiles::Count)?;
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
            entry
                .flag
                .map(|flag| Unwatched::new(from_top_level.clone(), flag))
        })
        .collect();
    if !unwatched.is_empty() {
        return Err(Obstacle::Unwatched(unwatched));
    }

    ensure_nothing_dirty(new_git, holder, &[&pathspec], IgnoredFiles::Count)?;
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::check_with;
    use crate::git::test_support::{
        add_a_submodule, commit_everything, committed_task, contained_in, git,
    };
    use crate::git::{Flag, Obstacle, Unwatched};
    use crate::test_support::{ScratchDir, TestOutcome};

    /// [`check_with`] on `task`, with the scratch directory as the project's
    /// workspace root.
    fn check(scratch: &Path, task: &Path) -> Result<PathBuf, Obstacle> {
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
            Err(Obstacle::Unwatched(vec![Unwatched::new(
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
    /// only the gitlink in the index shows it.
    #[test]
    fn a_directory_holding_a_clean_submodule_is_refused_naming_it() -> TestOutcome {
        let (scratch, task) = committed_task("git-submodule")?;
        add_a_submodule(scratch.path(), "task/vendor/upstream")?;

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
