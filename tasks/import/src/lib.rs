//! The `import` task: import a task crate from a registry, git or a path,
//! and regenerate the task list.

mod arguments;
mod cargo_add;
mod import;
mod refusals;
#[cfg(test)]
mod test_support;

use std::path::Path;

use arguments::ImportArguments;
use import::Import;
use rituals::{CommandLine, Failure, Outcome, Task};
use rituals_compose::{metadata, top_level};

/// This task, for a command line to mount under whatever name imports it.
///
/// Reads the composed CLI's command line it is invoked with, the same
/// opt-in any task can use, to find itself among the workspace's members
/// and, once it has finished writing, to regenerate with.
//
// No `# Examples` section: a `task()` function has exactly one call-site
// shape — a mount line in a generated file — and an example here could only
// restate `let task = import::task();`, which shows nothing a reader needs.
#[must_use]
pub fn task() -> Task {
    Task::receiving_command_line(
        "import a task crate from a registry, git or a path, and regenerate",
        |command_line: &CommandLine, arguments: ImportArguments| run(command_line, &arguments),
    )
}

fn run(command_line: &CommandLine, arguments: &ImportArguments) -> Outcome {
    let current_dir = std::env::current_dir()
        .map_err(|error| Failure::new("reading the current directory failed").caused_by(error))?;
    let import = prepare(command_line, arguments, &current_dir)?;
    import::finish(command_line, &import)
}

/// Decides everything `import` needs before anything is written, refusing at
/// the first thing that is wrong, and returns what the writes need. Each
/// check runs only once the one before it has passed:
///
/// 1. The key: the one given, or the crate's name, which has to be a name a
///    command can have. It comes from what a person typed, and needs no
///    subprocess.
/// 2. That this is the project the running command line belongs to. This is
///    deliberately before the checks on the key against the command line:
///    the running binary's own top level says something about the project
///    only once the binary is known to be the project's, and the other way
///    round, the global `ritual import x add` inside a `--cli acme` project
///    would report a collision instead of handing back `cargo acme ritual
///    import`.
/// 3. That the key is not the bin's own name, which the check after it
///    deliberately lets through (see [`refusals`]), and then that it is not
///    already a top-level command.
/// 4. That the key is not already spoken for in the project's manifest, and
///    that the task list it has resolves. The import ends by regenerating, so
///    a list that is already broken is refused now, while nothing has been
///    written.
fn prepare(
    command_line: &CommandLine,
    arguments: &ImportArguments,
    current_dir: &Path,
) -> Result<Import, Failure> {
    let package = command_line.identity().package_name();
    let import_command = top_level::management_command(command_line, "import");
    let key = arguments.key(&import_command, current_dir)?;
    let again = arguments.to_run_again(current_dir);

    let document = metadata::fetch_in_its_own_project(current_dir, package, "import", &again)?;
    refusals::ensure_the_key_is_not_the_bin_name(&key, command_line.identity().binary_name())?;
    top_level::ensure_command_is_free(command_line, key.as_str())?;
    let project = document.locate_project(package)?;
    refusals::already_imported_refusal(
        package,
        &key,
        project.declares_dependency_key(&key),
        project.lists_task(&key),
        &top_level::management_command(command_line, "regenerate"),
        &format!("{import_command} {again}"),
    )?;
    document.resolve_task_list(package)?;

    Ok(Import {
        package: package.to_string(),
        cargo_add_arguments: cargo_add::arguments(package, &key, arguments),
        key,
        again,
        current_dir: current_dir.to_path_buf(),
        workspace_root: project.workspace_root().to_path_buf(),
        cli_manifest_path: project.manifest_path().to_path_buf(),
        lockfile_path: project.workspace_root().join("Cargo.lock"),
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rituals::{CommandLine, Identity};

    use super::prepare;
    use crate::test_support::{ScratchDir, ScratchProject, TestOutcome, typed_arguments};

    /// The command line of a default project, `ritual`, as the running
    /// binary would hand it to the task: ritual's own bundle flattened into
    /// the top level.
    fn default_command_line() -> CommandLine {
        CommandLine::from_dispatch(
            Identity::from_macro_expansion("demo-ritual", "ritual", "0.1.0"),
            ["add", "regenerate", "new", "create", "import"],
        )
    }

    /// The refusal `prepare` gives for `words` typed in `current_dir`, or
    /// `None` when it prepares an import.
    fn refusal(command_line: &CommandLine, words: &[&str], current_dir: &Path) -> Option<String> {
        prepare(command_line, &typed_arguments(words), current_dir)
            .err()
            .map(|failure| failure.to_string())
    }

    /// A project with `crates` written beside the CLI, each marked as a task.
    fn project_with(
        tag: &str,
        crates: &[&str],
    ) -> Result<ScratchProject, Box<dyn std::error::Error>> {
        let project = ScratchProject::new(tag)?;
        for name in crates {
            project.write_crate(name, true)?;
        }
        Ok(project)
    }

    /// Declares `dependency` in the CLI manifest as a path dependency, and
    /// lists `tasks`, as a project that had already imported them would.
    fn declare(project: &ScratchProject, dependency: &str, tasks: &[&str]) -> TestOutcome {
        let list: Vec<String> = tasks.iter().map(|task| format!("\"{task}\"")).collect();
        std::fs::write(
            project.cli_manifest_path(),
            format!(
                "[package]\nname = \"demo-ritual\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
                 [dependencies]\n{dependency} = {{ path = \"../{dependency}\" }}\n\n\
                 [package.metadata.ritual]\ntasks = [{}]\n",
                list.join(", ")
            ),
        )?;
        Ok(())
    }

    #[test]
    fn a_new_key_in_the_projects_own_directory_is_prepared_for_writing() -> TestOutcome {
        let project = project_with("prepare-new-key", &["greeter"])?;

        let import = prepare(
            &default_command_line(),
            &typed_arguments(&["greeter", "hail", "--path", "../greeter"]),
            &project.cli_dir(),
        )?;

        assert_eq!(import.package, "demo-ritual");
        assert_eq!(import.key.as_str(), "hail");
        assert_eq!(
            import.cargo_add_arguments,
            [
                "add",
                "--package",
                "demo-ritual",
                "--rename",
                "hail",
                "--path",
                "../greeter",
                "--",
                "greeter"
            ]
        );
        assert_eq!(
            import.retry(),
            format!("running `import {}` again", import.again)
        );
        assert!(import.cli_manifest_path.ends_with("cli/Cargo.toml"));
        assert!(import.lockfile_path.ends_with("Cargo.lock"));
        assert_eq!(
            import.lockfile_path.parent(),
            Some(import.workspace_root.as_path())
        );
        Ok(())
    }

    #[test]
    fn outside_any_project_the_refusal_hands_back_the_command_with_an_absolute_path() -> TestOutcome
    {
        let empty = ScratchDir::new("prepare-outside")?;

        let message = refusal(
            &default_command_line(),
            &["greeter", "--path", "../greeter"],
            empty.path(),
        );

        let expected_path = empty.path().join("../greeter");
        assert_eq!(
            message.as_deref(),
            Some(
                format!(
                    "`import` works inside the project this command line belongs to; in your \
                     project, run `cargo ritual import greeter --path {}` (or `cargo <name> \
                     ritual import greeter --path {}` if it was made with `--cli <name>`)",
                    expected_path.display(),
                    expected_path.display()
                )
                .as_str()
            )
        );
        Ok(())
    }

    /// The project is found out before the key is weighed against the running
    /// binary's own top level, which means something only once this is the
    /// project's own binary: from outside, even a key that collides is told to
    /// go to the project first.
    #[test]
    fn the_project_is_checked_before_the_key_is_weighed_against_the_top_level() -> TestOutcome {
        let empty = ScratchDir::new("prepare-order")?;

        let message = refusal(
            &default_command_line(),
            &["greeter", "add", "--path", "../greeter"],
            empty.path(),
        );

        assert!(
            message
                .as_deref()
                .is_some_and(|message| message.starts_with("`import` works inside the project")),
            "expected the outside-project refusal first; got: {message:?}"
        );
        Ok(())
    }

    #[test]
    fn a_crate_name_that_cannot_be_a_key_is_refused_before_the_project_is_looked_at() -> TestOutcome
    {
        let empty = ScratchDir::new("prepare-unusable-key")?;

        let message = refusal(
            &default_command_line(),
            &["my_crate", "--path", "../my_crate"],
            empty.path(),
        );

        assert!(
            message
                .as_deref()
                .is_some_and(|message| message.starts_with("`my_crate` is not a usable name;")),
            "expected the key's refusal first; got: {message:?}"
        );
        Ok(())
    }

    #[test]
    fn the_bin_name_is_refused_as_a_key() -> TestOutcome {
        let project = project_with("prepare-bin-name", &["greeter"])?;

        let message = refusal(
            &default_command_line(),
            &["greeter", "ritual", "--path", "../greeter"],
            &project.cli_dir(),
        );

        assert_eq!(
            message.as_deref(),
            Some(
                "`ritual` is reserved for this command line's own commands; import this crate \
                 under another key"
            )
        );
        Ok(())
    }

    #[test]
    fn a_key_that_is_already_a_top_level_command_is_refused_by_the_shared_check() -> TestOutcome {
        let project = project_with("prepare-flattened", &["greeter"])?;

        let message = refusal(
            &default_command_line(),
            &["greeter", "add", "--path", "../greeter"],
            &project.cli_dir(),
        );

        assert!(
            message.as_deref().is_some_and(|message| message
                .starts_with("`add` would be a top-level command of `ritual` twice")),
            "expected the shared collision refusal; got: {message:?}"
        );
        Ok(())
    }

    #[test]
    fn a_key_that_is_a_dependency_and_listed_is_refused() -> TestOutcome {
        let project = project_with("prepare-already-a-task", &["wake", "greeter"])?;
        declare(&project, "wake", &["wake"])?;

        let message = refusal(
            &default_command_line(),
            &["greeter", "wake", "--path", "../greeter"],
            &project.cli_dir(),
        );

        assert_eq!(
            message.as_deref(),
            Some(
                "`wake` is already a task of `demo-ritual`; import this crate under another \
                 key, or, if its command is missing from the command line, run \
                 `cargo ritual regenerate`"
            )
        );
        Ok(())
    }

    #[test]
    fn a_key_listed_with_no_dependency_is_refused_with_the_import_to_run_again() -> TestOutcome {
        let project = project_with("prepare-listed-only", &["wake", "greeter"])?;
        declare(&project, "wake", &["wake", "greeter"])?;

        let message = refusal(
            &default_command_line(),
            &["greeter", "--path", "../greeter"],
            &project.cli_dir(),
        );

        let path = project.cli_dir().join("../greeter");
        assert_eq!(
            message.as_deref(),
            Some(
                format!(
                    "`greeter` is named in [package.metadata.ritual] tasks but `demo-ritual` has \
                     no dependency called `greeter`; drop it from the list and run \
                     `cargo ritual import greeter --path {}` again",
                    path.display()
                )
                .as_str()
            )
        );
        Ok(())
    }

    /// Cargo accepts `a-b` beside `a_b` and `cargo build` refuses the pair,
    /// so the refusal has to come from here, and name the spelling the
    /// manifest holds.
    #[test]
    fn a_key_that_rust_reads_as_an_existing_dependency_is_refused() -> TestOutcome {
        let project = project_with("prepare-extern-identifier", &["a_b", "greeter"])?;
        declare(&project, "a_b", &[])?;

        let message = refusal(
            &default_command_line(),
            &["greeter", "a-b", "--path", "../greeter"],
            &project.cli_dir(),
        );

        assert!(
            message.as_deref().is_some_and(|message| {
                message.starts_with(
                "`demo-ritual` already has a dependency called `a-b` (or `a_b`, which Rust reads \
                 as the same name) that is not in [package.metadata.ritual] tasks;"
            )
            }),
            "expected the dependency to be found under its other spelling; got: {message:?}"
        );
        Ok(())
    }

    #[test]
    fn a_project_with_no_task_list_is_refused_by_the_resolver_before_anything_is_written()
    -> TestOutcome {
        let project = project_with("prepare-no-list", &["greeter"])?.with_a_lockfile()?;
        std::fs::write(
            project.cli_manifest_path(),
            "[package]\nname = \"demo-ritual\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        let before = project.snapshot()?;

        let message = refusal(
            &default_command_line(),
            &["greeter", "--path", "../greeter"],
            &project.cli_dir(),
        );

        assert!(
            message.as_deref().is_some_and(|message| message
                .starts_with("`demo-ritual` has no [package.metadata.ritual] tasks list in")),
            "expected the resolver's refusal; got: {message:?}"
        );
        assert_eq!(project.snapshot()?, before, "a refusal writes nothing");
        Ok(())
    }
}
