//! Outside any project, `create <name>` makes a crate of its own in the
//! current directory, for a project to import later. It is private unless
//! `--public` says otherwise, and its manifest says so. It ends on the
//! `import --path` command that brings it in, and that command, run
//! exactly as printed, imports a task that runs.

mod support;

use support::created::{Audience, assert_a_fresh_manifest, files_in, fresh_lib};
use support::{
    Project, TempDir, TestOutcome, in_checkout, manifest, read_text, run_ritual, snapshot_tree,
};

/// Runs `ritual create lint --path <checkout> <flags>` in an empty directory
/// and returns what it printed.
fn create_a_standalone_ritual(
    checkout: &support::Checkout,
    working_dir: &TempDir,
    audience: Audience,
) -> support::Outcome<support::RunOutput> {
    let mut arguments = vec!["create", "lint", "--path", checkout.path_argument()?];
    arguments.extend_from_slice(audience.flags());
    let created = run_ritual(working_dir.path(), &arguments)?;
    created.expect_success(&format!("`ritual {}`", arguments.join(" ")));
    Ok(created)
}

/// Runs `create lint` outside any project and asserts the two files and the
/// manifest of a crate for `audience`.
fn assert_a_standalone_ritual_for(audience: Audience, prefix: &str) -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new(prefix)?;
        create_a_standalone_ritual(checkout, &working_dir, audience)?;

        let crate_dir = working_dir.path().join("lint");
        assert_eq!(files_in(&crate_dir)?, ["Cargo.toml", "src/lib.rs"]);
        let document = manifest::read(&crate_dir.join("Cargo.toml"))?;
        assert_a_fresh_manifest(&document, "lint", audience, false);
        assert_eq!(read_text(&crate_dir.join("src/lib.rs"))?, fresh_lib("lint"));
        Ok(())
    })
}

#[test]
fn a_standalone_ritual_is_private_by_default() -> TestOutcome {
    assert_a_standalone_ritual_for(Audience::Private, "create-standalone-private")
}

#[test]
fn a_standalone_public_ritual_has_no_publish_key() -> TestOutcome {
    assert_a_standalone_ritual_for(Audience::Public, "create-standalone-public")
}

/// The `import` command `create` prints, parsed back out of its output.
fn printed_import_command(stdout: &str) -> Option<Vec<String>> {
    let line = stdout.lines().find(|line| line.starts_with("next: "))?;
    let after = line.split_once(" run cargo ritual ")?.1;
    let command = after.split_once(" (or ")?.0;
    Some(
        command
            .split_whitespace()
            .map(ToString::to_string)
            .collect(),
    )
}

#[test]
fn the_printed_import_line_brings_the_private_ritual_into_a_project_where_it_runs() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("create-standalone-then-import")?;
        let created = create_a_standalone_ritual(checkout, &working_dir, Audience::Private)?;

        assert_eq!(
            support::created::stdout_lines(&created)[..2],
            ["created lint/Cargo.toml", "created lint/src/lib.rs"]
        );
        let command = printed_import_command(&created.stdout).ok_or_else(|| {
            format!(
                "expected a `next: … run cargo ritual import …` line; stdout was:\n{}",
                created.stdout
            )
        })?;
        assert_eq!(&command[..3], ["import", "lint", "--path"]);
        assert_eq!(
            support::path_to_str(&working_dir.path().join("lint"))?,
            command[3],
            "expected the printed command to name the crate it made"
        );
        assert_eq!(command.len(), 4, "the command was {command:?}");

        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let arguments: Vec<&str> = command.iter().map(String::as_str).collect();
        project
            .run_cli(&arguments)?
            .expect_success(&format!("the printed `ritual {}`", arguments.join(" ")));

        let cli = project.cli_manifest()?;
        assert_eq!(manifest::tasks(&cli)?, ["ritual", "lint"]);
        let imported = manifest::read(&working_dir.path().join("lint/Cargo.toml"))?;
        assert_a_fresh_manifest(&imported, "lint", Audience::Private, false);
        let ran = project.alias(&["lint"])?;
        ran.expect_success("`cargo ritual lint` after importing the created ritual");
        assert!(
            ran.stdout.contains("lint has nothing to do yet"),
            "stdout was:\n{}",
            ran.stdout
        );
        Ok(())
    })
}

#[test]
fn a_path_outside_a_project_is_refused_with_words_and_nothing_is_written() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("create-standalone-path")?;
        let before = snapshot_tree(working_dir.path())?;

        let refused = run_ritual(
            working_dir.path(),
            &[
                "create",
                "private/lint",
                "--path",
                checkout.path_argument()?,
            ],
        )?;
        refused.expect_failure("`ritual create private/lint` outside any project");
        let message = refused.sole_line_prefixed_with("ritual");
        assert!(
            message.contains("private/lint")
                && (message.contains(".rituals/") || message.contains("project")),
            "expected the refusal to name the path and say it belongs inside a project's \
             .rituals/; message was:\n{message}"
        );
        assert_eq!(before, snapshot_tree(working_dir.path())?);
        Ok(())
    })
}
