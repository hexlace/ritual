//! Assembling one composed command line from the framework's own tasks and
//! a project's imports, and dispatching a command line to the one that
//! matches.

mod assemble;

use std::process::ExitCode;

use assemble::{assemble, flatten};

use crate::command_line::CommandLine;
use crate::identity::Identity;
use crate::outcome::{Failure, Outcome};
use crate::report::write_to_stderr;
use crate::task::Task;

/// Assembles `imported` into one command line, dispatches the process's own
/// arguments against it, and maps the outcome to a process exit status.
///
/// The command line is named, for `Usage:` and `--version`, for
/// `identity.binary_name()` — the compiled bin name, fixed when the binary
/// was built, never how this particular process happened to be invoked. Task
/// assembly (`assemble`), flattening the bundle mounted under that same name
/// (`flatten`), and the refusal prefix (`refusal_line`) all key on it.
///
/// Every argument error clap raises on its own — an unknown flag, a missing
/// value, an unknown subcommand — exits 2 with clap's own formatting.
/// Everything ritual itself refuses, including a task's own [`crate::Failure`],
/// is written with `<bin name>: ` in front of the message and each of its
/// causes — the one place that prefix is added, so a task author never
/// writes it — and exits 1, or with the [`crate::RefusalStatus`] the
/// failure chose.
///
/// # Examples
///
/// ```ignore
/// // Not a compiled doctest: `identity!` needs a binary target, which a
/// // doctest is not, and `run` reads the process's own arguments through
/// // `clap::Command::get_matches`, so a runnable example would parse the
/// // doctest's own argv rather than anything meaningful. Real coverage
/// // comes from `crates/rituals-cli` compiling and running at all, plus its
/// // integration tests.
/// fn main() -> std::process::ExitCode {
///     rituals::run(rituals::identity!(), [("ritual", ritual::task())])
/// }
/// ```
#[must_use]
pub fn run(
    identity: Identity,
    imported: impl IntoIterator<Item = (&'static str, Task)>,
) -> ExitCode {
    let tasks = assemble(identity.binary_name(), imported);
    let top_level = match flatten(identity.binary_name(), tasks) {
        Ok(top_level) => top_level,
        Err(failure) => {
            // Nothing is parsed when flattening itself is refused: a clap
            // tree built from a refused top level would panic on the
            // collision in a debug build and silently drop a command in a
            // release build, so there is nothing safe to build here — not
            // even to answer `--help`.
            write_to_stderr(&refusal_line(identity.binary_name(), &failure));
            return ExitCode::FAILURE;
        }
    };
    let command = build_command(identity, &top_level.mounts);

    let matches = command.get_matches();
    let resolved = resolve(&top_level.mounts, &matches);

    let command_line =
        CommandLine::from_dispatch(identity, top_level.flattened, resolved.command_path);
    let outcome = resolved.task.invoke(&command_line, resolved.matches);
    if let Err(failure) = &outcome {
        write_to_stderr(&refusal_line(identity.binary_name(), failure));
    }
    exit_code_for(&outcome)
}

/// The leaf task a parse selected, its own matches, and the subcommand
/// names walked to reach it.
struct Resolved<'a> {
    task: &'a Task,
    matches: &'a clap::ArgMatches,
    /// From the top-level name down to the leaf's own, as the tree being
    /// walked spells them: the keys the project and its bundles chose.
    command_path: Vec<&'static str>,
}

/// Walks `matches` down through `tasks` to the leaf task the parse actually
/// selected, that leaf's own matches, and every name it walked on the way —
/// one step for the top-level subcommand, then one more per level of bundle
/// nesting, with a `while let` rather than recursion, so nesting depth never
/// becomes stack depth.
///
/// The loop has no fixed iteration limit because it does not need one: the
/// tree it walks is finite by construction, for the same reason
/// `declare_tree`'s is — a [`Task`]'s children are owned, not shared, so no
/// task can contain itself. Each iteration steps
/// one level down a tree clap has already matched a real parse path
/// through — `matches.subcommand()` can only return `Some` as many times as
/// the parse actually nested — so the walk consumes exactly one level of
/// that already-finite path per iteration.
///
/// Kept separate from [`run`] so it can be exercised against matches built
/// from [`build_command`] directly, without real process arguments.
fn resolve<'a>(tasks: &'a [(&'static str, Task)], matches: &'a clap::ArgMatches) -> Resolved<'a> {
    let Some((subcommand_name, subcommand_matches)) = matches.subcommand() else {
        // `subcommand_required(true)` makes clap exit before returning here
        // when no subcommand was given.
        unreachable!("clap enforces subcommand_required(true) before returning matches");
    };
    let Some((task_name, task)) = tasks.iter().find(|(name, _)| *name == subcommand_name) else {
        // clap can only report a subcommand name this tree declared.
        unreachable!("clap returned an undeclared subcommand name `{subcommand_name}`");
    };
    let mut task = task;
    let mut matches = subcommand_matches;
    let mut command_path = vec![*task_name];

    while let Some(children) = task.children() {
        let Some((child_name, child_matches)) = matches.subcommand() else {
            // `declare_tree` sets `subcommand_required(true)` on every
            // bundle's own command, the same way the top level sets it.
            unreachable!("subcommand_required(true) is set on every bundle (declare_tree)");
        };
        let Some((child_key, child_task)) = children.iter().find(|(name, _)| *name == child_name)
        else {
            // clap can only report a subcommand name this tree declared.
            unreachable!("clap returned an undeclared subcommand name `{child_name}`");
        };
        command_path.push(*child_key);
        task = child_task;
        matches = child_matches;
    }

    Resolved {
        task,
        matches,
        command_path,
    }
}

/// Builds the top-level `clap::Command` for one composed command line.
///
/// `identity.binary_name()` is handed to both [`clap::Command::new`] and
/// [`clap::Command::bin_name`]: without an explicit `bin_name`, clap fills it
/// from `argv[0]`, so `Usage:` would follow a rename even though `--version`
/// does not — the two-source disagreement this framework exists to remove,
/// merely relocated. With both the same fixed name, `--version` and `Usage:`
/// agree by construction and neither reads the process's own arguments.
/// `tasks` are declared as subcommands in the order given.
///
/// Kept separate from [`run`] so it can be exercised without real process
/// arguments.
fn build_command(identity: Identity, tasks: &[(&'static str, Task)]) -> clap::Command {
    let mut command = clap::Command::new(identity.binary_name())
        .bin_name(identity.binary_name())
        .version(identity.version())
        .subcommand_required(true)
        .arg_required_else_help(true);
    for (name, task) in tasks {
        command = command.subcommand(task.declare(name));
    }
    command
}

/// Builds the one line ritual writes to stderr for a refusal —
/// `<binary_name>: <message>: <cause>…`, every cause in `failure`'s chain
/// after its message — the one place this prefix is added, so nothing that
/// produces a message writes it a second time.
fn refusal_line(binary_name: &str, failure: &Failure) -> String {
    format!("{binary_name}: {}", failure.with_causes())
}

/// Maps a task's [`Outcome`] to the process exit status a caller sees:
/// success to [`ExitCode::SUCCESS`], a refusal to its own
/// [`Failure::status`], which is 1 unless the task chose another.
fn exit_code_for(outcome: &Outcome) -> ExitCode {
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => ExitCode::from(failure.status().get()),
    }
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use super::{build_command, exit_code_for, flatten, refusal_line, resolve};
    use crate::command_line::CommandLine;
    use crate::identity::Identity;
    use crate::outcome::{Failure, Outcome, RefusalStatus};
    use crate::task::Task;
    use crate::test_support::{NoArguments, run_ok};

    /// An arbitrary identity for tests that only need a binary name and a
    /// version, not a real composed CLI's own identity.
    fn an_identity(binary_name: &'static str, version: &'static str) -> Identity {
        Identity::from_macro_expansion("a-package", binary_name, version)
    }

    #[test]
    fn exit_code_for_ok_is_success() {
        assert_eq!(exit_code_for(&Ok(())), ExitCode::SUCCESS);
    }

    #[test]
    fn exit_code_for_err_is_failure() {
        let outcome: Outcome = Err(Failure::new("boom"));
        assert_eq!(exit_code_for(&outcome), ExitCode::FAILURE);
    }

    #[test]
    fn exit_code_for_a_failure_with_a_status_is_that_status() {
        let status = RefusalStatus::new(3).expect("3 is a usable refusal status");
        let outcome: Outcome = Err(Failure::new("3 tasks differ").exiting_with(status));
        assert_eq!(exit_code_for(&outcome), ExitCode::from(3));
    }

    #[test]
    fn refusal_line_carries_the_bin_prefix_exactly_once() {
        let failure = Failure::new("tasks/lint already exists");
        let line = refusal_line("myapp-ritual", &failure);
        assert_eq!(line, "myapp-ritual: tasks/lint already exists");
        assert_eq!(line.matches("myapp-ritual:").count(), 1);
    }

    /// An error that names only its own situation and hands whatever caused
    /// it on through `source()`, the usual convention for an error's
    /// `Display`.
    #[derive(Debug)]
    struct Link {
        message: &'static str,
        source: Option<Box<Self>>,
    }

    impl Link {
        fn new(message: &'static str) -> Self {
            Self {
                message,
                source: None,
            }
        }

        fn caused_by(mut self, cause: Self) -> Self {
            self.source = Some(Box::new(cause));
            self
        }
    }

    impl std::fmt::Display for Link {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(self.message)
        }
    }

    impl std::error::Error for Link {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.source
                .as_deref()
                .map(|link| link as &(dyn std::error::Error + 'static))
        }
    }

    #[test]
    fn refusal_line_with_one_cause_is_the_message_then_the_cause() {
        let failure = Failure::new("writing .rituals/lint/Cargo.toml failed")
            .caused_by(Link::new("disk full"));
        assert_eq!(
            refusal_line("myapp-ritual", &failure),
            "myapp-ritual: writing .rituals/lint/Cargo.toml failed: disk full"
        );
    }

    #[test]
    fn refusal_line_names_every_cause_in_the_chain_once_in_order() {
        let failure = Failure::new("syncing the declared set failed").caused_by(
            Link::new("the registry did not answer").caused_by(Link::new("connection refused")),
        );
        assert_eq!(
            refusal_line("myapp-ritual", &failure),
            "myapp-ritual: syncing the declared set failed: the registry did not answer: \
             connection refused"
        );
    }

    #[test]
    fn refusal_line_walks_through_a_failure_that_is_itself_a_cause() {
        let failure = Failure::new("importing lint failed").caused_by(
            Failure::new("regenerating the command line failed").caused_by(Link::new("disk full")),
        );
        assert_eq!(
            refusal_line("myapp-ritual", &failure),
            "myapp-ritual: importing lint failed: regenerating the command line failed: disk full"
        );
    }

    /// The global tool's binary is named `ritual` while its package is
    /// `rituals-cli`, so `--version` must report the former and never the
    /// latter — built through [`build_command`], with no real argv, exactly
    /// so this does not depend on how the test binary itself was invoked.
    #[test]
    fn version_names_the_compiled_bin_not_the_package() {
        let identity = an_identity("ritual", "0.0.0");
        let command = build_command(identity, &[]);
        let rendered = command.render_version();
        assert!(
            rendered.starts_with("ritual "),
            "expected --version to start with the bin name: {rendered:?}"
        );
        assert!(
            !rendered.contains("a-package"),
            "expected the package name to be absent from --version: {rendered:?}"
        );
    }

    /// A project can name its composed CLI's bin the same as its package —
    /// package and bin share one name here, and `--version` must still
    /// report exactly that name.
    #[test]
    fn version_names_the_bin_when_bin_and_package_share_a_name() {
        let identity = Identity::from_macro_expansion("demo-ritual", "demo-ritual", "0.1.0");
        let command = build_command(identity, &[]);
        let rendered = command.render_version();
        assert!(
            rendered.starts_with("demo-ritual "),
            "expected --version to start with the shared name: {rendered:?}"
        );
    }

    /// `Usage:` must name the compiled bin — `identity.binary_name()` — never
    /// `argv[0]`, through a parse error rendered from arguments whose own
    /// `argv[0]` names something else entirely. This is the unit-level half;
    /// `identity_survives_rename_and_symlink.rs` is the story-level half,
    /// actually invoking a renamed copy of the built binary.
    #[test]
    fn usage_line_names_the_compiled_bin_not_argv0() {
        let identity = an_identity("ritual", "0.0.0");
        let command = build_command(identity, &[]);
        let parsed = command.try_get_matches_from(["/some/where/renamed-probe", "nosuch"]);
        assert!(parsed.is_err(), "an unknown subcommand must be refused");
        if let Err(error) = parsed {
            let rendered = error.render().to_string();
            assert!(
                rendered.contains("Usage: ritual"),
                "expected the Usage: line to name the compiled bin: {rendered:?}"
            );
            assert!(
                !rendered.contains("renamed-probe"),
                "expected the Usage: line to never name argv[0]: {rendered:?}"
            );
        }
    }

    /// `resolve` walks down two levels of bundle nesting to reach the right
    /// leaf — not merely *a* leaf, but the specific one its argv path names.
    /// The resolved leaf's own handler always returns `Err` with a
    /// distinguishing message, so invoking it proves which task `resolve`
    /// actually returned, rather than only that dispatch did not panic.
    #[test]
    fn the_descend_loop_reaches_a_leaf_two_levels_down() {
        fn run_and_report_which_leaf_ran(_arguments: NoArguments) -> Outcome {
            Err(Failure::new("the leaf two levels down ran"))
        }

        let leaf = Task::new("a leaf, two levels down", run_and_report_which_leaf_ran);
        let inner = Task::group("an inner bundle", [("leaf", leaf)]);
        let sibling = Task::new("an unrelated top-level task", run_ok);
        let tasks = [("outer", inner), ("sibling", sibling)];

        let identity = an_identity("demo", "0.0.0");
        let command = build_command(identity, &tasks);
        let parsed = command.try_get_matches_from(["demo", "outer", "leaf"]);
        assert!(
            parsed.is_ok(),
            "parsing a two-level path should succeed: {parsed:?}"
        );

        if let Ok(matches) = parsed {
            let resolved = resolve(&tasks, &matches);
            let command_line = CommandLine::from_dispatch(identity, [], resolved.command_path);
            let outcome = resolved.task.invoke(&command_line, resolved.matches);
            let Err(failure) = outcome else {
                unreachable!("the resolved leaf's own handler always returns Err");
            };
            assert_eq!(failure.to_string(), "the leaf two levels down ran");
        }
    }

    /// Parses `argv` against `tasks` and returns the command path `resolve`
    /// walked, the way [`super::run`] builds and parses its own tree.
    fn command_path_for(tasks: &[(&'static str, Task)], argv: &[&str]) -> Vec<&'static str> {
        let command = build_command(an_identity("demo", "0.0.0"), tasks);
        let parsed = command.try_get_matches_from(argv);
        assert!(
            parsed.is_ok(),
            "parsing {argv:?} should succeed: {parsed:?}"
        );
        parsed.map_or_else(
            |_| Vec::new(),
            |matches| resolve(tasks, &matches).command_path,
        )
    }

    /// A bundle offering one child, `sync`, for a project to mount under
    /// whatever key it chooses.
    fn a_tools_bundle() -> Task {
        Task::group("a bundle of tools", [("sync", Task::new("sync", run_ok))])
    }

    /// The same bundle mounted under two keys reports each key in its own
    /// path: the top-level name is the project's choice of key, not anything
    /// the bundle carries.
    #[test]
    fn a_bundle_mounted_under_a_renamed_key_sees_that_key_in_its_path() {
        let tasks = [("acme", a_tools_bundle()), ("tools", a_tools_bundle())];

        assert_eq!(
            command_path_for(&tasks, &["demo", "tools", "sync"]),
            ["tools", "sync"]
        );
        assert_eq!(
            command_path_for(&tasks, &["demo", "acme", "sync"]),
            ["acme", "sync"]
        );
    }

    /// A task inside a bundle inside a bundle sees every level it was
    /// reached through, top first.
    #[test]
    fn a_task_two_bundles_deep_sees_every_level_in_its_path() {
        let inner = Task::group("an inner bundle", [("leaf", Task::new("leaf", run_ok))]);
        let outer = Task::group("an outer bundle", [("inner", inner)]);
        let tasks = [("outer", outer), ("sibling", Task::new("sibling", run_ok))];

        assert_eq!(
            command_path_for(&tasks, &["demo", "outer", "inner", "leaf"]),
            ["outer", "inner", "leaf"]
        );
        assert_eq!(command_path_for(&tasks, &["demo", "sibling"]), ["sibling"]);
    }

    /// A child of the bundle mounted under the bin's own name is a top-level
    /// command once flattened, so its path is its own name alone: the
    /// bundle's key is not a word anybody types.
    #[test]
    fn a_flattened_child_s_path_is_its_own_name() {
        let top_level = flatten("demo", vec![("demo", a_tools_bundle())]);
        assert!(
            top_level.is_ok(),
            "flattening should succeed: {top_level:?}"
        );
        if let Ok(top_level) = top_level {
            assert_eq!(
                command_path_for(&top_level.mounts, &["demo", "sync"]),
                ["sync"]
            );
        }
    }
}
