//! `--cli target` scaffolds a working project like any other name. The
//! composed CLI crate's directory is fixed at `ritual/` whatever `--cli`
//! says — `--cli` names the bin and the cargo alias and nothing on disk — so
//! no name a caller chooses becomes a top-level directory, `demo/target/`
//! is only ever Cargo's build output, and `cargo clean`, which deletes
//! exactly that directory, never touches the crate's source.
//!
//! Builds with Cargo's default locations rather than the suite's own (see
//! `support::process::cargo_with_default_locations`): what is under test is
//! that `cargo clean` acts on the project's own `target/`.

mod support;

use support::manifest;
use support::process::{built_binary_path, cargo_with_default_locations};
use support::{Project, TempDir, TestOutcome, help, in_checkout, run_binary};

const BIN_NAME: &str = "target";

/// The scaffold puts the composed CLI crate at `ritual/`, names its bin and
/// alias `target`, and creates no `target/` directory of its own.
fn assert_the_scaffold_leaves_target_to_cargo(project: &Project) -> TestOutcome {
    assert_eq!(
        project.composed_cli_dir(),
        project.root().join("ritual"),
        "expected the composed CLI crate's directory to be fixed at ritual/, not to follow \
         --cli"
    );
    manifest::assert_sole_bin_at_main_rs(&project.cli_manifest()?, BIN_NAME);
    assert_eq!(
        manifest::keys_of(&project.cargo_config()?, &["alias"]),
        [BIN_NAME]
    );
    assert!(
        !project.root().join("target").exists(),
        "expected no demo/target/ right after scaffolding — only a build creates it"
    );
    Ok(())
}

/// Builds, cleans, and builds again with Cargo's defaults; the crate's own
/// files survive the clean, and the rebuilt binary answers `--help`.
fn assert_the_project_survives_build_clean_build(project: &Project) -> TestOutcome {
    cargo_with_default_locations(project.root(), &["build"])?
        .expect_success("`cargo build` in the freshly scaffolded project");
    cargo_with_default_locations(project.root(), &["clean"])?
        .expect_success("`cargo clean`, deleting demo/target/ whole");

    for file in ["Cargo.toml", "src/main.rs"] {
        assert!(
            project.composed_cli_dir().join(file).is_file(),
            "expected demo/ritual/{file} to survive `cargo clean`"
        );
    }

    cargo_with_default_locations(project.root(), &["build"])?
        .expect_success("`cargo build` again, after `cargo clean`");
    let help = run_binary(
        &built_binary_path(&project.root().join("target"), BIN_NAME),
        project.root(),
        &["--help"],
    )?;
    help.expect_success("the rebuilt binary with `--help`");
    assert!(
        help::lists_command(&help.stdout, "ritual"),
        "expected --help to list `ritual`, the bundle's own (nested) key; stdout was:\n{}",
        help.stdout
    );
    Ok(())
}

#[test]
fn new_with_cli_target_survives_cargo_clean() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("new-cli-target-survives-clean")?;
        let project =
            Project::scaffold(checkout, working_dir.path(), "demo", &["--cli", BIN_NAME])?;

        assert_the_scaffold_leaves_target_to_cargo(&project)?;
        assert_the_project_survives_build_clean_build(&project)?;

        Ok(())
    })
}
