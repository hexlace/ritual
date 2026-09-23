//! The main chained story: scaffold a project, add a task to it, build and
//! run it, move it, and rebuild and rerun it — plus the declaration rules
//! that only make sense once a real composed CLI exists to test them
//! against (an unmarked dependency, a name collision with an already-mounted
//! task, and an adversarial name given to `add`).
//!
//! Everything in this file shares one scaffolded project, so ritual's crates
//! and their dependencies compile once for the whole story. Each phase is its
//! own function, so the test itself reads as the list of steps.
//!
//! Steps that a person would take through the project's `cargo ritual` alias
//! go through the alias, so the alias itself is exercised. Every check on
//! what the program wrote to stderr runs the built binary directly instead:
//! the alias is `cargo run`, whose own `Compiling …`/`Running …` lines share
//! that stream and already contain the names these checks look for.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use support::manifest;
use support::{
    OptionContext, Project, ResultContext, TempDir, TestOutcome, assert_trees_identical, help,
    in_checkout, path_to_str, snapshot_tree,
};

/// Phase 1 — a project produced by `new` contains exactly four things at
/// its root: a workspace manifest with an explicit (non-wildcard) member
/// list, a `.gitignore`, the composed CLI crate, and a `.cargo/config.toml`
/// defining the `cargo ritual` alias. Nothing else is written on the caller's behalf.
fn assert_new_writes_exactly_the_specified_things(project: &Project) -> TestOutcome {
    let mut top_level: Vec<String> = fs::read_dir(project.root())
        .context("reading the freshly scaffolded project root failed")?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_, _>>()
        .context("reading a directory entry failed")?;
    top_level.sort();
    let composed_cli_dir_name = project
        .composed_cli_dir()
        .file_name()
        .context("the composed CLI crate's directory has a name")?
        .to_string_lossy()
        .into_owned();
    let mut expected = vec![
        ".cargo".to_string(),
        ".gitignore".to_string(),
        "Cargo.toml".to_string(),
        composed_cli_dir_name,
    ];
    expected.sort();
    assert_eq!(
        top_level, expected,
        "expected exactly the workspace manifest, `.gitignore`, the composed CLI crate and \
         `.cargo/` at the project root"
    );

    let aliases = manifest::keys_of(&project.cargo_config()?, &["alias"]);
    assert_eq!(
        aliases,
        ["ritual"],
        "expected .cargo/config.toml to define exactly the `ritual` alias"
    );

    let members = manifest::workspace_members(&project.workspace_manifest()?)
        .context("expected the workspace manifest to carry a member list")?;
    assert!(
        members.iter().all(|member| !member.contains('*')),
        "expected an explicit member list with no wildcard; members were {members:?}"
    );

    Ok(())
}

/// Phase 2 — a project produced by `new` builds and runs its composed CLI
/// with plain Cargo, and that CLI carries ritual's four management tasks at
/// its top level.
fn assert_fresh_project_builds_and_lists_management_tasks(project: &Project) -> TestOutcome {
    let help = project.cargo(&["run", "--", "--help"])?;
    help.expect_success("`cargo run -- --help` on a freshly scaffolded project");

    for command in ["add", "regenerate", "new", "create"] {
        assert!(
            help::lists_command(&help.stdout, command),
            "expected --help to list the `{command}` task; stdout was:\n{}",
            help.stdout
        );
    }

    Ok(())
}

/// Phase 3 — `add` scaffolds a new task crate under `tasks/`, appends it to
/// the workspace's explicit member list, and the new task builds and runs
/// immediately, with no hand-editing.
fn add_a_task_and_verify_it_builds_and_runs(project: &Project) -> TestOutcome {
    project
        .alias(&["add", "greet"])?
        .expect_success("`cargo ritual add greet`");

    assert!(
        project.root().join("tasks/greet/Cargo.toml").is_file(),
        "expected `add greet` to scaffold tasks/greet/Cargo.toml under {}",
        project.root().display()
    );

    let members = manifest::workspace_members(&project.workspace_manifest()?)
        .context("expected the workspace manifest to carry a member list")?;
    assert!(
        members.iter().any(|member| member == "tasks/greet"),
        "expected `add` to append `tasks/greet` as an explicit workspace member; members \
         were {members:?}"
    );
    assert!(
        members.iter().all(|member| !member.contains('*')),
        "expected the appended member list to stay explicit; members were {members:?}"
    );

    project
        .alias(&["greet"])?
        .expect_success("`cargo ritual greet` immediately after `add greet`, with no hand-editing");

    Ok(())
}

/// Phase 4 — a task author gets working `--help`, and a validation error
/// on an argument the task never declared, without writing any parsing code.
///
/// The scaffold declares no arguments of its own, so this exercises the
/// scaffold's own `--help` and its rejection of a flag it never declared.
fn verify_generated_help_and_validation_plumbing(project: &Project) -> TestOutcome {
    let help = project.run_cli(&["greet", "--help"])?;
    help.expect_success("`greet --help`");
    assert!(
        help.stdout.contains("Usage: ritual greet"),
        "expected `greet --help` to show the task's own usage line; stdout was:\n{}",
        help.stdout
    );

    let bogus_flag = "--this-flag-does-not-exist-zzz123";
    let bogus = project.run_cli(&["greet", bogus_flag])?;
    bogus.expect_failure("`greet` with a flag the scaffold never declared");
    assert!(
        bogus.stderr.contains(bogus_flag),
        "expected the validation error to name the unrecognised flag; stderr was:\n{}",
        bogus.stderr
    );

    Ok(())
}

/// Phase 5 — running `regenerate` twice in a row with no other change
/// produces byte-identical output.
fn verify_regenerate_is_idempotent(project: &Project) -> TestOutcome {
    project
        .alias(&["regenerate"])?
        .expect_success("first `cargo ritual regenerate`");
    let snapshot_after_first = snapshot_tree(project.root())?;

    project
        .alias(&["regenerate"])?
        .expect_success("second `cargo ritual regenerate`, with no change in between");
    let snapshot_after_second = snapshot_tree(project.root())?;

    assert_trees_identical(
        "two consecutive `regenerate` runs with no change in between",
        &snapshot_after_first,
        &snapshot_after_second,
    );

    Ok(())
}

/// Phase 6 — a task crate declared as a local path dependency is relocated
/// by editing only the importing side: the dependency's `path` on the
/// composed CLI, and the workspace member entry Cargo also requires. The
/// moved crate's own files are unchanged byte for byte, and the task still
/// builds and runs.
fn move_task_crate_and_verify_still_works(project: &Project) -> TestOutcome {
    let old_dir = project.root().join("tasks/greet");
    let before_move = snapshot_tree(&old_dir)?;

    let new_dir = project.root().join("relocated-tasks/greet");
    let new_parent = new_dir
        .parent()
        .context("relocated-tasks/greet has a parent")?;
    fs::create_dir_all(new_parent).context("creating the relocation parent directory failed")?;
    fs::rename(&old_dir, &new_dir).context("moving the task crate directory failed")?;

    let after_move = snapshot_tree(&new_dir)?;
    assert_trees_identical(
        "the moved crate's own files, before and after the move",
        &before_move,
        &after_move,
    );

    manifest::edit(&project.cli_manifest_path(), |document| {
        manifest::set_dependency_path(document, "greet", "../relocated-tasks/greet")
    })?;
    manifest::edit(&project.workspace_manifest_path(), |document| {
        manifest::replace_member(document, "tasks/greet", "relocated-tasks/greet")
    })?;

    project.alias(&["greet"])?.expect_success(
        "`cargo ritual greet` after moving its crate and updating only the importing side",
    );

    Ok(())
}

/// Phase 7 — `add` refuses a name that would escape the project, naming the
/// offending name, and writes nothing anywhere.
fn verify_add_rejects_an_adversarial_name(project: &Project) -> TestOutcome {
    let before = snapshot_tree(project.root())?;

    let result = project.run_cli(&["add", "../escape-attempt"])?;
    result.expect_failure("`add ../escape-attempt`");
    assert!(
        result.stderr.contains("../escape-attempt"),
        "expected the refusal to name the offending name; stderr was:\n{}",
        result.stderr
    );

    let after = snapshot_tree(project.root())?;
    assert_trees_identical(
        "a rejected `add` name must write nothing inside the project",
        &before,
        &after,
    );

    let parent_of_project = project
        .root()
        .parent()
        .context("the project has a parent")?;
    assert!(
        !parent_of_project.join("escape-attempt").exists(),
        "a rejected `add ../escape-attempt` must not escape the project directory"
    );

    Ok(())
}

/// Writes a plain crate with no `[package.metadata.ritual]` table at all, in
/// `working_dir`, outside the project.
fn write_unmarked_crate(working_dir: &Path, crate_name: &str) -> support::Outcome<PathBuf> {
    let crate_dir = working_dir.join(crate_name);
    support::crates::write_crate(
        &crate_dir,
        &format!("[package]\nname = \"{crate_name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n"),
        "//! A plain crate with no `[package.metadata.ritual]` table.\n\n\
         /// Does nothing; this crate exists only to be an unmarked dependency.\n\
         pub fn placeholder() {}\n",
    )?;
    Ok(crate_dir)
}

/// Phase 8 — importing a dependency that never declared itself a task is
/// refused, with an error naming that dependency, and the refusal does not
/// disturb the tasks that already worked.
///
/// The manual-import path an `add`-scaffolded crate never takes: an
/// ordinary crate is pulled in with plain `cargo add`, and its name is
/// listed in `tasks = [...]` by hand.
///
/// The composed CLI manifest is restored to its pre-edit text once the
/// refusal is confirmed, and `regenerate` is shown to work on it again.
/// Without that, the unmarked dependency would still sit in `tasks = [...]`
/// and the next phase's `regenerate` would fail for this phase's reason
/// rather than its own.
fn verify_unmarked_dependency_is_refused(working_dir: &Path, project: &Project) -> TestOutcome {
    let dependency = "unmarked-fixture-dependency";
    let cli_manifest_path = project.cli_manifest_path();
    let pristine_manifest = support::read_text(&cli_manifest_path)?;

    let fixture_dir = write_unmarked_crate(working_dir, dependency)?;
    support::cargo(
        project.composed_cli_dir(),
        &project.target_dir(),
        &["add", "--path", path_to_str(&fixture_dir)?],
    )?
    .expect_success("plain `cargo add --path` for the unmarked fixture crate");
    manifest::edit(&cli_manifest_path, |document| {
        manifest::push_task(document, dependency)
    })?;

    let refused = project.run_cli(&["regenerate"])?;
    refused.expect_failure("`regenerate` with an unmarked dependency listed as a task");
    assert!(
        refused.stderr.contains(&format!("`{dependency}`")),
        "expected the refusal to name the unmarked dependency; stderr was:\n{}",
        refused.stderr
    );

    support::write_text(&cli_manifest_path, &pristine_manifest)?;
    project.alias(&["regenerate"])?.expect_success(
        "`cargo ritual regenerate` after restoring the manifest to its pre-refusal state",
    );
    project
        .alias(&["greet"])?
        .expect_success("`cargo ritual greet` must still work after a refused `regenerate`");

    Ok(())
}

/// Phase 9 — giving a task the same name as `regenerate` is refused, naming
/// the task, rather than shadowing the `regenerate` already mounted.
fn verify_task_name_collision_is_refused(project: &Project) -> TestOutcome {
    let collision = project.run_cli(&["add", "regenerate"])?;
    collision.expect_failure("`add regenerate`, colliding with the mounted `regenerate` task");
    assert!(
        collision
            .sole_line_prefixed_with("ritual")
            .contains("`regenerate`"),
        "expected the refusal to name the colliding task; stderr was:\n{}",
        collision.stderr
    );

    project.alias(&["regenerate"])?.expect_success(
        "the mounted `regenerate` task must not be shadowed by the refused attempt",
    );

    Ok(())
}

#[test]
fn task_lifecycle_from_new_through_move() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("task-lifecycle")?;
        let project = Project::scaffold(checkout, working_dir.path(), "my-project", &[])?;

        assert_new_writes_exactly_the_specified_things(&project)?;
        assert_fresh_project_builds_and_lists_management_tasks(&project)?;
        add_a_task_and_verify_it_builds_and_runs(&project)?;
        verify_generated_help_and_validation_plumbing(&project)?;
        verify_regenerate_is_idempotent(&project)?;
        move_task_crate_and_verify_still_works(&project)?;
        verify_add_rejects_an_adversarial_name(&project)?;
        verify_unmarked_dependency_is_refused(working_dir.path(), &project)?;
        verify_task_name_collision_is_refused(&project)?;

        Ok(())
    })
}
