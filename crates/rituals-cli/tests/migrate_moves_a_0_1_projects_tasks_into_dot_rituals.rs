//! `cargo ritual migrate` brings a project laid out the way ritual 0.1 made
//! it up to this version's layout: each task moves from `tasks/<name>` to
//! `.rituals/<name>`, the workspace and every dependency on it follow, and
//! the project builds and runs as before.
//!
//! The fixture is a 0.1 project with two tasks where `shout` depends on
//! `greet`, committed to git because `migrate` only runs where git can give
//! everything back. The resulting manifests are pinned exactly: only the
//! paths change, so comments and layout stay as the person wrote them.

mod support;

use support::migration::{
    assert_nothing_to_migrate, committed_project_where_shout_depends_on_greet, exists,
    fill_in_checkout, moved,
};
use support::tree::Snapshot;
use support::{
    Checkout, Project, TempDir, TestOutcome, assert_trees_identical, crates, git, in_checkout,
    manifest, read_text, snapshot_tree, tree,
};

/// The workspace manifest of that project once migrated: both tasks are
/// explicit members, in the places the old entries were.
const MIGRATED_WORKSPACE: &str = r#"[workspace]
members = [
    "ritual",
    ".rituals/greet",
    ".rituals/shout",
]
resolver = "3"

[workspace.dependencies]
rituals = { path = "@RITUALS@" }
"#;

/// The command line crate's manifest once migrated: both dependencies lead
/// to `.rituals/`, and the key list is as it was.
const MIGRATED_COMMAND_LINE_MANIFEST: &str = r#"[package]
name = "demo-ritual"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "ritual"
path = "src/main.rs"

[dependencies]
rituals.workspace = true
ritual = { package = "rituals-core", path = "@CORE@" }
greet = { path = "../.rituals/greet" }
shout = { path = "../.rituals/shout" }

[package.metadata.ritual]
tasks = ["ritual", "greet", "shout"]
"#;

/// `shout`'s manifest once migrated. `greet` moved beside it, so the way
/// from one to the other is the way it was.
const MIGRATED_DEPENDENT_TASK_MANIFEST: &str = r#"[package]
name = "shout"
version = "0.1.0"
edition = "2024"

[dependencies]
rituals.workspace = true
greet = { path = "../greet" }

[package.metadata.ritual]
task = true
"#;

/// Holds both migrated tasks to their move: each is at `.rituals/<name>`
/// byte for byte as it was at `tasks/<name>`, and `tasks/` is gone with
/// nothing left in it. Compares the project's tree before and after the run.
fn assert_both_tasks_moved_whole(project: &Project, before: &Snapshot, after: &Snapshot) {
    for task in ["greet", "shout"] {
        assert_trees_identical(
            &format!("the {task} task, moved from tasks/{task} to .rituals/{task}"),
            &moved(
                before,
                &format!("tasks/{task}"),
                &format!(".rituals/{task}"),
            ),
            &moved(
                after,
                &format!(".rituals/{task}"),
                &format!(".rituals/{task}"),
            ),
        );
    }
    assert!(
        !exists(&project.root().join("tasks")),
        "expected tasks/ to be removed once nothing was left in it"
    );
}

/// Holds the run to touching only what it should: the two tasks' directories
/// and the two manifests that name them, in the tree before and after it.
fn assert_only_the_tasks_and_two_manifests_changed(before: &Snapshot, after: &Snapshot) {
    let mut expected: Vec<std::path::PathBuf> = before
        .keys()
        .filter(|path| path.starts_with("tasks"))
        .chain(after.keys().filter(|path| path.starts_with(".rituals")))
        .cloned()
        .collect();
    expected.extend(["Cargo.toml".into(), "ritual/Cargo.toml".into()]);
    expected.sort();
    expected.dedup();
    assert_eq!(
        tree::changed_paths(before, after),
        expected,
        "expected only the tasks' directories and the two manifests to change"
    );
}

/// Reads the three manifests the run edited and compares each with its
/// pinned text, so only the paths changed and the person's layout stayed.
fn assert_the_manifests_are_pinned(project: &Project, checkout: &Checkout) -> TestOutcome {
    assert_eq!(
        read_text(&project.workspace_manifest_path())?,
        fill_in_checkout(MIGRATED_WORKSPACE, checkout)?
    );
    assert_eq!(
        read_text(&project.cli_manifest_path())?,
        fill_in_checkout(MIGRATED_COMMAND_LINE_MANIFEST, checkout)?
    );
    assert_eq!(
        read_text(&project.root().join(".rituals/shout/Cargo.toml"))?,
        MIGRATED_DEPENDENT_TASK_MANIFEST
    );
    support::migration::assert_dependency_leads_to(
        &project.root().join(".rituals/shout/Cargo.toml"),
        &["dependencies"],
        "greet",
        &project.root().join(".rituals/greet"),
    )?;
    Ok(())
}

/// Runs each migrated task through the project's own command line, which has
/// to build and print the task's own line.
fn assert_both_tasks_run(project: &Project) -> TestOutcome {
    for task in ["greet", "shout"] {
        let ran = project.run_cli(&[task])?;
        ran.expect_success(&format!("`{task}` after `migrate`"));
        assert!(
            ran.stderr.contains(&crates::ran_line(task))
                || ran.stdout.contains(&crates::ran_line(task)),
            "expected `{task}` to run its own code; stdout was:\n{}\nstderr was:\n{}",
            ran.stdout,
            ran.stderr
        );
    }
    Ok(())
}

#[test]
fn migrate_moves_both_tasks_and_the_command_line_builds_and_runs_them() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-two-tasks")?;
        let project = committed_project_where_shout_depends_on_greet(checkout, &working_dir)?;
        let before = snapshot_tree(project.root())?;

        let migrated = project.run_cli(&["migrate"])?;
        migrated.expect_success("`cargo ritual migrate` on a 0.1 project with two tasks");
        let after = snapshot_tree(project.root())?;

        assert_both_tasks_moved_whole(&project, &before, &after);
        assert_only_the_tasks_and_two_manifests_changed(&before, &after);
        assert_the_manifests_are_pinned(&project, checkout)?;

        // It names what it moved and which manifest it edited.
        support::migration::assert_names(
            &migrated,
            "`migrate`",
            &[
                "tasks/greet",
                ".rituals/greet",
                "tasks/shout",
                ".rituals/shout",
                "ritual/Cargo.toml",
            ],
        );

        assert_both_tasks_run(&project)?;
        Ok(())
    })
}

#[test]
fn a_second_migrate_says_nothing_to_migrate_and_changes_nothing() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-twice")?;
        let project = committed_project_where_shout_depends_on_greet(checkout, &working_dir)?;
        project
            .run_cli(&["migrate"])?
            .expect_success("the first `migrate`");
        // The person reviews the diff and commits it, as `migrate` leaves to
        // them, so the work tree is clean again for the second run.
        project.build()?;
        git::commit_everything(project.root())?;
        let before = snapshot_tree(project.root())?;

        let second = project.run_cli(&["migrate"])?;

        assert_nothing_to_migrate(&second, "a second `migrate`");
        assert_trees_identical(
            "a second `migrate` must change nothing",
            &before,
            &snapshot_tree(project.root())?,
        );
        Ok(())
    })
}

/// The person has not committed what the first run did, so the work tree is
/// dirty, which `migrate` refuses in. A run with nothing to migrate has
/// nothing to give back and writes nothing, so it does not look at git at all
/// and says so, rather than turning a finished migration into a refusal.
#[test]
fn a_second_migrate_before_committing_says_nothing_to_migrate_and_changes_nothing() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-twice-uncommitted")?;
        let project = committed_project_where_shout_depends_on_greet(checkout, &working_dir)?;
        project
            .run_cli(&["migrate"])?
            .expect_success("the first `migrate`");
        project.build()?;
        let status = git::git(project.root(), &["status", "--porcelain"])?;
        assert_ne!(
            status.stdout, "",
            "fixture precondition: the first run's changes must still be uncommitted"
        );
        let before = snapshot_tree(project.root())?;

        let second = project.run_cli(&["migrate"])?;

        assert_nothing_to_migrate(&second, "a second `migrate`, before committing the first");
        assert_trees_identical(
            "a second `migrate` before committing must change nothing",
            &before,
            &snapshot_tree(project.root())?,
        );
        Ok(())
    })
}

#[test]
fn a_project_already_in_the_new_layout_has_nothing_to_migrate() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-new-layout")?;
        let project =
            support::removal::project_with_a_committed_added_task(checkout, &working_dir, "greet")?;
        assert!(
            exists(&project.root().join(".rituals/greet/Cargo.toml")),
            "fixture precondition: `add greet` should have scaffolded .rituals/greet"
        );
        let before = snapshot_tree(project.root())?;

        let migrated = project.run_cli(&["migrate"])?;

        assert_nothing_to_migrate(
            &migrated,
            "`migrate` on a project already in the new layout",
        );
        assert_trees_identical(
            "`migrate` on a project already in the new layout",
            &before,
            &snapshot_tree(project.root())?,
        );
        Ok(())
    })
}

#[test]
fn a_project_with_no_tasks_of_its_own_has_nothing_to_migrate() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-no-tasks")?;
        let project = support::Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;
        let before = snapshot_tree(project.root())?;

        let migrated = project.run_cli(&["migrate"])?;

        assert_nothing_to_migrate(&migrated, "`migrate` on a project with no tasks of its own");
        assert_trees_identical(
            "`migrate` on a project with no tasks of its own",
            &before,
            &snapshot_tree(project.root())?,
        );
        Ok(())
    })
}

#[test]
fn migrate_changes_only_the_paths_in_a_manifest_and_keeps_the_persons_comments() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-keeps-comments")?;
        let project = support::legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        // What a person adds to the files ritual wrote: a trailing comment on
        // the member entry, and a comment above the dependency.
        let workspace_path = project.workspace_manifest_path();
        let commented_workspace = read_text(&workspace_path)?.replace(
            "    \"tasks/greet\",\n",
            "    \"tasks/greet\", # the greeter\n",
        );
        support::write_text(&workspace_path, &commented_workspace)?;
        let command_line_path = project.cli_manifest_path();
        let commented_command_line = read_text(&command_line_path)?.replace(
            "greet = { path = \"../tasks/greet\" }\n",
            "# the greeter lives with the other tooling\ngreet = { path = \"../tasks/greet\" }\n",
        );
        support::write_text(&command_line_path, &commented_command_line)?;
        assert!(
            commented_workspace.contains("# the greeter")
                && commented_command_line.contains("# the greeter lives"),
            "fixture precondition: the comments must be in the manifests"
        );
        project.build()?;
        git::init_and_commit_everything(project.root())?;

        project
            .run_cli(&["migrate"])?
            .expect_success("`migrate` on a project whose manifests carry comments");

        assert_eq!(
            read_text(&workspace_path)?,
            commented_workspace.replace("tasks/greet", ".rituals/greet"),
            "expected only the member's path to change in the workspace manifest"
        );
        assert_eq!(
            read_text(&command_line_path)?,
            commented_command_line.replace("tasks/greet", ".rituals/greet"),
            "expected only the dependency's path to change in the command line's manifest"
        );
        Ok(())
    })
}

#[test]
fn migrate_carries_the_files_git_ignores_along_with_a_task() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-ignored-files")?;
        let project = support::legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        // A file in the task that git ignores, so a clean work tree does not
        // show it and git cannot give it back: it has to travel with the
        // task, and no recovery depends on it.
        let gitignore = project.root().join(".gitignore");
        support::write_text(&gitignore, &format!("{}*.log\n", read_text(&gitignore)?))?;
        support::write_text(
            &project.root().join("tasks/greet/notes.log"),
            "a log only this machine has\n",
        )?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;
        let status = git::git(project.root(), &["status", "--porcelain"])?;
        assert_eq!(
            status.stdout, "",
            "fixture precondition: the work tree must be clean, ignored file apart"
        );

        project
            .run_cli(&["migrate"])?
            .expect_success("`migrate` on a task holding a file git ignores");

        assert_eq!(
            read_text(&project.root().join(".rituals/greet/notes.log"))?,
            "a log only this machine has\n",
            "expected the ignored file to move with its task"
        );
        assert!(
            !exists(&project.root().join("tasks/greet")),
            "expected nothing of the task to be left in tasks/greet"
        );
        let _ = manifest::read(&project.root().join(".rituals/greet/Cargo.toml"))?;
        Ok(())
    })
}
