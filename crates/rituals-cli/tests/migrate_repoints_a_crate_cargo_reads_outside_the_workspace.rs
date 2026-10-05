//! `migrate` repoints every manifest Cargo reads, not only the packages
//! `cargo metadata` lists.
//!
//! A crate kept out of the workspace and reached only through an optional
//! dependency that no feature turns on is read by Cargo when it resolves the
//! project, and it is not in `cargo metadata`'s packages. If it depends on a
//! task, the move has to repoint it too, or Cargo can no longer read the
//! project.
//!
//! A crate Cargo does not read is not one to repoint: one reached only
//! through a member's `[patch]`, which Cargo ignores, is left as it is.

mod support;

use support::crates::write_crate;
use support::migration::{assert_dependency_leads_to, assert_names, exists};
use support::{
    TempDir, TestOutcome, assert_trees_identical, git, in_checkout, manifest, path_to_str,
    read_text, snapshot_tree, write_text,
};

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

/// Cargo reads `[patch]` only from the workspace's root manifest, and warns
/// that a member's is ignored. A crate a member's `[patch]` alone reaches is
/// one Cargo never reads, so its path into `tasks/` is no reason to touch it,
/// and its sitting where git does not track it is no reason to refuse.
#[test]
fn a_crate_only_a_members_ignored_patch_reaches_is_left_alone() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-member-patch")?;
        let project = support::legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        let untracked = working_dir.path().join("untracked/x");
        write_crate(
            &untracked,
            &format!(
                "[package]\nname = \"x\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
                 [dependencies]\ngreet = {{ path = \"{}\" }}\n\n[workspace]\n",
                path_to_str(&project.root().join("tasks/greet"))?
            ),
            "//! A crate git does not track.\n",
        )?;
        let cli_manifest = project.cli_manifest_path();
        let cli_before = read_text(&cli_manifest)?;
        write_text(
            &cli_manifest,
            &format!(
                "{cli_before}\n[patch.crates-io]\nx = {{ path = \"{}\" }}\n",
                path_to_str(&untracked)?
            ),
        )?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;
        let untracked_before = snapshot_tree(&untracked)?;

        let migrated = project.run_cli(&["migrate"])?;

        migrated.expect_success("`migrate` with a member's `[patch]` into an untracked crate");
        assert!(
            exists(&project.root().join(".rituals/greet/Cargo.toml")),
            "expected the task to move to .rituals/greet"
        );
        assert_trees_identical(
            "`migrate` must leave a crate only an ignored `[patch]` reaches exactly as it was",
            &untracked_before,
            &snapshot_tree(&untracked)?,
        );
        project.build()?;
        Ok(())
    })
}
