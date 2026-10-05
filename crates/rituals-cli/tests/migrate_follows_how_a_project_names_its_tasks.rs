//! `migrate` moves the tasks and leaves the project's own choices alone: a
//! `members` glob stays a glob, an explicit list stays explicit, and
//! whatever in `tasks/` is not a task stays where it is, with `tasks/` left
//! in place for it and the output saying what is left.
//!
//! The path of every dependency on a moved task changes in every manifest
//! that has one, not only the command line crate's: a member outside
//! `tasks/`, a task left behind, and the workspace's own shared dependencies
//! and default members all lead to where the task is now.

mod support;

use support::crates::write_crate;
use support::migration::{assert_dependency_leads_to, assert_names, exists};
use support::{Project, TempDir, TestOutcome, git, in_checkout, legacy, manifest, snapshot_tree};

/// A crate that is not a task: no `[package.metadata.ritual]`, one library
/// source, and whatever `dependencies` (TOML lines) it declares.
fn write_plain_crate(directory: &std::path::Path, name: &str, dependencies: &str) -> TestOutcome {
    write_crate(
        directory,
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [dependencies]\n{dependencies}"
        ),
        "//! A crate that is not a task.\n",
    )
}

/// Runs `migrate` and asserts it succeeded and that the project builds as a
/// whole afterwards, every member included.
#[track_caller]
fn migrate_and_assert_the_workspace_builds(
    project: &Project,
) -> support::Outcome<support::RunOutput> {
    let migrated = project.run_cli(&["migrate"])?;
    migrated.expect_success("`migrate`");
    project
        .cargo(&["build", "--workspace"])?
        .expect_success("`cargo build --workspace` after `migrate`");
    Ok(migrated)
}

#[test]
fn a_members_glob_over_tasks_becomes_a_glob_over_dot_rituals() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-glob")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet", "shout"])?;
        manifest::edit(&project.workspace_manifest_path(), |document| {
            manifest::set_strings(document, &["workspace", "members"], &["ritual", "tasks/*"])
        })?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;

        migrate_and_assert_the_workspace_builds(&project)?;

        assert_eq!(
            manifest::workspace_members(&project.workspace_manifest()?),
            Some(vec!["ritual".to_string(), ".rituals/*".to_string()]),
            "expected the glob to stay a glob, over the new directory"
        );
        assert!(
            !exists(&project.root().join("tasks")),
            "expected tasks/ to be removed once nothing was left in it"
        );
        for task in ["greet", "shout"] {
            project
                .run_cli(&[task])?
                .expect_success(&format!("`{task}` after `migrate` over a members glob"));
        }
        Ok(())
    })
}

#[test]
fn an_explicit_members_list_stays_explicit_and_keeps_its_order() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-explicit")?;
        // A member the person listed between the tasks, to show each entry is
        // changed where it stands.
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet", "shout"])?;
        write_plain_crate(&project.root().join("tools/lint"), "lint", "")?;
        manifest::edit(&project.workspace_manifest_path(), |document| {
            manifest::set_strings(
                document,
                &["workspace", "members"],
                &["ritual", "tasks/greet", "tools/lint", "tasks/shout"],
            )
        })?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;

        migrate_and_assert_the_workspace_builds(&project)?;

        assert_eq!(
            manifest::workspace_members(&project.workspace_manifest()?),
            Some(
                ["ritual", ".rituals/greet", "tools/lint", ".rituals/shout"]
                    .map(str::to_string)
                    .to_vec()
            ),
            "expected each task's entry to change in place and the other members to be left alone"
        );
        Ok(())
    })
}

/// Holds a project that kept `tasks/helper`, which is not a task, to what
/// `migrate` leaves alone: the task `greet` is at `.rituals/greet` and gone
/// from `tasks/greet`, `tasks/helper` is where it was, and the workspace's
/// members changed only in the task's own entry.
fn assert_only_the_task_moved(project: &Project) -> TestOutcome {
    assert!(
        exists(&project.root().join(".rituals/greet/Cargo.toml")),
        "expected the task to move to .rituals/greet"
    );
    assert!(
        !exists(&project.root().join("tasks/greet")),
        "expected the task to leave tasks/greet"
    );
    assert!(
        exists(&project.root().join("tasks/helper/Cargo.toml")),
        "expected tasks/helper, which is not a task, to stay where it is"
    );
    assert_eq!(
        manifest::workspace_members(&project.workspace_manifest()?),
        Some(
            ["ritual", ".rituals/greet", "tasks/helper"]
                .map(str::to_string)
                .to_vec()
        ),
        "expected only the task's member entry to change"
    );
    Ok(())
}

#[test]
fn something_in_tasks_that_is_not_a_task_stays_and_is_named() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-leaves-a-non-task")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        // A plain crate the person keeps in tasks/, which uses the task.
        write_plain_crate(
            &project.root().join("tasks/helper"),
            "helper",
            "greet = { path = \"../greet\" }\n",
        )?;
        manifest::edit(&project.workspace_manifest_path(), |document| {
            manifest::push_member(document, "tasks/helper")
        })?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;
        let helper_before = snapshot_tree(&project.root().join("tasks/helper"))?;

        let migrated = migrate_and_assert_the_workspace_builds(&project)?;

        assert_only_the_task_moved(&project)?;

        // What stayed still reaches the task where it went.
        assert_dependency_leads_to(
            &project.root().join("tasks/helper/Cargo.toml"),
            &["dependencies"],
            "greet",
            &project.root().join(".rituals/greet"),
        )?;
        let helper_after = snapshot_tree(&project.root().join("tasks/helper"))?;
        assert_eq!(
            support::tree::changed_paths(&helper_before, &helper_after),
            [std::path::PathBuf::from("Cargo.toml")],
            "expected only the helper's manifest to change, and only for the dependency's path"
        );

        // The output says what moved, what was edited and what is left.
        assert_names(
            &migrated,
            "`migrate`",
            &[
                "tasks/greet",
                ".rituals/greet",
                "tasks/helper/Cargo.toml",
                "tasks/helper",
            ],
        );
        project
            .run_cli(&["greet"])?
            .expect_success("`greet` after `migrate` left a non-task in tasks/");
        Ok(())
    })
}

#[test]
fn a_member_outside_tasks_that_depends_on_a_moved_task_is_repointed() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-repoints-a-member")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        write_plain_crate(
            &project.root().join("tools/helper"),
            "helper",
            "greet = { path = \"../../tasks/greet\" }\n",
        )?;
        manifest::edit(&project.workspace_manifest_path(), |document| {
            manifest::push_member(document, "tools/helper")
        })?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;

        let migrated = migrate_and_assert_the_workspace_builds(&project)?;

        assert_dependency_leads_to(
            &project.root().join("tools/helper/Cargo.toml"),
            &["dependencies"],
            "greet",
            &project.root().join(".rituals/greet"),
        )?;
        assert_names(&migrated, "`migrate`", &["tools/helper/Cargo.toml"]);
        Ok(())
    })
}

#[test]
fn a_shared_workspace_dependency_on_a_moved_task_is_repointed() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-repoints-workspace-dependency")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        manifest::edit(&project.workspace_manifest_path(), |document| {
            manifest::add_path_dependency(
                document,
                &["workspace", "dependencies"],
                "greet",
                "tasks/greet",
            )
        })?;
        write_plain_crate(
            &project.root().join("tools/helper"),
            "helper",
            "greet.workspace = true\n",
        )?;
        manifest::edit(&project.workspace_manifest_path(), |document| {
            manifest::push_member(document, "tools/helper")
        })?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;

        migrate_and_assert_the_workspace_builds(&project)?;

        assert_dependency_leads_to(
            &project.workspace_manifest_path(),
            &["workspace", "dependencies"],
            "greet",
            &project.root().join(".rituals/greet"),
        )?;
        Ok(())
    })
}

#[test]
fn a_default_member_that_is_a_moved_task_follows_it() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-default-members")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        manifest::edit(&project.workspace_manifest_path(), |document| {
            document["workspace"]["default-members"] = toml_edit::value(
                ["ritual", "tasks/greet"]
                    .into_iter()
                    .collect::<toml_edit::Array>(),
            );
            Ok(())
        })?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;

        migrate_and_assert_the_workspace_builds(&project)?;

        let workspace = project.workspace_manifest()?;
        let default_members: Option<Vec<String>> =
            manifest::lookup(&workspace, &["workspace", "default-members"])
                .and_then(toml_edit::Item::as_array)
                .map(|array| {
                    array
                        .iter()
                        .filter_map(|member| member.as_str().map(str::to_string))
                        .collect()
                });
        assert_eq!(
            default_members,
            Some(vec!["ritual".to_string(), ".rituals/greet".to_string()]),
            "expected `default-members` to name the task where it went; manifest was:\n{workspace}"
        );
        Ok(())
    })
}
