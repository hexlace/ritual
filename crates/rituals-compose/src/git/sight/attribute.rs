//! One attribute git gives a path.

use std::fmt;

use super::AttributeState;

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

#[cfg(test)]
mod tests {
    use super::Attribute;
    use crate::git::sight::AttributeState;

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
