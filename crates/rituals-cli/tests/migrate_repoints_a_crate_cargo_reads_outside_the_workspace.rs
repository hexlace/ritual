//! `migrate` repoints every manifest Cargo reads, not only the packages
//! `cargo metadata` lists.
//!
//! A crate kept out of the workspace and reached only through an optional
//! dependency that no feature turns on is read by Cargo when it resolves the
//! project, and it is not in `cargo metadata`'s packages. If it depends on a
//! task, the move has to repoint it too, or Cargo can no longer read the
//! project.

mod support;

use support::crates::write_crate;
use support::migration::{assert_dependency_leads_to, assert_names, exists};
use support::{TempDir, TestOutcome, git, in_checkout, manifest};

#[test]
fn an_excluded_crate_reached_only_through_an_optional_dependency_is_repointed() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-excluded-optional")?;
        let project = support::legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        write_crate(
            &project.root().join("vendor/x"),
            "[package]\nname = \"x\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [dependencies]\ngreet = { path = \"../../tasks/greet\" }\n",
            "//! A crate kept out of the workspace.\n",
        )?;
        manifest::edit(&project.workspace_manifest_path(), |document| {
            manifest::set_workspace_exclude(document, &["vendor/x"])
        })?;
        manifest::edit(&project.cli_manifest_path(), |document| {
            manifest::add_path_dependency(document, &["dependencies"], "x", "../vendor/x")?;
            manifest::mark_optional(document, &["dependencies"], "x")
        })?;
        let listed = project.cargo(&["metadata", "--format-version", "1"])?;
        listed.expect_success("`cargo metadata` on the fixture");
        assert!(
            !listed.stdout.contains("vendor/x/Cargo.toml"),
            "fixture precondition: cargo metadata must not list the excluded crate"
        );
        project.build()?;
        git::init_and_commit_everything(project.root())?;

        let migrated = project.run_cli(&["migrate"])?;

        migrated.expect_success("`migrate` with a crate outside the workspace that reaches a task");
        assert!(
            exists(&project.root().join(".rituals/greet/Cargo.toml")),
            "expected the task to move to .rituals/greet"
        );
        assert_names(&migrated, "`migrate`", &["vendor/x/Cargo.toml"]);
        assert_dependency_leads_to(
            &project.root().join("vendor/x/Cargo.toml"),
            &["dependencies"],
            "greet",
            &project.root().join(".rituals/greet"),
        )?;
        project
            .cargo(&["metadata", "--format-version", "1"])?
            .expect_success("`cargo metadata` after `migrate`");
        project.build()?;
        Ok(())
    })
}
