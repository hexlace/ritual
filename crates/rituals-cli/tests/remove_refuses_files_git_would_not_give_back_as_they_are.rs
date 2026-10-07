//! `cargo ritual remove` deletes a task's directory only when this project's
//! git can give back these exact bytes, and `git status` saying "clean" is
//! not always that.
//!
//! A file marked `--assume-unchanged` or `--skip-worktree` is one git has
//! been told not to look at, so an edit to it reads as clean, and `git
//! checkout` gives back the committed file, or with skip-worktree nothing.
//! A file behind a clean filter is stored as the filter left it, which can
//! be less than is on disk. And a directory reached through a symbolic link
//! into another repository is clean to that repository's git, which is not
//! the one `git checkout` in this project asks. Each is refused, naming what
//! is wrong, with the tree byte-identical and `Cargo.lock` as it was; then
//! the cause is cleared and the same `remove` succeeds.

mod support;

use support::removal::{
    assert_remove_is_refused_and_leaves_the_lockfile, exists, project_with_a_committed_task,
};
use support::{TempDir, TestOutcome, assert_trees_identical, git, in_checkout, snapshot_tree};

/// Appends a line to `path`, an edit `git status` would normally see.
fn edit(path: &std::path::Path) -> TestOutcome {
    let mut text = support::read_text(path)?;
    text.push_str("// edited after the last commit\n");
    support::write_text(path, &text)
}

/// Asserts `git status` calls the project clean, so the refusal is about the
/// flag or the filter and not an edit git can see.
fn assert_status_is_clean(root: &std::path::Path) -> TestOutcome {
    let status = git::git(root, &["status", "--porcelain"])?;
    status.expect_success("`git status`");
    assert!(
        status.stdout.is_empty(),
        "fixture precondition: git status must call the project clean, got:\n{}",
        status.stdout
    );
    Ok(())
}

#[test]
fn an_edited_assume_unchanged_file_is_refused_naming_it() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-assume-unchanged")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        git::git(
            project.root(),
            &[
                "update-index",
                "--assume-unchanged",
                "tasks/greet/src/lib.rs",
            ],
        )?
        .expect_success("`git update-index --assume-unchanged`");
        edit(&project.root().join("tasks/greet/src/lib.rs"))?;
        assert_status_is_clean(project.root())?;

        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "greet",
            &[":/tasks/greet/src/lib.rs (assume-unchanged)"],
            |_| Ok(()),
        )?;

        git::git(
            project.root(),
            &[
                "update-index",
                "--no-assume-unchanged",
                "tasks/greet/src/lib.rs",
            ],
        )?
        .expect_success("`git update-index --no-assume-unchanged`");
        git::commit_everything(project.root())?;
        project
            .run_cli(&["remove", "greet"])?
            .expect_success("`remove greet` once the flag is cleared and the edit committed");
        assert!(!exists(&project.root().join("tasks/greet")));
        Ok(())
    })
}

#[test]
fn an_edited_skip_worktree_file_is_refused_naming_it() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-skip-worktree")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        git::git(
            project.root(),
            &["update-index", "--skip-worktree", "tasks/greet/src/lib.rs"],
        )?
        .expect_success("`git update-index --skip-worktree`");
        edit(&project.root().join("tasks/greet/src/lib.rs"))?;
        assert_status_is_clean(project.root())?;

        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "greet",
            &[":/tasks/greet/src/lib.rs (skip-worktree)"],
            |_| Ok(()),
        )?;

        git::git(
            project.root(),
            &[
                "update-index",
                "--no-skip-worktree",
                "tasks/greet/src/lib.rs",
            ],
        )?
        .expect_success("`git update-index --no-skip-worktree`");
        git::commit_everything(project.root())?;
        project
            .run_cli(&["remove", "greet"])?
            .expect_success("`remove greet` once the flag is cleared and the edit committed");
        assert!(!exists(&project.root().join("tasks/greet")));
        Ok(())
    })
}

/// A `sed`-style clean filter drops a line on the way into git, so the
/// committed file is short of what is on disk while `git status` calls it
/// clean. Deleting the file would lose the line git never stored, and a check
/// that trusted `git status` alone would let it go.
#[test]
fn a_file_behind_a_lossy_clean_filter_is_refused_naming_it() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-filter")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        git::git(
            project.root(),
            &["config", "filter.strip.clean", "sed '/SECRET/d'"],
        )?
        .expect_success("`git config filter.strip.clean`");
        support::write_text(
            &project.root().join("tasks/greet/.gitattributes"),
            "*.cfg filter=strip\n",
        )?;
        support::write_text(
            &project.root().join("tasks/greet/local.cfg"),
            "kept = 1\nSECRET = only on disk\n",
        )?;
        git::commit_everything(project.root())?;
        assert_status_is_clean(project.root())?;

        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "greet",
            &[":/tasks/greet/local.cfg (filter `strip`)"],
            |_| Ok(()),
        )?;

        std::fs::remove_file(project.root().join("tasks/greet/.gitattributes"))?;
        git::commit_everything(project.root())?;
        project
            .run_cli(&["remove", "greet"])?
            .expect_success("`remove greet` once nothing goes through the filter");
        assert!(!exists(&project.root().join("tasks/greet")));
        Ok(())
    })
}

/// `tasks` is a committed symbolic link into another, clean repository.
/// Cargo spells the task's directory through the link, as if it were inside
/// the project, and that repository's git calls it clean, but this project's
/// git could give none of it back: a check that asked whichever repository
/// holds the files would delete work nothing here can restore.
#[test]
fn a_task_reached_through_a_link_into_another_repository_is_refused() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-linked-repository")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        let elsewhere = working_dir.path().join("elsewhere");
        std::fs::rename(project.root().join("tasks"), &elsewhere)?;
        git::init_and_commit_everything(&elsewhere)?;
        std::os::unix::fs::symlink(&elsewhere, project.root().join("tasks"))?;
        git::commit_everything(project.root())?;
        assert_status_is_clean(project.root())?;
        let elsewhere_before = snapshot_tree(&elsewhere)?;

        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "greet",
            &["tasks/greet", "not inside the workspace root"],
            |_| Ok(()),
        )?;
        assert_trees_identical(
            "a refused `remove greet` must leave the other repository as it was",
            &elsewhere_before,
            &snapshot_tree(&elsewhere)?,
        );
        Ok(())
    })
}
