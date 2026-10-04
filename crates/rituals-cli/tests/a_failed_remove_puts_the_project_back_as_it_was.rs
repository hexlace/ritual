//! A `remove` that fails partway leaves the project as it was: the
//! generated `src/main.rs` and both manifests byte-identical, the task's
//! directory still in place, and a retry once the cause is cleared works.
//!
//! The failure is injected where a person could meet it: the workspace
//! `Cargo.toml` is read-only. Removing a workspace member's task writes that
//! file last — after the `tasks` entry has gone, the generated file has been
//! rewritten and the dependency line has been removed — so by the time the
//! write is refused, every earlier write has already landed and has to be
//! undone. (A failure between exactly two of those steps cannot be induced
//! from outside the process: the first two write the same manifest and the
//! generated file, and each must succeed for the next to be reached.)
//!
//! A process that ignores the read-only bit (root, on Unix) cannot be made to
//! fail this way, so the story says so and skips, as the repository's other
//! permission-based checks do.

mod support;

use support::removal::{
    assert_a_refusal, assert_help_lists, exists, project_with_a_committed_task,
};
use support::{TempDir, TestOutcome, assert_trees_identical, in_checkout, snapshot_tree};

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

#[test]
fn a_remove_that_cannot_write_the_workspace_manifest_puts_everything_back() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-failure-rolls-back")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        let bin_name = project.bin_name()?;

        // Built before the manifest is locked: building is not what is under
        // test, and Cargo may want to touch the lockfile.
        project.build()?;
        let workspace_manifest = project.workspace_manifest_path();
        if !made_read_only(&workspace_manifest)? {
            support::checkout::report_skip(
                "a failed `remove` could not be demonstrated because this process does not \
                 honour the read-only permission bit",
            );
            return Ok(());
        }

        let before = snapshot_tree(project.root())?;
        let failed = project.run_cli(&["remove", "greet"])?;

        // Unlocked before any assertion can return early.
        std::fs::set_permissions(&workspace_manifest, std::fs::Permissions::from_mode(0o644))?;

        let message = assert_a_refusal(
            &failed,
            &bin_name,
            "`remove greet` with a read-only Cargo.toml",
        );
        assert!(
            message.contains("Cargo.toml"),
            "expected the failure to name the file it could not write; stderr was:\n{message}"
        );
        assert_trees_identical(
            "a `remove` that failed must put the generated file, both manifests and the task \
             directory back",
            &before,
            &snapshot_tree(project.root())?,
        );

        // The project still builds and still has the task, and the same
        // command works once the cause is cleared.
        assert_help_lists(
            &project,
            "the failed `remove greet`",
            &[
                "add",
                "regenerate",
                "new",
                "create",
                "import",
                "remove",
                "migrate",
                "greet",
                "help",
            ],
        )?;
        project
            .run_cli(&["remove", "greet"])?
            .expect_success("`remove greet` again, with the manifest writable");
        assert!(
            !exists(&project.root().join("tasks/greet")),
            "expected tasks/greet to be deleted by the retry"
        );
        Ok(())
    })
}

/// Makes `path` read-only and reports whether that is enforced, by trying to
/// open it for writing. When it is not (the process is root, say), the file
/// is made writable again before returning `false`.
fn made_read_only(path: &Path) -> Result<bool, std::io::Error> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444))?;
    let enforced = std::fs::OpenOptions::new().write(true).open(path).is_err();
    if !enforced {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))?;
    }
    Ok(enforced)
}
