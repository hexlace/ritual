//! `migrate` moves the tasks this project's own command line imports, the
//! ones its `[package.metadata.ritual] tasks` list names, and nothing else
//! that happens to live in `tasks/`.
//!
//! A bundle crate that a command line imports whole has its children in
//! `tasks/` without the project having imported them, so they are not the
//! project's own tasks to move. A task that only another task depends on is
//! in the same position: it stays, and the path to it follows whichever
//! task reached it.

mod support;

use support::crates::{Child, bundle_lib, bundle_manifest, leaf_lib, leaf_manifest, write_crate};
use support::migration::{
    assert_dependency_leads_to, assert_names, assert_nothing_to_migrate, exists,
};
use support::{Project, assert_trees_identical};
use support::{TempDir, TestOutcome, git, in_checkout, manifest, snapshot_tree};

/// A project whose command line imports one bundle that lives outside
/// `tasks/`, while the bundle's children, `hello` and `wave`, are in
/// `tasks/` as workspace members, as a bundle of tasks put there by its own
/// author would have them.
fn project_importing_a_bundle(
    checkout: &support::Checkout,
    working_dir: &TempDir,
) -> support::Outcome<Project> {
    let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
    project.write_leaf("hello")?;
    project.write_leaf("wave")?;
    let children = [
        Child::Crate {
            key: "hello",
            crate_name: "hello",
        },
        Child::Crate {
            key: "wave",
            crate_name: "wave",
        },
    ];
    let bundle_directory = project.root().join("bundle");
    write_crate(
        &bundle_directory,
        &bundle_manifest("bundle", &children).replace("path = \"../", "path = \"../tasks/"),
        &bundle_lib("a bundle over tasks kept in tasks/", &children),
    )?;
    manifest::edit(&project.workspace_manifest_path(), |document| {
        manifest::push_member(document, "bundle")
    })?;
    project.mount(&bundle_directory, "bundle", "bundle")?;
    project
        .run_cli(&["regenerate"])?
        .expect_success("`regenerate` after mounting the bundle");
    project.build()?;
    git::init_and_commit_everything(project.root())?;
    Ok(project)
}

#[test]
fn a_bundle_repository_whose_children_live_in_tasks_has_nothing_to_migrate() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-bundle-children")?;
        let project = project_importing_a_bundle(checkout, &working_dir)?;
        let before = snapshot_tree(project.root())?;

        let migrated = project.run_cli(&["migrate"])?;

        assert_nothing_to_migrate(&migrated, "`migrate` on a project importing only a bundle");
        assert_trees_identical(
            "a project whose command line imports none of the tasks in tasks/ must be left alone",
            &before,
            &snapshot_tree(project.root())?,
        );
        Ok(())
    })
}

#[test]
fn a_task_only_another_task_uses_stays_in_tasks_and_the_path_to_it_follows() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-dependency-only")?;
        let project = support::legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        write_crate(
            &project.root().join("tasks/helper"),
            &leaf_manifest("helper"),
            &leaf_lib("helper"),
        )?;
        manifest::edit(&project.workspace_manifest_path(), |document| {
            manifest::push_member_on_its_own_line(document, "tasks/helper")
        })?;
        manifest::edit(&project.root().join("tasks/greet/Cargo.toml"), |document| {
            manifest::add_path_dependency(document, &["dependencies"], "helper", "../helper")
        })?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;

        let migrated = project.run_cli(&["migrate"])?;

        migrated.expect_success("`migrate` with a task only another task uses");
        assert!(
            exists(&project.root().join(".rituals/greet/Cargo.toml")),
            "expected the imported task to move to .rituals/greet"
        );
        assert!(
            exists(&project.root().join("tasks/helper/Cargo.toml")),
            "expected the task nothing imports to stay in tasks/helper"
        );
        assert!(
            !exists(&project.root().join(".rituals/helper")),
            "expected the task nothing imports not to be moved"
        );
        assert_dependency_leads_to(
            &project.root().join(".rituals/greet/Cargo.toml"),
            &["dependencies"],
            "helper",
            &project.root().join("tasks/helper"),
        )?;
        assert_names(&migrated, "`migrate`", &["kept tasks/"]);
        project.build()?;
        Ok(())
    })
}
