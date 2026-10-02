//! The `remove` task: take a task out of this project, in the order that
//! keeps it building.

mod git;
mod removal;
#[cfg(test)]
mod test_support;

use std::path::{Path, PathBuf};

use git::Obstacle;
use removal::{Manifests, Member, Removal};
use rituals::{CommandLine, Failure, Outcome, Task, clap};
use rituals_compose::metadata::{self, TaskImport};
use rituals_compose::sentence::join_with_and;
use rituals_compose::top_level;

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
    let removal = prepare(command_line, arguments)?;
    removal::finish(command_line, removal)
}

/// Reads the project and decides what `remove` would write, refusing when
/// that would be unsafe. Nothing is written until every refusal here has
/// passed.
///
/// The order is from the cheapest and most specific question to the widest:
/// which task the argument names, whether it is the one task that cannot be
/// removed, whether the rest of the list still resolves without it, and,
/// only for a task whose directory would be deleted, whether anything else
/// depends on that directory and whether git can give its files back.
fn prepare(command_line: &CommandLine, arguments: &RemoveArguments) -> Result<Removal, Failure> {
    let package = command_line.identity().package_name();
    let current_dir = std::env::current_dir()
        .map_err(|error| Failure::new("reading the current directory failed").caused_by(error))?;
    let document = metadata::fetch(&current_dir)?;
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
    // what is wrong and what to do, and nothing has been written for it to
    // undo.
    document.resolve_task_list_excluding(package, import.key())?;

    let project = document.locate_project(package)?;
    let workspace_root = project.workspace_root().to_path_buf();
    let manifests = Manifests::read(project.manifest_path(), &workspace_root.join("Cargo.toml"))?;

    let member = match (import.directory(), import.is_workspace_member()) {
        (Some(directory), true) => Some(plan_member(
            command_line,
            import,
            directory,
            &workspace_root,
            &manifests,
            &format!("{remove_command} {}", arguments.name),
        )?),
        (Some(_) | None, _) => None,
    };

    Ok(Removal {
        key: import.key().to_string(),
        argument: arguments.name.clone(),
        has_dependency: import.package_name().is_some(),
        // Dropped only when nothing else in the workspace depends on the
        // crate: another package's own dependency on it would otherwise stop
        // inheriting.
        drops_inherited_entry: manifests.cli().inherits_workspace_dependency(import.key())
            && import.other_dependents().is_empty(),
        manifests,
        workspace_root,
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

/// Decides whether a workspace member's directory can be deleted, and
/// returns it if it can.
///
/// Every refusal here is about the deletion: what else would break, whether
/// the directory is the project's to delete, and whether git can give it
/// back. All are reads.
fn plan_member(
    command_line: &CommandLine,
    import: &TaskImport<'_>,
    directory: &Path,
    workspace_root: &Path,
    manifests: &Manifests,
    retry_command: &str,
) -> Result<Member, Failure> {
    let shown = shown(directory, workspace_root);

    if !import.other_dependents().is_empty() {
        return Err(other_dependents_refusal(
            import.key(),
            &shown,
            import.other_dependents(),
        ));
    }
    if !import.members_inside().is_empty() {
        return Err(members_inside_refusal(&shown, import.members_inside()));
    }
    let relative = match directory.strip_prefix(workspace_root) {
        Ok(relative) if !relative.as_os_str().is_empty() => relative.to_string_lossy().into_owned(),
        Ok(_) | Err(_) => return Err(outside_the_workspace_refusal(&shown)),
    };
    if manifests.workspace().empties_default_members(&relative) {
        return Err(last_default_member_refusal(&shown));
    }

    git::ensure_git_can_give_back(directory).map_err(|obstacle| {
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
        relative,
    })
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

/// The refusal for a crate another workspace package also depends on.
fn other_dependents_refusal(key: &str, directory: &str, dependents: &[&str]) -> Failure {
    Failure::new(format!(
        "refusing to remove `{key}`: {directory} is also a dependency of {}, and deleting it \
         would break {}; remove that dependency first",
        join_with_and(&backticked(dependents)),
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
        Obstacle::OwnRepository => Failure::new(format!(
            "refusing to remove `{key}`: {directory} is a git repository of its own, so this \
             project's git has no record of what is in it; {by_hand}"
        )),
        Obstacle::Failed(message) => Failure::new(format!(
            "refusing to remove `{key}`: git could not say whether {directory} can be given \
             back: {message}"
        )),
        Obstacle::Dirty(files) => {
            let named: Vec<String> = files
                .iter()
                .map(|file| PathBuf::from(directory).join(file).display().to_string())
                .collect();
            Failure::new(format!(
                "refusing to remove `{key}`: {directory} has files git cannot give back — {}; \
                 commit or delete them, then run `{retry_command}` again",
                named.join(", ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        Obstacle, bundle_refusal, git_refusal, last_default_member_refusal, members_inside_refusal,
        neither_refusal, other_dependents_refusal, outside_the_workspace_refusal, pick,
        several_keys_refusal, shown,
    };

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
            other_dependents_refusal("greet", "tasks/greet", &["worker"]).to_string(),
            "refusing to remove `greet`: tasks/greet is also a dependency of `worker`, and \
             deleting it would break that crate; remove that dependency first"
        );
        assert_eq!(
            other_dependents_refusal("greet", "tasks/greet", &["a", "b"]).to_string(),
            "refusing to remove `greet`: tasks/greet is also a dependency of `a` and `b`, and \
             deleting it would break those crates; remove that dependency first"
        );
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

    #[test]
    fn dirty_files_are_each_named_from_the_workspace_root() {
        assert_eq!(
            refusal(Obstacle::Dirty(vec![
                PathBuf::from("scratch-notes.txt"),
                PathBuf::from("src/lib.rs"),
            ])),
            "refusing to remove `greet`: tasks/greet has files git cannot give back — \
             tasks/greet/scratch-notes.txt, tasks/greet/src/lib.rs; commit or delete them, \
             then run `cargo ritual remove greet` again"
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
            refusal(Obstacle::OwnRepository),
            format!(
                "refusing to remove `greet`: tasks/greet is a git repository of its own, so \
                 this project's git has no record of what is in it; {BY_HAND}"
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
