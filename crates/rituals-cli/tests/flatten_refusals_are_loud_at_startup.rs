//! Two refusals that can only be seen once a composed CLI starts, because
//! both depend on the compiled binary's own name, which no check before the
//! build can know:
//!
//! - whatever is mounted under the key equal to the bin name must be a
//!   bundle, and a plain task there makes the CLI refuse to start, naming
//!   the key and the bin name;
//! - if flattening that bundle would give the CLI two top-level commands of
//!   one name, the CLI refuses to start, naming both of the things that
//!   provide it.
//!
//! One story over a project scaffolded with `--cli housework`, whose bin
//! name is free for a hand-written crate to be mounted under; ritual's own
//! bundle sits at its ordinary key, `ritual`, throughout. `add` refuses to
//! scaffold a task under the bin's own name (see
//! `add_refuses_a_task_named_after_the_bin.rs`), so the plain task is
//! written and mounted by hand.

mod support;

use support::{Child, Project, TempDir, TestOutcome, crates, in_checkout, write_text};

const BIN_NAME: &str = "housework";

/// Mounts a plain leaf, `promoted`, under the bin's own name, regenerates
/// it into the generated file, and asserts the built CLI then refuses to
/// start, naming `housework` — here both the key and the bin name — and
/// saying a bundle is what belongs there.
fn assert_a_plain_task_at_the_bin_name_refuses_to_start(project: &Project) -> TestOutcome {
    let promoted_dir = project.write_leaf("promoted")?;
    project.mount(&promoted_dir, BIN_NAME, "promoted")?;
    project
        .alias(&["ritual", "regenerate"])?
        .expect_success("`cargo housework ritual regenerate` after mounting `promoted`");

    let refused = project.run_cli(&["--help"])?;
    refused.expect_failure("the built CLI, with a plain task mounted under the bin's own name");
    let message = refused.sole_line_prefixed_with(BIN_NAME);
    assert!(
        message.contains(&format!("`{BIN_NAME}`")),
        "expected the refusal to name `{BIN_NAME}`, the key and the bin name; message \
         was:\n{message}"
    );
    assert!(
        message.contains("Task::group"),
        "expected the refusal to say what to do about it — make the mounted task a bundle \
         with `Task::group`, or mount it under another name; message was:\n{message}"
    );
    Ok(())
}

/// Turns `promoted` into a bundle whose one child is named `ritual`, which
/// flattening puts beside ritual's own bundle key, and asserts the rebuilt
/// CLI refuses to start, naming `ritual` and the bin name the colliding
/// bundle is mounted under.
fn assert_a_colliding_flattened_child_refuses_to_start(project: &Project) -> TestOutcome {
    write_text(
        &project.root().join("tasks/promoted/src/lib.rs"),
        &crates::bundle_lib(
            "a second management bundle, for this test",
            &[Child::Inline { key: "ritual" }],
        ),
    )?;

    let refused = project.run_cli(&["--help"])?;
    refused.expect_failure(
        "the built CLI, with a flattened child named `ritual` colliding with ritual's own \
         bundle key",
    );
    let message = refused.sole_line_prefixed_with(BIN_NAME);
    for name in ["ritual", BIN_NAME] {
        assert!(
            message.contains(&format!("`{name}`")),
            "expected the refusal to name `{name}`; message was:\n{message}"
        );
    }
    assert!(
        message.contains("another name"),
        "expected the refusal to say what to do about it — rename the bundle's child, or \
         mount that task under another name; message was:\n{message}"
    );
    Ok(())
}

#[test]
fn a_plain_task_and_a_colliding_bundle_at_the_bin_name_both_refuse_loudly() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("flatten-refusals")?;
        let project =
            Project::scaffold(checkout, working_dir.path(), "demo", &["--cli", BIN_NAME])?;

        assert_a_plain_task_at_the_bin_name_refuses_to_start(&project)?;
        assert_a_colliding_flattened_child_refuses_to_start(&project)?;

        Ok(())
    })
}
