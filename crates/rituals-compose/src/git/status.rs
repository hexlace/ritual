//! What `git status` says is untracked, ignored or changed.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Obstacle, run_git};

/// Refuses when anything `pathspec` matches, asked from `asked_in`, is
/// untracked, ignored, or changed since the last commit, naming each.
pub(super) fn ensure_nothing_dirty(
    new_git: &impl Fn() -> Command,
    asked_in: &Path,
    pathspec: &str,
) -> Result<(), Obstacle> {
    let status = run_git(
        new_git,
        asked_in,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
            "--",
            pathspec,
        ],
    )?;
    // Git names each path in the status from the top level, whichever
    // directory it was asked from.
    let dirty: Vec<PathBuf> = parse_porcelain(&status)?
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if dirty.is_empty() {
        Ok(())
    } else {
        Err(Obstacle::Dirty(dirty))
    }
}

/// The path of every entry in `git status --porcelain=v1 -z` output.
///
/// An entry is two status letters, a space, then the path, ended by a NUL.
/// A rename or a copy is followed by one more NUL-ended field, the path it
/// came from, which is not an entry of its own. An entry too short to hold
/// its status is a failure rather than something to skip, since skipping it
/// could hide a file git cannot give back.
pub(super) fn parse_porcelain(output: &[u8]) -> Result<Vec<String>, Obstacle> {
    let text = String::from_utf8_lossy(output);
    let mut fields = text.split('\0');
    let mut paths = Vec::new();
    while let Some(field) = fields.next() {
        if field.is_empty() {
            // The output ends with a NUL, which leaves one empty field.
            continue;
        }
        let Some((status, path)) = field.split_at_checked(3) else {
            return Err(Obstacle::Failed(format!(
                "git status printed an entry this check cannot read: {field:?}"
            )));
        };
        if status.contains(['R', 'C']) {
            fields.next();
        }
        paths.push(path.to_string());
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::parse_porcelain;
    use crate::git::Obstacle;
    use crate::test_support::TestOutcome;

    /// Exercises the parser on one entry of each kind `--porcelain=v1 -z`
    /// prints that `remove` has to refuse over: untracked, ignored, modified
    /// in the work tree, modified in the index, added, and deleted. Each
    /// path is named exactly once and in the order git printed it.
    #[test]
    fn porcelain_names_every_untracked_ignored_and_changed_path() -> TestOutcome {
        // The continuation strips the next line's leading whitespace, so the
        // space that starts ` D` is written as an escape.
        let output = b"?? untracked.txt\0!! target/\0 M worktree.rs\0M  staged.rs\0A  added.rs\0\
            \x20D gone.rs\0";
        assert_eq!(
            parse_porcelain(output).map_err(|obstacle| format!("{obstacle:?}"))?,
            [
                "untracked.txt",
                "target/",
                "worktree.rs",
                "staged.rs",
                "added.rs",
                "gone.rs"
            ]
        );
        Ok(())
    }

    /// In `-z` mode a rename or a copy is followed by the path it came from
    /// as a field with no status of its own. Reading that field as an entry
    /// would name a path that is not a file in the directory, or fail on it.
    #[test]
    fn a_rename_entry_consumes_its_original_path() -> TestOutcome {
        let output = b"R  new.rs\0old.rs\0C  copy.rs\0source.rs\0?? other.txt\0";
        assert_eq!(
            parse_porcelain(output).map_err(|obstacle| format!("{obstacle:?}"))?,
            ["new.rs", "copy.rs", "other.txt"]
        );
        Ok(())
    }

    #[test]
    fn an_entry_too_short_to_hold_a_status_is_a_failure_not_a_skip() {
        let result = parse_porcelain(b"?? fine.txt\0x\0");
        assert!(
            matches!(&result, Err(Obstacle::Failed(message)) if message.contains("\"x\"")),
            "expected the unreadable entry to be named, got {result:?}"
        );
    }
}
