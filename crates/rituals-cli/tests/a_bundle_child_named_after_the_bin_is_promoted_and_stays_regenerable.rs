//! A bundle mounted under the project's own bin name may have a child whose
//! own key is the bin name too. That is a legitimate shape: flattening
//! promotes the child to the top level once, and does not flatten it again
//! because its key matches the bundle's. The built CLI lists the child and
//! dispatches to it, and `regenerate` and `add` agree with that startup
//! rule: `regenerate` accepts the project unchanged, and `add` of an
//! unrelated task lands the crate, both manifest edits and the generated
//! file's new line.

mod support;

use std::path::PathBuf;

use support::tree::changed_paths;
use support::{Child, Project, TempDir, TestOutcome, help, in_checkout, snapshot_tree};
use support::{generated, manifest};

const BIN_NAME: &str = "self-collision-demo";
const BUNDLE_ABOUT: &str = "top-level management, mounted at the bin name";

/// Writes a leaf, `promoted`, and a bundle, `management`, whose one child is
/// `promoted` under the bin's own name; mounts `management` under the bin's
/// own name too, and regenerates.
fn mount_a_bundle_whose_child_shares_the_bin_name(project: &Project) -> TestOutcome {
    project.write_leaf("promoted")?;
    let management_dir = project.write_bundle(
        "management",
        BUNDLE_ABOUT,
        &[Child::Crate {
            key: BIN_NAME,
            crate_name: "promoted",
        }],
    )?;
    project.mount(&management_dir, BIN_NAME, "management")?;
    project
        .run_cli(&["ritual", "regenerate"])?
        .expect_success("`ritual regenerate` after mounting the self-naming bundle");
    Ok(())
}

/// The built CLI lists the promoted child at the top level, never the
/// bundle's description, and runs the child when it is named.
fn assert_the_child_is_promoted_and_runs(project: &Project) -> TestOutcome {
    let help = project.run_cli(&["--help"])?;
    help.expect_success("`--help`");
    assert_eq!(
        help::command_names(&help.stdout),
        ["ritual", BIN_NAME, "help"],
        "stdout was:\n{}",
        help.stdout
    );
    assert!(
        !help.stdout.contains(BUNDLE_ABOUT),
        "expected the flattened bundle's own description to never appear in --help; stdout \
         was:\n{}",
        help.stdout
    );

    let promoted = project.run_cli(&[BIN_NAME])?;
    promoted.expect_success("the promoted child, run by its key");
    assert!(
        promoted.stdout.contains("promoted ran"),
        "expected the promoted child to run; stdout was:\n{}",
        promoted.stdout
    );
    Ok(())
}

/// `regenerate`, with nothing changed, reports the file up to date and
/// leaves it byte for byte as it was.
fn assert_regenerate_accepts_the_project_unchanged(project: &Project) -> TestOutcome {
    let generated_before = project.generated_file()?;
    let result = project.run_cli(&["ritual", "regenerate"])?;
    result.expect_success("`ritual regenerate` on the unchanged project");
    assert!(
        result.stdout.contains("already up to date"),
        "expected regenerate to report the file already up to date; stdout was:\n{}",
        result.stdout
    );
    assert_eq!(project.generated_file()?, generated_before);
    Ok(())
}

/// `add extra` changes exactly the new crate's directories and two files,
/// both manifests and the generated file — which mounts `extra` afterwards.
fn assert_add_lands_every_part_of_a_new_task(project: &Project) -> TestOutcome {
    let before = snapshot_tree(project.root())?;
    project
        .run_cli(&["ritual", "add", "extra"])?
        .expect_success("`ritual add extra` on this project");
    let after = snapshot_tree(project.root())?;

    let composed_cli = project
        .composed_cli_dir()
        .strip_prefix(project.root())?
        .to_path_buf();
    let mut expected = vec![
        PathBuf::from("Cargo.toml"),
        composed_cli.join("Cargo.toml"),
        composed_cli.join("src/main.rs"),
        PathBuf::from("tasks/extra"),
        PathBuf::from("tasks/extra/Cargo.toml"),
        PathBuf::from("tasks/extra/src"),
        PathBuf::from("tasks/extra/src/lib.rs"),
    ];
    expected.sort();
    assert_eq!(changed_paths(&before, &after), expected);

    assert!(
        manifest::tasks(&project.cli_manifest()?)?.contains(&"extra".to_string()),
        "expected [package.metadata.ritual] tasks to name `extra`"
    );
    let generated_file = project.generated_file()?;
    assert!(
        generated::mounted_entries(&generated_file)
            .contains(&("extra".to_string(), "extra".to_string())),
        "expected the generated file to mount `extra`; file was:\n{generated_file}"
    );
    Ok(())
}

#[test]
fn a_bundle_child_named_after_the_bin_is_promoted_and_regenerate_and_add_agree() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("bin-named-child")?;
        let project =
            Project::scaffold(checkout, working_dir.path(), BIN_NAME, &["--cli", BIN_NAME])?;

        mount_a_bundle_whose_child_shares_the_bin_name(&project)?;
        assert_the_child_is_promoted_and_runs(&project)?;
        assert_regenerate_accepts_the_project_unchanged(&project)?;
        assert_add_lands_every_part_of_a_new_task(&project)?;

        Ok(())
    })
}
