//! `cargo ritual remove` deletes a task's directory only when every file in
//! it is one git can give back. A file that is untracked, or changed since
//! the last commit, would be gone for good, so `remove` refuses, names the
//! files, and writes nothing. A project that is not a git repository can
//! give none of them back: `remove` refuses there too, names the directory,
//! and says to delete it by hand. Neither can a submodule inside the task,
//! even a clean one, since the project's git records only the commit it
//! points at: `remove` refuses it as a repository of its own, by name.
//!
//! Each refusal is checked against the same project once the cause is
//! cleared — the files committed, the repository made, the submodule
//! removed — where `remove` succeeds, so the refusal is shown to be about
//! that cause and not about anything else in the fixture.

mod support;

use support::removal::{
    assert_remove_is_refused_and_writes_nothing, exists, project_with_a_committed_task,
    project_with_an_uncommitted_task,
};
use support::{TempDir, TestOutcome, git, in_checkout, path_to_str, write_text};

#[test]
fn an_untracked_file_in_the_task_makes_remove_refuse_and_name_it() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-untracked")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        write_text(
            &project.root().join("tasks/greet/scratch-notes.txt"),
            "written after the last commit\n",
        )?;

        assert_remove_is_refused_and_writes_nothing(&project, "greet", &["scratch-notes.txt"])?;

        git::commit_everything(project.root())?;
        project
            .run_cli(&["remove", "greet"])?
            .expect_success("`remove greet` once the file is committed");
        assert!(
            !exists(&project.root().join("tasks/greet")),
            "expected tasks/greet to be deleted once nothing in it was at risk"
        );
        Ok(())
    })
}

#[test]
fn an_uncommitted_change_to_a_tracked_file_makes_remove_refuse_and_name_it() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-uncommitted")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        let lib = project.root().join("tasks/greet/src/lib.rs");
        let committed = support::read_text(&lib)?;
        write_text(
            &lib,
            &format!("{committed}\n// edited after the last commit\n"),
        )?;

        assert_remove_is_refused_and_writes_nothing(&project, "greet", &["src/lib.rs"])?;

        git::commit_everything(project.root())?;
        project
            .run_cli(&["remove", "greet"])?
            .expect_success("`remove greet` once the change is committed");
        assert!(
            !exists(&project.root().join("tasks/greet")),
            "expected tasks/greet to be deleted once nothing in it was at risk"
        );
        Ok(())
    })
}

#[test]
fn a_project_that_is_not_a_git_repository_is_refused_naming_the_directory() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-without-git")?;
        let project = project_with_an_uncommitted_task(checkout, &working_dir, "greet")?;
        assert!(
            !exists(&project.root().join(".git")),
            "fixture precondition: the project must not be a git repository"
        );

        assert_remove_is_refused_and_writes_nothing(
            &project,
            "greet",
            &["tasks/greet", "by hand"],
        )?;

        git::init_and_commit_everything(project.root())?;
        project
            .run_cli(&["remove", "greet"])?
            .expect_success("`remove greet` once the project is a committed git repository");
        assert!(
            !exists(&project.root().join("tasks/greet")),
            "expected tasks/greet to be deleted once git could give it back"
        );
        Ok(())
    })
}

#[test]
fn a_clean_submodule_in_the_task_is_refused_as_a_repository_of_its_own() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-submodule")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        let upstream = working_dir.path().join("upstream");
        std::fs::create_dir(&upstream)?;
        write_text(&upstream.join("vendored.txt"), "a file of its own\n")?;
        git::init_and_commit_everything(&upstream)?;
        // Git refuses to clone a local repository as a submodule unless the
        // file transport is allowed, here for this one command.
        git::git(
            &project.root().join("tasks/greet"),
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                path_to_str(&upstream)?,
                "vendor/upstream",
            ],
        )?
        .expect_success("`git submodule add` inside tasks/greet");
        git::commit_everything(project.root())?;
        let status = git::git(project.root(), &["status", "--porcelain"])?;
        assert!(
            status.stdout.is_empty(),
            "fixture precondition: the submodule must leave the project's status clean, got:\n{}",
            status.stdout
        );

        assert_remove_is_refused_and_writes_nothing(
            &project,
            "greet",
            &["tasks/greet/vendor/upstream", "a git repository of its own"],
        )?;

        git::git(project.root(), &["rm", "tasks/greet/vendor/upstream"])?
            .expect_success("`git rm` of the submodule");
        git::commit_everything(project.root())?;
        project
            .run_cli(&["remove", "greet"])?
            .expect_success("`remove greet` once the submodule is gone");
        assert!(
            !exists(&project.root().join("tasks/greet")),
            "expected tasks/greet to be deleted once git could give it back"
        );
        Ok(())
    })
}
