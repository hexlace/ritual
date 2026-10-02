//! `import` refuses, and says what to do instead, when it cannot or must not
//! go ahead: it is not running inside the project its command line belongs
//! to; the key is already a dependency of the project's command line crate;
//! the key would collide with a name the command line already answers to; or
//! the crate is not a task. A refused run leaves the project exactly as it
//! found it, down to the bytes of the lockfile.
//!
//! What each message says is read for substance, not wording: it names the
//! key or crate in question, and it points at something to do. The refusal
//! is the one line the program prefixes with its own name; Cargo's own lines
//! on the same stream, if any, are not the refusal.

mod support;

use std::fs;

use support::task_sources::{write_path_task, write_unmarked_crate};
use support::{
    Child, Project, RunOutput, TempDir, TestOutcome, assert_trees_identical, in_checkout, manifest,
    path_to_str, run_binary, run_ritual, snapshot_tree,
};

const CRATE: &str = "greeter";

/// The one stderr line `bin_name` prefixed as its own, without the prefix.
#[track_caller]
fn refusal_line<'output>(result: &'output RunOutput, bin_name: &str) -> &'output str {
    let prefix = format!("{bin_name}: ");
    let lines: Vec<&str> = result
        .stderr
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect();
    assert_eq!(
        lines.len(),
        1,
        "expected exactly one stderr line prefixed with `{prefix}`; stderr was:\n{}",
        result.stderr
    );
    lines.first().copied().unwrap_or_default()
}

/// Asserts that `message` points at something to do rather than only
/// reporting a problem. The wording is not fixed, so this reads for any of
/// the ways a remedy is phrased.
#[track_caller]
fn assert_says_what_to_do_instead(message: &str) {
    const REMEDY_WORDS: [&str; 12] = [
        "instead",
        "another",
        "different",
        "choose",
        "pick",
        "rename",
        "declare",
        "remove",
        "run ",
        "try",
        "ask",
        "give",
    ];
    assert!(
        REMEDY_WORDS.iter().any(|word| message.contains(word)),
        "expected the refusal to say what to do instead; message was:\n{message}"
    );
}

/// Asserts the refusal named `name`, and said what to do instead.
#[track_caller]
fn assert_refusal_names_and_advises(message: &str, name: &str) {
    assert!(
        message.contains(name),
        "expected the refusal to name `{name}`; message was:\n{message}"
    );
    assert_says_what_to_do_instead(message);
}

/// A task crate to import from a directory beside the project, so a refusal
/// is never for want of a good crate.
fn a_good_task(
    working_dir: &TempDir,
    checkout: &support::Checkout,
) -> support::Outcome<std::path::PathBuf> {
    let directory = working_dir.path().join(CRATE);
    write_path_task(&directory, checkout, CRATE, "0.1.0")?;
    Ok(directory)
}

#[test]
fn the_global_command_inside_a_project_hands_back_the_cargo_command_to_run() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-global-in-project")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let directory = a_good_task(&working_dir, checkout)?;
        let before = snapshot_tree(project.root())?;

        let result = run_ritual(
            project.root(),
            &["import", CRATE, "--path", path_to_str(&directory)?],
        )?;

        result.expect_failure("the global `ritual import` inside a project");
        let message = refusal_line(&result, "ritual");
        assert!(
            message.contains("cargo ritual import"),
            "expected the refusal to hand back `cargo ritual import`; message was:\n{message}"
        );
        assert_trees_identical(
            "a refused `import` must write nothing",
            &before,
            &snapshot_tree(project.root())?,
        );
        Ok(())
    })
}

#[test]
fn a_projects_command_line_run_inside_another_project_hands_back_the_cargo_command() -> TestOutcome
{
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-foreign-project")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let other = Project::scaffold(checkout, working_dir.path(), "other", &[])?;
        let directory = a_good_task(&working_dir, checkout)?;
        let binary = project.build()?;
        let before = snapshot_tree(other.root())?;

        let result = run_binary(
            &binary,
            other.root(),
            &["import", CRATE, "--path", path_to_str(&directory)?],
        )?;

        result.expect_failure("`demo`'s `ritual import` inside `other`");
        let message = refusal_line(&result, "ritual");
        assert!(
            message.contains("cargo ritual import"),
            "expected the refusal to hand back `cargo ritual import`; message was:\n{message}"
        );
        assert_trees_identical(
            "a refused `import` must write nothing",
            &before,
            &snapshot_tree(other.root())?,
        );
        Ok(())
    })
}

#[test]
fn the_global_command_outside_any_project_says_to_work_inside_one() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-outside-any-project")?;
        let directory = a_good_task(&working_dir, checkout)?;
        let empty = TempDir::new("import-outside-any-project-empty")?;

        let result = run_ritual(
            empty.path(),
            &["import", CRATE, "--path", path_to_str(&directory)?],
        )?;

        result.expect_failure("`ritual import` in a directory with no project");
        let message = refusal_line(&result, "ritual");
        assert!(
            message.contains("project"),
            "expected the refusal to say `import` works inside a project; message was:\n{message}"
        );
        assert_says_what_to_do_instead(message);
        assert!(
            snapshot_tree(empty.path())?.is_empty(),
            "a refused `import` must write nothing, not even into an empty directory"
        );
        Ok(())
    })
}

#[test]
fn a_key_that_is_already_a_dependency_is_refused_before_writing() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-key-is-a-dependency")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let wake_dir = project.write_leaf("wake")?;
        project.mount(&wake_dir, "wake", "wake")?;
        let directory = a_good_task(&working_dir, checkout)?;
        let binary = project.build()?;
        let before = snapshot_tree(project.root())?;
        let lockfile_before = fs::read(project.root().join("Cargo.lock"))?;

        let result = run_binary(
            &binary,
            project.root(),
            &["import", CRATE, "wake", "--path", path_to_str(&directory)?],
        )?;

        result.expect_failure("`import greeter wake`, with `wake` already a dependency");
        assert_refusal_names_and_advises(refusal_line(&result, "ritual"), "wake");
        assert_trees_identical(
            "a refused `import` must write nothing",
            &before,
            &snapshot_tree(project.root())?,
        );
        assert_eq!(
            lockfile_before,
            fs::read(project.root().join("Cargo.lock"))?,
            "a refused `import` must leave Cargo.lock as it was"
        );
        Ok(())
    })
}

#[test]
fn a_key_that_is_a_top_level_command_already_is_refused_before_writing() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-key-is-top-level")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let directory = a_good_task(&working_dir, checkout)?;
        let binary = project.build()?;
        let before = snapshot_tree(project.root())?;
        let lockfile_before = fs::read(project.root().join("Cargo.lock"))?;

        // In a default project ritual's own bundle is flattened into the
        // command line, so `add` is a top-level command.
        let result = run_binary(
            &binary,
            project.root(),
            &["import", CRATE, "add", "--path", path_to_str(&directory)?],
        )?;

        result.expect_failure("`import greeter add`, with `add` a top-level command already");
        assert_refusal_names_and_advises(refusal_line(&result, "ritual"), "add");
        assert_trees_identical(
            "a refused `import` must write nothing",
            &before,
            &snapshot_tree(project.root())?,
        );
        assert_eq!(
            lockfile_before,
            fs::read(project.root().join("Cargo.lock"))?,
            "a refused `import` must leave Cargo.lock as it was"
        );
        Ok(())
    })
}

#[test]
fn a_key_named_after_the_bin_is_refused_before_writing() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-key-is-the-bin")?;
        // With a bin name of its own, ritual's bundle is nested under `ritual`
        // rather than flattened, so `import` is reached as `chores ritual import`.
        let project =
            Project::scaffold(checkout, working_dir.path(), "demo", &["--cli", "chores"])?;
        let directory = a_good_task(&working_dir, checkout)?;
        let binary = project.build()?;
        let before = snapshot_tree(project.root())?;
        let lockfile_before = fs::read(project.root().join("Cargo.lock"))?;

        let result = run_binary(
            &binary,
            project.root(),
            &[
                "ritual",
                "import",
                CRATE,
                "chores",
                "--path",
                path_to_str(&directory)?,
            ],
        )?;

        result.expect_failure("`import greeter chores`, where `chores` is the bin's own name");
        assert_refusal_names_and_advises(refusal_line(&result, "chores"), "chores");
        assert_trees_identical(
            "a refused `import` must write nothing",
            &before,
            &snapshot_tree(project.root())?,
        );
        assert_eq!(
            lockfile_before,
            fs::read(project.root().join("Cargo.lock"))?,
            "a refused `import` must leave Cargo.lock as it was"
        );
        Ok(())
    })
}

#[test]
fn a_key_that_collides_with_a_flattened_bundle_child_is_refused_before_writing() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-key-collides-with-flattened")?;
        let project =
            Project::scaffold(checkout, working_dir.path(), "demo", &["--cli", "chores"])?;
        // A bundle mounted at the bin's own name is flattened into the
        // command line, so its `wake` child answers at the top level.
        let housework_dir = project.write_bundle(
            "housework",
            "chores management, for this test",
            &[Child::Inline { key: "wake" }],
        )?;
        project.mount(&housework_dir, "chores", "housework")?;
        project
            .run_cli(&["ritual", "regenerate"])?
            .expect_success("`ritual regenerate` after mounting `housework` at the bin name");
        let directory = a_good_task(&working_dir, checkout)?;
        let binary = project.build()?;
        let before = snapshot_tree(project.root())?;
        let lockfile_before = fs::read(project.root().join("Cargo.lock"))?;

        let result = run_binary(
            &binary,
            project.root(),
            &[
                "ritual",
                "import",
                CRATE,
                "wake",
                "--path",
                path_to_str(&directory)?,
            ],
        )?;

        result.expect_failure("`import greeter wake`, colliding with `housework`'s `wake`");
        let message = refusal_line(&result, "chores");
        assert_refusal_names_and_advises(message, "wake");
        assert!(
            message.contains("chores"),
            "expected the refusal to name `chores`, the bundle `wake` collides with; message \
             was:\n{message}"
        );
        assert_trees_identical(
            "a refused `import` must write nothing",
            &before,
            &snapshot_tree(project.root())?,
        );
        assert_eq!(
            lockfile_before,
            fs::read(project.root().join("Cargo.lock"))?,
            "a refused `import` must leave Cargo.lock as it was"
        );
        Ok(())
    })
}

#[test]
fn a_crate_that_is_not_a_task_is_refused_naming_the_crate() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-not-a-task")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let directory = working_dir.path().join("plain");
        write_unmarked_crate(&directory, "plain")?;
        let binary = project.build()?;

        let result = run_binary(
            &binary,
            project.root(),
            &["import", "plain", "--path", path_to_str(&directory)?],
        )?;

        result.expect_failure("`import plain`, a crate that is not a task");
        let message = refusal_line(&result, "ritual");
        assert_refusal_names_and_advises(message, "plain");
        assert!(
            message.contains("task"),
            "expected the refusal to say the crate is not a task; message was:\n{message}"
        );
        Ok(())
    })
}

#[test]
fn a_refused_task_check_leaves_the_manifest_and_lockfile_byte_for_byte() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-rollback-after-task-check")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let directory = working_dir.path().join("plain");
        write_unmarked_crate(&directory, "plain")?;
        // Built first, so a lockfile exists for the import to put back.
        let binary = project.build()?;

        let manifest_before = fs::read(project.cli_manifest_path())?;
        let lockfile_path = project.root().join("Cargo.lock");
        let lockfile_before = fs::read(&lockfile_path)?;
        let tree_before = snapshot_tree(project.root())?;

        let result = run_binary(
            &binary,
            project.root(),
            &["import", "plain", "--path", path_to_str(&directory)?],
        )?;
        result.expect_failure("`import plain`, a crate that is not a task");
        let message = refusal_line(&result, "ritual");
        assert!(
            message.contains("plain"),
            "expected the refusal to name `plain`, so this is the task check refusing and not \
             some other failure; message was:\n{message}"
        );

        assert_eq!(
            manifest_before,
            fs::read(project.cli_manifest_path())?,
            "the command line crate's Cargo.toml must be byte for byte as it was"
        );
        assert_eq!(
            lockfile_before,
            fs::read(&lockfile_path)?,
            "Cargo.lock must be byte for byte as it was"
        );
        assert_trees_identical(
            "a refused `import` must write nothing else either",
            &tree_before,
            &snapshot_tree(project.root())?,
        );

        // The control: Cargo's own `add` of the same crate does change both
        // files, so the bytes above were put back rather than never touched.
        let package = manifest::package_name(&project.cli_manifest()?)?;
        project
            .cargo(&[
                "add",
                "--package",
                &package,
                "plain",
                "--path",
                path_to_str(&directory)?,
            ])?
            .expect_success("plain `cargo add` of the same crate");
        assert_ne!(
            manifest_before,
            fs::read(project.cli_manifest_path())?,
            "expected `cargo add` to change the command line crate's Cargo.toml; if it did not, \
             this story proves nothing about putting it back"
        );
        assert_ne!(
            lockfile_before,
            fs::read(&lockfile_path)?,
            "expected `cargo add` to change Cargo.lock; if it did not, this story proves \
             nothing about putting it back"
        );
        Ok(())
    })
}
