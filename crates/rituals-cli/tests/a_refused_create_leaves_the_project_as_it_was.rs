//! A refused `create` writes nothing, `Cargo.lock` included. Reading the
//! project with `cargo metadata` creates a lockfile that is missing and
//! rewrites one that has fallen behind, so a command that reads the project
//! before it decides to refuse has to put the lockfile back, or not touch it.
//!
//! Each story first proves, with Cargo, that the lockfile it leaves behind
//! really is one `cargo metadata` would rewrite: otherwise a command that
//! never read the project could not be told from one that read it and put
//! things back.
//!
//! `create` run in a project that is not the one its command line belongs to
//! is refused with the command to run in the right one, as `import` and
//! `remove` refuse, or to make a ritual of its own outside any workspace, and
//! writes nothing there either.

mod support;

use support::created::{
    assert_refused_and_left_alone, leave_no_lockfile, leave_the_lockfile_stale, the_refusal,
};
use support::{Project, TempDir, TestOutcome, in_checkout, lockfile, run_binary, snapshot_tree};

/// The refusal of a name the project already has, as the project says it.
fn already_a_task(name: &str) -> String {
    format!("`{name}` is already a task of `demo-ritual`")
}

/// A project with one task, `greet`, made through `verb`, with its command
/// line built.
fn project_with_greet(
    checkout: &support::Checkout,
    prefix: &str,
    verb: &str,
) -> support::Outcome<(TempDir, Project, std::path::PathBuf)> {
    let working_dir = TempDir::new(prefix)?;
    let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
    project
        .alias(&[verb, "greet"])?
        .expect_success(&format!("`cargo ritual {verb} greet`"));
    let binary = project.build()?;
    Ok((working_dir, project, binary))
}

/// Refuses `<verb> greet` for a name already taken, and asserts the tree
/// and the lockfile are exactly as `unsettle` left them.
fn assert_a_taken_name_leaves_the_lockfile_alone(
    prefix: &str,
    verb: &str,
    unsettle: impl FnOnce(&Project) -> TestOutcome,
) -> TestOutcome {
    in_checkout(|checkout| {
        let (_working_dir, project, binary) = project_with_greet(checkout, prefix, verb)?;
        unsettle(&project)?;
        let lockfile_before = lockfile(project.root())?;

        assert_refused_and_left_alone(
            &project,
            &binary,
            project.root(),
            &[verb, "greet"],
            |message| message.starts_with(&already_a_task("greet")),
        )?;

        assert!(
            lockfile(project.root())? == lockfile_before,
            "a refused `{verb} greet` must leave Cargo.lock as it found it: it was {:?} bytes, \
             and is now {:?}",
            lockfile_before.as_ref().map(Vec::len),
            lockfile(project.root())?.as_ref().map(Vec::len),
        );
        Ok(())
    })
}

#[test]
fn a_refused_create_leaves_a_stale_lockfile_stale() -> TestOutcome {
    assert_a_taken_name_leaves_the_lockfile_alone("create-refused-stale", "create", |project| {
        leave_the_lockfile_stale(project, "greet")
    })
}

#[test]
fn a_refused_create_leaves_no_lockfile_where_there_was_none() -> TestOutcome {
    assert_a_taken_name_leaves_the_lockfile_alone("create-refused-no-lock", "create", |project| {
        leave_no_lockfile(project)
    })
}

/// `add` runs `create`'s in-project path, so the same promise holds for it.
#[test]
fn a_refused_add_leaves_a_stale_lockfile_stale() -> TestOutcome {
    assert_a_taken_name_leaves_the_lockfile_alone("add-refused-stale", "add", |project| {
        leave_the_lockfile_stale(project, "greet")
    })
}

#[test]
fn a_refused_add_leaves_no_lockfile_where_there_was_none() -> TestOutcome {
    assert_a_taken_name_leaves_the_lockfile_alone("add-refused-no-lock", "add", |project| {
        leave_no_lockfile(project)
    })
}

/// A project's own command line, run in another project, is not that
/// project's; so is the global `ritual`, which belongs to none. Both are
/// handed the two ways out: the command to run in the right place, or
/// `create` outside any workspace for a ritual of its own. `--path` changes
/// nothing about that, so it gets the same refusal rather than being told to
/// drop the flag. Nothing is written, `Cargo.lock` included: whose project
/// it is is asked before anything that could write one.
#[test]
fn create_in_a_project_that_is_not_the_command_lines_own_is_refused_with_its_remedy() -> TestOutcome
{
    in_checkout(|checkout| {
        let working_dir = TempDir::new("create-foreign-project")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let other = Project::scaffold(checkout, working_dir.path(), "other", &[])?;
        let binary = project.build()?;
        let before = snapshot_tree(other.root())?;

        // `other` has never been built, so it has no lockfile, and any
        // `cargo metadata` that resolved it would leave one behind.
        assert!(
            !other.root().join("Cargo.lock").exists(),
            "fixture precondition: `other` starts with no Cargo.lock"
        );
        let remedy = "`create` works inside the project this command line belongs to; in your \
                      project, run `cargo ritual create lint` (or `cargo <name> ritual create \
                      lint` if it was made with `--cli <name>`); or, for a ritual of its own, \
                      run create outside any Cargo workspace";
        let with_path = ["create", "lint", "--path", checkout.path_argument()?];

        let foreign = run_binary(&binary, other.root(), &["create", "lint"])?;
        assert_eq!(the_refusal(&foreign, "ritual"), remedy);
        let foreign = run_binary(&binary, other.root(), &with_path)?;
        assert_eq!(the_refusal(&foreign, "ritual"), remedy);

        let global = support::run_ritual(other.root(), &["create", "lint"])?;
        assert_eq!(the_refusal(&global, "ritual"), remedy);
        let global = support::run_ritual(other.root(), &with_path)?;
        assert_eq!(the_refusal(&global, "ritual"), remedy);

        assert_eq!(
            before,
            snapshot_tree(other.root())?,
            "a refused `create` must write nothing in the project it was refused in"
        );
        Ok(())
    })
}
