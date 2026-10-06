//! The `create` task: scaffold a task crate. In the running command line's
//! own project it goes in `.rituals/` and is imported, the project's own task
//! list regenerated; wherever a crate builds on its own it is a crate on its
//! own, for a project to import later; anywhere else it is refused.

mod arguments;
mod crate_files;
mod in_project;
mod standalone;
#[cfg(test)]
mod test_support;

use rituals::{CommandLine, Failure, Outcome, Task, clap};
use rituals_compose::metadata::{self, Owner};
use rituals_compose::source::{Source, SourceArguments};
use rituals_compose::workspace;

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
/// and, once it has finished writing, to regenerate with. Whose workspace it
/// runs in decides what it does: in its own project it scaffolds into
/// `.rituals/` and imports, and where a crate builds on its own it scaffolds
/// a crate of its own in the current directory.
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

/// Asks Cargo whose workspace this is, and does what that answer says.
///
/// In the running command line's own project it scaffolds into the project.
/// Anywhere else it makes a crate of its own, wherever Cargo would build one
/// standing alone: under no manifest at all, or under an ordinary package
/// with no `[workspace]`. Where it would not, it refuses before anything is
/// written, with the ways out that place has.
fn run(command_line: &CommandLine, arguments: &Arguments) -> Outcome {
    let current_dir = std::env::current_dir()
        .map_err(|error| Failure::new("reading the current directory failed").caused_by(error))?;
    let package = command_line.identity().package_name();
    match metadata::whose_workspace(&current_dir, package)? {
        Owner::TheCommandLine => {
            ensure_no_source_was_named(&arguments.source)?;
            in_project::scaffold(command_line, &arguments.scaffold, &current_dir)
        }
        Owner::AnotherCommandLine => Err(another_projects_refusal(&arguments.scaffold)),
        Owner::NoCommandLine => {
            workspace::ensure_the_directory_stands_alone(
                &current_dir,
                &no_projects_refusal(&arguments.scaffold),
            )?;
            standalone::scaffold(&arguments.scaffold, &arguments.source, &current_dir)
        }
    }
}

/// The way out every refusal here ends on: a crate of its own, made where it
/// builds on its own. It names no arguments, because outside a project
/// `create` takes a bare name where here it may have been given a path.
const MAKE_ONE_OF_ITS_OWN: &str = "run create outside any Cargo workspace";

/// The refusal in another composed command line's project: run that one, or
/// make a ritual of its own somewhere a crate stands alone. `--path` and
/// `--git` are left out of the first, which that project's `create` refuses.
fn another_projects_refusal(arguments: &ScaffoldArguments) -> Failure {
    let not_its_project =
        metadata::outside_its_project_refusal("create", &arguments.to_run_again());
    Failure::new(format!(
        "{not_its_project}; or, for a ritual of its own, {MAKE_ONE_OF_ITS_OWN}"
    ))
}

/// The refusal where no command line owns the workspace and a crate made
/// here would not build on its own: there is no `cargo ritual` to send a
/// person to, so it names the one way out there is.
fn no_projects_refusal(arguments: &ScaffoldArguments) -> workspace::Refusal {
    workspace::Refusal {
        attempted_command: format!("create {}", arguments.to_run_again()),
        why_not_here: "a crate made there would not build on its own".to_string(),
        what_to_do_instead: MAKE_ONE_OF_ITS_OWN.to_string(),
    }
}

/// Refuses a `--path` or `--git` given in the command line's own project,
/// where a new ritual inherits the workspace's `rituals` and the flag would
/// change nothing.
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
