//! `add` scaffolds a ritual into `.rituals/<name>`, not `tasks/<name>`: every
//! ritual lives in `.rituals/`, whoever it is for.
//!
//! `new` then `add <name>` leaves the task at `.rituals/<name>`, mounted
//! through an explicit `[workspace] members` entry of the same path (never a
//! glob), a dependency whose `path` leads there from the command line crate,
//! and nothing at all in `tasks/`. The command line builds and the task runs.
//! The layout is pinned exactly, so a change to where tasks go is a change to
//! this story.

mod support;

use support::removal::exists;
use support::{Project, help};
use support::{TempDir, TestOutcome, generated, in_checkout, manifest};

#[test]
fn a_task_added_to_a_new_project_lives_in_dot_rituals_and_runs() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("add-into-dot-rituals")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;

        project
            .alias(&["add", "greet"])?
            .expect_success("`cargo ritual add greet`");

        // The task's files are under .rituals/, and tasks/ was never made.
        assert!(
            exists(&project.root().join(".rituals/greet/Cargo.toml")),
            "expected `add greet` to scaffold .rituals/greet/Cargo.toml"
        );
        assert!(
            exists(&project.root().join(".rituals/greet/src/lib.rs")),
            "expected `add greet` to scaffold .rituals/greet/src/lib.rs"
        );
        assert!(
            !exists(&project.root().join("tasks")),
            "expected `add greet` to leave no tasks/ directory in the project"
        );

        // Mounted through an explicit member entry, written as a path.
        let workspace = project.workspace_manifest()?;
        assert_eq!(
            manifest::workspace_members(&workspace),
            Some(vec!["ritual".to_string(), ".rituals/greet".to_string()]),
            "expected `[workspace] members` to list the command line crate and .rituals/greet \
             and nothing else; manifest was:\n{workspace}"
        );

        // The command line crate depends on it by a path that leads there.
        let cli_manifest = project.cli_manifest()?;
        assert_eq!(
            manifest::dependency_path(&cli_manifest, &["dependencies"], "greet"),
            Some("../.rituals/greet"),
            "expected the `greet` dependency to lead to ../.rituals/greet; manifest \
             was:\n{cli_manifest}"
        );
        assert_eq!(
            manifest::tasks(&cli_manifest)?,
            ["ritual", "greet"],
            "expected `greet` to be mounted after the bundle"
        );
        assert_eq!(
            generated::mounted_entries(&project.generated_file()?),
            [
                ("ritual".to_string(), "ritual".to_string()),
                ("greet".to_string(), "greet".to_string()),
            ],
            "expected the generated command line to mount the bundle and `greet`"
        );

        // The command line builds, lists the task, and runs it.
        let listing = project.alias(&["--help"])?;
        listing.expect_success("`cargo ritual --help` after `add greet`");
        assert!(
            help::lists_command(&listing.stdout, "greet"),
            "expected --help to list `greet`; stdout was:\n{}",
            listing.stdout
        );
        project
            .alias(&["greet"])?
            .expect_success("`cargo ritual greet` immediately after `add greet`");
        Ok(())
    })
}

#[test]
fn a_second_added_task_joins_the_first_in_dot_rituals() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("add-two-into-dot-rituals")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        for task in ["greet", "shout"] {
            project
                .alias(&["add", task])?
                .expect_success(&format!("`cargo ritual add {task}`"));
        }

        assert_eq!(
            manifest::workspace_members(&project.workspace_manifest()?),
            Some(vec![
                "ritual".to_string(),
                ".rituals/greet".to_string(),
                ".rituals/shout".to_string(),
            ]),
            "expected one explicit member per task, in the order they were added"
        );
        assert!(
            !exists(&project.root().join("tasks")),
            "expected no tasks/ directory after adding two tasks"
        );
        for task in ["greet", "shout"] {
            project
                .alias(&[task])?
                .expect_success(&format!("`cargo ritual {task}`"));
        }
        Ok(())
    })
}

#[test]
fn new_does_not_write_a_member_glob_for_tasks_to_come() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("new-writes-no-task-glob")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;

        // A glob that matches nothing is read by Cargo as a literal path and
        // stops the workspace loading, so the members of a project with no
        // task yet are the command line crate alone.
        assert_eq!(
            manifest::workspace_members(&project.workspace_manifest()?),
            Some(vec!["ritual".to_string()]),
            "expected a freshly scaffolded project to list its command line crate alone"
        );
        assert!(
            !exists(&project.root().join(".rituals")) && !exists(&project.root().join("tasks")),
            "expected `new` to make neither .rituals/ nor tasks/ before there is a task"
        );
        Ok(())
    })
}
