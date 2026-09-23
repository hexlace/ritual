//! `add <bin-name>` is refused before anything is written, as a name reserved
//! for the command line's own commands: the slot named after the compiled
//! binary must hold a bundle, and `add` only scaffolds plain tasks. That holds
//! whether the composed CLI's package name and bin name differ (the default
//! shape) or are the same string.
//!
//! This refusal is `add`'s own, and it comes before the check `add` shares
//! with `regenerate` — which lets a name equal to the bin pass, because
//! `regenerate` has to accept the manifest entry of a bundle mounted under
//! the bin's own name.

mod support;

use support::manifest;
use support::{
    Project, RunOutput, TempDir, TestOutcome, assert_trees_identical, in_checkout, snapshot_tree,
};

/// Asserts that `result` refused `add` of the bin's own name,
/// `bin_name`, by naming it as reserved and asking for another name.
#[track_caller]
fn assert_refused_as_the_bin_slot(result: &RunOutput, bin_name: &str) {
    result.expect_failure(&format!("`add {bin_name}`"));
    let message = result.sole_line_prefixed_with(bin_name);
    assert!(
        message.contains(&format!("`{bin_name}`")),
        "expected the refusal to name `{bin_name}`, the bin; message was:\n{message}"
    );
    assert!(
        message.contains("is reserved for this command line's own commands"),
        "expected the refusal to say the bin's name is reserved; message was:\n{message}"
    );
    assert!(
        message.contains("give this task another name"),
        "expected the refusal to name the remedy; message was:\n{message}"
    );
}

/// Runs `add <bin>` through `add_path` (the words that reach `add` on this
/// project's command line) and asserts it refused and wrote nothing.
fn assert_add_of_the_bin_name_is_refused(project: &Project, add_path: &[&str]) -> TestOutcome {
    let bin_name = project.bin_name()?;
    let before = snapshot_tree(project.root())?;

    let mut arguments = add_path.to_vec();
    arguments.push(&bin_name);
    assert_refused_as_the_bin_slot(&project.run_cli(&arguments)?, &bin_name);

    let after = snapshot_tree(project.root())?;
    assert_trees_identical(
        &format!("a refused `add {bin_name}` must write nothing"),
        &before,
        &after,
    );
    Ok(())
}

/// The default shape: package `demo-ritual`, bin `ritual`.
#[test]
fn add_refuses_the_bin_name_when_package_and_bin_differ() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("add-bin-name-differ")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;

        assert_add_of_the_bin_name_is_refused(&project, &["add"])
    })
}

/// Package and bin as one string: `new same --cli same-ritual` names the
/// package `same-ritual` (the project name plus `-ritual`) and the bin
/// `same-ritual` (from `--cli`), with no hand-editing.
///
/// The bin, `same-ritual`, is not `ritual`, so ritual's own bundle is nested
/// and `add` is reached as `same-ritual ritual add`. The refusal compares the
/// name against the running command line's bin name however deep `add` was
/// reached.
#[test]
fn add_refuses_the_bin_name_when_package_and_bin_are_the_same_string() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("add-bin-name-same")?;
        let project = Project::scaffold(
            checkout,
            working_dir.path(),
            "same",
            &["--cli", "same-ritual"],
        )?;

        let cli_manifest = project.cli_manifest()?;
        assert_eq!(
            manifest::package_name(&cli_manifest)?,
            manifest::sole_bin_name(&cli_manifest)?,
            "expected the package name to equal the bin name, so this case tests \
             package == bin; manifest was:\n{cli_manifest}"
        );

        assert_add_of_the_bin_name_is_refused(&project, &["ritual", "add"])
    })
}
