//! The first step: a project's tasks move from `tasks/` to `.rituals/`.

mod plan;
mod refusals;

use std::path::{Path, PathBuf};

use rituals::Failure;
use rituals_compose::metadata::{self, Metadata};
use rituals_compose::rollback::Changes;

use crate::step::{Applied, Migrating};
use crate::tidy::{self, Vacated};

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

/// The task members under `tasks/`, when there are any.
///
/// A task is a workspace member that declares itself one, by the one rule the
/// task list is resolved with. Every ritual lives in `.rituals/` whoever it is
/// for, so every task under `tasks/` moves, whether or not the project's
/// command line imports it. Cargo reports each member's directory
/// absolute and without `.` or `..`, as it does the root, so they are
/// compared as the text they are.
pub(crate) fn find(document: &Metadata, root: &Path) -> Option<Candidates> {
    let from_directory = root.join(FROM);
    let mut directories: Vec<PathBuf> = document
        .workspace_members()
        .iter()
        .filter(|member| member.declares_a_task_crate())
        .map(|member| member.directory().to_path_buf())
        .filter(|directory| *directory != from_directory && directory.starts_with(&from_directory))
        .collect();
    directories.sort_unstable();
    directories.dedup();

    (!directories.is_empty()).then(|| Candidates {
        from_directory,
        to_directory: root.join(TO),
        directories,
    })
}

/// Moves every candidate and repoints everything that reached it, inside
/// `changes` so a failure at any point is put back.
///
/// Plans first, which refuses before anything is written; writes; removes
/// the directories the moves emptied; then asks Cargo to read the project
/// again and checks it reads what it should. The check is the last thing in
/// the run and sees the tree the run leaves, so a glob that matched only a
/// directory the moves emptied cannot pass it, and a result that no longer
/// resolves is a failure the rollback undoes.
pub(crate) fn apply(
    candidates: &Candidates,
    migrating: &Migrating<'_>,
    before: &Metadata,
    changes: &mut Changes,
) -> Result<Applied, Failure> {
    let planned = plan::plan(candidates, migrating, before)?;
    planned.write(changes)?;
    let tidied = tidy::tidy(
        migrating.root,
        &Vacated {
            boundary: candidates.from_directory.clone(),
            sources: candidates.directories.clone(),
        },
        changes,
    );
    let after = metadata::fetch_recording(changes, migrating.root)?;
    planned.verify(before, &after, migrating.package)?;
    let mut lines = planned.lines();
    lines.extend(tidied);
    Ok(Applied { lines, after })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use rituals_compose::metadata;
    use rituals_compose::rollback::{self, Wording};

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
        Ok(find(&document, root).map(|candidates| candidates.directories))
    }

    #[test]
    fn every_task_under_tasks_is_a_candidate_in_path_order() -> TestOutcome {
        let scratch = ScratchDir::resolved("step1-find")?;
        let root = scratch.path();
        legacy_project(root)?;

        let directories = found(root)?;

        assert_eq!(
            directories,
            Some(vec![root.join("tasks/greet"), root.join("tasks/shout")])
        );
        Ok(())
    }

    /// `helper` is a task that only `greet` depends on, and the command line
    /// does not list it. It is still a task under `tasks/`, so it moves with
    /// the rest, and since it and `greet` move together the way from one to
    /// the other is left as written.
    #[test]
    fn every_task_under_tasks_is_a_candidate_whatever_the_command_line_imports() -> TestOutcome {
        let scratch = ScratchDir::resolved("step1-dependency-only")?;
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
            Some(vec![
                root.join("tasks/greet"),
                root.join("tasks/helper"),
                root.join("tasks/shout"),
            ])
        );
        let lines = applied(root)??;
        assert!(
            lines.contains(&"moved tasks/helper to .rituals/helper".to_string()),
            "{lines:?}"
        );
        let greet = std::fs::read_to_string(root.join(".rituals/greet/Cargo.toml"))?;
        assert!(
            greet.contains("helper = { path = \"../helper\" }"),
            "{greet}"
        );
        assert!(root.join(".rituals/helper/Cargo.toml").is_file());
        assert!(!root.join("tasks/helper").exists());
        Ok(())
    }

    /// A task nested a level down is a candidate too, and its place under
    /// `.rituals/` keeps the level.
    #[test]
    fn a_task_nested_under_tasks_is_a_candidate() -> TestOutcome {
        let scratch = ScratchDir::resolved("step1-nested")?;
        let root = scratch.path();
        workspace(root, &["tasks/group/deep"], &["tasks/group/deep"], &[])?;

        assert_eq!(found(root)?, Some(vec![root.join("tasks/group/deep")]));
        Ok(())
    }

    #[test]
    fn a_plain_crate_in_tasks_is_not_a_candidate_beside_a_task() -> TestOutcome {
        let scratch = ScratchDir::resolved("step1-plain")?;
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
            .map_err(|failure| failure.with_causes().to_string())?;
        let migrating = migrating(root, repository);
        let before = metadata::fetch(root)?;
        let Some(candidates) = find(&before, root) else {
            return Err("fixture precondition: the project must have tasks to move".into());
        };
        let outcome = rollback::attempt(
            Wording::project(root, "running `migrate` again"),
            |changes| apply(&candidates, &migrating, &before, changes),
        );
        Ok(outcome
            .map(|applied| applied.lines)
            .map_err(|failure| failure.with_causes().to_string()))
    }

    #[test]
    fn the_step_moves_each_task_and_says_what_it_edited_and_deleted_in_the_order_it_did_it()
    -> TestOutcome {
        let scratch = ScratchDir::resolved("step1-apply")?;
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
                "deleted tasks/ (empty once its tasks moved out)",
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
        let scratch = ScratchDir::resolved("step1-sibling")?;
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
        let scratch = ScratchDir::resolved("step1-carried")?;
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

    /// `legacy_project` with a crate `vendor/x`, which depends on `greet`, kept
    /// out of the workspace by `exclude` and reached only through an
    /// optional dependency of the command line: Cargo reads it, and
    /// `cargo metadata` does not list it.
    fn with_an_excluded_crate_that_reaches_greet(root: &Path) -> TestOutcome {
        legacy_project(root)?;
        let workspace = std::fs::read_to_string(root.join("Cargo.toml"))?
            .replace("\"tasks/*\"]", "\"tasks/*\"]\nexclude = [\"vendor/x\"]");
        let command_line = std::fs::read_to_string(root.join("ritual/Cargo.toml"))?.replace(
            "[package.metadata.ritual]",
            "x = { path = \"../vendor/x\", optional = true }\n\n[package.metadata.ritual]",
        );
        write_files(
            root,
            &[
                ("Cargo.toml", &workspace),
                ("ritual/Cargo.toml", &command_line),
                (
                    "vendor/x/Cargo.toml",
                    &format!(
                        "{}\n[dependencies]\ngreet = {{ path = \"../../tasks/greet\" }}\n",
                        package("x", false)
                    ),
                ),
                ("vendor/x/src/lib.rs", "//! A fixture.\n"),
            ],
        )
    }

    /// The crate is outside the workspace and reachable only through an
    /// optional dependency, so `cargo metadata` never lists it, and Cargo
    /// still reads its path to `greet`: it has to follow `greet`. It does not
    /// move, so its own base is the same before and after.
    #[test]
    fn an_excluded_crate_reached_only_through_an_optional_dependency_is_repointed() -> TestOutcome {
        let scratch = ScratchDir::resolved("step1-excluded")?;
        let root = scratch.path();
        with_an_excluded_crate_that_reaches_greet(root)?;

        let lines = applied(root)??;

        assert!(
            lines.contains(
                &"updated vendor/x/Cargo.toml ([dependencies] greet path `../../tasks/greet` is \
                  now `../../.rituals/greet`)"
                    .to_string()
            ),
            "{lines:?}"
        );
        let excluded = std::fs::read_to_string(root.join("vendor/x/Cargo.toml"))?;
        assert!(
            excluded.contains("greet = { path = \"../../.rituals/greet\" }"),
            "{excluded}"
        );
        Ok(())
    }

    /// Git ignores `vendor/x`, so once migrate edits its manifest git could
    /// not give the old bytes back: the step refuses, naming the file and the
    /// edit, and writes nothing, so the refusal says nothing about putting
    /// anything back.
    #[test]
    fn an_untracked_manifest_that_needs_an_edit_is_refused() -> TestOutcome {
        let scratch = ScratchDir::resolved("step1-untracked")?;
        let root = scratch.path();
        with_an_excluded_crate_that_reaches_greet(root)?;
        write_files(root, &[(".gitignore", "/vendor/x/\n")])?;

        let failure = applied(root)?
            .err()
            .ok_or("an untracked manifest that needs an edit must be refused")?;

        assert_eq!(
            failure,
            "refusing to migrate: git does not track vendor/x/Cargo.toml, which Cargo reads and \
             which would need editing ([dependencies] greet path `../../tasks/greet` is now \
             `../../.rituals/greet`); git could not give it back once migrate edits it, so \
             commit it, or take that path out of it, then run `cargo ritual migrate` again"
        );
        assert!(root.join("tasks/greet/Cargo.toml").is_file());
        assert!(!root.join(".rituals").exists());
        Ok(())
    }

    /// Cargo reads a `paths` override in its configuration, which no manifest
    /// names: it is what the check after the move is for. The move is undone,
    /// manifests and directories both, and the failure says so.
    #[test]
    fn a_project_that_does_not_resolve_after_the_move_is_put_back() -> TestOutcome {
        let scratch = ScratchDir::resolved("step1-rollback")?;
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

    /// A ritual lives in a subdirectory of `.rituals/` whose name means nothing
    /// to ritual, so `.rituals/private` is no crate. The `tasks/*` glob becomes
    /// `.rituals/*`, which matches that directory too, and Cargo refuses a
    /// glob member without a manifest: the check after the move reports it and
    /// the move is undone.
    #[test]
    fn a_tasks_glob_that_would_match_a_grouping_directory_is_put_back() -> TestOutcome {
        let scratch = ScratchDir::resolved("step1-grouping-glob")?;
        let root = scratch.path();
        legacy_project(root)?;
        let members = std::fs::read_to_string(root.join("Cargo.toml"))?
            .replace("\"tasks/*\"]", "\"tasks/*\", \".rituals/private/lint\"]");
        write_files(
            root,
            &[
                ("Cargo.toml", &members),
                (".rituals/private/lint/Cargo.toml", &package("lint", true)),
                (".rituals/private/lint/src/lib.rs", "//! A fixture.\n"),
            ],
        )?;

        let failure = applied(root)?
            .err()
            .ok_or("a glob that matches a grouping directory must fail the check")?;

        assert!(
            failure.ends_with("ritual put the project back as it found it"),
            "{failure}"
        );
        assert!(
            failure.contains(".rituals/private"),
            "Cargo's own words name the directory: {failure}"
        );
        assert!(root.join("tasks/greet/Cargo.toml").is_file());
        assert!(root.join(".rituals/private/lint/Cargo.toml").is_file());
        assert!(!root.join(".rituals/greet").exists());
        Ok(())
    }

    /// A glob that does not lead with `tasks/` is the person's own and is left
    /// alone, so it may stop matching a task once the task has moved. Another
    /// crate still matches the glob, so Cargo loads the workspace, and nothing
    /// depends on `orphan`, so it is no longer a member: the check after the
    /// move names it and the move is undone.
    #[test]
    fn a_member_the_move_leaves_unreachable_is_named_and_the_project_put_back() -> TestOutcome {
        let scratch = ScratchDir::resolved("step1-lost-member")?;
        let root = scratch.path();
        legacy_project(root)?;
        write_files(
            root,
            &[
                (
                    "Cargo.toml",
                    "[workspace]\nmembers = [\"ritual\", \"tasks/greet\", \"tasks/shout\", \
                     \"t*/orphan\"]\nresolver = \"3\"\n\n[workspace.dependencies]\n\
                     rituals = { path = \"vendor/rituals\" }\n",
                ),
                (
                    "tasks/orphan/Cargo.toml",
                    "[package]\nname = \"orphan\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
                     [dependencies]\nrituals.workspace = true\n\n\
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
        let scratch = ScratchDir::resolved("step1-holds")?;
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
