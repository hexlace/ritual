//! A name that has been checked to be safe as a command, a directory name, a
//! Cargo package name and a Rust identifier.
//!
//! [`Name::new`] is the only constructor, and it validates. Every function in
//! this framework that joins a caller-supplied name into a path takes a
//! [`Name`], so an unchecked string can never reach a path join — the wrong
//! way is made impossible rather than remembered.

use std::fmt;

/// A name checked to be safe as a command, a directory name, a Cargo package
/// name and a Rust identifier.
///
/// It starts with a lowercase ASCII letter, continues with lowercase ASCII
/// letters, ASCII digits and hyphens, does not end with a hyphen, and is not
/// one of the three identifiers that cannot be written as a Rust raw
/// identifier.
///
/// # Examples
///
/// ```
/// use rituals::Name;
///
/// let name = Name::new("greet")?;
/// assert_eq!(name.as_str(), "greet");
///
/// assert!(Name::new("../evil").is_err());
/// assert!(Name::new("lint-").is_err());
/// # Ok::<(), rituals::InvalidName>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name(String);

/// The three identifiers that are reserved words in every Rust edition and
/// cannot be written as a raw identifier (`r#crate` is not valid syntax, for
/// example) — so a command's name, which must be writable as a Rust
/// identifier, cannot be one of these.
const RESERVED_IDENTIFIERS: [&str; 3] = ["crate", "self", "super"];

impl Name {
    /// Validates `text` against the one name rule, and refuses it otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidName`] when `text` does not start with a lowercase
    /// ASCII letter and continue with lowercase ASCII letters, ASCII digits
    /// and hyphens without a trailing hyphen, or when it is `crate`, `self`
    /// or `super`.
    pub fn new(text: &str) -> Result<Self, InvalidName> {
        if !has_valid_spelling(text) {
            return Err(InvalidName {
                text: text.to_string(),
                reason: InvalidNameReason::Spelling,
            });
        }

        if RESERVED_IDENTIFIERS.contains(&text) {
            return Err(InvalidName {
                text: text.to_string(),
                reason: InvalidNameReason::NotAnIdentifier,
            });
        }

        Ok(Self(text.to_string()))
    }

    /// Returns this name as the plain string it validated.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Checks the one name rule's spelling clause: non-empty, a lowercase ASCII
/// letter first, lowercase ASCII letters/digits/hyphens after, no trailing
/// hyphen.
fn has_valid_spelling(text: &str) -> bool {
    let mut characters = text.chars();

    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }

    let all_valid = characters.clone().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    });
    if !all_valid {
        return false;
    }

    !text.ends_with('-')
}

/// Why [`Name::new`] refused a piece of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InvalidNameReason {
    /// It fails the spelling clause of the one name rule.
    Spelling,
    /// It is `crate`, `self` or `super`, none of which can be written as a
    /// Rust raw identifier.
    NotAnIdentifier,
}

/// Why a piece of text was refused as a [`Name`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidName {
    text: String,
    reason: InvalidNameReason,
}

impl fmt::Display for InvalidName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.reason {
            InvalidNameReason::Spelling => {
                "a name starts with a lowercase letter, continues with lowercase letters, digits \
                 and hyphens, and does not end with a hyphen"
            }
            InvalidNameReason::NotAnIdentifier => {
                "`crate`, `self` and `super` cannot be written as a Rust identifier, and a \
                 command's name has to be one"
            }
        };

        write!(formatter, "`{}` is not a usable name; {reason}", self.text)
    }
}

impl std::error::Error for InvalidName {}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Name, has_valid_spelling};

    /// Every accepted name, joined under a fixed root, must stay under that
    /// root and add exactly one path component — the invariant every
    /// path-joining call site in this framework relies on `Name` to hold.
    #[test]
    fn every_accepted_name_joins_as_exactly_one_path_component() {
        let corpus = [
            "l", "l1", "a-b-c", "greet", "lint", "new", "create", "add-on", "z9",
        ];

        for candidate in corpus {
            let outcome = Name::new(candidate);
            assert!(
                outcome.is_ok(),
                "expected {candidate:?} to be accepted, got {outcome:?}"
            );

            if let Ok(name) = outcome {
                let root = Path::new("/root/tasks");
                let joined = root.join(name.as_str());
                assert!(
                    joined.starts_with(root),
                    "joining {candidate:?} escaped the root: {joined:?}"
                );
                assert_eq!(
                    joined.components().count(),
                    root.components().count() + 1,
                    "joining {candidate:?} did not add exactly one path component: {joined:?}"
                );
            }
        }
    }

    /// A curated corpus of shapes that must all be refused: path
    /// separators, absolute and traversal-like paths, an empty string, an
    /// uppercase letter, a leading digit and a trailing hyphen. Which
    /// clause of the one name rule refuses each one is covered separately,
    /// by [`badly_spelled_names_are_refused_for_spelling`] and
    /// [`reserved_identifiers_are_refused_as_not_an_identifier`] below.
    #[test]
    fn curated_invalid_names_are_all_refused() {
        let corpus = [
            "..", "../x", "a/b", "a\\b", "/abs", "C:\\x", "", "New", "1lint", "lint-",
        ];

        for candidate in corpus {
            assert!(
                Name::new(candidate).is_err(),
                "expected {candidate:?} to be refused"
            );
        }
    }

    #[test]
    fn reserved_identifiers_are_refused_as_not_an_identifier() {
        for reserved in ["crate", "self", "super"] {
            assert!(
                Name::new(reserved).is_err(),
                "expected {reserved:?} to be refused"
            );
            if let Err(error) = Name::new(reserved) {
                assert!(
                    error
                        .to_string()
                        .contains("cannot be written as a Rust identifier"),
                    "expected {reserved:?} to be refused for not being a Rust identifier, got: {error}"
                );
            }
        }
    }

    #[test]
    fn badly_spelled_names_are_refused_for_spelling() {
        for bad in ["..", "New", "1lint", "lint-", ""] {
            assert!(Name::new(bad).is_err(), "expected {bad:?} to be refused");
            if let Err(error) = Name::new(bad) {
                assert!(
                    error.to_string().contains("starts with a lowercase letter"),
                    "expected {bad:?} to be refused for spelling, got: {error}"
                );
            }
        }
    }

    /// Exhaustive over every ASCII byte value (0x00–0x7F) in the first
    /// position of a single-character candidate, and in the second position
    /// of a two-character candidate starting with a known-good first
    /// character — cheaper and stronger than sampling, since the input
    /// space `has_valid_spelling` classifies by is a fixed 128 values per
    /// position. A byte above 0x7F is never a complete UTF-8 character on
    /// its own, so it cannot appear here as a standalone candidate; every
    /// multi-byte character is covered instead by
    /// [`multi_byte_characters_are_refused_in_first_and_later_position`]
    /// below.
    #[test]
    fn every_ascii_byte_value_is_judged_correctly_in_first_and_later_position() {
        for byte in 0u8..=127 {
            let character = char::from(byte);

            let expected_first = character.is_ascii_lowercase();
            assert_eq!(
                has_valid_spelling(&character.to_string()),
                expected_first,
                "byte {byte:#04x} in first position disagreed with the rule"
            );

            let two_characters = format!("a{character}");
            let expected_later =
                (character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-')
                    && !two_characters.ends_with('-');
            assert_eq!(
                has_valid_spelling(&two_characters),
                expected_later,
                "byte {byte:#04x} in a later position disagreed with the rule"
            );
        }
    }

    /// `has_valid_spelling` classifies every character by three ASCII
    /// checks, so no multi-byte character can ever pass any of them — this
    /// samples a few, from accented Latin through a symbol outside the
    /// Basic Multilingual Plane, to check that in first and later position.
    #[test]
    fn multi_byte_characters_are_refused_in_first_and_later_position() {
        for character in ['é', 'ñ', '中', 'Ω', '🦀'] {
            assert!(
                !has_valid_spelling(&character.to_string()),
                "{character:?} must be refused in first position"
            );

            let two_characters = format!("a{character}");
            assert!(
                !has_valid_spelling(&two_characters),
                "{character:?} must be refused in a later position"
            );
        }
    }
}
