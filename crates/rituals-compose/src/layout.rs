//! Where a scaffolder puts a ritual.
//!
//! Every ritual lives in `.rituals/`, whoever it is for. This module is the
//! one place that names that directory: a task that scaffolds a task crate
//! asks where it goes and writes nothing under a directory name of its own.
//! There are two ways to ask. [`place_for`] takes a name and places the
//! ritual directly in `.rituals/`. [`place_at`] takes a path a person typed
//! and places the ritual wherever below `.rituals/` it leads; nothing here
//! reads a meaning into the directories between, except that a ritual's own
//! directory is not one of them: [`ensure_in_no_member`] refuses a place
//! inside a crate that is already there.
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
//! assert_eq!(layout::rituals_directory(root), Path::new("/work/demo/.rituals"));
//! # Ok::<(), rituals::InvalidName>(())
//! ```

use std::path::{Path, PathBuf};

use rituals::{Failure, Name, Outcome};

use crate::paths;

/// The directory every ritual lives in, named from the workspace
/// root. Private so that nothing spells it on its own: a caller asks
/// [`rituals_directory`] or [`place_for`].
const RITUALS_DIRECTORY: &str = ".rituals";

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
    name: Name,
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

    /// Returns the task's name: the last component of its directory, which is
    /// also its key in the task list and its crate's name.
    ///
    /// # Examples
    ///
    /// The key a task is listed under, read from where it was placed:
    ///
    /// ```
    /// use std::path::Path;
    ///
    /// use rituals::Name;
    /// use rituals_compose::layout;
    ///
    /// let place = layout::place_for(Path::new("/work/demo"), &Name::new("lint")?);
    ///
    /// assert_eq!(place.name().as_str(), "lint");
    /// # Ok::<(), rituals::InvalidName>(())
    /// ```
    #[must_use]
    pub const fn name(&self) -> &Name {
        &self.name
    }
}

/// A path as a person typed it, and the directory they typed it in.
///
/// A plain carrier for [`place_at`]: the two paths are different things that
/// share a type, and named fields make swapping them visible at the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TypedPath<'a> {
    /// The directory the command ran in, absolute.
    pub current_dir: &'a Path,
    /// The path as typed, read against `current_dir` the way a shell reads it.
    pub path: &'a Path,
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
/// let rituals = layout::rituals_directory(root);
///
/// assert_eq!(rituals, Path::new("/work/demo/.rituals"));
/// assert!(layout::place_for(root, &Name::new("lint")?).directory().starts_with(&rituals));
/// assert!(!Path::new("/work/demo/src").starts_with(&rituals));
/// # Ok::<(), rituals::InvalidName>(())
/// ```
///
/// # Panics
///
/// Panics when `workspace_root` is not absolute: every path a task writes is
/// absolute, so a relative root is a bug in the caller.
#[must_use]
pub fn rituals_directory(workspace_root: &Path) -> PathBuf {
    assert!(
        workspace_root.is_absolute(),
        "workspace_root must be absolute, got {}",
        workspace_root.display()
    );
    workspace_root.join(RITUALS_DIRECTORY)
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
    let rituals = rituals_directory(workspace_root);
    let directory = rituals.join(name.as_str());
    // A validated name is one path component, so this holds for every name
    // there is; it is checked because the member entry and the dependency
    // path both rely on the directory being inside `rituals`.
    assert!(
        directory.starts_with(&rituals),
        "joining a validated Name under {} must stay under it",
        rituals.display()
    );
    assert!(
        directory != rituals,
        "joining a validated Name under {} must not name it itself",
        rituals.display()
    );
    TaskPlace {
        directory,
        from_the_root: format!("{RITUALS_DIRECTORY}/{name}"),
        name: name.clone(),
    }
}

/// Returns where the task a person typed `typed.path` for goes, in the
/// workspace at `workspace_root`.
///
/// The path is read the way a shell reads it: against `typed.current_dir`,
/// with `.` and `..` folded as text, so `.rituals/private/lint` from the root
/// and `private/lint` from inside `.rituals/` name one place. It has to lead
/// strictly below the rituals directory; its last component is the task's name,
/// its key and its crate's name. Nothing is created and no link is followed.
///
/// # Errors
///
/// Returns a [`Failure`] naming `typed.path` as it was typed, where it leads
/// from the workspace root, and the rituals directory, when the path does not
/// lead strictly below it, and one naming
/// the problem when its last component is not a valid [`Name`].
///
/// # Examples
///
/// A path typed from inside `.rituals/` and the same place typed from the
/// root:
///
/// ```
/// use std::path::Path;
///
/// use rituals_compose::layout::{self, TypedPath};
///
/// let root = Path::new("/work/demo");
/// let inside = TypedPath {
///     current_dir: Path::new("/work/demo/.rituals"),
///     path: Path::new("private/lint"),
/// };
/// let from_the_root = TypedPath {
///     current_dir: root,
///     path: Path::new(".rituals/private/lint"),
/// };
///
/// assert_eq!(layout::place_at(root, inside)?, layout::place_at(root, from_the_root)?);
/// assert_eq!(layout::place_at(root, inside)?.from_the_root(), ".rituals/private/lint");
///
/// let outside = TypedPath { current_dir: root, path: Path::new("src/lint") };
/// assert!(layout::place_at(root, outside).is_err());
/// # Ok::<(), rituals::Failure>(())
/// ```
///
/// # Panics
///
/// Panics when `workspace_root` or `typed.current_dir` is not absolute.
pub fn place_at(workspace_root: &Path, typed: TypedPath<'_>) -> Result<TaskPlace, Failure> {
    assert!(
        typed.current_dir.is_absolute(),
        "current_dir must be absolute, got {}",
        typed.current_dir.display()
    );
    let rituals = rituals_directory(workspace_root);
    let folded = paths::normalize(&typed.current_dir.join(typed.path));
    if !folded.starts_with(&rituals) || folded == rituals {
        return Err(outside_the_rituals_directory(
            typed.path,
            &paths::relative(workspace_root, &folded),
        ));
    }

    // Strictly below `rituals`, so there is a last component and a path from
    // the root to spell. A component that is not valid UTF-8 is not
    // supported: it reads lossily here and `Name` refuses the last.
    let Some(last) = folded.file_name() else {
        unreachable!("a path strictly below the rituals directory has a last component");
    };
    let name = Name::new(&last.to_string_lossy())?;
    let Ok(below_the_root) = folded.strip_prefix(workspace_root) else {
        unreachable!("a path below the rituals directory is below the workspace root");
    };
    let from_the_root = below_the_root.to_string_lossy().into_owned();
    assert!(
        from_the_root.starts_with(RITUALS_DIRECTORY),
        "a path below the rituals directory starts with {RITUALS_DIRECTORY}, got {from_the_root}"
    );
    Ok(TaskPlace {
        directory: folded,
        from_the_root,
        name,
    })
}

/// Refuses a ritual placed at or below a crate already in the workspace.
///
/// A ritual's own directory is its crate, not a grouping directory: a crate
/// made inside it would sit inside another member, and that member could no
/// longer be removed on its own. `members` are the workspace's members, each
/// a package name and its directory, as Cargo spells them. Only those inside
/// the rituals directory are asked about, so a command line crate at the
/// workspace root, which every ritual is below, is no obstacle. `typed` is the
/// path as the person typed it, which the refusal hands back.
///
/// # Errors
///
/// Returns a [`Failure`] naming the path, where it leads and the member it
/// leads into or to.
///
/// # Examples
///
/// ```
/// use std::path::Path;
///
/// use rituals_compose::layout::{self, TypedPath};
///
/// let root = Path::new("/work/demo");
/// let members = [("lint", Path::new("/work/demo/.rituals/lint")), ("demo-ritual", root)];
/// let typed = Path::new(".rituals/lint/inner");
/// let place = layout::place_at(root, TypedPath { current_dir: root, path: typed })?;
///
/// assert!(layout::ensure_in_no_member(root, typed, &place, members).is_err());
///
/// let grouped = Path::new(".rituals/private/inner");
/// let place = layout::place_at(root, TypedPath { current_dir: root, path: grouped })?;
/// layout::ensure_in_no_member(root, grouped, &place, members)?;
/// # Ok::<(), rituals::Failure>(())
/// ```
pub fn ensure_in_no_member<'a>(
    workspace_root: &Path,
    typed: &Path,
    place: &TaskPlace,
    members: impl IntoIterator<Item = (&'a str, &'a Path)>,
) -> Outcome {
    let rituals = rituals_directory(workspace_root);
    let Some((member, directory)) = members.into_iter().find(|(_, directory)| {
        directory.starts_with(&rituals) && place.directory().starts_with(directory)
    }) else {
        return Ok(());
    };
    let into = if place.directory() == directory {
        format!("which is the crate `{member}`")
    } else {
        format!(
            "inside the crate `{member}` at {}",
            paths::relative(workspace_root, directory)
        )
    };
    Err(Failure::new(format!(
        "refusing to create {}: this one leads to {}, {into}; a ritual never goes inside \
         another crate, so give a path that leads elsewhere below {RITUALS_DIRECTORY}/",
        typed.display(),
        place.from_the_root(),
    )))
}

/// The refusal for a path that does not lead strictly below the rituals
/// directory: what was typed, and `landed`, where it leads, spelled from the
/// workspace root. Naming where it led, and that a path is read from the
/// current directory, is what makes the refusal make sense when what was
/// typed looks as if it is below the rituals directory but was typed from
/// somewhere else.
fn outside_the_rituals_directory(typed: &Path, landed: &str) -> Failure {
    Failure::new(format!(
        "refusing to create {}: a path is read from the current directory, and this one \
         leads to {landed}; inside a project every ritual lives below {RITUALS_DIRECTORY}/ at \
         the project's root, so give a bare name, or a path that leads below it",
        typed.display()
    ))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rituals::Name;

    use super::{TypedPath, ensure_in_no_member, place_at, place_for, rituals_directory};

    const ROOT: &str = "/work/demo";

    fn valid_name(value: &str) -> Name {
        Name::new(value).expect("a test passes only names it knows are valid")
    }

    #[test]
    fn tasks_live_in_dot_rituals_under_the_workspace_root() {
        assert_eq!(
            rituals_directory(Path::new(ROOT)),
            Path::new("/work/demo/.rituals")
        );
    }

    #[test]
    fn a_task_is_placed_under_the_rituals_directory() {
        let place = place_for(Path::new(ROOT), &valid_name("lint"));
        assert_eq!(place.directory(), Path::new("/work/demo/.rituals/lint"));
        assert!(
            place
                .directory()
                .starts_with(rituals_directory(Path::new(ROOT)))
        );
        assert_ne!(place.directory(), rituals_directory(Path::new(ROOT)));
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

    #[test]
    fn place_for_and_place_at_agree_on_the_name_and_the_place_of_a_bare_name() {
        let by_name = place_for(Path::new(ROOT), &valid_name("lint"));
        let by_path = place_at(Path::new(ROOT), typed(ROOT, ".rituals/lint"))
            .expect("a path directly below .rituals is placed");
        assert_eq!(by_name.name().as_str(), "lint");
        assert_eq!(by_path.name().as_str(), "lint");
        assert_eq!(by_name, by_path);
    }

    fn typed<'a>(current_dir: &'a str, path: &'a str) -> TypedPath<'a> {
        TypedPath {
            current_dir: Path::new(current_dir),
            path: Path::new(path),
        }
    }

    /// Every way of typing a place below `.rituals/` that is read as a shell
    /// reads it: from the root, from inside `.rituals/`, from a task's own
    /// directory, with `.` and `..` folded, and as an absolute path. Each
    /// row is the directory typed in, the path typed, and the place it must
    /// name from the workspace root.
    #[test]
    fn a_typed_path_is_read_as_a_shell_reads_it() {
        let cases = [
            (ROOT, ".rituals/lint", ".rituals/lint"),
            (ROOT, ".rituals/private/lint", ".rituals/private/lint"),
            ("/work/demo/.rituals", "lint", ".rituals/lint"),
            (
                "/work/demo/.rituals",
                "private/lint",
                ".rituals/private/lint",
            ),
            ("/work/demo/.rituals/first", "../lint", ".rituals/lint"),
            (
                "/work/demo/.rituals/first",
                "../private/lint",
                ".rituals/private/lint",
            ),
            (ROOT, ".rituals/a/../lint", ".rituals/lint"),
            (ROOT, "./.rituals/./lint", ".rituals/lint"),
            ("/work/demo/src", "../.rituals/lint", ".rituals/lint"),
            ("/elsewhere", "/work/demo/.rituals/lint", ".rituals/lint"),
        ];
        for (current_dir, path, expected) in cases {
            let placed = place_at(Path::new(ROOT), typed(current_dir, path));
            assert!(placed.is_ok(), "{path} from {current_dir}: {placed:?}");
            if let Ok(place) = placed {
                assert_eq!(place.from_the_root(), expected, "{path} from {current_dir}");
                assert_eq!(
                    place.directory(),
                    Path::new(ROOT).join(expected),
                    "{path} from {current_dir}"
                );
                assert_eq!(place.name().as_str(), "lint");
            }
        }
    }

    /// The paths that do not lead strictly below `.rituals/`: a sibling of
    /// it, the directory itself with and without a trailing slash, out of
    /// the project, outside by an absolute path, and one that names
    /// `.rituals/` but was typed from another directory. Each is refused
    /// naming what was typed, where it leads from the workspace root, and
    /// `.rituals/`.
    #[test]
    fn a_path_that_does_not_lead_strictly_below_the_rituals_directory_is_refused() {
        let cases = [
            (ROOT, "src/lint", "src/lint"),
            (ROOT, ".rituals", ".rituals"),
            (ROOT, ".rituals/", ".rituals"),
            (ROOT, "./.rituals/./..", "."),
            (ROOT, ".rituals/../lint", "lint"),
            (ROOT, "../x", "../x"),
            ("/work/demo/.rituals/first", "../../lint", "lint"),
            (ROOT, "/etc/lint", "../../etc/lint"),
            (ROOT, "/work/demo/.rituals", ".rituals"),
            ("/work/demo/ritual", ".rituals/lint", "ritual/.rituals/lint"),
        ];
        for (current_dir, path, landed) in cases {
            let refused = place_at(Path::new(ROOT), typed(current_dir, path));
            assert!(
                refused.is_err(),
                "{path} from {current_dir} was placed: {refused:?}"
            );
            if let Err(failure) = refused {
                let message = failure.to_string();
                assert!(
                    message.contains(&format!("refusing to create {path}:")),
                    "{message}"
                );
                assert!(
                    message.contains(&format!(
                        "a path is read from the current directory, and this one leads to \
                         {landed};"
                    )),
                    "{message}"
                );
                assert!(message.contains("below .rituals/"), "{message}");
            }
        }
    }

    #[test]
    fn a_last_component_that_is_not_a_valid_name_is_refused_by_name() {
        for path in [
            ".rituals/Lint",
            ".rituals/9lives",
            ".rituals/lint-",
            ".rituals/a/crate",
        ] {
            let refused = place_at(Path::new(ROOT), typed(ROOT, path));
            assert!(refused.is_err(), "{path} was placed: {refused:?}");
        }
    }

    #[test]
    #[should_panic(expected = "current_dir must be absolute")]
    fn a_relative_current_dir_is_a_bug() {
        let _ = place_at(Path::new(ROOT), typed("demo", "lint"));
    }

    /// The members of a project at [`ROOT`] whose command line crate is the
    /// workspace root, with one ritual directly in `.rituals/` and one in a
    /// grouping directory.
    fn members() -> [(&'static str, &'static Path); 3] {
        [
            ("demo-ritual", Path::new(ROOT)),
            ("lint", Path::new("/work/demo/.rituals/lint")),
            ("fmt", Path::new("/work/demo/.rituals/private/fmt")),
        ]
    }

    fn refusal_for(typed: &str) -> Option<String> {
        let root = Path::new(ROOT);
        let typed = Path::new(typed);
        let place = place_at(
            root,
            TypedPath {
                current_dir: root,
                path: typed,
            },
        )
        .expect("a test passes only paths that lead below the rituals directory");
        ensure_in_no_member(root, typed, &place, members())
            .err()
            .map(|failure| failure.to_string())
    }

    #[test]
    fn a_place_inside_a_ritual_is_refused_naming_it_and_where_it_is() {
        assert_eq!(
            refusal_for(".rituals/lint/src/inner").as_deref(),
            Some(
                "refusing to create .rituals/lint/src/inner: this one leads to \
                 .rituals/lint/src/inner, inside the crate `lint` at .rituals/lint; a ritual \
                 never goes inside another crate, so give a path that leads elsewhere below \
                 .rituals/"
            )
        );
        assert!(
            refusal_for(".rituals/private/fmt/inner").is_some_and(
                |refusal| refusal.contains("inside the crate `fmt` at .rituals/private/fmt")
            )
        );
    }

    #[test]
    fn a_place_that_is_a_ritual_is_refused_as_that_crate() {
        assert_eq!(
            refusal_for(".rituals/lint").as_deref(),
            Some(
                "refusing to create .rituals/lint: this one leads to .rituals/lint, which is the \
                 crate `lint`; a ritual never goes inside another crate, so give a path that \
                 leads elsewhere below .rituals/"
            )
        );
    }

    /// A grouping directory is no member, a name that only begins with a
    /// member's is a different directory, and the command line crate at the
    /// root holds every ritual without being one they go inside.
    #[test]
    fn a_place_beside_every_ritual_is_allowed_whatever_holds_the_root() {
        for typed in [".rituals/private/lint", ".rituals/lint-two", ".rituals/new"] {
            assert_eq!(refusal_for(typed), None, "{typed}");
        }
    }
}
