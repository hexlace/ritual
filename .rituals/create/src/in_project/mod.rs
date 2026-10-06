//! `create` inside a project: scaffold a task crate in `.rituals/`, add it to
//! the workspace and to the command line crate, and regenerate.
//
// `redundant_pub_crate` (clippy nursery) wants `pub` here because this
// module is private, but `pub(crate)` is the visibility that is actually
// true — it stays correct if this module is ever re-exported at a different
// level — so the nursery lint gives way.
#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) reflects this module's actual visibility even though the enclosing \
              module is private; see the note above"
)]

mod prepare;
mod refusals;
mod scaffolding;

use std::path::Path;

use prepare::{Request, Requested};
use rituals::{CommandLine, Failure, Name, Outcome, report};
use rituals_compose::generated_file::TaskKey;
use rituals_compose::metadata;
use rituals_compose::rollback::{self, Wording};

use crate::arguments::{NameOrPath, ScaffoldArguments};

/// Scaffolds a task into the project the running command line belongs to.
///
/// What `create` does inside a project, and so what the deprecated `add`
/// still does: the crate goes in `.rituals/`, is added to the workspace's
/// members and to the command line crate's dependencies and task list, and
/// the generated file is regenerated. All of it is one run, so a refusal or a
/// failure leaves the project as it was found, `Cargo.lock` included. Refuses
/// with the command to run in the right project when run anywhere else.
///
/// # Errors
///
/// Returns a [`Failure`] saying what to do instead when Cargo finds no
/// project, or one that is not the running command line's own, and one
/// naming the problem when the name or path is not valid, is already taken,
/// or does not lead below `.rituals/`.
///
/// # Examples
///
/// The deprecated `add` is `create`'s in-project path under another name:
///
/// ```
/// use rituals::{CommandLine, Task};
/// use rituals_core_create::{ScaffoldArguments, scaffold_in_project};
///
/// let task = Task::receiving_command_line(
///     "the old name for create, inside a project",
///     |command_line: &CommandLine, arguments: ScaffoldArguments| {
///         scaffold_in_project(command_line, &arguments)
///     },
/// );
/// # let _ = task;
/// ```
pub fn scaffold_in_project(command_line: &CommandLine, arguments: &ScaffoldArguments) -> Outcome {
    let current_dir = std::env::current_dir()
        .map_err(|error| Failure::new("reading the current directory failed").caused_by(error))?;
    metadata::ensure_inside_a_project(&current_dir, "create", &arguments.to_run_again())?;
    scaffold(command_line, arguments, &current_dir)
}

/// [`scaffold_in_project`] for a caller that has already asked Cargo whether
/// `current_dir` is in a project, so `create` asks it once.
pub(crate) fn scaffold(
    command_line: &CommandLine,
    arguments: &ScaffoldArguments,
    current_dir: &Path,
) -> Outcome {
    // A bare name comes from what was typed and needs no subprocess, so it
    // is decided before the run begins, when nothing has been recorded. A
    // path is decided inside it, by placing it.
    let requested = match arguments.name_or_path() {
        NameOrPath::Name(text) => Requested::Name(TaskKey::new(Name::new(text)?)?),
        NameOrPath::Path(path) => Requested::Path(path),
    };
    let run_again = arguments.to_run_again();

    let (lines, next_step) = rollback::attempt(Wording::project(&retry(&run_again)), |changes| {
        let mut scaffolding = prepare::prepare(
            changes,
            Request {
                command_line,
                current_dir,
                requested,
                audience: arguments.audience(),
                run_again: &run_again,
            },
        )?;
        let lines = scaffolding.write(changes, command_line)?;
        let next_step = scaffolding.next_step(command_line.identity().binary_name());
        Ok((lines, next_step))
    })?;

    for line in lines {
        report(line);
    }
    report(next_step);
    Ok(())
}

/// What a person runs again once they have checked whatever a failed run
/// could not put back.
fn retry(run_again: &str) -> String {
    format!("running `create {run_again}` again")
}

#[cfg(test)]
mod tests {
    use super::retry;

    /// The retry wording is the end of `create`'s failure message when the
    /// undo could not put everything back, so it names the command a person
    /// types again, with the argument and flags they gave.
    #[test]
    fn the_retry_names_create_and_the_words_it_was_given() {
        assert_eq!(retry("lint"), "running `create lint` again");
        assert_eq!(
            retry(".rituals/private/lint --public"),
            "running `create .rituals/private/lint --public` again"
        );
    }
}
