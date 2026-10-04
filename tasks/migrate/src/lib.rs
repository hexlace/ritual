//! The `migrate` task: bring this project up to the layout of the ritual it
//! runs.

// Every module here is private, so `pub(crate)` is the visibility an item
// actually has; `redundant_pub_crate` (clippy nursery) would have it `pub`,
// which `unreachable_pub` then calls unreachable.
#![expect(
    clippy::redundant_pub_crate,
    reason = "every module of this crate is private, so pub(crate) is the visibility that is \
              actually true; see the note above"
)]

mod mentions;
mod places;
mod precondition;
mod report;
mod step;
mod tasks_into_rituals;
#[cfg(test)]
mod test_support;
mod tidy;

use std::path::Path;

use precondition::{Repository, WorkTree};
use rituals::{CommandLine, Failure, Outcome, Task, clap, report};
use rituals_compose::metadata;
use rituals_compose::rollback::{self, Changes};
use rituals_compose::{top_level, workspace};
use step::{Context, Step};
use tidy::Vacated;

/// `migrate` takes no arguments: the layout a project is brought up to is the
/// one of the ritual running it.
#[derive(clap::Args)]
struct MigrateArguments {}

/// This task, for a command line to mount under whatever name imports it.
//
// No `# Examples` section: a `task()` function has exactly one call-site
// shape — a mount line in a generated file — and an example here could only
// restate `let task = migrate::task();`, which shows nothing a reader needs.
#[must_use]
pub fn task() -> Task {
    Task::receiving_command_line(
        "bring this project up to the layout of the ritual it runs",
        |command_line: &CommandLine, _arguments: MigrateArguments| run(command_line),
    )
}

/// What a run that applied at least one step did, for the report that
/// follows once the changes are kept.
struct Done<'a> {
    /// The repository the project is in, which was clean.
    repository: &'a Repository,
    lines: Vec<String>,
    vacated: Vec<Vacated>,
}

fn run(command_line: &CommandLine) -> Outcome {
    let current_dir = std::env::current_dir()
        .map_err(|error| Failure::new("reading the current directory failed").caused_by(error))?;
    // Where there is no project at all there is nothing to put back, so this
    // is refused before the run that would say it put the project back.
    metadata::ensure_inside_a_project(&current_dir, "migrate", "")?;
    let root = workspace::root(&current_dir)?;
    // Asked before the first `cargo metadata`, which can rewrite `Cargo.lock`
    // and would make a clean work tree dirty. Acted on only once a step
    // applies: a project with nothing to migrate has nothing to give back, so
    // it is told so whatever git says.
    let work_tree = WorkTree::take(&root);
    let migrate_command = top_level::management_command(command_line, "migrate");

    let done = rollback::attempt("running `migrate` again", |changes| {
        run_every_step_that_applies(
            changes,
            command_line.identity().package_name(),
            &current_dir,
            &Run {
                root: &root,
                work_tree: &work_tree,
                migrate_command: &migrate_command,
            },
        )
    })?;

    match done {
        None => report(report::NOTHING_TO_MIGRATE),
        Some(done) => {
            for line in done.lines {
                report(line);
            }
            // After the changes are kept, outside the rollback: what these
            // two do or fail to do is no reason to undo a migration that
            // worked.
            for line in tidy::tidy(&root, &done.vacated) {
                report(line);
            }
            for line in mentions::lines(&root, done.repository) {
                report(line);
            }
            report(report::NEXT);
        }
    }
    Ok(())
}

/// What every step of one run shares.
struct Run<'a> {
    root: &'a Path,
    work_tree: &'a WorkTree,
    migrate_command: &'a str,
}

/// Runs each step that applies, in release order, each reading the project
/// the one before left, and returns what they did, or `None` when no step
/// applied.
///
/// Inside the run's one rollback, so a step that fails puts back what every
/// step before it did too.
fn run_every_step_that_applies<'a>(
    changes: &mut Changes,
    package: &str,
    current_dir: &Path,
    run: &Run<'a>,
) -> Result<Option<Done<'a>>, Failure> {
    // `cargo metadata` creates or rewrites a missing or stale lockfile,
    // which this records first, so a refusal puts it back.
    let mut document =
        metadata::fetch_in_its_own_project(changes, current_dir, package, "migrate", "")?;
    let mut done: Option<Done<'a>> = None;
    for step in Step::IN_RELEASE_ORDER {
        let Some(migration) = step.applies(&document, run.root) else {
            continue;
        };
        let repository = run.work_tree.ensure_clean(run.migrate_command)?;
        let applied = migration.apply(
            &Context {
                root: run.root,
                package,
                migrate_command: run.migrate_command,
                repository,
            },
            &document,
            changes,
        )?;
        let so_far = done.get_or_insert_with(|| Done {
            repository,
            lines: Vec::new(),
            vacated: Vec::new(),
        });
        so_far.lines.extend(applied.lines);
        so_far.vacated.extend(applied.vacated);
        document = applied.after;
    }
    Ok(done)
}
