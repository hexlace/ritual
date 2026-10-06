//! The `add` task: `create`'s in-project path under its old name.
//!
//! A shim over [`create::scaffold_in_project`], so there is one
//! copy of the scaffolding. It takes the arguments `create` takes inside a
//! project.

use create::{ScaffoldArguments, scaffold_in_project};
use rituals::{CommandLine, Task};

/// This task, for a command line to mount under whatever name imports it.
///
/// Reads the composed CLI's command line it is invoked with, the same
/// opt-in any task can use, to find itself among the workspace's members
/// and, once it has finished writing, to regenerate with.
//
// No `# Examples` section: a `task()` function has exactly one call-site
// shape — a mount line in a generated file — and an example here could only
// restate `let task = add::task();`, which shows nothing a reader needs.
#[must_use]
pub fn task() -> Task {
    Task::receiving_command_line(
        "scaffold a task crate in this project, import it, and regenerate",
        |command_line: &CommandLine, arguments: ScaffoldArguments| {
            scaffold_in_project(command_line, &arguments)
        },
    )
}
