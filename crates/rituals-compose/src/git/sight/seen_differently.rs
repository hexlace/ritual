//! How git would see the files of a move differently: the refusal built from
//! what it reports.

use std::fmt;

use super::{AttributeChange, IgnoredFile, MovedFile};
use crate::git::{Unanswered, Unwatched, counted};

// See `Unanswered` for why the variants are public and the enum is
// exhaustive, and why none of this is a struct with a private kind.
/// How git would see the files of a move differently afterwards, or why it
/// could not be asked.
///
/// Holds [`Unanswered`] for the three facts every question here can fail on,
/// and for each other variant every file one kind of difference holds for.
/// Only the first kind that holds for any file is reported, and all of its
/// files with it: each kind has its own remedy, and one refusal that gave two
/// would be harder to act on. The kinds come in the order they are checked:
/// ignored afterwards, no longer ignored, other attributes, outside the
/// sparse checkout, and told not to be looked at.
///
/// Every path a variant holds is spelled from git's top level, the form the
/// `:/` pathspec takes. The variants state git's facts and no remedy: a
/// caller builds its own refusal from the one it receives, in its own words.
///
/// # Examples
///
/// ```
/// use rituals_compose::git::{SeenDifferently, Unanswered};
///
/// assert_eq!(
///     SeenDifferently::from(Unanswered::NotARepository).to_string(),
///     "the directory is not in a git repository"
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeenDifferently {
    /// Git could not answer at all.
    Unanswered(Unanswered),
    /// Files git sees now, tracked or not ignored, that it would ignore at
    /// their new place, with the rule that would: a commit after the move
    /// would leave out what is committed now.
    WouldBeIgnored(Vec<IgnoredFile>),
    /// Files git ignores now that nothing would ignore at their new place,
    /// with the rule that ignores each now: a commit after the move would add
    /// them.
    WouldNoLongerBeIgnored(Vec<IgnoredFile>),
    /// Files git sees at both places that would have other attributes at the
    /// new one: a filter, an end-of-line setting or anything else
    /// `.gitattributes` decides by path.
    AttributesWouldChange(Vec<AttributeChange>),
    /// Files git sees that the checkout's sparse-checkout patterns do not
    /// include at their new place, so git would not add them.
    OutsideSparseCheckout(Vec<MovedFile>),
    /// Tracked files git has been told not to look at, so an edit to one
    /// would be carried by a commit after the move without `git status`
    /// showing it.
    Unwatched(Vec<Unwatched>),
}

impl From<Unanswered> for SeenDifferently {
    fn from(unanswered: Unanswered) -> Self {
        Self::Unanswered(unanswered)
    }
}

impl fmt::Display for SeenDifferently {
    /// Says git's fact in lowercase, with no remedy and no trailing
    /// punctuation, so a caller can put it inside a sentence of its own.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unanswered(unanswered) => fmt::Display::fmt(unanswered, formatter),
            Self::WouldBeIgnored(files) => write!(
                formatter,
                "git would ignore {} it sees now",
                counted(files.len(), "file")
            ),
            Self::WouldNoLongerBeIgnored(files) => write!(
                formatter,
                "git would stop ignoring {} it ignores now",
                counted(files.len(), "file")
            ),
            Self::AttributesWouldChange(files) => write!(
                formatter,
                "git would give {} other attributes",
                counted(files.len(), "file")
            ),
            Self::OutsideSparseCheckout(files) => write!(
                formatter,
                "{} would be outside the sparse-checkout patterns",
                counted(files.len(), "file")
            ),
            Self::Unwatched(files) => write!(
                formatter,
                "git has been told not to look at {}",
                counted(files.len(), "tracked file")
            ),
        }
    }
}

impl std::error::Error for SeenDifferently {}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::SeenDifferently;
    use crate::git::sight::{IgnoreRule, IgnoredFile, MovedFile};
    use crate::git::{Flag, Unanswered, Unwatched};

    /// Every variant renders as git's fact, in lowercase, with no remedy and
    /// no trailing full stop, so a caller can build a sentence around it.
    #[test]
    fn every_seen_differently_says_git_s_fact_in_lowercase_with_no_remedy() {
        let moved = MovedFile::new(PathBuf::from("a"), PathBuf::from("b"));
        let ignored = IgnoredFile::new(
            moved.clone(),
            IgnoreRule::new(PathBuf::from(".gitignore"), 1, "b".to_string()),
        );
        let unwatched = Unwatched::new(PathBuf::from("a"), Flag::AssumeUnchanged);
        let cases = [
            (
                SeenDifferently::Unanswered(Unanswered::GitMissing),
                "`git` could not be run",
            ),
            (
                SeenDifferently::WouldBeIgnored(vec![ignored.clone()]),
                "git would ignore 1 file it sees now",
            ),
            (
                SeenDifferently::WouldBeIgnored(vec![ignored.clone(), ignored.clone()]),
                "git would ignore 2 files it sees now",
            ),
            (
                SeenDifferently::WouldNoLongerBeIgnored(vec![ignored]),
                "git would stop ignoring 1 file it ignores now",
            ),
            (
                SeenDifferently::AttributesWouldChange(Vec::new()),
                "git would give 0 files other attributes",
            ),
            (
                SeenDifferently::OutsideSparseCheckout(vec![moved]),
                "1 file would be outside the sparse-checkout patterns",
            ),
            (
                SeenDifferently::Unwatched(vec![unwatched]),
                "git has been told not to look at 1 tracked file",
            ),
        ];
        for (seen_differently, expected) in cases {
            assert_eq!(seen_differently.to_string(), expected);
            assert!(
                !expected.ends_with('.'),
                "{expected:?} must not end in a full stop"
            );
        }
    }
}
