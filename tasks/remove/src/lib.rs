//! The `remove` task: take a task out of this project, in the order that
//! keeps it building.

mod git;
mod removal;
#[cfg(test)]
mod test_support;

use std::path::{Path, PathBuf};

use git::{Flag, Obstacle};
use removal::{Manifests, Member, Removal};
use rituals::{CommandLine, Failure, Outcome, Task, clap};
use rituals_compose::metadata::{self, Metadata, TaskImport};
use rituals_compose::sentence::join_with_and;
use rituals_compose::{cargo_config, top_level, workspace};

/// The package every composed command line's own commands come from: the
/// bundle of `add`, `regenerate` and the rest, which nothing could put back
/// once it is gone.
const BUNDLE_PACKAGE: &str = "rituals-core";

/// `remove`'s one argument: which task to take out.
#[derive(clap::Args)]
struct RemoveArguments {
    // Not documented with `///`: clap would print the brackets of the table
    // name as written, and rustdoc would read them as a link.
    #[arg(
        value_name = "KEY|CRATE",
        help = "the task's key in [package.metadata.ritual] tasks, or the crate it imports"
    )]
    name: String,
}

/// This task, for a command line to mount under whatever name imports it.
///
/// Reads the composed CLI's command line it is invoked with, the same
/// opt-in any task can use, to find its own project and to regenerate with.
//
// No `# Examples` section: a `task()` function has exactly one call-site
// shape — a mount line in a generated file — and an example here could only
// restate `let task = remove::task();`, which shows nothing a reader needs.
#[must_use]
pub fn task() -> Task {
    Task::receiving_command_line(
        "take a task out of this project, in the order that keeps it building",
        |command_line: &CommandLine, arguments: RemoveArguments| run(command_line, &arguments),
    )
}

fn run(command_line: &CommandLine, arguments: &RemoveArguments) -> Outcome {
    let current_dir = std::env::current_dir()
        .map_err(|error| Failure::new("reading the current directory failed").caused_by(error))?;
    // Asked before anything runs that could write it: `cargo metadata`
    // creates or rewrites a missing or stale lockfile, and a refusal has to
    // be able to put it back.
    let lockfile = workspace::lockfile(&current_dir)?;
    removal::finish(command_line, &arguments.name, |changes| {
        let document =
            changes.run_changing(&[lockfile.as_path()], || metadata::fetch(&current_dir))?;
        prepare(command_line, arguments, &document, &current_dir, lockfile)
    })
}

/// Reads the project and decides what `remove` would write, refusing when
/// that would be unsafe. Runs inside the rollback, after the one read that
/// can write, the `cargo metadata` that fetched `document`, so a refusal
/// puts `lockfile` back as it was; everything here only reads.
///
/// The order is from the cheapest and most specific question to the widest:
/// which task the argument names, whether it is the one task that cannot be
/// removed, whether the rest of the list still resolves without it, and,
/// only for a task whose directory would be deleted, whether anything would
/// still read that directory once it is gone and whether git can give its
/// files back. `current_dir` is where the person ran `remove`, one of the
/// places Cargo reads its configuration from.
fn prepare(
    command_line: &CommandLine,
    arguments: &RemoveArguments,
    document: &Metadata,
    current_dir: &Path,
    lockfile: PathBuf,
) -> Result<Removal, Failure> {
    let package = command_line.identity().package_name();
    document.ensure_runs_in_its_own_project(package, "remove", &arguments.name)?;

    let imports = document.task_imports(package)?;
    let remove_command = top_level::management_command(command_line, "remove");
    let import = pick(
        &arguments.name,
        package,
        &remove_command,
        &imports,
        |task| (task.key(), task.package_name()),
    )?;
    if import.package_name() == Some(BUNDLE_PACKAGE) {
        return Err(bundle_refusal(import.key()));
    }

    // The same resolver `regenerate` runs, with this key left out, against
    // the metadata already fetched. Its refusal is returned as-is: it says
    // what is wrong and what to do.
    document.resolve_task_list_excluding(package, import.key())?;

    let project = document.locate_project(package)?;
    let workspace_root = project.workspace_root().to_path_buf();
    let manifests = Manifests::read(project.manifest_path(), &workspace_root.join("Cargo.toml"))?;

    // Dropped only when nothing else depends on the crate: another package's
    // own dependency on it would otherwise stop inheriting.
    let drops_inherited_entry = manifests.cli().inherits_workspace_dependency(import.key())
        && import.other_dependents().is_empty();

    let member = match (import.directory(), import.is_workspace_member()) {
        (Some(directory), true) => Some(plan_member(
            command_line,
            &Deletion {
                document,
                import,
                directory,
                workspace_root: &workspace_root,
                current_dir,
                manifests: &manifests,
                dropped_workspace_dependency: drops_inherited_entry.then(|| import.key()),
            },
            &format!("{remove_command} {}", arguments.name),
        )?),
        (Some(_) | None, _) => None,
    };

    Ok(Removal {
        key: import.key().to_string(),
        has_dependency: import.package_name().is_some(),
        drops_inherited_entry,
        manifests,
        workspace_root,
        lockfile,
        member,
    })
}

/// Picks the task `argument` names from `candidates`, each seen as its key
/// and the package it imports.
///
/// A key wins over a crate of the same name. Without a way to say which is
/// meant, refusing the name as ambiguous would make that key impossible to
/// remove, while the other import is always reachable by its own key.
fn pick<'a, T>(
    argument: &str,
    package: &str,
    remove_command: &str,
    candidates: &'a [T],
    view: impl Fn(&'a T) -> (&'a str, Option<&'a str>),
) -> Result<&'a T, Failure> {
    if let Some(keyed) = candidates
        .iter()
        .find(|candidate| view(candidate).0 == argument)
    {
        return Ok(keyed);
    }

    let importing: Vec<&'a T> = candidates
        .iter()
        .filter(|candidate| view(candidate).1 == Some(argument))
        .collect();
    match importing.as_slice() {
        [only] => Ok(only),
        [] => Err(neither_refusal(
            argument,
            package,
            &candidates
                .iter()
                .map(|candidate| view(candidate).0)
                .collect::<Vec<_>>(),
        )),
        several @ [first, ..] => Err(several_keys_refusal(
            argument,
            view(first).0,
            &several
                .iter()
                .map(|candidate| view(candidate).0)
                .collect::<Vec<_>>(),
            remove_command,
        )),
    }
}

/// What [`plan_member`] reads to decide whether a member's directory can be
/// deleted.
struct Deletion<'a> {
    document: &'a Metadata,
    import: &'a TaskImport<'a>,
    /// The directory, as `cargo metadata` gives it.
    directory: &'a Path,
    workspace_root: &'a Path,
    /// Where the person ran `remove`.
    current_dir: &'a Path,
    manifests: &'a Manifests,
    /// The `[workspace.dependencies]` key `remove` takes out with the
    /// dependency, if it takes one out.
    dropped_workspace_dependency: Option<&'a str>,
}

/// Decides whether a workspace member's directory can be deleted, and
/// returns it if it can.
///
/// Every refusal here is about the project once the directory is gone, and
/// is decided while it is still there: what would still read a path into it,
/// whether it is the project's to delete, and whether git can give it back.
/// All are reads. Cargo reads a path in many places, and each is asked
/// about: what every package declares and builds from, the workspace's
/// member lists, `[patch]`, `[replace]` and `[workspace.dependencies]`, and
/// Cargo's own configuration files.
fn plan_member(
    command_line: &CommandLine,
    deletion: &Deletion<'_>,
    retry_command: &str,
) -> Result<Member, Failure> {
    let Deletion {
        document,
        import,
        directory,
        workspace_root,
        current_dir,
        manifests,
        dropped_workspace_dependency,
    } = *deletion;
    let shown = shown(directory, workspace_root);

    let mut dependents: Vec<String> = import
        .other_dependents()
        .iter()
        .map(ToString::to_string)
        .collect();
    dependents.extend(document.dependents_outside_the_graph(directory)?);
    if !dependents.is_empty() {
        return Err(other_dependents_refusal(import.key(), &shown, &dependents));
    }
    if !import.members_inside().is_empty() {
        return Err(members_inside_refusal(&shown, import.members_inside()));
    }
    if !is_strictly_inside(directory, workspace_root)? {
        return Err(outside_the_workspace_refusal(&shown));
    }

    let workspace = manifests.workspace();
    if workspace.empties_default_members(directory) {
        return Err(last_default_member_refusal(&shown));
    }
    let globs = workspace.globs_left_matching_nothing(directory)?;
    if !globs.is_empty() {
        return Err(globs_refusal(&shown, &globs));
    }
    let mut entries = workspace.entries_pointing_under(directory, dropped_workspace_dependency);
    // A build reads the configuration above wherever it starts, and the
    // project's own command line can be run from any member's directory.
    let mut starts = vec![current_dir, workspace_root];
    starts.extend(document.member_directories());
    entries.extend(cargo_config::entries_pointing_under(&starts, directory)?);
    if !entries.is_empty() {
        return Err(entries_refusal(&shown, &entries));
    }

    let from_top_level =
        git::ensure_git_can_give_back(directory, workspace_root).map_err(|obstacle| {
            git_refusal(
                obstacle,
                import.key(),
                &shown,
                retry_command,
                &top_level::management_command(command_line, "regenerate"),
            )
        })?;

    Ok(Member {
        directory: directory.to_path_buf(),
        relative: shown,
        from_top_level,
    })
}

/// Whether `directory` lies strictly inside `workspace_root` once both are
/// resolved through symbolic links: a directory reached through a link that
/// leads out of the project is not the project's to delete, however Cargo
/// spells it.
fn is_strictly_inside(directory: &Path, workspace_root: &Path) -> Result<bool, Failure> {
    let resolved = |path: &Path| {
        std::fs::canonicalize(path).map_err(|error| {
            Failure::new(format!("reading {} failed", path.display())).caused_by(error)
        })
    };
    let directory = resolved(directory)?;
    let workspace_root = resolved(workspace_root)?;
    Ok(directory
        .strip_prefix(&workspace_root)
        .is_ok_and(|relative| !relative.as_os_str().is_empty()))
}

/// `directory` as the person knows it: from the workspace root when it is
/// inside it, as it is otherwise.
fn shown(directory: &Path, workspace_root: &Path) -> String {
    directory
        .strip_prefix(workspace_root)
        .unwrap_or(directory)
        .display()
        .to_string()
}

/// Wraps each name in backticks, ready for
/// [`join_with_and`](rituals_compose::sentence::join_with_and).
fn backticked(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| format!("`{name}`")).collect()
}

/// The refusal for an argument that is neither a key nor a crate any key
/// imports.
fn neither_refusal(argument: &str, package: &str, keys: &[&str]) -> Failure {
    let tasks = if keys.is_empty() {
        "it has no tasks".to_string()
    } else {
        format!("its tasks are {}", join_with_and(&backticked(keys)))
    };
    Failure::new(format!(
        "refusing to remove `{argument}`: it is neither a key in `{package}`'s \
         [package.metadata.ritual] tasks nor a crate one of them imports; {tasks}"
    ))
}

/// The refusal for a crate that more than one key imports, so the argument
/// does not say which to remove. `example_key` is the one the remedy shows.
fn several_keys_refusal(
    argument: &str,
    example_key: &str,
    keys: &[&str],
    remove_command: &str,
) -> Failure {
    Failure::new(format!(
        "refusing to remove `{argument}`: it is imported by more than one task, {}; remove one \
         by its key, such as `{remove_command} {example_key}`",
        join_with_and(&backticked(keys))
    ))
}

/// The refusal for the key that imports ritual's own commands.
fn bundle_refusal(key: &str) -> Failure {
    Failure::new(format!(
        "refusing to remove `{key}`: it imports `{BUNDLE_PACKAGE}`, ritual's own commands, and \
         without them nothing can put it back"
    ))
}

/// The refusal for a directory another package also depends on or builds
/// from.
fn other_dependents_refusal(key: &str, directory: &str, dependents: &[String]) -> Failure {
    let names: Vec<&str> = dependents.iter().map(String::as_str).collect();
    Failure::new(format!(
        "refusing to remove `{key}`: {directory} is also used by {}, and deleting it would \
         break {}; remove that dependency first",
        join_with_and(&backticked(&names)),
        if dependents.len() == 1 {
            "that crate"
        } else {
            "those crates"
        }
    ))
}

/// The refusal for a directory that holds other workspace members.
fn members_inside_refusal(directory: &str, members: &[&str]) -> Failure {
    Failure::new(format!(
        "refusing to delete {directory}: it also holds the workspace members {}, which \
         deleting it would take too; remove or move them first",
        join_with_and(&backticked(members))
    ))
}

/// The refusal for a directory that is not strictly inside the workspace.
fn outside_the_workspace_refusal(directory: &str) -> Failure {
    Failure::new(format!(
        "refusing to delete {directory}: it is not inside the workspace root, so it is not \
         this project's to delete; remove its dependency and its workspace member entry by hand"
    ))
}

/// The refusal for a directory whose `members` entry is the only one in
/// `default-members`.
fn last_default_member_refusal(directory: &str) -> Failure {
    Failure::new(format!(
        "refusing to delete {directory}: it is the only entry in [workspace] default-members, \
         and taking it out would leave that list empty; change default-members first"
    ))
}

/// The refusal for a directory whose deletion would leave globs in
/// `[workspace] members` or `default-members` matching nothing.
fn globs_refusal(directory: &str, globs: &[String]) -> Failure {
    let names: Vec<&str> = globs.iter().map(String::as_str).collect();
    let (globs, them) = if names.len() == 1 {
        ("glob", "it")
    } else {
        ("globs", "them")
    };
    Failure::new(format!(
        "refusing to delete {directory}: it is the last match of the [workspace] {globs} {}, \
         which would then match nothing, and Cargo reads a glob that matches nothing as a \
         path that does not exist; add an explicit member, or remove {them}, first",
        join_with_and(&backticked(&names))
    ))
}

/// The refusal for a directory that `[patch]`, `[replace]`,
/// `[workspace.dependencies]` or Cargo's configuration still points into.
fn entries_refusal(directory: &str, entries: &[String]) -> Failure {
    Failure::new(format!(
        "refusing to delete {directory}: Cargo would still read it through {}, used or not; \
         remove or repoint {} first",
        join_with_and(entries),
        if entries.len() == 1 { "it" } else { "them" }
    ))
}

/// Each path spelled from git's top level with the `:/` pathspec magic,
/// which git reads from there whatever directory it is run in.
fn from_the_top_level<'a>(paths: impl IntoIterator<Item = &'a PathBuf>) -> String {
    paths
        .into_iter()
        .map(|path| format!(":/{}", path.display()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The refusal for a directory git could not be asked about, or could not
/// vouch for.
///
/// `retry_command` is what a person runs again, as they would type it;
/// `regenerate_command` is how they would type `regenerate`.
fn git_refusal(
    obstacle: Obstacle,
    key: &str,
    directory: &str,
    retry_command: &str,
    regenerate_command: &str,
) -> Failure {
    let by_hand = format!(
        "take `{key}` out by hand — drop it from [package.metadata.ritual] tasks, run \
         `{regenerate_command}`, remove its dependency and workspace member entry, then \
         delete {directory} by hand"
    );
    match obstacle {
        Obstacle::NotARepository => Failure::new(format!(
            "refusing to remove `{key}`: this project is not a git repository, so nothing \
             could give {directory} back once it is deleted; {by_hand}"
        )),
        Obstacle::GitMissing => Failure::new(format!(
            "refusing to remove `{key}`: `git` could not be run, and remove needs it to check \
             that {directory} can be given back; {by_hand}"
        )),
        Obstacle::OwnRepository(repository) => {
            // Joining an empty path would add a trailing separator.
            let repository = if repository.as_os_str().is_empty() {
                directory.to_string()
            } else {
                Path::new(directory).join(repository).display().to_string()
            };
            Failure::new(format!(
                "refusing to remove `{key}`: {repository} is a git repository of its own, so \
                 this project's git has no record of what is in it; {by_hand}"
            ))
        }
        Obstacle::OtherRepository(repository) => Failure::new(format!(
            "refusing to remove `{key}`: {directory} is in the git repository at {}, not this \
             project's, so this project's git has no record of it; {by_hand}",
            repository.display()
        )),
        Obstacle::Failed(message) => Failure::new(format!(
            "refusing to remove `{key}`: git could not say whether {directory} can be given \
             back: {message}"
        )),
        Obstacle::Unwatched(files) => {
            let named: Vec<String> = files
                .iter()
                .map(|file| {
                    let flag = match file.flag {
                        Flag::AssumeUnchanged => "assume-unchanged",
                        Flag::SkipWorktree => "skip-worktree",
                    };
                    format!("{} ({flag})", from_the_top_level([&file.path]))
                })
                .collect();
            Failure::new(format!(
                "refusing to remove `{key}`: git has been told not to look at changes to {}, so \
                 it cannot vouch for them; clear the flag with `git update-index \
                 --no-assume-unchanged` or `--no-skip-worktree`, commit what changed, then run \
                 `{retry_command}` again",
                named.join(", ")
            ))
        }
        Obstacle::Filtered(files) => {
            let named: Vec<String> = files
                .iter()
                .map(|(path, filter)| format!("{} (filter `{filter}`)", from_the_top_level([path])))
                .collect();
            Failure::new(format!(
                "refusing to remove `{key}`: git stores {} through a filter, so what it would \
                 give back can differ from what is on disk; {by_hand}",
                named.join(", ")
            ))
        }
        Obstacle::Dirty(files) => Failure::new(format!(
            "refusing to remove `{key}`: {directory} has files git cannot give back — {}; \
             commit or delete them, then run `{retry_command}` again",
            from_the_top_level(&files)
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        Flag, Obstacle, bundle_refusal, entries_refusal, git_refusal, globs_refusal,
        is_strictly_inside, last_default_member_refusal, members_inside_refusal, neither_refusal,
        other_dependents_refusal, outside_the_workspace_refusal, pick, several_keys_refusal, shown,
    };
    use crate::git::Unwatched;
    use crate::test_support::{ScratchDir, TestOutcome};

    const REMOVE: &str = "cargo ritual remove";

    /// A task as `pick` sees it: its key and the package it imports.
    type Candidate<'a> = (&'a str, Option<&'a str>);

    fn view<'a>(candidate: &'a Candidate<'a>) -> (&'a str, Option<&'a str>) {
        *candidate
    }

    fn picked_key<'a>(argument: &str, candidates: &'a [Candidate<'a>]) -> Result<&'a str, String> {
        pick(argument, "demo-ritual", REMOVE, candidates, view)
            .map(|candidate| candidate.0)
            .map_err(|failure| failure.to_string())
    }

    #[test]
    fn a_key_picks_its_own_task() {
        let tasks = [("ritual", Some("rituals-core")), ("greet", Some("greet"))];
        assert_eq!(picked_key("greet", &tasks), Ok("greet"));
    }

    #[test]
    fn a_crate_picks_the_one_key_that_imports_it() {
        let tasks = [("ritual", Some("rituals-core")), ("hi", Some("greeter"))];
        assert_eq!(picked_key("greeter", &tasks), Ok("hi"));
    }

    /// `greet` is a key, and the crate `greet` is also what `other` imports:
    /// the key wins, and `other` stays reachable by its own key.
    #[test]
    fn a_key_wins_over_a_crate_of_the_same_name() {
        let tasks = [("greet", Some("hello")), ("other", Some("greet"))];
        assert_eq!(picked_key("greet", &tasks), Ok("greet"));
        assert_eq!(picked_key("other", &tasks), Ok("other"));
    }

    #[test]
    fn a_key_with_no_dependency_is_still_a_key() {
        let tasks = [("dangling", None)];
        assert_eq!(picked_key("dangling", &tasks), Ok("dangling"));
    }

    #[test]
    fn a_name_that_is_neither_is_refused_with_the_tasks_there_are() {
        let tasks = [("ritual", Some("rituals-core")), ("greet", Some("greet"))];
        assert_eq!(
            picked_key("nosuch", &tasks),
            Err(
                "refusing to remove `nosuch`: it is neither a key in `demo-ritual`'s \
                 [package.metadata.ritual] tasks nor a crate one of them imports; its tasks \
                 are `ritual` and `greet`"
                    .to_string()
            )
        );
    }

    #[test]
    fn a_name_is_refused_plainly_when_there_are_no_tasks() {
        assert_eq!(
            neither_refusal("nosuch", "demo-ritual", &[]).to_string(),
            "refusing to remove `nosuch`: it is neither a key in `demo-ritual`'s \
             [package.metadata.ritual] tasks nor a crate one of them imports; it has no tasks"
        );
    }

    /// A crate that is not a key and is imported by two: the refusal names
    /// both keys and shows how to remove one, spelled the way this command
    /// line is typed.
    #[test]
    fn a_crate_imported_by_two_keys_is_refused_naming_both() {
        let tasks = [("alpha", Some("shared")), ("beta", Some("shared"))];
        assert_eq!(
            picked_key("shared", &tasks),
            Err(
                "refusing to remove `shared`: it is imported by more than one task, `alpha` \
                 and `beta`; remove one by its key, such as `cargo ritual remove alpha`"
                    .to_string()
            )
        );
    }

    #[test]
    fn a_crate_imported_by_three_keys_names_all_three() {
        assert_eq!(
            several_keys_refusal("shared", "a", &["a", "b", "c"], REMOVE).to_string(),
            "refusing to remove `shared`: it is imported by more than one task, `a`, `b` and \
             `c`; remove one by its key, such as `cargo ritual remove a`"
        );
    }

    #[test]
    fn the_bundle_is_refused_whatever_its_key() {
        assert_eq!(
            bundle_refusal("tools").to_string(),
            "refusing to remove `tools`: it imports `rituals-core`, ritual's own commands, and \
             without them nothing can put it back"
        );
    }

    #[test]
    fn a_crate_another_package_depends_on_is_refused_naming_it() {
        assert_eq!(
            other_dependents_refusal("greet", "tasks/greet", &["worker".to_string()]).to_string(),
            "refusing to remove `greet`: tasks/greet is also used by `worker`, and deleting it \
             would break that crate; remove that dependency first"
        );
        assert_eq!(
            other_dependents_refusal("greet", "tasks/greet", &["a".to_string(), "b".to_string()])
                .to_string(),
            "refusing to remove `greet`: tasks/greet is also used by `a` and `b`, and deleting \
             it would break those crates; remove that dependency first"
        );
    }

    #[test]
    fn a_glob_left_matching_nothing_is_refused_naming_it() {
        assert_eq!(
            globs_refusal("tasks/greet", &["tasks/*".to_string()]).to_string(),
            "refusing to delete tasks/greet: it is the last match of the [workspace] glob \
             `tasks/*`, which would then match nothing, and Cargo reads a glob that matches \
             nothing as a path that does not exist; add an explicit member, or remove it, first"
        );
        assert!(
            globs_refusal(
                "tasks/greet",
                &["tasks/*".to_string(), "tasks/g*".to_string()]
            )
            .to_string()
            .contains("globs `tasks/*` and `tasks/g*`, which would then match nothing")
        );
    }

    #[test]
    fn entries_cargo_would_still_read_are_refused_naming_them() {
        assert_eq!(
            entries_refusal(
                "tasks/greet",
                &[
                    "[patch.crates-io] greet".to_string(),
                    "`paths` entry `tasks/greet` in /w/.cargo/config.toml".to_string()
                ]
            )
            .to_string(),
            "refusing to delete tasks/greet: Cargo would still read it through [patch.crates-io] \
             greet and `paths` entry `tasks/greet` in /w/.cargo/config.toml, used or not; \
             remove or repoint them first"
        );
    }

    /// A directory reached through a symbolic link that leads out of the
    /// workspace is outside it, however the path is spelled.
    #[test]
    fn a_directory_behind_a_link_out_of_the_workspace_is_not_inside_it() -> TestOutcome {
        let scratch = ScratchDir::new("strictly-inside")?;
        let workspace_root = scratch.path().join("project");
        let elsewhere = scratch.path().join("elsewhere");
        std::fs::create_dir_all(workspace_root.join("tasks/greet"))?;
        std::fs::create_dir_all(elsewhere.join("greet"))?;
        std::os::unix::fs::symlink(&elsewhere, workspace_root.join("linked"))?;

        assert!(is_strictly_inside(
            &workspace_root.join("tasks/greet"),
            &workspace_root
        )?);
        assert!(!is_strictly_inside(
            &workspace_root.join("linked/greet"),
            &workspace_root
        )?);
        assert!(!is_strictly_inside(&workspace_root, &workspace_root)?);
        Ok(())
    }

    #[test]
    fn a_directory_holding_members_is_refused_naming_them() {
        assert_eq!(
            members_inside_refusal("tasks/greet", &["inner"]).to_string(),
            "refusing to delete tasks/greet: it also holds the workspace members `inner`, which \
             deleting it would take too; remove or move them first"
        );
    }

    #[test]
    fn a_directory_outside_the_workspace_is_refused() {
        assert_eq!(
            outside_the_workspace_refusal("/elsewhere/greet").to_string(),
            "refusing to delete /elsewhere/greet: it is not inside the workspace root, so it \
             is not this project's to delete; remove its dependency and its workspace member \
             entry by hand"
        );
    }

    #[test]
    fn the_last_default_member_is_refused() {
        assert_eq!(
            last_default_member_refusal("tasks/greet").to_string(),
            "refusing to delete tasks/greet: it is the only entry in [workspace] \
             default-members, and taking it out would leave that list empty; change \
             default-members first"
        );
    }

    #[test]
    fn a_directory_is_shown_from_the_workspace_root_when_it_is_inside() {
        assert_eq!(
            shown(
                &PathBuf::from("/project/tasks/greet"),
                &PathBuf::from("/project")
            ),
            "tasks/greet"
        );
        assert_eq!(
            shown(
                &PathBuf::from("/elsewhere/greet"),
                &PathBuf::from("/project")
            ),
            "/elsewhere/greet"
        );
    }

    fn refusal(obstacle: Obstacle) -> String {
        git_refusal(
            obstacle,
            "greet",
            "tasks/greet",
            "cargo ritual remove greet",
            "cargo ritual regenerate",
        )
        .to_string()
    }

    const BY_HAND: &str = "take `greet` out by hand — drop it from [package.metadata.ritual] \
                           tasks, run `cargo ritual regenerate`, remove its dependency and \
                           workspace member entry, then delete tasks/greet by hand";

    /// Named from git's top level with the `:/` magic, so each can be handed
    /// to git from any directory of the project.
    #[test]
    fn dirty_files_are_each_named_from_gits_top_level() {
        assert_eq!(
            refusal(Obstacle::Dirty(vec![
                PathBuf::from("tasks/greet/scratch-notes.txt"),
                PathBuf::from("tasks/greet/src/lib.rs"),
            ])),
            "refusing to remove `greet`: tasks/greet has files git cannot give back — \
             :/tasks/greet/scratch-notes.txt, :/tasks/greet/src/lib.rs; commit or delete them, \
             then run `cargo ritual remove greet` again"
        );
    }

    #[test]
    fn files_git_does_not_look_at_are_each_named_with_their_flag() {
        assert_eq!(
            refusal(Obstacle::Unwatched(vec![
                Unwatched {
                    path: PathBuf::from("tasks/greet/local.toml"),
                    flag: Flag::AssumeUnchanged,
                },
                Unwatched {
                    path: PathBuf::from("tasks/greet/src/lib.rs"),
                    flag: Flag::SkipWorktree,
                },
            ])),
            "refusing to remove `greet`: git has been told not to look at changes to \
             :/tasks/greet/local.toml (assume-unchanged), :/tasks/greet/src/lib.rs \
             (skip-worktree), so it cannot vouch for them; clear the flag with `git \
             update-index --no-assume-unchanged` or `--no-skip-worktree`, commit what changed, \
             then run `cargo ritual remove greet` again"
        );
    }

    #[test]
    fn files_behind_a_filter_are_each_named_with_it() {
        assert_eq!(
            refusal(Obstacle::Filtered(vec![(
                PathBuf::from("tasks/greet/local.cfg"),
                "strip".to_string()
            )])),
            format!(
                "refusing to remove `greet`: git stores :/tasks/greet/local.cfg (filter \
                 `strip`) through a filter, so what it would give back can differ from what is \
                 on disk; {BY_HAND}"
            )
        );
    }

    #[test]
    fn a_directory_in_another_repository_is_told_to_delete_by_hand() {
        assert_eq!(
            refusal(Obstacle::OtherRepository(PathBuf::from("/elsewhere"))),
            format!(
                "refusing to remove `greet`: tasks/greet is in the git repository at /elsewhere, \
                 not this project's, so this project's git has no record of it; {BY_HAND}"
            )
        );
    }

    #[test]
    fn a_project_that_is_not_a_repository_is_told_to_delete_by_hand() {
        assert_eq!(
            refusal(Obstacle::NotARepository),
            format!(
                "refusing to remove `greet`: this project is not a git repository, so nothing \
                 could give tasks/greet back once it is deleted; {BY_HAND}"
            )
        );
    }

    #[test]
    fn a_missing_git_is_refused_like_a_missing_repository() {
        assert_eq!(
            refusal(Obstacle::GitMissing),
            format!(
                "refusing to remove `greet`: `git` could not be run, and remove needs it to \
                 check that tasks/greet can be given back; {BY_HAND}"
            )
        );
    }

    #[test]
    fn a_directory_that_is_its_own_repository_is_told_to_delete_by_hand() {
        assert_eq!(
            refusal(Obstacle::OwnRepository(PathBuf::new())),
            format!(
                "refusing to remove `greet`: tasks/greet is a git repository of its own, so \
                 this project's git has no record of what is in it; {BY_HAND}"
            )
        );
    }

    /// A submodule inside the directory is named where it is, from the
    /// workspace root.
    #[test]
    fn a_repository_inside_the_directory_is_named_where_it_is() {
        assert_eq!(
            refusal(Obstacle::OwnRepository(PathBuf::from("vendor/upstream"))),
            format!(
                "refusing to remove `greet`: tasks/greet/vendor/upstream is a git repository of \
                 its own, so this project's git has no record of what is in it; {BY_HAND}"
            )
        );
    }

    #[test]
    fn any_other_git_failure_is_passed_through_in_gits_words() {
        assert_eq!(
            refusal(Obstacle::Failed(
                "fatal: detected dubious ownership".to_string()
            )),
            "refusing to remove `greet`: git could not say whether tasks/greet can be given \
             back: fatal: detected dubious ownership"
        );
    }
}
