//! When `remove` deletes a task's directory and the deletion fails partway,
//! it says how to get the files back, and that advice works from wherever
//! the person is in the project, as `cargo ritual` itself does.
//!
//! The advice is a `git checkout` of the directory spelled from git's top
//! level with the `:/` pathspec magic, so running it from `ritual/` puts the
//! files back exactly as committed. A plain `tasks/greet` would be read from
//! `ritual/` and give back nothing.
//!
//! The failure is a read-only `src/` inside the task: `remove` deletes what
//! it can around it, then cannot empty it. A process that ignores the
//! read-only bit (root, on Unix) cannot be made to fail this way, so the
//! story says so and skips, as the repository's other permission-based
//! checks do.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use support::process::run_binary;
use support::removal::{assert_a_refusal, project_with_a_committed_task};
use support::{TempDir, TestOutcome, assert_trees_identical, git, in_checkout, snapshot_tree};

#[test]
fn the_advice_after_a_failed_deletion_gives_the_files_back_from_a_subdirectory() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-advice-from-a-subdirectory")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        let bin_name = project.bin_name()?;
        let binary = project.build()?;
        let task = project.root().join("tasks/greet");
        let committed = snapshot_tree(&task)?;

        let source = task.join("src");
        if !made_read_only(&source)? {
            support::checkout::report_skip(
                "a failed deletion could not be demonstrated because this process does not \
                 honour the read-only permission bit",
            );
            return Ok(());
        }
        let subdirectory = project.composed_cli_dir();
        let failed = run_binary(&binary, subdirectory, &["remove", "greet"]);
        // Writable again before any assertion can return early.
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755))?;
        let failed = failed?;

        let message = assert_a_refusal(&failed, &bin_name, "`remove greet` with a read-only src/");
        assert!(
            message.contains("failed partway"),
            "expected the deletion to fail partway; stderr was:\n{message}"
        );
        let advice = advice_in(message)
            .ok_or_else(|| format!("expected a `git checkout` to run; stderr was:\n{message}"))?;
        assert!(
            committed != snapshot_tree(&task)?,
            "fixture precondition: the failed deletion must have deleted something"
        );

        let arguments: Vec<&str> = advice.split_whitespace().skip(1).collect();
        git::git(subdirectory, &arguments)?
            .expect_success(&format!("`{advice}` run from the composed CLI's directory"));

        assert_trees_identical(
            &format!("`{advice}` run from a subdirectory must give back every file"),
            &committed,
            &snapshot_tree(&task)?,
        );
        Ok(())
    })
}

/// The `git checkout …` command the message quotes in backticks.
fn advice_in(message: &str) -> Option<&str> {
    let start = message.find("`git checkout")? + 1;
    let length = message[start..].find('`')?;
    Some(&message[start..start + length])
}

/// Makes the directory at `path` read-only and reports whether that is
/// enforced, by trying to create a file in it. When it is not (the process
/// is root, say), it is made writable again before returning `false`.
fn made_read_only(path: &Path) -> Result<bool, std::io::Error> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o555))?;
    let probe = path.join(".write-probe");
    let enforced = std::fs::File::create(&probe).is_err();
    if !enforced {
        std::fs::remove_file(&probe)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(enforced)
}
