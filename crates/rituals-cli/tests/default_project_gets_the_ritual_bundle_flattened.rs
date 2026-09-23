//! A project scaffolded with no `--cli` carries ritual's management bundle —
//! `add`, `regenerate`, `new`, `create` — as one import under the key
//! `ritual`, flattened into its top level because the project's own bin is
//! named `ritual` too. Hand-renaming that `[[bin]]` and rebuilding nests the
//! bundle under `ritual` on the next build, with no other file touched.
//!
//! Also: the workspace's dependency table names only `rituals`, and
//! `regenerate` run immediately after `new`, or immediately after `add`,
//! with no other change, changes nothing.

mod support;

use support::{
    Project, TempDir, TestOutcome, assert_trees_identical, help, in_checkout, run_binary,
    snapshot_tree,
};
use support::{generated, manifest};

/// The composed CLI's own manifest: a `[[bin]]` named `ritual`, a
/// `rituals` dependency inherited from the workspace, ritual's bundle
/// imported under `ritual` and nothing else, and `ritual` as its one task.
fn assert_cli_manifest_shape(project: &Project) -> TestOutcome {
    let cli_manifest = project.cli_manifest()?;

    manifest::assert_sole_bin_at_main_rs(&cli_manifest, "ritual");
    assert_eq!(
        manifest::keys_of(&cli_manifest, &["dependencies"]),
        ["rituals", "ritual"],
        "expected the composed CLI to depend on `rituals` and ritual's bundle alone; \
         manifest was:\n{cli_manifest}"
    );
    assert_eq!(
        manifest::lookup(&cli_manifest, &["dependencies", "rituals", "workspace"])
            .and_then(toml_edit::Item::as_bool),
        Some(true),
        "expected `rituals` to come from the workspace; manifest was:\n{cli_manifest}"
    );
    assert_eq!(
        manifest::string_at(&cli_manifest, &["dependencies", "ritual", "package"]),
        Some("rituals-core"),
        "expected the key `ritual` to import ritual's bundle; manifest was:\n{cli_manifest}"
    );
    assert_eq!(manifest::tasks(&cli_manifest)?, ["ritual"]);

    Ok(())
}

/// The workspace manifest's `[workspace.dependencies]` names `rituals` and
/// nothing else.
fn assert_workspace_manifest_shape(project: &Project) -> TestOutcome {
    let workspace_manifest = project.workspace_manifest()?;
    assert_eq!(
        manifest::keys_of(&workspace_manifest, &["workspace", "dependencies"]),
        ["rituals"],
        "manifest was:\n{workspace_manifest}"
    );
    Ok(())
}

/// The generated file calls `rituals::run` with `rituals::identity!()`
/// directly, reaches nothing through `rituals_compose`, and mounts exactly
/// one entry, ritual's bundle under `ritual`. Compared with whitespace
/// removed, so the check is about the tokens rather than the layout.
fn assert_generated_file_shape(project: &Project) -> TestOutcome {
    let generated_file = project.generated_file()?;
    let tokens = generated::tokens(&generated_file);

    assert!(
        tokens.contains("rituals::run(rituals::identity!(),["),
        "expected the generated file to call the framework's dispatcher and identity macro \
         directly; file was:\n{generated_file}"
    );
    assert!(
        !tokens.contains("rituals_compose::"),
        "expected the generated file to reach nothing through the composition library; \
         file was:\n{generated_file}"
    );
    assert_eq!(
        generated::mounted_entries(&generated_file),
        [("ritual".to_string(), "ritual".to_string())],
        "expected exactly one mounted entry, ritual's bundle under `ritual`; file \
         was:\n{generated_file}"
    );

    Ok(())
}

/// `regenerate`, with no change since the last write, reports the file up
/// to date and changes nothing on disk.
fn assert_regenerate_changes_nothing(project: &Project, after: &str) -> TestOutcome {
    let before = snapshot_tree(project.root())?;
    let result = project.run_cli(&["regenerate"])?;
    result.expect_success(&format!("`regenerate` immediately after {after}"));
    assert!(
        result.stdout.contains("already up to date"),
        "expected regenerate to report the file already up to date; stdout was:\n{}",
        result.stdout
    );
    assert_trees_identical(
        &format!("regenerating immediately after {after}, with no other change"),
        &before,
        &snapshot_tree(project.root())?,
    );
    Ok(())
}

/// `cargo ritual add my-task` scaffolds and mounts a task on the composed
/// CLI's top level beside the bundle, and `--help` then lists all five
/// commands directly, the bundle's own key not among them.
fn assert_an_added_task_sits_flat_beside_the_bundle(project: &Project) -> TestOutcome {
    project
        .alias(&["add", "my-task"])?
        .expect_success("`cargo ritual add my-task`");

    assert_eq!(
        manifest::tasks(&project.cli_manifest()?)?,
        ["ritual", "my-task"]
    );
    let generated_file = project.generated_file()?;
    assert_eq!(
        generated::mounted_entries(&generated_file),
        [
            ("ritual".to_string(), "ritual".to_string()),
            ("my-task".to_string(), "my_task".to_string()),
        ],
        "expected the generated file to mount `my-task` after ritual's bundle; file \
         was:\n{generated_file}"
    );

    let help = project.alias(&["--help"])?;
    help.expect_success("`cargo ritual --help`");
    assert_eq!(
        help::command_names(&help.stdout),
        ["add", "regenerate", "new", "create", "my-task", "help"],
        "stdout was:\n{}",
        help.stdout
    );
    Ok(())
}

/// Hand-renaming the `[[bin]]` target and rebuilding, with no other file
/// touched, nests ritual's bundle under `ritual`: `<new-name> ritual add`
/// reaches `add`, `<new-name> add` does not, and the generated file is left
/// as it was — flattening is decided from the bin name the binary was built
/// as.
fn assert_renaming_the_bin_nests_the_bundle(project: &Project) -> TestOutcome {
    let generated_before = project.generated_file()?;
    manifest::edit(&project.cli_manifest_path(), |document| {
        manifest::rename_sole_bin(document, "demo-renamed")
    })?;

    let renamed_binary = project.build()?;
    let top_level_help = run_binary(&renamed_binary, project.root(), &["--help"])?;
    top_level_help.expect_success("the renamed binary with `--help`");
    assert_eq!(
        help::command_names(&top_level_help.stdout),
        ["ritual", "my-task", "help"],
        "stdout was:\n{}",
        top_level_help.stdout
    );

    let nested_help = run_binary(&renamed_binary, project.root(), &["ritual", "--help"])?;
    nested_help.expect_success("the renamed binary with `ritual --help`");
    assert_eq!(
        help::command_names(&nested_help.stdout),
        ["add", "regenerate", "new", "create", "help"],
        "stdout was:\n{}",
        nested_help.stdout
    );

    help::assert_refuses_unrecognized_subcommand(
        &run_binary(&renamed_binary, project.root(), &["add", "--help"])?,
        "add",
    );
    assert_eq!(
        project.generated_file()?,
        generated_before,
        "building and running the renamed binary must leave the generated file as it was"
    );
    Ok(())
}

#[test]
fn a_default_project_gets_the_ritual_bundle_flattened_and_renaming_the_bin_nests_it() -> TestOutcome
{
    in_checkout(|checkout| {
        let working_dir = TempDir::new("default-project-bundle")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;

        assert_cli_manifest_shape(&project)?;
        assert_workspace_manifest_shape(&project)?;
        assert_generated_file_shape(&project)?;
        assert_regenerate_changes_nothing(&project, "`new`")?;
        assert_an_added_task_sits_flat_beside_the_bundle(&project)?;
        assert_regenerate_changes_nothing(&project, "`add`")?;
        assert_renaming_the_bin_nests_the_bundle(&project)?;

        Ok(())
    })
}
