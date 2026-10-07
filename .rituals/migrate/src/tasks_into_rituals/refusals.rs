//! Why the first step refuses, in the words a person reads, and the checks
//! that decide it.
//!
//! Every refusal about one task's directory opens with `refusing to move`
//! and the directory, so the person learns which task is in the way before
//! why, and ends with what to do and the command to run again.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use rituals::{Failure, Outcome};
use rituals_compose::git::{
    self, AttributeChange, Flag, IgnoreRule, IgnoredFile, MovedFile, SeenDifferently, Unanswered,
    Unwatched,
};
use rituals_compose::relocation::Relocation;
use rituals_compose::sentence::join_with_and;

use crate::places::{from_the_root, listed};
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
        let submodules = git::submodules_under(directory).map_err(|unanswered| {
            submodule_question_refusal(&unanswered, &shown, migrate_command)
        })?;
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

/// Refuses when moving the directories would change what git sees of the
/// files in them: a rule that ignores a file at its new place and not now, or
/// the reverse, other attributes there, a place outside the sparse checkout,
/// or a tracked file git has been told not to look at.
///
/// Git decides these by path, so a commit after the move would leave out a
/// file that is committed now, add one that is ignored now, or store one
/// through another filter, and `git status` would show each as an ordinary
/// change. The refusal opens `refusing to migrate` rather than naming a
/// directory, because one rule can catch every task.
///
/// The refusal names the directory the tasks move into, which the relocation
/// holds.
pub(super) fn ensure_the_moves_keep_what_git_sees(
    relocation: &Relocation,
    repository: &Repository,
    root: &Path,
    migrate_command: &str,
) -> Outcome {
    git::ensure_a_move_keeps_what_git_sees(relocation, root).map_err(|seen_differently| {
        seen_differently_refusal(
            &seen_differently,
            &from_the_root(relocation.moved_into(), root),
            repository,
            migrate_command,
        )
    })
}

/// Refuses when git does not track a manifest that needs an edit, because
/// once migrate has edited it git could not give its old bytes back.
///
/// `edits` is each such manifest with the first change it needs, as a person
/// reads one. A manifest git ignores or has never been told about is the
/// case: the work tree is clean, so it is ignored. One git tracks is fine
/// whatever a rule says about it.
pub(super) fn ensure_git_tracks_every_edited_manifest(
    edits: &[(PathBuf, String)],
    root: &Path,
    migrate_command: &str,
) -> Outcome {
    let manifests: Vec<PathBuf> = edits
        .iter()
        .map(|(manifest, _change)| manifest.clone())
        .collect();
    let untracked = git::files_git_does_not_track(&manifests, root).map_err(|unanswered| {
        precondition::unanswered_refusal(&unanswered, migrate_command, |message| {
            tracking_unknown(message)
        })
    })?;
    let Some(manifest) = untracked.first() else {
        return Ok(());
    };
    let change = edits
        .iter()
        .find(|(edited, _change)| edited == manifest)
        .map_or("", |(_manifest, change)| change.as_str());
    Err(manifest_is_not_tracked(
        &from_the_root(manifest, root),
        change,
        migrate_command,
    ))
}

/// The refusal for a git that could not say whether a directory holds a
/// submodule.
fn submodule_question_refusal(
    unanswered: &Unanswered,
    directory: &str,
    migrate_command: &str,
) -> Failure {
    precondition::unanswered_refusal(unanswered, migrate_command, |message| {
        submodule_unknown(directory, message)
    })
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

fn manifest_is_not_tracked(manifest: &str, change: &str, migrate_command: &str) -> Failure {
    Failure::new(format!(
        "refusing to migrate: git does not track {manifest}, which Cargo reads and which would \
         need editing ({change}); git could not give it back once migrate edits it, so commit \
         it, or take that path out of it, then run `{migrate_command}` again"
    ))
}

/// The refusal for what git would see differently, with every path spelled
/// from the project's root.
fn seen_differently_refusal(
    seen_differently: &SeenDifferently,
    to_directory: &str,
    repository: &Repository,
    migrate_command: &str,
) -> Failure {
    match seen_differently {
        SeenDifferently::Unanswered(unanswered) => {
            precondition::unanswered_refusal(unanswered, migrate_command, |message| {
                sight_unknown(message)
            })
        }
        SeenDifferently::WouldBeIgnored(files) => would_be_ignored(
            &files
                .iter()
                .map(|ignored| ignored_at(ignored, MovedFile::to, repository))
                .collect::<Vec<_>>(),
            to_directory,
            migrate_command,
        ),
        SeenDifferently::WouldNoLongerBeIgnored(files) => would_no_longer_be_ignored(
            &files
                .iter()
                .map(|ignored| ignored_at(ignored, MovedFile::from, repository))
                .collect::<Vec<_>>(),
            to_directory,
            migrate_command,
        ),
        SeenDifferently::AttributesWouldChange(changes) => attributes_would_change(
            &changes
                .iter()
                .map(|change| attribute_change_named(change, repository))
                .collect::<Vec<_>>(),
            to_directory,
            migrate_command,
        ),
        SeenDifferently::OutsideSparseCheckout(files) => outside_the_sparse_checkout(
            &files
                .iter()
                .map(|file| repository.shown(file.to()))
                .collect::<Vec<_>>(),
            to_directory,
            migrate_command,
        ),
        SeenDifferently::Unwatched(files) => unwatched_by_git(
            &files
                .iter()
                .map(|file| unwatched_named(file, repository))
                .collect::<Vec<_>>(),
            migrate_command,
        ),
    }
}

/// `it` for one file and `them` for more, for a sentence about the files.
const fn it_or_them(count: usize) -> &'static str {
    if count == 1 { "it" } else { "them" }
}

/// A file with the rule that ignores it: `tasks/greet/.env (`.env` in
/// .gitignore:2)`. `place` says which of the file's two places is named.
fn ignored_at(
    ignored: &IgnoredFile,
    place: fn(&MovedFile) -> &Path,
    repository: &Repository,
) -> String {
    format!(
        "{} ({})",
        repository.shown(place(ignored.file())),
        rule_named(ignored.rule(), repository)
    )
}

/// A rule as `` `pattern` in source:line``, the source spelled from the
/// project's root.
fn rule_named(rule: &IgnoreRule, repository: &Repository) -> String {
    let source = repository.shown(rule.source());
    format!("`{}` in {source}:{}", rule.pattern(), rule.line())
}

/// A file with its attributes at both places: `tasks/greet/a.bin
/// (`filter=lfs` now, none at .rituals/greet/a.bin)`.
fn attribute_change_named(change: &AttributeChange, repository: &Repository) -> String {
    let named = |attributes: &[git::Attribute]| {
        if attributes.is_empty() {
            "none".to_string()
        } else {
            attributes
                .iter()
                .map(|attribute| format!("`{attribute}`"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    let to = repository.shown(change.file().to());
    format!(
        "{} ({} now, {} at {to})",
        repository.shown(change.file().from()),
        named(change.before()),
        named(change.after())
    )
}

/// A file git has been told not to look at, with the flag: `tasks/greet/a.rs
/// (assume-unchanged)`.
fn unwatched_named(file: &Unwatched, repository: &Repository) -> String {
    let flag = match file.flag() {
        Flag::AssumeUnchanged => "assume-unchanged",
        Flag::SkipWorktree => "skip-worktree",
    };
    format!("{} ({flag})", repository.shown(file.path()))
}

/// `files` are each file git would ignore at its new place, with the rule.
fn would_be_ignored(files: &[String], to_directory: &str, migrate_command: &str) -> Failure {
    Failure::new(format!(
        "refusing to migrate: git would ignore {}, so a commit after the move would leave out \
         what is committed now; change the rules named so they do not match under \
         {to_directory}/, then run `{migrate_command}` again",
        listed(files)
    ))
}

/// `files` are each file git ignores now, with the rule.
fn would_no_longer_be_ignored(
    files: &[String],
    to_directory: &str,
    migrate_command: &str,
) -> Failure {
    let them = it_or_them(files.len());
    Failure::new(format!(
        "refusing to migrate: git ignores {} now, but nothing would ignore {them} under \
         {to_directory}/, so a commit after the move would add {them}; add a rule that ignores \
         {them} there, then run `{migrate_command}` again",
        listed(files)
    ))
}

/// `files` are each file whose attributes would differ, with both sets.
fn attributes_would_change(files: &[String], to_directory: &str, migrate_command: &str) -> Failure {
    Failure::new(format!(
        "refusing to migrate: git would give {} other attributes under {to_directory}/ than it \
         gives {} now; give {to_directory}/ the same attributes in .gitattributes, then run \
         `{migrate_command}` again",
        listed(files),
        it_or_them(files.len())
    ))
}

/// `files` are each new place the sparse checkout leaves out.
fn outside_the_sparse_checkout(
    files: &[String],
    to_directory: &str,
    migrate_command: &str,
) -> Failure {
    let them = it_or_them(files.len());
    Failure::new(format!(
        "refusing to migrate: {} would be outside this checkout's sparse-checkout patterns, so \
         git would not add {them}; add {them} with `git sparse-checkout add {to_directory}`, \
         then run `{migrate_command}` again",
        listed(files)
    ))
}

/// `files` are each tracked file git has been told not to look at, with the
/// flag.
fn unwatched_by_git(files: &[String], migrate_command: &str) -> Failure {
    Failure::new(format!(
        "refusing to migrate: git has been told not to look at changes to {}, so a commit after \
         the move could carry an edit git does not show; clear the flag with `git update-index \
         --no-assume-unchanged` or `--no-skip-worktree`, commit what changed, then run \
         `{migrate_command}` again",
        listed(files)
    ))
}

fn sight_unknown(git_words: &str) -> Failure {
    Failure::new(format!(
        "refusing to migrate: git could not say how it would see the files that move: \
         {git_words}"
    ))
}

fn tracking_unknown(git_words: &str) -> Failure {
    Failure::new(format!(
        "refusing to migrate: git could not say which of the manifests migrate edits it \
         tracks: {git_words}"
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

    use rituals_compose::git::fixture::git;

    use super::{
        Member, attributes_would_change, destination_exists, destination_is_not_a_directory,
        ensure_destinations_are_free, ensure_git_tracks_every_edited_manifest,
        ensure_none_holds_a_submodule, ensure_none_holds_other_members,
        ensure_the_moves_keep_what_git_sees, holds_a_submodule, holds_members,
        manifest_is_not_tracked, outside_the_sparse_checkout, sight_unknown, tracking_unknown,
        unwatched_by_git, would_be_ignored, would_no_longer_be_ignored,
    };
    use crate::precondition::WorkTree;
    use crate::test_support::{ScratchDir, TestOutcome, init_and_commit, write_files};
    use rituals_compose::relocation::Relocation;

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

    #[test]
    fn an_untracked_manifest_is_refused_with_the_edit_and_what_to_do() {
        assert_eq!(
            manifest_is_not_tracked(
                "vendor/x/Cargo.toml",
                "[dependencies] greet path `../../tasks/greet` is now `../../.rituals/greet`",
                MIGRATE
            )
            .to_string(),
            "refusing to migrate: git does not track vendor/x/Cargo.toml, which Cargo reads and \
             which would need editing ([dependencies] greet path `../../tasks/greet` is now \
             `../../.rituals/greet`); git could not give it back once migrate edits it, so \
             commit it, or take that path out of it, then run `cargo ritual migrate` again"
        );
    }

    #[test]
    fn a_git_that_could_not_say_what_it_tracks_is_refused_in_its_own_words() {
        assert_eq!(
            tracking_unknown("bad object").to_string(),
            "refusing to migrate: git could not say which of the manifests migrate edits it \
             tracks: bad object"
        );
    }

    #[test]
    fn a_tracked_manifest_that_needs_an_edit_is_fine() -> TestOutcome {
        let scratch = ScratchDir::resolved("refusals-tracked-manifest")?;
        let root = scratch.path();
        write_files(root, &[("Cargo.toml", "# committed\n")])?;
        init_and_commit(root)?;

        let checked = ensure_git_tracks_every_edited_manifest(
            &[(root.join("Cargo.toml"), "a change".to_string())],
            root,
            MIGRATE,
        );

        assert!(checked.is_ok());
        Ok(())
    }

    #[test]
    fn nothing_to_edit_asks_git_nothing() {
        assert!(
            ensure_git_tracks_every_edited_manifest(&[], Path::new("/nowhere"), MIGRATE).is_ok()
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
        let scratch = ScratchDir::resolved("refusals-free")?;
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
        let scratch = ScratchDir::resolved("refusals-file")?;
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
        let scratch = ScratchDir::resolved("refusals-taken")?;
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
        let scratch = ScratchDir::resolved("refusals-dangling")?;
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
        let scratch = ScratchDir::resolved("refusals-no-submodule")?;
        let root = scratch.path();
        write_files(root, &[("tasks/greet/Cargo.toml", "x\n")])?;
        let work_tree = repository_of(root)?;
        let repository = work_tree
            .ensure_clean(MIGRATE)
            .map_err(|failure| failure.with_causes().to_string())?;

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
        )?;
        Ok(())
    }

    #[test]
    fn a_submodule_inside_a_task_directory_is_refused_naming_it() -> TestOutcome {
        let scratch = ScratchDir::resolved("refusals-submodule")?;
        let root = scratch.path();
        write_files(root, &[("tasks/greet/Cargo.toml", "x\n")])?;
        init_and_commit(root)?;
        add_gitlink(root, "tasks/greet/vendor")?;
        git(root, &["commit", "--quiet", "--message", "link"])?;
        let work_tree = WorkTree::take(root);
        let repository = work_tree
            .ensure_clean(MIGRATE)
            .map_err(|failure| failure.with_causes().to_string())?;

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
        let scratch = ScratchDir::resolved("refusals-submodule-itself")?;
        let root = scratch.path();
        write_files(root, &[("a.txt", "x\n")])?;
        init_and_commit(root)?;
        add_gitlink(root, "tasks/greet")?;
        git(root, &["commit", "--quiet", "--message", "link"])?;
        let work_tree = WorkTree::take(root);
        let repository = work_tree
            .ensure_clean(MIGRATE)
            .map_err(|failure| failure.with_causes().to_string())?;

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

    fn named(places: &[&str]) -> Vec<String> {
        places.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_file_git_would_ignore_is_refused_with_the_rule_and_how_to_change_it() {
        assert_eq!(
            would_be_ignored(
                &named(&[".rituals/greet/Cargo.toml (`.*` in .gitignore:2)"]),
                ".rituals",
                MIGRATE
            )
            .to_string(),
            "refusing to migrate: git would ignore .rituals/greet/Cargo.toml (`.*` in \
             .gitignore:2), so a commit after the move would leave out what is committed now; \
             change the rules named so they do not match under .rituals/, then run `cargo \
             ritual migrate` again"
        );
        assert!(
            would_be_ignored(&named(&["a (r)", "b (r)"]), ".rituals", MIGRATE)
                .to_string()
                .contains("git would ignore a (r), b (r), so a commit")
        );
    }

    #[test]
    fn an_ignored_file_the_new_place_would_not_ignore_is_refused_in_the_singular() {
        assert_eq!(
            would_no_longer_be_ignored(
                &named(&["tasks/greet/.env (`.env` in .gitignore:2)"]),
                ".rituals",
                MIGRATE
            )
            .to_string(),
            "refusing to migrate: git ignores tasks/greet/.env (`.env` in .gitignore:2) now, \
             but nothing would ignore it under .rituals/, so a commit after the move would add \
             it; add a rule that ignores it there, then run `cargo ritual migrate` again"
        );
    }

    #[test]
    fn ignored_files_the_new_place_would_not_ignore_are_refused_in_the_plural() {
        assert_eq!(
            would_no_longer_be_ignored(
                &named(&["a (`x` in .gitignore:1)", "b (`x` in .gitignore:1)"]),
                ".rituals",
                MIGRATE
            )
            .to_string(),
            "refusing to migrate: git ignores a (`x` in .gitignore:1), b (`x` in \
             .gitignore:1) now, but nothing would ignore them under .rituals/, so a commit \
             after the move would add them; add a rule that ignores them there, then run `cargo \
             ritual migrate` again"
        );
    }

    #[test]
    fn a_file_that_would_have_other_attributes_is_refused_in_the_singular_and_the_plural() {
        assert_eq!(
            attributes_would_change(
                &named(&[
                    "tasks/greet/a.bin (`filter=lfs`, `-text` now, none at .rituals/greet/a.bin)"
                ]),
                ".rituals",
                MIGRATE
            )
            .to_string(),
            "refusing to migrate: git would give tasks/greet/a.bin (`filter=lfs`, `-text` now, \
             none at .rituals/greet/a.bin) other attributes under .rituals/ than it gives it \
             now; give .rituals/ the same attributes in .gitattributes, then run `cargo ritual \
             migrate` again"
        );
        assert!(
            attributes_would_change(&named(&["a", "b"]), ".rituals", MIGRATE)
                .to_string()
                .contains("than it gives them now;")
        );
    }

    #[test]
    fn a_new_place_outside_the_sparse_checkout_is_refused_with_how_to_add_it() {
        assert_eq!(
            outside_the_sparse_checkout(
                &named(&[".rituals/greet/Cargo.toml"]),
                ".rituals",
                MIGRATE
            )
            .to_string(),
            "refusing to migrate: .rituals/greet/Cargo.toml would be outside this checkout's \
             sparse-checkout patterns, so git would not add it; add it with `git \
             sparse-checkout add .rituals`, then run `cargo ritual migrate` again"
        );
        assert!(
            outside_the_sparse_checkout(&named(&["a", "b"]), ".rituals", MIGRATE)
                .to_string()
                .contains("so git would not add them; add them with")
        );
    }

    #[test]
    fn a_file_git_was_told_not_to_look_at_is_refused_with_how_to_clear_the_flag() {
        assert_eq!(
            unwatched_by_git(&named(&["tasks/greet/a.rs (assume-unchanged)"]), MIGRATE).to_string(),
            "refusing to migrate: git has been told not to look at changes to \
             tasks/greet/a.rs (assume-unchanged), so a commit after the move could carry an \
             edit git does not show; clear the flag with `git update-index \
             --no-assume-unchanged` or `--no-skip-worktree`, commit what changed, then run \
             `cargo ritual migrate` again"
        );
    }

    #[test]
    fn a_git_that_could_not_say_how_it_would_see_the_files_is_refused_in_its_own_words() {
        assert_eq!(
            sight_unknown("bad object").to_string(),
            "refusing to migrate: git could not say how it would see the files that move: bad \
             object"
        );
    }

    /// The whole check against a real repository: a project ignore file that
    /// would ignore the new directory is refused, naming each file where it
    /// would be, the rule, and the line it is on, and fixing the rule lets
    /// the same check through.
    #[test]
    fn the_check_refuses_a_rule_that_would_ignore_the_new_directory_and_passes_once_it_is_fixed()
    -> TestOutcome {
        let scratch = ScratchDir::resolved("refusals-sight")?;
        let root = scratch.path();
        write_files(
            root,
            &[
                ("tasks/greet/Cargo.toml", "[package]\n"),
                (".gitignore", "/target\n.*\n!.gitignore\n"),
            ],
        )?;
        init_and_commit(root)?;
        let work_tree = WorkTree::take(root);
        let repository = work_tree
            .ensure_clean(MIGRATE)
            .map_err(|failure| failure.with_causes().to_string())?;
        let relocation = Relocation::new(
            &root.join("tasks"),
            &root.join(".rituals"),
            [root.join("tasks/greet")],
        );
        let check = || {
            ensure_the_moves_keep_what_git_sees(&relocation, repository, root, MIGRATE)
                .err()
                .map(|failure| failure.to_string())
        };

        assert_eq!(
            check().as_deref(),
            Some(
                "refusing to migrate: git would ignore .rituals/greet/Cargo.toml (`.*` in \
                 .gitignore:2), so a commit after the move would leave out what is committed \
                 now; change the rules named so they do not match under .rituals/, then run \
                 `cargo ritual migrate` again"
            )
        );

        write_files(
            root,
            &[(".gitignore", "/target\n.*\n!.gitignore\n!.rituals\n")],
        )?;
        assert_eq!(check(), None);
        Ok(())
    }
}
