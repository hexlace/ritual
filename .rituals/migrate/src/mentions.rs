//! The files that still mention `tasks/` once the tasks have moved.
//!
//! They are listed and never edited: whether a path in a workflow or a
//! script means the task directory is for the person to judge.

use std::path::Path;

use rituals_compose::git;

use crate::precondition::Repository;
use crate::report;

/// `tasks/` at the start of a line or after anything that cannot be part of
/// a longer name, as git's extended regular expressions read it, so that
/// `subtasks/` and `my-tasks/` are not mentions.
const MENTIONS_TASKS: &str = "(^|[^A-Za-z0-9_.-])tasks/";

/// One line for each file in the repository that mentions `tasks/`, sorted,
/// each spelled from the project's root; or the one line that says the
/// search could not be made.
///
/// Every file the repository would carry is searched, whether or not
/// `migrate` edited it, since the mentions themselves are never edited.
/// `root` is a directory inside the repository, as `repository` knows it.
pub(crate) fn lines(root: &Path, repository: &Repository) -> Vec<String> {
    match git::files_mentioning(root, MENTIONS_TASKS) {
        Ok(files) => {
            let mut shown: Vec<String> = files.iter().map(|file| repository.shown(file)).collect();
            // Sorted again from the root: git sorted them from the top level,
            // and a file above the root spells with `..`.
            shown.sort_unstable();
            shown
                .iter()
                .map(|file| report::still_mentions(file))
                .collect()
        }
        Err(unanswered) => vec![report::listing_failed(&unanswered)],
    }
}

#[cfg(test)]
mod tests {
    use super::lines;
    use crate::precondition::WorkTree;
    use crate::test_support::{ScratchDir, TestOutcome, init_and_commit, write_files};

    fn lines_in(root: &std::path::Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let work_tree = WorkTree::take(root);
        let repository = work_tree
            .ensure_clean("cargo ritual migrate")
            .map_err(|failure| failure.with_causes().to_string())?;
        Ok(lines(root, repository))
    }

    #[test]
    fn every_file_that_mentions_tasks_is_listed_in_path_order() -> TestOutcome {
        let scratch = ScratchDir::resolved("mentions-listed")?;
        let root = scratch.path();
        write_files(
            root,
            &[
                (".github/workflows/ci.yml", "- run: ls tasks/greet\n"),
                ("scripts/check.sh", "ls tasks/\n"),
                ("readme.md", "# demo\n\nThe tooling is in tasks/greet.\n"),
                ("notes.txt", "says nothing about the layout\n"),
            ],
        )?;
        init_and_commit(root)?;

        assert_eq!(
            lines_in(root)?,
            [
                ".github/workflows/ci.yml still mentions tasks/",
                "readme.md still mentions tasks/",
                "scripts/check.sh still mentions tasks/",
            ]
        );
        Ok(())
    }

    /// A search git cannot make is a line, not a failure: the tasks have
    /// moved by then, and what is lost is only the list. The repository is
    /// known from before the run, and the directory searched is one git can
    /// no longer place in any.
    #[test]
    fn a_search_git_cannot_make_is_one_line_that_says_what_to_do() -> TestOutcome {
        let scratch = ScratchDir::resolved("mentions-failed")?;
        let repository_root = scratch.path().join("repository");
        write_files(&repository_root, &[("a.txt", "x\n")])?;
        init_and_commit(&repository_root)?;
        let work_tree = WorkTree::take(&repository_root);
        let repository = work_tree
            .ensure_clean("cargo ritual migrate")
            .map_err(|failure| failure.with_causes().to_string())?;
        let elsewhere = scratch.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere)?;

        let listed = lines(&elsewhere, repository);

        assert_eq!(
            listed,
            [
                "git could not list the files that still mention tasks/: the directory is not \
                 in a git repository; every task has already moved and Cargo reads the \
                 project as it should, so look for them by hand"
            ]
        );
        Ok(())
    }

    #[test]
    fn nothing_mentioning_it_is_no_lines() -> TestOutcome {
        let scratch = ScratchDir::resolved("mentions-none")?;
        write_files(scratch.path(), &[("notes.txt", "nothing\n")])?;
        init_and_commit(scratch.path())?;

        assert!(lines_in(scratch.path())?.is_empty());
        Ok(())
    }

    /// A longer name that ends in `tasks/` is not the directory.
    #[test]
    fn a_longer_name_ending_in_tasks_is_not_a_mention() -> TestOutcome {
        let scratch = ScratchDir::resolved("mentions-longer")?;
        write_files(
            scratch.path(),
            &[
                ("a.md", "see subtasks/ and my-tasks/ and a_tasks/\n"),
                ("b.md", "(tasks/greet)\n"),
            ],
        )?;
        init_and_commit(scratch.path())?;

        assert_eq!(lines_in(scratch.path())?, ["b.md still mentions tasks/"]);
        Ok(())
    }

    /// Git does not track an ignored file, so it is not part of the project's
    /// history; a file that is not yet tracked is, since a commit would carry
    /// it.
    #[test]
    fn an_ignored_file_is_left_out_and_an_untracked_one_is_listed() -> TestOutcome {
        let scratch = ScratchDir::resolved("mentions-ignored")?;
        let root = scratch.path();
        write_files(root, &[(".gitignore", "*.log\n"), ("a.txt", "x\n")])?;
        init_and_commit(root)?;
        // The repository is known clean before the run writes anything, and
        // the run leaves files behind that git has not been told about.
        let work_tree = WorkTree::take(root);
        write_files(
            root,
            &[
                ("build.log", "built tasks/greet\n"),
                ("new.md", "tasks/greet\n"),
            ],
        )?;

        let repository = work_tree
            .ensure_clean("cargo ritual migrate")
            .map_err(|failure| failure.with_causes().to_string())?;

        assert_eq!(lines(root, repository), ["new.md still mentions tasks/"]);
        Ok(())
    }

    /// In a monorepo the project's root is below the repository's top level:
    /// a file above it is listed with the steps up that reach it. Git lists them
    /// from the top level, so the order from the root is another one, and the
    /// lines are sorted again.
    #[test]
    fn a_file_outside_the_projects_root_is_spelled_from_it() -> TestOutcome {
        let scratch = ScratchDir::resolved("mentions-monorepo")?;
        let top = scratch.path();
        let root = top.join("project");
        write_files(
            top,
            &[
                (".github/workflows/ci.yml", "- run: ls project/tasks/\n"),
                ("project/Cargo.toml", "# x\n"),
                ("project/scripts/check.sh", "ls tasks/\n"),
                ("zzz/notes.md", "see project/tasks/\n"),
            ],
        )?;
        init_and_commit(top)?;

        assert_eq!(
            lines_in(&root)?,
            [
                "../.github/workflows/ci.yml still mentions tasks/",
                "../zzz/notes.md still mentions tasks/",
                "scripts/check.sh still mentions tasks/",
            ]
        );
        Ok(())
    }
}
