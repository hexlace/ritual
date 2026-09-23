//! Every line the framework writes to standard error — a task's own
//! refusal, and the warning written when an import cannot be mounted — is
//! prefixed with the bin name, what a person actually typed, never the
//! crate's package name.
//!
//! A default scaffold's composed CLI has package `demo-ritual` and bin
//! `ritual`, two different strings, so an ordinary project shows the
//! difference.

mod support;

use support::{Project, ResultContext, TempDir, TestOutcome, in_checkout, write_text};

/// `add`, asked to scaffold a name whose `tasks/<name>` directory is already
/// there, refuses with a line prefixed `ritual: `.
#[test]
fn a_tasks_own_refusal_is_prefixed_with_the_bin_name_not_the_package_name() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("refusal-prefix-task-failure")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        std::fs::create_dir_all(project.root().join("tasks/already-here"))
            .context("creating the leftover tasks/already-here failed")?;

        let result = project.run_cli(&["add", "already-here"])?;
        result.expect_failure("`add already-here`, with tasks/already-here already present");
        assert!(
            result
                .sole_line_prefixed_with("ritual")
                .contains("already-here"),
            "expected the refusal to name the offending entry; stderr was:\n{}",
            result.stderr
        );
        Ok(())
    })
}

/// A generated file that mounts `ritual` twice — written by hand, since
/// `regenerate` refuses to write one — makes the CLI drop the second and
/// warn, with a line prefixed `ritual: ` naming `ritual`.
///
/// The CLI still starts: a line duplicated in the generated file is one
/// `regenerate` can remove, so the command line stays usable enough to run
/// it. (A collision that flattening creates is refused instead — see
/// `flatten_refusals_are_loud_at_startup.rs` — because the colliding name is
/// compiled into a bundle, where `regenerate` cannot reach it.)
#[test]
fn an_import_that_cannot_be_mounted_is_reported_with_the_bin_name_not_the_package_name()
-> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("refusal-prefix-mount-collision")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;

        let generated_file = project.generated_file()?;
        let import_line = "            (\"ritual\", ritual::task()),\n";
        assert_eq!(
            generated_file.matches(import_line).count(),
            1,
            "expected the generated file to mount ritual's bundle on one line of its own; \
             file was:\n{generated_file}"
        );
        write_text(
            &project.generated_file_path(),
            &generated_file.replacen(import_line, &import_line.repeat(2), 1),
        )?;

        let result = project.run_cli(&["--help"])?;
        result.expect_success("the CLI with ritual's bundle mounted twice, run with `--help`");
        let warning = result.sole_line_prefixed_with("ritual");
        assert!(
            warning.contains("`ritual` collides"),
            "expected the warning to name the colliding import `ritual`; warning \
             was:\n{warning}"
        );
        Ok(())
    })
}
