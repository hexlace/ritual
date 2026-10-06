//! A bundle is mounted under whatever key the project chose, so the command
//! that reaches one of its tasks is the project's to spell, not the
//! bundle's. A task that tells a person to run it again asks the command
//! line it was handed for that command, and what it prints is what the
//! person types: `cargo ritual tools db sync` for a bundle mounted under
//! `tools`, and `cargo acme tools db sync` in a project made with
//! `--cli acme`.
//!
//! The bundle here is `acme-tools`, mounted under `tools` rather than its own
//! name, with `sync` one bundle further down, under `db`. The remedy it
//! prints is then run, word for word, through `cargo` in the project, and
//! reaches the same task: that is what makes it one a person can paste.

mod support;

use std::fmt::Write as _;

use support::{
    Outcome, Project, RunOutput, TempDir, TestOutcome, crates, failure, in_checkout, manifest,
};

/// The bundle's crate name, which a project could mount it under as it is.
const CRATE_NAME: &str = "acme-tools";

/// The key this story mounts the bundle under instead.
const MOUNT_KEY: &str = "tools";

/// What `sync` refuses with, ahead of the command it names.
const REFUSAL: &str = "the database is not set up yet; set it up, then run";

/// Renders the bundle's `src/lib.rs`: `db`, a bundle holding `sync`, whose
/// handler refuses with a remedy spelled by `CommandLine::cargo_command`.
fn bundle_lib() -> String {
    let mut output = String::new();
    output.push_str("//! A hand-written bundle crate, built for this story.\n\n");
    output.push_str("use rituals::{CommandLine, Failure, Outcome, Task, clap};\n\n");
    output.push_str("/// What `sync` accepts on the command line — nothing.\n");
    output.push_str("#[derive(clap::Args)]\n");
    output.push_str("struct Arguments {}\n\n");
    output.push_str("/// This bundle, for a command line to mount under whatever key it likes.\n");
    output.push_str("#[must_use]\n");
    output.push_str("pub fn task() -> Task {\n");
    output.push_str("    let sync = Task::receiving_command_line(\"sync the database\", sync);\n");
    output.push_str("    let db = Task::group(\"the project's database\", [(\"sync\", sync)]);\n");
    output.push_str("    Task::group(\"tools for a project\", [(\"db\", db)])\n");
    output.push_str("}\n\n");
    output.push_str("fn sync(command_line: &CommandLine, _arguments: Arguments) -> Outcome {\n");
    output.push_str("    Err(Failure::new(format!(\n");
    let _ = writeln!(output, "        \"{REFUSAL} `{{}}` again\",");
    output.push_str("        command_line.cargo_command()\n");
    output.push_str("    )))\n");
    output.push_str("}\n");
    output
}

/// Writes the bundle under `.rituals/acme-tools`, makes it a workspace
/// member, mounts it under [`MOUNT_KEY`], and regenerates through
/// `regenerate`, typed as `regenerate_path` says it is in this project.
fn write_and_mount_the_bundle(project: &Project, regenerate_path: &[&str]) -> TestOutcome {
    let member = format!(".rituals/{CRATE_NAME}");
    let crate_dir = project.root().join(&member);
    crates::write_crate(
        &crate_dir,
        &crates::bundle_manifest(CRATE_NAME, &[]),
        &bundle_lib(),
    )?;
    manifest::edit(&project.workspace_manifest_path(), |document| {
        manifest::push_member(document, &member)
    })?;
    project.mount(&crate_dir, MOUNT_KEY, CRATE_NAME)?;
    project
        .run_cli(regenerate_path)?
        .expect_success("`regenerate` after mounting the bundle under `tools`");
    Ok(())
}

/// The command a refusal line names between its last pair of backticks.
fn named_command(refusal: &str) -> Outcome<&str> {
    match refusal.rsplit('`').nth(1) {
        Some(command) if !command.is_empty() => Ok(command),
        _ => failure(format!(
            "expected a command in backticks; the refusal was: {refusal}"
        )),
    }
}

/// Runs `cargo tools db sync` through the built binary, asserts the remedy
/// names `expected`, then runs that remedy through `cargo` in the project
/// and asserts it reaches `sync` again.
fn assert_the_remedy_is_the_command_typed(
    project: &Project,
    bin_name: &str,
    expected: &str,
) -> TestOutcome {
    let output = project.run_cli(&[MOUNT_KEY, "db", "sync"])?;
    output.expect_failure("`tools db sync`, which always refuses");
    let refusal = output.sole_line_prefixed_with(bin_name);
    assert_eq!(refusal, format!("{REFUSAL} `{expected}` again"));

    let command = named_command(refusal)?;
    let Some(arguments) = command.strip_prefix("cargo ") else {
        return failure(format!("expected a cargo command; it was `{command}`"));
    };
    let words: Vec<&str> = arguments.split(' ').collect();
    let pasted: RunOutput = project.cargo(&words)?;
    pasted.expect_failure(&format!("`{command}`, pasted from the refusal"));
    assert!(
        pasted.stderr.contains(&format!("{bin_name}: {refusal}")),
        "expected `{command}` to reach `sync` again; stderr was:\n{}",
        pasted.stderr
    );
    Ok(())
}

#[test]
fn a_bundle_under_a_key_of_the_project_s_choosing_names_that_key_in_its_remedy() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("bundle-names-its-command")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;

        write_and_mount_the_bundle(&project, &["regenerate"])?;
        assert_the_remedy_is_the_command_typed(&project, "ritual", "cargo ritual tools db sync")
    })
}

#[test]
fn under_a_named_cli_the_remedy_names_that_cli() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("bundle-names-its-command-named-cli")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &["--cli", "acme"])?;

        write_and_mount_the_bundle(&project, &["ritual", "regenerate"])?;
        assert_the_remedy_is_the_command_typed(&project, "acme", "cargo acme tools db sync")
    })
}
