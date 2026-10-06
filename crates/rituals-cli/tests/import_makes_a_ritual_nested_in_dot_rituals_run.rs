//! `import <crate> --path <directory>` takes a ritual that lives on disk and
//! makes it a command in one step. A ritual that is already a workspace
//! member nested below `.rituals/` at any depth, spelled relative to the
//! project or in full, imports and runs like any other.
//!
//! The fixtures write the ritual where a person grouping their rituals would
//! have it, as a member that declares itself a task, and leave the command
//! line knowing nothing of it.

mod support;

use support::nested::write_ritual;
use support::{Project, TempDir, TestOutcome, in_checkout, manifest, path_to_str};

#[test]
fn a_nested_ritual_imports_by_a_path_from_the_project_and_runs() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-nested-relative")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        write_ritual(&project, ".rituals/private/lint", "lint")?;

        project
            .run_cli(&["import", "lint", "--path", ".rituals/private/lint"])?
            .expect_success("`import lint --path .rituals/private/lint`");

        let cli_manifest = project.cli_manifest()?;
        assert!(
            manifest::tasks(&cli_manifest)?.contains(&"lint".to_string()),
            "expected `lint` among the project's tasks; manifest was:\n{cli_manifest}"
        );
        let ran = project.run_cli(&["lint"])?;
        ran.expect_success("the imported `lint` command, with no step after the import");
        assert!(
            ran.stdout.contains("lint ran"),
            "expected the nested ritual to answer; stdout was:\n{}",
            ran.stdout
        );
        Ok(())
    })
}

#[test]
fn a_ritual_nested_two_levels_deep_imports_by_its_full_path_and_runs() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-nested-deep")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let directory = write_ritual(&project, ".rituals/a/b/greet", "greet")?;

        project
            .run_cli(&["import", "greet", "--path", path_to_str(&directory)?])?
            .expect_success("`import greet --path <full path to .rituals/a/b/greet>`");

        let ran = project.run_cli(&["greet"])?;
        ran.expect_success("the imported `greet` command, with no step after the import");
        assert!(
            ran.stdout.contains("greet ran"),
            "expected the nested ritual to answer; stdout was:\n{}",
            ran.stdout
        );
        Ok(())
    })
}
