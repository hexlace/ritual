//! A project can carry a scaffolder of its own called `add` alongside
//! ritual's, and the two never collide. Scaffolded with `--cli acme` and
//! given a hand-written bundle mounted under the key `acme` — its own bin
//! name, so flattened — the project answers `acme add` with its own task and
//! `acme ritual add` with ritual's, because ritual's bundle sits under the
//! ordinary key `ritual` and stays nested.

mod support;

use support::manifest;
use support::{Child, Project, TempDir, TestOutcome, help, in_checkout};

const BIN_NAME: &str = "acme";
const OWN_ADD_CRATE: &str = "acme-own-add";

/// Writes a bundle, `acme-own`, whose one child is a leaf mounted under
/// `add`, mounts it under the bin's own name, and regenerates.
fn write_and_mount_the_projects_own_bundle(project: &Project) -> TestOutcome {
    project.write_leaf(OWN_ADD_CRATE)?;
    let bundle_dir = project.write_bundle(
        "acme-own",
        "acme's own scaffolder",
        &[Child::Crate {
            key: "add",
            crate_name: OWN_ADD_CRATE,
        }],
    )?;
    project.mount(&bundle_dir, BIN_NAME, "acme-own")?;

    project
        .run_cli(&["ritual", "regenerate"])?
        .expect_success("`ritual regenerate` after mounting the project's own bundle");
    Ok(())
}

/// `--help` lists the project's own `add` at the top level and `ritual`
/// beside it; `add` runs the project's own leaf, and `ritual add` runs
/// ritual's real scaffolder, which writes a task crate and mounts it.
fn assert_the_two_scaffolders_never_collide(project: &Project) -> TestOutcome {
    let help = project.run_cli(&["--help"])?;
    help.expect_success("`--help`");
    assert_eq!(
        help::command_names(&help.stdout),
        ["ritual", "add", "help"],
        "stdout was:\n{}",
        help.stdout
    );

    let own_add = project.run_cli(&["add"])?;
    own_add.expect_success("`add`, the project's own flattened scaffolder");
    assert!(
        own_add.stdout.contains(&format!("{OWN_ADD_CRATE} ran")),
        "expected `add` to run the project's own task, not ritual's; stdout was:\n{}",
        own_add.stdout
    );

    let ritual_add = project.run_cli(&["ritual", "add", "a-real-task"])?;
    ritual_add.expect_success("`ritual add a-real-task`, ritual's real scaffolder");
    assert!(
        project
            .root()
            .join("tasks/a-real-task/Cargo.toml")
            .is_file(),
        "expected `ritual add a-real-task` to scaffold tasks/a-real-task/Cargo.toml — the \
         project's own leaf never writes files"
    );
    assert!(
        !ritual_add.stdout.contains(&format!("{OWN_ADD_CRATE} ran")),
        "expected `ritual add` to never run the project's own task; stdout was:\n{}",
        ritual_add.stdout
    );
    assert_eq!(
        manifest::tasks(&project.cli_manifest()?)?,
        ["ritual", BIN_NAME, "a-real-task"],
        "expected ritual's `add` to mount the new task on the composed CLI's own top level"
    );

    Ok(())
}

#[test]
fn a_projects_own_add_and_ritual_add_are_two_commands() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("own-add-beside-ritual-add")?;
        let project =
            Project::scaffold(checkout, working_dir.path(), "demo", &["--cli", BIN_NAME])?;

        write_and_mount_the_projects_own_bundle(&project)?;
        assert_the_two_scaffolders_never_collide(&project)?;

        Ok(())
    })
}
