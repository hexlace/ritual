//! A file whose attributes would differ at its new place.

use super::{Attribute, MovedFile};

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
