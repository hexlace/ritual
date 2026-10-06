//! A project can keep its command line crate at the workspace root: one
//! `Cargo.toml` holding the command line's `[package]` and the `[workspace]`.
//! `create` has two manifests to edit, the workspace's member list and the
//! command line's dependency and task list, and they are one file. Both
//! edits land, the file is written and reported once, and the task builds
//! and runs.
//!
//! The same story runs through `add` for the same reason: it is `create`'s
//! in-project path under another name.

mod support;

use support::created::{
    Audience, Expected, assert_the_task_builds_and_runs, assert_the_task_is_scaffolded,
    stdout_lines,
};
use support::root_cli::fold_the_command_line_into_the_root;
use support::{Project, TempDir, TestOutcome, in_checkout, manifest, run_binary};

/// A scaffolded project with its command line crate folded into the root,
/// and the command line built.
fn project_with_its_command_line_at_the_root(
    checkout: &support::Checkout,
    prefix: &str,
) -> support::Outcome<(TempDir, Project, std::path::PathBuf)> {
    let working_dir = TempDir::new(prefix)?;
    let scaffolded = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
    let project = fold_the_command_line_into_the_root(&scaffolded)?;
    assert_eq!(
        project.cli_manifest_path(),
        project.workspace_manifest_path(),
        "fixture precondition: the command line crate is the workspace root"
    );
    let binary = project.build()?;
    Ok((working_dir, project, binary))
}

fn assert_both_edits_land_and_the_task_runs(verb: &str, prefix: &str) -> TestOutcome {
    in_checkout(|checkout| {
        let (_working_dir, project, binary) =
            project_with_its_command_line_at_the_root(checkout, prefix)?;

        let created = run_binary(&binary, project.root(), &[verb, "lint"])?;
        created.expect_success(&format!(
            "`ritual {verb} lint` with the command line at the root"
        ));

        assert_eq!(
            stdout_lines(&created),
            [
                "created .rituals/lint/Cargo.toml",
                "created .rituals/lint/src/lib.rs",
                "updated Cargo.toml",
                "updated src/main.rs (tasks: ritual, lint)",
                "next: edit .rituals/lint/src/lib.rs, then run cargo ritual lint",
            ],
            "the one manifest is reported once"
        );
        let root_manifest = project.workspace_manifest()?;
        assert_eq!(
            manifest::workspace_members(&root_manifest),
            Some(vec![".rituals/lint".to_string()]),
            "the member entry must land; root manifest was:\n{root_manifest}"
        );
        assert_the_task_is_scaffolded(
            &project,
            &Expected {
                directory: ".rituals/lint",
                name: "lint",
                audience: Audience::Private,
                members: &[".rituals/lint"],
                tasks: &["ritual", "lint"],
                dependency_path: ".rituals/lint",
            },
        )?;
        assert_the_task_builds_and_runs(&project, "lint")
    })
}

#[test]
fn create_with_the_command_line_at_the_workspace_root_lands_both_edits() -> TestOutcome {
    assert_both_edits_land_and_the_task_runs("create", "create-root-command-line")
}

#[test]
fn add_with_the_command_line_at_the_workspace_root_lands_both_edits() -> TestOutcome {
    assert_both_edits_land_and_the_task_runs("add", "add-root-command-line")
}
