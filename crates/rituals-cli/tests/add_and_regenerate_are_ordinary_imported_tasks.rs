//! `add` and `regenerate` are ordinary imported tasks, not framework
//! built-ins. A scaffolded project imports ritual's management bundle —
//! `rituals-core`, whose children are `add`, `regenerate`, `new` and
//! `create` — the way it imports any task: a Cargo dependency, sourced from
//! the same `--path` checkout as `rituals`, plus one entry, `ritual`, in
//! `[package.metadata.ritual] tasks`. Nothing is reserved for those tasks at
//! the assembly step: dropping the bundle the way any import is dropped
//! drops all four of them from the command line.
//!
//! Every phase below shares one scaffolded project, so ritual's crates and
//! their dependencies compile once for the whole story.

mod support;

use support::{
    Project, TempDir, TestOutcome, assert_trees_identical, help, in_checkout, path_to_str,
    snapshot_tree,
};
use support::{generated, manifest};

/// Phase 1 — the project imports ritual's bundle under the key `ritual`,
/// from this checkout's own `crates/rituals-core`, and lists that key as its
/// one task.
fn assert_the_bundle_comes_from_the_checkouts_crates_directory(
    project: &Project,
    checkout: &support::Checkout,
) -> TestOutcome {
    let cli_manifest = project.cli_manifest()?;
    let bundle_source = checkout.root().join("crates/rituals-core");

    assert_eq!(
        manifest::string_at(&cli_manifest, &["dependencies", "ritual", "package"]),
        Some("rituals-core"),
        "expected the dependency `ritual` to be the package `rituals-core`; manifest \
         was:\n{cli_manifest}"
    );
    assert_eq!(
        manifest::string_at(&cli_manifest, &["dependencies", "ritual", "path"]),
        Some(path_to_str(&bundle_source)?),
        "expected ritual's bundle to be sourced from this checkout's own crates/rituals-core; \
         manifest was:\n{cli_manifest}"
    );
    assert_eq!(
        manifest::tasks(&cli_manifest)?,
        ["ritual"],
        "expected the bundle to be the project's one task"
    );

    Ok(())
}

/// Phase 2 — a freshly scaffolded project's `--help` lists the bundle's
/// four children, in the order the bundle declares them, and nothing else
/// but clap's own `help`: they are mounted because the manifest names the
/// bundle, flattened because the project's bin is named `ritual` too.
fn assert_help_lists_the_bundles_children_in_order(project: &Project) -> TestOutcome {
    let help = project.cargo(&["run", "--", "--help"])?;
    help.expect_success("`cargo run -- --help` on a freshly scaffolded project");

    assert_eq!(
        help::command_names(&help.stdout),
        ["add", "regenerate", "new", "create", "help"],
        "stdout was:\n{}",
        help.stdout
    );

    Ok(())
}

/// Phase 3 — a task added later is scaffolded depending on `rituals` alone,
/// inherited from the workspace, and is listed after the bundle's children: `new` wrote the bundle into
/// `tasks` first, `add` appends after it, and flattening keeps that order.
fn assert_a_later_task_follows_the_bundle(project: &Project) -> TestOutcome {
    project
        .alias(&["add", "greet"])?
        .expect_success("`cargo ritual add greet`");

    let task_manifest = manifest::read(&project.root().join("tasks/greet/Cargo.toml"))?;
    assert_eq!(
        manifest::keys_of(&task_manifest, &["dependencies"]),
        ["rituals"],
        "expected the scaffolded task to depend on `rituals` alone; manifest \
         was:\n{task_manifest}"
    );
    assert_eq!(
        manifest::lookup(&task_manifest, &["dependencies", "rituals", "workspace"])
            .and_then(toml_edit::Item::as_bool),
        Some(true),
        "expected the scaffolded task to declare `rituals.workspace = true`; manifest \
         was:\n{task_manifest}"
    );

    let help = project.alias(&["--help"])?;
    help.expect_success("`cargo ritual --help` after `add greet`");
    assert_eq!(
        help::command_names(&help.stdout),
        ["add", "regenerate", "new", "create", "greet", "help"],
        "stdout was:\n{}",
        help.stdout
    );

    Ok(())
}

/// Phase 4 — naming a new task `add` is refused before anything is
/// written, naming `add`, and the refused attempt leaves the mounted
/// tasks working.
///
/// `add` is not a dependency key of this project's own manifest; it is a
/// top-level command because the bundle mounted under the bin's own name
/// flattens it there, and a second command under that name would collide
/// with it.
fn verify_naming_a_task_add_is_refused(project: &Project) -> TestOutcome {
    let before = snapshot_tree(project.root())?;

    let result = project.run_cli(&["add", "add"])?;
    result.expect_failure("`add add`, colliding with the mounted `add`");
    assert!(
        result.sole_line_prefixed_with("ritual").contains("`add`"),
        "expected the refusal to name `add`; stderr was:\n{}",
        result.stderr
    );

    let after = snapshot_tree(project.root())?;
    assert_trees_identical(
        "a refused `add add` must write nothing before refusing",
        &before,
        &after,
    );

    project
        .alias(&["regenerate"])?
        .expect_success("`cargo ritual regenerate` must still work after the refused attempt");

    Ok(())
}

/// Phase 5 — removing ritual's bundle the way any import is removed drops
/// all four of its children from the command line at once, and leaves
/// `greet` in place.
///
/// The `tasks` entry and the dependency cannot go in one edit: the generated
/// file names every mounted crate (`ritual::task()`), and `regenerate` is
/// itself a build of that file. So the entry goes first, `regenerate` runs
/// while the dependency still resolves and rewrites the file without it,
/// and only then does the dependency go. That order is Cargo's, and removing
/// any task, `greet` included, takes the same three steps.
fn verify_removing_the_bundle_drops_all_four_children(project: &Project) -> TestOutcome {
    manifest::edit(&project.cli_manifest_path(), |document| {
        manifest::remove_task(document, "ritual")
    })?;
    regenerate_and_verify_the_bundle_is_dropped_from_the_generated_file(project)?;
    remove_the_bundle_dependency_and_verify_the_command_line(project)
}

/// Phase 5, second step — `regenerate`, while the bundle's dependency still
/// resolves, writes a file that no longer mounts the bundle and still
/// mounts `greet`.
fn regenerate_and_verify_the_bundle_is_dropped_from_the_generated_file(
    project: &Project,
) -> TestOutcome {
    project
        .alias(&["regenerate"])?
        .expect_success("`cargo ritual regenerate` after removing the bundle's tasks entry");

    let generated_file = project.generated_file()?;
    assert_eq!(
        generated::mounted_entries(&generated_file),
        [("greet".to_string(), "greet".to_string())],
        "expected the regenerated file to mount `greet` and no longer ritual's bundle; file \
         was:\n{generated_file}"
    );

    Ok(())
}

/// Phase 5, third step — with nothing left naming the bundle, its
/// dependency goes, and the command line still builds: `--help` lists only
/// `greet` and clap's `help`, and `add` is no longer a command at all —
/// clap, not ritual, refuses it.
fn remove_the_bundle_dependency_and_verify_the_command_line(project: &Project) -> TestOutcome {
    manifest::edit(&project.cli_manifest_path(), |document| {
        manifest::remove_dependency(document, "ritual")
    })?;

    let help_after_removal = project.alias(&["--help"])?;
    help_after_removal.expect_success(
        "`cargo ritual --help` after removing the bundle's tasks entry, regenerating, and \
         removing its dependency",
    );
    assert_eq!(
        help::command_names(&help_after_removal.stdout),
        ["greet", "help"],
        "stdout was:\n{}",
        help_after_removal.stdout
    );

    help::assert_refuses_unrecognized_subcommand(
        &project.run_cli(&["add", "another-task"])?,
        "add",
    );

    Ok(())
}

#[test]
fn add_and_regenerate_behave_as_ordinary_imported_tasks() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("add-regenerate-ordinary-tasks")?;
        let project = Project::scaffold(checkout, working_dir.path(), "ordinary-tasks", &[])?;

        assert_the_bundle_comes_from_the_checkouts_crates_directory(&project, checkout)?;
        assert_help_lists_the_bundles_children_in_order(&project)?;
        assert_a_later_task_follows_the_bundle(&project)?;
        verify_naming_a_task_add_is_refused(&project)?;
        verify_removing_the_bundle_drops_all_four_children(&project)?;

        Ok(())
    })
}
