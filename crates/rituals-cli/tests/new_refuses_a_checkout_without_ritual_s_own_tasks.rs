//! `new --path` names two directories of the checkout in the project it
//! writes: the `rituals` crate and ritual's own tasks in `.rituals/ritual`.
//! A checkout that has the first and not the second, such as one from before
//! ritual's own tasks lived in `.rituals/`, is refused before anything is
//! written, naming what is missing, rather than leaving a project whose first
//! build fails on a path that is not there.
//!
//! The checkout here is made by hand and holds only the `rituals` crate's
//! manifest, which is all the refusal reads; nothing is built from it.

mod support;

use support::{TempDir, TestOutcome, path_to_str, run_ritual, snapshot_tree};

#[test]
fn a_checkout_without_ritual_s_own_tasks_is_refused_before_writing() -> TestOutcome {
    let checkout = TempDir::new("new-checkout-without-bundle")?;
    let rituals = checkout.path().join("crates/rituals");
    std::fs::create_dir_all(&rituals)?;
    std::fs::write(
        rituals.join("Cargo.toml"),
        "[package]\nname = \"rituals\"\n",
    )?;
    let working_dir = TempDir::new("new-checkout-without-bundle-work")?;
    let before = snapshot_tree(working_dir.path())?;

    let result = run_ritual(
        working_dir.path(),
        &["new", "demo", "--path", path_to_str(checkout.path())?],
    )?;

    result.expect_failure("`new demo --path` against a checkout without .rituals/ritual");
    let message = result.sole_line_prefixed_with("ritual");
    assert!(
        message.contains(".rituals/ritual/Cargo.toml"),
        "expected the refusal to name the manifest it did not find; message was:\n{message}"
    );
    assert!(
        message.contains(path_to_str(checkout.path())?),
        "expected the refusal to name the checkout; message was:\n{message}"
    );
    assert_eq!(
        snapshot_tree(working_dir.path())?,
        before,
        "a refused `new` must write nothing"
    );
    Ok(())
}
