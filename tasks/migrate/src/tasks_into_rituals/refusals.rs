//! Why the first step refuses, in the words a person reads, and the checks
//! that decide it.
//!
//! Every refusal about one task's directory opens with `refusing to move`
//! and the directory, so the person learns which task is in the way before
//! why, and ends with what to do and the command to run again.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use rituals::{Failure, Outcome};
use rituals_compose::git::{self, Obstacle};
use rituals_compose::sentence::join_with_and;

use crate::places::from_the_root;
use crate::precondition::{self, Repository};

/// A workspace member, as far as the checks here read one.
pub(super) struct Member {
    pub(super) directory: PathBuf,
    pub(super) package: String,
}

/// Refuses when a directory to move holds a workspace member, which moving
/// the directory would take along: what depends on it would not follow.
///
/// `members` is every member of the workspace, the directory's own included.
pub(super) fn ensure_none_holds_other_members(
    directories: &[PathBuf],
    members: &[Member],
    root: &Path,
    migrate_command: &str,
) -> Outcome {
    for directory in directories {
        let inside: Vec<String> = members
            .iter()
            .filter(|member| member.directory != *directory)
            .filter(|member| member.directory.starts_with(directory))
            .map(|member| format!("`{}`", member.package))
            .collect();
        if !inside.is_empty() {
            return Err(holds_members(
                &from_the_root(directory, root),
                &inside,
                migrate_command,
            ));
        }
    }
    Ok(())
}

/// Refuses when the directory the tasks move into exists and is not a
/// directory, or when a task's own destination is taken by anything at all.
///
/// `moves` is each task directory and where it would go. The directory is
/// asked about first: with a file where it should be, every destination is
/// unreachable and none of them is the thing in the way.
pub(super) fn ensure_destinations_are_free(
    moves: &[(PathBuf, PathBuf)],
    to_directory: &Path,
    root: &Path,
    migrate_command: &str,
) -> Outcome {
    let Some((first, _)) = moves.first() else {
        return Ok(());
    };
    // `symlink_metadata` does not follow a link, so a link to nothing is
    // something that is there, as `Changes::rename` also finds it.
    match std::fs::symlink_metadata(to_directory) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(destination_is_not_a_directory(
                &from_the_root(first, root),
                &from_the_root(to_directory, root),
                migrate_command,
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(unreadable(to_directory, error)),
    }
    for (directory, destination) in moves {
        match std::fs::symlink_metadata(destination) {
            Ok(_) => {
                return Err(destination_exists(
                    &from_the_root(directory, root),
                    &from_the_root(destination, root),
                    migrate_command,
                ));
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(unreadable(destination, error)),
        }
    }
    Ok(())
}

/// Refuses when a directory to move is a git submodule or holds one, which
/// git records as a commit and a plain move would leave unrecorded.
pub(super) fn ensure_none_holds_a_submodule(
    directories: &[PathBuf],
    repository: &Repository,
    root: &Path,
    migrate_command: &str,
) -> Outcome {
    for directory in directories {
        let shown = from_the_root(directory, root);
        let submodules = git::submodules_under(directory)
            .map_err(|obstacle| submodule_question_refusal(&obstacle, &shown, migrate_command))?;
        if let Some(gitlink) = submodules.first() {
            return Err(holds_a_submodule(
                &shown,
                &repository.shown(gitlink),
                migrate_command,
            ));
        }
    }
    Ok(())
}

/// The refusal for a git that could not say whether a directory holds a
/// submodule.
fn submodule_question_refusal(
    obstacle: &Obstacle,
    directory: &str,
    migrate_command: &str,
) -> Failure {
    match obstacle {
        Obstacle::GitMissing | Obstacle::NotARepository => {
            precondition::refusal(obstacle, migrate_command)
        }
        Obstacle::Failed(message) => submodule_unknown(directory, message),
        Obstacle::OwnRepository(_)
        | Obstacle::OtherRepository(_)
        | Obstacle::Unwatched(_)
        | Obstacle::Filtered(_)
        | Obstacle::Dirty(_) => unreachable!(
            "git::submodules_under returns only GitMissing, NotARepository and Failed, got \
             {obstacle:?}"
        ),
    }
}

fn unreadable(path: &Path, error: std::io::Error) -> Failure {
    Failure::new(format!("reading {} failed", path.display())).caused_by(error)
}

fn holds_members(directory: &str, members: &[String], migrate_command: &str) -> Failure {
    Failure::new(format!(
        "refusing to move {directory}: it also holds the workspace members {}, which moving it \
         would take too; move them out of it first, then run `{migrate_command}` again",
        join_with_and(members)
    ))
}

fn destination_is_not_a_directory(
    directory: &str,
    to_directory: &str,
    migrate_command: &str,
) -> Failure {
    Failure::new(format!(
        "refusing to move {directory}: {to_directory} already exists and is not a directory; \
         move it out of the way, then run `{migrate_command}` again"
    ))
}

fn destination_exists(directory: &str, destination: &str, migrate_command: &str) -> Failure {
    Failure::new(format!(
        "refusing to move {directory}: {destination} already exists; move it out of the way, \
         then run `{migrate_command}` again"
    ))
}

fn holds_a_submodule(directory: &str, gitlink: &str, migrate_command: &str) -> Failure {
    Failure::new(format!(
        "refusing to move {directory}: {gitlink} is a git submodule, which migrate cannot \
         move; move {directory} with `git mv`, update its members entry and every path to it, \
         then run `{migrate_command}` again"
    ))
}

fn submodule_unknown(directory: &str, git_words: &str) -> Failure {
    Failure::new(format!(
        "refusing to move {directory}: git could not say whether it holds a git submodule: \
         {git_words}"
    ))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        Member, destination_exists, destination_is_not_a_directory, ensure_destinations_are_free,
        ensure_none_holds_a_submodule, ensure_none_holds_other_members, holds_a_submodule,
        holds_members,
    };
    use crate::precondition::WorkTree;
    use crate::test_support::{ScratchDir, TestOutcome, git, init_and_commit, write_files};

    const MIGRATE: &str = "cargo ritual migrate";

    #[test]
    fn a_directory_holding_members_is_refused_naming_them() {
        assert_eq!(
            holds_members("tasks/x", &["`y`".to_string()], MIGRATE).to_string(),
            "refusing to move tasks/x: it also holds the workspace members `y`, which moving \
             it would take too; move them out of it first, then run `cargo ritual migrate` \
             again"
        );
    }

    #[test]
    fn a_destination_that_is_not_a_directory_is_refused_with_what_to_do() {
        assert_eq!(
            destination_is_not_a_directory("tasks/x", ".rituals", MIGRATE).to_string(),
            "refusing to move tasks/x: .rituals already exists and is not a directory; move it \
             out of the way, then run `cargo ritual migrate` again"
        );
    }

    #[test]
    fn a_taken_destination_is_refused_with_what_to_do() {
        assert_eq!(
            destination_exists("tasks/x", ".rituals/x", MIGRATE).to_string(),
            "refusing to move tasks/x: .rituals/x already exists; move it out of the way, then \
             run `cargo ritual migrate` again"
        );
    }

    #[test]
    fn a_submodule_is_refused_with_how_to_move_it_by_hand() {
        assert_eq!(
            holds_a_submodule("tasks/x", "tasks/x/vendor", MIGRATE).to_string(),
            "refusing to move tasks/x: tasks/x/vendor is a git submodule, which migrate cannot \
             move; move tasks/x with `git mv`, update its members entry and every path to it, \
             then run `cargo ritual migrate` again"
        );
    }

    fn member(directory: &str, package: &str) -> Member {
        Member {
            directory: PathBuf::from(directory),
            package: package.to_string(),
        }
    }

    #[test]
    fn a_task_directory_with_no_other_member_inside_is_fine() {
        let directories = [PathBuf::from("/w/tasks/x")];
        let members = [member("/w/tasks/x", "x"), member("/w/tools/y", "y")];
        assert!(
            ensure_none_holds_other_members(&directories, &members, Path::new("/w"), MIGRATE)
                .is_ok()
        );
    }

    #[test]
    fn a_member_inside_a_task_directory_is_refused_by_package_name() {
        let directories = [PathBuf::from("/w/tasks/x")];
        let members = [
            member("/w/tasks/x", "x"),
            member("/w/tasks/x/inner", "inner"),
        ];
        let refused =
            ensure_none_holds_other_members(&directories, &members, Path::new("/w"), MIGRATE)
                .err()
                .map(|failure| failure.to_string());
        assert_eq!(
            refused.as_deref(),
            Some(
                "refusing to move tasks/x: it also holds the workspace members `inner`, which \
                 moving it would take too; move them out of it first, then run \
                 `cargo ritual migrate` again"
            )
        );
    }

    /// A member beside the task, in a directory that only shares the name's
    /// start, is not inside it.
    #[test]
    fn a_member_in_a_directory_sharing_the_prefix_is_not_inside() {
        let directories = [PathBuf::from("/w/tasks/x")];
        let members = [
            member("/w/tasks/x", "x"),
            member("/w/tasks/x-extra", "extra"),
        ];
        assert!(
            ensure_none_holds_other_members(&directories, &members, Path::new("/w"), MIGRATE)
                .is_ok()
        );
    }

    #[test]
    fn two_members_inside_are_both_named() {
        let directories = [PathBuf::from("/w/tasks/x")];
        let members = [
            member("/w/tasks/x", "x"),
            member("/w/tasks/x/a", "a"),
            member("/w/tasks/x/b", "b"),
        ];
        let refused =
            ensure_none_holds_other_members(&directories, &members, Path::new("/w"), MIGRATE)
                .err()
                .map(|failure| failure.to_string());
        assert!(
            refused.is_some_and(|message| message.contains("the workspace members `a` and `b`")),
            "both inner members must be named"
        );
    }

    fn moves(root: &Path, tasks: &[&str]) -> Vec<(PathBuf, PathBuf)> {
        tasks
            .iter()
            .map(|task| {
                (
                    root.join("tasks").join(task),
                    root.join(".rituals").join(task),
                )
            })
            .collect()
    }

    #[test]
    fn free_destinations_are_accepted_whether_or_not_the_directory_exists() -> TestOutcome {
        let scratch = ScratchDir::new("refusals-free")?;
        let root = scratch.path();
        let moves = moves(root, &["greet"]);
        assert!(
            ensure_destinations_are_free(&moves, &root.join(".rituals"), root, MIGRATE).is_ok()
        );

        std::fs::create_dir_all(root.join(".rituals/other"))?;
        assert!(
            ensure_destinations_are_free(&moves, &root.join(".rituals"), root, MIGRATE).is_ok()
        );
        Ok(())
    }

    #[test]
    fn a_file_where_the_directory_goes_is_refused_before_any_destination_is_looked_at()
    -> TestOutcome {
        let scratch = ScratchDir::new("refusals-file")?;
        let root = scratch.path();
        write_files(root, &[(".rituals", "not a directory\n")])?;

        let refused = ensure_destinations_are_free(
            &moves(root, &["greet"]),
            &root.join(".rituals"),
            root,
            MIGRATE,
        )
        .err()
        .map(|failure| failure.to_string());

        assert_eq!(
            refused.as_deref(),
            Some(
                "refusing to move tasks/greet: .rituals already exists and is not a directory; \
                 move it out of the way, then run `cargo ritual migrate` again"
            )
        );
        Ok(())
    }

    #[test]
    fn a_taken_destination_is_refused_whatever_is_in_it() -> TestOutcome {
        let scratch = ScratchDir::new("refusals-taken")?;
        let root = scratch.path();
        write_files(root, &[(".rituals/shout/mine.txt", "mine\n")])?;

        let refused = ensure_destinations_are_free(
            &moves(root, &["greet", "shout"]),
            &root.join(".rituals"),
            root,
            MIGRATE,
        )
        .err()
        .map(|failure| failure.to_string());

        assert_eq!(
            refused.as_deref(),
            Some(
                "refusing to move tasks/shout: .rituals/shout already exists; move it out of \
                 the way, then run `cargo ritual migrate` again"
            )
        );
        Ok(())
    }

    /// A link to nothing is something: Cargo and `rename` both treat the
    /// name as taken.
    #[test]
    fn a_link_to_nothing_where_a_task_goes_is_taken() -> TestOutcome {
        let scratch = ScratchDir::new("refusals-dangling")?;
        let root = scratch.path();
        std::fs::create_dir_all(root.join(".rituals"))?;
        std::os::unix::fs::symlink(root.join("nowhere"), root.join(".rituals/greet"))?;

        let refused = ensure_destinations_are_free(
            &moves(root, &["greet"]),
            &root.join(".rituals"),
            root,
            MIGRATE,
        );

        assert!(refused.is_err());
        Ok(())
    }

    fn repository_of(root: &Path) -> Result<WorkTree, Box<dyn std::error::Error>> {
        init_and_commit(root)?;
        Ok(WorkTree::take(root))
    }

    #[test]
    fn a_task_directory_with_no_submodule_is_fine() -> TestOutcome {
        let scratch = ScratchDir::new("refusals-no-submodule")?;
        let root = scratch.path();
        write_files(root, &[("tasks/greet/Cargo.toml", "x\n")])?;
        let work_tree = repository_of(root)?;
        let repository = work_tree
            .ensure_clean(MIGRATE)
            .map_err(|failure| failure.to_string())?;

        let checked =
            ensure_none_holds_a_submodule(&[root.join("tasks/greet")], repository, root, MIGRATE);

        assert!(checked.is_ok());
        Ok(())
    }

    /// Git records a submodule as a commit: a gitlink in the index that no
    /// file in the work tree is tracked under. Made here with `update-index`
    /// and the empty directory a submodule that was never cloned leaves, so
    /// the fixture needs no second repository and the work tree stays clean.
    fn add_gitlink(root: &Path, path: &str) -> TestOutcome {
        std::fs::create_dir_all(root.join(path))?;
        git(
            root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{},{path}", "1".repeat(40)),
            ],
        )
    }

    #[test]
    fn a_submodule_inside_a_task_directory_is_refused_naming_it() -> TestOutcome {
        let scratch = ScratchDir::new("refusals-submodule")?;
        let root = scratch.path();
        write_files(root, &[("tasks/greet/Cargo.toml", "x\n")])?;
        init_and_commit(root)?;
        add_gitlink(root, "tasks/greet/vendor")?;
        git(root, &["commit", "--quiet", "--message", "link"])?;
        let work_tree = WorkTree::take(root);
        let repository = work_tree
            .ensure_clean(MIGRATE)
            .map_err(|failure| failure.to_string())?;

        let refused =
            ensure_none_holds_a_submodule(&[root.join("tasks/greet")], repository, root, MIGRATE)
                .err()
                .map(|failure| failure.to_string());

        assert_eq!(
            refused.as_deref(),
            Some(
                "refusing to move tasks/greet: tasks/greet/vendor is a git submodule, which \
                 migrate cannot move; move tasks/greet with `git mv`, update its members entry \
                 and every path to it, then run `cargo ritual migrate` again"
            )
        );
        Ok(())
    }

    #[test]
    fn a_task_directory_that_is_itself_a_submodule_is_refused_naming_it() -> TestOutcome {
        let scratch = ScratchDir::new("refusals-submodule-itself")?;
        let root = scratch.path();
        write_files(root, &[("a.txt", "x\n")])?;
        init_and_commit(root)?;
        add_gitlink(root, "tasks/greet")?;
        git(root, &["commit", "--quiet", "--message", "link"])?;
        let work_tree = WorkTree::take(root);
        let repository = work_tree
            .ensure_clean(MIGRATE)
            .map_err(|failure| failure.to_string())?;

        let refused =
            ensure_none_holds_a_submodule(&[root.join("tasks/greet")], repository, root, MIGRATE)
                .err()
                .map(|failure| failure.to_string());

        assert!(
            refused.is_some_and(|message| message
                .starts_with("refusing to move tasks/greet: tasks/greet is a git submodule")),
            "the directory itself is the gitlink"
        );
        Ok(())
    }
}
