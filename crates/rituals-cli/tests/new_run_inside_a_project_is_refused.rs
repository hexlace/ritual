//! Running a project's own `new inner` from inside that project's checkout
//! is refused before anything is written, naming the workspace root and
//! pointing at the project's own `add`.
//!
//! The refusal is read from the built binary run directly: `cargo run`'s own
//! progress lines on the same stream name the crate `rituals-core-add` and
//! paths under the project root, which would answer both checks below
//! before the program printed anything.

mod support;

use support::{
    Project, TempDir, TestOutcome, assert_trees_identical, in_checkout, path_to_str, snapshot_tree,
};

#[test]
fn new_run_inside_a_scaffolded_project_is_refused_before_writing() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("new-inside-project")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let binary = project.build()?;

        let before = snapshot_tree(project.root())?;
        let result = support::run_binary(
            &binary,
            project.root(),
            &["new", "inner", "--path", checkout.path_argument()?],
        )?;
        result.expect_failure("`new inner`, run from inside the project's own checkout");

        let message = result.sole_line_prefixed_with(&project.bin_name()?);
        assert!(
            message.contains(path_to_str(project.root())?),
            "expected the refusal to name the workspace root ({}); message was:\n{message}",
            project.root().display()
        );
        assert!(
            message.contains("`add"),
            "expected the refusal to point at the project's own `add`; message was:\n{message}"
        );

        assert_trees_identical(
            "a refused `new inner`, run from inside an existing project, must write nothing",
            &before,
            &snapshot_tree(project.root())?,
        );
        assert!(
            !project.root().join("inner").exists(),
            "a refused `new inner` must not create demo/inner"
        );
        Ok(())
    })
}
