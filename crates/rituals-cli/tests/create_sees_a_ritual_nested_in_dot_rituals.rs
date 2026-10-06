//! `create` checks the project's existing rituals before it writes anything,
//! and a ritual nested below `.rituals/` at any depth is as much an existing
//! ritual as one at `.rituals/<name>`: ritual reads no meaning into the
//! directory names between.
//!
//! The fixtures are projects `new` scaffolded, with rituals written beneath
//! `.rituals/private/` and `.rituals/a/b/` the way a person who groups
//! their rituals would have them.

mod support;

use support::nested::{mounted_ritual, write_ritual};
use support::removal::{assert_a_refusal, exists};
use support::{Project, TempDir, TestOutcome, assert_trees_identical, in_checkout, snapshot_tree};

#[test]
fn a_name_a_nested_ritual_already_provides_is_refused_as_already_a_task() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("create-nested-listed")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        mounted_ritual(&project, ".rituals/private/lint", "lint")?;
        project.build()?;
        let before = snapshot_tree(project.root())?;

        let created = project.run_cli(&["create", "lint"])?;

        let message = assert_a_refusal(
            &created,
            "ritual",
            "`create lint`, which a nested ritual provides",
        );
        assert!(
            message.contains("`lint` is already a task of `demo-ritual`"),
            "expected the refusal to say `lint` is already a task; stderr was:\n{message}"
        );
        assert_trees_identical(
            "a refused `create lint` must write nothing",
            &before,
            &snapshot_tree(project.root())?,
        );
        Ok(())
    })
}

#[test]
fn a_deeply_nested_ritual_the_command_line_does_not_list_still_holds_its_name() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("create-nested-unlisted")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        write_ritual(&project, ".rituals/a/b/greet", "greet")?;
        project.build()?;
        let before = snapshot_tree(project.root())?;

        let created = project.run_cli(&["create", "greet"])?;

        let message = assert_a_refusal(
            &created,
            "ritual",
            "`create greet`, a package a nested ritual is",
        );
        assert!(
            message.contains("already has a package called `greet`"),
            "expected the refusal to say the workspace already has a package called `greet`; \
             stderr was:\n{message}"
        );
        assert_trees_identical(
            "a refused `create greet` must write nothing",
            &before,
            &snapshot_tree(project.root())?,
        );
        Ok(())
    })
}

#[test]
fn a_new_name_is_created_beside_nested_rituals_and_the_project_builds() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("create-beside-nested")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        mounted_ritual(&project, ".rituals/private/lint", "lint")?;
        mounted_ritual(&project, ".rituals/a/b/greet", "greet")?;

        project
            .run_cli(&["create", "tidy"])?
            .expect_success("`create tidy` beside rituals nested in .rituals/");

        assert!(
            exists(&project.root().join(".rituals/tidy/Cargo.toml")),
            "expected `create tidy` to scaffold .rituals/tidy"
        );
        for (command, line) in [("lint", "lint ran"), ("greet", "greet ran")] {
            let ran = project.run_cli(&[command])?;
            ran.expect_success(&format!("the nested ritual's `{command}` command"));
            assert!(
                ran.stdout.contains(line),
                "expected `{command}` to answer `{line}`; stdout was:\n{}",
                ran.stdout
            );
        }
        Ok(())
    })
}
