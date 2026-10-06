//! Outside any project, `create` scaffolds an ordinary Cargo crate that marks
//! itself a task, as `[package.metadata.ritual] task = true`, and builds
//! entirely on its own — no enclosing project required. Wherever Cargo would
//! place that crate inside a workspace, it is inside a project and `create`
//! scaffolds into that project instead.

mod support;

use support::manifest;
use support::{
    TempDir, TestOutcome, assert_trees_identical, cargo, in_checkout, path_to_str, run_ritual,
    snapshot_tree,
};

#[test]
fn create_scaffolds_a_task_crate_that_builds_on_its_own() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("create-standalone")?;

        run_ritual(
            working_dir.path(),
            &[
                "create",
                "greeting-task",
                "--path",
                checkout.path_argument()?,
            ],
        )?
        .expect_success("`ritual create greeting-task --path <checkout>`");

        let crate_dir = working_dir.path().join("greeting-task");
        let manifest_path = crate_dir.join("Cargo.toml");
        let document = manifest::read(&manifest_path)?;
        assert!(
            manifest::declares_itself_a_task(&document),
            "expected the scaffolded crate to declare `[package.metadata.ritual] task = true`; \
             manifest was:\n{document}"
        );

        cargo(
            &crate_dir,
            &crate_dir.join("target"),
            &["build", "--manifest-path", path_to_str(&manifest_path)?],
        )?
        .expect_success("building the standalone scaffolded task crate");

        Ok(())
    })
}

/// A directory the workspace root's `exclude` covers, but which has no
/// `Cargo.toml` of its own, is still inside that workspace to Cargo:
/// `cargo locate-project` walks past it to the root. So `create` there is
/// inside a project, not standing alone, and takes the in-project path, where
/// this workspace, with no members, is refused in Cargo's own words and
/// nothing is written.
#[test]
fn create_in_an_excluded_directory_with_no_manifest_is_refused_and_writes_nothing() -> TestOutcome {
    let workspace = TempDir::new("create-excluded-empty")?;
    support::write_text(
        &workspace.path().join("Cargo.toml"),
        "[workspace]\nmembers = []\nexclude = [\"scratch\"]\nresolver = \"3\"\n",
    )?;
    let scratch = workspace.path().join("scratch");
    std::fs::create_dir(&scratch)?;
    let before = snapshot_tree(workspace.path())?;

    let refused = run_ritual(&scratch, &["create", "ex"])?;
    refused.expect_failure("`ritual create ex` in an excluded directory with no manifest");
    let message = refused.sole_line_prefixed_with("ritual");
    assert!(
        message.contains("contains no package"),
        "expected Cargo's own words about the workspace; message was:\n{message}"
    );
    assert!(
        message.contains(path_to_str(workspace.path())?),
        "expected the refusal to name the workspace root; message was:\n{message}"
    );

    assert_trees_identical(
        "a refused `create ex` must leave the workspace as it was, with no lockfile written",
        &before,
        &snapshot_tree(workspace.path())?,
    );
    Ok(())
}
