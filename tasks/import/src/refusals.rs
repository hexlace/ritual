//! Every refusal of `import`'s own, and what each says to do instead.
//!
//! The ones that come from further in are `rituals-compose`'s: a project that
//! is not this command line's, a key that is already a top-level command, a
//! task list that is broken, and a dependency that is not a task. They reach
//! the person in their own words.
//
// `redundant_pub_crate` (clippy nursery) wants `pub` here because this
// module is private, but `pub(crate)` is the visibility that is actually
// true — it stays correct if this module is ever re-exported at a different
// level — so the nursery lint gives way.
#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) reflects this module's actual visibility even though the enclosing \
              module is private; see the note above"
)]

use rituals::{Failure, InvalidName, Name, Outcome};

/// The refusal for a crate name that cannot be a command's name, when the
/// person gave no key to import it under.
///
/// `unusable` says why the name is not usable. `command_to_run` is the
/// command to run instead, with a suggestion where the key goes, so the
/// remedy is one to copy.
pub(crate) fn unusable_default_key(unusable: &InvalidName, command_to_run: &str) -> Failure {
    Failure::new(format!(
        "{unusable}; the key defaults to the crate's name, so give one: `{command_to_run}`"
    ))
}

/// Refuses when `key` is the bin name this command line was built as.
///
/// Whatever is mounted under the bin's name becomes the command line's own
/// top level and has to be a bundle, and `import` brings in a task. The check
/// that every other key goes through lets that one key by, because
/// `regenerate` shares it, so this runs first and refuses on `import`'s own
/// behalf.
pub(crate) fn ensure_the_key_is_not_the_bin_name(key: &Name, binary_name: &str) -> Outcome {
    if key.as_str() == binary_name {
        return Err(Failure::new(format!(
            "`{binary_name}` is reserved for this command line's own commands; import this \
             crate under another key"
        )));
    }
    Ok(())
}

/// Refuses when `key` is already spoken for in the composed CLI's own
/// manifest: a dependency, whether or not it is also listed, or listed with
/// no matching dependency.
///
/// The two flags decide four outcomes, so they are matched as a pair: a
/// reader sees every state at once, and the compiler checks that none was
/// left out. `regenerate` is how a person types this command line's
/// `regenerate`, and `import_again` how they type this import again, each as
/// [`rituals_compose::top_level::management_command`] spells it, so every
/// remedy here can be copied as written.
///
/// A dependency counts as declared when its key reads as `key` to rustc,
/// with `-` as `_`, so the second arm names the underscore spelling too when
/// `key` has a hyphen: the manifest line it is about may be spelled either
/// way.
pub(crate) fn already_imported_refusal(
    package: &str,
    key: &Name,
    already_a_dependency: bool,
    already_listed: bool,
    regenerate: &str,
    import_again: &str,
) -> Outcome {
    match (already_a_dependency, already_listed) {
        (true, true) => Err(Failure::new(format!(
            "`{key}` is already a task of `{package}`; import this crate under another key, or, \
             if its command is missing from the command line, run `{regenerate}`"
        ))),
        (true, false) => Err(Failure::new(format!(
            "`{package}` already has a dependency called `{key}`{other_spelling} that is not in \
             [package.metadata.ritual] tasks; import this crate under another key, or, to make \
             that dependency the task, add `\"{key}\"` to that list and run `{regenerate}`",
            other_spelling = underscore_spelling_clause(key),
        ))),
        (false, true) => Err(Failure::new(format!(
            "`{key}` is named in [package.metadata.ritual] tasks but `{package}` has no \
             dependency called `{key}`; drop it from the list and run `{import_again}` again"
        ))),
        (false, false) => Ok(()),
    }
}

/// The clause naming the underscore spelling of `key`, when it has a hyphen,
/// led by a space: " (or `a_b`, which Rust reads as the same name)" for
/// `a-b`. Empty when there is no other spelling to name.
fn underscore_spelling_clause(key: &Name) -> String {
    if !key.as_str().contains('-') {
        return String::new();
    }
    format!(
        " (or `{}`, which Rust reads as the same name)",
        key.as_str().replace('-', "_")
    )
}

#[cfg(test)]
mod tests {
    use rituals::Name;

    use super::{
        already_imported_refusal, ensure_the_key_is_not_the_bin_name, unusable_default_key,
    };

    const REGENERATE: &str = "cargo ritual regenerate";
    const IMPORT_AGAIN: &str = "cargo ritual import greeter --path /work/greeter";

    fn valid_name(value: &str) -> Name {
        Name::new(value).expect("a test passes only names it knows are valid")
    }

    fn refusal(already_a_dependency: bool, already_listed: bool, key: &str) -> Option<String> {
        already_imported_refusal(
            "demo-ritual",
            &valid_name(key),
            already_a_dependency,
            already_listed,
            REGENERATE,
            IMPORT_AGAIN,
        )
        .err()
        .map(|failure| failure.to_string())
    }

    #[test]
    fn a_key_that_is_a_dependency_and_listed_is_refused_with_two_ways_out() {
        assert_eq!(
            refusal(true, true, "greeter").as_deref(),
            Some(
                "`greeter` is already a task of `demo-ritual`; import this crate under another \
                 key, or, if its command is missing from the command line, run \
                 `cargo ritual regenerate`"
            )
        );
    }

    #[test]
    fn a_key_that_is_a_dependency_but_not_listed_is_refused_naming_the_list_to_edit() {
        assert_eq!(
            refusal(true, false, "greeter").as_deref(),
            Some(
                "`demo-ritual` already has a dependency called `greeter` that is not in \
                 [package.metadata.ritual] tasks; import this crate under another key, or, to \
                 make that dependency the task, add `\"greeter\"` to that list and run \
                 `cargo ritual regenerate`"
            )
        );
    }

    /// A hyphen and an underscore are one name to rustc, so a dependency
    /// declared as `a_b` is what refuses the key `a-b`, and the person
    /// searching their manifest for what they typed would not find it.
    #[test]
    fn a_hyphenated_key_taken_by_a_dependency_names_the_underscore_spelling_too() {
        assert_eq!(
            refusal(true, false, "a-b").as_deref(),
            Some(
                "`demo-ritual` already has a dependency called `a-b` (or `a_b`, which Rust reads \
                 as the same name) that is not in [package.metadata.ritual] tasks; import this \
                 crate under another key, or, to make that dependency the task, add `\"a-b\"` to \
                 that list and run `cargo ritual regenerate`"
            )
        );
    }

    #[test]
    fn a_key_that_is_listed_with_no_dependency_is_refused_with_the_import_to_run_again() {
        assert_eq!(
            refusal(false, true, "greeter").as_deref(),
            Some(
                "`greeter` is named in [package.metadata.ritual] tasks but `demo-ritual` has no \
                 dependency called `greeter`; drop it from the list and run \
                 `cargo ritual import greeter --path /work/greeter` again"
            )
        );
    }

    #[test]
    fn a_key_that_is_neither_is_allowed() {
        assert_eq!(refusal(false, false, "greeter"), None);
    }

    #[test]
    fn the_bin_name_is_refused_as_reserved_and_asks_for_another_key() {
        let result = ensure_the_key_is_not_the_bin_name(&valid_name("chores"), "chores");

        assert!(result.is_err(), "expected the bin's own name to be refused");
        if let Err(failure) = result {
            assert_eq!(
                failure.to_string(),
                "`chores` is reserved for this command line's own commands; import this crate \
                 under another key"
            );
        }
    }

    /// The check keys on the whole name: one that only contains the bin's
    /// name, or that differs from it, passes.
    #[test]
    fn a_key_that_is_not_the_bin_name_passes() {
        assert!(ensure_the_key_is_not_the_bin_name(&valid_name("chores-helper"), "chores").is_ok());
        assert!(ensure_the_key_is_not_the_bin_name(&valid_name("greeter"), "chores").is_ok());
    }

    #[test]
    fn an_unusable_default_key_says_to_give_one_and_shows_how() {
        let unusable = Name::new("my_crate").err();
        assert!(unusable.is_some(), "`my_crate` is not a usable name");
        if let Some(unusable) = unusable {
            assert_eq!(
                unusable_default_key(&unusable, "cargo ritual import my_crate my-crate")
                    .to_string(),
                "`my_crate` is not a usable name; a name starts with a lowercase letter, \
                 continues with lowercase letters, digits and hyphens, and does not end with a \
                 hyphen; the key defaults to the crate's name, so give one: \
                 `cargo ritual import my_crate my-crate`"
            );
        }
    }
}
