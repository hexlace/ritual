//! The git check `migrate` makes before it writes anything.

use std::path::{Path, PathBuf};

use rituals::Failure;
use rituals_compose::git::{self, NotClean, Unanswered};

use crate::places::spelled_from;

/// How many uncommitted paths a refusal names before it counts the rest, so
/// a project that was never committed does not print a screenful.
const DIRTY_PATHS_NAMED: usize = 10;

/// Where the project's root is in the repository its clean work tree is in.
pub(crate) struct Repository {
    root_from_top_level: PathBuf,
}

impl Repository {
    /// `path`, which git spelled from the repository's top level, as the
    /// person knows it: from the project's root.
    pub(crate) fn shown(&self, path_from_top_level: &Path) -> String {
        spelled_from(&self.root_from_top_level, path_from_top_level)
    }
}

/// What git said about the work tree before anything was read or written.
pub(crate) struct WorkTree(Result<Repository, NotClean>);

impl WorkTree {
    /// Asks git whether everything in the work tree `workspace_root` is in is
    /// committed.
    pub(crate) fn take(workspace_root: &Path) -> Self {
        Self(located(workspace_root))
    }

    /// The repository when its work tree was clean, and the refusal that says
    /// what git found when it was not.
    pub(crate) fn ensure_clean(&self, migrate_command: &str) -> Result<&Repository, Failure> {
        self.0
            .as_ref()
            .map_err(|not_clean| refusal(not_clean, migrate_command))
    }
}

/// Asks git about the work tree `workspace_root` is in, and where the root is
/// in it.
fn located(workspace_root: &Path) -> Result<Repository, NotClean> {
    let top_level = git::ensure_work_tree_is_clean(workspace_root)?;
    // Git spells paths from a top level that is resolved through symbolic
    // links, so the root is too before the two are compared.
    let resolved_root = std::fs::canonicalize(workspace_root).map_err(|error| {
        NotClean::Unanswered(Unanswered::Failed(format!(
            "reading {} failed: {error}",
            workspace_root.display()
        )))
    })?;
    let root_from_top_level = resolved_root
        .strip_prefix(&top_level)
        .map(Path::to_path_buf)
        .map_err(|_| {
            NotClean::Unanswered(Unanswered::Failed(format!(
                "{} is not inside the repository at {}",
                resolved_root.display(),
                top_level.display()
            )))
        })?;
    Ok(Repository {
        root_from_top_level,
    })
}

/// The refusal for what git said about the work tree.
pub(crate) fn refusal(not_clean: &NotClean, migrate_command: &str) -> Failure {
    match not_clean {
        NotClean::Unanswered(unanswered) => {
            unanswered_refusal(unanswered, migrate_command, |message| {
                Failure::new(format!(
                    "refusing to migrate: git could not say whether the work tree is clean: \
                     {message}"
                ))
            })
        }
        NotClean::Dirty(files) => Failure::new(format!(
            "refusing to migrate: the work tree has changes that are not committed — {}; \
             commit or discard them, then run `{migrate_command}` again",
            dirty_paths(files)
        )),
    }
}

/// The refusal for a git that could not answer: the same words whatever it
/// was asked when there is no repository or no `git`, and `failed`'s own when
/// git ran and said something else, since what it said is about the question.
pub(crate) fn unanswered_refusal(
    unanswered: &Unanswered,
    migrate_command: &str,
    failed: impl FnOnce(&str) -> Failure,
) -> Failure {
    match unanswered {
        Unanswered::NotARepository => Failure::new(format!(
            "refusing to migrate: this project is not in a git repository, so nothing could \
             give back what migrate moves and edits; make it one and commit everything in it, \
             then run `{migrate_command}` again"
        )),
        Unanswered::GitMissing => Failure::new(format!(
            "refusing to migrate: `git` could not be run, and migrate needs it to check that \
             git can give back everything migrate moves and edits; make `git` available on \
             PATH, then run `{migrate_command}` again"
        )),
        Unanswered::Failed(message) => failed(message),
    }
}

/// The first few of `files`, each from the top level with the `:/` pathspec
/// magic, which git reads from there whatever directory it is run in, and a
/// count of the rest.
fn dirty_paths(files: &[PathBuf]) -> String {
    let named: Vec<String> = files
        .iter()
        .take(DIRTY_PATHS_NAMED)
        .map(|file| format!(":/{}", file.display()))
        .collect();
    let named = named.join(", ");
    match files.len().checked_sub(DIRTY_PATHS_NAMED) {
        Some(rest) if rest > 0 => format!("{named} and {rest} more"),
        Some(_) | None => named,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rituals_compose::git::{NotClean, Unanswered};

    use super::{WorkTree, refusal};
    use crate::test_support::{ScratchDir, TestOutcome, init_and_commit};

    const MIGRATE: &str = "cargo ritual migrate";

    #[test]
    fn a_project_outside_a_repository_is_refused_with_how_to_make_one() {
        assert_eq!(
            refusal(&Unanswered::NotARepository.into(), MIGRATE).to_string(),
            "refusing to migrate: this project is not in a git repository, so nothing could \
             give back what migrate moves and edits; make it one and commit everything in it, \
             then run `cargo ritual migrate` again"
        );
    }

    #[test]
    fn a_missing_git_is_refused_with_what_to_do_about_it() {
        assert_eq!(
            refusal(&Unanswered::GitMissing.into(), MIGRATE).to_string(),
            "refusing to migrate: `git` could not be run, and migrate needs it to check that \
             git can give back everything migrate moves and edits; make `git` available on \
             PATH, then run `cargo ritual migrate` again"
        );
    }

    #[test]
    fn uncommitted_files_are_named_from_the_top_level() {
        let files = vec![PathBuf::from("notes.txt"), PathBuf::from("src/a.rs")];
        assert_eq!(
            refusal(&NotClean::Dirty(files), MIGRATE).to_string(),
            "refusing to migrate: the work tree has changes that are not committed — \
             :/notes.txt, :/src/a.rs; commit or discard them, then run `cargo ritual migrate` \
             again"
        );
    }

    #[test]
    fn ten_uncommitted_files_are_all_named() {
        let files: Vec<PathBuf> = (1..=10).map(|n| PathBuf::from(format!("f{n}"))).collect();
        let message = refusal(&NotClean::Dirty(files), MIGRATE).to_string();
        assert!(message.contains(":/f1, "), "{message}");
        assert!(message.contains(":/f10;"), "{message}");
        assert!(!message.contains("more"), "{message}");
    }

    /// Past ten the list is cut, so a project that has never been committed
    /// does not print a screenful: the rest is counted.
    #[test]
    fn past_ten_uncommitted_files_the_rest_are_counted() {
        let files: Vec<PathBuf> = (1..=13).map(|n| PathBuf::from(format!("f{n}"))).collect();
        let message = refusal(&NotClean::Dirty(files), MIGRATE).to_string();
        assert!(message.contains(":/f10 and 3 more;"), "{message}");
        assert!(!message.contains("f11"), "{message}");
    }

    #[test]
    fn a_git_failure_is_refused_in_git_s_own_words() {
        assert_eq!(
            refusal(
                &Unanswered::Failed("bad object".to_string()).into(),
                MIGRATE
            )
            .to_string(),
            "refusing to migrate: git could not say whether the work tree is clean: bad object"
        );
    }

    #[test]
    fn the_command_is_spelled_for_the_command_line_that_runs_it() {
        let message = refusal(
            &Unanswered::NotARepository.into(),
            "cargo acme ritual migrate",
        )
        .to_string();
        assert!(
            message.ends_with("then run `cargo acme ritual migrate` again"),
            "{message}"
        );
    }

    #[test]
    fn a_committed_work_tree_is_clean_and_knows_where_its_root_is() -> TestOutcome {
        let scratch = ScratchDir::new("precondition-clean")?;
        let root = scratch.path().join("project");
        std::fs::create_dir_all(root.join("ritual"))?;
        std::fs::write(root.join("ritual/Cargo.toml"), "# committed\n")?;
        init_and_commit(&root)?;

        let work_tree = WorkTree::take(&root);

        let repository = work_tree
            .ensure_clean(MIGRATE)
            .map_err(|failure| failure.to_string())?;
        assert_eq!(
            repository.shown(std::path::Path::new("ritual/Cargo.toml")),
            "ritual/Cargo.toml"
        );
        Ok(())
    }

    /// The project's root can be below the repository's top level, as in a
    /// monorepo: a path git reports from the top level is then spelled from
    /// the root, with `..` to leave it.
    #[test]
    fn a_root_below_the_top_level_spells_paths_from_itself() -> TestOutcome {
        let scratch = ScratchDir::new("precondition-below")?;
        let root = scratch.path().join("monorepo/project");
        std::fs::create_dir_all(&root)?;
        std::fs::write(root.join("Cargo.toml"), "# committed\n")?;
        std::fs::create_dir_all(scratch.path().join("monorepo/.github"))?;
        std::fs::write(scratch.path().join("monorepo/.github/ci.yml"), "x\n")?;
        init_and_commit(&scratch.path().join("monorepo"))?;

        let work_tree = WorkTree::take(&root);

        let repository = work_tree
            .ensure_clean(MIGRATE)
            .map_err(|failure| failure.to_string())?;
        assert_eq!(
            repository.shown(std::path::Path::new("project/Cargo.toml")),
            "Cargo.toml"
        );
        assert_eq!(
            repository.shown(std::path::Path::new(".github/ci.yml")),
            "../.github/ci.yml"
        );
        Ok(())
    }

    #[test]
    fn a_changed_file_makes_the_work_tree_dirty() -> TestOutcome {
        let scratch = ScratchDir::new("precondition-changed")?;
        std::fs::write(scratch.path().join("notes.txt"), "as committed\n")?;
        init_and_commit(scratch.path())?;
        std::fs::write(scratch.path().join("notes.txt"), "changed\n")?;

        let work_tree = WorkTree::take(scratch.path());

        let refused = work_tree
            .ensure_clean(MIGRATE)
            .err()
            .map(|failure| failure.to_string());
        let refused = refused.ok_or("a changed file must make the work tree dirty")?;
        assert!(refused.contains(":/notes.txt"), "{refused}");
        Ok(())
    }

    #[test]
    fn an_untracked_file_makes_the_work_tree_dirty() -> TestOutcome {
        let scratch = ScratchDir::new("precondition-untracked")?;
        std::fs::write(scratch.path().join("a.txt"), "x\n")?;
        init_and_commit(scratch.path())?;
        std::fs::write(scratch.path().join("scratch.txt"), "not in git\n")?;

        let work_tree = WorkTree::take(scratch.path());

        let refused = work_tree
            .ensure_clean(MIGRATE)
            .err()
            .map(|failure| failure.to_string());
        let refused = refused.ok_or("an untracked file must make the work tree dirty")?;
        assert!(refused.contains(":/scratch.txt"), "{refused}");
        Ok(())
    }

    /// A file git ignores is not something a commit would carry, and `migrate`
    /// carries it along with its directory, so it does not count.
    #[test]
    fn an_ignored_file_does_not_make_the_work_tree_dirty() -> TestOutcome {
        let scratch = ScratchDir::new("precondition-ignored")?;
        std::fs::write(scratch.path().join(".gitignore"), "*.log\n")?;
        init_and_commit(scratch.path())?;
        std::fs::write(scratch.path().join("build.log"), "ignored\n")?;

        let work_tree = WorkTree::take(scratch.path());

        assert!(work_tree.ensure_clean(MIGRATE).is_ok());
        Ok(())
    }
}
