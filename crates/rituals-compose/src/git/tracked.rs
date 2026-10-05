//! Which files of a project git does not track.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Unanswered, canonical, run_git, top_level_of};

/// Lists the files among `paths` that git does not track, in the order
/// given, each as it was given.
///
/// A file is tracked when the index of the repository `workspace_root` is in
/// has an entry for it. A file git ignores, one it has not been told about,
/// and one outside the repository are not: git could not give any of them
/// back once something edits it. Only the index is asked, so a tracked file
/// that is changed, deleted from the work tree, or matched by an ignore rule
/// is still tracked.
///
/// Every path is absolute and names a file that exists, which is resolved
/// through symbolic links before it is compared. A path that is not UTF-8 is
/// left out of the answer, as git is not asked about it.
///
/// # Examples
///
/// ```no_run
/// use std::path::{Path, PathBuf};
///
/// use rituals_compose::git;
///
/// // Needs a real repository on disk and runs `git`, so this example is
/// // `no_run`.
/// let manifests = [PathBuf::from("/work/project/vendor/x/Cargo.toml")];
/// for manifest in git::files_git_does_not_track(&manifests, Path::new("/work/project"))? {
///     println!("git does not track {}", manifest.display());
/// }
/// # Ok::<(), rituals_compose::git::Unanswered>(())
/// ```
///
/// # Errors
///
/// Returns [`Unanswered::GitMissing`] when `git` cannot be run,
/// [`Unanswered::NotARepository`] when `workspace_root` is not in a git
/// repository, and [`Unanswered::Failed`] in git's own words when anything
/// else goes wrong, including a path that cannot be resolved.
pub fn files_git_does_not_track(
    paths: &[PathBuf],
    workspace_root: &Path,
) -> Result<Vec<PathBuf>, Unanswered> {
    files_git_does_not_track_with(|| Command::new("git"), paths, workspace_root)
}

/// [`files_git_does_not_track`], with `git` started from `new_git`.
fn files_git_does_not_track_with(
    new_git: impl Fn() -> Command,
    paths: &[PathBuf],
    workspace_root: &Path,
) -> Result<Vec<PathBuf>, Unanswered> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let top_level = top_level_of(&new_git, workspace_root)?;

    // Each path as the pathspec git reads it and as it is compared, from the
    // top level. A path the repository does not hold has no spelling: it is
    // untracked, and git is not asked about it.
    let mut spelled: Vec<(&PathBuf, Option<String>)> = Vec::with_capacity(paths.len());
    for path in paths {
        let resolved = canonical(path)?;
        let from_the_top_level = resolved
            .strip_prefix(&top_level)
            .ok()
            .and_then(Path::to_str)
            .map(str::to_string);
        // A path that is not UTF-8 is a file name this tool does not support:
        // it is left out of the answer rather than guessed at.
        if resolved.to_str().is_some() {
            spelled.push((path, from_the_top_level));
        }
    }

    // `:(literal)` makes every character of a name literal, so a `[` or a `*`
    // in a directory's name is not read as a pattern.
    let pathspecs: Vec<String> = spelled
        .iter()
        .filter_map(|(_path, from_the_top_level)| from_the_top_level.as_deref())
        .map(|from_the_top_level| format!(":(literal){from_the_top_level}"))
        .collect();
    let tracked: BTreeSet<String> = if pathspecs.is_empty() {
        BTreeSet::new()
    } else {
        let mut arguments = vec!["ls-files", "-z", "--cached", "--"];
        arguments.extend(pathspecs.iter().map(String::as_str));
        let listed = run_git(&new_git, &top_level, &arguments)?;
        String::from_utf8_lossy(&listed)
            .split('\0')
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect()
    };

    Ok(spelled
        .into_iter()
        .filter(|(_path, from_the_top_level)| {
            from_the_top_level
                .as_ref()
                .is_none_or(|spelling| !tracked.contains(spelling))
        })
        .map(|(path, _spelling)| path.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::files_git_does_not_track_with;
    use crate::git::Unanswered;
    use crate::git::fixture::{commit_everything, git};
    use crate::git::test_support::contained_in;
    use crate::test_support::{ScratchDir, TestOutcome};

    fn write(root: &Path, file: &str, contents: &str) -> TestOutcome {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().ok_or("a fixture file has a directory")?)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    fn untracked(scratch: &Path, files: &[&str]) -> Result<Vec<PathBuf>, Unanswered> {
        let paths: Vec<PathBuf> = files.iter().map(|file| scratch.join(file)).collect();
        files_git_does_not_track_with(contained_in(scratch), &paths, scratch)
    }

    /// `kept/Cargo.toml` is tracked, `vendor/Cargo.toml` is ignored by the
    /// project's own rule, and `loose/Cargo.toml` was never added.
    fn project(tag: &str) -> Result<ScratchDir, Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new(tag)?;
        let root = scratch.path();
        write(root, "kept/Cargo.toml", "# tracked\n")?;
        write(root, ".gitignore", "vendor/\n")?;
        git(root, &["init"])?;
        write(root, "vendor/Cargo.toml", "# ignored\n")?;
        commit_everything(root)?;
        write(root, "loose/Cargo.toml", "# not added\n")?;
        Ok(scratch)
    }

    /// A tracked file is not listed; an ignored one and one never added are,
    /// each as it was given and in the order given.
    #[test]
    fn a_file_the_index_lacks_is_listed_and_a_tracked_one_is_not() -> TestOutcome {
        let scratch = project("tracked-mixed")?;
        let root = scratch.path();

        let found = untracked(
            root,
            &["vendor/Cargo.toml", "kept/Cargo.toml", "loose/Cargo.toml"],
        )?;

        assert_eq!(
            found,
            [
                root.join("vendor/Cargo.toml"),
                root.join("loose/Cargo.toml")
            ]
        );
        Ok(())
    }

    /// A tracked file stays tracked when an ignore rule would match it, and
    /// when it is deleted from the work tree: only the index decides.
    #[test]
    fn a_tracked_file_stays_tracked_whatever_a_rule_says() -> TestOutcome {
        let scratch = project("tracked-forced")?;
        let root = scratch.path();
        write(root, "forced/secret.txt", "# force-added\n")?;
        git(root, &["add", "--force", "forced/secret.txt"])?;
        write(root, ".gitignore", "vendor/\nsecret.txt\n")?;

        assert_eq!(
            untracked(root, &["forced/secret.txt"])?,
            Vec::<PathBuf>::new()
        );
        Ok(())
    }

    /// A file in a repository above the project, or beside it, is outside
    /// the repository the project is in, so git has nothing to say about it
    /// and it is untracked.
    #[test]
    fn a_file_outside_the_repository_is_untracked() -> TestOutcome {
        let scratch = ScratchDir::new("tracked-outside")?;
        let root = scratch.path().join("project");
        write(&root, "kept/Cargo.toml", "# tracked\n")?;
        write(scratch.path(), "elsewhere/Cargo.toml", "# outside\n")?;
        git(&root, &["init"])?;
        commit_everything(&root)?;
        let outside = scratch.path().join("elsewhere/Cargo.toml");

        let found = files_git_does_not_track_with(
            contained_in(&root),
            &[outside.clone(), root.join("kept/Cargo.toml")],
            &root,
        )?;

        assert_eq!(found, [outside]);
        Ok(())
    }

    /// A name with a glob character in it is that name and nothing wider: a
    /// tracked `a[1]/Cargo.toml` does not make `a1/Cargo.toml` tracked.
    #[test]
    fn a_name_with_a_glob_character_is_taken_literally() -> TestOutcome {
        let scratch = ScratchDir::new("tracked-literal")?;
        let root = scratch.path();
        write(root, "a1/Cargo.toml", "# neighbour\n")?;
        git(root, &["init"])?;
        commit_everything(root)?;
        write(root, "a[1]/Cargo.toml", "# not added\n")?;

        let found = untracked(root, &["a[1]/Cargo.toml", "a1/Cargo.toml"])?;

        assert_eq!(found, [root.join("a[1]/Cargo.toml")]);
        Ok(())
    }

    #[test]
    fn no_files_is_no_answer_to_give_and_asks_git_nothing() -> TestOutcome {
        let scratch = ScratchDir::new("tracked-none")?;

        // Not a repository, so any question put to git would fail.
        assert_eq!(untracked(scratch.path(), &[])?, Vec::<PathBuf>::new());
        Ok(())
    }

    #[test]
    fn outside_any_repository_is_refused_as_no_repository() -> TestOutcome {
        let scratch = ScratchDir::new("tracked-no-repository")?;
        write(scratch.path(), "Cargo.toml", "# nowhere\n")?;

        assert_eq!(
            untracked(scratch.path(), &["Cargo.toml"]),
            Err(Unanswered::NotARepository)
        );
        Ok(())
    }

    #[test]
    fn a_missing_git_binary_is_refused() -> TestOutcome {
        let scratch = ScratchDir::new("tracked-git-missing")?;
        write(scratch.path(), "Cargo.toml", "# nowhere\n")?;

        assert_eq!(
            files_git_does_not_track_with(
                || Command::new("ritual-test-no-such-git"),
                &[scratch.path().join("Cargo.toml")],
                scratch.path(),
            ),
            Err(Unanswered::GitMissing)
        );
        Ok(())
    }
}
