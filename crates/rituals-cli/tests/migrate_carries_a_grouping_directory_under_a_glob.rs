//! A 0.1 project that grouped tasks in a directory under `tasks/`, named
//! the group's tasks one by one and kept the group directory out of its
//! `tasks/*` glob with `exclude`, migrates as a whole: the group's tasks
//! move into the same group under `.rituals/`, the `exclude` follows them,
//! and the glob is replaced, because nothing it matched stays behind.
//!
//! The project is the one Cargo loads in 0.1: `tasks/*` matches `tasks/greet`
//! and `tasks/group`, `tasks/group` is excluded, and `tasks/group/lint` is a
//! member by name. A crate outside `tasks/` depends on `lint` by path.
//!
//! The same project migrates whether or not `.rituals/group` is already
//! excluded beside `tasks/group`, and after either run every `cargo` command
//! still reads the workspace.

mod support;

use support::crates::{leaf_lib, leaf_manifest, write_crate};
use support::migration::{assert_dependency_leads_to, assert_names, everything_written, exists};
use support::{
    Checkout, Outcome, Project, RunOutput, TempDir, TestOutcome, git, in_checkout, legacy,
    manifest, nested,
};

/// The members of the grouped 0.1 project, in its own order.
const MEMBERS: [&str; 4] = ["ritual", "tasks/*", "tools/user", "tasks/group/lint"];

/// The grouped 0.1 project, committed, with `exclude` holding `excluded`.
///
/// `greet` sits directly under `tasks/` and `lint` in `tasks/group/`; the
/// command line mounts both, and `tools/user`, a crate that is not a task,
/// depends on `lint`.
fn grouped_project(
    checkout: &Checkout,
    working_dir: &TempDir,
    excluded: &[&str],
) -> Outcome<Project> {
    let project = legacy::project_with_tasks(checkout, working_dir, &[])?;
    for (directory, task) in [("tasks/greet", "greet"), ("tasks/group/lint", "lint")] {
        write_crate(
            &project.root().join(directory),
            &leaf_manifest(task),
            &leaf_lib(task),
        )?;
        nested::list_as_a_task(&project, directory, task)?;
    }
    write_crate(
        &project.root().join("tools/user"),
        "[package]\nname = \"user\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
         [dependencies]\nlint = { path = \"../../tasks/group/lint\" }\n",
        "//! A crate that is not a task, and uses one.\n",
    )?;
    manifest::edit(&project.workspace_manifest_path(), |document| {
        manifest::set_strings(document, &["workspace", "members"], &MEMBERS)?;
        manifest::set_workspace_exclude(document, excluded)
    })?;
    project
        .run_cli(&["regenerate"])?
        .expect_success("`regenerate` in the grouped 0.1 project");
    project.build()?;
    git::init_and_commit_everything(project.root())?;
    Ok(project)
}

/// `migrate` succeeded, Cargo reads the workspace afterwards, and every task
/// runs from its new place.
#[track_caller]
fn assert_migrated_and_loadable(project: &Project) -> Outcome<RunOutput> {
    let migrated = project.run_cli(&["migrate"])?;
    migrated.expect_success("`migrate` on the grouped 0.1 project");
    project
        .cargo(&["metadata", "--format-version", "1", "--no-deps"])?
        .expect_success("`cargo metadata` after `migrate`");
    for task in ["greet", "lint"] {
        project
            .run_cli(&[task])?
            .expect_success(&format!("`{task}` after `migrate`"));
    }
    Ok(migrated)
}

/// What both runs leave: the glob replaced, the group's task moved into the
/// group under `.rituals/`, `exclude` holding only the new group, `tasks/`
/// gone, and the crate that uses `lint` reaching it where it went.
fn assert_the_group_moved_whole(project: &Project) -> TestOutcome {
    let document = project.workspace_manifest()?;
    assert_eq!(
        manifest::workspace_members(&document),
        Some(
            ["ritual", ".rituals/*", "tools/user", ".rituals/group/lint"]
                .map(str::to_string)
                .to_vec()
        ),
        "expected the glob to be replaced, since nothing it matched stays, and the group's task \
         to be repointed in place"
    );
    let excluded: Vec<String> = manifest::lookup(&document, &["workspace", "exclude"])
        .and_then(toml_edit::Item::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(
        excluded,
        [".rituals/group"],
        "expected `exclude` to follow the group to .rituals/group and keep no entry for a \
         directory that is gone"
    );
    assert!(
        exists(&project.root().join(".rituals/group/lint/Cargo.toml")),
        "expected lint to move into the group under .rituals/"
    );
    assert!(
        !exists(&project.root().join("tasks")),
        "expected tasks/ to be removed once nothing was left in it"
    );
    assert_dependency_leads_to(
        &project.root().join("tools/user/Cargo.toml"),
        &["dependencies"],
        "lint",
        &project.root().join(".rituals/group/lint"),
    )
}

#[test]
fn a_group_excluded_from_the_glob_moves_and_its_exclude_follows() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-group-under-a-glob")?;
        let project = grouped_project(checkout, &working_dir, &["tasks/group"])?;

        let migrated = assert_migrated_and_loadable(&project)?;

        assert_the_group_moved_whole(&project)?;
        assert_names(
            &migrated,
            "`migrate`",
            &[
                "tasks/group/lint",
                ".rituals/group/lint",
                "`tasks/*` is now `.rituals/*`",
                "`tasks/group` is now `.rituals/group`",
            ],
        );
        Ok(())
    })
}

/// The readme's advice for a group that is already under `.rituals/` is to
/// exclude it; a person who did that before migrating gets the same project
/// as one who did not, and no glob left matching nothing.
#[test]
fn a_group_already_excluded_at_its_new_place_moves_the_same_way() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-group-already-excluded")?;
        let project = grouped_project(checkout, &working_dir, &["tasks/group", ".rituals/group"])?;

        let migrated = assert_migrated_and_loadable(&project)?;

        assert_the_group_moved_whole(&project)?;
        assert_names(
            &migrated,
            "`migrate`",
            &[
                "tasks/group/lint",
                ".rituals/group/lint",
                "`tasks/*` is now `.rituals/*`",
                "`tasks/group`",
            ],
        );
        let written = everything_written(&migrated);
        assert!(
            !written.contains("keeps `tasks/*`"),
            "expected no report of a glob kept for a directory that empties; it wrote:\n{written}"
        );
        Ok(())
    })
}
