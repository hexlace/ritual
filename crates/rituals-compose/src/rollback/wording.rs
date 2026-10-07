//! What a failed run says about what it did to the project, in the words of
//! the kind of run it was.

use std::path::{Path, PathBuf};

use crate::paths::relative;
use crate::sentence::join_with_and;

/// How [`attempt`](super::attempt) words the end of a failure that it undid:
/// what was put back, or could not be, and what to check before running
/// again.
///
/// There are exactly two kinds of run, and so two constructors. One changes
/// a project that was already there ([`Wording::project`]); the other makes a
/// directory from nothing and fills it ([`Wording::fresh_directory`]), so
/// undoing it is removing the directory. A third phrasing has nowhere to
/// come from, which keeps every task's report reading the same way.
///
/// Each names a path the way the rest of its run does: a project's paths
/// from the project root, as its `created` and `updated` lines do, and a
/// fresh directory as it was typed, as its success line does.
///
/// When the undo put nothing back, because every change was already as found
/// or nothing had been recorded, the failure is returned exactly as the run
/// raised it and no wording is used: a refusal that came before ritual
/// changed anything must not say a recovery happened. When nothing was put
/// back and something could not be, the failure says only what could not be.
///
/// # Examples
///
/// A run that edits a project says the project was put back:
///
/// ```
/// use rituals::Failure;
/// use rituals_compose::rollback::{self, Wording};
///
/// # let directory = std::env::temp_dir()
/// #     .join(format!("rituals-compose-doctest-wording-project-{}", std::process::id()));
/// # std::fs::create_dir_all(&directory)?;
/// let lockfile = directory.join("Cargo.lock");
///
/// let wording = Wording::project(&directory, "running `cargo ritual import lint` again");
/// let outcome = rollback::attempt(wording, |changes| {
///     changes.write(&lockfile, "version = 4\n")?;
///     Err::<(), _>(Failure::new("the task check failed"))
/// });
///
/// let failure = outcome.err().ok_or("the run was meant to fail")?;
/// assert_eq!(
///     failure.to_string(),
///     "the task check failed; ritual put the project back as it found it"
/// );
/// # std::fs::remove_dir_all(&directory)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// A run that makes a directory says the directory was removed:
///
/// ```
/// use std::path::Path;
///
/// use rituals::Failure;
/// use rituals_compose::rollback::{self, Wording};
///
/// # let current_dir = std::env::temp_dir()
/// #     .join(format!("rituals-compose-doctest-wording-fresh-{}", std::process::id()));
/// # std::fs::create_dir_all(&current_dir)?;
/// let wording =
///     Wording::fresh_directory(&current_dir, Path::new("lint"), "running `create lint` again");
/// let outcome = rollback::attempt(wording, |changes| {
///     changes.reserve_directory(&current_dir.join("lint"))?;
///     Err::<(), _>(Failure::new("writing lint/Cargo.toml failed"))
/// });
///
/// let failure = outcome.err().ok_or("the run was meant to fail")?;
/// assert_eq!(
///     failure.to_string(),
///     "writing lint/Cargo.toml failed; ritual removed lint so a retry starts clean"
/// );
/// assert!(!current_dir.join("lint").exists());
/// # std::fs::remove_dir_all(&current_dir)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Wording<'a> {
    kind: Kind<'a>,
    /// The directory every path in the report is spelled from.
    spelled_from: &'a Path,
    retry: &'a str,
}

/// Which of the two kinds of run a [`Wording`] is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind<'a> {
    /// A run that changes a project that was already there.
    Project,
    /// A run that makes this directory from nothing, spelled as the person
    /// typed it.
    FreshDirectory(&'a Path),
}

impl<'a> Wording<'a> {
    /// The wording for a run that changes the project rooted at `root`.
    ///
    /// `root` is the directory holding the workspace's root manifest, and
    /// every path the report names is spelled from it, as the run's
    /// `created` and `updated` lines are. `retry` is what a person runs to
    /// try again, because only the caller knows how it was reached: a task
    /// spells it from [`rituals::CommandLine::cargo_command`] and the words
    /// it was given, so it can be pasted however the task was mounted, such
    /// as ``"running `cargo ritual import lint` again"``. It ends the report
    /// when something could not be put back: "check it before running
    /// `cargo ritual import lint` again".
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    ///
    /// use rituals_compose::rollback::Wording;
    ///
    /// let retry = "running `cargo ritual remove lint` again";
    /// let wording = Wording::project(Path::new("/work/acme"), retry);
    /// # let _ = wording;
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `root` is not absolute, since a path spelled from a
    /// relative one depends on a working directory it does not carry.
    #[must_use]
    pub fn project(root: &'a Path, retry: &'a str) -> Self {
        assert_absolute(root);
        Self {
            kind: Kind::Project,
            spelled_from: root,
            retry,
        }
    }

    /// The wording for a run that makes `directory` from nothing, in
    /// `current_dir`, and fills it, so that undoing it is removing it.
    ///
    /// `directory` is spelled as the person typed it, relative to
    /// `current_dir`, because that is how the report names it; the run
    /// reserves `current_dir.join(directory)` first, through
    /// [`Changes::reserve_directory`](super::Changes::reserve_directory). A
    /// run that reserves another directory is a bug, and the undo panics
    /// rather than report the wrong directory as removed. `retry` ends the
    /// report as it does for [`Wording::project`]. A run that makes a
    /// directory from nothing runs where there is no project, and so no
    /// `cargo` alias to spell it with, so it names the command as typed
    /// after the binary's name, such as ``"running `new demo` again"``.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    ///
    /// use rituals_compose::rollback::Wording;
    ///
    /// let wording =
    ///     Wording::fresh_directory(Path::new("/work"), Path::new("demo"), "running `new demo` again");
    /// # let _ = wording;
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `current_dir` is not absolute, as for [`Wording::project`].
    #[must_use]
    pub fn fresh_directory(current_dir: &'a Path, directory: &'a Path, retry: &'a str) -> Self {
        assert_absolute(current_dir);
        Self {
            kind: Kind::FreshDirectory(directory),
            spelled_from: current_dir,
            retry,
        }
    }

    /// The directory a run with this wording reserves first, if it is one
    /// that makes a directory from nothing.
    pub(super) fn reserved_directory(&self) -> Option<PathBuf> {
        match self.kind {
            Kind::Project => None,
            Kind::FreshDirectory(directory) => Some(self.spelled_from.join(directory)),
        }
    }

    /// `path` as the report names it: from the project root, or from the
    /// directory a fresh one was made in.
    ///
    /// # Panics
    ///
    /// Panics if `path` is not absolute: every path a run records is, so one
    /// that is not is a bug in the run.
    pub(super) fn spell(&self, path: &Path) -> String {
        relative(self.spelled_from, path)
    }

    /// Where a command the report offers is run from, for the paths in it
    /// to mean what they say.
    pub(super) const fn where_commands_run(&self) -> &'static str {
        match self.kind {
            Kind::Project => "from the project root",
            Kind::FreshDirectory(_) => "from where ritual ran",
        }
    }

    /// The clause that continues a failure's sentence when every change that
    /// was made has been undone.
    pub(super) fn restored(&self) -> String {
        match self.kind {
            Kind::Project => "; ritual put the project back as it found it".to_string(),
            Kind::FreshDirectory(directory) => format!(
                "; ritual removed {} so a retry starts clean",
                directory.display()
            ),
        }
    }

    /// The clause that continues a failure's sentence when some changes
    /// were put back and `not_restored` could not be, naming each and what
    /// to do before running again.
    pub(super) fn partly_restored(&self, not_restored: &[String]) -> String {
        match self.kind {
            Kind::Project => format!(
                "; ritual put the project back except for {}",
                self.what_to_check(not_restored)
            ),
            Kind::FreshDirectory(_) => self.nothing_restored(not_restored),
        }
    }

    /// The clause that continues a failure's sentence when nothing was put
    /// back and `not_restored` could not be: it says only that, because the
    /// project was not put back at all.
    pub(super) fn nothing_restored(&self, not_restored: &[String]) -> String {
        let verb = match self.kind {
            Kind::Project => "put back",
            Kind::FreshDirectory(_) => "remove",
        };
        format!(
            "; ritual could not {verb} {}",
            self.what_to_check(not_restored)
        )
    }

    /// `not_restored`, listed, and what to do about it before running again.
    fn what_to_check(&self, not_restored: &[String]) -> String {
        assert!(
            !not_restored.is_empty(),
            "a clause about what could not be undone needs something that could not"
        );
        let pronoun = if not_restored.len() == 1 {
            "check it"
        } else {
            "check them"
        };
        format!(
            "{} — {pronoun} before {}",
            join_with_and(not_restored),
            self.retry
        )
    }
}

/// Asserts that `directory`, which a report spells paths from, is absolute.
fn assert_absolute(directory: &Path) {
    assert!(
        directory.is_absolute(),
        "a report spells paths from an absolute directory, got {}",
        directory.display()
    );
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::Wording;

    const ROOT: &str = "/work/acme";

    #[test]
    fn a_project_run_that_was_undone_says_the_project_was_put_back() {
        // The fixed sentence every project-changing task reports.
        let wording = Wording::project(Path::new(ROOT), "running `demo` again");
        assert_eq!(
            wording.restored(),
            "; ritual put the project back as it found it"
        );
    }

    #[test]
    fn a_fresh_directory_run_that_was_undone_names_the_directory_as_typed() {
        let wording = Wording::fresh_directory(
            Path::new("/work"),
            Path::new("demo"),
            "running `new demo` again",
        );
        assert_eq!(
            wording.restored(),
            "; ritual removed demo so a retry starts clean"
        );
    }

    #[test]
    fn what_could_not_be_put_back_is_named_with_one_pronoun_or_the_other() {
        // One item reads "check it", several read "check them", for each
        // kind of run, whether or not anything else was put back.
        let one = ["Cargo.toml".to_string()];
        let two = ["a".to_string(), "b".to_string()];
        let project = Wording::project(Path::new(ROOT), "running `demo` again");
        let fresh = Wording::fresh_directory(
            Path::new("/work"),
            Path::new("demo"),
            "running `new demo` again",
        );

        assert_eq!(
            project.partly_restored(&one),
            "; ritual put the project back except for Cargo.toml — check it before running \
             `demo` again"
        );
        assert_eq!(
            project.partly_restored(&two),
            "; ritual put the project back except for a and b — check them before running \
             `demo` again"
        );
        assert_eq!(
            project.nothing_restored(&one),
            "; ritual could not put back Cargo.toml — check it before running `demo` again"
        );
        assert_eq!(
            project.nothing_restored(&two),
            "; ritual could not put back a and b — check them before running `demo` again"
        );
        for clause in [fresh.partly_restored(&one), fresh.nothing_restored(&one)] {
            assert_eq!(
                clause,
                "; ritual could not remove Cargo.toml — check it before running `new demo` again"
            );
        }
        assert_eq!(
            fresh.nothing_restored(&two),
            "; ritual could not remove a and b — check them before running `new demo` again"
        );
    }

    #[test]
    fn a_path_is_spelled_from_the_root_or_from_where_the_directory_was_made() {
        let project = Wording::project(Path::new(ROOT), "again");
        let fresh = Wording::fresh_directory(Path::new("/work"), Path::new("demo"), "again");

        assert_eq!(
            project.spell(Path::new("/work/acme/.rituals/lint")),
            ".rituals/lint"
        );
        assert_eq!(fresh.spell(Path::new("/work/demo")), "demo");
    }

    #[test]
    #[should_panic(expected = "needs something that could not")]
    fn a_clause_about_nothing_that_could_not_be_undone_is_a_bug() {
        let _ = Wording::project(Path::new(ROOT), "running `demo` again").nothing_restored(&[]);
    }

    #[test]
    #[should_panic(expected = "spells paths from an absolute directory")]
    fn a_relative_root_is_a_bug() {
        let _ = Wording::project(Path::new("acme"), "running `demo` again");
    }

    #[test]
    fn only_a_fresh_directory_wording_has_a_reserved_directory() {
        assert_eq!(
            Wording::project(Path::new(ROOT), "again").reserved_directory(),
            None
        );
        assert_eq!(
            Wording::fresh_directory(Path::new("/work"), Path::new("demo"), "again")
                .reserved_directory(),
            Some(PathBuf::from("/work/demo"))
        );
    }
}
