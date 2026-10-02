//! The one rule for the name Rust sees.
//!
//! Cargo turns a hyphen in a dependency key into an underscore to make the
//! extern-crate identifier rustc is given, so `ritual-task` and `ritual_task`
//! are one name to Rust. Every place that compares a key with what Cargo
//! reports, or tells a person about the other spelling, goes through here, so
//! the rule is written once.

/// Returns the name Rust sees for a dependency key or package name: the
/// spelling with every hyphen read as an underscore.
///
/// # Examples
///
/// ```
/// use rituals_compose::rust_name::extern_identifier;
///
/// assert_eq!(extern_identifier("ritual-task"), "ritual_task");
/// assert_eq!(extern_identifier("lint"), "lint");
/// ```
#[must_use]
pub fn extern_identifier(spelling: &str) -> String {
    spelling.replace('-', "_")
}

/// Returns the clause that tells a person the other spelling of `spelling`,
/// led by a space, or an empty string when there is no other spelling.
///
/// # Examples
///
/// ```
/// use rituals_compose::rust_name::other_spelling_clause;
///
/// assert_eq!(
///     other_spelling_clause("a-b"),
///     " (or `a_b`, which Rust reads as the same name)"
/// );
/// assert_eq!(other_spelling_clause("ab"), "");
/// ```
#[must_use]
pub fn other_spelling_clause(spelling: &str) -> String {
    let identifier = extern_identifier(spelling);
    if identifier == spelling {
        String::new()
    } else {
        format!(" (or `{identifier}`, which Rust reads as the same name)")
    }
}

#[cfg(test)]
mod tests {
    use super::{extern_identifier, other_spelling_clause};

    #[test]
    fn every_hyphen_reads_as_an_underscore() {
        assert_eq!(extern_identifier("a-b-c"), "a_b_c");
    }

    #[test]
    fn a_name_with_no_hyphen_is_unchanged() {
        assert_eq!(extern_identifier("a_b"), "a_b");
        assert_eq!(extern_identifier(""), "");
    }

    #[test]
    fn the_other_spelling_is_named_only_when_there_is_one() {
        assert_eq!(
            other_spelling_clause("a-b"),
            " (or `a_b`, which Rust reads as the same name)"
        );
        assert_eq!(other_spelling_clause("a_b"), "");
    }
}
