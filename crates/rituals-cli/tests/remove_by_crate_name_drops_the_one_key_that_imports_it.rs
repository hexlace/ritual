//! `cargo ritual remove <crate>` finds the task by the crate it imports,
//! whatever key it is mounted under, and removes it as `remove <key>` would.
//!
//! The fixture is a hand-written workspace member whose crate name differs
//! from the key it is mounted under, so the name the person types is never
//! a key in `tasks`.

mod support;

use support::removal::{assert_help_lists, exists, members_of, project_with_a_committed_task};
use support::{Project, TempDir, TestOutcome, generated, git, in_checkout, manifest};

#[test]
fn removing_a_task_by_its_crate_name_drops_the_key_that_imports_it() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-by-crate-name")?;
        let project = a_project_with_chore_task_mounted_as_chore(checkout, &working_dir)?;

        let members_before = members_of(&project)?;
        assert!(
            members_before
                .iter()
                .any(|member| member == ".rituals/chore-task"),
            "fixture precondition: .rituals/chore-task should be a member; members were \
             {members_before:?}"
        );

        project
            .run_cli(&["remove", "chore-task"])?
            .expect_success("`cargo ritual remove chore-task`, by crate name");

        assert_the_chore_key_and_directory_are_gone(&project)?;

        let generated_file = project.generated_file()?;
        assert_eq!(
            generated::mounted_entries(&generated_file),
            [
                ("ritual".to_string(), "ritual".to_string()),
                ("greet".to_string(), "greet".to_string()),
            ],
            "expected the regenerated file to mount what is left; file was:\n{generated_file}"
        );

        assert_help_lists(
            &project,
            "`remove chore-task`",
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
        )
    })
}

/// A committed project with `greet` and a hand-written `chore-task` crate
/// mounted under the key `chore`, so the crate's name is never a key.
fn a_project_with_chore_task_mounted_as_chore(
    checkout: &support::Checkout,
    working_dir: &TempDir,
) -> support::Outcome<Project> {
    let project = project_with_a_committed_task(checkout, working_dir, "greet")?;
    let crate_dir = project.write_leaf("chore-task")?;
    project.mount(&crate_dir, "chore", "chore-task")?;
    project
        .alias(&["regenerate"])?
        .expect_success("`cargo ritual regenerate` after mounting chore-task as `chore`");
    git::commit_everything(project.root())?;
    Ok(project)
}

/// Only the key importing `chore-task` has left, and its dependency line,
/// member entry and directory with it; the other task is untouched.
#[track_caller]
fn assert_the_chore_key_and_directory_are_gone(project: &Project) -> TestOutcome {
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
        !members_of(project)?
            .iter()
            .any(|member| member == ".rituals/chore-task"),
        "expected .rituals/chore-task to leave `[workspace] members`"
    );
    assert!(
        !exists(&project.root().join(".rituals/chore-task")),
        "expected .rituals/chore-task to be deleted"
    );
    assert!(
        exists(&project.root().join("tasks/greet/Cargo.toml")),
        "expected the other task's directory to stay"
    );
    Ok(())
}
