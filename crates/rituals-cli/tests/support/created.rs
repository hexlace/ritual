//! What `create` leaves behind, written out once for the stories that read it.
//!
//! The values here are fixed by what a task scaffolded into a project has
//! always looked like (the files, the manifest tables, the report lines the
//! earlier `add` printed, with `.rituals/<name>` for the directory) plus the
//! one thing that is new: a private ritual says `publish = false` in its
//! `[package]` and a public one has no `publish` key. None of it is read
//! back from ritual's own code.

use std::path::Path;

use toml_edit::DocumentMut;

use super::process::{cargo_query, run_binary};
use super::{
    OptionContext, Outcome, Project, ResultContext, RunOutput, TestOutcome, failure, generated,
    manifest, read_text, snapshot_tree, tree,
};

/// The line the deprecated `add` prints on standard error.
pub(crate) const ADD_IS_NOW_CREATE: &str = "add is now create, and will be removed in ritual 0.3.0";

/// Who a scaffolded ritual is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Audience {
    /// The default: `publish = false`.
    Private,
    /// `--public`: no `publish` key.
    Public,
}

impl Audience {
    /// The `create` arguments that choose this audience.
    pub(crate) const fn flags(self) -> &'static [&'static str] {
        match self {
            Self::Private => &[],
            Self::Public => &["--public"],
        }
    }
}

/// A fresh task's `src/lib.rs`, exactly.
pub(crate) fn fresh_lib(name: &str) -> String {
    format!(
        "//! The `{name}` task.

use rituals::{{Outcome, Task, clap, report}};

/// What this task accepts on the command line.
#[derive(clap::Args)]
struct Arguments {{}}

/// This task, for a command line to mount under whatever name imports it.
#[must_use]
pub fn task() -> Task {{
    Task::new(\"one line about what {name} does\", run)
}}

// `Arguments` has no fields yet, so this one is unused. Drop the underscore
// when you add the first field.
fn run(_arguments: Arguments) -> Outcome {{
    report(\"{name} has nothing to do yet\");
    Ok(())
}}
"
    )
}

/// What is wrong with a task crate's manifest, or `None` when it is what a
/// fresh ritual for `audience` has: a `[package]` of `name`, `version`,
/// `edition` and, for a private ritual, `publish = false`; the mark that makes
/// it a task; and, when `rituals_dependency_is_inherited`, `rituals` taken
/// from the workspace.
pub(crate) fn manifest_problem(
    document: &DocumentMut,
    name: &str,
    audience: Audience,
    rituals_dependency_is_inherited: bool,
) -> Option<String> {
    let mut expected_keys = vec!["edition", "name", "version"];
    if audience == Audience::Private {
        expected_keys.push("publish");
    }
    expected_keys.sort_unstable();
    let mut package_keys = manifest::keys_of(document, &["package"]);
    package_keys.retain(|key| key != "metadata");
    package_keys.sort_unstable();
    if package_keys != expected_keys {
        return Some(format!(
            "the [package] table of a {audience:?} ritual should hold exactly {expected_keys:?}, \
             it holds {package_keys:?}"
        ));
    }
    if manifest::string_at(document, &["package", "name"]) != Some(name) {
        return Some(format!("the package is not named `{name}`"));
    }
    let publish =
        manifest::lookup(document, &["package", "publish"]).and_then(toml_edit::Item::as_bool);
    if publish != (audience == Audience::Private).then_some(false) {
        return Some(format!(
            "`publish` should be false for a private ritual and absent for a public one, it is \
             {publish:?}"
        ));
    }
    if !manifest::declares_itself_a_task(document) {
        return Some("`[package.metadata.ritual] task = true` is missing".to_string());
    }
    let inherited = manifest::lookup(document, &["dependencies", "rituals", "workspace"])
        .and_then(toml_edit::Item::as_bool);
    if rituals_dependency_is_inherited && inherited != Some(true) {
        return Some("`rituals.workspace = true` is missing".to_string());
    }
    None
}

/// Asserts [`manifest_problem`] finds none.
#[track_caller]
pub(crate) fn assert_a_fresh_manifest(
    document: &DocumentMut,
    name: &str,
    audience: Audience,
    rituals_dependency_is_inherited: bool,
) {
    let problem = manifest_problem(document, name, audience, rituals_dependency_is_inherited);
    assert!(
        problem.is_none(),
        "{}; manifest was:\n{document}",
        problem.unwrap_or_default()
    );
}

/// The relative paths of every file under `directory`, sorted.
pub(crate) fn files_in(directory: &Path) -> Outcome<Vec<String>> {
    let mut files = tree::files_under(directory)?
        .iter()
        .map(|path| {
            path.strip_prefix(directory)
                .context("a file under the directory has the directory as its prefix")
                .map(|relative| relative.display().to_string())
        })
        .collect::<Outcome<Vec<_>>>()?;
    files.sort_unstable();
    Ok(files)
}

/// Where a scaffolded task is, and what the project says about it afterwards.
pub(crate) struct Expected<'a> {
    /// The task's directory from the workspace root, such as `.rituals/lint`.
    pub(crate) directory: &'a str,
    /// The task's name, its key and its crate's name.
    pub(crate) name: &'a str,
    pub(crate) audience: Audience,
    /// Every `[workspace] members` entry afterwards, in order.
    pub(crate) members: &'a [&'a str],
    /// Every `[package.metadata.ritual] tasks` entry afterwards, in order.
    pub(crate) tasks: &'a [&'a str],
    /// The `path` of the command line crate's dependency on the task.
    pub(crate) dependency_path: &'a str,
}

/// Asserts the project holds exactly what a `create` of `expected.name`
/// leaves: the two files of the task, its manifest, the workspace member
/// entry, the dependency, the tasks list, and the generated mounts.
#[track_caller]
pub(crate) fn assert_the_task_is_scaffolded(project: &Project, expected: &Expected) -> TestOutcome {
    let directory = project.root().join(expected.directory);
    assert_eq!(
        files_in(&directory)?,
        ["Cargo.toml", "src/lib.rs"],
        "expected exactly the manifest and the library under {}",
        expected.directory
    );
    assert_a_fresh_manifest(
        &manifest::read(&directory.join("Cargo.toml"))?,
        expected.name,
        expected.audience,
        true,
    );
    assert_eq!(
        read_text(&directory.join("src/lib.rs"))?,
        fresh_lib(expected.name)
    );

    let workspace = project.workspace_manifest()?;
    assert_eq!(
        manifest::workspace_members(&workspace),
        Some(expected.members.iter().map(ToString::to_string).collect()),
        "workspace manifest was:\n{workspace}"
    );
    let cli = project.cli_manifest()?;
    assert_eq!(
        manifest::dependency_path(&cli, &["dependencies"], expected.name),
        Some(expected.dependency_path),
        "command line manifest was:\n{cli}"
    );
    assert_eq!(manifest::tasks(&cli)?, expected.tasks);
    let mounts: Vec<(String, String)> = expected
        .tasks
        .iter()
        .map(|task| ((*task).to_string(), (*task).to_string()))
        .collect();
    assert_eq!(
        generated::mounted_entries(&project.generated_file()?),
        mounts,
        "the generated command line must mount every listed task"
    );
    Ok(())
}

/// Builds the project's command line and runs the task it scaffolded, as a
/// person does with `cargo <bin> <name>`, asserting it ran.
#[track_caller]
pub(crate) fn assert_the_task_builds_and_runs(project: &Project, name: &str) -> TestOutcome {
    let ran = project.alias(&[name])?;
    ran.expect_success(&format!("`cargo {} {name}`", project.bin_name()?));
    assert!(
        ran.stdout
            .contains(&format!("{name} has nothing to do yet")),
        "expected the fresh task to report that it has nothing to do; stdout was:\n{}",
        ran.stdout
    );
    Ok(())
}

/// The lines a `create` of `directory` reports in a project whose command
/// line crate is `ritual/` and whose binary is `ritual`.
pub(crate) fn report_in_a_default_project(
    directory: &str,
    name: &str,
    tasks_after: &[&str],
) -> Vec<String> {
    vec![
        format!("created {directory}/Cargo.toml"),
        format!("created {directory}/src/lib.rs"),
        "updated Cargo.toml".to_string(),
        "updated ritual/Cargo.toml".to_string(),
        format!(
            "updated ritual/src/main.rs (tasks: {})",
            tasks_after.join(", ")
        ),
        format!("next: edit {directory}/src/lib.rs, then run cargo ritual {name}"),
    ]
}

/// The lines of `output`'s standard output.
pub(crate) fn stdout_lines(output: &RunOutput) -> Vec<String> {
    output.stdout.lines().map(ToString::to_string).collect()
}

/// Asserts `output` is a refusal ritual wrote, with `bin_name`'s prefix, and
/// returns the one line it wrote.
#[track_caller]
pub(crate) fn the_refusal<'output>(output: &'output RunOutput, bin_name: &str) -> &'output str {
    output.expect_failure("the command");
    assert!(
        !output.stderr.contains("unrecognized subcommand"),
        "expected ritual's own refusal, not clap's; stderr was:\n{}",
        output.stderr
    );
    output.sole_line_prefixed_with(bin_name)
}

/// Asserts `output` is a refusal from the deprecated `add`: exactly two lines
/// on stderr, the deprecation notice and then the refusal with `bin_name`'s
/// prefix, and returns the refusal.
#[track_caller]
pub(crate) fn the_refusal_after_the_notice<'output>(
    output: &'output RunOutput,
    bin_name: &str,
) -> &'output str {
    output.expect_failure("the command");
    let lines: Vec<&str> = output.stderr.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "expected the notice and then the refusal on stderr; stderr was:\n{}",
        output.stderr
    );
    assert_eq!(
        lines[0], ADD_IS_NOW_CREATE,
        "expected the deprecation notice first; stderr was:\n{}",
        output.stderr
    );
    let message = lines[1].strip_prefix(&format!("{bin_name}: "));
    assert!(
        message.is_some(),
        "expected the refusal to be prefixed with `{bin_name}: `; stderr was:\n{}",
        output.stderr
    );
    message.unwrap_or_default()
}

/// Runs `arguments` on the built command line `binary` in `directory`,
/// asserts it was refused with a message for which `says` holds, and that
/// the project's tree is byte-identical to what it was.
#[track_caller]
pub(crate) fn assert_refused_and_left_alone(
    project: &Project,
    binary: &Path,
    directory: &Path,
    arguments: &[&str],
    says: impl FnOnce(&str) -> bool,
) -> TestOutcome {
    let before = snapshot_tree(project.root())?;
    let output = run_binary(binary, directory, arguments)?;
    let bin_name = project.bin_name()?;
    // `add` says it is going away before it does `create`'s work, refusals
    // included, so its refusal is the line after that notice.
    let message = if arguments.first() == Some(&"add") {
        the_refusal_after_the_notice(&output, &bin_name)
    } else {
        the_refusal(&output, &bin_name)
    };
    assert!(
        says(message),
        "the refusal of `{}` did not say what was wanted; it said:\n{message}",
        arguments.join(" ")
    );
    tree::assert_trees_identical(
        &format!("a refused `{}` must write nothing", arguments.join(" ")),
        &before,
        &snapshot_tree(project.root())?,
    );
    Ok(())
}

/// Leaves `Cargo.lock` stale by dropping the package `package` from it,
/// and proves with Cargo itself that reading the project would rewrite it:
/// `cargo metadata` is run, the lockfile is asserted to have changed, and the
/// stale bytes are put back. Without that proof a story about a refusal
/// leaving the lockfile alone could pass on a lockfile nothing would touch.
pub(crate) fn leave_the_lockfile_stale(project: &Project, package: &str) -> TestOutcome {
    let lockfile = project.root().join("Cargo.lock");
    let current = read_text(&lockfile)?;
    let header = format!("[[package]]\nname = \"{package}\"\n");
    let start = current
        .find(&header)
        .context("the built project's Cargo.lock names the package")?;
    let end = current[start..]
        .find("\n\n")
        .map_or(current.len(), |offset| start + offset + 2);
    let stale = format!("{}{}", &current[..start], &current[end..]);
    super::write_text(&lockfile, &stale)?;
    assert_cargo_metadata_rewrites(project, Some(stale.as_bytes()))
}

/// Removes `Cargo.lock`, with the same proof as [`leave_the_lockfile_stale`].
pub(crate) fn leave_no_lockfile(project: &Project) -> TestOutcome {
    std::fs::remove_file(project.root().join("Cargo.lock"))
        .context("removing the built project's Cargo.lock")?;
    assert_cargo_metadata_rewrites(project, None)
}

/// Runs `cargo metadata` in the project, asserts the lockfile is no longer
/// what `before` says it was, then puts `before` back.
fn assert_cargo_metadata_rewrites(project: &Project, before: Option<&[u8]>) -> TestOutcome {
    let lockfile = project.root().join("Cargo.lock");
    cargo_query(project.root(), &["metadata", "--format-version", "1"])?
        .expect_success("`cargo metadata` on the project with an unsettled lockfile");
    let after = tree::lockfile(project.root())?;
    if after.as_deref() == before {
        return failure(
            "fixture precondition: `cargo metadata` left the unsettled lockfile as it was, so a \
             refusal that read the project could not be told from one that did not",
        );
    }
    before.map_or_else(
        || std::fs::remove_file(&lockfile).context("removing the rewritten lockfile"),
        |bytes| std::fs::write(&lockfile, bytes).context("restoring the stale lockfile"),
    )
}
