//! Assembling one composed command line from the framework's own tasks and
//! a project's imports, and dispatching a command line to the one that
//! matches.

mod assemble;

use std::fmt;
use std::process::ExitCode;

use assemble::{assemble, flatten};

use crate::command_line::CommandLine;
use crate::identity::Identity;
use crate::outcome::Outcome;
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
/// exits 1 with `<bin name>: ` in front of the message — the one place that
/// prefix is added, so a task author never writes it.
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
            report_to_stderr(refusal_line(identity.binary_name(), &failure));
            return ExitCode::FAILURE;
        }
    };
    let command = build_command(identity, &top_level.mounts);

    let matches = command.get_matches();
    let (task, matches) = resolve(&top_level.mounts, &matches);

    let command_line = CommandLine::from_dispatch(identity, top_level.flattened);
    let outcome = task.invoke(&command_line, matches);
    if let Err(failure) = &outcome {
        report_to_stderr(refusal_line(identity.binary_name(), failure));
    }
    exit_code_for(&outcome)
}

/// Walks `matches` down through `tasks` to the leaf task the parse actually
/// selected, and that leaf's own matches — one step for the top-level
/// subcommand, then one more per level of bundle nesting, with a `while let`
/// rather than recursion, so nesting depth never becomes stack depth.
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
fn resolve<'a>(
    tasks: &'a [(&'static str, Task)],
    matches: &'a clap::ArgMatches,
) -> (&'a Task, &'a clap::ArgMatches) {
    let Some((subcommand_name, subcommand_matches)) = matches.subcommand() else {
        // `subcommand_required(true)` makes clap exit before returning here
        // when no subcommand was given.
        unreachable!("clap enforces subcommand_required(true) before returning matches");
    };
    let Some((_, task)) = tasks.iter().find(|(name, _)| *name == subcommand_name) else {
        // clap can only report a subcommand name this tree declared.
        unreachable!("clap returned an undeclared subcommand name `{subcommand_name}`");
    };
    let mut task = task;
    let mut matches = subcommand_matches;

    while let Some(children) = task.children() {
        let Some((child_name, child_matches)) = matches.subcommand() else {
            // `declare_tree` sets `subcommand_required(true)` on every
            // bundle's own command, the same way the top level sets it.
            unreachable!("subcommand_required(true) is set on every bundle (declare_tree)");
        };
        let Some((_, child_task)) = children.iter().find(|(name, _)| *name == child_name) else {
            // clap can only report a subcommand name this tree declared.
            unreachable!("clap returned an undeclared subcommand name `{child_name}`");
        };
        task = child_task;
        matches = child_matches;
    }

    (task, matches)
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
/// `<binary_name>: <message>` — the one place this prefix is added, so
/// nothing that produces a message writes it a second time.
fn refusal_line(binary_name: &str, message: impl fmt::Display) -> String {
    format!("{binary_name}: {message}")
}

/// Maps a task's [`Outcome`] to the process exit status a caller sees:
/// success to [`ExitCode::SUCCESS`], any refusal to [`ExitCode::FAILURE`].
const fn exit_code_for(outcome: &Outcome) -> ExitCode {
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

/// Writes one line to stderr — this framework's one refusal-reporting site,
/// mirroring [`crate::report()`]'s one stdout-reporting site.
///
/// Uses a locked `writeln!` rather than `eprintln!` for the same reason
/// `report` avoids `println!`: a failed write is discarded rather than
/// panicking.
fn report_to_stderr(message: impl AsRef<str>) {
    use std::io::Write;

    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{}", message.as_ref());
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use super::{build_command, exit_code_for, refusal_line, resolve};
    use crate::command_line::CommandLine;
    use crate::identity::Identity;
    use crate::outcome::{Failure, Outcome};
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
    fn refusal_line_carries_the_bin_prefix_exactly_once() {
        let failure = Failure::new("tasks/lint already exists");
        let line = refusal_line("myapp-ritual", &failure);
        assert_eq!(line, "myapp-ritual: tasks/lint already exists");
        assert_eq!(line.matches("myapp-ritual:").count(), 1);
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
            let (task, task_matches) = resolve(&tasks, &matches);
            let command_line = CommandLine::from_dispatch(identity, []);
            let outcome = task.invoke(&command_line, task_matches);
            let Err(failure) = outcome else {
                unreachable!("the resolved leaf's own handler always returns Err");
            };
            assert_eq!(failure.to_string(), "the leaf two levels down ran");
        }
    }
}
