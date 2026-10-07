//! A refusal whose cause has a cause of its own prints all of them, each
//! once, in order, joined with `: `, after the bin name.
//!
//! A task's errors usually follow the convention that an error's `Display`
//! names only its own situation and hands whatever caused it on through
//! `source()`. The refusal line walks that chain, so a cause two levels down
//! still reaches the person reading it, and nothing in it is printed twice.

mod support;

use support::{Project, TempDir, TestOutcome, in_checkout, write_text};

/// The source of a leaf task that refuses with a three-link chain: its own
/// message, an error naming what it could not reach, and the error under that.
const REFUSING_LIB: &str = r#"//! A task that refuses with a cause that has a cause of its own.

use rituals::{Failure, Outcome, Task, clap};

/// What this task accepts on the command line: nothing.
#[derive(clap::Args)]
struct Arguments {}

/// An error that names only its own situation and hands the rest on.
#[derive(Debug)]
struct Link {
    message: &'static str,
    source: Option<Box<Link>>,
}

impl std::fmt::Display for Link {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for Link {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|link| link as &(dyn std::error::Error + 'static))
    }
}

/// This task, for a command line to mount under whatever name imports it.
#[must_use]
pub fn task() -> Task {
    Task::new("refuses with a chain of causes", run)
}

fn run(_arguments: Arguments) -> Outcome {
    let unreachable = Link {
        message: "the registry did not answer",
        source: Some(Box::new(Link {
            message: "connection refused",
            source: None,
        })),
    };
    Err(Failure::new("syncing the declared set failed").caused_by(unreachable))
}
"#;

#[test]
fn a_refusal_with_a_cause_under_its_cause_prints_all_three_once_in_order() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("refusal-cause-chain")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let crate_dir = project.write_leaf("sync")?;
        write_text(&crate_dir.join("src/lib.rs"), REFUSING_LIB)?;
        project.mount(&crate_dir, "sync", "sync")?;
        project
            .run_cli(&["regenerate"])?
            .expect_success("`regenerate` after mounting `sync`");

        let result = project.run_cli(&["sync"])?;
        result.expect_failure("`sync`, whose task always refuses");
        assert_eq!(
            result.sole_line_prefixed_with("ritual"),
            "syncing the declared set failed: the registry did not answer: connection refused",
            "expected every cause, each once and in order; stderr was:\n{}",
            result.stderr
        );
        Ok(())
    })
}
