//! How a path is named to the person running `migrate`.
//!
//! Every path a report line prints is one a person can open from the
//! project's root, so there is one place that spells them.

use std::path::{Component, Path, PathBuf};

/// `path` as the person knows it: from `root` when it is inside it, and as
/// it is otherwise.
pub(crate) fn from_the_root(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// `path` spelled from `directory`, where both are written from the same
/// base and neither has `.` or `..` in it: `.rituals/x` from `ritual` is
/// `../.rituals/x`, and a path that is `directory` itself is `.`.
///
/// For paths git reports, which are spelled from the repository's top level
/// whichever directory of it git was asked from, and a project's root that
/// may be below it.
pub(crate) fn spelled_from(directory: &Path, path: &Path) -> String {
    let directory: Vec<Component<'_>> = directory.components().collect();
    let path: Vec<Component<'_>> = path.components().collect();
    let shared = directory
        .iter()
        .zip(&path)
        .take_while(|(left, right)| left == right)
        .count();

    let mut spelled = PathBuf::new();
    for _ in shared..directory.len() {
        spelled.push("..");
    }
    for component in &path[shared..] {
        spelled.push(component);
    }
    if spelled.as_os_str().is_empty() {
        return ".".to_string();
    }
    spelled.display().to_string()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{from_the_root, spelled_from};

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

    #[test]
    fn a_path_below_the_directory_drops_the_directory() {
        assert_eq!(
            spelled_from(Path::new("demo"), Path::new("demo/.github/ci.yml")),
            ".github/ci.yml"
        );
    }

    #[test]
    fn a_path_beside_the_directory_leaves_it_with_one_step_up() {
        assert_eq!(
            spelled_from(Path::new("demo"), Path::new(".github/ci.yml")),
            "../.github/ci.yml"
        );
    }

    #[test]
    fn a_directory_below_the_top_level_needs_a_step_up_for_each_level() {
        assert_eq!(
            spelled_from(Path::new("a/b"), Path::new("a/c/ci.yml")),
            "../c/ci.yml"
        );
        assert_eq!(
            spelled_from(Path::new("a/b"), Path::new("x.md")),
            "../../x.md"
        );
    }

    #[test]
    fn a_directory_that_is_the_top_level_leaves_the_path_as_it_is() {
        assert_eq!(
            spelled_from(Path::new(""), Path::new("scripts/check.sh")),
            "scripts/check.sh"
        );
    }

    #[test]
    fn a_name_sharing_only_a_prefix_is_not_inside_the_directory() {
        assert_eq!(
            spelled_from(Path::new("demo"), Path::new("demo-extra/x")),
            "../demo-extra/x"
        );
    }

    #[test]
    fn the_directory_itself_is_the_current_directory() {
        assert_eq!(spelled_from(Path::new("demo"), Path::new("demo")), ".");
    }
}
