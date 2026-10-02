//! Comparing paths the way Cargo does: lexically, without asking the
//! filesystem.
//!
//! Cargo joins a relative path in a manifest onto the manifest's directory
//! and removes `.` and `..` components as text, never resolving a symbolic
//! link. Two spellings it reads as one directory, such as `tasks/./lint`
//! and `x/../tasks/lint`, compare equal here once both are normalised.
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

use std::path::{Component, Path, PathBuf};

/// `path` with every `.` component removed and every `..` taking out the
/// component before it, as text.
///
/// A `..` with nothing before it to take out is dropped at the root, the way
/// `/..` is `/`, and kept at the start of a relative path, which has nothing
/// to resolve it against.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match result.components().next_back() {
                Some(Component::Normal(_)) => {
                    result.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                Some(Component::ParentDir | Component::CurDir) | None => result.push(".."),
            },
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Whether `path` is `directory` or lies under it, once both are
/// normalised. Compared by component, so `tasks/lint-extra` is not under
/// `tasks/lint`.
pub(crate) fn lies_under(path: &Path, directory: &Path) -> bool {
    normalize(path).starts_with(normalize(directory))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{lies_under, normalize};

    #[test]
    fn dot_and_dot_dot_and_repeated_separators_are_removed_as_text() {
        for spelling in [
            "/w/tasks/lint",
            "/w/tasks/./lint",
            "/w/tasks//lint",
            "/w/x/../tasks/lint",
            "/w/tasks/lint/",
            "/w/./tasks/lint/.",
        ] {
            assert_eq!(
                normalize(Path::new(spelling)),
                PathBuf::from("/w/tasks/lint"),
                "{spelling}"
            );
        }
    }

    #[test]
    fn a_dot_dot_past_the_root_stays_at_the_root_and_one_starting_a_relative_path_stays() {
        assert_eq!(normalize(Path::new("/../a")), PathBuf::from("/a"));
        assert_eq!(normalize(Path::new("../a/../b")), PathBuf::from("../b"));
        assert_eq!(normalize(Path::new("../../a")), PathBuf::from("../../a"));
    }

    #[test]
    fn a_path_lies_under_its_directory_and_the_directory_itself_but_not_a_sibling() {
        let directory = Path::new("/w/tasks/lint");
        assert!(lies_under(Path::new("/w/tasks/lint"), directory));
        assert!(lies_under(Path::new("/w/tasks/lint/src/lib.rs"), directory));
        assert!(lies_under(Path::new("/w/x/../tasks/lint/inner"), directory));
        assert!(!lies_under(Path::new("/w/tasks/lint-extra"), directory));
        assert!(!lies_under(Path::new("/w/tasks"), directory));
    }
}
