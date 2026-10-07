//! A committed `Cargo.lock` can be current without being byte for byte what
//! Cargo writes: a hand-resolved merge conflict can leave one with its blank
//! lines gone or a comment added. `cargo metadata --locked` reads it as it
//! is, and the unlocked `cargo metadata` each of these tasks runs rewrites it
//! into Cargo's own layout. A task that promises to put the project back as it
//! found it records the lockfile before that rewrite, so when it fails the
//! lockfile is put back as it was committed, not as Cargo would have written
//! it.
//!
//! Each story here commits such a lockfile, proves with Cargo that it is one
//! Cargo reformats, and makes one task fail with a refusal it only reaches
//! after its first full `cargo metadata`:
//!
//! - `create` and the deprecated `add`, given the bin's own name, which is
//!   refused once the project is read;
//! - `import`, given a path crate that is not a task, which is refused after
//!   `cargo add` has resolved it and a second `cargo metadata` has read it;
//! - `remove`, given the key that imports ritual's own commands, which is
//!   refused once the key is found in the project's task list;
//! - `migrate`, in a 0.1 project with an untracked file, which is refused
//!   once a step is found to apply and the work tree is not clean.

mod support;

use std::path::{Path, PathBuf};

use support::created::{the_refusal, the_refusal_after_the_notice};
use support::process::cargo_query;
use support::task_sources::write_unmarked_crate;
use support::{
    Checkout, OptionContext, Outcome, Project, TempDir, TestOutcome, git, in_checkout, legacy,
    lockfile, path_to_str, read_text, run_binary, snapshot_tree, tree, write_text,
};

/// What a failed run says last when it kept its promise.
const PUT_BACK: &str = "; ritual put the project back as it found it";

/// The comment a person resolving a merge conflict by hand might leave.
const HAND_RESOLVED: &str = "# resolved by hand after a merge\n";

/// `lockfile` as a hand-resolved merge might leave it: every blank line gone,
/// and a comment at the end. The packages and their versions are untouched.
fn out_of_cargos_layout(lockfile: &str) -> String {
    let mut text = String::new();
    for line in lockfile.lines().filter(|line| !line.is_empty()) {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str(HAND_RESOLVED);
    text
}

/// The project's `Cargo.lock`, which a build has already written.
fn the_lockfile(project: &Project) -> Outcome<Vec<u8>> {
    lockfile(project.root())?.context("fixture precondition: the built project has a Cargo.lock")
}

/// Rewrites the built project's `Cargo.lock` out of Cargo's layout, proves
/// with Cargo that it is current and that Cargo would reformat it, then
/// commits the project whole.
///
/// `cargo metadata --locked` accepting it and leaving it alone is what makes
/// it current. The unlocked `cargo metadata` rewriting it is the control: a
/// lockfile Cargo would leave alone could not tell a task that read the
/// project before its snapshot from one that read it after.
fn commit_a_non_canonical_lockfile(project: &Project) -> TestOutcome {
    let path = project.root().join("Cargo.lock");
    let non_canonical = out_of_cargos_layout(&read_text(&path)?);
    write_text(&path, &non_canonical)?;

    cargo_query(
        project.root(),
        &["metadata", "--locked", "--format-version", "1"],
    )?
    .expect_success("`cargo metadata --locked` on a current lockfile out of Cargo's layout");
    assert_eq!(
        the_lockfile(project)?,
        non_canonical.as_bytes(),
        "fixture precondition: `cargo metadata --locked` must leave the lockfile as it is"
    );

    cargo_query(project.root(), &["metadata", "--format-version", "1"])?
        .expect_success("`cargo metadata` on a current lockfile out of Cargo's layout");
    assert_ne!(
        the_lockfile(project)?,
        non_canonical.as_bytes(),
        "fixture precondition: the unlocked `cargo metadata` must rewrite the lockfile, or a \
         task that read the project before its snapshot could not be told from one that did not"
    );
    write_text(&path, &non_canonical)?;

    git::init_and_commit_everything(project.root())
}

/// Runs `arguments` on the built command line `binary` at the project root,
/// asserts it failed with a refusal containing `refused_for`, that the
/// project, `Cargo.lock` included, is byte for byte as it was before, and
/// that the refusal says it put the project back.
#[track_caller]
fn assert_refused_and_put_back(
    project: &Project,
    binary: &Path,
    arguments: &[&str],
    refused_for: &str,
) -> TestOutcome {
    let what = format!("`{}`", arguments.join(" "));
    let tree_before = snapshot_tree(project.root())?;
    let lockfile_before = the_lockfile(project)?;

    let output = run_binary(binary, project.root(), arguments)?;
    let bin_name = project.bin_name()?;
    // `add` says it is going away before it does `create`'s work, refusals
    // included, so its refusal is the line after that notice.
    let message = if arguments.first() == Some(&"add") {
        the_refusal_after_the_notice(&output, &bin_name)
    } else {
        the_refusal(&output, &bin_name)
    };
    assert!(
        message.contains(refused_for),
        "expected {what} to be refused for `{refused_for}`, the refusal this story chose; it \
         said:\n{message}"
    );
    // The lockfile is held before the message, because a run that rewrote it
    // before its record was taken finds nothing to put back and so does not
    // claim to have put anything back: the claim's absence would answer for
    // the lockfile, and this story is about the lockfile.
    let lockfile_after = the_lockfile(project)?;
    assert!(
        lockfile_after == lockfile_before,
        "a failed {what} must leave Cargo.lock byte for byte as it was committed; it was {} \
         bytes and is now {}, {} the comment a hand-resolved merge left",
        lockfile_before.len(),
        lockfile_after.len(),
        if String::from_utf8_lossy(&lockfile_after).ends_with(HAND_RESOLVED) {
            "still ending on"
        } else {
            "without"
        },
    );
    assert!(
        message.ends_with(PUT_BACK),
        "expected {what} to say it put the project back; it said:\n{message}"
    );
    tree::assert_trees_identical(
        &format!("a failed {what} must leave the project as it found it"),
        &tree_before,
        &snapshot_tree(project.root())?,
    );
    Ok(())
}

/// A default project with `prepare` run on it, built, then given a committed
/// lockfile out of Cargo's layout, and its built command line. The build
/// comes before the lockfile is rewritten, because a build rewrites it into
/// Cargo's layout.
fn project_with_a_non_canonical_lockfile(
    checkout: &Checkout,
    working_dir: &TempDir,
    prepare: impl FnOnce(&Project) -> TestOutcome,
) -> Outcome<(Project, PathBuf)> {
    let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
    prepare(&project)?;
    let binary = project.build()?;
    commit_a_non_canonical_lockfile(&project)?;
    Ok((project, binary))
}

#[test]
fn a_failed_create_leaves_the_lockfile_as_committed() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("lock-create")?;
        let (project, binary) =
            project_with_a_non_canonical_lockfile(checkout, &working_dir, |_| Ok(()))?;
        let bin_name = project.bin_name()?;

        assert_refused_and_put_back(
            &project,
            &binary,
            &["create", &bin_name],
            "is reserved for this command line's own commands",
        )
    })
}

#[test]
fn a_failed_add_leaves_the_lockfile_as_committed() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("lock-add")?;
        let (project, binary) =
            project_with_a_non_canonical_lockfile(checkout, &working_dir, |_| Ok(()))?;
        let bin_name = project.bin_name()?;

        assert_refused_and_put_back(
            &project,
            &binary,
            &["add", &bin_name],
            "is reserved for this command line's own commands",
        )
    })
}

#[test]
fn a_failed_import_leaves_the_lockfile_as_committed() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("lock-import")?;
        let directory = working_dir.path().join("plain");
        write_unmarked_crate(&directory, "plain")?;
        let (project, binary) =
            project_with_a_non_canonical_lockfile(checkout, &working_dir, |_| Ok(()))?;

        assert_refused_and_put_back(
            &project,
            &binary,
            &["import", "plain", "--path", path_to_str(&directory)?],
            "declare `task = true` there first",
        )
    })
}

#[test]
fn a_failed_remove_leaves_the_lockfile_as_committed() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("lock-remove")?;
        let (project, binary) =
            project_with_a_non_canonical_lockfile(checkout, &working_dir, |_| Ok(()))?;

        assert_refused_and_put_back(
            &project,
            &binary,
            &["remove", "ritual"],
            "without them nothing can put it back",
        )
    })
}

#[test]
fn a_failed_migrate_leaves_the_lockfile_as_committed() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("lock-migrate")?;
        let (project, binary) =
            project_with_a_non_canonical_lockfile(checkout, &working_dir, |project| {
                legacy::add_task(project, "greet")
            })?;
        write_text(&project.root().join("scratch.txt"), "not in git yet\n")?;

        assert_refused_and_put_back(
            &project,
            &binary,
            &["migrate"],
            "the work tree has changes that are not committed",
        )
    })
}
