//! The composed command line a task is running inside.

use crate::identity::Identity;

/// The composed command line a task is running inside, and where in it the
/// task was reached.
///
/// It carries the identity of the binary the command line was built as, the
/// commands its top level gets from the bundle mounted under that same name,
/// and the path of subcommand names that reached the running task.
///
/// A task reaches one of these by opting in with
/// [`crate::Task::receiving_command_line`] in place of [`crate::Task::new`].
///
/// # Examples
///
/// ```
/// use rituals::{CommandLine, Outcome, report};
///
/// fn run(command_line: &CommandLine) -> Outcome {
///     let identity = command_line.identity();
///     report(format!("{} {}", identity.binary_name(), identity.version()));
///     for command in command_line.flattened_commands() {
///         report(format!("  {command}"));
///     }
///     report(format!("running as {}", command_line.cargo_command()));
///     Ok(())
/// }
/// # let identity = rituals::Identity::from_macro_expansion("acme-cli", "acme", "0.1.0");
/// # let command_line = CommandLine::from_dispatch(identity, ["add", "regenerate"], ["tools", "sync"]);
/// # assert_eq!(command_line.identity().binary_name(), "acme");
/// # assert_eq!(command_line.flattened_commands(), ["add", "regenerate"]);
/// # assert_eq!(command_line.command_path(), ["tools", "sync"]);
/// # assert_eq!(command_line.cargo_command(), "cargo acme tools sync");
/// # run(&command_line)?;
/// # Ok::<(), rituals::Failure>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    identity: Identity,
    flattened_commands: Vec<&'static str>,
    command_path: Vec<&'static str>,
}

impl CommandLine {
    /// The identity of the command line this task is mounted in.
    #[must_use]
    pub const fn identity(&self) -> Identity {
        self.identity
    }

    /// The commands this command line's top level gets from the bundle
    /// mounted under the name the binary was built as — the ones a task
    /// cannot find by reading a project's manifest, because nothing in the
    /// manifest names them. Empty when nothing is mounted under that name.
    #[must_use]
    pub fn flattened_commands(&self) -> &[&'static str] {
        &self.flattened_commands
    }

    /// The subcommand names dispatch walked to reach the running task, from
    /// the top level down: `["tools", "sync"]` for a bundle mounted under
    /// `tools` whose child `sync` is running.
    ///
    /// The first name is the top-level command, which the project chose
    /// rather than the bundle: the key a bundle or a task is mounted under,
    /// as the project's dependency line spells it. The one exception is a
    /// child of the bundle mounted under the bin's own name, which sits at
    /// the top level under its own name, so its path is that name alone.
    /// These are the words a person typed after the command line's own name,
    /// and they are true however the binary was reached.
    #[must_use]
    pub fn command_path(&self) -> &[&'static str] {
        &self.command_path
    }

    /// The command a person types to run this task inside a project made by
    /// `ritual new`: `cargo <bin> <command path…>`, such as
    /// `cargo ritual tools sync`, or `cargo acme tools sync` in a project
    /// made with `--cli acme`.
    ///
    /// It holds there because `new` names the bin and the project's cargo
    /// alias together, `--cli` included, so the alias is the bin's name. It
    /// does not hold where nothing made that alias: the globally installed
    /// binary, a binary run directly, or an alias renamed by hand. Where it
    /// cannot be relied on, [`Self::command_path`] still can.
    ///
    /// A task that tells a person to run it again spells the command with
    /// this, so the remedy can be copied as it is printed.
    #[must_use]
    pub fn cargo_command(&self) -> String {
        let mut command = format!("cargo {}", self.identity.binary_name());
        for name in &self.command_path {
            command.push(' ');
            command.push_str(name);
        }
        command
    }

    /// Builds a `CommandLine` from what the dispatcher assembled.
    ///
    /// Hidden rather than private because a caller outside this crate needs
    /// one too: a task crate's own unit tests, which stand in for the
    /// dispatcher when they hand a handler a `CommandLine`. The name says
    /// where a real one comes from, so any other call site reads as what it
    /// is — a stand-in for dispatch, not a command line that is running.
    ///
    /// # Panics
    ///
    /// When `command_path` is empty. Dispatch always walks at least the
    /// top-level name before it runs a task, so an empty path is a stand-in
    /// that does not describe any command line that can run.
    #[doc(hidden)]
    #[must_use]
    pub fn from_dispatch(
        identity: Identity,
        flattened_commands: impl IntoIterator<Item = &'static str>,
        command_path: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        let command_path: Vec<&'static str> = command_path.into_iter().collect();
        assert!(
            !command_path.is_empty(),
            "a task always runs under at least one subcommand name"
        );
        Self {
            identity,
            flattened_commands: flattened_commands.into_iter().collect(),
            command_path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CommandLine;
    use crate::identity::Identity;

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[test]
    fn command_line_is_send_and_sync() {
        assert_send::<CommandLine>();
        assert_sync::<CommandLine>();
    }

    fn an_identity(binary_name: &'static str) -> Identity {
        Identity::from_macro_expansion("a-package", binary_name, "1.2.3")
    }

    /// Two lists of names of the same type, side by side in one
    /// constructor: distinct values read back through their own accessors
    /// is what catches the two being transposed.
    #[test]
    fn each_field_reads_back_what_from_dispatch_was_given() {
        let identity = an_identity("a-binary");
        let command_line =
            CommandLine::from_dispatch(identity, ["add", "build"], ["tools", "sync"]);

        assert_eq!(command_line.identity(), identity);
        assert_eq!(command_line.flattened_commands(), ["add", "build"]);
        assert_eq!(command_line.command_path(), ["tools", "sync"]);
    }

    #[test]
    fn flattened_commands_is_empty_when_nothing_was_given() {
        let command_line = CommandLine::from_dispatch(an_identity("a-binary"), [], ["sync"]);

        assert!(command_line.flattened_commands().is_empty());
    }

    #[test]
    fn cargo_command_names_the_bin_then_every_level_of_the_path() {
        let command_line =
            CommandLine::from_dispatch(an_identity("acme"), [], ["tools", "db", "sync"]);

        assert_eq!(command_line.cargo_command(), "cargo acme tools db sync");
    }

    /// The default project's bin is `ritual`, and a flattened child's path
    /// is its own name, so the command is the one the default alias runs.
    #[test]
    fn cargo_command_for_a_flattened_child_is_the_bin_then_its_name() {
        let command_line = CommandLine::from_dispatch(an_identity("ritual"), ["sync"], ["sync"]);

        assert_eq!(command_line.cargo_command(), "cargo ritual sync");
    }

    #[test]
    #[should_panic(expected = "a task always runs under at least one subcommand name")]
    fn from_dispatch_refuses_an_empty_command_path() {
        let _ = CommandLine::from_dispatch(an_identity("a-binary"), [], []);
    }
}
