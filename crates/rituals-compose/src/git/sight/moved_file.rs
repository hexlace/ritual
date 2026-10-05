//! A file that moves, as git is asked about it.

use std::path::{Path, PathBuf};

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
