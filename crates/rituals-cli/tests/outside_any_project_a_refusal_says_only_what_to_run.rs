//! Run where there is no Cargo project at all, `import` and `remove` refuse
//! before they begin, so the refusal is the whole message: the command to
//! run inside a project, and nothing about putting a project back, since
//! there is none.

mod support;

use support::{TempDir, TestOutcome, run_ritual, snapshot_tree};

/// The refusal for `command` typed as `typed` outside any project.
fn outside_refusal(command: &str, typed: &str) -> String {
    format!(
        "`{command}` works inside the project this command line belongs to; in your project, \
         run `cargo ritual {typed}` (or `cargo <name> ritual {typed}` if it was made with \
         `--cli <name>`)"
    )
}

#[test]
fn import_outside_any_project_refuses_with_the_command_alone() -> TestOutcome {
    let empty = TempDir::new("import-outside-whole-message")?;

    let result = run_ritual(empty.path(), &["import", "greeter", "--path", "/w/greeter"])?;

    result.expect_failure("`ritual import` where there is no project");
    assert_eq!(
        result.sole_line_prefixed_with("ritual"),
        outside_refusal("import", "import greeter --path /w/greeter")
    );
    assert!(
        snapshot_tree(empty.path())?.is_empty(),
        "a refused `import` writes nothing"
    );
    Ok(())
}

#[test]
fn remove_outside_any_project_refuses_with_the_command_alone() -> TestOutcome {
    let empty = TempDir::new("remove-outside-whole-message")?;

    let result = run_ritual(empty.path(), &["remove", "lint"])?;

    result.expect_failure("`ritual remove` where there is no project");
    assert_eq!(
        result.sole_line_prefixed_with("ritual"),
        outside_refusal("remove", "remove lint")
    );
    assert!(
        snapshot_tree(empty.path())?.is_empty(),
        "a refused `remove` writes nothing"
    );
    Ok(())
}
