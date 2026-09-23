//! The one check `add` and `regenerate` run before they write: whether a
//! command name a project is about to mount is still free at the top level
//! of the command line currently running.

use rituals::{CommandLine, Failure, Outcome};

/// Refuses when `command` is already a top-level command of `command_line`.
///
/// Either supplied by the bundle mounted under the name the binary was
/// built as, or `help`, which clap adds to every command that has
/// subcommands.
///
/// The command name comes from what a person typed (`add`) or from the
/// project's own manifest (`regenerate`); the flattened commands come from the
/// binary that person is running — both are this project's own, not supplied by
/// anyone else. Only exact name equality is checked: unusable spellings are
/// already refused before this ever runs — by [`rituals::Name::new`] for the
/// name `add` was given, and by the task-list resolver, along with duplicates,
/// for the names `regenerate` reads.
///
/// A `command` equal to the bin's own name always passes, checked before
/// either comparison below: that key is the one a bin-name-mounted bundle's
/// own manifest entry has, and it is also the key that bundle legitimately
/// promotes a child under when the child happens to share it — the startup
/// rule does not flatten a child a second time just because its own key
/// equals the bundle's. Comparing it against the flattened list anyway
/// would refuse the bundle's entry against the very child it promoted,
/// which is a collision between a thing and itself, not a real one — and
/// unlike a real collision, nothing about the project's manifest fixes it,
/// so `regenerate` would refuse forever. Whether the crate behind that key
/// is actually a bundle is not visible until it is compiled, so that
/// question is not this function's to catch at all; it surfaces at the
/// command line's next startup instead.
///
/// `add`'s own `ensure_the_name_is_not_the_bin_name` refuses that same name
/// earlier and on `add`'s own behalf, before this check ever runs — so the
/// exemption above is exactly what it says, a rule about `regenerate`'s
/// manifest entry and a bundle's promoted child, and not a statement that
/// `add` will scaffold a task there.
///
/// # Examples
///
/// ```
/// use rituals::{CommandLine, Outcome, Task, clap};
/// use rituals_compose::top_level;
///
/// #[derive(clap::Args)]
/// struct AddArguments {
///     name: String,
/// }
///
/// fn run(command_line: &CommandLine, arguments: AddArguments) -> Outcome {
///     top_level::ensure_command_is_free(command_line, &arguments.name)?;
///     // ... the rest of a writing task's own work goes here.
///     Ok(())
/// }
///
/// // Never invoked here: this shows where the check goes in a writing
/// // task, not what running one does.
/// let _task = Task::receiving_command_line("scaffold a task", run);
/// ```
///
/// # Errors
///
/// Returns a [`Failure`] naming `command`, where the running command line
/// already gets it from, and what to change.
pub fn ensure_command_is_free(command_line: &CommandLine, command: &str) -> Outcome {
    if command == command_line.identity().binary_name() {
        return Ok(());
    }

    if command == "help" {
        return Err(help_collision_refusal(command_line));
    }

    let already_flattened = command_line.flattened_commands().contains(&command);
    if already_flattened {
        return Err(flattened_collision_refusal(command_line, command));
    }

    Ok(())
}

/// The refusal for a name already supplied by the bundle mounted under the
/// bin's own name.
fn flattened_collision_refusal(command_line: &CommandLine, command: &str) -> Failure {
    let bin_name = command_line.identity().binary_name();
    Failure::new(format!(
        "`{command}` would be a top-level command of `{bin_name}` twice: the bundle mounted \
         under that name, which this command line was built as, already provides it. Give \
         this task another name, or mount that bundle under a different key in \
         [package.metadata.ritual] tasks."
    ))
}

/// The refusal for `help`, which needs no list at all: clap already
/// occupies it on every command line that has any subcommand.
fn help_collision_refusal(command_line: &CommandLine) -> Failure {
    let bin_name = command_line.identity().binary_name();
    Failure::new(format!(
        "`help` would be a top-level command of `{bin_name}` twice: clap gives every command \
         that has subcommands a `help` of its own. Give this task another name."
    ))
}

/// The dependency key every project `new` scaffolds imports ritual's own
/// bundle under.
///
/// The bundle flattens into the top level when this key equals the bin
/// name, which is the default, and stays reached through this key when the
/// project names its command line something else.
pub const MANAGEMENT_BUNDLE_KEY: &str = "ritual";

/// Returns what a person types to run `command` from ritual's own bundle.
///
/// That is `cargo ritual regenerate` in a default project, and
/// `cargo acme ritual regenerate` in one made with `--cli acme`.
///
/// Every line ritual prints that names one of its own commands as a next
/// step spells it with this, so the hint is one a person can copy.
///
/// # Examples
///
/// ```
/// use rituals::{CommandLine, Identity};
/// use rituals_compose::top_level;
///
/// let default = CommandLine::from_dispatch(
///     Identity::from_macro_expansion("demo-ritual", "ritual", "0.1.0"),
///     ["add", "regenerate", "new", "create"],
/// );
/// assert_eq!(top_level::management_command(&default, "regenerate"), "cargo ritual regenerate");
///
/// let named = CommandLine::from_dispatch(
///     Identity::from_macro_expansion("demo-ritual", "acme", "0.1.0"),
///     [],
/// );
/// assert_eq!(top_level::management_command(&named, "regenerate"), "cargo acme ritual regenerate");
/// ```
#[must_use]
pub fn management_command(command_line: &CommandLine, command: &str) -> String {
    let bin_name = command_line.identity().binary_name();
    // Ritual's bundle is flattened exactly when the bin is named after the
    // key it is mounted under. The running binary cannot see which key a
    // project mounted it under, so this names the one `new` writes; a
    // project that remounted the bundle by hand chose that key itself.
    // The flattened command list cannot decide this: it names commands, not
    // the bundle they came from, so another flattened bundle's own
    // `regenerate` would read as ritual's.
    if bin_name == MANAGEMENT_BUNDLE_KEY {
        return format!("cargo {bin_name} {command}");
    }
    format!("cargo {bin_name} {MANAGEMENT_BUNDLE_KEY} {command}")
}

#[cfg(test)]
mod tests {
    use rituals::Identity;

    use super::{ensure_command_is_free, management_command};

    /// A command line called `acme` that flattens a bundle of its own, with
    /// a child named `regenerate`, while ritual's bundle stays nested under
    /// `ritual`: the hint must reach ritual's `regenerate`, not acme's.
    #[test]
    fn a_hint_reaches_ritual_when_another_bundle_flattens_a_command_of_the_same_name() {
        let command_line = a_command_line(&["regenerate", "deploy"]);
        assert_eq!(
            management_command(&command_line, "regenerate"),
            "cargo acme ritual regenerate"
        );
    }

    #[test]
    fn a_hint_is_flat_when_the_command_line_is_named_after_rituals_bundle() {
        let command_line = rituals::CommandLine::from_dispatch(
            Identity::from_macro_expansion("demo-ritual", "ritual", "0.1.0"),
            ["add", "regenerate", "new", "create"],
        );
        assert_eq!(
            management_command(&command_line, "regenerate"),
            "cargo ritual regenerate"
        );
    }

    fn a_command_line(flattened_commands: &[&'static str]) -> rituals::CommandLine {
        rituals::CommandLine::from_dispatch(
            Identity::from_macro_expansion("acme-ritual", "acme", "0.1.0"),
            flattened_commands.iter().copied(),
        )
    }

    #[test]
    fn a_command_in_the_flattened_list_is_refused_naming_the_command_and_the_bin() {
        let command_line = a_command_line(&["add", "build"]);
        let result = ensure_command_is_free(&command_line, "add");
        assert!(result.is_err(), "expected the collision to be refused");
        if let Err(failure) = result {
            let message = failure.to_string();
            assert!(message.contains("`add`"), "message was: {message}");
            assert!(message.contains("`acme`"), "message was: {message}");
        }
    }

    #[test]
    fn a_command_not_in_the_flattened_list_passes() {
        let command_line = a_command_line(&["add", "build"]);
        let result = ensure_command_is_free(&command_line, "lint");
        assert!(result.is_ok(), "expected no refusal: {result:?}");
    }

    #[test]
    fn an_empty_flattened_list_passes_everything_except_help() {
        let command_line = a_command_line(&[]);
        assert!(ensure_command_is_free(&command_line, "add").is_ok());
        assert!(ensure_command_is_free(&command_line, "anything").is_ok());
    }

    #[test]
    fn help_is_refused_against_an_empty_list_naming_clap() {
        let command_line = a_command_line(&[]);
        let result = ensure_command_is_free(&command_line, "help");
        assert!(result.is_err(), "expected `help` to be refused");
        if let Err(failure) = result {
            let message = failure.to_string();
            assert!(message.contains("`help`"), "message was: {message}");
            assert!(message.contains("clap"), "message was: {message}");
        }
    }

    /// The exact shape a bin-name-mounted bundle produces when one of its
    /// own children is keyed the same as the bundle: the bin's own name
    /// (`acme`, per [`a_command_line`]'s identity) sits in the flattened
    /// list because that child was promoted — and the manifest entry for
    /// the bundle itself, whose key is also `acme`, must still pass rather
    /// than being refused against the child it legitimately promoted.
    #[test]
    fn the_bin_name_key_passes_even_when_the_flattened_list_contains_it() {
        let command_line = a_command_line(&["acme"]);
        let result = ensure_command_is_free(&command_line, "acme");
        assert!(
            result.is_ok(),
            "expected the bin's own name to pass: {result:?}"
        );
    }

    /// The bin-name exemption does not widen into a blanket pass: a
    /// genuinely different flattened name, and `help`, are still refused
    /// even when the bin's own name sits in the same list.
    #[test]
    fn a_flattened_name_and_help_are_still_refused_when_the_bin_name_is_also_in_the_list() {
        let command_line = a_command_line(&["acme", "add"]);
        assert!(ensure_command_is_free(&command_line, "acme").is_ok());
        assert!(ensure_command_is_free(&command_line, "add").is_err());
        assert!(ensure_command_is_free(&command_line, "help").is_err());
    }
}
