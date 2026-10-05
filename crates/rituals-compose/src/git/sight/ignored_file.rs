//! A file that moves and the ignore rule that decides what git sees of it.

use super::{IgnoreRule, MovedFile};

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
