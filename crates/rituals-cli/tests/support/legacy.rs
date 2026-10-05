//! A project in the layout ritual 0.1 made: the tasks `add` scaffolded live
//! in `tasks/<name>`, not `.rituals/<name>`.
//!
//! `add` scaffolds into `.rituals/` from 0.2.0, so a story that needs a 0.1
//! project cannot get one by running `add`. This writes what 0.1's `add`
//! wrote, directly, and `support_helpers_read_what_they_claim.rs` holds the
//! output against the text 0.1's own `add` produced.
//!
//! # What 0.1's `add <name>` wrote
//!
//! Captured by running 0.1's `new demo` and `add greet`, then reading the
//! files:
//!
//! - `tasks/greet/Cargo.toml` — `[package]` `name = "greet"`, `version =
//!   "0.1.0"`, `edition = "2024"`; `[dependencies]` `rituals.workspace =
//!   true`; `[package.metadata.ritual]` `task = true`;
//! - `tasks/greet/src/lib.rs` — the task's source;
//! - the root `Cargo.toml`, `[workspace] members` gaining the explicit entry
//!   `"tasks/greet"` after `"ritual"`;
//! - the command line crate's `Cargo.toml`, `[dependencies]` gaining
//!   `greet = { path = "../tasks/greet" }` after the `ritual` bundle's line,
//!   and `[package.metadata.ritual] tasks` gaining `"greet"`;
//! - the command line crate's generated `src/main.rs`, mounting `greet`.
//!
//! The source in `src/lib.rs` is this suite's own hand-written leaf (see
//! [`crates::leaf_lib`]) rather than 0.1's template text, so a story can tell
//! two tasks apart by what each prints; nothing about a migration depends on
//! what a task's source says.

use super::{Checkout, Outcome, Project, TempDir, TestOutcome, crates, git, manifest};

/// Adds the task `name` to `project` as 0.1's `add` did: the crate in
/// `tasks/<name>`, the member entry, the dependency and the `tasks` key on
/// the command line crate, and the regenerated command line.
pub(crate) fn add_task(project: &Project, name: &str) -> TestOutcome {
    crates::write_crate(
        &project.root().join("tasks").join(name),
        &crates::leaf_manifest(name),
        &crates::leaf_lib(name),
    )?;
    manifest::edit(&project.workspace_manifest_path(), |document| {
        manifest::push_member_on_its_own_line(document, &format!("tasks/{name}"))
    })?;
    manifest::edit(&project.cli_manifest_path(), |document| {
        manifest::add_path_dependency(
            document,
            &["dependencies"],
            name,
            &format!("../tasks/{name}"),
        )?;
        manifest::push_task(document, name)
    })?;
    project.run_cli(&["regenerate"])?.expect_success(&format!(
        "`regenerate` after adding {name} in the 0.1 layout"
    ));
    Ok(())
}

/// Writes the hand-written leaf crate `name` in `tasks/<name>` and lists it
/// among the workspace's members, as 0.1 left a task the command line does
/// not import: nothing mounts it.
pub(crate) fn write_unmounted_task(project: &Project, name: &str) -> TestOutcome {
    let member = format!("tasks/{name}");
    crates::write_crate(
        &project.root().join(&member),
        &crates::leaf_manifest(name),
        &crates::leaf_lib(name),
    )?;
    manifest::edit(&project.workspace_manifest_path(), |document| {
        manifest::push_member(document, &member)
    })
}

/// A project `new` scaffolded and 0.1's `add` gave each of `tasks`, in the
/// order given. Not under version control.
pub(crate) fn project_with_tasks(
    checkout: &Checkout,
    working_dir: &TempDir,
    tasks: &[&str],
) -> Outcome<Project> {
    let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
    for task in tasks {
        add_task(&project, task)?;
    }
    Ok(project)
}

/// [`project_with_tasks`], committed whole to a git repository of its own:
/// what a person has after upgrading `rituals-core` with their work saved,
/// the state `migrate` is allowed to start from.
///
/// The command line is built before the commit so the lockfile it carries is
/// the one a build leaves, and a later build does not make the tree dirty.
pub(crate) fn committed_project_with_tasks(
    checkout: &Checkout,
    working_dir: &TempDir,
    tasks: &[&str],
) -> Outcome<Project> {
    let project = project_with_tasks(checkout, working_dir, tasks)?;
    project.build()?;
    git::init_and_commit_everything(project.root())?;
    Ok(project)
}
