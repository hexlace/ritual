//! `regenerate` writes the command line from the rituals the project lists,
//! and a listed ritual nested below `.rituals/` at any depth is mounted like
//! any other: the command line builds with it and the ritual runs through
//! the composed CLI.
//!
//! The fixture lists two nested rituals the way a person who moved them
//! there by hand would, a dependency and a `tasks` entry, and leaves the
//! generated file for `regenerate` to write.

mod support;

use support::nested::{list_as_a_task, write_ritual};
use support::{Project, TempDir, TestOutcome, generated, in_checkout};

#[test]
fn regenerate_mounts_rituals_nested_at_more_than_one_level_and_they_run() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("regenerate-nested")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        write_ritual(&project, ".rituals/private/lint", "lint")?;
        list_as_a_task(&project, ".rituals/private/lint", "lint")?;
        write_ritual(&project, ".rituals/a/b/greet", "greet")?;
        list_as_a_task(&project, ".rituals/a/b/greet", "greet")?;

        project
            .run_cli(&["regenerate"])?
            .expect_success("`regenerate` with rituals nested in .rituals/");

        let generated_file = project.generated_file()?;
        let mounted: Vec<String> = generated::mounted_entries(&generated_file)
            .into_iter()
            .map(|(key, _crate)| key)
            .collect();
        assert_eq!(
            mounted,
            ["ritual", "lint", "greet"],
            "expected the nested rituals to be mounted; file was:\n{generated_file}"
        );
        for (command, line) in [("lint", "lint ran"), ("greet", "greet ran")] {
            let ran = project.run_cli(&[command])?;
            ran.expect_success(&format!("the nested ritual's `{command}` command"));
            assert!(
                ran.stdout.contains(line),
                "expected `{command}` to answer `{line}`; stdout was:\n{}",
                ran.stdout
            );
        }
        Ok(())
    })
}
