//! Every file under the directories that move: tracked, untracked and
//! ignored, and how the index stands towards each.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use crate::git::{Flag, Unanswered, run_git};

/// How the index stands towards an entry of the listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Standing {
    /// The index has the file and git is watching it.
    Tracked,
    /// The index has the file with a flag that stops `git status` looking at
    /// it.
    Flagged(Flag),
    /// The index does not have it: it is untracked or ignored, which the
    /// ignore rules say.
    Untracked,
}

/// One entry of `git ls-files -v --cached --others`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Entry {
    /// The path from git's top level, with a trailing `/` when git lists a
    /// whole directory, which it does for a repository of its own.
    pub(super) path: String,
    pub(super) standing: Standing,
}

impl Entry {
    /// Whether this is a whole directory rather than a file: a nested
    /// repository, which git stores as a gitlink or not at all.
    pub(super) fn is_a_directory(&self) -> bool {
        self.path.ends_with('/')
    }
}

/// Lists every file at or under `directories`, each spelled from
/// `top_level`, tracked, untracked and ignored alike, in the order git
/// gives them.
///
/// `:(literal)` makes every character of a name literal, so a `[` or a `*` in
/// a directory's name is not read as a pattern. A directory that is a
/// symbolic link is one entry, the link, as git keeps it.
pub(super) fn list(
    new_git: &impl Fn() -> Command,
    top_level: &Path,
    directories: &[String],
) -> Result<Vec<Entry>, Unanswered> {
    if directories.is_empty() {
        return Ok(Vec::new());
    }
    let pathspecs: Vec<String> = directories
        .iter()
        .map(|directory| format!(":(literal){directory}"))
        .collect();
    let mut arguments = vec!["ls-files", "-z", "-v", "--cached", "--others", "--"];
    arguments.extend(pathspecs.iter().map(String::as_str));
    parse_listing(&run_git(new_git, top_level, &arguments)?)
}

/// Every entry in `git ls-files -z -v --cached --others` output.
///
/// An entry is a one-letter tag, a space and the path, ended by a NUL. The
/// tag is `H` for a tracked file, lowercase for one marked assume-unchanged,
/// `S` for one marked skip-worktree, `M` for one with unresolved conflicts,
/// and `?` for one that is not tracked. An entry this cannot read, or a tag
/// it does not know, is a failure rather than something to skip, since
/// skipping it could hide a file that moves. A file listed once for each
/// conflicting stage is one entry.
pub(super) fn parse_listing(output: &[u8]) -> Result<Vec<Entry>, Unanswered> {
    let text = String::from_utf8_lossy(output);
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut entries = Vec::new();
    for entry in text.split('\0').filter(|entry| !entry.is_empty()) {
        let unreadable = || {
            Unanswered::Failed(format!(
                "git ls-files printed an entry this check cannot read: {entry:?}"
            ))
        };
        let (tag, path) = entry.split_once(' ').ok_or_else(unreadable)?;
        let mut letters = tag.chars();
        let (Some(letter), None) = (letters.next(), letters.next()) else {
            return Err(unreadable());
        };
        let standing = match letter {
            '?' => Standing::Untracked,
            'H' | 'M' => Standing::Tracked,
            'S' => Standing::Flagged(Flag::SkipWorktree),
            lowercase if lowercase.is_ascii_lowercase() => Standing::Flagged(Flag::AssumeUnchanged),
            _ => return Err(unreadable()),
        };
        if seen.insert(path) {
            entries.push(Entry {
                path: path.to_string(),
                standing,
            });
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::{Entry, Standing, parse_listing};
    use crate::git::{Flag, Unanswered};

    #[test]
    fn a_listing_is_read_for_how_the_index_stands() {
        let output = b"H tasks/greet/Cargo.toml\0\
            ? tasks/greet/.env\0\
            h tasks/greet/run.sh\0\
            S tasks/greet/local.toml\0\
            ? tasks/greet/vendor/up/\0";

        let entries = parse_listing(output).expect("a listing git could have printed is read");

        let expected = |path: &str, standing| Entry {
            path: path.to_string(),
            standing,
        };
        assert_eq!(
            entries,
            [
                expected("tasks/greet/Cargo.toml", Standing::Tracked),
                expected("tasks/greet/.env", Standing::Untracked),
                expected(
                    "tasks/greet/run.sh",
                    Standing::Flagged(Flag::AssumeUnchanged)
                ),
                expected(
                    "tasks/greet/local.toml",
                    Standing::Flagged(Flag::SkipWorktree)
                ),
                expected("tasks/greet/vendor/up/", Standing::Untracked),
            ]
        );
        assert!(!entries[0].is_a_directory());
        assert!(entries[4].is_a_directory());
    }

    /// An unmerged file is listed once per stage, and is one file.
    #[test]
    fn a_file_listed_for_each_of_its_conflicting_stages_is_one_entry() {
        let output = b"M tasks/greet/a.rs\0M tasks/greet/a.rs\0M tasks/greet/a.rs\0";

        assert_eq!(parse_listing(output).map(|entries| entries.len()), Ok(1));
    }

    #[test]
    fn an_entry_this_cannot_read_is_a_failure_not_a_skip() {
        for output in [
            &b"no-tag-and-no-space\0"[..],
            &b"HS tasks/greet/two-letter-tag\0"[..],
            &b" tasks/greet/empty-tag\0"[..],
            &b"K tasks/greet/a-tag-git-does-not-print-here\0"[..],
            &b"7 tasks/greet/a-digit\0"[..],
        ] {
            let result = parse_listing(output);
            assert!(
                matches!(
                    &result,
                    Err(Unanswered::Failed(message)) if message.contains("cannot read")
                ),
                "expected {output:?} to be refused, got {result:?}"
            );
        }
    }

    #[test]
    fn nothing_listed_is_an_empty_listing() {
        assert_eq!(parse_listing(b""), Ok(Vec::new()));
    }
}
