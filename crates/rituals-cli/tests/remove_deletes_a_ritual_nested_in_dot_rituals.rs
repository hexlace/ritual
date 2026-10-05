//! `remove <key>` takes a ritual out of the project completely, and a ritual
//! nested below `.rituals/` at any depth goes the same way as one at
//! `.rituals/<name>`: its `tasks` entry, dependency line, directory and
//! `[workspace] members` entry are gone, and the command line still builds.
//!
//! The fixtures are committed to a git repository first, as a person would
//! have them: `remove` deletes a ritual's directory only when git can give
//! it back. Rituals beside the removed one are left alone.

mod support;

use support::nested::mounted_ritual;
use support::removal::{exists, members_of};
use support::{Project, TempDir, TestOutcome, git, help, in_checkout, manifest};

#[test]
fn removing_a_nested_ritual_deletes_its_directory_and_member_entry() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-nested")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        mounted_ritual(&project, ".rituals/private/lint", "lint")?;
        mounted_ritual(&project, ".rituals/a/b/greet", "greet")?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;

        for (key, directory) in [
            ("lint", ".rituals/private/lint"),
            ("greet", ".rituals/a/b/greet"),
        ] {
            project
                .run_cli(&["remove", key])?
                .expect_success(&format!("`remove {key}`, a ritual in {directory}"));

            assert!(
                !exists(&project.root().join(directory)),
                "expected {directory} to be deleted"
            );
            let members = members_of(&project)?;
            assert!(
                !members.iter().any(|member| member == directory),
                "expected {directory} to leave `[workspace] members`; members were {members:?}"
            );
            let cli_manifest = project.cli_manifest()?;
            assert!(
                !manifest::tasks(&cli_manifest)?.contains(&key.to_string()),
                "expected `{key}` to leave `tasks`; manifest was:\n{cli_manifest}"
            );
            assert!(
                manifest::lookup(&cli_manifest, &["dependencies", key]).is_none(),
                "expected the `{key}` dependency line to be gone; manifest was:\n{cli_manifest}"
            );
            help::assert_refuses_unrecognized_subcommand(&project.run_cli(&[key])?, key);
        }
        Ok(())
    })
}

#[test]
fn removing_one_nested_ritual_leaves_its_neighbour_and_a_project_that_builds() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-nested-neighbour")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        mounted_ritual(&project, ".rituals/private/lint", "lint")?;
        mounted_ritual(&project, ".rituals/private/tidy", "tidy")?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;

        project
            .run_cli(&["remove", "lint"])?
            .expect_success("`remove lint`, beside a neighbour in .rituals/private");

        assert!(
            exists(&project.root().join(".rituals/private/tidy/Cargo.toml")),
            "expected the neighbouring ritual to stay"
        );
        let ran = project.run_cli(&["tidy"])?;
        ran.expect_success("the neighbour's `tidy` command, after the removal");
        assert!(
            ran.stdout.contains("tidy ran"),
            "expected `tidy` to answer; stdout was:\n{}",
            ran.stdout
        );
        assert_members_hold(&project, ".rituals/private/tidy")
    })
}

#[track_caller]
fn assert_members_hold(project: &Project, member: &str) -> TestOutcome {
    let members = members_of(project)?;
    assert!(
        members.iter().any(|listed| listed == member),
        "expected {member} to stay a member; members were {members:?}"
    );
    Ok(())
}
