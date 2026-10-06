//! The support module's own parsers and manifest edits, checked once, here,
//! rather than in every test binary that includes the module.

mod support;

use std::path::Path;

use support::generated::mounted_entries;
use support::help::{command_names, lists_command};
use support::manifest;
use support::tree::{Entry, changed_paths};
use support::{ResultContext, TempDir, TestOutcome, run_binary, snapshot_tree, write_text};

/// `ritual --help`, captured from this repository's own `ritual` binary:
/// a `Usage:` line naming the bin, a `Commands:` block, and an `Options:`
/// block after it.
const CAPTURED_HELP: &str = "Usage: ritual <COMMAND>

Commands:
  add         scaffold a task crate in this project, import it, and regenerate
  regenerate  rewrite src/main.rs from the imported tasks
  new         scaffold a project with a command line of its own
  create      scaffold a task crate on its own, for a project to import later
  import      import a task crate from a registry, git or a path, and regenerate
  remove      take a task out of this project, in the order that keeps it building
  migrate     bring this project up to the layout of the ritual it runs
  help        Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
";

#[test]
fn command_names_reads_exactly_the_commands_block_in_order() {
    assert_eq!(
        command_names(CAPTURED_HELP),
        [
            "add",
            "regenerate",
            "new",
            "create",
            "import",
            "remove",
            "migrate",
            "help"
        ]
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

/// An optional path dependency and an exclude list are written where Cargo
/// reads them, and marking a dependency that is not there is refused.
#[test]
fn marking_a_dependency_optional_and_excluding_a_directory_write_what_cargo_reads() {
    let mut document = parse(
        "[workspace]\nmembers = [\"a\"]\n\n[dependencies]\nx = { path = \"../x\" }\ny = \"1\"\n",
    );

    assert!(manifest::mark_optional(&mut document, &["dependencies"], "x").is_ok());
    assert!(manifest::set_workspace_exclude(&mut document, &["vendor/x"]).is_ok());

    assert_eq!(
        manifest::lookup(&document, &["dependencies", "x", "optional"])
            .and_then(toml_edit::Item::as_bool),
        Some(true)
    );
    assert_eq!(
        manifest::lookup(&document, &["workspace", "exclude"])
            .and_then(toml_edit::Item::as_array)
            .map(|excluded| excluded
                .iter()
                .filter_map(toml_edit::Value::as_str)
                .collect()),
        Some(vec!["vendor/x"])
    );
    assert!(manifest::mark_optional(&mut document, &["dependencies"], "y").is_err());
    assert!(manifest::mark_optional(&mut document, &["dependencies"], "absent").is_err());
    assert!(manifest::set_workspace_exclude(&mut parse("[package]\n"), &["x"]).is_err());
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
        "[workspace.dependencies]\n\
         ritual = { package = \"rituals-core\", path = \".rituals/ritual\" }\n\
         rituals = { path = \"crates/rituals\" }\n",
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

/// What ritual 0.1's own `add greet` wrote into a project `new demo` made,
/// captured by running that `add` before `add` moved to `.rituals/`: the root
/// manifest, the command line crate's manifest, the task's manifest and the
/// regenerated command line. `@RITUALS@` and `@CORE@` stand for the
/// checkout's `crates/rituals` and `.rituals/ritual`.
const CAPTURED_0_1_WORKSPACE: &str = r#"[workspace]
members = [
    "ritual",
    "tasks/greet",
]
resolver = "3"

[workspace.dependencies]
rituals = { path = "@RITUALS@" }
"#;

const CAPTURED_0_1_COMMAND_LINE_MANIFEST: &str = r#"[package]
name = "demo-ritual"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "ritual"
path = "src/main.rs"

[dependencies]
rituals.workspace = true
ritual = { package = "rituals-core", path = "@CORE@" }
greet = { path = "../tasks/greet" }

[package.metadata.ritual]
tasks = ["ritual", "greet"]
"#;

const CAPTURED_0_1_TASK_MANIFEST: &str = r#"[package]
name = "greet"
version = "0.1.0"
edition = "2024"

[dependencies]
rituals.workspace = true

[package.metadata.ritual]
task = true
"#;

/// The mounted tasks in the command line 0.1's `add greet` regenerated.
const CAPTURED_0_1_MOUNTS: [(&str, &str); 2] = [("ritual", "ritual"), ("greet", "greet")];

/// The 0.1-layout fixture is what 0.1's `add` wrote, byte for byte in every
/// manifest, so a story that migrates it starts from a real 0.1 project and
/// not from the fixture's own idea of one.
#[test]
fn the_0_1_layout_fixture_reproduces_what_0_1s_add_wrote() -> TestOutcome {
    support::in_checkout(|checkout| {
        let working_dir = TempDir::new("0-1-fixture-shape")?;
        let project = support::legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        let rituals = support::path_to_str(&checkout.root().join("crates/rituals"))?.to_string();
        let core = support::path_to_str(&checkout.root().join(".rituals/ritual"))?.to_string();
        let filled = |captured: &str| {
            captured
                .replace("@RITUALS@", &rituals)
                .replace("@CORE@", &core)
        };

        assert_eq!(
            support::read_text(&project.workspace_manifest_path())?,
            filled(CAPTURED_0_1_WORKSPACE)
        );
        assert_eq!(
            support::read_text(&project.cli_manifest_path())?,
            filled(CAPTURED_0_1_COMMAND_LINE_MANIFEST)
        );
        assert_eq!(
            support::read_text(&project.root().join("tasks/greet/Cargo.toml"))?,
            CAPTURED_0_1_TASK_MANIFEST
        );
        assert_eq!(
            mounted_entries(&project.generated_file()?),
            CAPTURED_0_1_MOUNTS.map(|(key, name)| (key.to_string(), name.to_string()))
        );
        Ok(())
    })
}

/// The rule a stand-in for the developer's own global ignore file holds, and
/// the file it matches. Nothing in a fixture project is named like this, so a
/// match can only have come from the machine.
const MACHINE_ONLY_RULE: &str = "**/machine-only.txt\n";
const MACHINE_ONLY_PATH: &str = "notes/machine-only.txt";

/// A repository holding `MACHINE_ONLY_PATH`, and a stand-in for a developer's
/// home holding `.config/git/ignore` with `MACHINE_ONLY_RULE`.
struct MachineWithAGlobalIgnore {
    repository: TempDir,
    home: TempDir,
}

impl MachineWithAGlobalIgnore {
    fn new(prefix: &str) -> support::Outcome<Self> {
        let repository = TempDir::new(&format!("{prefix}-repository"))?;
        let home = TempDir::new(&format!("{prefix}-home"))?;
        support::git::git(repository.path(), &["init"])?.expect_success("`git init`");
        std::fs::create_dir_all(repository.path().join("notes"))
            .context("creating notes/ failed")?;
        write_text(&repository.path().join(MACHINE_ONLY_PATH), "x\n")?;
        std::fs::create_dir_all(home.path().join(".config/git"))
            .context("creating the stand-in .config/git failed")?;
        write_text(&home.path().join(".config/git/ignore"), MACHINE_ONLY_RULE)?;
        Ok(Self { repository, home })
    }

    /// The `.config` directory git reads `git/ignore` from when
    /// `XDG_CONFIG_HOME` is set to it.
    fn config_home(&self) -> std::path::PathBuf {
        self.home.path().join(".config")
    }

    /// `git check-ignore -v --no-index` on the file only the machine ignores.
    fn check_ignore(&self) -> std::process::Command {
        let mut command = std::process::Command::new("git");
        command
            .args(["check-ignore", "-v", "--no-index", MACHINE_ONLY_PATH])
            .current_dir(self.repository.path())
            .stdin(std::process::Stdio::null());
        command
    }
}

fn ignored_by_the_machine(output: &std::process::Output) -> bool {
    String::from_utf8_lossy(&output.stdout).contains(MACHINE_ONLY_PATH)
}

/// A developer's global ignore file at `$XDG_CONFIG_HOME/git/ignore` is read
/// by git even when `GIT_CONFIG_GLOBAL` is `/dev/null`, so the first half of
/// this story is the positive control: git with only that variable set
/// *does* match. The second half sets the same environment on a command and
/// then isolates it, and git no longer does.
#[test]
fn isolating_git_stops_a_global_ignore_file_in_the_xdg_config_directory() -> TestOutcome {
    let machine = MachineWithAGlobalIgnore::new("isolation-xdg")?;

    let mut unisolated = machine.check_ignore();
    unisolated
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("XDG_CONFIG_HOME", machine.config_home());
    let control = unisolated.output().context("spawning git failed")?;
    assert!(
        ignored_by_the_machine(&control),
        "expected git to read the stand-in global ignore file when only \
         GIT_CONFIG_GLOBAL is set; stdout was:\n{}",
        String::from_utf8_lossy(&control.stdout)
    );

    let mut isolated = machine.check_ignore();
    isolated.env("XDG_CONFIG_HOME", machine.config_home());
    support::git::isolate_from_the_machine(&mut isolated);
    let output = isolated.output().context("spawning git failed")?;
    assert!(
        !ignored_by_the_machine(&output),
        "an isolated git must not read a global ignore file; stdout was:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        output.stderr.is_empty(),
        "isolating git must not make it warn; stderr was:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

/// With `XDG_CONFIG_HOME` unset git reads `$HOME/.config/git/ignore`, which is
/// where a developer's file is on a machine that never set the variable.
/// Isolating must stop that fallback too.
#[test]
fn isolating_git_stops_a_global_ignore_file_under_the_home_directory() -> TestOutcome {
    let machine = MachineWithAGlobalIgnore::new("isolation-home")?;

    let mut isolated = machine.check_ignore();
    isolated.env("HOME", machine.home.path());
    support::git::isolate_from_the_machine(&mut isolated);
    let output = isolated.output().context("spawning git failed")?;

    assert!(
        !ignored_by_the_machine(&output),
        "an isolated git must not read $HOME/.config/git/ignore; stdout was:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
}

/// The shell script a story's binary runs: `git check-ignore` on the
/// machine-only file, with `HOME` standing in for a developer's home.
///
/// A story's binary is started by [`run_binary`] and nothing else, so the
/// script, not the parent environment, carries the developer's home in.
fn check_ignore_script(machine: &MachineWithAGlobalIgnore) -> String {
    format!(
        "HOME={} exec git check-ignore -v --no-index {MACHINE_ONLY_PATH}",
        machine.home.path().display()
    )
}

#[test]
fn a_binary_started_by_a_story_does_not_read_the_machines_global_ignore_file() -> TestOutcome {
    let machine = MachineWithAGlobalIgnore::new("isolation-run-binary")?;

    let output = run_binary(
        Path::new("sh"),
        machine.repository.path(),
        &["-c", &check_ignore_script(&machine)],
    )?;

    assert!(
        !output.stdout.contains(MACHINE_ONLY_PATH),
        "a binary run by a story must not see the machine's global ignore file; stdout was:\n{}",
        output.stdout
    );
    Ok(())
}

/// A `GIT_CONFIG_COUNT` environment, with the injected `core.excludesFile`
/// naming `ignore_file`, the way a tool that starts a developer's shell can
/// hand git a configuration no file holds.
fn inject_excludes_file_by_count(command: &mut std::process::Command, ignore_file: &Path) {
    command
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.excludesFile")
        .env("GIT_CONFIG_VALUE_0", ignore_file);
}

/// `git check-ignore` reads configuration that arrives in the environment as
/// well as in files, and a `GIT_CONFIG_GLOBAL` of `/dev/null` does not stop
/// it. The positive control hands an unisolated git an `excludesFile` through
/// `GIT_CONFIG_COUNT` and it matches; the same environment on an isolated
/// command does not.
#[test]
fn isolating_git_stops_configuration_injected_through_a_count() -> TestOutcome {
    let machine = MachineWithAGlobalIgnore::new("isolation-count")?;
    let ignore_file = machine.home.path().join(".config/git/ignore");

    let mut unisolated = machine.check_ignore();
    unisolated
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("XDG_CONFIG_HOME", "/dev/null");
    inject_excludes_file_by_count(&mut unisolated, &ignore_file);
    let control = unisolated.output().context("spawning git failed")?;
    assert!(
        ignored_by_the_machine(&control),
        "expected git to read an excludesFile injected through GIT_CONFIG_COUNT; \
         stdout was:\n{}",
        String::from_utf8_lossy(&control.stdout)
    );

    let mut isolated = machine.check_ignore();
    inject_excludes_file_by_count(&mut isolated, &ignore_file);
    support::git::isolate_from_the_machine(&mut isolated);
    let output = isolated.output().context("spawning git failed")?;
    assert!(
        !ignored_by_the_machine(&output),
        "an isolated git must not read configuration injected through \
         GIT_CONFIG_COUNT; stdout was:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
}

/// `GIT_CONFIG_PARAMETERS` is the older way a parent hands git configuration,
/// and git also reads it. Same shape as the count test: unisolated matches,
/// isolated does not.
#[test]
fn isolating_git_stops_configuration_injected_through_parameters() -> TestOutcome {
    let machine = MachineWithAGlobalIgnore::new("isolation-parameters")?;
    let ignore_file = machine.home.path().join(".config/git/ignore");
    let parameters = format!("'core.excludesFile'='{}'", ignore_file.display());

    let mut unisolated = machine.check_ignore();
    unisolated
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("XDG_CONFIG_HOME", "/dev/null")
        .env("GIT_CONFIG_PARAMETERS", &parameters);
    let control = unisolated.output().context("spawning git failed")?;
    assert!(
        ignored_by_the_machine(&control),
        "expected git to read an excludesFile injected through \
         GIT_CONFIG_PARAMETERS; stdout was:\n{}",
        String::from_utf8_lossy(&control.stdout)
    );

    let mut isolated = machine.check_ignore();
    isolated.env("GIT_CONFIG_PARAMETERS", &parameters);
    support::git::isolate_from_the_machine(&mut isolated);
    let output = isolated.output().context("spawning git failed")?;
    assert!(
        !ignored_by_the_machine(&output),
        "an isolated git must not read configuration injected through \
         GIT_CONFIG_PARAMETERS; stdout was:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(())
}

/// A fresh task's manifest as `create` writes it for each audience.
const PRIVATE_MANIFEST: &str = "[package]\nname = \"lint\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\
    publish = false\n\n[dependencies]\nrituals.workspace = true\n\n[package.metadata.ritual]\n\
    task = true\n";
const PUBLIC_MANIFEST: &str = "[package]\nname = \"lint\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
    [dependencies]\nrituals.workspace = true\n\n[package.metadata.ritual]\ntask = true\n";

/// The audience check tells the two manifests apart, in both directions, and
/// names what differs, so a story that asserts on it cannot pass on either.
#[test]
fn the_audience_check_tells_a_private_manifest_from_a_public_one() {
    use support::created::{Audience, manifest_problem};

    let private = parse(PRIVATE_MANIFEST);
    let public = parse(PUBLIC_MANIFEST);
    assert_eq!(
        manifest_problem(&private, "lint", Audience::Private, true),
        None
    );
    assert_eq!(
        manifest_problem(&public, "lint", Audience::Public, true),
        None
    );
    assert!(
        manifest_problem(&private, "lint", Audience::Public, true)
            .is_some_and(|problem| problem.contains("publish")),
        "a private manifest is not a public one"
    );
    assert!(
        manifest_problem(&public, "lint", Audience::Private, true)
            .is_some_and(|problem| problem.contains("publish")),
        "a public manifest is not a private one"
    );
    assert!(manifest_problem(&private, "other", Audience::Private, true).is_some());
    let published_true = parse(&PRIVATE_MANIFEST.replace("publish = false", "publish = true"));
    assert!(manifest_problem(&published_true, "lint", Audience::Private, true).is_some());
    let not_inherited =
        parse(&PRIVATE_MANIFEST.replace("rituals.workspace = true", "rituals = \"1\""));
    assert!(manifest_problem(&not_inherited, "lint", Audience::Private, true).is_some());
    assert_eq!(
        manifest_problem(&not_inherited, "lint", Audience::Private, false),
        None
    );
}

/// The fresh `src/lib.rs` the stories compare against is what the shipped
/// binary writes outside a project, where nothing about the file depends on
/// who the ritual is for.
#[test]
fn the_fresh_library_text_is_what_a_standalone_create_writes() -> TestOutcome {
    support::in_checkout(|checkout| {
        let working_dir = TempDir::new("fresh-lib-text")?;
        support::run_ritual(
            working_dir.path(),
            &["create", "lint", "--path", checkout.path_argument()?],
        )?
        .expect_success("`ritual create lint`");
        assert_eq!(
            support::read_text(&working_dir.path().join("lint/src/lib.rs"))?,
            support::created::fresh_lib("lint")
        );
        Ok(())
    })
}

/// Folding the command line into the root leaves one manifest with both a
/// `[package]` and a `[workspace]`, no `ritual/` directory, and a project
/// Cargo still loads.
#[test]
fn folding_the_command_line_into_the_root_leaves_a_project_cargo_loads() -> TestOutcome {
    support::in_checkout(|checkout| {
        let working_dir = TempDir::new("fold-the-command-line")?;
        let scaffolded = support::Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let project = support::root_cli::fold_the_command_line_into_the_root(&scaffolded)?;

        assert_eq!(
            project.cli_manifest_path(),
            project.workspace_manifest_path()
        );
        assert!(!project.root().join("ritual").exists());
        let root = project.workspace_manifest()?;
        assert!(root.get("package").is_some() && root.get("workspace").is_some());
        assert_eq!(manifest::workspace_members(&root), Some(vec![]));
        assert_eq!(manifest::tasks(&root)?, ["ritual"]);
        project.build()?;
        Ok(())
    })
}

/// The two ways of unsettling the lockfile each leave it as claimed, and each
/// proves with Cargo that reading the project would change it.
#[test]
fn the_unsettled_lockfile_helpers_leave_what_they_claim() -> TestOutcome {
    support::in_checkout(|checkout| {
        let working_dir = TempDir::new("unsettle-the-lockfile")?;
        let project = support::legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        let lockfile = project.root().join("Cargo.lock");
        assert!(support::read_text(&lockfile)?.contains("name = \"greet\""));

        support::created::leave_the_lockfile_stale(&project, "greet")?;
        let stale = support::read_text(&lockfile)?;
        assert!(!stale.contains("name = \"greet\""), "greet is dropped");
        assert!(stale.contains("name = \"demo-ritual\""), "the rest is kept");

        support::created::leave_no_lockfile(&project)?;
        assert!(!lockfile.exists());

        assert!(
            support::created::leave_the_lockfile_stale(&project, "nosuch").is_err(),
            "a package the lockfile does not name cannot be dropped from it"
        );
        Ok(())
    })
}
