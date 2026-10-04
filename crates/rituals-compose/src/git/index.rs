//! What the index records: gitlinks, and the flags that make `git status`
//! stop looking at a file; and what `.gitattributes` says about each.

use std::path::{Path, PathBuf};

use super::Obstacle;

/// A tracked file git has been told not to look at, and which flag says so.
///
/// Only [`ensure_git_can_give_back`](super::ensure_git_can_give_back) builds
/// one, from what git reports, so every value a caller reads is one git
/// vouched for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unwatched {
    path: PathBuf,
    flag: Flag,
}

impl Unwatched {
    pub(super) const fn new(path: PathBuf, flag: Flag) -> Self {
        Self { path, flag }
    }

    /// The file, spelled from git's top level.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, Obstacle};
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// if let Err(Obstacle::Unwatched(files)) =
    ///     git::ensure_git_can_give_back(Path::new(".rituals/lint"), Path::new("."))
    /// {
    ///     for file in &files {
    ///         println!("git ignores changes to {}", file.path().display());
    ///     }
    /// }
    /// ```
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The flag that makes git stop looking at the file.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, Flag, Obstacle};
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// if let Err(Obstacle::Unwatched(files)) =
    ///     git::ensure_git_can_give_back(Path::new(".rituals/lint"), Path::new("."))
    /// {
    ///     let skipped = files.iter().filter(|file| file.flag() == Flag::SkipWorktree);
    ///     println!("{} files are skip-worktree", skipped.count());
    /// }
    /// ```
    #[must_use]
    pub const fn flag(&self) -> Flag {
        self.flag
    }
}

/// The index flags that make `git status` stop looking at a file.
///
/// # Examples
///
/// ```
/// use rituals_compose::git::Flag;
///
/// let name = |flag| match flag {
///     Flag::AssumeUnchanged => "assume-unchanged",
///     Flag::SkipWorktree => "skip-worktree",
/// };
/// assert_eq!(name(Flag::SkipWorktree), "skip-worktree");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    /// `git update-index --assume-unchanged`: an edit is never seen, and
    /// `git checkout` gives back the committed file without it.
    AssumeUnchanged,
    /// `git update-index --skip-worktree`: an edit is never seen, and `git
    /// checkout` does not give the file back at all.
    SkipWorktree,
}

/// One entry of `git ls-files --stage -v -z`, as far as this check reads
/// it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct IndexEntry {
    pub(super) path: PathBuf,
    pub(super) is_gitlink: bool,
    pub(super) flag: Option<Flag>,
}

/// Every entry in `git ls-files --stage -v -z` output.
///
/// An entry is a one-letter tag and a space, then the mode, the object name
/// and the stage, separated by spaces, then a tab and the path, ended by a
/// NUL. The tag is lowercase for an entry marked assume-unchanged and `S`
/// for one marked skip-worktree. An entry this cannot read is a failure
/// rather than something to skip, since skipping it could hide a submodule
/// or a flag.
pub(super) fn parse_index(output: &[u8]) -> Result<Vec<IndexEntry>, Obstacle> {
    let text = String::from_utf8_lossy(output);
    let mut entries = Vec::new();
    for entry in text.split('\0').filter(|entry| !entry.is_empty()) {
        let unreadable = || {
            Obstacle::Failed(format!(
                "git ls-files printed an entry this check cannot read: {entry:?}"
            ))
        };
        let (fields, path) = entry.split_once('\t').ok_or_else(unreadable)?;
        let (tag, rest) = fields.split_once(' ').ok_or_else(unreadable)?;
        let mut tag_letters = tag.chars();
        let (Some(letter), None) = (tag_letters.next(), tag_letters.next()) else {
            return Err(unreadable());
        };
        let flag = if letter.is_ascii_lowercase() {
            Some(Flag::AssumeUnchanged)
        } else if letter == 'S' {
            Some(Flag::SkipWorktree)
        } else {
            None
        };
        entries.push(IndexEntry {
            path: PathBuf::from(path),
            is_gitlink: rest.starts_with("160000 "),
            flag,
        });
    }
    Ok(entries)
}

/// Every path and its `filter` value in `git check-attr -z filter` output.
///
/// Each answer is three NUL-ended fields: the path, the attribute's name,
/// and its value — `unspecified`, `unset`, `set`, or the value it was given.
/// Output that does not come in threes is a failure, since misreading it
/// could pair a file with the wrong filter.
pub(super) fn parse_attributes(output: &[u8]) -> Result<Vec<(PathBuf, String)>, Obstacle> {
    let text = String::from_utf8_lossy(output);
    let fields: Vec<&str> = text.split_terminator('\0').collect();
    let answers = fields.chunks_exact(3);
    if !answers.remainder().is_empty() {
        return Err(Obstacle::Failed(format!(
            "git check-attr printed output this check cannot read: {text:?}"
        )));
    }
    Ok(answers
        .map(|answer| (PathBuf::from(answer[0]), answer[2].to_string()))
        .collect())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Flag, IndexEntry, parse_attributes, parse_index};
    use crate::git::Obstacle;
    use crate::test_support::TestOutcome;

    #[test]
    fn the_index_is_read_for_gitlinks_and_flags() -> TestOutcome {
        let output = b"H 100644 0123456789abcdef0123456789abcdef01234567 0\tsrc/lib.rs\0\
            H 160000 89abcdef0123456789abcdef0123456789abcdef 0\tvendor/upstream\0\
            h 100755 0123456789abcdef0123456789abcdef01234567 0\trun.sh\0\
            S 100644 0123456789abcdef0123456789abcdef01234567 0\tlocal.toml\0";
        let entry = |path: &str, is_gitlink, flag| IndexEntry {
            path: PathBuf::from(path),
            is_gitlink,
            flag,
        };
        assert_eq!(
            parse_index(output).map_err(|obstacle| format!("{obstacle:?}"))?,
            [
                entry("src/lib.rs", false, None),
                entry("vendor/upstream", true, None),
                entry("run.sh", false, Some(Flag::AssumeUnchanged)),
                entry("local.toml", false, Some(Flag::SkipWorktree)),
            ]
        );
        Ok(())
    }

    #[test]
    fn an_index_entry_this_cannot_read_is_a_failure_not_a_skip() {
        for output in [
            &b"H 160000 89abcdef 0\0"[..],
            &b"160000 89abcdef 0\tno-tag\0"[..],
            &b"HS 100644 89abcdef 0\ttwo-letter-tag\0"[..],
        ] {
            let result = parse_index(output);
            assert!(
                matches!(
                    &result,
                    Err(Obstacle::Failed(message)) if message.contains("cannot read")
                ),
                "expected {output:?} to be refused, got {result:?}"
            );
        }
    }

    #[test]
    fn attributes_are_read_in_threes() -> TestOutcome {
        let output = b"a.cfg\0filter\0strip\0b.rs\0filter\0unspecified\0";
        assert_eq!(
            parse_attributes(output).map_err(|obstacle| format!("{obstacle:?}"))?,
            [
                (PathBuf::from("a.cfg"), "strip".to_string()),
                (PathBuf::from("b.rs"), "unspecified".to_string()),
            ]
        );
        let result = parse_attributes(b"a.cfg\0filter\0");
        assert!(
            matches!(&result, Err(Obstacle::Failed(message)) if message.contains("cannot read")),
            "expected output not in threes to be refused, got {result:?}"
        );
        Ok(())
    }
}
