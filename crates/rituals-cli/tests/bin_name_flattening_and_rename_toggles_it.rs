//! A bundle mounted under a key that is exactly the compiled binary's own
//! name has its children appear directly as top-level commands: the key is
//! not typed, does not appear as a command, and the bundle's own
//! description never appears in `--help`. A bundle mounted under any other
//! key is reached only by typing that key first. And since the flattened
//! key is the compiled binary's own name, not a flag in the generated file,
//! renaming the binary by hand and rebuilding is enough, on its own, to flip
//! which of those two shapes a bundle has.
//!
//! The project is scaffolded with `--cli flatten-demo`, a bin name no other
//! bundle occupies; ritual's own bundle stays nested under its ordinary key,
//! `ritual`, throughout.

mod support;

use std::path::Path;

use support::manifest;
use support::{Child, Project, RunOutput, TempDir, TestOutcome, help, in_checkout, run_binary};

const BIN_NAME: &str = "flatten-demo";
const RENAMED_BIN_NAME: &str = "flatten-demo-renamed";
const FLATTENED_ABOUT: &str = "housework";

/// Writes a bundle, `chores` (children `wake`, `wash`), mounted under the
/// bin's own name, and a second bundle, `extras` (children `cook`,
/// `clean`), mounted under the ordinary key `other` — the contrasting case
/// in the same build — and regenerates.
fn write_and_mount_both_bundles(project: &Project) -> TestOutcome {
    for leaf in ["wake", "wash", "cook", "clean"] {
        project.write_leaf(leaf)?;
    }
    let chores_dir = project.write_bundle(
        "chores",
        FLATTENED_ABOUT,
        &[
            Child::Crate {
                key: "wake",
                crate_name: "wake",
            },
            Child::Crate {
                key: "wash",
                crate_name: "wash",
            },
        ],
    )?;
    project.mount(&chores_dir, BIN_NAME, "chores")?;
    let extras_dir = project.write_bundle(
        "extras",
        "extra chores nobody asked for",
        &[
            Child::Crate {
                key: "cook",
                crate_name: "cook",
            },
            Child::Crate {
                key: "clean",
                crate_name: "clean",
            },
        ],
    )?;
    project.mount(&extras_dir, "other", "extras")?;

    project
        .run_cli(&["ritual", "regenerate"])?
        .expect_success("`ritual regenerate` after mounting both bundles");
    Ok(())
}

/// Asserts that `--help` shows `chores` flattened — its children at the top
/// level, its key and description nowhere — and `extras` nested under
/// `other`.
fn assert_help_shows_one_bundle_flattened_and_one_nested(project: &Project) -> TestOutcome {
    let help = project.run_cli(&["--help"])?;
    help.expect_success("`--help`");

    assert_eq!(
        help::command_names(&help.stdout),
        ["ritual", "wake", "wash", "other", "help"],
        "stdout was:\n{}",
        help.stdout
    );
    assert!(
        !help.stdout.contains(FLATTENED_ABOUT),
        "expected the flattened bundle's own description to never appear in --help; \
         stdout was:\n{}",
        help.stdout
    );
    Ok(())
}

/// Asserts that a flattened child runs directly, a nested child does not,
/// and the nested child runs once reached through its bundle's key.
fn assert_dispatch_follows_the_shape(project: &Project) -> TestOutcome {
    let wake = project.run_cli(&["wake"])?;
    wake.expect_success("`wake`, a flattened bundle's child, run directly");
    assert!(
        wake.stdout.contains("wake ran"),
        "expected the flattened leaf to run; stdout was:\n{}",
        wake.stdout
    );

    help::assert_refuses_unrecognized_subcommand(&project.run_cli(&["cook"])?, "cook");

    let nested = project.run_cli(&["other", "cook"])?;
    nested.expect_success("`other cook`");
    assert!(
        nested.stdout.contains("cook ran"),
        "expected the nested leaf to run once reached through its key; stdout was:\n{}",
        nested.stdout
    );
    Ok(())
}

/// Renames the composed CLI's `[[bin]]` target in the manifest.
fn rename_the_bin(project: &Project) -> TestOutcome {
    manifest::edit(&project.cli_manifest_path(), |document| {
        manifest::rename_sole_bin(document, RENAMED_BIN_NAME)
    })
}

/// Runs `renamed_binary` with `arguments` at the project root.
fn run_renamed(
    project: &Project,
    renamed_binary: &Path,
    arguments: &[&str],
) -> support::Outcome<RunOutput> {
    run_binary(renamed_binary, project.root(), arguments)
}

/// Rebuilt under the new name, with nothing else changed, the bundle that
/// was flattened is nested: its children leave the top level, and its key —
/// the old bin name, unchanged in the generated file — is an ordinary
/// command that reaches them.
fn assert_renaming_the_bin_and_rebuilding_unflattens_it(project: &Project) -> TestOutcome {
    let generated_before = project.generated_file()?;
    rename_the_bin(project)?;
    let renamed_binary = project.build()?;

    let help = run_renamed(project, &renamed_binary, &["--help"])?;
    help.expect_success("the renamed binary with `--help`");
    assert_eq!(
        help::command_names(&help.stdout),
        ["ritual", BIN_NAME, "other", "help"],
        "stdout was:\n{}",
        help.stdout
    );

    let child = run_renamed(project, &renamed_binary, &[BIN_NAME, "wake"])?;
    child.expect_success("the un-flattened bundle's child, reached through its own key");
    assert!(
        child.stdout.contains("wake ran"),
        "expected the un-flattened leaf to still run; stdout was:\n{}",
        child.stdout
    );
    assert_eq!(
        project.generated_file()?,
        generated_before,
        "building and running the renamed binary must leave the generated file as it was"
    );
    Ok(())
}

#[test]
fn the_bin_name_flattens_a_bundle_and_renaming_it_toggles_that() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("bin-name-flatten")?;
        let project =
            Project::scaffold(checkout, working_dir.path(), BIN_NAME, &["--cli", BIN_NAME])?;

        write_and_mount_both_bundles(&project)?;
        assert_help_shows_one_bundle_flattened_and_one_nested(&project)?;
        assert_dispatch_follows_the_shape(&project)?;
        assert_renaming_the_bin_and_rebuilding_unflattens_it(&project)?;

        Ok(())
    })
}
