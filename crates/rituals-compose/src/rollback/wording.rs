//! What a failed run says about what it did to the project, in the words of
//! the kind of run it was.

use std::path::Path;

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
/// When the undo put nothing back, because every change was already as found
/// or nothing had been recorded, the failure is returned exactly as the run
/// raised it and no wording is used: a refusal that came before ritual
/// changed anything must not say a recovery happened.
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
/// let outcome = rollback::attempt(Wording::project("running `import lint` again"), |changes| {
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
/// # let directory = std::env::temp_dir()
/// #     .join(format!("rituals-compose-doctest-wording-fresh-{}", std::process::id()));
/// # std::fs::create_dir_all(&directory)?;
/// let made = directory.join("lint");
///
/// let outcome = rollback::attempt(
///     Wording::fresh_directory(Path::new("lint"), "running `create lint` again"),
///     |changes| {
///         changes.reserve_directory(&made)?;
///         Err::<(), _>(Failure::new("writing lint/Cargo.toml failed"))
///     },
/// );
///
/// let failure = outcome.err().ok_or("the run was meant to fail")?;
/// assert_eq!(
///     failure.to_string(),
///     "writing lint/Cargo.toml failed; ritual removed lint so a retry starts clean"
/// );
/// assert!(!made.exists());
/// # std::fs::remove_dir_all(&directory)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Wording<'a> {
    kind: Kind<'a>,
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
    /// The wording for a run that changes a project that was already there.
    ///
    /// `retry` is what a person types to try again, such as ``"running
    /// `import lint` again"``, because only the caller knows. It ends the
    /// report when something could not be put back: "check it before
    /// running `import lint` again".
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::rollback::Wording;
    ///
    /// let wording = Wording::project("running `remove lint` again");
    /// # let _ = wording;
    /// ```
    #[must_use]
    pub const fn project(retry: &'a str) -> Self {
        Self {
            kind: Kind::Project,
            retry,
        }
    }

    /// The wording for a run that makes `directory` from nothing and fills
    /// it, so that undoing it is removing it.
    ///
    /// `directory` is the directory the run reserves first, through
    /// [`Changes::reserve_directory`](super::Changes::reserve_directory),
    /// spelled as the person typed it, because that is how the report names
    /// it when it was removed. A run that names one directory here and
    /// reserves another is a bug, and the undo panics rather than report the
    /// wrong directory as removed. `retry` is as for [`Wording::project`].
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    ///
    /// use rituals_compose::rollback::Wording;
    ///
    /// let wording = Wording::fresh_directory(Path::new("demo"), "running `new demo` again");
    /// # let _ = wording;
    /// ```
    #[must_use]
    pub const fn fresh_directory(directory: &'a Path, retry: &'a str) -> Self {
        Self {
            kind: Kind::FreshDirectory(directory),
            retry,
        }
    }

    /// The directory a run with this wording reserves first, if it is one
    /// that makes a directory from nothing.
    pub(super) const fn reserved_directory(&self) -> Option<&'a Path> {
        match self.kind {
            Kind::Project => None,
            Kind::FreshDirectory(directory) => Some(directory),
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

    /// The clause that continues a failure's sentence when `not_restored`
    /// could not be undone, naming each and what to do before running again.
    pub(super) fn partly_restored(&self, not_restored: &[String]) -> String {
        assert!(
            !not_restored.is_empty(),
            "a clause about what could not be undone needs something that could not"
        );
        let pronoun = if not_restored.len() == 1 {
            "check it"
        } else {
            "check them"
        };
        let list = join_with_and(not_restored);
        let retry = self.retry;
        match self.kind {
            Kind::Project => {
                format!(
                    "; ritual put the project back except for {list} — {pronoun} before {retry}"
                )
            }
            Kind::FreshDirectory(_) => {
                format!("; ritual could not remove {list} — {pronoun} before {retry}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::Wording;

    #[test]
    fn a_project_run_that_was_undone_says_the_project_was_put_back() {
        // The fixed sentence every project-changing task reports.
        let wording = Wording::project("running `demo` again");
        assert_eq!(
            wording.restored(),
            "; ritual put the project back as it found it"
        );
    }

    #[test]
    fn a_fresh_directory_run_that_was_undone_names_the_directory_as_typed() {
        let wording = Wording::fresh_directory(Path::new("demo"), "running `new demo` again");
        assert_eq!(
            wording.restored(),
            "; ritual removed demo so a retry starts clean"
        );
    }

    #[test]
    fn what_could_not_be_put_back_is_named_with_one_pronoun_or_the_other() {
        // One item reads "check it", several read "check them", for each
        // kind of run.
        let one = ["/work/Cargo.toml".to_string()];
        let two = ["/work/a".to_string(), "/work/b".to_string()];
        let project = Wording::project("running `demo` again");
        let fresh = Wording::fresh_directory(Path::new("demo"), "running `new demo` again");

        assert_eq!(
            project.partly_restored(&one),
            "; ritual put the project back except for /work/Cargo.toml — check it before \
             running `demo` again"
        );
        assert_eq!(
            project.partly_restored(&two),
            "; ritual put the project back except for /work/a and /work/b — check them before \
             running `demo` again"
        );
        assert_eq!(
            fresh.partly_restored(&one),
            "; ritual could not remove /work/Cargo.toml — check it before running `new demo` \
             again"
        );
        assert_eq!(
            fresh.partly_restored(&two),
            "; ritual could not remove /work/a and /work/b — check them before running `new \
             demo` again"
        );
    }

    #[test]
    #[should_panic(expected = "needs something that could not")]
    fn a_clause_about_nothing_that_could_not_be_undone_is_a_bug() {
        let _ = Wording::project("running `demo` again").partly_restored(&[]);
    }

    #[test]
    fn only_a_fresh_directory_wording_has_a_reserved_directory() {
        let directory = Path::new("demo");
        assert_eq!(Wording::project("again").reserved_directory(), None);
        assert_eq!(
            Wording::fresh_directory(directory, "again").reserved_directory(),
            Some(directory)
        );
    }
}
