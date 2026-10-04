//! A task whose dependency points at a crate outside this project's
//! workspace is one the project borrows, not one it owns: `remove` drops its
//! `tasks` entry and its dependency line and leaves the crate's directory
//! exactly as it found it.
//!
//! The fixture is a standalone crate `ritual create` made in a directory of
//! its own, imported by `path`. The project is not a git repository, which
//! shows the guards about git are about directories `remove` deletes — there
//! is nothing here to delete.

mod support;

use std::path::PathBuf;

use support::removal::{assert_help_lists, exists, members_of};
use support::{
    Checkout, Outcome, Project, TempDir, TestOutcome, assert_trees_identical, generated,
    in_checkout, manifest, run_ritual, snapshot_tree,
};

#[test]
fn a_task_imported_from_outside_the_workspace_loses_only_its_dependency_line() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-keeps-external-path-dependency")?;
        let (project, crate_dir) =
            a_project_importing_a_crate_from_outside(checkout, &working_dir)?;

        let members_before = members_of(&project)?;
        let crate_before = snapshot_tree(&crate_dir)?;
        assert!(
            crate_before.contains_key(std::path::Path::new("Cargo.toml")),
            "fixture precondition: the external crate should be on disk"
        );

        project
            .run_cli(&["remove", "chore"])?
            .expect_success("`cargo ritual remove chore`, a path dependency outside the workspace");

        assert_the_chore_key_and_dependency_line_are_gone(&project)?;
        assert_eq!(
            generated::mounted_entries(&project.generated_file()?),
            [("ritual".to_string(), "ritual".to_string())],
            "expected the regenerated file to mount the bundle alone"
        );
        assert_eq!(
            members_of(&project)?,
            members_before,
            "expected the workspace members to be left alone: the crate was never one"
        );

        assert!(
            exists(&crate_dir),
            "expected the external crate's directory to stay"
        );
        assert_trees_identical(
            "`remove` must leave a crate outside the workspace exactly as it was",
            &crate_before,
            &snapshot_tree(&crate_dir)?,
        );

        assert_help_lists(
            &project,
            "`remove chore`",
            &[
                "add",
                "regenerate",
                "new",
                "create",
                "import",
                "remove",
                "migrate",
                "help",
            ],
        )
    })
}

/// A project that imports, by `path`, a standalone `chore` crate that
/// `ritual create` made in a directory of its own; returns the project and
/// that crate's directory.
fn a_project_importing_a_crate_from_outside(
    checkout: &Checkout,
    working_dir: &TempDir,
) -> Outcome<(Project, PathBuf)> {
    let work = working_dir.path().join("work");
    std::fs::create_dir(&work)?;
    let project = Project::scaffold(checkout, &work, "demo", &[])?;

    let external = working_dir.path().join("external");
    std::fs::create_dir(&external)?;
    run_ritual(
        &external,
        &["create", "chore", "--path", checkout.path_argument()?],
    )?
    .expect_success("`ritual create chore --path <checkout>`");
    let crate_dir = external.join("chore");

    project.mount(&crate_dir, "chore", "chore")?;
    project
        .alias(&["regenerate"])?
        .expect_success("`cargo ritual regenerate` after importing the external crate");
    Ok((project, crate_dir))
}

/// `chore` has left `tasks`, and its dependency line with it.
#[track_caller]
fn assert_the_chore_key_and_dependency_line_are_gone(project: &Project) -> TestOutcome {
    let cli_manifest = project.cli_manifest()?;
    assert_eq!(
        manifest::tasks(&cli_manifest)?,
        ["ritual"],
        "expected `chore` to leave `tasks`; manifest was:\n{cli_manifest}"
    );
    assert!(
        manifest::lookup(&cli_manifest, &["dependencies", "chore"]).is_none(),
        "expected the `chore` dependency line to be gone; manifest was:\n{cli_manifest}"
    );
    Ok(())
}
