//! `import <crate> --git <url>` takes a task crate from a git repository,
//! and Cargo's own `--tag` pins it, so the key runs the release the tag
//! names rather than the newest commit.
//!
//! The repository is made in the test, on disk, and cloned by `file://`
//! URL, and the project resolves its registry crates from a directory, so
//! nothing here reaches a network once the project's own dependencies are
//! in place.

mod support;

use support::task_sources::{
    LocalRegistry, commit_task_release, tag_current_commit, task_output, write_git_task_repository,
};
use support::{Project, TempDir, TestOutcome, in_checkout, manifest};

const CRATE: &str = "greeter";

#[test]
fn a_task_imported_from_a_git_repository_runs_under_its_crate_name() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-git-default-key")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        LocalRegistry::install(&project, checkout, &working_dir.path().join("registry"))?;
        let url =
            write_git_task_repository(&working_dir.path().join("repository"), CRATE, "0.1.0")?;

        project
            .run_cli(&["import", CRATE, "--git", &url])?
            .expect_success("`import greeter --git <url>`");

        let cli_manifest = project.cli_manifest()?;
        assert!(
            manifest::tasks(&cli_manifest)?.contains(&CRATE.to_string()),
            "expected `{CRATE}` among the project's tasks; manifest was:\n{cli_manifest}"
        );

        let ran = project.run_cli(&[CRATE])?;
        ran.expect_success("the imported `greeter` command, with no step after the import");
        assert!(
            ran.stdout.contains(&task_output(CRATE, "0.1.0")),
            "expected the imported crate to answer; stdout was:\n{}",
            ran.stdout
        );
        Ok(())
    })
}

#[test]
fn a_tag_given_to_import_pins_the_release_that_runs() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-git-tag")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        LocalRegistry::install(&project, checkout, &working_dir.path().join("registry"))?;
        let repository = working_dir.path().join("repository");
        let url = write_git_task_repository(&repository, CRATE, "0.1.0")?;
        tag_current_commit(&repository, "release")?;
        commit_task_release(&repository, CRATE, "0.2.0")?;

        project
            .run_cli(&["import", CRATE, "--git", &url, "--tag", "release"])?
            .expect_success("`import greeter --git <url> --tag release`");

        let ran = project.run_cli(&[CRATE])?;
        ran.expect_success("the imported `greeter` command");
        assert!(
            ran.stdout.contains(&task_output(CRATE, "0.1.0")),
            "expected the tagged release, 0.1.0, to answer and not the newer commit; stdout \
             was:\n{}",
            ran.stdout
        );
        Ok(())
    })
}
