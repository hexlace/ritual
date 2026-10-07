//! The lines `migrate` prints, each built by a pure function so what a
//! person reads is pinned without running a project.
//!
//! Every line is a past-tense verb and then the path (`moved`, `updated`,
//! `deleted`, `kept`), with any detail in parentheses after it, or the path
//! and then what is true of it (`<path> still mentions tasks/`). A directory
//! is written with a trailing `/`, a value quoted from a manifest in
//! backticks.

use std::fmt::Display;

use rituals::Failure;
use rituals_compose::git::Unanswered;
use rituals_compose::sentence::join_with_and;

/// What a run says when no step applied, and nothing else.
pub(crate) const NOTHING_TO_MIGRATE: &str = "nothing to migrate";

/// The last line of a run that changed the project: what the person does
/// next. Both commands are named because `git diff` alone does not show the
/// files that moved, which git sees as untracked until they are added.
pub(crate) const NEXT: &str =
    "next: review the changes with git status and git diff, then commit them";

/// A task's directory that moved, both ends spelled from the project's root.
pub(crate) fn moved(from: &str, to: &str) -> String {
    format!("moved {from} to {to}")
}

/// One value changed in the manifest `file`, where `change` reads as the
/// clause that follows the file's name.
pub(crate) fn updated(file: &str, change: &impl Display) -> String {
    format!("updated {file} ({change})")
}

/// A directory the moves left empty, which `migrate` removed.
pub(crate) fn deleted(directory: &str) -> String {
    format!("deleted {directory}/ (empty once its tasks moved out)")
}

/// A directory the moves did not empty, with what is still in it: each entry
/// as its name, a directory's with a trailing `/`.
pub(crate) fn kept(directory: &str, entries: &[String]) -> String {
    format!(
        "kept {directory}/, which still holds {}",
        join_with_and(entries)
    )
}

/// A directory `migrate` could not list the contents of.
pub(crate) fn kept_unlisted(directory: &str, cause: &std::io::Error) -> String {
    format!("kept {directory}/, whose contents could not be listed: {cause}")
}

/// A file that still mentions the legacy directory, which `migrate` does not
/// edit.
pub(crate) fn still_mentions(path: &str) -> String {
    format!("{path} still mentions tasks/")
}

/// An empty directory that could not be removed, after every task had moved.
pub(crate) fn deletion_failed(directory: &str, cause: std::io::Error) -> String {
    Failure::new(format!(
        "deleting {directory}/ failed after every task had moved, and Cargo reads the \
         project as it should; delete {directory}/ by hand"
    ))
    .caused_by(cause)
    .with_causes()
    .to_string()
}

/// A search for the files that still mention the legacy directory that git
/// could not make, after every task had moved.
pub(crate) fn listing_failed(unanswered: &Unanswered) -> String {
    format!(
        "git could not list the files that still mention tasks/: {unanswered}; every task has \
         already moved and Cargo reads the project as it should, so look for them by hand"
    )
}

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind};

    use rituals_compose::git::Unanswered;

    use super::{
        NEXT, NOTHING_TO_MIGRATE, deleted, deletion_failed, kept, kept_unlisted, listing_failed,
        moved, still_mentions, updated,
    };

    #[test]
    fn a_move_names_both_ends() {
        assert_eq!(
            moved("tasks/greet", ".rituals/greet"),
            "moved tasks/greet to .rituals/greet"
        );
    }

    #[test]
    fn an_edit_names_the_file_and_puts_the_change_in_parentheses() {
        assert_eq!(
            updated(
                "ritual/Cargo.toml",
                &"[dependencies] greet path `../tasks/greet` is now `../.rituals/greet`"
            ),
            "updated ritual/Cargo.toml ([dependencies] greet path `../tasks/greet` is now \
             `../.rituals/greet`)"
        );
    }

    #[test]
    fn a_deleted_directory_says_why_it_was_empty() {
        assert_eq!(
            deleted("tasks"),
            "deleted tasks/ (empty once its tasks moved out)"
        );
    }

    #[test]
    fn a_kept_directory_names_what_is_still_in_it() {
        assert_eq!(
            kept("tasks", &["helper/".to_string(), "notes.md".to_string()]),
            "kept tasks/, which still holds helper/ and notes.md"
        );
        assert_eq!(
            kept(
                "tasks",
                &["a/".to_string(), "b/".to_string(), "c.md".to_string()]
            ),
            "kept tasks/, which still holds a/, b/ and c.md"
        );
        assert_eq!(
            kept("tasks", &["helper/".to_string()]),
            "kept tasks/, which still holds helper/"
        );
    }

    #[test]
    fn a_kept_directory_that_could_not_be_listed_says_so() {
        assert_eq!(
            kept_unlisted("tasks", &Error::new(ErrorKind::PermissionDenied, "denied")),
            "kept tasks/, whose contents could not be listed: denied"
        );
    }

    #[test]
    fn a_file_that_mentions_the_old_directory_leads_with_its_path() {
        assert_eq!(
            still_mentions(".github/workflows/ci.yml"),
            ".github/workflows/ci.yml still mentions tasks/"
        );
    }

    #[test]
    fn a_failed_deletion_leads_with_what_failed_and_says_cargo_reads_the_project() {
        assert_eq!(
            deletion_failed("tasks", Error::new(ErrorKind::PermissionDenied, "denied")),
            "deleting tasks/ failed after every task had moved, and Cargo reads the project \
             as it should; delete tasks/ by hand: denied"
        );
    }

    #[test]
    fn a_failed_listing_says_what_failed_in_gits_words_and_what_to_do() {
        assert_eq!(
            listing_failed(&Unanswered::Failed("bad pattern".to_string())),
            "git could not list the files that still mention tasks/: bad pattern; every task \
             has already moved and Cargo reads the project as it should, so look for them by \
             hand"
        );
    }

    #[test]
    fn the_two_fixed_lines_read_as_the_plan_words_them() {
        assert_eq!(NOTHING_TO_MIGRATE, "nothing to migrate");
        assert_eq!(
            NEXT,
            "next: review the changes with git status and git diff, then commit them"
        );
    }
}
