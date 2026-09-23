//! The support module's own parsers and manifest edits, checked once, here,
//! rather than in every test binary that includes the module.

mod support;

use support::generated::mounted_entries;
use support::help::{command_names, lists_command};
use support::manifest;
use support::tree::{Entry, changed_paths};
use support::{TempDir, TestOutcome, snapshot_tree};

/// `ritual --help`, captured from this repository's own `ritual` binary:
/// a `Usage:` line naming the bin, a `Commands:` block, and an `Options:`
/// block after it.
const CAPTURED_HELP: &str = "Usage: ritual <COMMAND>

Commands:
  add         scaffold a task crate in this project, import it, and regenerate
  regenerate  rewrite src/main.rs from the imported tasks
  new         scaffold a project with a command line of its own
  create      scaffold a task crate on its own, for a project to import later
  help        Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
";

#[test]
fn command_names_reads_exactly_the_commands_block_in_order() {
    assert_eq!(
        command_names(CAPTURED_HELP),
        ["add", "regenerate", "new", "create", "help"]
    );
}

/// `ritual` is on the `Usage:` line, and `Print` begins a description; neither
/// is a command.
#[test]
fn a_name_outside_the_commands_block_or_inside_a_description_is_not_a_command() {
    assert!(!lists_command(CAPTURED_HELP, "ritual"));
    assert!(!lists_command(CAPTURED_HELP, "scaffold"));
    assert!(!lists_command(CAPTURED_HELP, "Print"));
    assert!(lists_command(CAPTURED_HELP, "create"));
}

/// A name that is a prefix of a listed command is not that command.
#[test]
fn a_prefix_of_a_listed_command_is_not_a_command() {
    assert!(!lists_command(CAPTURED_HELP, "re"));
    assert!(!lists_command(CAPTURED_HELP, "regen"));
}

/// A description long enough to wrap continues on a line indented to the
/// description column; its first word is not a command. The input is laid
/// out the way clap wraps a long description.
#[test]
fn a_wrapped_description_line_is_not_a_command() {
    let help = "Usage: demo <COMMAND>

Commands:
  add   scaffold a task crate in this project, import it, and
        regenerate the generated task list
  help  Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
";
    assert_eq!(command_names(help), ["add", "help"]);
}

/// Help with no `Commands:` block lists nothing, rather than reading some
/// other block as commands.
#[test]
fn help_without_a_commands_block_lists_nothing() {
    assert!(
        command_names("Usage: demo [OPTIONS]\n\nOptions:\n  -h, --help  Print help\n").is_empty()
    );
}

fn parse(text: &str) -> toml_edit::DocumentMut {
    let parsed = text.parse::<toml_edit::DocumentMut>();
    assert!(parsed.is_ok(), "{parsed:?}");
    parsed.unwrap_or_default()
}

/// A `[[bin]]` mentioned only in a comment is not a bin target.
#[test]
fn a_bin_table_in_a_comment_is_not_a_bin_target() {
    let document = parse(
        "[package]\nname = \"a-task\"\n# unlike a composed CLI's manifest, this crate has no [[bin]] target\n",
    );
    assert!(manifest::bin_names(&document).is_empty());
}

/// A bin target's path is read as written, and a target that leaves it to
/// Cargo has none.
#[test]
fn bin_targets_reads_each_name_with_its_path() {
    let document = parse(
        "[package]\nname = \"p\"\n\n[[bin]]\nname = \"one\"\npath = \"src/main.rs\"\n\n[[bin]]\nname = \"two\"\n",
    );
    assert_eq!(
        manifest::bin_targets(&document),
        [
            ("one".to_string(), Some("src/main.rs".to_string())),
            ("two".to_string(), None),
        ]
    );
}

/// Renaming the bin leaves `[package] name` alone, even when both held the
/// same string.
#[test]
fn renaming_the_bin_leaves_the_package_name_alone() {
    let mut document =
        parse("[package]\nname = \"same\"\n\n[[bin]]\nname = \"same\"\npath = \"src/main.rs\"\n");
    assert!(manifest::rename_sole_bin(&mut document, "renamed").is_ok());
    assert_eq!(manifest::bin_names(&document), ["renamed"]);
    assert!(matches!(
        manifest::package_name(&document).as_deref(),
        Ok("same")
    ));
}

/// Removing an entry that is not in the list is refused, rather than
/// leaving the manifest unchanged as though it had worked.
#[test]
fn removing_an_absent_task_or_dependency_is_refused() {
    let mut document = parse(
        "[dependencies]\naddendum.workspace = true\n\n[package.metadata.ritual]\ntasks = [\"regenerate\"]\n",
    );
    assert!(manifest::remove_task(&mut document, "add").is_err());
    assert!(manifest::remove_dependency(&mut document, "add").is_err());
    assert!(manifest::remove_task(&mut document, "regenerate").is_ok());
    assert!(matches!(manifest::tasks(&document).as_deref(), Ok([])));
}

#[test]
fn changed_paths_names_added_removed_and_changed_files_once_each() {
    let file = |bytes: &str| Entry::File(bytes.as_bytes().to_vec());
    let before = [
        ("kept", file("same")),
        ("changed", file("old")),
        ("removed", file("gone")),
    ]
    .map(|(path, entry)| (path.into(), entry))
    .into();
    let after = [
        ("kept", file("same")),
        ("changed", file("new")),
        ("added", file("here")),
    ]
    .map(|(path, entry)| (path.into(), entry))
    .into();
    assert_eq!(
        changed_paths(&before, &after),
        ["added", "changed", "removed"].map(std::path::PathBuf::from)
    );
}

/// An empty directory is part of the tree: creating one, and the parent it
/// needed, are both changes.
#[test]
fn a_snapshot_records_an_empty_directory() -> TestOutcome {
    let root = TempDir::new("snapshot-empty-directory")?;
    let before = snapshot_tree(root.path())?;
    std::fs::create_dir_all(root.path().join("tasks/leftover"))?;
    let after = snapshot_tree(root.path())?;
    assert_eq!(
        changed_paths(&before, &after),
        ["tasks", "tasks/leftover"].map(std::path::PathBuf::from)
    );
    assert_eq!(
        after.get(std::path::Path::new("tasks/leftover")),
        Some(&Entry::Directory)
    );
    Ok(())
}

/// A generated file, captured from a project scaffolded by `ritual new demo`
/// after `cargo ritual add my-task`, from its `main` function down.
const CAPTURED_GENERATED_FILE: &str = "#[rustfmt::skip]
fn main() -> std::process::ExitCode {
    rituals::run(
        rituals::identity!(),
        [
            (\"ritual\", ritual::task()),
            (\"my-task\", my_task::task()),
        ],
    )
}
";

#[test]
fn mounted_entries_reads_every_entry_in_order() {
    assert_eq!(
        mounted_entries(CAPTURED_GENERATED_FILE),
        [
            ("ritual".to_string(), "ritual".to_string()),
            ("my-task".to_string(), "my_task".to_string()),
        ]
    );
}

/// The same entries laid out differently are the same entries; a string
/// that only looks like the start of an entry is not one.
#[test]
fn mounted_entries_ignores_layout_and_near_misses() {
    let relaid = "rituals::run(rituals::identity!(), [(\"ritual\",\n ritual::task()), \
                  (\"note\", \"not a task\"), (\"my-task\",my_task::task())])";
    assert_eq!(
        mounted_entries(relaid),
        [
            ("ritual".to_string(), "ritual".to_string()),
            ("my-task".to_string(), "my_task".to_string()),
        ]
    );
    assert!(mounted_entries("fn main() {}").is_empty());
}

/// A dependency's package is its `package` field, or its key when it has
/// none, and an inherited dependency is read where the workspace declares
/// it.
#[test]
fn dependency_package_follows_a_workspace_inherited_dependency() {
    let workspace = parse(
        "[workspace.dependencies]\nritual = { package = \"rituals-core\", path = \"crates/rituals-core\" }\nrituals = { path = \"crates/rituals\" }\n",
    );
    let direct = parse(
        "[dependencies]\nritual = { package = \"rituals-core\", path = \"../rituals-core\" }\n",
    );
    let inherited = parse("[dependencies]\nritual.workspace = true\nrituals.workspace = true\n");

    for manifest in [&direct, &inherited] {
        assert_eq!(
            manifest::dependency_package(manifest, &workspace, "ritual").as_deref(),
            Some("rituals-core")
        );
    }
    assert_eq!(
        manifest::dependency_package(&inherited, &workspace, "rituals").as_deref(),
        Some("rituals")
    );
    assert_eq!(
        manifest::dependency_package(&direct, &workspace, "absent"),
        None
    );
}
