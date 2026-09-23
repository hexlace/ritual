//! `rituals-compose` re-exports nothing from `rituals`: every crate that
//! needs `rituals::Task`, `rituals::CommandLine` or `rituals::run` names
//! `rituals` itself, and a composed CLI depends on `rituals` alone, never on
//! the composition library. A second path to the same items would be a
//! second way to depend on them, and `rituals-compose` would become part of
//! every composed CLI's public surface.
//!
//! Read from source, since a missing re-export shows up as a compile error
//! in some other crate, not as anything a test here could run: every `pub
//! use` in `rituals-compose`'s source must name something other than
//! `rituals`.

mod support;

use support::{TestOutcome, in_checkout, read_text, tree};

/// Whether `line` is a public `use` of `rituals` or of anything inside it:
/// `pub use rituals;`, `pub use rituals::…`, or `pub use ::rituals::…`.
fn re_exports_rituals(line: &str) -> bool {
    let Some(path) = line.trim_start().strip_prefix("pub use ") else {
        return false;
    };
    let path = path.trim_start().trim_start_matches("::");
    path.starts_with("rituals;") || path.starts_with("rituals::")
}

#[test]
fn the_composition_library_re_exports_nothing_from_rituals() -> TestOutcome {
    in_checkout(|checkout| {
        let source = checkout.root().join("crates/rituals-compose/src");
        let mut re_exports = Vec::new();
        for path in tree::files_under(&source)? {
            if path.extension().is_some_and(|extension| extension == "rs") {
                for line in read_text(&path)?.lines() {
                    if re_exports_rituals(line) {
                        re_exports.push(format!("{}: {line}", path.display()));
                    }
                }
            }
        }
        assert!(
            re_exports.is_empty(),
            "expected `rituals-compose` to re-export nothing from `rituals`; found:\n{}",
            re_exports.join("\n")
        );
        Ok(())
    })
}

#[test]
fn the_re_export_check_recognises_every_spelling_of_one() {
    for re_export in [
        "pub use rituals;",
        "pub use rituals::run;",
        "    pub use rituals::{CommandLine, Identity, Task, identity};",
        "pub use ::rituals::Task;",
    ] {
        assert!(re_exports_rituals(re_export), "{re_export:?}");
    }
    for not_a_re_export in [
        "use rituals::Task;",
        "pub(crate) use rituals::Task;",
        "pub use toml_edit::DocumentMut;",
        "pub use rituals_compose::manifest;",
        "//! Nothing from `rituals` is re-exported here.",
    ] {
        assert!(!re_exports_rituals(not_a_re_export), "{not_a_re_export:?}");
    }
}
