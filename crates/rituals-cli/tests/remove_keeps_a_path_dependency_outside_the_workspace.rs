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

use support::removal::{exists, members_of};
use support::{
    Project, TempDir, TestOutcome, assert_trees_identical, generated, help, in_checkout, manifest,
    run_ritual, snapshot_tree,
};

#[test]
fn a_task_imported_by_path_from_outside_the_workspace_loses_only_its_dependency_line() -> TestOutcome
{
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-keeps-external-path-dependency")?;
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

        let members_before = members_of(&project)?;
        let crate_before = snapshot_tree(&crate_dir)?;
        assert!(
            crate_before.contains_key(std::path::Path::new("Cargo.toml")),
            "fixture precondition: the external crate should be on disk"
        );

        project
            .run_cli(&["remove", "chore"])?
            .expect_success("`cargo ritual remove chore`, a path dependency outside the workspace");

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

        let help_output = project.alias(&["--help"])?;
        help_output.expect_success("`cargo ritual --help` after `remove chore`");
        assert_eq!(
            help::command_names(&help_output.stdout),
            ["add", "regenerate", "new", "create", "remove", "help"],
            "stdout was:\n{}",
            help_output.stdout
        );
        Ok(())
    })
}
