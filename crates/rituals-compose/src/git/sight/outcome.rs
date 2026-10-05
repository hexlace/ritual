//! What git would see differently once a set of directories has moved: the
//! types a refusal is built from.

use std::fmt;
use std::path::{Path, PathBuf};

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

/// A file that moves, where it is and where it would be, both spelled from
/// git's top level.
///
/// Only [`ensure_a_move_keeps_what_git_sees`](super::ensure_a_move_keeps_what_git_sees)
/// builds one, from what git reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedFile {
    from: PathBuf,
    to: PathBuf,
}

impl MovedFile {
    pub(super) const fn new(from: PathBuf, to: PathBuf) -> Self {
        Self { from, to }
    }

    /// Where the file is now.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, SeenDifferently};
    /// use rituals_compose::relocation::Relocation;
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// let root = Path::new("/work/project");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/greet")],
    /// );
    /// if let Err(SeenDifferently::OutsideSparseCheckout(files)) =
    ///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
    /// {
    ///     for file in &files {
    ///         println!("{} is not in the sparse checkout", file.from().display());
    ///     }
    /// }
    /// ```
    #[must_use]
    pub fn from(&self) -> &Path {
        &self.from
    }

    /// Where the file would be once the move is done.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, SeenDifferently};
    /// use rituals_compose::relocation::Relocation;
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// let root = Path::new("/work/project");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/greet")],
    /// );
    /// if let Err(SeenDifferently::OutsideSparseCheckout(files)) =
    ///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
    /// {
    ///     for file in &files {
    ///         println!("git would not add {}", file.to().display());
    ///     }
    /// }
    /// ```
    #[must_use]
    pub fn to(&self) -> &Path {
        &self.to
    }
}

/// A file that moves, with the ignore rule that decides what git sees of it:
/// the rule that would ignore it at its new place, or the one that ignores it
/// now.
///
/// Only [`ensure_a_move_keeps_what_git_sees`](super::ensure_a_move_keeps_what_git_sees)
/// builds one, from what git reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredFile {
    file: MovedFile,
    rule: IgnoreRule,
}

impl IgnoredFile {
    pub(super) const fn new(file: MovedFile, rule: IgnoreRule) -> Self {
        Self { file, rule }
    }

    /// The file, where it is and where it would be.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, SeenDifferently};
    /// use rituals_compose::relocation::Relocation;
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// let root = Path::new("/work/project");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/greet")],
    /// );
    /// if let Err(SeenDifferently::WouldBeIgnored(files)) =
    ///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
    /// {
    ///     for ignored in &files {
    ///         println!("git would ignore {}", ignored.file().to().display());
    ///     }
    /// }
    /// ```
    #[must_use]
    pub const fn file(&self) -> &MovedFile {
        &self.file
    }

    /// The rule that decides it.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, SeenDifferently};
    /// use rituals_compose::relocation::Relocation;
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// let root = Path::new("/work/project");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/greet")],
    /// );
    /// if let Err(SeenDifferently::WouldBeIgnored(files)) =
    ///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
    /// {
    ///     for ignored in &files {
    ///         println!("because of {}", ignored.rule());
    ///     }
    /// }
    /// ```
    #[must_use]
    pub const fn rule(&self) -> &IgnoreRule {
        &self.rule
    }
}

/// One line of an ignore file, with where it is.
///
/// Only [`ensure_a_move_keeps_what_git_sees`](super::ensure_a_move_keeps_what_git_sees)
/// builds one, from what git reports.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::git::{self, SeenDifferently};
/// use rituals_compose::relocation::Relocation;
///
/// // Needs a real repository on disk and runs `git`, so this example is
/// // `no_run`.
/// let root = Path::new("/work/project");
/// let relocation = Relocation::new(
///     &root.join("tasks"),
///     &root.join(".rituals"),
///     [root.join("tasks/greet")],
/// );
/// if let Err(SeenDifferently::WouldBeIgnored(files)) =
///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
/// {
///     // Prints, for example, `.*` in .gitignore:2
///     println!("{}", files[0].rule());
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreRule {
    source: PathBuf,
    line: u32,
    pattern: String,
}

impl IgnoreRule {
    pub(super) const fn new(source: PathBuf, line: u32, pattern: String) -> Self {
        Self {
            source,
            line,
            pattern,
        }
    }

    /// The file the rule is written in: spelled from git's top level when it
    /// is inside the repository's work tree, and as git reports it when it is
    /// not, such as a global excludes file.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, SeenDifferently};
    /// use rituals_compose::relocation::Relocation;
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// let root = Path::new("/work/project");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/greet")],
    /// );
    /// if let Err(SeenDifferently::WouldBeIgnored(files)) =
    ///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
    /// {
    ///     println!("edit {}", files[0].rule().source().display());
    /// }
    /// ```
    #[must_use]
    pub fn source(&self) -> &Path {
        &self.source
    }

    /// The line of [`source`](Self::source) the rule is on, counting from
    /// one.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, SeenDifferently};
    /// use rituals_compose::relocation::Relocation;
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// let root = Path::new("/work/project");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/greet")],
    /// );
    /// if let Err(SeenDifferently::WouldBeIgnored(files)) =
    ///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
    /// {
    ///     println!("line {}", files[0].rule().line());
    /// }
    /// ```
    #[must_use]
    pub const fn line(&self) -> u32 {
        self.line
    }

    /// The pattern, as written.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, SeenDifferently};
    /// use rituals_compose::relocation::Relocation;
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// let root = Path::new("/work/project");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/greet")],
    /// );
    /// if let Err(SeenDifferently::WouldBeIgnored(files)) =
    ///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
    /// {
    ///     println!("the pattern is {}", files[0].rule().pattern());
    /// }
    /// ```
    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }
}

impl fmt::Display for IgnoreRule {
    /// Says the rule as `` `pattern` in source:line``.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "`{}` in {}:{}",
            self.pattern,
            self.source.display(),
            self.line
        )
    }
}

/// A file that moves whose attributes would differ at its new place, with
/// both sets.
///
/// Only [`ensure_a_move_keeps_what_git_sees`](super::ensure_a_move_keeps_what_git_sees)
/// builds one, from what git reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeChange {
    file: MovedFile,
    before: Vec<Attribute>,
    after: Vec<Attribute>,
}

impl AttributeChange {
    pub(super) const fn new(
        file: MovedFile,
        before: Vec<Attribute>,
        after: Vec<Attribute>,
    ) -> Self {
        Self {
            file,
            before,
            after,
        }
    }

    /// The file, where it is and where it would be.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, SeenDifferently};
    /// use rituals_compose::relocation::Relocation;
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// let root = Path::new("/work/project");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/greet")],
    /// );
    /// if let Err(SeenDifferently::AttributesWouldChange(changes)) =
    ///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
    /// {
    ///     println!("{}", changes[0].file().from().display());
    /// }
    /// ```
    #[must_use]
    pub const fn file(&self) -> &MovedFile {
        &self.file
    }

    /// The attributes git gives the file now, by name, none of them
    /// unspecified.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, SeenDifferently};
    /// use rituals_compose::relocation::Relocation;
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// let root = Path::new("/work/project");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/greet")],
    /// );
    /// if let Err(SeenDifferently::AttributesWouldChange(changes)) =
    ///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
    /// {
    ///     for attribute in changes[0].before() {
    ///         println!("now: {attribute}");
    ///     }
    /// }
    /// ```
    #[must_use]
    pub fn before(&self) -> &[Attribute] {
        &self.before
    }

    /// The attributes git would give the file at its new place, by name,
    /// none of them unspecified.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, SeenDifferently};
    /// use rituals_compose::relocation::Relocation;
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// let root = Path::new("/work/project");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/greet")],
    /// );
    /// if let Err(SeenDifferently::AttributesWouldChange(changes)) =
    ///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
    /// {
    ///     for attribute in changes[0].after() {
    ///         println!("afterwards: {attribute}");
    ///     }
    /// }
    /// ```
    #[must_use]
    pub fn after(&self) -> &[Attribute] {
        &self.after
    }
}

/// One attribute git gives a path.
///
/// Only [`ensure_a_move_keeps_what_git_sees`](super::ensure_a_move_keeps_what_git_sees)
/// builds one, from what git reports.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::git::{self, SeenDifferently};
/// use rituals_compose::relocation::Relocation;
///
/// // Needs a real repository on disk and runs `git`, so this example is
/// // `no_run`.
/// let root = Path::new("/work/project");
/// let relocation = Relocation::new(
///     &root.join("tasks"),
///     &root.join(".rituals"),
///     [root.join("tasks/greet")],
/// );
/// if let Err(SeenDifferently::AttributesWouldChange(changes)) =
///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
/// {
///     // Prints, for example, `filter=lfs`, `-text` or `binary`.
///     println!("{}", changes[0].before()[0]);
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    name: String,
    state: AttributeState,
}

impl Attribute {
    /// Reads an attribute from the two fields `git check-attr -z` prints for
    /// it: the name, and the value, which is `set`, `unset` or what it was
    /// given. `unspecified` means git gives the path no such attribute, so it
    /// is no attribute at all.
    pub(super) fn from_check_attr(name: &str, value: &str) -> Option<Self> {
        let state = match value {
            "unspecified" => return None,
            "set" => AttributeState::Set,
            "unset" => AttributeState::Unset,
            given => AttributeState::Value(given.to_string()),
        };
        Some(Self {
            name: name.to_string(),
            state,
        })
    }

    /// The attribute's name.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, SeenDifferently};
    /// use rituals_compose::relocation::Relocation;
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// let root = Path::new("/work/project");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/greet")],
    /// );
    /// if let Err(SeenDifferently::AttributesWouldChange(changes)) =
    ///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
    /// {
    ///     println!("{}", changes[0].before()[0].name());
    /// }
    /// ```
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether it is set, unset, or has a value.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::git::{self, AttributeState, SeenDifferently};
    /// use rituals_compose::relocation::Relocation;
    ///
    /// // Needs a real repository on disk and runs `git`, so this example is
    /// // `no_run`.
    /// let root = Path::new("/work/project");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/greet")],
    /// );
    /// if let Err(SeenDifferently::AttributesWouldChange(changes)) =
    ///     git::ensure_a_move_keeps_what_git_sees(&relocation, root)
    /// {
    ///     let filtered = changes[0]
    ///         .before()
    ///         .iter()
    ///         .any(|attribute| matches!(attribute.state(), AttributeState::Value(_)));
    ///     println!("a value is given: {filtered}");
    /// }
    /// ```
    #[must_use]
    pub const fn state(&self) -> &AttributeState {
        &self.state
    }
}

impl fmt::Display for Attribute {
    /// Says the attribute as `.gitattributes` writes it: `name`, `-name` or
    /// `name=value`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.state {
            AttributeState::Set => formatter.write_str(&self.name),
            AttributeState::Unset => write!(formatter, "-{}", self.name),
            AttributeState::Value(value) => write!(formatter, "{}={value}", self.name),
        }
    }
}

/// What an [`Attribute`] says about a path.
///
/// # Examples
///
/// ```
/// use rituals_compose::git::AttributeState;
///
/// let describe = |state: &AttributeState| match state {
///     AttributeState::Set => "set".to_string(),
///     AttributeState::Unset => "unset".to_string(),
///     AttributeState::Value(value) => format!("given {value}"),
/// };
/// assert_eq!(describe(&AttributeState::Value("lfs".to_string())), "given lfs");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeState {
    /// Written as `name`.
    Set,
    /// Written as `-name`.
    Unset,
    /// Written as `name=value`, with the value.
    Value(String),
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        Attribute, AttributeState, IgnoreRule, IgnoredFile, MovedFile, SeenDifferently, Unwatched,
    };
    use crate::git::{Flag, Unanswered};

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

    #[test]
    fn a_rule_is_said_as_its_pattern_in_its_file_and_line() {
        let rule = IgnoreRule::new(PathBuf::from("sub/.gitignore"), 7, ".*".to_string());

        assert_eq!(rule.to_string(), "`.*` in sub/.gitignore:7");
    }

    /// An attribute is written the way `.gitattributes` writes it, and git's
    /// `unspecified` is no attribute at all.
    #[test]
    fn an_attribute_is_read_from_check_attr_and_written_as_gitattributes_does() {
        let written = |name, value| {
            Attribute::from_check_attr(name, value).map(|attribute| attribute.to_string())
        };

        assert_eq!(written("binary", "set").as_deref(), Some("binary"));
        assert_eq!(written("text", "unset").as_deref(), Some("-text"));
        assert_eq!(written("filter", "lfs").as_deref(), Some("filter=lfs"));
        assert_eq!(written("diff", "unspecified"), None);
        assert_eq!(
            Attribute::from_check_attr("eol", "lf").map(|attribute| attribute.state().clone()),
            Some(AttributeState::Value("lf".to_string()))
        );
    }
}
