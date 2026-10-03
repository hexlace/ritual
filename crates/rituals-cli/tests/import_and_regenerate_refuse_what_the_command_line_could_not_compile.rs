//! `import` and `regenerate` refuse a task the generated command line could
//! not compile with: a task built on a different `rituals` from the one the
//! project uses, and a key that would hide a crate the generated file names
//! (`std` or `core`), which `add` refuses too. They also refuse a task crate
//! that names no `rituals` of its own, since without one ritual cannot tell
//! which `rituals` its task is built on. Once such a task were written, the command line would
//! no longer build, and with it the `remove` that would take the task back
//! out, so the refusal has to come first.
//!
//! `import` refuses before anything it writes is kept, and leaves the project
//! as it found it, down to the bytes of the lockfile: a key is refused before
//! anything runs, and a task once `cargo add` has said what it is built on,
//! with what `cargo add` wrote put back. `regenerate` refuses the same task
//! written in by hand, through the same rule, and leaves the generated file
//! as it was. `add` refuses such a key before it scaffolds anything.

mod support;

use support::task_sources::{
    LocalRegistry, write_facade_task, write_other_rituals, write_path_task,
    write_path_task_built_on,
};
use support::{
    Project, RunOutput, TempDir, TestOutcome, assert_trees_identical, in_checkout, lockfile,
    path_to_str, run_binary, snapshot_tree,
};

const CRATE: &str = "greeter";

/// What the rollback adds to a refusal made after `cargo add` has written.
const PUT_BACK: &str = "; ritual put the project back as it found it";

/// The release series a person names for `version`: `0.2` for any `0.2.x`,
/// `1` for any `1.x.y`. The refusal tells them to import a release made for
/// the one the project uses.
fn release_series(version: &str) -> String {
    let mut parts = version.split('.');
    match (parts.next(), parts.next()) {
        (Some("0"), Some(minor)) => format!("0.{minor}"),
        (Some(major), _) => major.to_string(),
        (None, _) => version.to_string(),
    }
}

/// Runs the project's built `binary` with `arguments` at its root, asserts
/// it was refused and that the project, lockfile included, is exactly as it
/// was, and returns the refusal.
fn refused_leaving_the_project_as_it_was(
    project: &Project,
    binary: &std::path::Path,
    arguments: &[&str],
) -> support::Outcome<String> {
    let before = snapshot_tree(project.root())?;
    let lockfile_before = lockfile(project.root())?;

    let result: RunOutput = run_binary(binary, project.root(), arguments)?;

    result.expect_failure(&format!("`ritual {}`", arguments.join(" ")));
    assert_trees_identical(
        "a refused run must write nothing",
        &before,
        &snapshot_tree(project.root())?,
    );
    assert_eq!(
        lockfile_before,
        lockfile(project.root())?,
        "a refused run must leave Cargo.lock as it was"
    );
    Ok(result.sole_line_prefixed_with("ritual").to_string())
}

#[test]
fn import_refuses_a_path_task_built_for_another_release_of_rituals() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-other-rituals-release")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let other_rituals = working_dir.path().join("other-rituals");
        write_other_rituals(&other_rituals, "0.0.1")?;
        let task_dir = working_dir.path().join(CRATE);
        write_path_task_built_on(&task_dir, &other_rituals, CRATE, "0.1.0")?;
        let binary = project.build()?;

        let message = refused_leaving_the_project_as_it_was(
            &project,
            &binary,
            &["import", CRATE, "--path", path_to_str(&task_dir)?],
        )?;

        assert_eq!(
            message,
            format!(
                "`{CRATE}` is built for rituals 0.0.1 and this project uses {}; import a release \
                 of it made for {}{PUT_BACK}",
                rituals::VERSION,
                release_series(rituals::VERSION)
            )
        );
        Ok(())
    })
}

/// The same version from another place is still another crate to Rust, so
/// the versions alone would not say why: the refusal names both places. The
/// project's `rituals` comes from the checkout by path, and the task, from a
/// registry nothing patches, is built on the registry's release of the same
/// version, as a task from crates.io is in a project made with `--path`.
#[test]
fn import_refuses_a_registry_task_built_on_the_same_version_of_rituals_from_elsewhere()
-> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-registry-same-rituals-version")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let registry =
            LocalRegistry::install_unpatched(&project, &working_dir.path().join("registry"))?;
        registry.publish_task_built_for_rituals(CRATE, "0.1.0", rituals::VERSION)?;
        let binary = project.build()?;

        let message = refused_leaving_the_project_as_it_was(&project, &binary, &["import", CRATE])?;

        let version = rituals::VERSION;
        assert_eq!(
            message,
            format!(
                "`{CRATE}` is built for rituals {version} from crates.io and this project uses \
                 rituals {version} from {}, which Rust reads as two different crates; build both \
                 on the same rituals{PUT_BACK}",
                checkout.root().join("crates").join("rituals").display()
            )
        );
        Ok(())
    })
}

#[test]
fn import_refuses_a_registry_task_built_for_another_release_of_rituals() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-registry-other-rituals")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let registry =
            LocalRegistry::install(&project, checkout, &working_dir.path().join("registry"))?;
        registry.publish_task_built_for_rituals(CRATE, "0.1.0", "0.0.1")?;
        let binary = project.build()?;

        let message = refused_leaving_the_project_as_it_was(&project, &binary, &["import", CRATE])?;

        assert_eq!(
            message,
            format!(
                "`{CRATE}` is built for rituals 0.0.1 and this project uses {}; import a release \
                 of it made for {}{PUT_BACK}",
                rituals::VERSION,
                release_series(rituals::VERSION)
            )
        );
        Ok(())
    })
}

#[test]
fn regenerate_refuses_a_hand_written_import_of_a_task_built_on_another_rituals() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("regenerate-other-rituals")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let other_rituals = working_dir.path().join("other-rituals");
        write_other_rituals(&other_rituals, "0.0.1")?;
        let task_dir = working_dir.path().join(CRATE);
        write_path_task_built_on(&task_dir, &other_rituals, CRATE, "0.1.0")?;
        // Built before the hand edit: the task cannot compile against the
        // checkout's command line, which is the point.
        let binary = project.build()?;
        project.mount(&task_dir, CRATE, CRATE)?;

        let message = refused_leaving_the_project_as_it_was(&project, &binary, &["regenerate"])?;

        assert_eq!(
            message,
            format!(
                "`{CRATE}` is named in [package.metadata.ritual] tasks, but it is built for \
                 rituals 0.0.1 and this project uses {}; depend on a release of it made for {}, \
                 or drop `{CRATE}` from the list",
                rituals::VERSION,
                release_series(rituals::VERSION)
            )
        );
        Ok(())
    })
}

/// The refusal for a key that would hide the crate of the same name the
/// generated file is built on.
fn hides_a_crate(key: &str) -> String {
    format!(
        "`{key}` would hide Rust's own `{key}` crate, which the generated command line is \
         built on, and it would no longer compile; give this task another key"
    )
}

#[test]
fn import_refuses_the_keys_std_and_core() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-std-core")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let task_dir = working_dir.path().join(CRATE);
        write_path_task(&task_dir, checkout, CRATE, "0.1.0")?;
        let binary = project.build()?;

        for key in ["std", "core"] {
            let message = refused_leaving_the_project_as_it_was(
                &project,
                &binary,
                &["import", CRATE, key, "--path", path_to_str(&task_dir)?],
            )?;

            assert_eq!(message, hides_a_crate(key));
        }
        Ok(())
    })
}

#[test]
fn regenerate_refuses_a_hand_written_task_under_std_or_core() -> TestOutcome {
    in_checkout(|checkout| {
        for key in ["std", "core"] {
            let working_dir = TempDir::new(&format!("regenerate-{key}"))?;
            let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
            let task_dir = working_dir.path().join(CRATE);
            write_path_task(&task_dir, checkout, CRATE, "0.1.0")?;
            let binary = project.build()?;
            project.mount(&task_dir, key, CRATE)?;

            let message =
                refused_leaving_the_project_as_it_was(&project, &binary, &["regenerate"])?;

            assert_eq!(message, hides_a_crate(key));
        }
        Ok(())
    })
}

#[test]
fn add_refuses_the_names_std_and_core() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("add-std-core")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let binary = project.build()?;

        for name in ["std", "core"] {
            let message = refused_leaving_the_project_as_it_was(&project, &binary, &["add", name])?;

            assert_eq!(message, hides_a_crate(name));
            let task_crate_dir = project.root().join("tasks").join(name);
            assert!(
                !task_crate_dir.exists(),
                "a refused `add {name}` must not scaffold {}",
                task_crate_dir.display()
            );
        }
        Ok(())
    })
}

/// A task crate that only re-exports another crate's task does build, but
/// ritual cannot see which `rituals` that task is built on without a direct
/// dependency of its own, so it is refused with the dependency to add.
const FACADE: &str = "facade";

/// The refusal for [`FACADE`], after what `import` or `regenerate` puts in
/// front of it and before what it puts after.
const NAMES_NO_DIRECT_RITUALS: &str = "does not name rituals as a direct dependency, so ritual \
     cannot tell which rituals its task is built on; add `rituals` to its [dependencies]";

#[test]
fn import_refuses_a_task_crate_that_names_no_rituals_of_its_own() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-facade")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let task_dir = working_dir.path().join(CRATE);
        write_path_task(&task_dir, checkout, CRATE, "0.1.0")?;
        let facade_dir = working_dir.path().join(FACADE);
        write_facade_task(&facade_dir, FACADE, &task_dir, CRATE)?;
        let binary = project.build()?;

        let message = refused_leaving_the_project_as_it_was(
            &project,
            &binary,
            &["import", FACADE, "--path", path_to_str(&facade_dir)?],
        )?;

        assert_eq!(
            message,
            format!("`{FACADE}` declares `task = true` but {NAMES_NO_DIRECT_RITUALS}{PUT_BACK}")
        );
        Ok(())
    })
}

#[test]
fn regenerate_refuses_a_hand_written_task_crate_that_names_no_rituals_of_its_own() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("regenerate-facade")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let task_dir = working_dir.path().join(CRATE);
        write_path_task(&task_dir, checkout, CRATE, "0.1.0")?;
        let facade_dir = working_dir.path().join(FACADE);
        write_facade_task(&facade_dir, FACADE, &task_dir, CRATE)?;
        let binary = project.build()?;
        project.mount(&facade_dir, FACADE, FACADE)?;

        let message = refused_leaving_the_project_as_it_was(&project, &binary, &["regenerate"])?;

        assert_eq!(
            message,
            format!(
                "`{FACADE}` is named in [package.metadata.ritual] tasks, but `{FACADE}` \
                 {NAMES_NO_DIRECT_RITUALS}, or drop `{FACADE}` from the list"
            )
        );
        Ok(())
    })
}
