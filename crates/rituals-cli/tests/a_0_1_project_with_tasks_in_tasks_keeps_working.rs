//! A project laid out the way ritual 0.1 made it, its tasks in `tasks/`,
//! still builds, runs, regenerates and removes its tasks on this version.
//!
//! A task is a workspace member and a path dependency, so ritual never needed
//! it to be in `tasks/`: nothing outside `migrate` may assume a directory
//! name. The project here is committed to git, as `remove` requires before it
//! deletes a task's directory, and is built the way 0.1's `add` built one (see
//! `support::legacy`).

mod support;

use support::legacy::committed_project_with_tasks;
use support::migration::exists;
use support::{
    TempDir, TestOutcome, assert_trees_identical, generated, help, in_checkout, manifest,
    snapshot_tree,
};

#[test]
fn a_project_with_tasks_in_tasks_builds_and_runs_them() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("0-1-builds-and-runs")?;
        let project = committed_project_with_tasks(checkout, &working_dir, &["greet", "shout"])?;

        let listing = project.run_cli(&["--help"])?;
        listing.expect_success("`--help` on a project with tasks in tasks/");
        assert_eq!(
            help::command_names(&listing.stdout),
            [
                "add",
                "regenerate",
                "new",
                "create",
                "import",
                "remove",
                "migrate",
                "greet",
                "shout",
                "help"
            ],
            "stdout was:\n{}",
            listing.stdout
        );
        for task in ["greet", "shout"] {
            let ran = project.run_cli(&[task])?;
            ran.expect_success(&format!("`{task}` from a task in tasks/"));
            assert!(
                ran.stderr.contains(&support::crates::ran_line(task))
                    || ran.stdout.contains(&support::crates::ran_line(task)),
                "expected `{task}` to run its own code; stdout was:\n{}\nstderr was:\n{}",
                ran.stdout,
                ran.stderr
            );
        }
        Ok(())
    })
}

#[test]
fn regenerate_in_a_project_with_tasks_in_tasks_changes_nothing() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("0-1-regenerates")?;
        let project = committed_project_with_tasks(checkout, &working_dir, &["greet"])?;
        let before = snapshot_tree(project.root())?;

        project
            .run_cli(&["regenerate"])?
            .expect_success("`regenerate` on a project with a task in tasks/");

        assert_trees_identical(
            "`regenerate` on an up-to-date project with tasks in tasks/",
            &before,
            &snapshot_tree(project.root())?,
        );
        Ok(())
    })
}

#[test]
fn remove_takes_a_task_out_of_tasks_and_leaves_a_project_that_builds() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("0-1-removes")?;
        let project = committed_project_with_tasks(checkout, &working_dir, &["greet", "shout"])?;

        project
            .run_cli(&["remove", "greet"])?
            .expect_success("`remove greet` on a task in tasks/");

        assert!(
            !exists(&project.root().join("tasks/greet")),
            "expected tasks/greet to be deleted"
        );
        assert!(
            exists(&project.root().join("tasks/shout/Cargo.toml")),
            "expected the other task in tasks/ to be left alone"
        );
        assert_eq!(
            manifest::workspace_members(&project.workspace_manifest()?),
            Some(vec!["ritual".to_string(), "tasks/shout".to_string()]),
            "expected only `tasks/greet` to leave `[workspace] members`"
        );
        assert_eq!(
            generated::mounted_entries(&project.generated_file()?),
            [
                ("ritual".to_string(), "ritual".to_string()),
                ("shout".to_string(), "shout".to_string()),
            ],
            "expected the regenerated command line to mount the bundle and `shout`"
        );
        project
            .run_cli(&["shout"])?
            .expect_success("`shout`, the task left in tasks/ after removing `greet`");
        Ok(())
    })
}

#[test]
fn a_task_added_to_a_project_with_tasks_in_tasks_lives_in_dot_rituals_beside_them() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("0-1-then-add")?;
        let project = committed_project_with_tasks(checkout, &working_dir, &["greet"])?;
        let greet_before = snapshot_tree(&project.root().join("tasks/greet"))?;

        project
            .run_cli(&["add", "shout"])?
            .expect_success("`add shout` on a project whose task is in tasks/");

        assert!(
            exists(&project.root().join(".rituals/shout/Cargo.toml")),
            "expected the new task to be scaffolded into .rituals/shout"
        );
        assert!(
            !exists(&project.root().join("tasks/shout")),
            "expected the new task not to be scaffolded into tasks/"
        );
        assert_trees_identical(
            "the task already in tasks/, after `add` of another",
            &greet_before,
            &snapshot_tree(&project.root().join("tasks/greet"))?,
        );
        assert_eq!(
            manifest::workspace_members(&project.workspace_manifest()?),
            Some(vec![
                "ritual".to_string(),
                "tasks/greet".to_string(),
                ".rituals/shout".to_string(),
            ]),
            "expected the new member after the old"
        );
        for task in ["greet", "shout"] {
            project
                .run_cli(&[task])?
                .expect_success(&format!("`{task}` in a project with both layouts"));
        }
        Ok(())
    })
}
