//! Inside a project, `create <name>` does what `add <name>` did in 0.1, in
//! `.rituals/`: the task's two files, an explicit `[workspace] members`
//! entry, a dependency and a tasks entry on the command line crate, and a
//! regenerated command line that builds and runs the task.
//!
//! A ritual is private unless it says otherwise: its `[package]` carries
//! `publish = false`. `--public` leaves that key out and changes nothing
//! else.

mod support;

use support::created::{
    Audience, Expected, assert_the_task_builds_and_runs, assert_the_task_is_scaffolded,
    report_in_a_default_project, stdout_lines,
};
use support::removal::exists;
use support::{Project, TempDir, TestOutcome, help, in_checkout, manifest, run_binary};

/// `create lint` in a fresh project with the given audience flags: asserts
/// the whole tree and the report, then that the task builds and runs.
fn assert_create_inside_a_fresh_project(
    audience: Audience,
    working_dir_prefix: &str,
) -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new(working_dir_prefix)?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let binary = project.build()?;

        let mut arguments = vec!["create", "lint"];
        arguments.extend_from_slice(audience.flags());
        let created = run_binary(&binary, project.root(), &arguments)?;
        created.expect_success(&format!("`ritual {}`", arguments.join(" ")));

        assert_eq!(
            stdout_lines(&created),
            report_in_a_default_project(".rituals/lint", "lint", &["ritual", "lint"])
        );
        assert_the_task_is_scaffolded(
            &project,
            &Expected {
                directory: ".rituals/lint",
                name: "lint",
                audience,
                members: &["ritual", ".rituals/lint"],
                tasks: &["ritual", "lint"],
                dependency_path: "../.rituals/lint",
            },
        )?;
        assert!(
            !exists(&project.root().join("tasks")),
            "expected no tasks/ directory"
        );
        assert_the_task_builds_and_runs(&project, "lint")
    })
}

#[test]
fn a_private_ritual_is_scaffolded_into_dot_rituals_and_runs() -> TestOutcome {
    assert_create_inside_a_fresh_project(Audience::Private, "create-private-in-project")
}

#[test]
fn a_public_ritual_is_the_same_tree_without_the_publish_key() -> TestOutcome {
    assert_create_inside_a_fresh_project(Audience::Public, "create-public-in-project")
}

/// Reached through the project's own `cargo ritual` alias, as a person
/// types it, a second `create` adds to what the first left.
#[test]
fn a_second_ritual_joins_the_first_in_the_member_list_and_the_task_list() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("create-two-in-project")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        for name in ["greet", "shout"] {
            project
                .alias(&["create", name])?
                .expect_success(&format!("`cargo ritual create {name}`"));
        }

        assert_the_task_is_scaffolded(
            &project,
            &Expected {
                directory: ".rituals/shout",
                name: "shout",
                audience: Audience::Private,
                members: &["ritual", ".rituals/greet", ".rituals/shout"],
                tasks: &["ritual", "greet", "shout"],
                dependency_path: "../.rituals/shout",
            },
        )?;
        assert!(
            !exists(&project.root().join("tasks")),
            "expected no tasks/ directory after creating two rituals"
        );
        let listing = project.alias(&["--help"])?;
        listing.expect_success("`cargo ritual --help` after creating two rituals");
        for name in ["greet", "shout"] {
            assert!(
                help::lists_command(&listing.stdout, name),
                "expected --help to list `{name}`; stdout was:\n{}",
                listing.stdout
            );
            assert_the_task_builds_and_runs(&project, name)?;
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
