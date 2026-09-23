//! `ritual new demo --path <checkout> --cli mytool` gives the project a
//! binary named `mytool` and an alias `cargo mytool`. The bundle's mount
//! key, `ritual`, is not the bin's name, so ritual's management bundle stays
//! nested: `cargo mytool ritual add my-task` reaches `add`, `cargo mytool
//! my-task` runs the added task directly, and `mytool add`, with no `ritual`
//! in between, is not a command.

mod support;

use support::manifest;
use support::{Project, TempDir, TestOutcome, help, in_checkout};

const BIN_NAME: &str = "mytool";

/// The composed CLI's `[[bin]]` is named `mytool`, and the project's one
/// cargo alias carries that same name.
fn assert_the_bin_and_alias_are_named_after_the_cli(project: &Project) -> TestOutcome {
    manifest::assert_sole_bin_at_main_rs(&project.cli_manifest()?, BIN_NAME);
    assert_eq!(
        manifest::keys_of(&project.cargo_config()?, &["alias"]),
        [BIN_NAME],
        "expected the project's cargo alias to be named `{BIN_NAME}`"
    );
    Ok(())
}

/// `cargo mytool ritual add my-task` reaches `add` through the nested
/// bundle and scaffolds `my-task`; `cargo mytool my-task` then runs it.
fn add_through_the_nested_bundle_then_run_directly(project: &Project) -> TestOutcome {
    project
        .alias(&["ritual", "add", "my-task"])?
        .expect_success("`cargo mytool ritual add my-task`, through the nested bundle");
    assert!(
        project.root().join("tasks/my-task/Cargo.toml").is_file(),
        "expected `ritual add my-task` to scaffold tasks/my-task/Cargo.toml"
    );

    let run_result = project.alias(&["my-task"])?;
    run_result.expect_success("`cargo mytool my-task`, the newly added task run directly");
    assert!(
        run_result.stdout.contains("my-task has nothing to do yet"),
        "expected the freshly scaffolded task's default body to report itself; stdout \
         was:\n{}",
        run_result.stdout
    );
    Ok(())
}

/// `add` is not a top-level command: `--help` lists ritual's bundle key and
/// the added task, and `mytool add` is refused by clap as unrecognised.
fn assert_add_is_not_reachable_without_ritual_first(project: &Project) -> TestOutcome {
    let help = project.run_cli(&["--help"])?;
    help.expect_success("`mytool --help`");
    assert_eq!(
        help::command_names(&help.stdout),
        ["ritual", "my-task", "help"],
        "stdout was:\n{}",
        help.stdout
    );

    help::assert_refuses_unrecognized_subcommand(
        &project.run_cli(&["add", "another-task"])?,
        "add",
    );
    Ok(())
}

#[test]
fn a_named_cli_gets_the_ritual_bundle_nested_under_its_own_key() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("named-cli-nests-bundle")?;
        let project =
            Project::scaffold(checkout, working_dir.path(), "demo", &["--cli", BIN_NAME])?;

        assert_the_bin_and_alias_are_named_after_the_cli(&project)?;
        add_through_the_nested_bundle_then_run_directly(&project)?;
        assert_add_is_not_reachable_without_ritual_first(&project)?;

        Ok(())
    })
}
