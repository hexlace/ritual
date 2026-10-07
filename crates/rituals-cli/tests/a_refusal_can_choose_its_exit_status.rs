//! A task can refuse with an exit status of its own, so a caller such as a
//! CI job can tell one kind of refusal from another without reading stderr.
//! The status changes nothing else: the refusal line is the one the same
//! failure prints at the default status, 1.

mod support;

use support::{Project, TempDir, TestOutcome, in_checkout, write_text};

/// The source of a leaf task that refuses with `3 tasks differ`, at the
/// status its one argument names, or at the default when it is given none.
const REFUSING_LIB: &str = r#"//! A task that refuses, at an exit status of its caller's choosing.

use rituals::{Failure, Outcome, RefusalStatus, Task, clap};

/// What this task accepts on the command line.
#[derive(clap::Args)]
struct Arguments {
    /// the exit status to refuse with
    status: Option<u8>,
}

/// This task, for a command line to mount under whatever name imports it.
#[must_use]
pub fn task() -> Task {
    Task::new("refuses, at the status it is given", run)
}

fn run(arguments: Arguments) -> Outcome {
    let failure = Failure::new("3 tasks differ from the declared set");
    let Some(code) = arguments.status else {
        return Err(failure);
    };
    let status = RefusalStatus::new(code)
        .map_err(|error| Failure::new("the status is not one a refusal can use").caused_by(error))?;
    Err(failure.exiting_with(status))
}
"#;

#[test]
fn a_refusal_at_status_three_exits_three_and_prints_what_it_prints_at_one() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("refusal-exit-status")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let crate_dir = project.write_leaf("check")?;
        write_text(&crate_dir.join("src/lib.rs"), REFUSING_LIB)?;
        project.mount(&crate_dir, "check", "check")?;
        project
            .run_cli(&["regenerate"])?
            .expect_success("`regenerate` after mounting `check`");

        let at_default = project.run_cli(&["check"])?;
        let at_three = project.run_cli(&["check", "3"])?;

        assert_eq!(
            at_default.exit_code,
            Some(1),
            "expected a plain refusal to exit 1; stderr was:\n{}",
            at_default.stderr
        );
        assert_eq!(
            at_three.exit_code,
            Some(3),
            "expected the refusal to exit with the status it chose; stderr was:\n{}",
            at_three.stderr
        );
        assert_eq!(
            at_three.sole_line_prefixed_with("ritual"),
            at_default.sole_line_prefixed_with("ritual"),
            "the status must not change the refusal line"
        );
        assert_eq!(
            at_three.sole_line_prefixed_with("ritual"),
            "3 tasks differ from the declared set"
        );
        Ok(())
    })
}
