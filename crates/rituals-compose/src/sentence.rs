//! One house style for a refusal that names more than one thing.
//!
//! The value is not the handful of lines of code; it is that every refusal
//! in this framework that lists more than one thing reads the same way. A
//! task crate writing its own copy would be a second house style nothing
//! would notice diverging from the first.

/// Joins `items` the way a sentence lists things: one alone, or a
/// comma-separated run with "and" before the last — never a bare debug
/// list.
///
/// The empty case is defined, not assumed away: an empty slice joins to an
/// empty string, since nothing about this function's own contract depends
/// on any particular caller's behaviour.
///
/// # Examples
///
/// ```
/// use rituals_compose::sentence::join_with_and;
///
/// assert_eq!(join_with_and(&["tasks/lint".to_string()]), "tasks/lint");
/// assert_eq!(
///     join_with_and(&["tasks/lint".to_string(), "tasks/fmt".to_string()]),
///     "tasks/lint and tasks/fmt"
/// );
/// ```
#[must_use]
pub fn join_with_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [initial @ .., last] => format!("{} and {last}", initial.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::join_with_and;

    #[test]
    fn an_empty_slice_joins_to_an_empty_string() {
        assert_eq!(join_with_and(&[]), "");
    }

    #[test]
    fn one_item_is_itself_with_no_conjunction() {
        assert_eq!(join_with_and(&["a".to_string()]), "a");
    }

    #[test]
    fn two_items_are_joined_with_and_and_no_comma() {
        assert_eq!(
            join_with_and(&["a".to_string(), "b".to_string()]),
            "a and b"
        );
    }

    #[test]
    fn three_items_are_comma_joined_with_and_before_the_last() {
        assert_eq!(
            join_with_and(&["a".to_string(), "b".to_string(), "c".to_string()]),
            "a, b and c"
        );
    }
}
