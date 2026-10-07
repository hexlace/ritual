//! `cargo ritual remove` refuses to delete a task's directory that Cargo
//! would still read once it was gone, however the path that reaches it is
//! spelled.
//!
//! Cargo opens a path through the filesystem, so `Tasks/Lint` names
//! `tasks/lint` on a file system that folds case, `alias/lint` names it when
//! `alias` is a link to `tasks`, and an absolute path through a link above
//! the project names it as surely as the resolved path `cargo metadata`
//! reports, the way `/tmp` leads to `/private/tmp` on macOS. Each such
//! spelling in a `[patch]`, an `include` or a target's `path` is refused, with
//! the tree byte-identical, `Cargo.lock` included; then the cause is cleared
//! and the same `remove` succeeds and leaves a project that builds with
//! `--locked`.
//!
//! The other direction holds too: when the task directory is itself a link,
//! deleting it leaves what it points at, so a `[patch]` naming the link's
//! target is no reason to refuse.

mod support;

use support::checkout::report_skip;
use support::removal::{
    assert_remove_is_refused_and_leaves_the_lockfile, exists, project_with_a_committed_task,
};
use support::{Project, TempDir, TestOutcome, git, in_checkout, manifest, path_to_str, write_text};

/// Asserts the project builds now, so a refusal is about what `remove`
/// would do to it and not about a fixture that was already broken. Brings
/// the lockfile up to date with the fixture's own edits.
fn assert_it_builds(project: &Project, when: &str) -> TestOutcome {
    project
        .cargo(&["build"])?
        .expect_success(&format!("`cargo build` {when}"));
    Ok(())
}

/// Removes `lint`, which must now succeed, deleting `tasks/lint` and leaving
/// a project that builds with its lockfile as `remove` left it.
fn assert_removing_lint_succeeds_and_it_builds(project: &Project) -> TestOutcome {
    project
        .run_cli(&["remove", "lint"])?
        .expect_success("`remove lint`");
    assert!(
        !exists(&project.root().join("tasks/lint")),
        "expected tasks/lint to be deleted"
    );
    project
        .cargo(&["build", "--locked"])?
        .expect_success("`cargo build --locked` after `remove lint`");
    Ok(())
}

/// Writes `contents` to `path` under the project root, creating its
/// directory, checks the project builds, and commits. Returns what was
/// there before, if anything, for [`restore`].
fn write_and_commit(
    project: &Project,
    path: &str,
    contents: &str,
) -> support::Outcome<Option<String>> {
    let path_on_disk = project.root().join(path);
    let before = support::read_text(&path_on_disk).ok();
    if let Some(parent) = path_on_disk.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_text(&path_on_disk, contents)?;
    assert_it_builds(project, &format!("with {path} holding:\n{contents}"))?;
    git::commit_everything(project.root())?;
    Ok(before)
}

/// Puts `path` back as `before`, deleting it when there was nothing there,
/// and commits.
fn restore(project: &Project, path: &str, before: Option<&str>) -> TestOutcome {
    let path_on_disk = project.root().join(path);
    match before {
        Some(before) => write_text(&path_on_disk, before)?,
        None => std::fs::remove_file(&path_on_disk)?,
    }
    git::commit_everything(project.root())
}

/// A setting Cargo reads that refers into `tasks/lint`, so deleting that
/// directory breaks the build, written with `spelling` standing for the
/// directory, from the workspace root unless the reference says otherwise.
struct Reference {
    /// The file the setting goes in, from the project root.
    file: &'static str,
    /// What that file holds, given the spelling of `tasks/lint` from the
    /// workspace root and what the file held before.
    contents: fn(&str, Option<&str>) -> String,
    /// What the refusal has to say, given the same spelling.
    expected: fn(&str) -> String,
}

/// Every kind of reference into the task directory this suite spells each
/// way: a `[patch]` in `.cargo/config.toml` and in the workspace manifest,
/// an `include` that is not `optional`, and a target's `path` in another
/// member.
const REFERENCES: [Reference; 4] = [
    Reference {
        file: ".cargo/config.toml",
        contents: |spelling, before| {
            format!(
                "{}\n[patch.crates-io]\nlint = {{ path = \"{spelling}\" }}\n",
                before.unwrap_or_default()
            )
        },
        expected: |_| "[patch.crates-io] lint in ".to_string(),
    },
    Reference {
        file: "Cargo.toml",
        contents: |spelling, before| {
            format!(
                "{}\n[patch.crates-io]\nlint = {{ path = \"{spelling}\" }}\n",
                before.unwrap_or_default()
            )
        },
        expected: |_| "[patch.crates-io] lint".to_string(),
    },
    Reference {
        file: ".cargo/config.toml",
        contents: |spelling, before| {
            format!(
                "include = [\"../{spelling}/settings.toml\"]\n{}",
                before.unwrap_or_default()
            )
        },
        expected: |spelling| format!("`include` of `../{spelling}/settings.toml` in "),
    },
    Reference {
        file: "tasks/fmt/Cargo.toml",
        contents: |spelling, before| {
            format!(
                "{}\n[[bin]]\nname = \"extra\"\npath = \"../../{spelling}/src/extra.rs\"\n",
                before.unwrap_or_default()
            )
        },
        expected: |_| "tasks/lint is also used by `fmt`".to_string(),
    },
];

/// A project with the tasks `lint` and `fmt` committed, and in `lint` the
/// two files the references point at.
fn project_with_lint_and_fmt(
    checkout: &support::Checkout,
    working_dir: &TempDir,
) -> support::Outcome<Project> {
    let project = project_with_a_committed_task(checkout, working_dir, "lint")?;
    support::legacy::add_task(&project, "fmt")?;
    write_text(
        &project.root().join("tasks/lint/settings.toml"),
        "[alias]\n",
    )?;
    write_text(
        &project.root().join("tasks/lint/src/extra.rs"),
        "fn main() {}\n",
    )?;
    git::commit_everything(project.root())?;
    Ok(project)
}

/// Writes every reference with `tasks/lint` spelled `spelling`, each
/// refused with the project left as it was, and then removes `lint`.
fn assert_every_reference_is_refused(project: &Project, spelling: &str) -> TestOutcome {
    for reference in &REFERENCES {
        let before = support::read_text(&project.root().join(reference.file)).ok();
        let contents = (reference.contents)(spelling, before.as_deref());
        write_and_commit(project, reference.file, &contents)?;
        assert_remove_is_refused_and_leaves_the_lockfile(
            project,
            "lint",
            &[&(reference.expected)(spelling)],
            |_| Ok(()),
        )?;
        restore(project, reference.file, before.as_deref())?;
    }
    assert_removing_lint_succeeds_and_it_builds(project)
}

/// Every reference spelled `Tasks/Lint`, which a file system that folds
/// case reads as `tasks/lint`, so Cargo still reads the task directory
/// through it while a comparison of path text would see another directory.
/// A file system that tells case apart reads it as a directory that does not
/// exist, so there the fixture cannot be built, and the story says it was
/// skipped.
#[test]
fn every_reference_in_another_case_is_refused_where_the_file_system_folds_case() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-other-case")?;
        let project = project_with_lint_and_fmt(checkout, &working_dir)?;
        if !project.root().join("TASKS/LINT").exists() {
            report_skip(
                "this file system tells `Tasks/Lint` from `tasks/lint`, so no other case names \
                 the task directory",
            );
            return Ok(());
        }
        assert_every_reference_is_refused(&project, "Tasks/Lint")
    })
}

/// Every reference spelled through `alias`, a link to `tasks` inside the
/// project, which Cargo follows to the task directory while a comparison of
/// path text would see another one.
#[test]
fn every_reference_through_a_link_in_the_project_is_refused() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-through-alias")?;
        let project = project_with_lint_and_fmt(checkout, &working_dir)?;
        std::os::unix::fs::symlink("tasks", project.root().join("alias"))?;
        git::commit_everything(project.root())?;
        assert_every_reference_is_refused(&project, "alias/lint")
    })
}

/// A `[patch]` in `.cargo/config.toml` written as an absolute path through a
/// link above the project, while `cargo metadata` gives `remove` the
/// resolved path. Linked here rather than through `/tmp`, so the story
/// holds wherever the system's temporary directory is not itself a link.
#[test]
fn a_patch_through_a_link_above_the_project_is_refused() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-absolute-through-link")?;
        let project = project_with_lint_and_fmt(checkout, &working_dir)?;
        let through = working_dir.path().join("through");
        std::os::unix::fs::symlink(project.root(), &through)?;
        let spelling = through.join("tasks/lint");
        let spelling = path_to_str(&spelling)?;

        let file = ".cargo/config.toml";
        let before = support::read_text(&project.root().join(file))?;
        write_and_commit(
            &project,
            file,
            &format!("{before}\n[patch.crates-io]\nlint = {{ path = \"{spelling}\" }}\n"),
        )?;
        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "lint",
            &["[patch.crates-io] lint in ", ".cargo/config.toml"],
            |_| Ok(()),
        )?;

        restore(&project, file, Some(&before))?;
        assert_removing_lint_succeeds_and_it_builds(&project)
    })
}

/// A member nested in the task directory, listed through a link: deleting
/// the directory takes it too, so `remove` refuses, naming it, as it does
/// for one listed by the plain path.
#[test]
fn a_member_inside_the_directory_listed_through_a_link_is_refused() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-nested-member-through-link")?;
        let project = project_with_lint_and_fmt(checkout, &working_dir)?;
        std::os::unix::fs::symlink("tasks", project.root().join("alias"))?;
        let inner = project.root().join("tasks/lint/inner");
        std::fs::create_dir_all(inner.join("src"))?;
        write_text(&inner.join("src/lib.rs"), "")?;
        write_text(
            &inner.join("Cargo.toml"),
            "[package]\nname = \"inner\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        let workspace_manifest = project.workspace_manifest_path();
        manifest::edit(&workspace_manifest, |document| {
            manifest::push_member(document, "alias/lint/inner")
        })?;
        assert_it_builds(&project, "with a member listed through a link")?;
        git::commit_everything(project.root())?;

        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "lint",
            &["it also holds the workspace members `inner`"],
            |_| Ok(()),
        )?;
        Ok(())
    })
}

/// `tasks/lint` as a committed link to `vendor/lint`, and a `[patch]` naming
/// `vendor/lint`: deleting the link leaves the target, so the `[patch]` is
/// still read from where it was, `remove` goes through, and the project
/// builds.
#[test]
fn a_patch_naming_the_target_of_a_linked_task_directory_does_not_stop_remove() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-passes-patch-into-link-target")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "lint")?;
        let link = project.root().join("tasks/lint");
        let target = project.root().join("vendor/lint");
        std::fs::create_dir(project.root().join("vendor"))?;
        std::fs::rename(&link, &target)?;
        std::os::unix::fs::symlink("../vendor/lint", &link)?;
        let file = ".cargo/config.toml";
        let before = support::read_text(&project.root().join(file))?;
        write_text(
            &project.root().join(file),
            &format!("{before}\n[patch.crates-io]\nlint = {{ path = \"vendor/lint\" }}\n"),
        )?;
        assert_it_builds(
            &project,
            "with the task behind a link and a [patch] to its target",
        )?;
        git::commit_everything(project.root())?;

        assert_removing_lint_succeeds_and_it_builds(&project)?;
        assert!(
            target.join("Cargo.toml").exists(),
            "expected the link's target to be left alone"
        );
        assert!(
            project.root().join(file).exists(),
            "expected the configuration to be left alone"
        );
        Ok(())
    })
}
