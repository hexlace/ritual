//! `add` and `regenerate` find and rewrite a project's own generated
//! command-line file when the composed CLI's package name and its bin name
//! are two different strings — the default shape `new` scaffolds (package
//! `<name>-ritual`, bin `ritual`). Both tasks learn which package is theirs
//! from the command line's own identity, the way any task can.
//!
//! The first assertion — package and bin really are two different strings —
//! is what makes the story exercise that case rather than pass for a
//! project where the two happen to agree.

mod support;

use support::{Project, TempDir, TestOutcome, assert_trees_identical, in_checkout, snapshot_tree};
use support::{generated, manifest};

const TASK: &str = "identity-fixture-task";

/// Asserts that the scaffolded composed CLI's package name and bin name
/// differ.
fn assert_package_and_bin_differ(project: &Project) -> TestOutcome {
    let cli_manifest = project.cli_manifest()?;
    let package_name = manifest::package_name(&cli_manifest)?;
    let bin_name = manifest::sole_bin_name(&cli_manifest)?;
    assert_ne!(
        package_name, bin_name,
        "expected the scaffolded package name and [[bin]] name to differ — this story \
         exercises that case; manifest was:\n{cli_manifest}"
    );
    Ok(())
}

/// `add` scaffolded the task crate, imported it on the composed CLI, and
/// regenerated the generated file to mount it.
fn assert_add_scaffolded_imported_and_regenerated(project: &Project) -> TestOutcome {
    assert!(
        project
            .root()
            .join(format!("tasks/{TASK}/Cargo.toml"))
            .is_file(),
        "expected `add` to scaffold tasks/{TASK}/Cargo.toml"
    );

    let cli_manifest = project.cli_manifest()?;
    assert!(
        manifest::keys_of(&cli_manifest, &["dependencies"]).contains(&TASK.to_string()),
        "expected `add` to import `{TASK}` as a dependency; manifest was:\n{cli_manifest}"
    );
    assert!(
        manifest::tasks(&cli_manifest)?.contains(&TASK.to_string()),
        "expected `add` to list `{TASK}` in [package.metadata.ritual] tasks; manifest \
         was:\n{cli_manifest}"
    );

    let generated_file = project.generated_file()?;
    assert_eq!(
        generated::mounted_entries(&generated_file),
        [
            ("ritual".to_string(), "ritual".to_string()),
            (TASK.to_string(), TASK.replace('-', "_")),
        ],
        "expected `add` to have regenerated the generated file to mount `{TASK}` after \
         ritual's bundle; file was:\n{generated_file}"
    );

    Ok(())
}

/// `regenerate`, immediately after `add` and with no other change, changes
/// nothing.
fn assert_regenerate_after_add_changes_nothing(project: &Project) -> TestOutcome {
    let snapshot_after_add = snapshot_tree(project.root())?;
    project
        .run_cli(&["regenerate"])?
        .expect_success("`regenerate` immediately after `add`");
    let snapshot_after_regenerate = snapshot_tree(project.root())?;
    assert_trees_identical(
        "regenerating immediately after add, with no other change",
        &snapshot_after_add,
        &snapshot_after_regenerate,
    );
    Ok(())
}

/// The newly added task runs through the rebuilt binary and reports its
/// scaffold's default line.
fn assert_the_added_task_runs(project: &Project) -> TestOutcome {
    let result = project.run_cli(&[TASK])?;
    result.expect_success(&format!("the rebuilt binary, running `{TASK}`"));
    assert!(
        result
            .stdout
            .contains(&format!("{TASK} has nothing to do yet")),
        "expected the freshly scaffolded task's default body to report itself; stdout \
         was:\n{}",
        result.stdout
    );
    Ok(())
}

#[test]
fn add_scaffolds_and_regenerate_stays_correct_when_package_and_bin_differ() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("add-regenerate-package-ne-bin")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;

        assert_package_and_bin_differ(&project)?;
        project
            .run_cli(&["add", TASK])?
            .expect_success(&format!("`add {TASK}`"));
        assert_add_scaffolded_imported_and_regenerated(&project)?;
        assert_regenerate_after_add_changes_nothing(&project)?;
        assert_the_added_task_runs(&project)?;

        Ok(())
    })
}
