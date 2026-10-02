//! `import <crate> --path <directory> [<key>]` takes a task crate that lives
//! on disk and makes it a command of the project's own command line in one
//! step: the dependency, the entry in the task list, and the regenerated
//! command line. Nothing else is needed before the key runs.
//!
//! The key is the crate's name unless a second argument names another, and
//! the crate prints its own name and release when it runs, so a story can
//! tell that it is the imported crate answering under whichever key.

mod support;

use support::help::assert_refuses_unrecognized_subcommand;
use support::task_sources::{task_output, write_path_task};
use support::{
    Project, TempDir, TestOutcome, in_checkout, lockfile, manifest, path_to_str, run_binary,
};

const CRATE: &str = "greeter";
const VERSION: &str = "0.1.0";

#[test]
fn a_task_imported_from_a_path_runs_under_its_crate_name() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-path-default-key")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let task_dir = working_dir.path().join(CRATE);
        write_path_task(&task_dir, checkout, CRATE, VERSION)?;

        project
            .run_cli(&["import", CRATE, "--path", path_to_str(&task_dir)?])?
            .expect_success("`import greeter --path <directory>`");

        let cli_manifest = project.cli_manifest()?;
        assert!(
            manifest::tasks(&cli_manifest)?.contains(&CRATE.to_string()),
            "expected `{CRATE}` among the project's tasks; manifest was:\n{cli_manifest}"
        );
        assert_eq!(
            manifest::dependency_package(&cli_manifest, &project.workspace_manifest()?, CRATE),
            Some(CRATE.to_string()),
            "expected a dependency named `{CRATE}` on the crate `{CRATE}`; manifest was:\n\
             {cli_manifest}"
        );

        let ran = project.run_cli(&[CRATE])?;
        ran.expect_success("the imported `greeter` command, with no step after the import");
        assert!(
            ran.stdout.contains(&task_output(CRATE, VERSION)),
            "expected the imported crate to answer; stdout was:\n{}",
            ran.stdout
        );
        Ok(())
    })
}

#[test]
fn an_explicit_key_renames_the_imported_task() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-path-explicit-key")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let task_dir = working_dir.path().join(CRATE);
        write_path_task(&task_dir, checkout, CRATE, VERSION)?;

        project
            .run_cli(&["import", CRATE, "tidy", "--path", path_to_str(&task_dir)?])?
            .expect_success("`import greeter tidy --path <directory>`");

        let cli_manifest = project.cli_manifest()?;
        let tasks = manifest::tasks(&cli_manifest)?;
        assert!(
            tasks.contains(&"tidy".to_string()),
            "expected `tidy` among the project's tasks; they were {tasks:?}"
        );
        assert!(
            !tasks.contains(&CRATE.to_string()),
            "the crate's own name must not also be a task; they were {tasks:?}"
        );
        assert_eq!(
            manifest::dependency_package(&cli_manifest, &project.workspace_manifest()?, "tidy"),
            Some(CRATE.to_string()),
            "expected the dependency `tidy` to be the crate `{CRATE}`; manifest was:\n\
             {cli_manifest}"
        );

        let ran = project.run_cli(&["tidy"])?;
        ran.expect_success("the imported command, under the key it was given");
        assert!(
            ran.stdout.contains(&task_output(CRATE, VERSION)),
            "expected the crate `{CRATE}` to answer to `tidy`; stdout was:\n{}",
            ran.stdout
        );

        assert_refuses_unrecognized_subcommand(&project.run_cli(&[CRATE])?, CRATE);
        Ok(())
    })
}

#[test]
fn a_task_imported_into_a_project_with_no_lockfile_says_it_created_one() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-path-creates-lockfile")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let task_dir = working_dir.path().join(CRATE);
        write_path_task(&task_dir, checkout, CRATE, VERSION)?;
        let binary = project.build()?;
        std::fs::remove_file(project.root().join("Cargo.lock"))?;
        assert_eq!(
            lockfile(project.root())?,
            None,
            "this story starts with no lockfile"
        );

        let result = run_binary(
            &binary,
            project.root(),
            &["import", CRATE, "--path", path_to_str(&task_dir)?],
        )?;

        result.expect_success("`import greeter --path <directory>`, with no lockfile");
        assert!(
            result.stdout.contains("created Cargo.lock"),
            "expected the import to say it created the lockfile; stdout was:\n{}",
            result.stdout
        );
        assert!(
            !result.stdout.contains("updated Cargo.lock"),
            "a lockfile that was not there was not updated; stdout was:\n{}",
            result.stdout
        );
        assert!(
            lockfile(project.root())?.is_some(),
            "the import's lockfile must be on disk"
        );
        Ok(())
    })
}
