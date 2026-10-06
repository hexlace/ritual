//! The `create` task: scaffold a task crate. Inside a project it goes in
//! `.rituals/` and is imported, the project's own task list regenerated;
//! outside any project it is a crate on its own, for a project to import
//! later.

mod arguments;
mod crate_files;
mod in_project;
mod standalone;
#[cfg(test)]
mod test_support;

use rituals::{CommandLine, Failure, Outcome, Task, clap};
use rituals_compose::metadata::{self, Surroundings};
use rituals_compose::source::{Source, SourceArguments};

pub use arguments::ScaffoldArguments;
pub use in_project::scaffold_in_project;

/// `create`'s arguments: the ritual to scaffold, and where ritual's own
/// crates come from for one made outside a project.
#[derive(clap::Args)]
struct Arguments {
    #[command(flatten)]
    scaffold: ScaffoldArguments,

    #[command(flatten)]
    source: SourceArguments,
}

/// This task, for a command line to mount under whatever name imports it.
///
/// Reads the composed CLI's command line it is invoked with, the same
/// opt-in any task can use, to find itself among the workspace's members
/// and, once it has finished writing, to regenerate with. Where it runs
/// decides what it does: inside a project it scaffolds into `.rituals/` and
/// imports, and outside any project it scaffolds a crate of its own in the
/// current directory.
//
// No `# Examples` section: a `task()` function has exactly one call-site
// shape — a mount line in a generated file — and an example here could only
// restate `let task = create::task();`, which shows nothing a reader needs.
#[must_use]
pub fn task() -> Task {
    Task::receiving_command_line(
        "scaffold a task crate: into this project, or on its own outside any project",
        |command_line: &CommandLine, arguments: Arguments| run(command_line, &arguments),
    )
}

/// Asks Cargo once whether there is a project here, and does what that
/// answer says.
fn run(command_line: &CommandLine, arguments: &Arguments) -> Outcome {
    let current_dir = std::env::current_dir()
        .map_err(|error| Failure::new("reading the current directory failed").caused_by(error))?;
    match metadata::surroundings(&current_dir)? {
        Surroundings::InsideAProject => {
            ensure_no_source_was_named(&arguments.source)?;
            in_project::scaffold(command_line, &arguments.scaffold, &current_dir)
        }
        Surroundings::OutsideAnyProject => {
            standalone::scaffold(&arguments.scaffold, &arguments.source, &current_dir)
        }
    }
}

/// Refuses a `--path` or `--git` given inside a project, where a new ritual
/// inherits the workspace's `rituals` and the flag would change nothing.
fn ensure_no_source_was_named(source: &SourceArguments) -> Outcome {
    let flag = match source.resolve() {
        Source::Registry => return Ok(()),
        Source::Path(_) => "--path",
        Source::Git(_) => "--git",
        Source::Inherited => {
            unreachable!("resolving the arguments names a flag or none, never the workspace's")
        }
    };
    Err(Failure::new(format!(
        "{flag} chooses where a crate made outside a project gets rituals from; inside a \
         project a new ritual inherits the workspace's, so leave {flag} off"
    )))
}
