//! `cargo ritual remove <key>` takes a task `add` scaffolded out of the
//! project completely — its `tasks` entry, its dependency line, its
//! directory and its `[workspace] members` entry — and leaves a command line
//! that builds, in the one order Cargo allows.
//!
//! Done by hand, those steps only work in a fixed order: drop the key,
//! regenerate while the dependency still resolves, then drop the dependency.
//! Any other order leaves a generated file that names a crate Cargo can no
//! longer find, so the project stops building and `regenerate` cannot run to
//! repair it. Here the project builds again afterwards, and the removed
//! command is gone from `--help`.
//!
//! The fixture is committed to a git repository first, as a person would
//! have it: `remove` deletes the task's directory only when git can give it
//! back.

mod support;

use support::removal::{
    assert_help_lists, exists, members_of, project_with_a_committed_added_task,
};
use support::{Project, TempDir, TestOutcome, generated, help, in_checkout, manifest, write_text};

#[test]
fn removing_a_scaffolded_task_by_key_deletes_it_and_leaves_a_project_that_builds() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-scaffolded-by-key")?;
        let project = project_with_a_committed_added_task(checkout, &working_dir, "greet")?;

        // Something the person is working on elsewhere in the project: only
        // the task's own files decide whether `remove` may delete the task.
        write_text(&project.root().join("notes.txt"), "not part of any task\n")?;

        let members_before = members_of(&project)?;
        assert!(
            members_before
                .iter()
                .any(|member| member == ".rituals/greet"),
            "fixture precondition: `add greet` should have made .rituals/greet a member; members \
             were {members_before:?}"
        );
        assert!(
            exists(&project.root().join(".rituals/greet/Cargo.toml")),
            "fixture precondition: .rituals/greet should exist"
        );

        let removed = project.run_cli(&["remove", "greet"])?;
        removed.expect_success("`cargo ritual remove greet`");

        assert_the_task_left_the_manifests_and_the_disk(&project, &members_before)?;

        // The generated file no longer mounts it, and what the person had
        // lying around is untouched.
        let generated_file = project.generated_file()?;
        assert_eq!(
            generated::mounted_entries(&generated_file),
            [("ritual".to_string(), "ritual".to_string())],
            "expected the regenerated file to mount the bundle alone; file was:\n{generated_file}"
        );
        assert!(
            exists(&project.root().join("notes.txt")),
            "expected the unrelated file outside the task to be left alone"
        );

        // The order that used to break the build now works: the project's
        // command line builds, and the removed command is gone from it.
        assert_help_lists(
            &project,
            "`remove greet`",
            &[
                "add",
                "regenerate",
                "new",
                "create",
                "import",
                "remove",
                "help",
            ],
        )?;
        help::assert_refuses_unrecognized_subcommand(&project.run_cli(&["greet"])?, "greet");

        Ok(())
    })
}

/// The key, the dependency line, the member entry and the directory of
/// `greet` are gone; every other member stays.
#[track_caller]
fn assert_the_task_left_the_manifests_and_the_disk(
    project: &Project,
    members_before: &[String],
) -> TestOutcome {
    // The key and the dependency line are gone from the command line's
    // manifest.
    let cli_manifest = project.cli_manifest()?;
    assert_eq!(
        manifest::tasks(&cli_manifest)?,
        ["ritual"],
        "expected `greet` to leave `tasks`; manifest was:\n{cli_manifest}"
    );
    assert!(
        manifest::lookup(&cli_manifest, &["dependencies", "greet"]).is_none(),
        "expected the `greet` dependency line to be gone; manifest was:\n{cli_manifest}"
    );

    // The directory and the member entry are gone with it.
    let expected_members: Vec<String> = members_before
        .iter()
        .filter(|member| member.as_str() != ".rituals/greet")
        .cloned()
        .collect();
    assert_eq!(
        members_of(project)?,
        expected_members,
        "expected only `.rituals/greet` to leave `[workspace] members`"
    );
    assert!(
        !exists(&project.root().join(".rituals/greet")),
        "expected .rituals/greet to be deleted"
    );
    Ok(())
}
