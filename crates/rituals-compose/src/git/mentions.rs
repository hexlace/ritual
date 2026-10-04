//! Which files of a repository mention a pattern.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Obstacle, failure_of, run_git_for_output, top_level_of};

/// Lists every file of the repository `directory` is in whose text matches
/// `extended_regex`, for a person to look at.
///
/// The whole repository is searched, whichever directory of it `directory`
/// is: the files git tracks, and the ones it does not track but would not
/// ignore, which is what a commit would carry. A file git ignores is left out,
/// and so is a binary file. `extended_regex` is read by git as a POSIX
/// extended regular expression, a line at a time, so `^` and `$` anchor a line
/// and `tasks/` alone would match `subtasks/` too.
///
/// Each path is spelled from git's top level and the result is sorted, so a
/// caller can show it as it is. Nothing matching is an empty list and not a
/// failure.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::git;
///
/// // Needs a real repository on disk and runs `git`, so this example is
/// // `no_run`.
/// // `tasks/` at the start of a line, or after anything that is not part of
/// // a longer name, so `subtasks/` is not a mention.
/// let regex = "(^|[^A-Za-z0-9_.-])tasks/";
/// for file in git::files_mentioning(Path::new("."), regex)? {
///     println!("{} still mentions tasks/", file.display());
/// }
/// # Ok::<(), rituals_compose::git::Obstacle>(())
/// ```
///
/// # Errors
///
/// Returns [`Obstacle::GitMissing`] when `git` cannot be run,
/// [`Obstacle::NotARepository`] when `directory` is not in a git repository,
/// and [`Obstacle::Failed`] in git's own words when it cannot read the
/// expression or anything else goes wrong. It returns no other variant.
///
/// # Panics
///
/// Panics if `extended_regex` is empty, which matches every file and so
/// answers nothing.
pub fn files_mentioning(directory: &Path, extended_regex: &str) -> Result<Vec<PathBuf>, Obstacle> {
    files_mentioning_with(|| Command::new("git"), directory, extended_regex)
}

/// [`files_mentioning`], with `git` started from `new_git`.
fn files_mentioning_with(
    new_git: impl Fn() -> Command,
    directory: &Path,
    extended_regex: &str,
) -> Result<Vec<PathBuf>, Obstacle> {
    assert!(
        !extended_regex.is_empty(),
        "an empty expression matches every file, so it answers nothing"
    );
    // `git grep` outside a repository searches the directory as it is,
    // which is not the question, so a repository is required first.
    top_level_of(&new_git, directory)?;

    let output = run_git_for_output(
        &new_git,
        directory,
        &[
            "grep",
            "--untracked",
            "--full-name",
            "-I",
            "-l",
            "-z",
            "-E",
            "-e",
            extended_regex,
            "--",
            ":/",
        ],
        &[],
    )?;
    // `git grep` exits 1, saying nothing, when no file matches. Anything it
    // says alongside a failing status is a failure, such as an expression it
    // cannot compile.
    match (output.status.code(), output.stderr.is_empty()) {
        (Some(0), _) => {}
        (Some(1), true) => return Ok(Vec::new()),
        _ => return Err(failure_of(&output)),
    }

    let listed = String::from_utf8_lossy(&output.stdout);
    let mut files: Vec<PathBuf> = listed
        .split('\0')
        .filter(|file| !file.is_empty())
        .map(PathBuf::from)
        .collect();
    files.sort();
    files.dedup();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::files_mentioning_with;
    use crate::git::Obstacle;
    use crate::git::test_support::{commit_everything, contained_in, git};
    use crate::test_support::{ScratchDir, TestOutcome};

    /// The pattern a caller looking for `tasks/` as a path would hand over:
    /// the name at the start of a line or after something that cannot be
    /// part of a longer name.
    const TASKS: &str = "(^|[^A-Za-z0-9_.-])tasks/";

    fn write(root: &Path, file: &str, contents: &str) -> TestOutcome {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().ok_or("a fixture file has a directory")?)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// A repository where `.github/workflows/ci.yml` and `readme.md` mention
    /// `tasks/`, `unrelated.md` and `subtasks.md` do not, and `.gitignore`
    /// ignores `target/` and `ignored.md`; nothing is untracked yet.
    fn committed_project(tag: &str) -> Result<ScratchDir, Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new(tag)?;
        let root = scratch.path();
        write(
            root,
            ".github/workflows/ci.yml",
            "run: cargo test -p tasks/lint\n",
        )?;
        write(root, "readme.md", "tasks/ hold the project's tools\n")?;
        write(root, "unrelated.md", "nothing here\n")?;
        write(root, "subtasks.md", "see subtasks/ for more\n")?;
        write(root, ".gitignore", "target/\nignored.md\n")?;
        git(root, &["init"])?;
        commit_everything(root)?;
        Ok(scratch)
    }

    fn mentioning(scratch: &Path, directory: &Path, regex: &str) -> Result<Vec<PathBuf>, Obstacle> {
        files_mentioning_with(contained_in(scratch), directory, regex)
    }

    /// Tracked and untracked files are listed, from the top level; a file git
    /// ignores, a binary file, and a name that merely ends in `tasks/` are
    /// not.
    #[test]
    fn tracked_and_untracked_files_that_match_are_listed_and_the_rest_are_not() -> TestOutcome {
        let scratch = committed_project("mentions-match")?;
        let root = scratch.path();
        write(root, "notes/todo.md", "move tasks/greet\n")?;
        write(root, "ignored.md", "tasks/ in an ignored file\n")?;
        write(root, "target/debug/x.txt", "tasks/ in build output\n")?;
        write(root, "image.bin", "tasks/\0binary\n")?;

        assert_eq!(
            mentioning(root, root, TASKS),
            Ok(vec![
                PathBuf::from(".github/workflows/ci.yml"),
                PathBuf::from("notes/todo.md"),
                PathBuf::from("readme.md"),
            ])
        );
        Ok(())
    }

    #[test]
    fn nothing_matching_is_an_empty_list_and_not_a_failure() -> TestOutcome {
        let scratch = committed_project("mentions-none")?;

        assert_eq!(
            mentioning(scratch.path(), scratch.path(), "no such phrase anywhere"),
            Ok(Vec::new())
        );
        Ok(())
    }

    /// The whole repository is searched, not the directory the question is
    /// put from, and every path is spelled from the top level.
    #[test]
    fn asked_from_a_subdirectory_it_searches_the_whole_repository() -> TestOutcome {
        let scratch = committed_project("mentions-subdirectory")?;
        let root = scratch.path();
        write(root, "crates/cli/src/lib.rs", "// quiet\n")?;
        commit_everything(root)?;

        assert_eq!(
            mentioning(root, &root.join("crates/cli"), TASKS),
            Ok(vec![
                PathBuf::from(".github/workflows/ci.yml"),
                PathBuf::from("readme.md"),
            ])
        );
        Ok(())
    }

    /// An expression git cannot compile is git's failure, said in git's own
    /// words, and not an empty answer.
    #[test]
    fn an_invalid_expression_is_git_s_failure_in_git_s_words() -> TestOutcome {
        let scratch = committed_project("mentions-invalid")?;

        let result = mentioning(scratch.path(), scratch.path(), "(unclosed");

        assert!(
            matches!(&result, Err(Obstacle::Failed(message)) if !message.is_empty()),
            "expected git's own message, got {result:?}"
        );
        Ok(())
    }

    #[test]
    fn outside_any_repository_is_refused_as_no_repository() -> TestOutcome {
        let scratch = ScratchDir::new("mentions-no-repository")?;
        write(scratch.path(), "readme.md", "tasks/ everywhere\n")?;

        assert_eq!(
            mentioning(scratch.path(), scratch.path(), TASKS),
            Err(Obstacle::NotARepository)
        );
        Ok(())
    }

    #[test]
    fn a_missing_git_binary_is_refused_when_searching() -> TestOutcome {
        let scratch = ScratchDir::new("mentions-git-missing")?;

        assert_eq!(
            files_mentioning_with(
                || Command::new("ritual-test-no-such-git"),
                scratch.path(),
                TASKS
            ),
            Err(Obstacle::GitMissing)
        );
        Ok(())
    }

    #[test]
    #[should_panic(expected = "an empty expression matches every file")]
    fn an_empty_expression_is_a_bug() {
        let _ = files_mentioning_with(|| Command::new("git"), Path::new("/"), "");
    }
}
