//! What `migrate` does after its changes are kept cannot undo them: when
//! deleting the directory the moves emptied fails, the run says what failed
//! and still succeeds, because every task has already moved and Cargo reads
//! the project as it should.
//!
//! The failure is made from outside the process, with a project root that
//! cannot be written to: the tasks move into a `.rituals/` that was already
//! there, so nothing before the cleanup needs the root, and the empty
//! `tasks/` then cannot be deleted from it. A process that ignores permission
//! bits (root, on Unix) cannot make this, so the story says so and skips, as
//! the repository's other permission-based checks do.

mod support;

use support::migration::{assert_names, exists, made_directory_read_only, made_writable};
use support::{TempDir, TestOutcome, git, in_checkout, legacy};

#[test]
fn a_directory_it_cannot_delete_afterwards_is_reported_and_the_migration_stands() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-cleanup-fails")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet", "shout"])?;
        // An empty directory is not something git tracks, so the work tree
        // stays clean, and `migrate` finds the place it moves tasks into
        // already there.
        std::fs::create_dir_all(project.root().join(".rituals"))?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;
        if !made_directory_read_only(project.root())? {
            support::checkout::report_skip(
                "a `migrate` whose cleanup fails could not be demonstrated because this process \
                 does not honour the read-only permission bit",
            );
            return Ok(());
        }

        let migrated = project.run_cli(&["migrate"]);

        // Before anything is asserted, so a failure cannot leave a project
        // that cannot be cleaned up.
        made_writable(project.root())?;
        let migrated = migrated?;
        migrated.expect_success("`migrate`, whose cleanup failed after the tasks had moved");
        assert_names(
            &migrated,
            "`migrate`",
            &["tasks/", ".rituals/greet", ".rituals/shout", "by hand"],
        );
        for task in ["greet", "shout"] {
            assert!(
                exists(
                    &project
                        .root()
                        .join(".rituals")
                        .join(task)
                        .join("Cargo.toml")
                ),
                "expected {task} to have moved and stayed moved"
            );
            assert!(
                !exists(&project.root().join("tasks").join(task)),
                "expected {task} to have left tasks/"
            );
        }
        assert!(
            exists(&project.root().join("tasks")),
            "expected the directory that could not be deleted to be left in place"
        );
        project
            .cargo(&["build", "--workspace"])?
            .expect_success("`cargo build --workspace` after `migrate`");
        project
            .run_cli(&["greet"])?
            .expect_success("`greet` after a `migrate` whose cleanup failed");
        Ok(())
    })
}
