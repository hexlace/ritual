//! `create` takes a path as well as a name, wherever the path leads inside
//! `.rituals/`: the task lands there, its member entry spells the path from
//! the workspace root, and it runs as a command under its last component.
//! A path is read the way a shell reads it, from the directory the command
//! is run in. A path that does not lead strictly below `.rituals/` is
//! refused, naming `.rituals/` and where the path led from the project root,
//! and leaves the project byte-identical.
//! `--path` and `--git` choose where an outside crate's dependency comes
//! from, so inside a project, where the task inherits it, they are refused.

mod support;

use support::created::{
    Audience, Expected, assert_refused_and_left_alone, assert_the_task_builds_and_runs,
    assert_the_task_is_scaffolded, stdout_lines,
};
use support::removal::exists;
use support::{Project, TempDir, TestOutcome, in_checkout, run_binary};

/// A project that already has one task, so `.rituals/` exists, and the
/// built command line.
fn project_with_a_first_task(
    checkout: &support::Checkout,
    prefix: &str,
) -> support::Outcome<(TempDir, Project, std::path::PathBuf)> {
    let working_dir = TempDir::new(prefix)?;
    let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
    project
        .alias(&["create", "first"])?
        .expect_success("`cargo ritual create first`");
    let binary = project.build()?;
    Ok((working_dir, project, binary))
}

fn assert_nested_task_placed_and_runs(project: &Project) -> TestOutcome {
    assert_the_task_is_scaffolded(
        project,
        &Expected {
            directory: ".rituals/private/lint",
            name: "lint",
            audience: Audience::Private,
            members: &["ritual", ".rituals/first", ".rituals/private/lint"],
            tasks: &["ritual", "first", "lint"],
            dependency_path: "../.rituals/private/lint",
        },
    )?;
    assert!(
        !exists(&project.root().join(".rituals/lint")),
        "expected no .rituals/lint: the task is where the path said"
    );
    assert_the_task_builds_and_runs(project, "lint")
}

#[test]
fn a_path_from_the_root_places_the_task_there() -> TestOutcome {
    in_checkout(|checkout| {
        let (_working_dir, project, binary) =
            project_with_a_first_task(checkout, "create-nested-root")?;

        let created = run_binary(
            &binary,
            project.root(),
            &["create", ".rituals/private/lint"],
        )?;
        created.expect_success("`ritual create .rituals/private/lint` at the root");

        assert_eq!(
            stdout_lines(&created),
            [
                "created .rituals/private/lint/Cargo.toml",
                "created .rituals/private/lint/src/lib.rs",
                "updated Cargo.toml",
                "updated ritual/Cargo.toml",
                "updated ritual/src/main.rs (tasks: ritual, first, lint)",
                "next: edit .rituals/private/lint/src/lib.rs, then run cargo ritual lint",
            ]
        );
        assert_nested_task_placed_and_runs(&project)
    })
}

#[test]
fn a_path_read_from_inside_dot_rituals_means_the_same_place() -> TestOutcome {
    in_checkout(|checkout| {
        let (_working_dir, project, binary) =
            project_with_a_first_task(checkout, "create-nested-inside")?;

        let created = run_binary(
            &binary,
            &project.root().join(".rituals"),
            &["create", "private/lint"],
        )?;
        created.expect_success("`ritual create private/lint` from inside .rituals/");

        assert_eq!(
            stdout_lines(&created).first().map(String::as_str),
            Some("created .rituals/private/lint/Cargo.toml"),
            "expected the report to spell the path from the workspace root; stdout was:\n{}",
            created.stdout
        );
        assert_nested_task_placed_and_runs(&project)
    })
}

#[test]
fn a_bare_name_is_placed_directly_in_dot_rituals_from_any_directory() -> TestOutcome {
    in_checkout(|checkout| {
        let (_working_dir, project, binary) =
            project_with_a_first_task(checkout, "create-bare-from-below")?;

        run_binary(
            &binary,
            &project.root().join(".rituals/first"),
            &["create", "lint"],
        )?
        .expect_success("`ritual create lint` from inside a task's directory");

        assert!(
            exists(&project.root().join(".rituals/lint/Cargo.toml")),
            "expected a bare name to go to <root>/.rituals/lint whatever the current directory"
        );
        assert!(!exists(&project.root().join(".rituals/first/lint")));
        Ok(())
    })
}

#[test]
fn a_path_that_does_not_lead_below_dot_rituals_is_refused_naming_it() -> TestOutcome {
    in_checkout(|checkout| {
        let (_working_dir, project, binary) =
            project_with_a_first_task(checkout, "create-nested-refused")?;

        for arguments in [
            &["create", "tools/lint"][..],
            &["create", ".rituals/../lint"],
            &["create", ".rituals/"],
            &["create", "./.rituals/./.."],
            &["create", "../lint"],
        ] {
            assert_refused_and_left_alone(
                &project,
                &binary,
                project.root(),
                arguments,
                |message| message.contains(".rituals/"),
            )?;
        }
        Ok(())
    })
}

#[test]
fn a_path_that_climbs_out_of_dot_rituals_from_below_is_refused() -> TestOutcome {
    in_checkout(|checkout| {
        let (_working_dir, project, binary) =
            project_with_a_first_task(checkout, "create-nested-climbs")?;

        assert_refused_and_left_alone(
            &project,
            &binary,
            &project.root().join(".rituals/first"),
            &["create", "../../lint"],
            |message| message.contains(".rituals/"),
        )
    })
}

/// `.rituals/lint` typed from `ritual/` leads to `ritual/.rituals/lint`, so
/// the refusal says where it led and how a path is read, rather than that
/// `.rituals/lint` is not below `.rituals/`.
#[test]
fn a_path_typed_from_a_subdirectory_is_refused_saying_where_it_led() -> TestOutcome {
    in_checkout(|checkout| {
        let (_working_dir, project, binary) =
            project_with_a_first_task(checkout, "create-nested-from-subdirectory")?;

        assert_refused_and_left_alone(
            &project,
            &binary,
            &project.root().join("ritual"),
            &["create", ".rituals/lint"],
            |message| {
                message
                    == "refusing to create .rituals/lint: a path is read from the current \
                        directory, and this one leads to ritual/.rituals/lint; inside a project \
                        every ritual lives below .rituals/ at the project's root, so give a bare \
                        name, or a path that leads below it"
            },
        )
    })
}

#[test]
fn a_source_flag_is_refused_inside_a_project() -> TestOutcome {
    in_checkout(|checkout| {
        let (_working_dir, project, binary) =
            project_with_a_first_task(checkout, "create-source-in-project")?;

        for flag in [
            &["--path", checkout.path_argument()?][..],
            &["--git", "https://example.invalid/ritual.git"],
        ] {
            let mut arguments = vec!["create", "lint"];
            arguments.extend_from_slice(flag);
            assert_refused_and_left_alone(
                &project,
                &binary,
                project.root(),
                &arguments,
                |message| message.contains(flag[0]),
            )?;
        }
        Ok(())
    })
}
