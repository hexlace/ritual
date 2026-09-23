//! `add` and `regenerate` both refuse, before writing anything, when the
//! name in play would give the composed CLI two top-level commands with the
//! same name once a bin-name-mounted bundle is flattened. Each refusal names
//! both the name and the bundle it collides with.
//!
//! One chained story over one project scaffolded with `--cli chores`, whose
//! bin name, `chores`, is free for a hand-written bundle to be mounted
//! under; ritual's own bundle sits at its ordinary key, `ritual`, and is
//! reached as `chores ritual add`/`chores ritual regenerate`. The bundle at
//! `chores` has one child, `wake`, and is set up once and reused for both
//! refusals: `add wake` first (refused, so nothing to clean up), then a
//! plain `wake` import added by hand, the way a caller bypassing `add`
//! could, for `regenerate`.

mod support;

use support::{Child, Project, TempDir, TestOutcome, in_checkout, snapshot_tree};

const BIN_NAME: &str = "chores";

/// Mounts a bundle, `housework`, whose one inline child is `wake`, under
/// the bin's own name, and regenerates so the built CLI carries it.
fn mount_a_bundle_with_a_wake_child_at_the_bin_name(project: &Project) -> TestOutcome {
    let housework_dir = project.write_bundle(
        "housework",
        "chores management, for this test",
        &[Child::Inline { key: "wake" }],
    )?;
    project.mount(&housework_dir, BIN_NAME, "housework")?;

    project
        .run_cli(&["ritual", "regenerate"])?
        .expect_success("`ritual regenerate` after mounting `housework` at the bin name");
    Ok(())
}

/// Asserts that `message`, a refusal of `wake`, names `wake` and the bundle
/// key it collides with.
#[track_caller]
fn assert_names_wake_and_the_bundle_key(message: &str) {
    assert!(
        message.contains("`wake`"),
        "expected the refusal to name `wake`; message was:\n{message}"
    );
    assert!(
        message.contains(&format!("`{BIN_NAME}`")),
        "expected the refusal to name `{BIN_NAME}`, the key of the bundle `wake` collides \
         with; message was:\n{message}"
    );
}

/// `add wake` collides with the bundle's flattened `wake` child, and is
/// refused before anything is written.
fn assert_add_refuses_the_flatten_collision(project: &Project) -> TestOutcome {
    let before = snapshot_tree(project.root())?;

    let add_result = project.run_cli(&["ritual", "add", "wake"])?;
    add_result.expect_failure("`ritual add wake`, colliding with `housework`'s flattened child");
    assert_names_wake_and_the_bundle_key(add_result.sole_line_prefixed_with(BIN_NAME));

    let after = snapshot_tree(project.root())?;
    support::assert_trees_identical(
        "a refused `add wake` must leave every file and directory as it was — no tasks/wake, not even empty, and no manifest edit",
        &before,
        &after,
    );

    Ok(())
}

/// A plain `wake` import, mounted by hand, collides with the bundle's
/// flattened `wake` child, and `regenerate` refuses before rewriting the
/// generated file.
fn assert_regenerate_refuses_the_hand_added_collision(project: &Project) -> TestOutcome {
    let generated_before = project.generated_file()?;

    let wake_dir = project.write_leaf("wake")?;
    project.mount(&wake_dir, "wake", "wake")?;

    let regenerate_result = project.run_cli(&["ritual", "regenerate"])?;
    regenerate_result.expect_failure(
        "`ritual regenerate` with a plain `wake` import colliding with `housework`'s \
         flattened `wake` child",
    );
    assert_names_wake_and_the_bundle_key(regenerate_result.sole_line_prefixed_with(BIN_NAME));

    assert_eq!(
        project.generated_file()?,
        generated_before,
        "a refused `regenerate` must not rewrite the generated file"
    );

    Ok(())
}

#[test]
fn add_and_regenerate_both_refuse_a_flatten_collision_before_writing() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("pre-write-flatten-collision")?;
        let project =
            Project::scaffold(checkout, working_dir.path(), "demo", &["--cli", BIN_NAME])?;

        mount_a_bundle_with_a_wake_child_at_the_bin_name(&project)?;
        assert_add_refuses_the_flatten_collision(&project)?;
        assert_regenerate_refuses_the_hand_added_collision(&project)?;

        Ok(())
    })
}
