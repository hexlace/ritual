//! `migrate` moves directories and rewrites manifests, so it only starts
//! where git can give everything back: it refuses in a work tree with
//! uncommitted changes or untracked files, and outside a git repository,
//! before it writes anything. The person reviews the diff and commits it.
//!
//! Each story refuses first, checks the tree is byte-identical and that the
//! refusal says what is in the way, then clears the cause and shows the same
//! command now moves the task: a refusal that named the wrong thing, or was
//! about something else, would not survive that.

mod support;

use support::migration::{assert_names, exists};
use support::removal::assert_a_refusal;
use support::{
    Project, TempDir, TestOutcome, assert_trees_identical, git, in_checkout, legacy, snapshot_tree,
    write_text,
};

/// Runs `migrate` on `project`, asserts ritual refused it naming each of
/// `expected` (compared without regard to case) and left the whole tree
/// byte-identical.
#[track_caller]
fn assert_migrate_is_refused_and_writes_nothing(
    project: &Project,
    expected: &[&str],
) -> TestOutcome {
    let before = snapshot_tree(project.root())?;
    let bin_name = project.bin_name()?;

    let refused = project.run_cli(&["migrate"])?;

    let message = assert_a_refusal(&refused, &bin_name, "`migrate`").to_lowercase();
    for name in expected {
        assert!(
            message.contains(&name.to_lowercase()),
            "expected the refusal of `migrate` to name `{name}`; stderr was:\n{}",
            refused.stderr
        );
    }
    assert_trees_identical(
        "a refused `migrate` must write nothing",
        &before,
        &snapshot_tree(project.root())?,
    );
    Ok(())
}

/// Asserts `migrate` now moves the project's task, and says so.
#[track_caller]
fn assert_migrate_now_moves_greet(project: &Project) -> TestOutcome {
    let migrated = project.run_cli(&["migrate"])?;
    migrated.expect_success("`migrate` once the cause of the refusal was cleared");
    assert_names(&migrated, "`migrate`", &["greet"]);
    assert!(
        exists(&project.root().join(".rituals/greet/Cargo.toml")),
        "expected .rituals/greet to exist once `migrate` could run"
    );
    assert!(
        !exists(&project.root().join("tasks/greet")),
        "expected tasks/greet to have moved once `migrate` could run"
    );
    Ok(())
}

#[test]
fn a_work_tree_with_an_uncommitted_change_is_refused_before_anything_is_written() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-refuses-dirty")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        write_text(&project.root().join("notes.txt"), "as committed\n")?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;
        write_text(
            &project.root().join("notes.txt"),
            "changed since the commit\n",
        )?;

        assert_migrate_is_refused_and_writes_nothing(&project, &["notes.txt"])?;

        git::commit_everything(project.root())?;
        assert_migrate_now_moves_greet(&project)
    })
}

#[test]
fn a_work_tree_with_an_untracked_file_is_refused_before_anything_is_written() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-refuses-untracked")?;
        let project = legacy::committed_project_with_tasks(checkout, &working_dir, &["greet"])?;
        write_text(&project.root().join("scratch.txt"), "not in git yet\n")?;

        assert_migrate_is_refused_and_writes_nothing(&project, &["scratch.txt"])?;

        git::commit_everything(project.root())?;
        assert_migrate_now_moves_greet(&project)
    })
}

#[test]
fn a_project_outside_a_git_repository_is_refused_before_anything_is_written() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-refuses-without-git")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        assert!(
            !exists(&project.root().join(".git")),
            "fixture precondition: the project must not be a git repository"
        );

        assert_migrate_is_refused_and_writes_nothing(&project, &["git"])?;

        git::init_and_commit_everything(project.root())?;
        assert_migrate_now_moves_greet(&project)
    })
}
