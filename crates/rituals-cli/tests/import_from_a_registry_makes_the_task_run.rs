//! `import <crate>[@<version>] [<key>]` takes a task crate from the
//! registry, at the version Cargo's grammar names (`@0.1.0`, no `v`) or
//! the newest release when none is named.
//!
//! The registry is a directory Cargo reads in place of crates.io (see
//! `support::task_sources`), holding two releases of one task crate whose
//! output names the release, so the story can tell which one was chosen.
//! Importing resolves, locks and builds against that directory alone.

mod support;

use support::task_sources::{LocalRegistry, task_output};
use support::{Project, TempDir, TestOutcome, in_checkout, manifest};

const CRATE: &str = "greeter";

/// A default project whose registry holds `greeter` 0.1.0 and 0.2.0.
fn project_with_two_releases(
    checkout: &support::Checkout,
    working_dir: &TempDir,
) -> support::Outcome<Project> {
    let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
    let registry =
        LocalRegistry::install(&project, checkout, &working_dir.path().join("registry"))?;
    registry.publish_task(CRATE, "0.1.0")?;
    registry.publish_task(CRATE, "0.2.0")?;
    Ok(project)
}

#[test]
fn a_task_imported_at_a_named_version_runs_that_release_under_the_given_key() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-registry-version")?;
        let project = project_with_two_releases(checkout, &working_dir)?;

        project
            .run_cli(&["import", "greeter@0.1.0", "hail"])?
            .expect_success("`import greeter@0.1.0 hail`");

        let cli_manifest = project.cli_manifest()?;
        assert!(
            manifest::tasks(&cli_manifest)?.contains(&"hail".to_string()),
            "expected `hail` among the project's tasks; manifest was:\n{cli_manifest}"
        );

        let ran = project.run_cli(&["hail"])?;
        ran.expect_success("the imported `hail` command, with no step after the import");
        assert!(
            ran.stdout.contains(&task_output(CRATE, "0.1.0")),
            "expected release 0.1.0 to answer, as named, and not the newer 0.2.0; stdout \
             was:\n{}",
            ran.stdout
        );
        Ok(())
    })
}

#[test]
fn a_task_imported_without_a_version_runs_the_newest_release_under_its_crate_name() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-registry-newest")?;
        let project = project_with_two_releases(checkout, &working_dir)?;

        project
            .run_cli(&["import", CRATE])?
            .expect_success("`import greeter`");

        let ran = project.run_cli(&[CRATE])?;
        ran.expect_success("the imported `greeter` command, with no step after the import");
        assert!(
            ran.stdout.contains(&task_output(CRATE, "0.2.0")),
            "expected the newest release, 0.2.0, to answer; stdout was:\n{}",
            ran.stdout
        );
        Ok(())
    })
}
