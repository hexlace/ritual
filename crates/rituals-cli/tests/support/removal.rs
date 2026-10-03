//! Fixtures and checks shared by the stories about `remove`.

use std::path::Path;

use super::process::run_binary;
use super::{
    Checkout, Outcome, Project, ResultContext, RunOutput, TempDir, TestOutcome,
    assert_trees_identical, git, help, manifest, snapshot_tree,
};

/// A project with one task `add` scaffolded under `tasks/<task>`, committed
/// whole to a git repository of its own.
///
/// What a person has after `cargo ritual add <task>` and a commit — the
/// state in which `remove` is allowed to delete the task's directory.
pub(crate) fn project_with_a_committed_task(
    checkout: &Checkout,
    working_dir: &TempDir,
    task: &str,
) -> Outcome<Project> {
    let project = project_with_an_uncommitted_task(checkout, working_dir, task)?;
    git::init_and_commit_everything(project.root())?;
    Ok(project)
}

/// A project with one task `add` scaffolded under `tasks/<task>`, in a
/// directory that is not a git repository.
pub(crate) fn project_with_an_uncommitted_task(
    checkout: &Checkout,
    working_dir: &TempDir,
    task: &str,
) -> Outcome<Project> {
    let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
    project
        .alias(&["add", task])?
        .expect_success(&format!("`cargo ritual add {task}`"));
    Ok(project)
}

/// Asserts `output` is a refusal ritual wrote — one that starts with the
/// bin name's prefix — rather than clap turning the command away, and
/// returns everything it wrote to stderr.
///
/// A bare non-zero exit cannot tell a refusal from a command that does not
/// exist yet, a crash, or an unrelated failure; the prefix is the framework's
/// own shape for a refusal, and clap's rejection of an unknown subcommand
/// never has it.
#[track_caller]
pub(crate) fn assert_a_refusal<'output>(
    output: &'output RunOutput,
    bin_name: &str,
    what: &str,
) -> &'output str {
    output.expect_failure(what);
    assert!(
        !output.stderr.contains("unrecognized subcommand"),
        "expected {what} to be refused by ritual, but clap did not know the command; stderr \
         was:\n{}",
        output.stderr
    );
    let prefix = format!("{bin_name}: ");
    assert!(
        output.stderr.starts_with(&prefix),
        "expected {what} to be refused with a line prefixed `{prefix}`; stderr was:\n{}",
        output.stderr
    );
    &output.stderr
}

/// Runs `remove <name>` on `project`'s own command line, asserts it was
/// refused and that its message contains every one of `expected`, and
/// asserts the project's whole tree — every manifest, the generated file,
/// every task directory — is byte-identical to what it was.
#[track_caller]
pub(crate) fn assert_remove_is_refused_and_writes_nothing(
    project: &Project,
    name: &str,
    expected: &[&str],
) -> TestOutcome {
    assert_invocation_is_refused_and_writes_nothing(project, &["remove", name], expected)
}

/// The same check for a command line that reaches `remove` some other way,
/// such as under the key a bundle is mounted as: runs `arguments` on the
/// project's own command line.
#[track_caller]
pub(crate) fn assert_invocation_is_refused_and_writes_nothing(
    project: &Project,
    arguments: &[&str],
    expected: &[&str],
) -> TestOutcome {
    let before = snapshot_tree(project.root())?;
    let bin_name = project.bin_name()?;
    let what = format!("`{} {}`", bin_name, arguments.join(" "));

    let output = project.run_cli(arguments)?;
    let message = assert_a_refusal(&output, &bin_name, &what);
    for needle in expected {
        assert!(
            message.contains(needle),
            "expected the refusal of {what} to contain `{needle}`; stderr was:\n{message}"
        );
    }

    assert_trees_identical(
        &format!("a refused {what} must write nothing"),
        &before,
        &snapshot_tree(project.root())?,
    );
    Ok(())
}

/// The same check, with the workspace's `Cargo.lock` held to it too, after
/// `unsettle` has left that lockfile stale or absent.
///
/// The tree snapshot skips `Cargo.lock`, which a build legitimately
/// rewrites, so this builds the command line first, then calls `unsettle`
/// with the lockfile's path, then runs the built binary itself: a refusal
/// must leave the lockfile exactly as `unsettle` did, byte for byte or
/// absent, though `remove` reads the project with `cargo metadata`, which
/// creates or rewrites a lockfile that is missing or behind.
#[track_caller]
pub(crate) fn assert_remove_is_refused_and_leaves_the_lockfile(
    project: &Project,
    name: &str,
    expected: &[&str],
    unsettle: impl FnOnce(&Path) -> TestOutcome,
) -> TestOutcome {
    let binary = project.build()?;
    let lockfile = project.root().join("Cargo.lock");
    unsettle(&lockfile)?;

    let before = snapshot_tree(project.root())?;
    let lockfile_before = read_if_present(&lockfile)?;
    let bin_name = project.bin_name()?;
    let what = format!("`{bin_name} remove {name}`");

    let output = run_binary(&binary, project.root(), &["remove", name])?;
    let message = assert_a_refusal(&output, &bin_name, &what);
    for needle in expected {
        assert!(
            message.contains(needle),
            "expected the refusal of {what} to contain `{needle}`; stderr was:\n{message}"
        );
    }

    assert_trees_identical(
        &format!("a refused {what} must write nothing"),
        &before,
        &snapshot_tree(project.root())?,
    );
    assert!(
        read_if_present(&lockfile)? == lockfile_before,
        "a refused {what} must leave Cargo.lock as it found it: it was {}, and is now {}",
        describe(lockfile_before.as_deref()),
        describe(read_if_present(&lockfile)?.as_deref()),
    );
    Ok(())
}

/// The bytes at `path`, or `None` when there is no file there.
fn read_if_present(path: &Path) -> Outcome<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context(&format!("reading {} failed", path.display())),
    }
}

/// A lockfile's state, for a failure message.
fn describe(bytes: Option<&[u8]>) -> String {
    bytes.map_or_else(
        || "absent".to_string(),
        |bytes| format!("{} bytes", bytes.len()),
    )
}

/// The members of the project's `[workspace]`, as written.
pub(crate) fn members_of(project: &Project) -> Outcome<Vec<String>> {
    manifest::workspace_members(&project.workspace_manifest()?)
        .ok_or_else(|| "the project's workspace manifest lists no members".into())
}

/// Whether `path` exists as anything at all.
pub(crate) fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// Asserts the project's own `--help` succeeds and lists exactly
/// `expected`, in order. `after` says what just happened, for the failure
/// message: "`remove greet`" reads as "after `remove greet`".
#[track_caller]
pub(crate) fn assert_help_lists(project: &Project, after: &str, expected: &[&str]) -> TestOutcome {
    let help_output = project.alias(&["--help"])?;
    help_output.expect_success(&format!("`cargo ritual --help` after {after}"));
    assert_eq!(
        help::command_names(&help_output.stdout),
        expected,
        "stdout was:\n{}",
        help_output.stdout
    );
    Ok(())
}
