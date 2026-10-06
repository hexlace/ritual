//! `new` and `create` outside a project make a directory and then fill it.
//! When a write fails after the directory exists, nothing of theirs is left
//! behind, and the failure ends by saying what was removed so a retry starts
//! clean.
//!
//! `create` inside a project edits a project that was already there, so its
//! failure ends by saying the project was put back as it was found. The
//! failure here is a workspace manifest Cargo refuses ritual write access to,
//! a read-only file: a process that ignores permission bits (root) cannot
//! make one, and the story says it was skipped.
//!
//! `new` and the standalone `create` are made to fail by a limit on how big
//! a file the process may write, with the signal that kills a process that
//! passes it ignored, so a write fails with an error the way it does on a
//! full disk. The limit is no bytes at all: making a directory writes no
//! file, so the directory exists when the first file cannot be written.

mod support;

use std::path::Path;

use support::created::the_refusal;
use support::migration::made_file_read_only;
use support::process::ritual_binary;
use support::{Project, TempDir, TestOutcome, in_checkout, run_binary, snapshot_tree};

#[test]
fn a_create_that_cannot_write_the_workspace_manifest_puts_the_project_back() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("create-fails-workspace-manifest")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let binary = project.build()?;
        let manifest_path = project.workspace_manifest_path();
        if !made_file_read_only(&manifest_path)? {
            support::checkout::report_skip(
                "a `create` that fails writing a read-only workspace Cargo.toml could not be \
                 demonstrated because this process does not honour the read-only permission bit",
            );
            return Ok(());
        }
        let before = snapshot_tree(project.root())?;

        let failed = run_binary(&binary, project.root(), &["create", "lint"])?;

        let message = the_refusal(&failed, "ritual");
        assert!(
            message.contains("Cargo.toml"),
            "expected the failure to name the manifest it could not write; message was:\n{message}"
        );
        assert!(
            message.ends_with("; ritual put the project back as it found it"),
            "expected the failure to say the project was put back; message was:\n{message}"
        );
        assert_eq!(
            before,
            snapshot_tree(project.root())?,
            "a failed `create` must leave the project as it found it"
        );
        Ok(())
    })
}

/// Runs the `ritual` this suite was built with in `directory`, under a limit
/// of no bytes on the size of any file it writes, with the signal that would
/// kill it for crossing the limit ignored.
fn run_ritual_limited_in_file_size(
    directory: &Path,
    arguments: &[&str],
) -> support::Outcome<support::RunOutput> {
    let script = "trap '' XFSZ; ulimit -f 0; exec \"$0\" \"$@\"";
    let mut shell_arguments = vec!["-c", script, support::path_to_str(ritual_binary())?];
    shell_arguments.extend_from_slice(arguments);
    run_binary(Path::new("/bin/sh"), directory, &shell_arguments)
}

#[test]
fn a_new_that_fails_while_writing_removes_the_project_directory() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("new-fails-while-writing")?;
        let before = snapshot_tree(working_dir.path())?;

        let failed = run_ritual_limited_in_file_size(
            working_dir.path(),
            &["new", "demo", "--path", checkout.path_argument()?],
        )?;

        let message = the_refusal(&failed, "ritual");
        assert!(
            message.ends_with("; ritual removed demo so a retry starts clean"),
            "expected the failure to say the project directory was removed; message was:\n{message}"
        );
        assert!(
            message.contains("writing "),
            "expected the failure to be a write that failed, not something that stopped the run \
             earlier; message was:\n{message}"
        );
        assert_eq!(before, snapshot_tree(working_dir.path())?);
        Ok(())
    })
}

#[test]
fn a_standalone_create_that_fails_while_writing_removes_its_directory() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("create-fails-while-writing")?;
        let before = snapshot_tree(working_dir.path())?;

        let failed = run_ritual_limited_in_file_size(
            working_dir.path(),
            &["create", "lint", "--path", checkout.path_argument()?],
        )?;

        let message = the_refusal(&failed, "ritual");
        assert!(
            message.ends_with("; ritual removed lint so a retry starts clean"),
            "expected the failure to say the crate directory was removed; message was:\n{message}"
        );
        assert!(
            message.contains("writing "),
            "expected the failure to be a write that failed; message was:\n{message}"
        );
        assert_eq!(before, snapshot_tree(working_dir.path())?);
        Ok(())
    })
}
