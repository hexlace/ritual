//! Comparing and spelling paths: comparing them the way Cargo reaches them,
//! spelled as Cargo spells them and then followed through the filesystem, and
//! spelling one from another directory.
//!
//! Cargo joins most relative paths onto a base and removes `.` and `..`
//! components as text before it opens anything, so `tasks/./lint` and
//! `x/../tasks/lint` are one directory to it whatever `x` is. Then it opens
//! the result, and the filesystem decides what that names: `Tasks/Lint` on a
//! file system that folds case, `alias/lint` when `alias` is a link to
//! `tasks`, `/tmp/w/tasks/lint` when `/tmp` is a link to `/private/tmp`. So
//! whether a path reaches a directory is asked of the filesystem, by file
//! identity, while the directory still exists.
//!
//! [`relative`] is the one way to spell the path that leads from one
//! directory to another, wherever a path is written for a person or a
//! manifest to read.
//!
//! # Examples
//!
//! Spelling where a task's directory is from a crate that depends on it:
//!
//! ```
//! use std::path::Path;
//!
//! use rituals_compose::paths::relative;
//!
//! let path = relative(Path::new("/w/ritual"), Path::new("/w/.rituals/greet"));
//! assert_eq!(path, "../.rituals/greet");
//! ```

use std::ffi::OsString;
use std::fs::Metadata;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Component, Path, PathBuf};

/// How many symbolic links one path may lead through before it is no longer
/// followed: more than either kernel this runs on follows (32 on macOS, 40 on
/// Linux), so a path past it is one the system refuses to open too.
const LINKS_FOLLOWED_LIMIT: u32 = 40;

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

/// The forward-slash path that leads from `from_directory` to `to`, both
/// absolute, spelled the same on every platform because its separator is
/// always `/`.
///
/// Climbs out of `from_directory` to the directory the two share, then
/// descends to `to`; the same directory is `.`, since nothing else can be
/// written where a manifest wants a path. Both are compared by component and
/// taken as they are, so a caller that wants `.` and `..` resolved first
/// normalises them first.
///
/// # Panics
///
/// Panics if either path is not absolute, since a relative path between two
/// relative paths depends on a working directory neither carries.
///
/// # Examples
///
/// The same directory is `.`, and a directory that only shares a name prefix
/// with another is not inside it:
///
/// ```
/// use std::path::Path;
///
/// use rituals_compose::paths::relative;
///
/// assert_eq!(relative(Path::new("/w/demo"), Path::new("/w/demo")), ".");
/// assert_eq!(
///     relative(Path::new("/w/demo"), Path::new("/w/demo-extra/x")),
///     "../demo-extra/x"
/// );
/// ```
#[must_use]
pub fn relative(from_directory: &Path, to: &Path) -> String {
    assert!(
        from_directory.is_absolute(),
        "from_directory must be absolute, got {}",
        from_directory.display()
    );
    assert!(
        to.is_absolute(),
        "to must be absolute, got {}",
        to.display()
    );

    let from_components: Vec<_> = from_directory.components().collect();
    let to_components: Vec<_> = to.components().collect();

    let shared = from_components
        .iter()
        .zip(to_components.iter())
        .take_while(|(from, to)| from == to)
        .count();

    let ascents = from_components.len() - shared;
    let mut segments: Vec<String> = std::iter::repeat_n("..".to_string(), ascents).collect();
    segments.extend(
        to_components[shared..]
            .iter()
            .map(|component| component.as_os_str().to_string_lossy().into_owned()),
    );

    if segments.is_empty() {
        ".".to_string()
    } else {
        segments.join("/")
    }
}

/// Whether `path` is `directory` or lies under it, for a path Cargo
/// normalises as text before opening: a dependency's, a `[patch]`'s, a
/// `paths` override's, a member's or a target's.
///
/// Normalised first, as Cargo does, then followed as [`opens_through`]
/// follows it. Compared by component, so `tasks/lint-extra` is not under
/// `tasks/lint`.
pub(crate) fn lies_under(path: &Path, directory: &Path) -> bool {
    opens_through(&normalize(path), directory)
}

/// Whether opening `path`, as written, passes through `directory`: the path
/// is the directory, or reaches something under it.
///
/// Followed the way the system opens it, one component at a time, so a `..`
/// after a symbolic link goes up from where the link leads, as it does for
/// an `include`, which Cargo opens as written. Each step is compared with
/// `directory` by file identity, the device and inode of the entry itself
/// rather than of what it leads to; so when `directory` is a symbolic link,
/// a path through the link reaches it and a path to the link's target does
/// not, because deleting the link leaves its target where it was.
///
/// Where the filesystem cannot answer, the comparison is made as text
/// instead: when `directory` does not exist, when `path` is relative, and
/// for the part of `path` past the first component that does not exist,
/// joined onto the part that does, resolved, and compared with `directory`
/// resolved the same way up to its own last component.
pub(crate) fn opens_through(path: &Path, directory: &Path) -> bool {
    let Ok(target) = directory.symlink_metadata() else {
        return normalize(path).starts_with(normalize(directory));
    };
    if !path.is_absolute() {
        return normalize(path).starts_with(normalize(directory));
    }

    // Every component of `resolved` exists and none is a link, so its parent
    // as text is its parent on disk.
    let mut resolved = PathBuf::from("/");
    let mut pending = steps_in_reverse(path);
    let mut links_followed: u32 = 0;
    while let Some(step) = pending.pop() {
        let name = match step {
            Step::Up => {
                resolved.pop();
                continue;
            }
            Step::Into(name) => name,
        };
        let next = resolved.join(&name);
        let Ok(entry) = next.symlink_metadata() else {
            pending.push(Step::Into(name));
            return lies_under_as_text(&resolved, &pending, directory);
        };
        if is_same_entry(&entry, &target) {
            return true;
        }
        if !entry.file_type().is_symlink() {
            resolved = next;
            continue;
        }
        links_followed += 1;
        let link = match std::fs::read_link(&next) {
            Ok(link) if links_followed <= LINKS_FOLLOWED_LIMIT => link,
            Ok(_) | Err(_) => {
                pending.push(Step::Into(name));
                return lies_under_as_text(&resolved, &pending, directory);
            }
        };
        if link.is_absolute() {
            resolved = PathBuf::from("/");
        }
        pending.extend(steps_in_reverse(&link));
    }
    false
}

/// One component of a path still to follow.
enum Step {
    Up,
    Into(OsString),
}

/// The steps `path` takes, last first, so popping them follows it in order.
/// The root and `.` take no step.
fn steps_in_reverse(path: &Path) -> Vec<Step> {
    path.components()
        .rev()
        .filter_map(|component| match component {
            Component::ParentDir => Some(Step::Up),
            Component::Normal(name) => Some(Step::Into(name.to_os_string())),
            Component::RootDir | Component::CurDir | Component::Prefix(_) => None,
        })
        .collect()
}

/// Whether `left` and `right` describe one entry on disk.
fn is_same_entry(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

/// The text comparison [`opens_through`] falls back to: `resolved`, which
/// exists, with the `pending` steps that do not joined on, against
/// `directory` resolved up to its last component, which is kept as it is so
/// that a link there stays the link.
fn lies_under_as_text(resolved: &Path, pending: &[Step], directory: &Path) -> bool {
    let mut candidate = resolved.to_path_buf();
    for step in pending.iter().rev() {
        match step {
            Step::Up => candidate.push(".."),
            Step::Into(name) => candidate.push(name),
        }
    }
    let directory = normalize(directory);
    let resolved_directory = match (directory.parent(), directory.file_name()) {
        (Some(parent), Some(name)) => std::fs::canonicalize(parent)
            .map_or_else(|_| directory.clone(), |parent| parent.join(name)),
        _ => directory,
    };
    normalize(&candidate).starts_with(resolved_directory)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};

    use super::{lies_under, normalize, opens_through, relative};
    use crate::test_support::{ScratchDir, TestOutcome, report_skip};

    /// A scratch directory holding `tasks/lint/src` and `tasks/fmt`, and the
    /// directory's path resolved through every link above it, the way
    /// `cargo metadata` hands a path over.
    fn project(tag: &str) -> Result<(ScratchDir, PathBuf), Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new(tag)?;
        std::fs::create_dir_all(scratch.path().join("tasks/lint/src"))?;
        std::fs::create_dir_all(scratch.path().join("tasks/fmt"))?;
        let root = std::fs::canonicalize(scratch.path())?;
        Ok((scratch, root))
    }

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

    #[test]
    fn a_path_through_a_link_to_a_directory_above_it_lies_under_it() -> TestOutcome {
        let (_scratch, root) = project("paths-link-above")?;
        symlink("tasks", root.join("alias"))?;
        symlink(root.join("tasks"), root.join("absolute"))?;
        let directory = root.join("tasks/lint");

        assert!(lies_under(&root.join("alias/lint"), &directory));
        assert!(lies_under(&root.join("alias/lint/src/lib.rs"), &directory));
        assert!(lies_under(&root.join("absolute/lint/src"), &directory));
        assert!(!lies_under(&root.join("alias/fmt"), &directory));
        assert!(!lies_under(&root.join("alias/lint-extra"), &directory));
        Ok(())
    }

    /// The scratch directory is spelled here as the system's temporary
    /// directory gives it, which on macOS is through the `/var` link, while
    /// the directory is spelled resolved, as `cargo metadata` gives it.
    #[test]
    fn a_path_through_a_link_above_the_project_lies_under_a_resolved_directory() -> TestOutcome {
        let (scratch, root) = project("paths-link-above-project")?;
        let directory = root.join("tasks/lint");

        assert!(lies_under(
            &scratch.path().join("tasks/lint/src"),
            &directory
        ));
        assert!(!lies_under(&scratch.path().join("tasks/fmt"), &directory));
        Ok(())
    }

    /// Deleting a link leaves what it points at, so a path into the link
    /// lies under it and a path to its target does not.
    #[test]
    fn a_directory_that_is_a_link_is_reached_through_the_link_and_not_its_target() -> TestOutcome {
        let (_scratch, root) = project("paths-link-at-directory")?;
        std::fs::create_dir_all(root.join("vendor/lint/src"))?;
        symlink("../vendor/lint", root.join("tasks/linked"))?;
        symlink("tasks/linked", root.join("to-the-link"))?;
        let directory = root.join("tasks/linked");

        assert!(lies_under(&root.join("tasks/linked"), &directory));
        assert!(lies_under(&root.join("tasks/linked/src"), &directory));
        assert!(lies_under(&root.join("to-the-link/src"), &directory));
        assert!(!lies_under(&root.join("vendor/lint"), &directory));
        assert!(!lies_under(&root.join("vendor/lint/src"), &directory));
        Ok(())
    }

    #[test]
    fn another_case_lies_under_the_directory_where_the_file_system_folds_case() -> TestOutcome {
        let (_scratch, root) = project("paths-case")?;
        if !root.join("TASKS").exists() {
            report_skip("this file system tells `TASKS` from `tasks`, so no other case names it");
            return Ok(());
        }
        let directory = root.join("tasks/lint");

        assert!(lies_under(&root.join("Tasks/Lint"), &directory));
        assert!(lies_under(&root.join("TASKS/LINT/src"), &directory));
        assert!(!lies_under(&root.join("Tasks/Fmt"), &directory));
        Ok(())
    }

    /// Cargo removes a `..` as text from the paths it normalises, and the
    /// system takes it up from where a link leads in a path opened as
    /// written, such as an `include`.
    #[test]
    fn a_dot_dot_after_a_link_is_text_for_a_normalised_path_and_physical_for_an_opened_one()
    -> TestOutcome {
        let (_scratch, root) = project("paths-dot-dot-after-link")?;
        std::fs::create_dir_all(root.join("a/b"))?;
        std::fs::create_dir_all(root.join("a/x"))?;
        std::fs::create_dir_all(root.join("x"))?;
        symlink("a/b", root.join("l"))?;
        let through = root.join("l/../x");

        assert!(lies_under(&through, &root.join("x")));
        assert!(!lies_under(&through, &root.join("a/x")));
        assert!(opens_through(&through, &root.join("a/x")));
        assert!(!opens_through(&through, &root.join("x")));
        Ok(())
    }

    /// Past the first component that does not exist the filesystem has no
    /// answer, so the rest is compared as text, after the part that does
    /// exist is resolved, and with a link at the directory kept as the link.
    #[test]
    fn past_a_missing_component_the_rest_is_compared_as_text() -> TestOutcome {
        let (_scratch, root) = project("paths-missing")?;
        symlink("tasks", root.join("alias"))?;
        std::fs::create_dir_all(root.join("vendor/lint"))?;
        symlink("../vendor/lint", root.join("tasks/linked"))?;
        let directory = root.join("tasks/lint");

        assert!(opens_through(&root.join("ghost/../tasks/lint"), &directory));
        assert!(opens_through(
            &root.join("alias/ghost/../lint/x"),
            &directory
        ));
        assert!(!opens_through(&root.join("alias/ghost/../fmt"), &directory));
        assert!(opens_through(
            &root.join("tasks/ghost/../linked"),
            &root.join("tasks/linked")
        ));
        assert!(!opens_through(
            &root.join("vendor/ghost/../lint"),
            &root.join("tasks/linked")
        ));
        Ok(())
    }

    #[test]
    fn a_link_that_leads_back_to_itself_lies_under_nothing() -> TestOutcome {
        let (_scratch, root) = project("paths-link-loop")?;
        symlink("loop", root.join("loop"))?;

        assert!(!lies_under(&root.join("loop/x"), &root.join("tasks/lint")));
        Ok(())
    }

    #[test]
    fn a_relative_path_climbs_to_the_shared_directory_then_descends() {
        for (from, to, expected) in [
            ("/w/ritual", "/w/.rituals/greet", "../.rituals/greet"),
            ("/w/crates/cli", "/w/tasks/new", "../../tasks/new"),
            ("/w", "/w/tasks/lint", "tasks/lint"),
            ("/w/tasks/shout", "/w/tasks/greet", "../greet"),
            ("/w/.rituals/shout", "/w/tasks/helper", "../../tasks/helper"),
            ("/w/tasks/lint", "/w", "../.."),
            ("/w/tasks/lint", "/w/tasks", ".."),
            // Nothing in common but the root.
            ("/a/b", "/c/d", "../../c/d"),
            ("/", "/c/d", "c/d"),
            ("/a/b", "/", "../.."),
            // A directory is not inside another that only shares its prefix.
            ("/w/demo", "/w/demo-extra/x", "../demo-extra/x"),
            ("/w/demo", "/w/demo/.github/ci.yml", ".github/ci.yml"),
        ] {
            assert_eq!(
                relative(Path::new(from), Path::new(to)),
                expected,
                "{from} -> {to}"
            );
        }
    }

    /// The same directory is `.`, not the empty string, which no manifest
    /// can use as a path.
    #[test]
    fn a_relative_path_from_a_directory_to_itself_is_a_dot() {
        assert_eq!(relative(Path::new("/w/tasks"), Path::new("/w/tasks")), ".");
        assert_eq!(relative(Path::new("/"), Path::new("/")), ".");
    }

    /// Joined back to the directory it was made from, a relative path leads
    /// to where it was made to.
    #[test]
    fn a_relative_path_joined_back_leads_to_its_target() {
        for (from, to) in [
            ("/w/ritual", "/w/.rituals/greet"),
            ("/w/tasks/shout", "/w/tasks/greet"),
            ("/w/tasks/lint", "/w"),
            ("/a/b", "/c/d"),
            ("/w/tasks", "/w/tasks"),
        ] {
            let back = relative(Path::new(from), Path::new(to));
            assert_eq!(
                normalize(&Path::new(from).join(&back)),
                Path::new(to),
                "{from} + {back}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "from_directory must be absolute")]
    fn a_relative_origin_has_no_relative_path() {
        let _ = relative(Path::new("tasks"), Path::new("/w/tasks"));
    }

    #[test]
    #[should_panic(expected = "to must be absolute")]
    fn a_relative_target_has_no_relative_path() {
        let _ = relative(Path::new("/w"), Path::new("tasks"));
    }
}
