//! One line of an ignore file.

use std::fmt;
use std::path::{Path, PathBuf};

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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::IgnoreRule;

    #[test]
    fn a_rule_is_said_as_its_pattern_in_its_file_and_line() {
        let rule = IgnoreRule::new(PathBuf::from("sub/.gitignore"), 7, ".*".to_string());

        assert_eq!(rule.to_string(), "`.*` in sub/.gitignore:7");
    }
}
