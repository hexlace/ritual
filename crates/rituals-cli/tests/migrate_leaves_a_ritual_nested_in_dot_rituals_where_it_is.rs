//! A ritual anywhere beneath `.rituals/` is already in the layout `migrate`
//! brings a project to, so `migrate` does not touch it: a project whose
//! rituals are all nested there has nothing to migrate and is left byte for
//! byte as it was, and a project that also holds a ritual in `tasks/` moves
//! that one and only that one.
//!
//! The fixtures are committed to a git repository, because `migrate` only
//! runs where git can give everything back.

mod support;

use support::migration::{assert_nothing_to_migrate, exists};
use support::nested::mounted_ritual;
use support::{
    Project, TempDir, TestOutcome, assert_trees_identical, git, in_checkout, legacy, snapshot_tree,
};

#[test]
fn a_project_whose_rituals_are_all_nested_in_dot_rituals_has_nothing_to_migrate() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-all-nested")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        mounted_ritual(&project, ".rituals/private/lint", "lint")?;
        mounted_ritual(&project, ".rituals/a/b/greet", "greet")?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;
        let before = snapshot_tree(project.root())?;

        let migrated = project.run_cli(&["migrate"])?;

        assert_nothing_to_migrate(&migrated, "`migrate` on a project with nested rituals only");
        assert_trees_identical(
            "a project whose rituals are all in .rituals/ must be left alone",
            &before,
            &snapshot_tree(project.root())?,
        );
        Ok(())
    })
}

#[test]
fn a_nested_ritual_stays_untouched_while_a_task_in_tasks_moves() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-mixed-nested")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["old"])?;
        mounted_ritual(&project, ".rituals/x/y", "nested")?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;
        let nested_before = snapshot_tree(&project.root().join(".rituals/x/y"))?;

        project
            .run_cli(&["migrate"])?
            .expect_success("`migrate` with a task in tasks/ and a ritual nested in .rituals/");

        assert!(
            exists(&project.root().join(".rituals/old/Cargo.toml")),
            "expected the task in tasks/ to move to .rituals/old"
        );
        assert!(
            !exists(&project.root().join("tasks")),
            "expected no tasks/ to be left once its only task has moved"
        );
        assert_trees_identical(
            "the ritual nested in .rituals/x/y",
            &nested_before,
            &snapshot_tree(&project.root().join(".rituals/x/y"))?,
        );
        for (command, line) in [("old", "old ran"), ("nested", "nested ran")] {
            let ran = project.run_cli(&[command])?;
            ran.expect_success(&format!("the `{command}` command, after `migrate`"));
            assert!(
                ran.stdout.contains(line),
                "expected `{command}` to answer `{line}`; stdout was:\n{}",
                ran.stdout
            );
        }
        Ok(())
    })
}
