//! Where a path goes when a set of directories moves.
//!
//! A task that moves directories has to repoint every path that reaches
//! them. A [`Relocation`] is the answer to the one question that needs:
//! given a path, where is what it names after the move? It is built once,
//! from the directories that move, and read-only after that, so every
//! manifest it repoints sees the same answer. [`Manifest::repoint`] applies
//! it to every path a manifest writes.
//!
//! [`Manifest::repoint`]: crate::manifest::Manifest::repoint
//!
//! # Examples
//!
//! Two tasks move from `tasks/` to `.rituals/`, and a path that leads to one
//! of them leads to its new home:
//!
//! ```
//! use std::path::{Path, PathBuf};
//!
//! use rituals_compose::relocation::Relocation;
//!
//! let root = std::env::temp_dir().join("workspace");
//! let relocation = Relocation::new(
//!     &root.join("tasks"),
//!     &root.join(".rituals"),
//!     [root.join("tasks/greet"), root.join("tasks/shout")],
//! );
//!
//! assert_eq!(
//!     relocation.destination(&root.join("tasks/greet/src/lib.rs")),
//!     Some(root.join(".rituals/greet/src/lib.rs"))
//! );
//! // `vendor` does not move, so there is nowhere new for it to be.
//! assert_eq!(relocation.destination(&root.join("vendor/x")), None);
//! ```

use std::path::{Path, PathBuf};

use crate::paths::{normalize, relative};

/// A set of directories that move from one parent to another, and where each
/// path at or under one of them goes.
///
/// Every moved directory is a strict descendant of the directory it moves
/// out of, and keeps its place beneath the directory it moves into: moving
/// `tasks/` to `.rituals/` sends `tasks/greet` to `.rituals/greet` and
/// `tasks/group/deep` to `.rituals/group/deep`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relocation {
    from_directory: PathBuf,
    to_directory: PathBuf,
    moved: Vec<PathBuf>,
}

impl Relocation {
    /// Builds the relocation of every directory in `moved`, which each lie
    /// under `from_directory`, to the same place under `to_directory`.
    ///
    /// All the paths are taken as normalised text, the way Cargo reads a
    /// path before it opens anything: `.` and `..` components removed.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::relocation::Relocation;
    ///
    /// let root = std::env::temp_dir().join("workspace");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/lint")],
    /// );
    ///
    /// assert_eq!(
    ///     relocation.destination(&root.join("tasks/lint")),
    ///     Some(root.join(".rituals/lint"))
    /// );
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `from_directory`, `to_directory` or any moved directory is
    /// not absolute, since a relative path depends on a working directory
    /// none of them carries, or if a moved directory is `from_directory`
    /// itself or is not under it, since then there is no place under
    /// `to_directory` for it to go.
    #[must_use]
    pub fn new(
        from_directory: &Path,
        to_directory: &Path,
        moved: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        assert!(
            from_directory.is_absolute(),
            "from_directory must be absolute, got {}",
            from_directory.display()
        );
        assert!(
            to_directory.is_absolute(),
            "to_directory must be absolute, got {}",
            to_directory.display()
        );
        let from_directory = normalize(from_directory);
        let to_directory = normalize(to_directory);

        let moved: Vec<PathBuf> = moved
            .into_iter()
            .map(|directory| {
                assert!(
                    directory.is_absolute(),
                    "a moved directory must be absolute, got {}",
                    directory.display()
                );
                let directory = normalize(&directory);
                assert!(
                    directory != from_directory,
                    "{} is the directory everything moves out of, so it cannot be one of the moved directories",
                    directory.display()
                );
                assert!(
                    directory.starts_with(&from_directory),
                    "{} is not under {}, so it has no place under {}",
                    directory.display(),
                    from_directory.display(),
                    to_directory.display()
                );
                directory
            })
            .collect();

        Self {
            from_directory,
            to_directory,
            moved,
        }
    }

    /// Where `path` is once the directories have moved, when `path` is at or
    /// under one of them, and `None` when it stays where it is.
    ///
    /// `path` is taken as normalised text, and compared by component, so
    /// `tasks/lint-extra` is not under a moved `tasks/lint`.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::relocation::Relocation;
    ///
    /// let root = std::env::temp_dir().join("workspace");
    /// let relocation = Relocation::new(
    ///     &root.join("tasks"),
    ///     &root.join(".rituals"),
    ///     [root.join("tasks/lint")],
    /// );
    ///
    /// assert_eq!(
    ///     relocation.destination(&root.join("tasks/./lint/../lint/Cargo.toml")),
    ///     Some(root.join(".rituals/lint/Cargo.toml"))
    /// );
    /// assert_eq!(relocation.destination(&root.join("tasks/lint-extra")), None);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `path` is not absolute.
    #[must_use]
    pub fn destination(&self, path: &Path) -> Option<PathBuf> {
        assert!(
            path.is_absolute(),
            "a path to relocate must be absolute, got {}",
            path.display()
        );
        let path = normalize(path);
        if !self
            .moved
            .iter()
            .any(|directory| path.starts_with(directory))
        {
            return None;
        }
        // Every moved directory is under `from_directory`, so a path under
        // one of them is too.
        let below = path.strip_prefix(&self.from_directory).unwrap_or_else(|_| {
            unreachable!(
                "{} is under a moved directory, and every moved directory is under {}",
                path.display(),
                self.from_directory.display()
            )
        });
        Some(self.to_directory.join(below))
    }

    /// The directory every moved directory moves out of.
    pub(crate) fn moved_out_of(&self) -> &Path {
        &self.from_directory
    }

    /// The directory every moved directory moves into.
    pub(crate) fn moved_into(&self) -> &Path {
        &self.to_directory
    }

    /// Every directory that moves, normalised.
    pub(crate) fn moved(&self) -> &[PathBuf] {
        &self.moved
    }

    /// What a path `written` in a manifest should say after the move, or
    /// `None` to keep what is written.
    ///
    /// The manifest is in `base_before` now and in `base_after` once the move
    /// is done, the same directory when the manifest itself does not move.
    /// The path it names is `written` joined to `base_before` and
    /// normalised; the target it should name afterwards is where that goes
    /// under this relocation, or that same place when it does not move.
    ///
    /// - An absolute `written` is replaced only when its target moves, by the
    ///   target's new absolute path.
    /// - A relative `written` is kept when, joined to `base_after`, it
    ///   already reaches the new target, so a person's spelling survives
    ///   whenever it still works.
    /// - Otherwise it is respelled relative to `base_after`.
    ///
    /// # Panics
    ///
    /// Panics if `base_before` or `base_after` is not absolute.
    pub(crate) fn repointed(
        &self,
        written: &str,
        base_before: &Path,
        base_after: &Path,
    ) -> Option<String> {
        assert!(
            base_before.is_absolute(),
            "base_before must be absolute, got {}",
            base_before.display()
        );
        assert!(
            base_after.is_absolute(),
            "base_after must be absolute, got {}",
            base_after.display()
        );
        let written = Path::new(written);
        if written.is_absolute() {
            return self
                .destination(written)
                .map(|target| target.display().to_string());
        }

        let old_target = normalize(&base_before.join(written));
        let new_target = self
            .destination(&old_target)
            .unwrap_or_else(|| old_target.clone());
        if normalize(&base_after.join(written)) == new_target {
            return None;
        }
        Some(relative(&normalize(base_after), &new_target))
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::Relocation;

    const FROM: &str = "/w/tasks";
    const TO: &str = "/w/.rituals";

    /// `tasks/` moving to `.rituals/`, with two tasks, one grouped task and
    /// one whose name another starts with.
    fn relocation() -> Relocation {
        Relocation::new(
            Path::new(FROM),
            Path::new(TO),
            [
                "/w/tasks/greet",
                "/w/tasks/shout",
                "/w/tasks/lint",
                "/w/tasks/group/deep",
            ]
            .map(PathBuf::from),
        )
    }

    /// `written`, in a manifest in `base_before` that is in `base_after` once
    /// the move is done, as the table below reads each row.
    type Row = (
        &'static str,
        &'static str,
        &'static str,
        Option<&'static str>,
    );

    /// Every shape a path can take against the move, one row each: what is
    /// written, where the manifest is before and after, and what it should
    /// say afterwards (`None` to keep what is written).
    const REPOINTED: &[Row] = &[
        // A manifest that stays put, reaching a task that moves.
        (
            "../tasks/greet",
            "/w/ritual",
            "/w/ritual",
            Some("../.rituals/greet"),
        ),
        ("tasks/greet", "/w", "/w", Some(".rituals/greet")),
        // Spelled as the person wrote it, the new spelling is canonical.
        ("./tasks/greet/", "/w", "/w", Some(".rituals/greet")),
        (
            "x/../../tasks/greet",
            "/w/ritual",
            "/w/ritual",
            Some("../.rituals/greet"),
        ),
        // Inside a moved directory, and a grouped one.
        ("tasks/greet/src", "/w", "/w", Some(".rituals/greet/src")),
        ("tasks/group/deep", "/w", "/w", Some(".rituals/group/deep")),
        // A sibling that shares a prefix with a moved name does not move.
        ("../tasks/lint-extra", "/w/ritual", "/w/ritual", None),
        (
            "../tasks/lint",
            "/w/ritual",
            "/w/ritual",
            Some("../.rituals/lint"),
        ),
        // Outside `tasks/`.
        ("../vendor/x", "/w/ritual", "/w/ritual", None),
        ("../crates/x", "/w/ritual", "/w/ritual", None),
        // `tasks/` itself is where tasks move from, not a task that moves.
        ("tasks", "/w", "/w", None),
        // Both ends move together, so the spelling still works.
        ("../greet", "/w/tasks/shout", "/w/.rituals/shout", None),
        ("../greet/", "/w/tasks/shout", "/w/.rituals/shout", None),
        (
            "../../greet",
            "/w/tasks/group/deep",
            "/w/.rituals/group/deep",
            None,
        ),
        // A moved manifest reaching something that stays, at the same depth.
        (
            "../../vendor/x",
            "/w/tasks/shout",
            "/w/.rituals/shout",
            None,
        ),
        ("build.rs", "/w/tasks/greet", "/w/.rituals/greet", None),
        (
            "../../README.md",
            "/w/tasks/greet",
            "/w/.rituals/greet",
            None,
        ),
        // A moved manifest reaching something beside it that stays.
        (
            "../helper",
            "/w/tasks/shout",
            "/w/.rituals/shout",
            Some("../../tasks/helper"),
        ),
        (
            "../README.md",
            "/w/tasks/greet",
            "/w/.rituals/greet",
            Some("../../tasks/README.md"),
        ),
        (
            "..",
            "/w/tasks/shout",
            "/w/.rituals/shout",
            Some("../../tasks"),
        ),
        (
            "../..",
            "/w/tasks/group/deep",
            "/w/.rituals/group/deep",
            Some("../../../tasks"),
        ),
        // Absolute paths are replaced only when their target moves.
        (
            "/w/tasks/greet",
            "/w/ritual",
            "/w/ritual",
            Some("/w/.rituals/greet"),
        ),
        (
            "/w/tasks/./greet/",
            "/w/ritual",
            "/w/ritual",
            Some("/w/.rituals/greet"),
        ),
        ("/w/vendor/x", "/w/ritual", "/w/ritual", None),
        ("/w/vendor/x", "/w/tasks/shout", "/w/.rituals/shout", None),
    ];

    #[test]
    fn every_shape_of_path_is_repointed_or_kept_as_its_row_says() {
        let relocation = relocation();
        for (written, base_before, base_after, expected) in REPOINTED {
            let repointed =
                relocation.repointed(written, Path::new(base_before), Path::new(base_after));
            assert_eq!(
                repointed.as_deref(),
                *expected,
                "`{written}` written in {base_before}, which is in {base_after} afterwards"
            );
        }
    }

    /// Whatever it says, a repointed path reaches the target the old one
    /// did, moved: joined to where the manifest is afterwards, it leads
    /// where the old path led before, under the relocation.
    #[test]
    fn a_repointed_path_reaches_where_the_old_one_led_after_the_move() {
        let relocation = relocation();
        for (written, base_before, base_after, _expected) in REPOINTED {
            let (base_before, base_after) = (Path::new(base_before), Path::new(base_after));
            let before = crate::paths::normalize(&base_before.join(written));
            let target = relocation.destination(&before).unwrap_or(before);
            let now = relocation
                .repointed(written, base_before, base_after)
                .unwrap_or_else(|| (*written).to_string());
            assert_eq!(
                crate::paths::normalize(&base_after.join(&now)),
                target,
                "`{written}` in {} became `{now}` in {}",
                base_before.display(),
                base_after.display()
            );
        }
    }

    #[test]
    fn a_path_at_or_under_a_moved_directory_has_a_destination() {
        let relocation = relocation();
        for (path, expected) in [
            ("/w/tasks/greet", Some("/w/.rituals/greet")),
            ("/w/tasks/greet/", Some("/w/.rituals/greet")),
            (
                "/w/tasks/greet/src/lib.rs",
                Some("/w/.rituals/greet/src/lib.rs"),
            ),
            ("/w/tasks/./x/../greet", Some("/w/.rituals/greet")),
            (
                "/w/tasks/group/deep/src",
                Some("/w/.rituals/group/deep/src"),
            ),
            // Not at or under a moved directory.
            ("/w/tasks", None),
            ("/w/tasks/group", None),
            ("/w/tasks/lint-extra", None),
            ("/w/tasks/lint-extra/src", None),
            ("/w", None),
            ("/w/vendor/greet", None),
            ("/w/.rituals/greet", None),
        ] {
            assert_eq!(
                relocation.destination(Path::new(path)),
                expected.map(PathBuf::from),
                "{path}"
            );
        }
    }

    #[test]
    fn no_moved_directory_moves_nothing() {
        let relocation = Relocation::new(Path::new(FROM), Path::new(TO), Vec::new());
        assert_eq!(relocation.destination(Path::new("/w/tasks/greet")), None);
        assert_eq!(
            relocation.repointed(
                "../tasks/greet",
                Path::new("/w/ritual"),
                Path::new("/w/ritual")
            ),
            None
        );
    }

    #[test]
    #[should_panic(expected = "from_directory must be absolute")]
    fn a_relative_from_directory_is_a_bug() {
        let _ = Relocation::new(Path::new("tasks"), Path::new(TO), Vec::new());
    }

    #[test]
    #[should_panic(expected = "to_directory must be absolute")]
    fn a_relative_to_directory_is_a_bug() {
        let _ = Relocation::new(Path::new(FROM), Path::new(".rituals"), Vec::new());
    }

    #[test]
    #[should_panic(expected = "a moved directory must be absolute")]
    fn a_relative_moved_directory_is_a_bug() {
        let _ = Relocation::new(
            Path::new(FROM),
            Path::new(TO),
            [PathBuf::from("tasks/greet")],
        );
    }

    #[test]
    #[should_panic(expected = "is not under /w/tasks")]
    fn a_moved_directory_outside_the_directory_it_moves_out_of_is_a_bug() {
        let _ = Relocation::new(
            Path::new(FROM),
            Path::new(TO),
            [PathBuf::from("/w/vendor/x")],
        );
    }

    #[test]
    #[should_panic(expected = "so it cannot be one of the moved directories")]
    fn the_directory_it_moves_out_of_cannot_be_a_moved_directory() {
        let _ = Relocation::new(Path::new(FROM), Path::new(TO), [PathBuf::from("/w/tasks/")]);
    }

    #[test]
    #[should_panic(expected = "a path to relocate must be absolute")]
    fn a_relative_path_has_no_destination() {
        let _ = relocation().destination(Path::new("tasks/greet"));
    }

    #[test]
    #[should_panic(expected = "base_before must be absolute")]
    fn a_relative_manifest_directory_is_a_bug() {
        let _ = relocation().repointed("../tasks/greet", Path::new("ritual"), Path::new("/ritual"));
    }

    #[test]
    #[should_panic(expected = "base_after must be absolute")]
    fn a_relative_base_after_is_a_bug() {
        let _ = relocation().repointed("../tasks/greet", Path::new("/ritual"), Path::new("ritual"));
    }
}
