//! A refusal that comes before ritual has changed anything says what is in
//! the way and what to do. It does not say ritual "put the project back as it
//! found it": nothing was moved, so there was nothing to put back, and a
//! person reading that sentence is told a recovery happened that did not.
//!
//! `migrate` refuses on what git says of the work tree only after it has
//! read the project, inside the run that would undo its changes, which makes
//! its refusals the case to read: an uncommitted change, an untracked file,
//! and no repository at all.

mod support;

use support::created::the_refusal;
use support::{Project, TempDir, TestOutcome, git, in_checkout, legacy, snapshot_tree, write_text};

/// Runs `migrate` on `project`, asserts it was refused with `refusal` in its
/// message, that the message does not claim a recovery, and that the tree is
/// byte-identical to what it was.
#[track_caller]
fn assert_refused_without_claiming_a_recovery(project: &Project, refusal: &str) -> TestOutcome {
    let before = snapshot_tree(project.root())?;
    let output = project.run_cli(&["migrate"])?;
    let message = the_refusal(&output, &project.bin_name()?);
    assert!(
        message.contains(refusal),
        "expected the refusal to say `{refusal}`; message was:\n{message}"
    );
    assert!(
        !message.contains("put the project back"),
        "expected a refusal that came before any change not to claim the project was put back; \
         message was:\n{message}"
    );
    assert_eq!(before, snapshot_tree(project.root())?);
    Ok(())
}

#[test]
fn an_uncommitted_change_is_refused_without_a_claim_of_recovery() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("refusal-no-claim-dirty")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        write_text(&project.root().join("notes.txt"), "as committed\n")?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;
        write_text(
            &project.root().join("notes.txt"),
            "changed since the commit\n",
        )?;

        assert_refused_without_claiming_a_recovery(
            &project,
            "refusing to migrate: the work tree has changes that are not committed",
        )
    })
}

#[test]
fn an_untracked_file_is_refused_without_a_claim_of_recovery() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("refusal-no-claim-untracked")?;
        let project = legacy::committed_project_with_tasks(checkout, &working_dir, &["greet"])?;
        write_text(&project.root().join("scratch.txt"), "not in git yet\n")?;

        assert_refused_without_claiming_a_recovery(
            &project,
            "refusing to migrate: the work tree has changes that are not committed",
        )
    })
}

#[test]
fn a_project_outside_a_git_repository_is_refused_without_a_claim_of_recovery() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("refusal-no-claim-no-git")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;

        assert_refused_without_claiming_a_recovery(
            &project,
            "refusing to migrate: this project is not in a git repository",
        )
    })
}
