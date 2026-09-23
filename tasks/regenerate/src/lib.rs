//! The `regenerate` task: rewrite the composed command line's `src/main.rs` from
//! the tasks it imports.

use rituals::{CommandLine, Task, clap};
use rituals_compose::generated_file;

/// `regenerate` takes no arguments; the composed CLI's command line arrives
/// through the invocation instead.
#[derive(clap::Args)]
struct RegenerateArguments {}

/// This task, for a command line to mount under whatever name imports it.
///
/// Rewrites the composed command line's generated `main.rs` from the tasks
/// its manifest names in `[package.metadata.ritual] tasks`, leaving the file
/// untouched when nothing has changed.
//
// No `# Examples` section: a `task()` function has exactly one call-site
// shape — a mount line in a generated file — and an example here could only
// restate `let task = regenerate::task();`, which shows nothing a reader needs.
#[must_use]
pub fn task() -> Task {
    Task::receiving_command_line(
        "rewrite src/main.rs from the imported tasks",
        |command_line: &CommandLine, _arguments: RegenerateArguments| {
            generated_file::regenerate(command_line)
        },
    )
}

#[cfg(test)]
mod tests {
    use rituals::clap::{self, Parser};

    use super::{RegenerateArguments, task};

    /// `RegenerateArguments` flattened into a minimal command, the way a
    /// real task's `#[command(flatten)]` wires it — enough to exercise
    /// clap's parsing of the shape this crate declares, without going
    /// through `Task::declare`, which is crate-internal to `rituals`.
    #[derive(clap::Parser)]
    struct Command {
        #[command(flatten)]
        regenerate: RegenerateArguments,
    }

    /// `Task`'s own `about` text is the only public door onto what
    /// `Task::new`/`Task::receiving_command_line` were given —
    /// `Task::declare` and its rendered `clap::Command` are crate-internal
    /// to `rituals` — so this reads it through `Task`'s `Debug` impl, the
    /// same constraint `rituals-core`'s child-order test documents.
    #[test]
    fn task_names_its_purpose_in_its_about_text() {
        let rendered = format!("{:?}", task());
        assert!(
            rendered.contains(r#"about: "rewrite src/main.rs from the imported tasks""#),
            "expected the about text in Debug output: {rendered}"
        );
    }

    /// `regenerate`'s own command line arrives through the invocation, not
    /// through any flag or positional argument — an empty argument list
    /// must parse.
    #[test]
    fn regenerate_takes_no_arguments() {
        let parsed = Command::try_parse_from(["program"]);
        assert!(
            parsed.is_ok(),
            "expected no arguments to parse, got: {:?}",
            parsed.err()
        );
    }

    /// The negative-space companion to the test above: a stray positional
    /// is not silently accepted.
    #[test]
    fn an_unexpected_argument_is_refused() {
        let parsed = Command::try_parse_from(["program", "extra"]);
        assert!(
            parsed.is_err(),
            "expected an unexpected argument to be refused, but it parsed"
        );
    }
}
