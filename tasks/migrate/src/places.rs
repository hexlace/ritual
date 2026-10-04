//! How a path is named to the person running `migrate`.
//!
//! Every path a report line prints is one a person can open from the
//! project's root, so there is one place that spells them.

use std::path::Path;

use rituals_compose::paths;

/// `path` as the person knows it: from `root` when it is inside it, and as
/// it is otherwise.
pub(crate) fn from_the_root(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// `path_from_top_level`, which git spelled from the repository's
/// `top_level`, as the person knows it: from `root`, the project's root,
/// which may be below the top level. Both are absolute and resolved through
/// symbolic links, as git's top level is.
pub(crate) fn from_the_root_in(
    path_from_top_level: &Path,
    top_level: &Path,
    root: &Path,
) -> String {
    paths::relative(root, &top_level.join(path_from_top_level))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{from_the_root, from_the_root_in};

    #[test]
    fn a_path_inside_the_root_is_spelled_from_it() {
        assert_eq!(
            from_the_root(Path::new("/w/demo/tasks/greet"), Path::new("/w/demo")),
            "tasks/greet"
        );
    }

    #[test]
    fn a_path_outside_the_root_is_spelled_as_it_is() {
        assert_eq!(
            from_the_root(Path::new("/elsewhere/greet"), Path::new("/w/demo")),
            "/elsewhere/greet"
        );
    }

    /// Git spells a path from the top level; the person reads it from the
    /// root, with `..` to leave it, `.` for the root itself, and a name that
    /// only shares a prefix with the root not taken for inside it.
    #[test]
    fn a_path_from_the_top_level_is_spelled_from_a_root_below_it() {
        let spelled =
            |path: &str| from_the_root_in(Path::new(path), Path::new("/w"), Path::new("/w/demo"));
        assert_eq!(spelled("demo/.github/ci.yml"), ".github/ci.yml");
        assert_eq!(spelled(".github/ci.yml"), "../.github/ci.yml");
        assert_eq!(spelled("demo-extra/x"), "../demo-extra/x");
        assert_eq!(spelled("demo"), ".");
        assert_eq!(spelled(""), "..");
    }

    #[test]
    fn a_root_that_is_the_top_level_leaves_the_path_as_it_is() {
        assert_eq!(
            from_the_root_in(
                Path::new("scripts/check.sh"),
                Path::new("/w"),
                Path::new("/w")
            ),
            "scripts/check.sh"
        );
    }
}
