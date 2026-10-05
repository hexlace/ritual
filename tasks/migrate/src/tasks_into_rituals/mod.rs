//! The first step: a project's tasks move from `tasks/` to `.rituals/`.

mod plan;
mod refusals;

use std::path::{Path, PathBuf};

use rituals::Failure;
use rituals_compose::metadata::{self, Metadata, TaskImport, WorkspaceMember};
use rituals_compose::rollback::Changes;

use crate::step::{Applied, Migrating};
use crate::tidy::Vacated;

// This step owns both ends literally, and so will every step after it:
// `tasks/` is where 0.1 put a project's tasks and `.rituals/` is where 0.2
// does, for good. `layout::tasks_directory` says where scaffolding puts a
// task today, and reading the destination from it would make this step move
// tasks to wherever the layout says next. A later release that changes the
// layout adds a step that runs after this one in release order, so a project
// at any older layout is brought forward one release at a time.

/// The directory this step moves tasks out of.
const FROM: &str = "tasks";

/// The directory this step moves tasks into.
const TO: &str = ".rituals";

/// The task directories this step would move.
#[derive(Debug)]
pub(crate) struct Candidates {
    from_directory: PathBuf,
    to_directory: PathBuf,
    directories: Vec<PathBuf>,
}

/// The tasks under `tasks/` that `package`'s command line imports, when there
/// are any.
///
/// A task is a workspace member that declares itself one, by the one rule the
/// task list is resolved with, and the command line imports it when its
/// `[package.metadata.ritual] tasks` list names it. A task under `tasks/`
/// that only another task depends on is neither moved nor listed: it stays
/// where it is, and what reaches it is repointed. A project with no task
/// under `tasks/` has nothing to migrate whatever its list holds, so the list
/// is read only once there is one.
///
/// Cargo reports each member's directory absolute and without `.` or `..`, as
/// it does the root and the directory an import's `path` points at once
/// `task_imports` has checked it against the package, so they are compared as
/// the text they are.
///
/// # Errors
///
/// Returns a [`Failure`] when `package` cannot be located or its list cannot
/// be read, which only matters once a task under `tasks/` exists.
pub(crate) fn find(
    document: &Metadata,
    root: &Path,
    package: &str,
) -> Result<Option<Candidates>, Failure> {
    let from_directory = root.join(FROM);
    let under_tasks: Vec<&Path> = document
        .workspace_members()
        .iter()
        .filter(|member| member.declares_a_task_crate())
        .map(WorkspaceMember::directory)
        .filter(|directory| *directory != from_directory && directory.starts_with(&from_directory))
        .collect();
    if under_tasks.is_empty() {
        return Ok(None);
    }

    let imported: Vec<&Path> = document
        .task_imports(package)?
        .iter()
        .filter(|import| import.is_workspace_member())
        .filter_map(TaskImport::directory)
        .collect();
    let mut directories: Vec<PathBuf> = under_tasks
        .into_iter()
        .filter(|directory| imported.contains(directory))
        .map(Path::to_path_buf)
        .collect();
    directories.sort_unstable();
    directories.dedup();

    Ok((!directories.is_empty()).then(|| Candidates {
        from_directory,
        to_directory: root.join(TO),
        directories,
    }))
}

/// Moves every candidate and repoints everything that reached it, inside
/// `changes` so a failure at any point is put back.
///
/// Plans first, which refuses before anything is written; writes; then asks
/// Cargo to read the project again and checks it reads what it should. The
/// check is the last thing in the run, so a result that no longer resolves
/// is a failure the rollback undoes.
pub(crate) fn apply(
    candidates: &Candidates,
    migrating: &Migrating<'_>,
    before: &Metadata,
    changes: &mut Changes,
) -> Result<Applied, Failure> {
    let planned = plan::plan(candidates, migrating, before)?;
    planned.write(changes)?;
    let after = metadata::fetch_recording(changes, migrating.root)?;
    planned.verify(before, &after, migrating.package)?;
    Ok(Applied {
        lines: planned.lines(),
        vacated: vec![Vacated {
            boundary: candidates.from_directory.clone(),
            sources: candidates.directories.clone(),
        }],
        after,
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use rituals_compose::metadata;
    use rituals_compose::rollback;

    use super::{apply, find};
    use crate::precondition::WorkTree;
    use crate::step::Migrating;
    use crate::test_support::{
        ScratchDir, TestOutcome, init_and_commit, package, workspace, write_files,
    };

    const MIGRATE: &str = "cargo ritual migrate";

    /// The 0.1 project of the stories: a command line crate, and two tasks,
    /// `shout` using `greet`, all named by one glob over `tasks/`.
    ///
    /// The command line lists the tasks and not ritual's own bundle, and
    /// everything depends on a stand-in `rituals` crate, because a test with
    /// no network cannot resolve the real ones and the task list only asks
    /// that the dependency is there.
    fn legacy_project(root: &Path) -> TestOutcome {
        let task = |name: &str, dependencies: &str| {
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
                 [dependencies]\nrituals.workspace = true\n{dependencies}\n\
                 [package.metadata.ritual]\ntask = true\n"
            )
        };
        write_files(
            root,
            &[
                (
                    "Cargo.toml",
                    "[workspace]\nmembers = [\"ritual\", \"tasks/*\"]\nresolver = \"3\"\n\n\
                     [workspace.dependencies]\nrituals = { path = \"vendor/rituals\" }\n",
                ),
                (
                    "ritual/Cargo.toml",
                    "[package]\nname = \"ritual\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
                     [[bin]]\nname = \"ritual\"\npath = \"src/main.rs\"\n\n\
                     [dependencies]\nrituals.workspace = true\n\
                     greet = { path = \"../tasks/greet\" }\n\
                     shout = { path = \"../tasks/shout\" }\n\n\
                     [package.metadata.ritual]\ntasks = [\"greet\", \"shout\"]\n",
                ),
                ("ritual/src/main.rs", "fn main() {}\n"),
                ("tasks/greet/Cargo.toml", &task("greet", "")),
                ("tasks/greet/src/lib.rs", "//! A fixture.\n"),
                (
                    "tasks/shout/Cargo.toml",
                    &task("shout", "greet = { path = \"../greet\" }\n"),
                ),
                ("tasks/shout/src/lib.rs", "//! A fixture.\n"),
                ("vendor/rituals/Cargo.toml", &package("rituals", false)),
                (
                    "vendor/rituals/src/lib.rs",
                    "//! A stand-in for ritual's framework.\n",
                ),
            ],
        )
    }

    fn found(root: &Path) -> Result<Option<Vec<PathBuf>>, Box<dyn std::error::Error>> {
        let document = metadata::fetch(root)?;
        Ok(find(&document, root, "ritual")?.map(|candidates| candidates.directories))
    }

    #[test]
    fn every_task_under_tasks_is_a_candidate_in_path_order() -> TestOutcome {
        let scratch = ScratchDir::new("step1-find")?;
        let root = scratch.path();
        legacy_project(root)?;

        let directories = found(root)?;

        assert_eq!(
            directories,
            Some(vec![root.join("tasks/greet"), root.join("tasks/shout")])
        );
        Ok(())
    }

    /// `helper` is a task that only `greet` depends on: the command line does
    /// not list it, so it is not one of the project's own tasks to move. It
    /// stays in `tasks/`, and the path `greet` reaches it by follows `greet`.
    #[test]
    fn a_task_only_another_task_depends_on_is_not_a_candidate() -> TestOutcome {
        let scratch = ScratchDir::new("step1-dependency-only")?;
        let root = scratch.path();
        legacy_project(root)?;
        write_files(
            root,
            &[
                ("tasks/helper/Cargo.toml", &package("helper", true)),
                ("tasks/helper/src/lib.rs", "//! A fixture.\n"),
                (
                    "tasks/greet/Cargo.toml",
                    "[package]\nname = \"greet\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
                     [dependencies]\nrituals.workspace = true\n\
                     helper = { path = \"../helper\" }\n\n\
                     [package.metadata.ritual]\ntask = true\n",
                ),
            ],
        )?;

        assert_eq!(
            found(root)?,
            Some(vec![root.join("tasks/greet"), root.join("tasks/shout")])
        );
        let lines = applied(root)??;
        assert!(
            lines.contains(
                &"updated .rituals/greet/Cargo.toml ([dependencies] helper path `../helper` is \
                  now `../../tasks/helper`)"
                    .to_string()
            ),
            "{lines:?}"
        );
        assert!(root.join("tasks/helper/Cargo.toml").is_file());
        assert!(!root.join(".rituals/helper").exists());
        Ok(())
    }

    /// A task nested a level down is a candidate too, and its place under
    /// `.rituals/` keeps the level.
    #[test]
    fn a_task_nested_under_tasks_is_a_candidate() -> TestOutcome {
        let scratch = ScratchDir::new("step1-nested")?;
        let root = scratch.path();
        workspace(root, &["tasks/group/deep"], &["tasks/group/deep"], &[])?;

        assert_eq!(found(root)?, Some(vec![root.join("tasks/group/deep")]));
        Ok(())
    }

    #[test]
    fn a_plain_crate_in_tasks_is_not_a_candidate_beside_a_task() -> TestOutcome {
        let scratch = ScratchDir::new("step1-plain")?;
        let root = scratch.path();
        workspace(
            root,
            &["tasks/greet", "tasks/helper"],
            &["tasks/greet", "tasks/helper"],
            &["tasks/helper"],
        )?;

        assert_eq!(found(root)?, Some(vec![root.join("tasks/greet")]));
        Ok(())
    }

    fn migrating<'a>(
        root: &'a Path,
        repository: &'a crate::precondition::Repository,
    ) -> Migrating<'a> {
        Migrating {
            root,
            package: "ritual",
            migrate_command: MIGRATE,
            repository,
        }
    }

    /// Runs the step the way `run` does, inside one rollback, on a committed
    /// copy of the project.
    fn applied(root: &Path) -> Result<Result<Vec<String>, String>, Box<dyn std::error::Error>> {
        init_and_commit(root)?;
        let work_tree = WorkTree::take(root);
        let repository = work_tree
            .ensure_clean(MIGRATE)
            .map_err(|failure| failure.to_string())?;
        let migrating = migrating(root, repository);
        let before = metadata::fetch(root)?;
        let Some(candidates) = find(&before, root, "ritual")? else {
            return Err("fixture precondition: the project must have tasks to move".into());
        };
        let outcome = rollback::attempt("running `migrate` again", |changes| {
            apply(&candidates, &migrating, &before, changes)
        });
        Ok(outcome
            .map(|applied| applied.lines)
            .map_err(|failure| failure.to_string()))
    }

    #[test]
    fn the_step_moves_each_task_and_says_what_it_edited_in_the_order_it_wrote() -> TestOutcome {
        let scratch = ScratchDir::new("step1-apply")?;
        let root = scratch.path();
        legacy_project(root)?;

        let lines = applied(root)??;

        assert_eq!(
            lines,
            [
                "moved tasks/greet to .rituals/greet",
                "moved tasks/shout to .rituals/shout",
                "updated Cargo.toml ([workspace] members `tasks/*` is now `.rituals/*`)",
                "updated ritual/Cargo.toml ([dependencies] greet path `../tasks/greet` is now \
                 `../.rituals/greet`)",
                "updated ritual/Cargo.toml ([dependencies] shout path `../tasks/shout` is now \
                 `../.rituals/shout`)",
            ]
        );
        assert!(root.join(".rituals/greet/Cargo.toml").is_file());
        assert!(root.join(".rituals/shout/Cargo.toml").is_file());
        assert!(!root.join("tasks/greet").exists());
        Ok(())
    }

    /// `shout` and `greet` moved together, so the way from one to the other
    /// is the way it was, and the person's spelling is kept.
    #[test]
    fn a_path_between_two_moved_tasks_is_left_as_written() -> TestOutcome {
        let scratch = ScratchDir::new("step1-sibling")?;
        let root = scratch.path();
        legacy_project(root)?;

        applied(root)??;

        let shout = std::fs::read_to_string(root.join(".rituals/shout/Cargo.toml"))?;
        assert!(shout.contains("greet = { path = \"../greet\" }"), "{shout}");
        Ok(())
    }

    /// A task's own manifest is edited where it stands and then carried by
    /// its move: `greet` reaches a helper that stays in `tasks/`, and from
    /// `.rituals/greet` that is no longer `../helper`. Written the other way
    /// round, the edit would land in a directory that is already gone.
    #[test]
    fn a_task_that_depends_on_something_left_behind_is_edited_and_carried() -> TestOutcome {
        let scratch = ScratchDir::new("step1-carried")?;
        let root = scratch.path();
        legacy_project(root)?;
        write_files(
            root,
            &[
                ("tasks/helper/Cargo.toml", &package("helper", false)),
                ("tasks/helper/src/lib.rs", "//! A fixture.\n"),
                (
                    "tasks/greet/Cargo.toml",
                    "[package]\nname = \"greet\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
                     [dependencies]\nrituals.workspace = true\n\
                     helper = { path = \"../helper\" }\n\n\
                     [package.metadata.ritual]\ntask = true\n",
                ),
            ],
        )?;

        let lines = applied(root)??;

        assert!(
            lines.contains(
                &"updated .rituals/greet/Cargo.toml ([dependencies] helper path `../helper` is \
                  now `../../tasks/helper`)"
                    .to_string()
            ),
            "{lines:?}"
        );
        let greet = std::fs::read_to_string(root.join(".rituals/greet/Cargo.toml"))?;
        assert!(
            greet.contains("helper = { path = \"../../tasks/helper\" }"),
            "{greet}"
        );
        assert!(!root.join("tasks/greet").exists());
        assert!(root.join("tasks/helper/Cargo.toml").is_file());
        Ok(())
    }

    /// Cargo reads a `paths` override in its configuration, which no manifest
    /// names: it is what the check after the move is for. The move is undone,
    /// manifests and directories both, and the failure says so.
    #[test]
    fn a_project_that_does_not_resolve_after_the_move_is_put_back() -> TestOutcome {
        let scratch = ScratchDir::new("step1-rollback")?;
        let root = scratch.path();
        legacy_project(root)?;
        write_files(
            root,
            &[(".cargo/config.toml", "paths = [\"tasks/greet\"]\n")],
        )?;
        let manifest_before = std::fs::read_to_string(root.join("Cargo.toml"))?;

        let failure = applied(root)?.err().ok_or("the move must fail the check")?;

        assert!(
            failure.ends_with("ritual put the project back as it found it"),
            "{failure}"
        );
        assert!(root.join("tasks/greet/Cargo.toml").is_file());
        assert!(!root.join(".rituals").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("Cargo.toml"))?,
            manifest_before
        );
        Ok(())
    }

    /// A glob that does not lead with `tasks/` is the person's own and is left
    /// alone, so it may stop matching a task once the task has moved. Another
    /// crate still matches the glob, so Cargo loads the workspace, and the
    /// workspace excludes the place `orphan` moves to, so the command line's
    /// dependency on it no longer makes it a member: the check after the move
    /// names it and the move is undone.
    #[test]
    fn a_member_the_move_leaves_unreachable_is_named_and_the_project_put_back() -> TestOutcome {
        let scratch = ScratchDir::new("step1-lost-member")?;
        let root = scratch.path();
        legacy_project(root)?;
        write_files(
            root,
            &[
                (
                    "Cargo.toml",
                    "[workspace]\nmembers = [\"ritual\", \"tasks/greet\", \"tasks/shout\", \
                     \"t*/orphan\"]\nexclude = [\".rituals/orphan\"]\nresolver = \"3\"\n\n\
                     [workspace.dependencies]\nrituals = { path = \"vendor/rituals\" }\n",
                ),
                (
                    "ritual/Cargo.toml",
                    "[package]\nname = \"ritual\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
                     [[bin]]\nname = \"ritual\"\npath = \"src/main.rs\"\n\n\
                     [dependencies]\nrituals.workspace = true\n\
                     greet = { path = \"../tasks/greet\" }\n\
                     shout = { path = \"../tasks/shout\" }\n\
                     orphan = { path = \"../tasks/orphan\" }\n\n\
                     [package.metadata.ritual]\ntasks = [\"greet\", \"shout\", \"orphan\"]\n",
                ),
                (
                    "tasks/orphan/Cargo.toml",
                    // Not inheriting from the workspace: once excluded, it has no
                    // workspace root to inherit from.
                    "[package]\nname = \"orphan\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
                     [dependencies]\nrituals = { path = \"../../vendor/rituals\" }\n\n\
                     [package.metadata.ritual]\ntask = true\n",
                ),
                ("tasks/orphan/src/lib.rs", "//! A fixture.\n"),
                ("tools/orphan/Cargo.toml", &package("twin", false)),
                ("tools/orphan/src/lib.rs", "//! A fixture.\n"),
            ],
        )?;

        let failure = applied(root)?
            .err()
            .ok_or("the lost member must fail the check")?;

        assert_eq!(
            failure,
            "after the move, Cargo no longer reads .rituals/orphan as a workspace member; ritual \
             put the project back as it found it"
        );
        assert!(root.join("tasks/orphan/Cargo.toml").is_file());
        assert!(!root.join(".rituals").exists());
        Ok(())
    }

    #[test]
    fn a_task_directory_holding_a_member_is_refused_before_anything_moves() -> TestOutcome {
        let scratch = ScratchDir::new("step1-holds")?;
        let root = scratch.path();
        workspace(
            root,
            &["tasks/greet", "tasks/greet/inner"],
            &["tasks/greet", "tasks/greet/inner"],
            &["tasks/greet/inner"],
        )?;

        let failure = applied(root)?
            .err()
            .ok_or("a task holding a member must be refused")?;

        assert!(
            failure.starts_with(
                "refusing to move tasks/greet: it also holds the workspace members `inner`"
            ),
            "{failure}"
        );
        assert!(root.join("tasks/greet/Cargo.toml").is_file());
        assert!(!root.join(".rituals").exists());
        Ok(())
    }
}
