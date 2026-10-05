//! `migrate` moves every task in `tasks/`, whether or not the project's own
//! command line imports it: a workspace member under `tasks/` that declares
//! itself a task is in the 0.1 layout, and the 0.2 layout has no `tasks/`.
//!
//! A bundle crate that a command line imports whole has its children in
//! `tasks/` without the project having imported them; they move with the
//! rest, and the bundle's paths to them follow. A task that only another
//! task uses moves too, with the dependent's path repointed. Nothing is
//! left behind, so no `kept tasks/` line is written and `tasks/` is gone.

mod support;

use support::Project;
use support::crates::{Child, bundle_lib, bundle_manifest, leaf_lib, leaf_manifest, write_crate};
use support::migration::{assert_dependency_leads_to, assert_names, everything_written, exists};
use support::{TempDir, TestOutcome, git, in_checkout, manifest};

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
fn a_bundle_repositorys_children_move_from_tasks_and_the_bundle_follows() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-bundle-children")?;
        let project = project_importing_a_bundle(checkout, &working_dir)?;

        let migrated = project.run_cli(&["migrate"])?;

        migrated.expect_success("`migrate` on a project importing only a bundle");
        for child in ["hello", "wave"] {
            assert!(
                exists(&project.root().join(format!(".rituals/{child}/Cargo.toml"))),
                "expected the bundle's child `{child}` to move to .rituals/{child}"
            );
            assert_dependency_leads_to(
                &project.root().join("bundle/Cargo.toml"),
                &["dependencies"],
                child,
                &project.root().join(format!(".rituals/{child}")),
            )?;
        }
        assert!(
            !exists(&project.root().join("tasks")),
            "expected no tasks/ to be left once every task in it has moved"
        );
        assert_names(&migrated, "`migrate`", &["hello", "wave"]);
        let written = everything_written(&migrated);
        assert!(
            !written.contains("kept tasks/"),
            "nothing is left in tasks/, so nothing is kept; it wrote:\n{written}"
        );

        let ran = project.run_cli(&["bundle", "hello"])?;
        ran.expect_success("the bundle's child, after the move");
        assert!(
            ran.stdout.contains("hello ran"),
            "expected the moved child to answer; stdout was:\n{}",
            ran.stdout
        );
        Ok(())
    })
}

#[test]
fn a_task_only_another_task_uses_moves_too_and_the_path_to_it_follows() -> TestOutcome {
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
            exists(&project.root().join(".rituals/helper/Cargo.toml")),
            "expected the task only another task uses to move to .rituals/helper"
        );
        assert!(
            !exists(&project.root().join("tasks")),
            "expected no tasks/ to be left once every task in it has moved"
        );
        assert_dependency_leads_to(
            &project.root().join(".rituals/greet/Cargo.toml"),
            &["dependencies"],
            "helper",
            &project.root().join(".rituals/helper"),
        )?;
        let written = everything_written(&migrated);
        assert!(
            !written.contains("kept tasks/"),
            "nothing is left in tasks/, so nothing is kept; it wrote:\n{written}"
        );
        project.build()?;
        Ok(())
    })
}
