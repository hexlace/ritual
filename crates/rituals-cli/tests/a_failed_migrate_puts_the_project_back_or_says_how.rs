//! A `migrate` that fails partway keeps the promise `remove` makes: it
//! leaves a project that builds in the new layout, or leaves it as it found
//! it, or says plainly that git has everything back and gives the command
//! that does it.
//!
//! Each story makes `migrate` fail where a person could meet it, from
//! outside the process, then holds the result to whichever of those the run
//! chose. A project left byte-identical passes. A project left changed has to
//! print a `git` command in backticks, and running that command, from the
//! project's root, must give back every byte. Neither is fixed here: a run
//! that left the project changed and printed nothing, or printed a command
//! that does not restore it, fails. Whichever way it went, the cause is then
//! cleared and the same `migrate` must work.
//!
//! Failures made with permission bits (a read-only manifest, a read-only
//! `tasks/`) cannot be made by a process that ignores them (root, on Unix),
//! so those stories say so and skip, as the repository's other
//! permission-based checks do. The two that need no permissions make the
//! destination unavailable instead: `.rituals` already exists as a file, and
//! the task's own destination already exists.

mod support;

use std::path::{Path, PathBuf};

use support::migration::{
    assert_failure_kept_the_promise, assert_failure_kept_the_promise_reading, exists,
    made_directory_read_only, made_file_read_only, made_writable,
};
use support::process::run_binary;
use support::{
    Outcome, Project, TempDir, TestOutcome, git, in_checkout, legacy, manifest, read_text,
    snapshot_tree, write_text,
};

/// A committed 0.1 project with two tasks, `shout` using `greet`, and a plain
/// member `tools/helper` outside `tasks/` that also uses `greet`, so `migrate`
/// has a directory to move for each task and a manifest to edit in four
/// places: the workspace's, the command line's, the helper's and none for
/// `shout`, whose way to `greet` is unchanged.
fn a_project_with_work_in_several_places(
    checkout: &support::Checkout,
    working_dir: &TempDir,
) -> Outcome<Project> {
    let project = legacy::project_with_tasks(checkout, working_dir, &["greet", "shout"])?;
    manifest::edit(&project.root().join("tasks/shout/Cargo.toml"), |document| {
        manifest::add_path_dependency(document, &["dependencies"], "greet", "../greet")
    })?;
    support::crates::write_crate(
        &project.root().join("tools/helper"),
        "[package]\nname = \"helper\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
         [dependencies]\ngreet = { path = \"../../tasks/greet\" }\n",
        "//! A crate that is not a task.\n",
    )?;
    manifest::edit(&project.workspace_manifest_path(), |document| {
        manifest::push_member(document, "tools/helper")
    })?;
    project.build()?;
    git::init_and_commit_everything(project.root())?;
    Ok(project)
}

/// Runs `migrate` with the project's already-built command line (so nothing
/// but `migrate` touches the tree), unlocks `unlock` — so a failed assertion
/// cannot leave a read-only file behind and the printed recovery has
/// something to write to — and holds the result to the promise, naming
/// `names`. Then the cause is gone, and `migrate` must work.
fn assert_it_keeps_the_promise_then_works_once_the_cause_is_cleared(
    project: &Project,
    unlock: &[&Path],
    names: &[&str],
    clear_the_cause: impl FnOnce(&Project) -> support::TestOutcome,
) -> TestOutcome {
    let binary = project.build()?;
    let before = snapshot_tree(project.root())?;

    let failed = run_binary(&binary, project.root(), &["migrate"]);
    for path in unlock {
        made_writable(path)?;
    }
    let failed = failed?;

    assert_failure_kept_the_promise(project, "`migrate`", &failed, names, &before)?;

    clear_the_cause(project)?;
    let retried = project.run_cli(&["migrate"])?;
    retried.expect_success("`migrate` once the cause of the failure was cleared");
    for task in ["greet", "shout"] {
        project
            .run_cli(&[task])?
            .expect_success(&format!("`{task}` after the retried `migrate`"));
    }
    assert!(
        exists(&project.root().join(".rituals/greet/Cargo.toml"))
            && !exists(&project.root().join("tasks/greet")),
        "expected the retried `migrate` to have moved the tasks"
    );
    Ok(())
}

/// Reports a skipped story, for a process that ignores permission bits.
fn skip_for_permissions(what: &str) {
    support::checkout::report_skip(&format!(
        "a `migrate` that fails on {what} could not be demonstrated because this process does \
         not honour the read-only permission bit"
    ));
}

#[test]
fn a_migrate_that_cannot_write_the_workspace_manifest_keeps_its_promise() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-fails-workspace-manifest")?;
        let project = a_project_with_work_in_several_places(checkout, &working_dir)?;
        let manifest_path = project.workspace_manifest_path();
        if !made_file_read_only(&manifest_path)? {
            skip_for_permissions("a read-only workspace Cargo.toml");
            return Ok(());
        }
        assert_it_keeps_the_promise_then_works_once_the_cause_is_cleared(
            &project,
            &[&manifest_path],
            &["Cargo.toml"],
            |_| Ok(()),
        )
    })
}

#[test]
fn a_migrate_that_cannot_write_the_command_lines_manifest_keeps_its_promise() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-fails-command-line-manifest")?;
        let project = a_project_with_work_in_several_places(checkout, &working_dir)?;
        let manifest_path = project.cli_manifest_path();
        if !made_file_read_only(&manifest_path)? {
            skip_for_permissions("a read-only command line Cargo.toml");
            return Ok(());
        }
        assert_it_keeps_the_promise_then_works_once_the_cause_is_cleared(
            &project,
            &[&manifest_path],
            &["ritual/Cargo.toml"],
            |_| Ok(()),
        )
    })
}

#[test]
fn a_migrate_that_cannot_write_a_members_manifest_keeps_its_promise() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-fails-helper-manifest")?;
        let project = a_project_with_work_in_several_places(checkout, &working_dir)?;
        let manifest_path = project.root().join("tools/helper/Cargo.toml");
        if !made_file_read_only(&manifest_path)? {
            skip_for_permissions("a read-only member Cargo.toml");
            return Ok(());
        }
        assert_it_keeps_the_promise_then_works_once_the_cause_is_cleared(
            &project,
            &[&manifest_path],
            &["tools/helper"],
            |_| Ok(()),
        )
    })
}

#[test]
fn a_migrate_that_cannot_move_a_task_out_of_tasks_keeps_its_promise() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-fails-tasks-directory")?;
        let project = a_project_with_work_in_several_places(checkout, &working_dir)?;
        let tasks = project.root().join("tasks");
        if !made_directory_read_only(&tasks)? {
            skip_for_permissions("a read-only tasks/ directory");
            return Ok(());
        }
        assert_it_keeps_the_promise_then_works_once_the_cause_is_cleared(
            &project,
            &[&tasks],
            &["tasks"],
            |_| Ok(()),
        )
    })
}

#[test]
fn a_migrate_whose_destination_is_a_file_keeps_its_promise() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-fails-dot-rituals-is-a-file")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet", "shout"])?;
        write_text(&project.root().join(".rituals"), "not a directory\n")?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;

        assert_it_keeps_the_promise_then_works_once_the_cause_is_cleared(
            &project,
            &[],
            &[".rituals"],
            |project| {
                git::git(project.root(), &["rm", "--quiet", "--force", ".rituals"])?
                    .expect_success("`git rm .rituals` to clear the cause");
                git::git(project.root(), &["commit", "--quiet", "--message", "clear"])?
                    .expect_success("`git commit` to clear the cause");
                Ok(())
            },
        )
    })
}

#[test]
fn a_migrate_whose_destination_for_a_task_is_taken_keeps_its_promise() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-fails-destination-taken")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet", "shout"])?;
        // Something already at .rituals/greet, tracked, so the work tree is
        // clean and `migrate` has to notice for itself.
        std::fs::create_dir_all(project.root().join(".rituals/greet"))?;
        write_text(&project.root().join(".rituals/greet/mine.txt"), "mine\n")?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;

        assert_it_keeps_the_promise_then_works_once_the_cause_is_cleared(
            &project,
            &[],
            &[".rituals/greet"],
            |project| {
                git::git(
                    project.root(),
                    &["rm", "-r", "--quiet", "--force", ".rituals"],
                )?
                .expect_success("`git rm -r .rituals` to clear the cause");
                git::git(project.root(), &["commit", "--quiet", "--message", "clear"])?
                    .expect_success("`git commit` to clear the cause");
                Ok(())
            },
        )
    })
}

/// The tree without every file called `name`, wherever it is, and without
/// each directory that held nothing else: a directory a file left behind
/// alone is where the file is, not part of what git can give back.
fn without_file_named(snapshot: &support::tree::Snapshot, name: &str) -> support::tree::Snapshot {
    let mut kept: support::tree::Snapshot = snapshot
        .iter()
        .filter(|(path, _)| path.file_name().is_none_or(|last| last != name))
        .map(|(path, entry)| (path.clone(), entry.clone()))
        .collect();
    let holders: Vec<PathBuf> = snapshot
        .keys()
        .filter(|path| path.file_name().is_some_and(|last| last == name))
        .flat_map(|path| {
            path.ancestors()
                .skip(1)
                .map(Path::to_path_buf)
                .collect::<Vec<_>>()
        })
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .collect();
    // Deepest first, so a directory emptied by dropping its child goes too.
    let mut holders = holders;
    holders.sort_by_key(|holder| std::cmp::Reverse(holder.components().count()));
    for holder in holders {
        let has_anything_left = kept
            .keys()
            .any(|path| path != &holder && path.starts_with(&holder));
        if !has_anything_left {
            kept.remove(&holder);
        }
    }
    kept
}

/// The directories, relative to `root`, that hold a file called `name`.
fn places_holding(snapshot: &support::tree::Snapshot, name: &str) -> Vec<PathBuf> {
    snapshot
        .keys()
        .filter(|path| path.file_name().is_some_and(|last| last == name))
        .cloned()
        .collect()
}

/// A committed 0.1 project with the task `greet`, holding a file git
/// ignores, `tasks/greet/notes.log`, so a clean work tree does not show it
/// and git cannot give it back.
fn project_with_an_ignored_file_in_greet(
    checkout: &support::Checkout,
    working_dir: &TempDir,
) -> Outcome<Project> {
    let project = legacy::project_with_tasks(checkout, working_dir, &["greet"])?;
    let gitignore = project.root().join(".gitignore");
    write_text(&gitignore, &format!("{}*.log\n", read_text(&gitignore)?))?;
    let ignored = project.root().join("tasks/greet/notes.log");
    write_text(&ignored, "a log only this machine has\n")?;
    project.build()?;
    git::init_and_commit_everything(project.root())?;
    Ok(project)
}

/// Holds the ignored file to its own promise once a run has failed: the file
/// is somewhere in the project, in exactly one place, with the bytes it had.
/// Reads the project as it is now, and returns where the file is, relative to
/// the project's root.
fn the_ignored_file_survives_in_one_place(project: &Project) -> Outcome<PathBuf> {
    let after = snapshot_tree(project.root())?;
    let kept = places_holding(&after, "notes.log");
    assert_eq!(
        kept.len(),
        1,
        "expected the ignored file to be kept in exactly one place, found {kept:?}"
    );
    let entry = after.get(&kept[0]);
    assert_eq!(
        entry,
        Some(&support::tree::Entry::File(
            b"a log only this machine has\n".to_vec()
        )),
        "expected the ignored file's bytes to survive intact at {}",
        kept[0].display()
    );
    Ok(kept[0].clone())
}

/// A task's ignored file is the one thing git cannot give back, and a clean
/// work tree does not show it. When `migrate` fails, it has to be where the
/// project says it is: put back where it was, or, if it is left in the new
/// place, named in the output. Left where it was, the retried `migrate`
/// carries it along.
#[test]
fn a_failed_migrate_does_not_lose_a_files_git_ignores() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-fails-with-ignored-file")?;
        let project = project_with_an_ignored_file_in_greet(checkout, &working_dir)?;

        let manifest_path = project.workspace_manifest_path();
        if !made_file_read_only(&manifest_path)? {
            skip_for_permissions("a read-only workspace Cargo.toml");
            return Ok(());
        }
        let binary = project.build()?;
        let before = snapshot_tree(project.root())?;

        let failed = run_binary(&binary, project.root(), &["migrate"]);
        made_writable(&manifest_path)?;
        let failed = failed?;

        // Everything git can give back is held to the promise; the ignored
        // file is held to its own: it is somewhere, intact.
        let tree_without_the_file = || -> Outcome<_> {
            Ok(without_file_named(
                &snapshot_tree(project.root())?,
                "notes.log",
            ))
        };
        assert_failure_kept_the_promise_reading(
            &project,
            "`migrate`",
            &failed,
            &["Cargo.toml"],
            &without_file_named(&before, "notes.log"),
            tree_without_the_file,
        )?;

        let kept = the_ignored_file_survives_in_one_place(&project)?;
        if let Some(place) = kept
            .parent()
            .filter(|place| !place.starts_with("tasks/greet"))
        {
            let written = support::migration::everything_written(&failed);
            assert!(
                written.contains(&place.display().to_string()),
                "expected the output to say the ignored file is in {}; it wrote:\n{written}",
                place.display()
            );
            return Ok(());
        }

        // Left where it was, the file is in the way of nothing: the same
        // `migrate` now works, and the file goes with its task.
        project
            .run_cli(&["migrate"])?
            .expect_success("`migrate` once the cause of the failure was cleared");
        assert_eq!(
            read_text(&project.root().join(".rituals/greet/notes.log"))?,
            "a log only this machine has\n",
            "expected the ignored file to move with its task on the retry"
        );
        Ok(())
    })
}
