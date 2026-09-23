//! The composed command line a task is running inside.

use crate::identity::Identity;

/// The composed command line a task is running inside: the identity of the
/// binary it was built as, and the commands its top level gets from the
/// bundle mounted under that same name.
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
///     Ok(())
/// }
/// # let identity = rituals::Identity::from_macro_expansion("acme-cli", "acme", "0.1.0");
/// # let command_line = CommandLine::from_dispatch(identity, ["add", "regenerate"]);
/// # assert_eq!(command_line.identity().binary_name(), "acme");
/// # assert_eq!(command_line.flattened_commands(), ["add", "regenerate"]);
/// # run(&command_line)?;
/// # Ok::<(), rituals::Failure>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    identity: Identity,
    flattened_commands: Vec<&'static str>,
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

    /// Builds a `CommandLine` from what the dispatcher assembled.
    ///
    /// Hidden rather than private because a caller outside this crate needs
    /// one too: a task crate's own unit tests, which stand in for the
    /// dispatcher when they hand a handler a `CommandLine`. The name says
    /// where a real one comes from, so any other call site reads as what it
    /// is — a stand-in for dispatch, not a command line that is running.
    #[doc(hidden)]
    #[must_use]
    pub fn from_dispatch(
        identity: Identity,
        flattened_commands: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        Self {
            identity,
            flattened_commands: flattened_commands.into_iter().collect(),
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

    #[test]
    fn identity_and_flattened_commands_read_back_what_from_dispatch_was_given() {
        let identity = Identity::from_macro_expansion("a-package", "a-binary", "1.2.3");
        let command_line = CommandLine::from_dispatch(identity, ["add", "build"]);

        assert_eq!(command_line.identity(), identity);
        assert_eq!(command_line.flattened_commands(), ["add", "build"]);
    }

    #[test]
    fn flattened_commands_is_empty_when_nothing_was_given() {
        let identity = Identity::from_macro_expansion("a-package", "a-binary", "1.2.3");
        let command_line = CommandLine::from_dispatch(identity, []);

        assert!(command_line.flattened_commands().is_empty());
    }
}
