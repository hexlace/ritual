//! `cargo ritual remove <crate>` finds the task by the crate it imports,
//! whatever key it is mounted under, and removes it as `remove <key>` would.
//!
//! The fixture is a hand-written workspace member whose crate name differs
//! from the key it is mounted under, so the name the person types is never
//! a key in `tasks`.

mod support;

use support::removal::{exists, members_of, project_with_a_committed_task};
use support::{TempDir, TestOutcome, generated, git, help, in_checkout, manifest};

#[test]
fn removing_a_task_by_its_crate_name_drops_the_key_that_imports_it() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-by-crate-name")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;

        let crate_dir = project.write_leaf("chore-task")?;
        project.mount(&crate_dir, "chore", "chore-task")?;
        project
            .alias(&["regenerate"])?
            .expect_success("`cargo ritual regenerate` after mounting chore-task as `chore`");
        git::commit_everything(project.root())?;

        let members_before = members_of(&project)?;
        assert!(
            members_before
                .iter()
                .any(|member| member == "tasks/chore-task"),
            "fixture precondition: tasks/chore-task should be a member; members were \
             {members_before:?}"
        );

        project
            .run_cli(&["remove", "chore-task"])?
            .expect_success("`cargo ritual remove chore-task`, by crate name");

        let cli_manifest = project.cli_manifest()?;
        assert_eq!(
            manifest::tasks(&cli_manifest)?,
            ["ritual", "greet"],
            "expected only the key importing chore-task to leave `tasks`; manifest \
             was:\n{cli_manifest}"
        );
        assert!(
            manifest::lookup(&cli_manifest, &["dependencies", "chore"]).is_none(),
            "expected the `chore` dependency line to be gone; manifest was:\n{cli_manifest}"
        );
        assert!(
            manifest::lookup(&cli_manifest, &["dependencies", "greet"]).is_some(),
            "expected the other task's dependency line to stay; manifest was:\n{cli_manifest}"
        );

        assert!(
            !members_of(&project)?
                .iter()
                .any(|member| member == "tasks/chore-task"),
            "expected tasks/chore-task to leave `[workspace] members`"
        );
        assert!(
            !exists(&project.root().join("tasks/chore-task")),
            "expected tasks/chore-task to be deleted"
        );
        assert!(
            exists(&project.root().join("tasks/greet/Cargo.toml")),
            "expected the other task's directory to stay"
        );

        let generated_file = project.generated_file()?;
        assert_eq!(
            generated::mounted_entries(&generated_file),
            [
                ("ritual".to_string(), "ritual".to_string()),
                ("greet".to_string(), "greet".to_string()),
            ],
            "expected the regenerated file to mount what is left; file was:\n{generated_file}"
        );

        let help_output = project.alias(&["--help"])?;
        help_output.expect_success("`cargo ritual --help` after `remove chore-task`");
        assert_eq!(
            help::command_names(&help_output.stdout),
            [
                "add",
                "regenerate",
                "new",
                "create",
                "remove",
                "greet",
                "help"
            ],
            "stdout was:\n{}",
            help_output.stdout
        );

        Ok(())
    })
}
