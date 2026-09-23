//! `new`, `create`, and `add` all take a caller-supplied name and turn it
//! into paths on disk. Each refuses a name that would escape the intended
//! location — a path separator, `..`, an absolute path — or collide with
//! something already there, rather than writing outside that location or
//! overwriting something unrelated.
//!
//! `add` needs a scaffolded, built project to run in, so its case is a step
//! of `task_lifecycle_and_declaration_rules.rs`, reusing that story's build.
//! This file covers `new` and `create`, which need no build to be refused.
//!
//! Every test checks two things: the refusal names the whole offending name,
//! backticked, the way every name refusal quotes it; and the filesystem is
//! safe — nothing written where the name pointed, and a pre-existing
//! colliding entry left byte for byte alone. A non-zero exit alone is not
//! trusted, since a command line can fail for reasons that have nothing to
//! do with the name.

mod support;

use std::fs;
use std::path::Path;

use support::{
    ResultContext, RunOutput, TempDir, TestOutcome, path_to_str, run_ritual, snapshot_tree,
};

/// Runs `ritual <command> <name>` in `working_dir`, with the default source:
/// a name is refused before any source is read, so none needs to exist.
fn run_scaffolder(working_dir: &Path, command: &str, name: &str) -> support::Outcome<RunOutput> {
    run_ritual(working_dir, &[command, name])
}

/// Asserts that `result` is a refusal of `name` that quotes it whole.
#[track_caller]
fn assert_refuses_naming(result: &RunOutput, name: &str) {
    result.expect_failure(&format!("the name `{name}`"));
    assert!(
        result.stderr.contains(&format!("`{name}`")),
        "expected the refusal to quote the offending name `{name}`; stderr was:\n{}",
        result.stderr
    );
}

/// A name containing a path separator must not create the directory it
/// names, or anything beneath it.
#[test]
fn create_rejects_a_name_containing_a_path_separator() -> TestOutcome {
    let working_dir = TempDir::new("create-path-separator")?;
    let before = snapshot_tree(working_dir.path())?;

    let name = "escape-hatch/evil";
    assert_refuses_naming(&run_scaffolder(working_dir.path(), "create", name)?, name);

    assert_eq!(
        before,
        snapshot_tree(working_dir.path())?,
        "a rejected name must not create `escape-hatch/` or anything under it"
    );
    Ok(())
}

/// A name of exactly `..` must not be taken as "the parent directory".
///
/// The working directory is nested one level inside a scope of its own, so
/// the parent this test snapshots is a directory it alone controls — not
/// the shared system temp root, which other processes may be writing to.
#[test]
fn create_rejects_a_name_of_dot_dot() -> TestOutcome {
    let scope = TempDir::new("create-dot-dot")?;
    let working_dir = scope.path().join("working");
    fs::create_dir(&working_dir).context("creating the nested working directory failed")?;
    let parent_before = snapshot_tree(scope.path())?;

    assert_refuses_naming(&run_scaffolder(&working_dir, "create", "..")?, "..");

    assert_eq!(
        parent_before,
        snapshot_tree(scope.path())?,
        "a rejected `..` name must not write anything into the working directory's parent"
    );
    Ok(())
}

/// An absolute path used as a name must not be honoured as a destination.
///
/// The absolute path points inside this test's own scope, so nothing else
/// can create it, and a leftover from an earlier run cannot exist.
#[test]
fn create_rejects_an_absolute_path_as_a_name() -> TestOutcome {
    let scope = TempDir::new("create-absolute-path")?;
    let working_dir = scope.path().join("working");
    fs::create_dir(&working_dir).context("creating the nested working directory failed")?;
    let absolute_target = scope.path().join("absolute-escape-target");
    let absolute_name = path_to_str(&absolute_target)?;

    assert_refuses_naming(
        &run_scaffolder(&working_dir, "create", absolute_name)?,
        absolute_name,
    );

    assert!(
        !absolute_target.exists(),
        "a rejected absolute-path name must not create anything at that path"
    );
    Ok(())
}

/// A name that collides with something already on disk must be refused
/// rather than overwritten.
#[test]
fn create_rejects_a_name_that_collides_with_something_already_on_disk() -> TestOutcome {
    let working_dir = TempDir::new("create-collision")?;
    let colliding_path = working_dir.path().join("already-here");
    let original_contents = b"do not touch: pre-existing, unrelated to ritual\n";
    fs::write(&colliding_path, original_contents)
        .context("writing the colliding fixture failed")?;

    let result = run_scaffolder(working_dir.path(), "create", "already-here")?;
    result.expect_failure("create with a name colliding with an existing entry");
    assert!(
        result.stderr.contains("already-here"),
        "expected the refusal to name the colliding entry; stderr was:\n{}",
        result.stderr
    );

    let contents_after =
        fs::read(&colliding_path).context("reading the colliding fixture back failed")?;
    assert_eq!(
        contents_after, original_contents,
        "a rejected colliding name must leave the pre-existing entry byte for byte untouched"
    );
    Ok(())
}

/// `new` validates names by the same one rule as `create` and `add`, so one
/// representative case covers it here.
#[test]
fn new_rejects_a_name_containing_a_path_separator() -> TestOutcome {
    let working_dir = TempDir::new("new-path-separator")?;
    let before = snapshot_tree(working_dir.path())?;

    let name = "nested/project";
    assert_refuses_naming(&run_scaffolder(working_dir.path(), "new", name)?, name);

    assert_eq!(
        before,
        snapshot_tree(working_dir.path())?,
        "a rejected name must not create `nested/` or anything under it"
    );
    Ok(())
}
