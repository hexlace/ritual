//! `cargo ritual remove` refuses, before it writes anything, to delete a
//! task's directory that Cargo would still read once it was gone.
//!
//! Every check `remove` makes runs while the directory is still there, and a
//! check of the project as it is can pass for a project that breaks the
//! moment the directory goes. So each of these asks about the project
//! without it: a crate that declares a path into the directory, whatever its
//! kind, feature or place, and a `members` glob whose last match the
//! directory is. Each story's project builds, and passes `cargo metadata`,
//! before `remove` runs; each refusal leaves the tree byte-identical,
//! `Cargo.lock` included, and then the cause is cleared and the same
//! `remove` succeeds and leaves a project that builds with `--locked`.
//!
//! A `members` entry spelled the way Cargo reads it but not the way it was
//! written by `add` — `tasks/./lint`, `tasks//lint`, `x/../tasks/lint`, the
//! absolute path — is not a refusal: `remove` reads it as Cargo does, takes
//! it out, and the project builds.

mod support;

use support::removal::{
    assert_remove_is_refused_and_leaves_the_lockfile, exists, members_of,
    project_with_a_committed_task,
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

/// Adds the task `name` with `add`, then commits everything.
fn add_and_commit(project: &Project, name: &str) -> TestOutcome {
    project
        .alias(&["add", name])?
        .expect_success(&format!("`cargo ritual add {name}`"));
    git::commit_everything(project.root())
}

/// Removes `name`, which must now succeed, deleting its directory and
/// leaving a project that builds with its lockfile as `remove` left it.
fn assert_remove_succeeds_and_it_builds(project: &Project, name: &str) -> TestOutcome {
    project
        .run_cli(&["remove", name])?
        .expect_success(&format!("`remove {name}`"));
    assert!(
        !exists(&project.root().join("tasks").join(name)),
        "expected tasks/{name} to be deleted"
    );
    project
        .cargo(&["build", "--locked"])?
        .expect_success(&format!("`cargo build --locked` after `remove {name}`"));
    Ok(())
}

/// Delphi's first reproduction: `fmt` depends on `lint` behind a feature
/// nothing turns on, so the resolved graph has no edge for it, and Cargo
/// still reads `lint`'s manifest to load the workspace.
#[test]
fn an_optional_dependent_behind_a_feature_is_refused_by_name() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-optional-dependent")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "lint")?;
        add_and_commit(&project, "fmt")?;
        let fmt_manifest = project.root().join("tasks/fmt/Cargo.toml");
        manifest::edit(&fmt_manifest, |document| {
            document["dependencies"]["lint"] =
                toml_edit::Item::Value(toml_edit::Value::InlineTable(
                    [
                        ("path", toml_edit::Value::from("../lint")),
                        ("optional", toml_edit::Value::from(true)),
                    ]
                    .into_iter()
                    .collect(),
                ));
            document["features"]["with-lint"] =
                toml_edit::value(toml_edit::Array::from_iter(["dep:lint"]));
            Ok(())
        })?;
        assert_it_builds(&project, "with the optional dependency")?;
        git::commit_everything(project.root())?;

        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "lint",
            &["tasks/lint is also used by `fmt`"],
            |_| Ok(()),
        )?;

        manifest::edit(&fmt_manifest, |document| {
            manifest::remove_dependency(document, "lint")?;
            document.remove("features");
            Ok(())
        })?;
        git::commit_everything(project.root())?;
        assert_remove_succeeds_and_it_builds(&project, "lint")
    })
}

/// A crate outside the workspace that depends on `lint`, reached from a
/// member only through an optional dependency: `cargo metadata` does not
/// list it at all, and Cargo still reads it, and so `lint`, when it
/// resolves the lockfile.
#[test]
fn a_dependent_outside_the_workspace_is_refused_by_name() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-outside-dependent")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "lint")?;
        add_and_commit(&project, "fmt")?;

        let helper = working_dir.path().join("helper");
        std::fs::create_dir_all(helper.join("src"))?;
        write_text(&helper.join("src/lib.rs"), "")?;
        write_text(
            &helper.join("Cargo.toml"),
            &format!(
                "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
                 [dependencies]\nlint = {{ path = \"{}\" }}\n\n[workspace]\n",
                path_to_str(&project.root().join("tasks/lint"))?
            ),
        )?;
        let fmt_manifest = project.root().join("tasks/fmt/Cargo.toml");
        let helper_path = path_to_str(&helper)?.to_string();
        manifest::edit(&fmt_manifest, |document| {
            document["dependencies"]["helper"] =
                toml_edit::Item::Value(toml_edit::Value::InlineTable(
                    [
                        ("path", toml_edit::Value::from(helper_path.as_str())),
                        ("optional", toml_edit::Value::from(true)),
                    ]
                    .into_iter()
                    .collect(),
                ));
            Ok(())
        })?;
        assert_it_builds(&project, "with the dependency outside the workspace")?;
        git::commit_everything(project.root())?;

        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "lint",
            &["tasks/lint is also used by `helper`"],
            |_| Ok(()),
        )?;

        manifest::edit(&fmt_manifest, |document| {
            manifest::remove_dependency(document, "helper")
        })?;
        git::commit_everything(project.root())?;
        assert_remove_succeeds_and_it_builds(&project, "lint")
    })
}

/// Delphi's third reproduction: with `members = ["ritual", "tasks/*"]` and
/// one task, the glob's only match is the directory, and Cargo reads a glob
/// that matches nothing as a literal path.
#[test]
fn the_last_match_of_a_members_glob_is_refused_naming_the_glob() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-last-glob-match")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        let workspace_manifest = project.workspace_manifest_path();
        manifest::edit(&workspace_manifest, |document| {
            manifest::replace_member(document, "tasks/greet", "tasks/*")
        })?;
        assert_it_builds(&project, "with a glob in members")?;
        git::commit_everything(project.root())?;

        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "greet",
            &["`tasks/*`", "add an explicit member, or remove it"],
            |_| Ok(()),
        )?;

        // A second task under the glob keeps it matching something.
        add_and_commit(&project, "other")?;
        assert_remove_succeeds_and_it_builds(&project, "greet")?;
        assert!(
            members_of(&project)?
                .iter()
                .any(|member| member == "tasks/*"),
            "expected the glob the person wrote to stay"
        );
        Ok(())
    })
}

/// Delphi's fourth reproduction: a `members` entry Cargo reads as the
/// directory, spelled some other way. Each is taken out, and the project
/// builds.
#[test]
fn a_member_entry_spelled_any_way_cargo_reads_it_is_taken_out() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-member-spellings")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "lint")?;
        let absolute = project.root().join("tasks/abs");
        let spellings = [
            ("lint", "tasks/./lint".to_string()),
            ("dbl", "tasks//dbl".to_string()),
            ("dots", "x/../tasks/dots".to_string()),
            ("abs", path_to_str(&absolute)?.to_string()),
        ];
        for (task, _) in &spellings[1..] {
            add_and_commit(&project, task)?;
        }
        let workspace_manifest = project.workspace_manifest_path();
        manifest::edit(&workspace_manifest, |document| {
            for (task, spelling) in &spellings {
                manifest::replace_member(document, &format!("tasks/{task}"), spelling)?;
            }
            Ok(())
        })?;
        assert_it_builds(&project, "with members spelled four ways")?;
        git::commit_everything(project.root())?;

        for (task, spelling) in &spellings {
            assert_remove_succeeds_and_it_builds(&project, task)?;
            assert!(
                !members_of(&project)?.contains(spelling),
                "expected the entry {spelling:?} to be taken out with tasks/{task}"
            );
            git::commit_everything(project.root())?;
        }
        Ok(())
    })
}
