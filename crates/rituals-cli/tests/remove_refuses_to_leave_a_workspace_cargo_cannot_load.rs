//! `cargo ritual remove` refuses, before it writes anything, to delete a
//! task's directory that Cargo would still read once it was gone.
//!
//! Every check `remove` makes runs while the directory is still there, and a
//! check of the project as it is can pass for a project that breaks the
//! moment the directory goes. So each of these asks about the project
//! without it: a crate that declares a path into the directory, whatever its
//! kind, feature or place, a `members` glob whose last match the directory
//! is, and a `[patch]` or `paths` into it, in the manifest, in
//! `.cargo/config.toml`, in a file that includes, at any depth, or in a
//! member's own `.cargo/config.toml`; and an include, not `optional`, of a
//! file inside it. Each story's project builds, and passes `cargo metadata`,
//! before `remove` runs; each refusal leaves the tree byte-identical,
//! `Cargo.lock` included, and then the cause is cleared and the same
//! `remove` succeeds and leaves a project that builds with `--locked`. An
//! `optional` include whose file is missing is no reason to refuse, and is
//! not one.
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

/// A `[patch]` entry pointing into the directory, in the workspace manifest
/// or in the project's `.cargo/config.toml`, is read by Cargo on every build
/// whether or not anything uses it, so deleting the directory breaks the
/// build. Each is refused, naming the entry, and once both are gone the
/// same `remove` succeeds.
#[test]
fn a_patch_into_the_directory_is_refused_in_the_manifest_and_in_cargo_config() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-patch-into-it")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        let patch = "[patch.crates-io]\ngreet = { path = \"tasks/greet\" }\n";

        let workspace_manifest = project.workspace_manifest_path();
        let manifest_before = support::read_text(&workspace_manifest)?;
        write_text(&workspace_manifest, &format!("{manifest_before}\n{patch}"))?;
        assert_it_builds(&project, "with an unused [patch] in the manifest")?;
        git::commit_everything(project.root())?;
        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "greet",
            &["[patch.crates-io] greet", "remove or repoint it first"],
            |_| Ok(()),
        )?;

        write_text(&workspace_manifest, &manifest_before)?;
        let cargo_config = project.root().join(".cargo/config.toml");
        let config_before = support::read_text(&cargo_config)?;
        write_text(&cargo_config, &format!("{config_before}\n{patch}"))?;
        assert_it_builds(&project, "with an unused [patch] in .cargo/config.toml")?;
        git::commit_everything(project.root())?;
        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "greet",
            &["[patch.crates-io] greet in ", ".cargo/config.toml"],
            |_| Ok(()),
        )?;

        write_text(&cargo_config, &config_before)?;
        git::commit_everything(project.root())?;
        assert_remove_succeeds_and_it_builds(&project, "greet")
    })
}

/// An unused `[patch]` into `tasks/greet`, which Cargo reads on every build
/// wherever it is written.
const PATCH_INTO_GREET: &str = "[patch.crates-io]\ngreet = { path = \"tasks/greet\" }\n";

/// Puts `include` at the top of the project's `.cargo/config.toml`, where a
/// top-level key has to go to stay out of the tables below it, and writes
/// each of `files` under the project root; then checks the project builds
/// and commits it. Returns what `.cargo/config.toml` held before.
fn include_configuration(
    project: &Project,
    include: &str,
    files: &[(&str, &str)],
) -> support::Outcome<String> {
    let cargo_config = project.root().join(".cargo/config.toml");
    let before = support::read_text(&cargo_config)?;
    write_text(&cargo_config, &format!("{include}\n{before}"))?;
    for (path, contents) in files {
        let path = project.root().join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_text(&path, contents)?;
    }
    assert_it_builds(project, &format!("with `{include}` in .cargo/config.toml"))?;
    git::commit_everything(project.root())?;
    Ok(before)
}

/// Puts `.cargo/config.toml` back as `before`, deletes `files`, and commits.
fn clear_configuration(project: &Project, before: &str, files: &[(&str, &str)]) -> TestOutcome {
    write_text(&project.root().join(".cargo/config.toml"), before)?;
    for (path, _) in files {
        std::fs::remove_file(project.root().join(path))?;
    }
    git::commit_everything(project.root())
}

/// Cargo reads every file a configuration file includes, so a `[patch]` or
/// a `paths` override into the directory is read there as surely as in
/// `.cargo/config.toml` itself. Each is refused, naming the included file,
/// and once the include is gone the same `remove` succeeds.
#[test]
fn a_patch_or_paths_in_an_included_file_is_refused_naming_that_file() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-included-config")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;

        for (setting, expected) in [
            (PATCH_INTO_GREET, "[patch.crates-io] greet in "),
            (
                "paths = [\"tasks/greet\"]\n",
                "`paths` entry `tasks/greet` in ",
            ),
        ] {
            let files = [(".cargo/extra.toml", setting)];
            let before = include_configuration(&project, "include = [\"extra.toml\"]", &files)?;
            assert_remove_is_refused_and_leaves_the_lockfile(
                &project,
                "greet",
                &[expected, ".cargo/extra.toml"],
                |_| Ok(()),
            )?;
            clear_configuration(&project, &before, &files)?;
        }

        assert_remove_succeeds_and_it_builds(&project, "greet")
    })
}

/// An included file can include another, and Cargo follows it; so is the
/// check, to the `[patch]` two files down.
#[test]
fn a_patch_two_includes_down_is_refused_naming_its_file() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-nested-include")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        let files = [
            (
                ".cargo/first.toml",
                "include = [{ path = \"second.toml\" }]\n",
            ),
            (".cargo/second.toml", PATCH_INTO_GREET),
        ];

        let before = include_configuration(&project, "include = [\"first.toml\"]", &files)?;
        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "greet",
            &["[patch.crates-io] greet in ", ".cargo/second.toml"],
            |_| Ok(()),
        )?;

        clear_configuration(&project, &before, &files)?;
        assert_remove_succeeds_and_it_builds(&project, "greet")
    })
}

/// Cargo builds without an `optional` include whose file is missing, so one
/// is no reason to refuse: `remove` goes through and the project builds.
#[test]
fn a_missing_optional_include_does_not_stop_remove() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-passes-missing-optional-include")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        include_configuration(
            &project,
            "include = [{ path = \"absent.toml\", optional = true }]",
            &[],
        )?;

        assert_remove_succeeds_and_it_builds(&project, "greet")
    })
}

/// An include that is not `optional` of a file inside the directory: once
/// the directory goes, Cargo refuses to build at all, so `remove` refuses
/// first, naming the include.
#[test]
fn a_required_include_of_a_file_in_the_directory_is_refused() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-include-into-it")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        let include = "include = [\"../tasks/greet/settings.toml\"]";
        let files = [("tasks/greet/settings.toml", "[alias]\n")];

        let before = include_configuration(&project, include, &files)?;
        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "greet",
            &["`include` of `../tasks/greet/settings.toml` in "],
            |_| Ok(()),
        )?;

        clear_configuration(&project, &before, &files)?;
        assert_remove_succeeds_and_it_builds(&project, "greet")
    })
}

/// A build started in a member's directory reads that directory's
/// `.cargo/config.toml`, which a build from the root never sees. A `[patch]`
/// into the task there is refused when `remove` runs from the root, and
/// once it is gone the same `remove` succeeds and the member still builds
/// from its own directory.
#[test]
fn a_patch_in_a_members_own_cargo_config_is_refused_from_the_root() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-member-config")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;
        let member = project.composed_cli_dir().to_path_buf();
        let relative = member
            .strip_prefix(project.root())?
            .join(".cargo/config.toml");
        let back_to_root = "../".repeat(member.strip_prefix(project.root())?.components().count());
        std::fs::create_dir_all(member.join(".cargo"))?;
        write_text(
            &member.join(".cargo/config.toml"),
            &PATCH_INTO_GREET.replace("tasks/greet", &format!("{back_to_root}tasks/greet")),
        )?;
        let build_from_member =
            || support::process::cargo(&member, &project.target_dir(), &["build"]);
        build_from_member()?.expect_success("`cargo build` in the member's directory");
        git::commit_everything(project.root())?;

        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "greet",
            &["[patch.crates-io] greet in ", path_to_str(&relative)?],
            |_| Ok(()),
        )?;

        std::fs::remove_file(member.join(".cargo/config.toml"))?;
        git::commit_everything(project.root())?;
        assert_remove_succeeds_and_it_builds(&project, "greet")?;
        build_from_member()?.expect_success("`cargo build` in the member's directory after");
        Ok(())
    })
}
