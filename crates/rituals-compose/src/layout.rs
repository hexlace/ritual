//! Where a scaffolder puts a ritual.
//!
//! Every ritual lives in `.rituals/`, whoever it is for. This module is the
//! one place that names that directory: a task that scaffolds a task crate
//! asks [`place_for`] where it goes and writes nothing under a directory name
//! of its own. It places a new ritual directly in `.rituals/`; a ritual
//! already further down is the project's own arrangement, and nothing here
//! reads a meaning into the names above it.
//!
//! # Examples
//!
//! Asking where a task called `lint` goes in the project at `/work/demo`:
//!
//! ```
//! use std::path::Path;
//!
//! use rituals::Name;
//! use rituals_compose::layout;
//!
//! let root = Path::new("/work/demo");
//! let place = layout::place_for(root, &Name::new("lint")?);
//!
//! assert_eq!(place.directory(), Path::new("/work/demo/.rituals/lint"));
//! assert_eq!(place.from_the_root(), ".rituals/lint");
//! assert_eq!(layout::tasks_directory(root), Path::new("/work/demo/.rituals"));
//! # Ok::<(), rituals::InvalidName>(())
//! ```

use std::path::{Path, PathBuf};

use rituals::Name;

/// The directory every ritual lives in, named from the workspace
/// root. Private so that nothing spells it on its own: a caller asks
/// [`tasks_directory`] or [`place_for`].
const TASKS_DIRECTORY: &str = ".rituals";

/// Where a task crate is written, spelled the two ways a task needs it.
///
/// # Examples
///
/// The one place a scaffolder writes the task crate, and the one spelling
/// of it a `[workspace] members` entry and a report line share:
///
/// ```
/// use std::path::Path;
///
/// use rituals::Name;
/// use rituals_compose::layout;
///
/// let place = layout::place_for(Path::new("/work/demo"), &Name::new("lint")?);
///
/// let written_to = place.directory().join("Cargo.toml");
/// let member = format!("members = [\"{}\"]", place.from_the_root());
/// assert_eq!(written_to, Path::new("/work/demo/.rituals/lint/Cargo.toml"));
/// assert_eq!(member, "members = [\".rituals/lint\"]");
/// # Ok::<(), rituals::InvalidName>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskPlace {
    directory: PathBuf,
    from_the_root: String,
}

impl TaskPlace {
    /// Returns the task crate's directory, absolute.
    ///
    /// # Examples
    ///
    /// The directory a scaffolder creates, before it writes any file in it:
    ///
    /// ```
    /// use std::path::Path;
    ///
    /// use rituals::Name;
    /// use rituals_compose::layout;
    ///
    /// let place = layout::place_for(Path::new("/work/demo"), &Name::new("lint")?);
    ///
    /// assert_eq!(place.directory(), Path::new("/work/demo/.rituals/lint"));
    /// assert!(place.directory().is_absolute());
    /// # Ok::<(), rituals::InvalidName>(())
    /// ```
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Returns the task crate's directory as a path from the workspace root,
    /// always spelled with `/`: the form a `[workspace] members` entry and a
    /// report line use.
    ///
    /// # Examples
    ///
    /// The line a task reports for the crate it wrote, spelled the way a
    /// person reads it from the project's root:
    ///
    /// ```
    /// use std::path::Path;
    ///
    /// use rituals::Name;
    /// use rituals_compose::layout;
    ///
    /// let place = layout::place_for(Path::new("/work/demo"), &Name::new("lint")?);
    ///
    /// assert_eq!(
    ///     format!("added {}", place.from_the_root()),
    ///     "added .rituals/lint"
    /// );
    /// # Ok::<(), rituals::InvalidName>(())
    /// ```
    #[must_use]
    pub fn from_the_root(&self) -> &str {
        &self.from_the_root
    }
}

/// Returns the directory every ritual lives under.
///
/// # Examples
///
/// Whether a directory is inside the one every ritual lives in, as a task
/// that lists them asks:
///
/// ```
/// use std::path::Path;
///
/// use rituals::Name;
/// use rituals_compose::layout;
///
/// let root = Path::new("/work/demo");
/// let tasks = layout::tasks_directory(root);
///
/// assert_eq!(tasks, Path::new("/work/demo/.rituals"));
/// assert!(layout::place_for(root, &Name::new("lint")?).directory().starts_with(&tasks));
/// assert!(!Path::new("/work/demo/src").starts_with(&tasks));
/// # Ok::<(), rituals::InvalidName>(())
/// ```
///
/// # Panics
///
/// Panics when `workspace_root` is not absolute: every path a task writes is
/// absolute, so a relative root is a bug in the caller.
#[must_use]
pub fn tasks_directory(workspace_root: &Path) -> PathBuf {
    assert!(
        workspace_root.is_absolute(),
        "workspace_root must be absolute, got {}",
        workspace_root.display()
    );
    workspace_root.join(TASKS_DIRECTORY)
}

/// Returns where the task called `name` goes in the workspace at
/// `workspace_root`.
///
/// Nothing is created: this only says where the task crate belongs.
///
/// # Examples
///
/// Scaffolding `lint` into the project at `/work/demo`: the directory to
/// create and the entry to add to `[workspace] members`:
///
/// ```
/// use std::path::Path;
///
/// use rituals::Name;
/// use rituals_compose::layout;
///
/// let place = layout::place_for(Path::new("/work/demo"), &Name::new("lint")?);
///
/// assert_eq!(place.directory(), Path::new("/work/demo/.rituals/lint"));
/// assert_eq!(place.from_the_root(), ".rituals/lint");
/// # Ok::<(), rituals::InvalidName>(())
/// ```
///
/// # Panics
///
/// Panics when `workspace_root` is not absolute.
#[must_use]
pub fn place_for(workspace_root: &Path, name: &Name) -> TaskPlace {
    let tasks = tasks_directory(workspace_root);
    let directory = tasks.join(name.as_str());
    // A validated name is one path component, so this holds for every name
    // there is; it is checked because the member entry and the dependency
    // path both rely on the directory being inside `tasks`.
    assert!(
        directory.starts_with(&tasks),
        "joining a validated Name under {} must stay under it",
        tasks.display()
    );
    assert!(
        directory != tasks,
        "joining a validated Name under {} must not name it itself",
        tasks.display()
    );
    TaskPlace {
        directory,
        from_the_root: format!("{TASKS_DIRECTORY}/{name}"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rituals::Name;

    use super::{place_for, tasks_directory};

    const ROOT: &str = "/work/demo";

    fn valid_name(value: &str) -> Name {
        Name::new(value).expect("a test passes only names it knows are valid")
    }

    #[test]
    fn tasks_live_in_dot_rituals_under_the_workspace_root() {
        assert_eq!(
            tasks_directory(Path::new(ROOT)),
            Path::new("/work/demo/.rituals")
        );
    }

    #[test]
    fn a_task_is_placed_under_the_tasks_directory() {
        let place = place_for(Path::new(ROOT), &valid_name("lint"));
        assert_eq!(place.directory(), Path::new("/work/demo/.rituals/lint"));
        assert!(
            place
                .directory()
                .starts_with(tasks_directory(Path::new(ROOT)))
        );
        assert_ne!(place.directory(), tasks_directory(Path::new(ROOT)));
    }

    #[test]
    fn the_path_from_the_root_is_spelled_with_a_slash() {
        let place = place_for(Path::new(ROOT), &valid_name("lint"));
        assert_eq!(place.from_the_root(), ".rituals/lint");
    }

    #[test]
    fn a_hyphenated_name_is_kept_as_typed() {
        let place = place_for(Path::new(ROOT), &valid_name("code-lint"));
        assert_eq!(
            place.directory(),
            Path::new("/work/demo/.rituals/code-lint")
        );
        assert_eq!(place.from_the_root(), ".rituals/code-lint");
    }

    #[test]
    #[should_panic(expected = "workspace_root must be absolute")]
    fn a_relative_workspace_root_is_a_bug() {
        let _ = place_for(Path::new("demo"), &valid_name("lint"));
    }
}
