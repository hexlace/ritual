//! `migrate` renames directories, and git decides by a file's path whether to
//! ignore it, which attributes to give it and whether the sparse checkout
//! includes it. A rule keyed on `tasks/` stops applying after the move and one
//! that matches under `.rituals/` starts, so a commit after the move can leave
//! out a file that is committed now, add one that is ignored now, or store one
//! through another filter, and `git status` shows each as an ordinary change.
//! So `migrate` asks git about every file under every task, in the real work
//! tree and in the place it will be, and refuses before it writes anything when
//! git would see any of them differently.
//!
//! Each refusing story refuses first, checks the tree is byte-identical and
//! that the refusal names the file and the rule, then clears the cause and
//! shows the same command now moves the task and git sees the files as it
//! should: a refusal that named the wrong thing, or was about something else,
//! would not survive that.

mod support;

use support::migration::{
    assert_migrate_is_refused_and_writes_nothing, assert_refusal_wrote_nothing, exists,
};
use support::{
    Checkout, Project, TempDir, TestOutcome, git, in_checkout, legacy, read_text, snapshot_tree,
    write_text,
};

/// The rules a project keeps its dot directories out of version control with,
/// but for the ones it needs: the project's own ignore file, then those.
const DOT_DIRECTORIES_IGNORED: &str = "/target\n.*\n!.gitignore\n!.github\n!.cargo\n";

/// A 0.1 project with the task `greet`, committed, whose `.gitignore` keeps
/// its build directory out and nothing else.
fn committed_project(checkout: &Checkout, working_dir: &TempDir) -> support::Outcome<Project> {
    let project = legacy::project_with_tasks(checkout, working_dir, &["greet"])?;
    write_text(&project.root().join(".gitignore"), "/target\n")?;
    project.build()?;
    git::init_and_commit_everything(project.root())?;
    Ok(project)
}

/// Replaces the project's `.gitignore` with `contents`, and commits.
fn commit_with_gitignore(project: &Project, contents: &str) -> TestOutcome {
    write_text(&project.root().join(".gitignore"), contents)?;
    git::commit_everything(project.root())
}

/// Appends `line` to the file at `relative` in the project, and commits.
fn append_and_commit(project: &Project, relative: &str, line: &str) -> TestOutcome {
    let path = project.root().join(relative);
    let mut contents = read_text(&path)?;
    contents.push_str(line);
    write_text(&path, &contents)?;
    git::commit_everything(project.root())
}

/// Asserts `migrate` now moves the project's task, and that git sees the task
/// at its new place the way a commit needs: every file `git add --all` finds
/// there is added.
#[track_caller]
fn assert_migrate_now_moves_greet(project: &Project) -> TestOutcome {
    let migrated = project.run_cli(&["migrate"])?;
    migrated.expect_success("`migrate` once the cause of the refusal was cleared");
    assert!(
        exists(&project.root().join(".rituals/greet/Cargo.toml")),
        "expected .rituals/greet to exist once `migrate` could run"
    );
    assert!(
        !exists(&project.root().join("tasks/greet")),
        "expected tasks/greet to have moved once `migrate` could run"
    );
    git::git(project.root(), &["add", "--all"])?.expect_success("`git add --all` after `migrate`");
    let tracked = git::git(
        project.root(),
        &["ls-files", "--", ".rituals/greet/Cargo.toml"],
    )?;
    assert!(
        tracked.stdout.contains(".rituals/greet/Cargo.toml"),
        "expected a commit after `migrate` to carry .rituals/greet/Cargo.toml; `git ls-files` said \
         {:?}",
        tracked.stdout
    );
    Ok(())
}

#[test]
fn a_project_gitignore_that_ignores_dot_directories_is_refused() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-sight-dot-directories")?;
        let project = committed_project(checkout, &working_dir)?;
        commit_with_gitignore(&project, DOT_DIRECTORIES_IGNORED)?;

        assert_migrate_is_refused_and_writes_nothing(
            &project,
            &[".rituals/greet/Cargo.toml", "`.*`", ".gitignore:2"],
        )?;

        append_and_commit(&project, ".gitignore", "!.rituals\n")?;
        assert_migrate_now_moves_greet(&project)
    })
}

#[test]
fn the_same_rule_only_in_a_global_excludes_file_is_refused() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-sight-global-excludes")?;
        let project = committed_project(checkout, &working_dir)?;
        let global_ignore = working_dir.path().join("global-ignore");
        write_text(&global_ignore, DOT_DIRECTORIES_IGNORED)?;
        let configuration = working_dir.path().join("global-gitconfig");
        write_text(
            &configuration,
            &format!("[core]\n\texcludesFile = {}\n", global_ignore.display()),
        )?;
        let global_ignore = support::path_to_str(&global_ignore)?;
        let before = snapshot_tree(project.root())?;

        let refused =
            project.run_cli_with_global_git_configuration(&["migrate"], &configuration)?;

        assert_refusal_wrote_nothing(
            &project,
            &before,
            &refused,
            &[&format!("{global_ignore}:2"), "`.*`"],
        )?;

        commit_with_gitignore(&project, "/target\n!.rituals\n")?;
        let migrated =
            project.run_cli_with_global_git_configuration(&["migrate"], &configuration)?;
        migrated.expect_success("`migrate` once the project's own rule re-includes .rituals");
        assert!(
            exists(&project.root().join(".rituals/greet/Cargo.toml")),
            "expected .rituals/greet to exist once `migrate` could run"
        );
        Ok(())
    })
}

#[test]
fn an_ignored_env_file_the_new_path_would_not_ignore_is_refused() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-sight-env-file")?;
        let project = committed_project(checkout, &working_dir)?;
        let env_file = project.root().join("tasks/greet/.env");
        write_text(&env_file, "SECRET=1\n")?;
        commit_with_gitignore(&project, "/target\ntasks/greet/.env\n")?;
        assert!(
            snapshot_tree(project.root())?.contains_key(std::path::Path::new("tasks/greet/.env")),
            "fixture precondition: the ignored file must be on disk to be moved"
        );

        assert_migrate_is_refused_and_writes_nothing(
            &project,
            &["tasks/greet/.env", ".gitignore:2"],
        )?;

        append_and_commit(&project, ".gitignore", ".rituals/greet/.env\n")?;
        assert_migrate_now_moves_greet(&project)?;
        assert_eq!(
            read_text(&project.root().join(".rituals/greet/.env"))?,
            "SECRET=1\n",
            "the ignored file must arrive as it was"
        );
        git::git(
            project.root(),
            &["check-ignore", "--quiet", ".rituals/greet/.env"],
        )?
        .expect_success("the moved .env to be ignored where it now is");
        Ok(())
    })
}

#[test]
fn a_gitattributes_rule_keyed_on_tasks_is_refused() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-sight-attributes")?;
        let project = committed_project(checkout, &working_dir)?;
        write_text(
            &project.root().join(".gitattributes"),
            "tasks/**/*.bin filter=lfs diff=lfs merge=lfs -text\n",
        )?;
        std::fs::create_dir_all(project.root().join("tasks/greet/assets"))?;
        std::fs::write(project.root().join("tasks/greet/assets/logo.bin"), b"bin\0")?;
        // An isolated git has no LFS driver, so the commit stores the file as it
        // is: what matters here is that the attributes are decided by path.
        git::commit_everything(project.root())?;

        assert_migrate_is_refused_and_writes_nothing(
            &project,
            &["tasks/greet/assets/logo.bin", "filter=lfs"],
        )?;

        append_and_commit(
            &project,
            ".gitattributes",
            ".rituals/**/*.bin filter=lfs diff=lfs merge=lfs -text\n",
        )?;
        assert_migrate_now_moves_greet(&project)
    })
}

#[test]
fn an_ignore_file_inside_a_task_moves_with_it_and_is_no_reason_to_refuse() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-sight-inner-ignore")?;
        let project = committed_project(checkout, &working_dir)?;
        write_text(&project.root().join("tasks/greet/.gitignore"), ".env\n")?;
        write_text(&project.root().join("tasks/greet/.env"), "SECRET=1\n")?;
        git::commit_everything(project.root())?;

        assert_migrate_now_moves_greet(&project)?;

        assert!(
            exists(&project.root().join(".rituals/greet/.env")),
            "expected the ignored file to move with the task"
        );
        git::git(
            project.root(),
            &["check-ignore", "--quiet", ".rituals/greet/.env"],
        )?
        .expect_success("the moved .env to be ignored by the ignore file that moved with it");
        Ok(())
    })
}
