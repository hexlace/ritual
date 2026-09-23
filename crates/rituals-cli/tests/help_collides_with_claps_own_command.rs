//! `help` is not a name ritual reserves; clap already put a command there.
//! Every composed CLI with more than one subcommand gets clap's own `help`
//! subcommand, so a task imported under the key `help` collides with that,
//! not with anything ritual keeps for itself — the same rule every other
//! collision follows: a collision is only ever with what is actually there.
//!
//! Three checkpoints: `add help` is refused before anything is written;
//! `regenerate` refuses a `help` import added to the manifests by hand; and
//! a `help` task written into the generated file by hand, past both of
//! them, makes the built CLI refuse to start.

mod support;

use support::{
    Checkout, Project, TempDir, TestOutcome, assert_trees_identical, in_checkout, snapshot_tree,
};

/// Scaffolds a project under `--cli <name>`, so ritual's own bundle is
/// nested under `ritual` and the top level holds nothing but it and clap's
/// `help`.
fn scaffold(checkout: &Checkout, working_dir: &TempDir, name: &str) -> support::Outcome<Project> {
    Project::scaffold(checkout, working_dir.path(), name, &["--cli", name])
}

/// Asserts that `result` refused `help` with one line naming it and
/// crediting clap with the command it collides with.
#[track_caller]
fn assert_refuses_help(result: &support::RunOutput, bin_name: &str, what: &str) {
    result.expect_failure(what);
    let message = result.sole_line_prefixed_with(bin_name);
    assert!(
        message.contains("`help`"),
        "expected the refusal to name `help`; message was:\n{message}"
    );
    assert!(
        message.contains("clap"),
        "expected the refusal to say `help` is clap's own command, not one ritual reserves; \
         message was:\n{message}"
    );
}

#[test]
fn add_help_is_refused_before_writing_anything() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("add-help-pre-write")?;
        let project = scaffold(checkout, &working_dir, "help-collision-add")?;

        let before = snapshot_tree(project.root())?;
        assert_refuses_help(
            &project.run_cli(&["ritual", "add", "help"])?,
            "help-collision-add",
            "`ritual add help`",
        );
        assert_trees_identical(
            "a refused `add help` must write nothing",
            &before,
            &snapshot_tree(project.root())?,
        );
        Ok(())
    })
}

/// Mounts a `help` leaf on both manifests by hand — `add` would refuse — and
/// shows `regenerate` refusing it before rewriting the generated file; then
/// writes it into the generated file by hand as well and shows the built
/// CLI refusing to start.
#[test]
fn a_hand_mounted_help_task_is_refused_by_regenerate_and_at_startup() -> TestOutcome {
    in_checkout(|checkout| {
        let bin_name = "help-collision-startup";
        let working_dir = TempDir::new("help-collision-startup")?;
        let project = scaffold(checkout, &working_dir, bin_name)?;

        let help_dir = project.write_leaf("help")?;
        project.mount(&help_dir, "help", "help")?;

        let generated_file = project.generated_file()?;
        assert_refuses_help(
            &project.run_cli(&["ritual", "regenerate"])?,
            bin_name,
            "`ritual regenerate` with `help` mounted on the manifests",
        );
        assert_eq!(
            project.generated_file()?,
            generated_file,
            "a refused `regenerate` must not rewrite the generated file"
        );

        let closing_marker = "        ],\n";
        assert!(
            generated_file.matches(closing_marker).count() == 1,
            "expected one closing `],` of the generated file's mounted-tasks array; file \
         was:\n{generated_file}"
        );
        support::write_text(
            &project.generated_file_path(),
            &generated_file.replacen(
                closing_marker,
                &format!("            (\"help\", help::task()),\n{closing_marker}"),
                1,
            ),
        )?;

        let refused = project.run_cli(&["--help"])?;
        refused
            .expect_failure("the built CLI, with `help` written into the generated file by hand");
        assert!(
            refused.sole_line_prefixed_with(bin_name).contains("`help`"),
            "expected the refusal to name `help`; stderr was:\n{}",
            refused.stderr
        );
        Ok(())
    })
}
