//! Removing one entry from a TOML array without disturbing the text around
//! it.
//!
//! The counterpart of what `push_matching_style` does for an append: the
//! removed entry's own line goes, including a comment written on it, and
//! every line the entry did not own stays.

use toml_edit::Array;

use super::raw_text;

/// How an entry sits in its array, which decides where the text around it
/// goes when it is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    /// The only entry.
    Only,
    /// An entry with another after it.
    Followed,
    /// The last of several, with a comma after it.
    LastWithComma,
    /// The last of several, with nothing after it before `]`.
    LastWithoutComma,
}

impl Position {
    fn of(array: &Array, index: usize) -> Self {
        match (
            array.len(),
            index + 1 == array.len(),
            array.trailing_comma(),
        ) {
            (1, _, _) => Self::Only,
            (_, false, _) => Self::Followed,
            (_, true, true) => Self::LastWithComma,
            (_, true, false) => Self::LastWithoutComma,
        }
    }
}

/// The text between one entry's comma, or the `[`, and the entry itself,
/// taken apart by who it belongs to.
///
/// `toml_edit` keeps all of it in the entry's prefix: the comment written on
/// the previous entry's line (`head`, through the first newline), the
/// comment lines written above this entry (`above`), and the indent that
/// puts this entry at the start of its line (`indent`). A prefix that never
/// breaks a line is only the space between two entries on one line, which
/// is all `indent`.
struct Prefix {
    head: String,
    above: String,
    indent: String,
    breaks_a_line: bool,
}

impl Prefix {
    fn of(text: &str) -> Self {
        let (Some(first), Some(last)) = (text.find('\n'), text.rfind('\n')) else {
            return Self {
                head: String::new(),
                above: String::new(),
                indent: text.to_string(),
                breaks_a_line: false,
            };
        };
        let above_starts = first + 1;
        let above = if last >= above_starts {
            text[above_starts..=last].to_string()
        } else {
            String::new()
        };
        Self {
            head: text[..=first].to_string(),
            above,
            indent: text[last + 1..].to_string(),
            breaks_a_line: true,
        }
    }
}

/// `text` after its first newline, or all of it when it never breaks a line.
///
/// A closing text, the text before `]` or after a last comma, begins with the
/// end of the removed entry's own line: its comment and its newline. What
/// follows is lines that belong to nobody in particular.
fn after_the_first_line(text: &str) -> String {
    text.find('\n')
        .map_or_else(|| text.to_string(), |first| text[first + 1..].to_string())
}

/// Removes the entry at `index` from `array`, the counterpart of
/// `push_matching_style`: the entry's own line goes, including a comment
/// written on it, and nothing else does.
///
/// `toml_edit` splits what surrounds an entry across its neighbours. The
/// comment on an entry's line is stored in the *next* entry's prefix, or in
/// the array's `trailing` text for the last entry that has a comma, or in
/// the entry's own suffix for the last one that has not. So each case moves
/// the text that is not the removed entry's own onto whatever now follows
/// the gap, and drops the rest:
///
/// - With an entry after it, the next entry takes the removed one's place:
///   the comment on the line before it, and the comment lines above it,
///   stay in front; the comment on the removed entry's own line, which was in
///   the next entry's prefix, goes.
/// - The last entry with a comma is taken out of the array's `trailing` text
///   the same way.
/// - The last entry without a comma leaves the one before it last, and
///   without a comma, so the comment on its line moves from where the comma
///   was to before the `]`.
/// - The only entry leaves an empty array whose `trailing` text holds
///   whatever is left, so `[\n    "a",\n]` becomes `[\n]`.
///
/// # Panics
///
/// Panics when `index` is not a position in `array`.
pub(super) fn remove_matching_style(array: &mut Array, index: usize) {
    assert!(
        index < array.len(),
        "cannot remove entry {index} of an array of {}",
        array.len()
    );

    match Position::of(array, index) {
        Position::Only => remove_the_only_entry(array),
        Position::Followed => remove_an_entry_with_a_successor(array, index),
        Position::LastWithComma => remove_the_last_entry_after_which_a_comma_sits(array, index),
        Position::LastWithoutComma => remove_the_last_entry_with_no_comma(array, index),
    }
}

fn remove_an_entry_with_a_successor(array: &mut Array, index: usize) {
    let removed = Prefix::of(&prefix_of(array, index));
    let next = Prefix::of(&prefix_of(array, index + 1));

    // An entry that starts its own line keeps its own indent; one sharing a
    // line with its neighbours takes the removed entry's place, so that
    // removing the first of `["a", "b"]` leaves `["b"]` and not `[ "b"]`.
    let indent = if next.breaks_a_line {
        &next.indent
    } else {
        &removed.indent
    };
    let prefix = format!("{}{}{}{indent}", removed.head, removed.above, next.above);

    if let Some(next) = array.get_mut(index + 1) {
        next.decor_mut().set_prefix(prefix);
    }
    array.remove(index);
}

fn remove_the_last_entry_after_which_a_comma_sits(array: &mut Array, index: usize) {
    let removed = Prefix::of(&prefix_of(array, index));
    let trailing = raw_text(Some(array.trailing()));

    let trailing = format!(
        "{}{}{}",
        removed.head,
        removed.above,
        after_the_first_line(&trailing)
    );
    array.remove(index);
    array.set_trailing(trailing);
}

fn remove_the_last_entry_with_no_comma(array: &mut Array, index: usize) {
    let removed = Prefix::of(&prefix_of(array, index));
    let closing = suffix_of(array, index);
    let previous_suffix = suffix_of(array, index - 1);

    let suffix = format!(
        "{previous_suffix}{}{}{}",
        removed.head,
        removed.above,
        after_the_first_line(&closing)
    );
    array.remove(index);
    if let Some(previous) = array.get_mut(index - 1) {
        previous.decor_mut().set_suffix(suffix);
    }
}

fn remove_the_only_entry(array: &mut Array) {
    let removed = Prefix::of(&prefix_of(array, 0));
    let closing = if array.trailing_comma() {
        raw_text(Some(array.trailing()))
    } else {
        suffix_of(array, 0)
    };

    let inside = format!(
        "{}{}{}",
        removed.head,
        removed.above,
        after_the_first_line(&closing)
    );
    array.clear();
    array.set_trailing_comma(false);
    array.set_trailing(inside);
}

/// The prefix text of the entry at `index`.
fn prefix_of(array: &Array, index: usize) -> String {
    raw_text(array.get(index).and_then(|value| value.decor().prefix()))
}

/// The suffix text of the entry at `index`.
fn suffix_of(array: &Array, index: usize) -> String {
    raw_text(array.get(index).and_then(|value| value.decor().suffix()))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use toml_edit::DocumentMut;

    use super::remove_matching_style;
    use crate::test_support::TestOutcome;

    /// Parses `source` as `members = <array>`, removes the entry at `index`
    /// with [`remove_matching_style`], and asserts both that the rendered
    /// text is exactly `expected` and that re-parsing it yields the entries
    /// `expected_members` — the re-parse is the oracle that catches a comma
    /// swallowed by a comment, which renders as plausible-looking text but
    /// does not parse back to the same entries.
    fn assert_removal_renders(
        source: &str,
        index: usize,
        expected: &str,
        expected_members: &[&str],
    ) -> TestOutcome {
        let mut document: DocumentMut = format!("members = {source}\n").parse()?;
        let array = document
            .get_mut("members")
            .and_then(toml_edit::Item::as_array_mut)
            .expect("each fixture's members key is an array");
        remove_matching_style(array, index);

        let rendered = document.to_string();
        assert_eq!(
            rendered,
            format!("members = {expected}\n"),
            "rendered text did not match"
        );
        assert_eq!(
            members_of(&rendered)?,
            expected_members,
            "re-parsed entries did not match"
        );
        Ok(())
    }

    /// The `members` entries `text` parses to.
    fn members_of(text: &str) -> Result<Vec<String>, Box<dyn Error>> {
        let reparsed: DocumentMut = text.parse()?;
        Ok(reparsed
            .get("members")
            .and_then(toml_edit::Item::as_array)
            .map(|array| {
                array
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    // Inline arrays.

    #[test]
    fn removing_the_only_entry_of_an_inline_array_leaves_it_empty() -> TestOutcome {
        assert_removal_renders("[\"a\"]", 0, "[]", &[])
    }

    #[test]
    fn removing_the_first_entry_of_an_inline_array() -> TestOutcome {
        assert_removal_renders("[\"a\", \"b\"]", 0, "[\"b\"]", &["b"])
    }

    #[test]
    fn removing_a_middle_entry_of_an_inline_array() -> TestOutcome {
        assert_removal_renders("[\"a\", \"b\", \"c\"]", 1, "[\"a\", \"c\"]", &["a", "c"])
    }

    #[test]
    fn removing_the_last_entry_of_an_inline_array() -> TestOutcome {
        assert_removal_renders("[\"a\", \"b\"]", 1, "[\"a\"]", &["a"])
    }

    #[test]
    fn removing_from_an_inline_array_with_a_trailing_comma() -> TestOutcome {
        assert_removal_renders("[\"a\", \"b\",]", 1, "[\"a\",]", &["a"])?;
        assert_removal_renders("[\"a\", \"b\",]", 0, "[\"b\",]", &["b"])?;
        assert_removal_renders("[\"a\",]", 0, "[]", &[])
    }

    #[test]
    fn removing_from_an_inline_array_padded_inside_its_brackets() -> TestOutcome {
        // The space before `]` belongs to the array's closing, not to the
        // removed entry, so it stays.
        assert_removal_renders("[ \"a\", \"b\" ]", 1, "[ \"a\" ]", &["a"])?;
        assert_removal_renders("[ \"a\", \"b\" ]", 0, "[ \"b\" ]", &["b"])?;
        assert_removal_renders("[ \"a\", \"b\", ]", 1, "[ \"a\", ]", &["a"])?;
        assert_removal_renders("[ \"a\" ]", 0, "[ ]", &[])
    }

    // Multi-line arrays.

    #[test]
    fn removing_the_first_middle_and_last_entry_of_a_multi_line_array() -> TestOutcome {
        let source = "[\n    \"a\",\n    \"b\",\n    \"c\",\n]";
        assert_removal_renders(source, 0, "[\n    \"b\",\n    \"c\",\n]", &["b", "c"])?;
        assert_removal_renders(source, 1, "[\n    \"a\",\n    \"c\",\n]", &["a", "c"])?;
        assert_removal_renders(source, 2, "[\n    \"a\",\n    \"b\",\n]", &["a", "b"])
    }

    #[test]
    fn removing_the_only_entry_of_a_multi_line_array_leaves_it_empty() -> TestOutcome {
        assert_removal_renders("[\n    \"a\",\n]", 0, "[\n]", &[])?;
        assert_removal_renders("[\n    \"a\"\n]", 0, "[\n]", &[])
    }

    #[test]
    fn removing_from_a_multi_line_array_with_no_trailing_comma() -> TestOutcome {
        let source = "[\n    \"a\",\n    \"b\"\n]";
        assert_removal_renders(source, 1, "[\n    \"a\"\n]", &["a"])?;
        assert_removal_renders(source, 0, "[\n    \"b\"\n]", &["b"])
    }

    #[test]
    fn removing_from_a_multi_line_array_reproduces_a_tab_indent() -> TestOutcome {
        let source = "[\n\t\"a\",\n\t\"b\",\n]";
        assert_removal_renders(source, 0, "[\n\t\"b\",\n]", &["b"])?;
        assert_removal_renders(source, 1, "[\n\t\"a\",\n]", &["a"])
    }

    // A comment on the removed entry's line goes with it.

    #[test]
    fn a_comment_on_the_removed_entrys_line_goes_with_it() -> TestOutcome {
        let source = "[\n    \"a\", # ca\n    \"b\", # cb\n    \"c\", # cc\n]";
        assert_removal_renders(
            source,
            0,
            "[\n    \"b\", # cb\n    \"c\", # cc\n]",
            &["b", "c"],
        )?;
        assert_removal_renders(
            source,
            1,
            "[\n    \"a\", # ca\n    \"c\", # cc\n]",
            &["a", "c"],
        )?;
        assert_removal_renders(
            source,
            2,
            "[\n    \"a\", # ca\n    \"b\", # cb\n]",
            &["a", "b"],
        )
    }

    #[test]
    fn a_comment_on_the_last_entrys_line_goes_with_it_when_there_is_no_comma() -> TestOutcome {
        // The previous entry's comment sat after its comma, and the comma is
        // about to stop existing, so the comment has to land before `]`
        // rather than be swallowed by a comma that follows it.
        assert_removal_renders(
            "[\n    \"a\", # ca\n    \"b\" # cb\n]",
            1,
            "[\n    \"a\" # ca\n]",
            &["a"],
        )
    }

    #[test]
    fn a_comment_on_the_only_entrys_line_goes_with_it() -> TestOutcome {
        assert_removal_renders("[\n    \"a\", # ca\n]", 0, "[\n]", &[])?;
        assert_removal_renders("[\n    \"a\" # ca\n]", 0, "[\n]", &[])
    }

    #[test]
    fn a_comment_on_the_line_of_the_bracket_stays() -> TestOutcome {
        assert_removal_renders(
            "[ # the list\n    \"a\",\n    \"b\",\n]",
            0,
            "[ # the list\n    \"b\",\n]",
            &["b"],
        )
    }

    // A comment on a line of its own above the removed entry stays.

    #[test]
    fn a_comment_line_above_the_removed_entry_stays() -> TestOutcome {
        assert_removal_renders(
            "[\n    \"a\",\n    # why b\n    \"b\",\n    \"c\",\n]",
            1,
            "[\n    \"a\",\n    # why b\n    \"c\",\n]",
            &["a", "c"],
        )?;
        assert_removal_renders(
            "[\n    # why a\n    \"a\",\n    \"b\",\n]",
            0,
            "[\n    # why a\n    \"b\",\n]",
            &["b"],
        )
    }

    #[test]
    fn a_comment_line_above_the_last_entry_stays() -> TestOutcome {
        assert_removal_renders(
            "[\n    \"a\",\n    # why b\n    \"b\",\n]",
            1,
            "[\n    \"a\",\n    # why b\n]",
            &["a"],
        )?;
        assert_removal_renders(
            "[\n    \"a\",\n    # why b\n    \"b\"\n]",
            1,
            "[\n    \"a\"\n    # why b\n]",
            &["a"],
        )
    }

    #[test]
    fn a_comment_line_above_the_only_entry_stays() -> TestOutcome {
        assert_removal_renders("[\n    # why a\n    \"a\",\n]", 0, "[\n    # why a\n]", &[])?;
        assert_removal_renders("[\n    # why a\n    \"a\"\n]", 0, "[\n    # why a\n]", &[])
    }

    #[test]
    fn a_comment_line_above_the_closing_bracket_stays() -> TestOutcome {
        assert_removal_renders(
            "[\n    \"a\",\n    \"b\", # cb\n    # the end\n]",
            1,
            "[\n    \"a\",\n    # the end\n]",
            &["a"],
        )
    }

    #[test]
    fn a_comment_above_the_removed_entry_and_one_on_its_line_are_told_apart() -> TestOutcome {
        assert_removal_renders(
            "[\n    \"a\", # ca\n    # why b\n    \"b\", # cb\n    # why c\n    \"c\", # cc\n]",
            1,
            "[\n    \"a\", # ca\n    # why b\n    # why c\n    \"c\", # cc\n]",
            &["a", "c"],
        )
    }

    #[test]
    fn a_comma_on_its_own_line_after_a_comment_is_removed_with_its_entry() -> TestOutcome {
        // `"a"` carries a comment before its comma and one after it; both are
        // on `"a"`'s lines, so both go.
        assert_removal_renders(
            "[\n    \"a\" # suffix\n    , # trailing\n    \"b\",\n]",
            0,
            "[\n    \"b\",\n]",
            &["b"],
        )?;
        assert_removal_renders(
            "[\n    \"a\",\n    \"b\" # suffix\n    , # trailing\n]",
            1,
            "[\n    \"a\",\n]",
            &["a"],
        )
    }

    /// Whatever the layout, removing any one entry leaves an array that
    /// parses to the others, in order — the property the exact-text tests
    /// above pin for the layouts a person is likely to write, here swept
    /// over every position of a wider set of layouts.
    #[test]
    fn removing_any_entry_of_any_layout_leaves_the_others_parseable() -> TestOutcome {
        let layouts = [
            "[\"a\", \"b\", \"c\"]",
            "[\"a\", \"b\", \"c\",]",
            "[ \"a\", \"b\", \"c\" ]",
            "[\"a\",\"b\",\"c\"]",
            "[\n    \"a\",\n    \"b\",\n    \"c\",\n]",
            "[\n    \"a\",\n    \"b\",\n    \"c\"\n]",
            "[\n\t\"a\",\n\t\"b\", # cb\n\t\"c\"\n]",
            "[\n    # about a\n    \"a\", # ca\n    # about b\n    \"b\", # cb\n    \"c\" # cc\n]",
            "[\n    # about a\n    \"a\", # ca\n    # about b\n    \"b\", # cb\n    \"c\", # cc\n    # end\n]",
            "[\n    \"a\" # sa\n    , # ta\n    \"b\",\n    \"c\" # sc\n    , # tc\n]",
            "[ # list\n    \"a\", \"b\",\n    \"c\",\n]",
        ];
        let entries = ["a", "b", "c"];

        for layout in layouts {
            for index in 0..entries.len() {
                let mut document: DocumentMut = format!("members = {layout}\n").parse()?;
                let array = document
                    .get_mut("members")
                    .and_then(toml_edit::Item::as_array_mut)
                    .expect("each layout is an array");
                remove_matching_style(array, index);

                let rendered = document.to_string();
                let expected: Vec<&str> = entries
                    .iter()
                    .enumerate()
                    .filter(|(position, _entry)| *position != index)
                    .map(|(_position, entry)| *entry)
                    .collect();
                assert_eq!(
                    members_of(&rendered)?,
                    expected,
                    "removing entry {index} of {layout:?} rendered {rendered:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    #[should_panic(expected = "cannot remove entry 2 of an array of 1")]
    fn removing_a_position_that_is_not_in_the_array_panics() {
        let mut document: DocumentMut = "members = [\"a\"]\n".parse().expect("valid TOML");
        let array = document
            .get_mut("members")
            .and_then(toml_edit::Item::as_array_mut)
            .expect("an array");
        remove_matching_style(array, 2);
    }
}
