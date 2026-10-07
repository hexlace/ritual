//! Putting a project back exactly as a run found it, when the run does not
//! finish.
//!
//! A management task that writes to a project promises that a run which
//! fails partway leaves the project as it was, and says so when it cannot.
//! [`attempt`] keeps that promise for every task that writes: the run gets a
//! [`Changes`], records each change through it before making it, and if the
//! run returns a failure, every recorded change is undone, the most recent
//! first, and the failure says what was put back, or what could not be. A
//! caller chooses the words with a [`Wording`].
//!
//! The only way to hold a [`Changes`] is inside [`attempt`], so a run cannot
//! forget to undo, and nothing can be recorded once the undo has started.
//! Each way of recording takes its snapshot before the change it makes:
//! [`Changes::write`] writes the file itself, [`Changes::run_changing`] runs
//! the change it guards, [`Changes::reserve_directory`] creates the directory
//! itself, [`Changes::rename`] moves the path itself, and
//! [`Changes::remove_empty_directory`] removes the directory itself. A change
//! made around
//! [`Changes`], such as a direct `std::fs` write or a command not run through
//! [`Changes::run_changing`], is not recorded at all.
//!
//! [`Changes::rename`] moves a directory whole, so the undo moves it back
//! whole, ignored files that no version control can restore included; a file
//! written through [`Changes::write`] before its directory moved is put back
//! at the path it was written at, because the directory goes back first.
//!
//! [`Changes::recorded_as_absent`] says what a run found when it first
//! recorded a file, for a run that reports a file as created rather than
//! updated.
//!
//! A failure the undo put nothing back for, because nothing had been recorded
//! or every change was already as found, is returned exactly as the run raised
//! it: a refusal that came before ritual changed anything says what is in the
//! way, and does not claim a recovery. One whose undo put nothing back and
//! could not put something back says only what it could not.
//!
//! The promise covers a run that returns a failure. A run that panics is not
//! undone.
//!
//! # Examples
//!
//! A run that writes two files and then fails puts both back: the one that
//! existed gets its bytes back, and the one it created is gone.
//!
//! ```
//! use rituals::Failure;
//! use rituals_compose::rollback::{self, Wording};
//!
//! # let directory = std::env::temp_dir()
//! #     .join(format!("rituals-compose-doctest-rollback-module-{}", std::process::id()));
//! # std::fs::create_dir_all(&directory)?;
//! let lockfile = directory.join("Cargo.lock");
//! let created = directory.join("notes.txt");
//! std::fs::write(&lockfile, "version = 4\n")?;
//!
//! let wording = Wording::project(&directory, "running `cargo ritual import lint` again");
//! let outcome = rollback::attempt(wording, |changes| {
//!     changes.write(&lockfile, "version = 4\n# changed\n")?;
//!     changes.write(&created, "a file that was not there before\n")?;
//!     Err::<(), _>(Failure::new("the task check failed"))
//! });
//!
//! let failure = outcome.err().ok_or("the run was meant to fail")?;
//! assert_eq!(
//!     failure.to_string(),
//!     "the task check failed; ritual put the project back as it found it"
//! );
//! assert_eq!(std::fs::read_to_string(&lockfile)?, "version = 4\n");
//! assert!(!created.exists());
//! # std::fs::remove_dir_all(&directory)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//
// Like `Manifest::write`, nothing here checks that a file on disk still
// holds what the run last wrote before putting the original back: one
// person runs a management task by hand, in one checkout, and every write it
// makes is visible in `git diff` before it is committed.

mod wording;

use std::path::{Path, PathBuf};

use rituals::Failure;

pub use wording::Wording;

/// Runs `run`, and if it fails, undoes every change it recorded in the
/// [`Changes`] it was handed, most recent first.
///
/// On success the changes are kept and `run`'s value is returned. On
/// failure the failure is returned with what happened to the project added
/// to it, in the words of `wording`:
/// - when every change was undone and at least one had to be put back,
///   `"<failure>; ritual put the project back as it found it"` for
///   [`Wording::project`], or `"<failure>; ritual removed <directory> so a
///   retry starts clean"` for [`Wording::fresh_directory`];
/// - when something could not be put back, every path that was not, followed
///   by `"— check it before <retry>"`: after `"ritual put the project back
///   except for"` when other changes were put back, and after `"ritual could
///   not put back"` when none were, so a report never claims a recovery that
///   did not happen. A [`Wording::fresh_directory`] says `"ritual could not
///   remove"` either way. A directory created only to hold something that
///   could not be removed is not named as well. `retry` is the caller's own,
///   because only the caller knows what a person types to try again;
/// - when the undo put nothing back and nothing failed, because nothing had
///   been recorded or every change was already as found, the failure exactly
///   as `run` raised it, with nothing added.
///
/// Every path is named as [`Wording`] says: from the project root, or from
/// the directory a fresh one was made in.
///
/// A full stop ending the failure is dropped only when a clause continues
/// its sentence.
///
/// # Examples
///
/// A run that creates a task's directory and fails before finishing leaves
/// no directory behind:
///
/// ```
/// use rituals::Failure;
/// use rituals_compose::rollback::{self, Wording};
///
/// # let directory = std::env::temp_dir()
/// #     .join(format!("rituals-compose-doctest-rollback-attempt-{}", std::process::id()));
/// # std::fs::create_dir_all(&directory)?;
/// let task_directory = directory.join(".rituals/lint");
///
/// let wording = Wording::project(&directory, "running `cargo ritual create lint` again");
/// let outcome = rollback::attempt(wording, |changes| {
///     changes.reserve_directory(&task_directory)?;
///     std::fs::create_dir_all(task_directory.join("src")).map_err(|error| {
///         Failure::new("creating the task's directory failed").caused_by(error)
///     })?;
///     Err::<(), _>(Failure::new("writing the workspace manifest failed"))
/// });
///
/// assert!(outcome.is_err());
/// assert!(!directory.join(".rituals").exists());
/// # std::fs::remove_dir_all(&directory)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// Returns `run`'s own failure, extended as described above.
///
/// # Panics
///
/// Panics if `wording` is a [`Wording::fresh_directory`] and the run recorded
/// something before it reserved that directory, or reserved another one
/// first, because the report would then name a directory the run did not
/// make.
///
/// Panics if called inside another [`attempt`] on the same thread. A nested
/// run's changes would be kept when it succeeds, and then survive the outer
/// run's failure while that failure says the project was put back. Code that
/// writes inside a run takes the outer run's `&mut Changes` instead.
pub fn attempt<T>(
    wording: Wording<'_>,
    run: impl FnOnce(&mut Changes) -> Result<T, Failure>,
) -> Result<T, Failure> {
    let _inside = InsideAttempt::enter();
    let mut changes = Changes::new();
    run(&mut changes).map_err(|failure| changes.undo(failure, wording))
}

// RS-NO-STATICS: a thread-local is the only place a check for nesting can
// live, because a nested call is handed nothing that ties it to the outer
// one. The inconsistency the rule warns about, two linked versions of this
// crate each with their own flag, is a case the check cannot reach anyway:
// a run under one version cannot hand its `Changes` to the other.
thread_local! {
    /// Whether this thread is inside [`attempt`]'s run.
    static INSIDE_ATTEMPT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks this thread as inside [`attempt`] for as long as it is held, and
/// clears the mark when dropped, so a run that panics does not leave every
/// later [`attempt`] on the thread refused.
struct InsideAttempt;

impl InsideAttempt {
    fn enter() -> Self {
        assert!(
            !INSIDE_ATTEMPT.replace(true),
            "rollback::attempt cannot be nested: the inner run's changes would survive the \
             outer run's failure; pass the outer run's &mut Changes down instead"
        );
        Self
    }
}

impl Drop for InsideAttempt {
    fn drop(&mut self) {
        INSIDE_ATTEMPT.set(false);
    }
}

/// What a run inside [`attempt`] has changed, recorded before each change is
/// made, so that a run that fails can be undone.
///
/// A file is recorded with the bytes it held, or as absent; a reserved
/// directory with the directories that had to be created to hold it.
/// Everything is undone in the reverse of the order it was recorded in.
#[derive(Debug)]
pub struct Changes {
    steps: Vec<Step>,
}

/// One recorded change, and how to undo it.
#[derive(Debug)]
enum Step {
    /// A file the run writes, or lets something else write. `original` is
    /// `None` when there was no file there, and undoing it removes whatever
    /// the run left in its place.
    File {
        path: PathBuf,
        original: Option<Vec<u8>>,
    },
    /// A directory the run reserved. Everything in it is the run's, so
    /// undoing it removes it and everything inside.
    Directory { path: PathBuf },
    /// A directory created only to hold a reserved one. Undoing it removes
    /// it only if it is empty: one that is not still holds something the
    /// undo did not manage to remove, and that has to be reported, not
    /// deleted.
    Parent { path: PathBuf },
    /// A path the run moved. Undoing it moves it back, whole, and only when
    /// `to` holds it and `from` is empty: a `from` that exists again would
    /// take it in as a child.
    Rename { from: PathBuf, to: PathBuf },
    /// An empty directory the run removed, with the permissions it had.
    /// Undoing it creates it again, empty, with those permissions.
    Removed {
        path: PathBuf,
        permissions: std::fs::Permissions,
    },
}

impl Changes {
    const fn new() -> Self {
        Self { steps: Vec::new() }
    }

    /// Writes `contents` to `path`, first recording what was there so a
    /// failed run can put it back byte for byte, or remove the file if there
    /// was none.
    ///
    /// A file written more than once in a run is restored to what it held
    /// before the first write.
    ///
    /// # Examples
    ///
    /// The generated file is rewritten, a later step fails, and the
    /// generated file goes back to exactly what it was:
    ///
    /// ```
    /// use rituals::Failure;
    /// use rituals_compose::rollback::{self, Wording};
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-rollback-write-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let generated = directory.join("main.rs");
    /// let original = "fn main() { rituals::run(); }\n";
    /// std::fs::write(&generated, original)?;
    ///
    /// let wording = Wording::project(&directory, "running `cargo ritual remove lint` again");
    /// let outcome = rollback::attempt(wording, |changes| {
    ///     changes.write(&generated, "fn main() {}\n")?;
    ///     Err::<(), _>(Failure::new("removing the dependency failed"))
    /// });
    ///
    /// assert!(outcome.is_err());
    /// assert_eq!(std::fs::read_to_string(&generated)?, original);
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming `path` if what is there cannot be read,
    /// or is a symbolic link to nothing, in which case nothing is written, or
    /// if writing it fails.
    pub fn write(&mut self, path: &Path, contents: impl AsRef<[u8]>) -> Result<(), Failure> {
        self.record_file(path)?;
        std::fs::write(path, contents).map_err(|error| {
            Failure::new(format!("writing {} failed", path.display())).caused_by(error)
        })
    }

    /// Records what each of `paths` holds, then runs `change`, for a change
    /// made by something other than this process, such as `cargo add`
    /// rewriting a manifest and `Cargo.lock`.
    ///
    /// Each file is restored byte for byte if the run fails, or removed if
    /// it did not exist before. `change` cannot reach this [`Changes`], so
    /// the snapshot is always taken before it runs.
    ///
    /// # Examples
    ///
    /// A command creates a lockfile where there was none, the run is then
    /// refused, and the lockfile is gone again:
    ///
    /// ```
    /// use rituals::Failure;
    /// use rituals_compose::rollback::{self, Wording};
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-rollback-run-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let lockfile = directory.join("Cargo.lock");
    ///
    /// let wording = Wording::project(&directory, "running `cargo ritual import lint` again");
    /// let outcome = rollback::attempt(wording, |changes| {
    ///     changes.run_changing(&[lockfile.as_path()], || {
    ///         // Stands in for running `cargo add`.
    ///         std::fs::write(&lockfile, "version = 4\n")
    ///             .map_err(|error| Failure::new("cargo add failed").caused_by(error))
    ///     })?;
    ///     Err::<(), _>(Failure::new("`lint` is not a task"))
    /// });
    ///
    /// assert!(outcome.is_err());
    /// assert!(!lockfile.exists());
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming the path if one of `paths` cannot be
    /// read, or is a symbolic link to nothing, in which case `change` does
    /// not run, or `change`'s own failure.
    pub fn run_changing<T>(
        &mut self,
        paths: &[&Path],
        change: impl FnOnce() -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        for path in paths {
            self.record_file(path)?;
        }
        change()
    }

    /// Creates `path` as an empty directory for this run to fill, along with
    /// every directory above it that does not exist yet, and records each one
    /// this call created, so a failed run removes them all. The caller fills
    /// the directory however it likes.
    ///
    /// The directory is created in one step that fails if anything is
    /// already there, so a directory that appears while the run is going is
    /// refused rather than taken for the run's own. A directory above it that
    /// appears in the meantime is used and left alone, because this call did
    /// not create it.
    ///
    /// The reserved directory is removed with everything in it. A directory
    /// above it is removed only if it is empty by then, so if the reserved
    /// directory could not be removed, both are reported rather than one
    /// being deleted out from under the other.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::rollback::{self, Wording};
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-rollback-reserve-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let existing = directory.join(".rituals/lint");
    /// std::fs::create_dir_all(&existing)?;
    ///
    /// // A directory that already exists is not the run's to remove.
    /// let wording = Wording::project(&directory, "running `cargo ritual create lint` again");
    /// let outcome = rollback::attempt(wording, |changes| {
    ///     changes.reserve_directory(&existing)
    /// });
    ///
    /// assert!(outcome.is_err());
    /// assert!(existing.exists());
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming `path` if it already exists: a directory
    /// that was there before the run is not the run's to remove. Returns a
    /// [`Failure`] naming the directory that could not be created if
    /// creating it fails for any other reason; the directories above it that
    /// this call did create are already recorded, so a failed run removes
    /// them.
    pub fn reserve_directory(&mut self, path: &Path) -> Result<(), Failure> {
        self.create_missing_parents_of(path)?;
        match std::fs::create_dir(path) {
            Ok(()) => {
                self.steps.push(Step::Directory {
                    path: path.to_path_buf(),
                });
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(Failure::new(format!(
                    "{} already exists, so ritual will not create it",
                    path.display()
                )))
            }
            Err(error) => Err(creating_failed(path, error)),
        }
    }

    /// Moves `from` to `to`, creating the directories above `to` that are
    /// missing, and records the move so a failed run moves it back.
    ///
    /// The move carries everything under `from`, files that are ignored and
    /// files that were never tracked included, which no version control can
    /// give back; the undo carries them back the same way. A file written
    /// through [`Changes::write`] before the move is put back at the path it
    /// was written at, with its original bytes, because the directory goes
    /// back before the file is restored. The directories created above `to`
    /// are recorded as [`Changes::reserve_directory`] records its own, so the
    /// undo removes them once they are empty.
    ///
    /// The arguments are in the order of [`std::fs::rename`], which this
    /// calls.
    ///
    /// When the undo cannot move the directory back, the report names both
    /// paths and, when `to` still holds it and `from` is empty, the `mv`
    /// command that puts it back, with where to run it from, its paths
    /// spelled as the report's are. When `from` exists
    /// again, no command is offered, because `mv` would put one directory
    /// inside the other.
    ///
    /// # Examples
    ///
    /// A task's directory moves, a later step fails, and it is back where it
    /// was with its build output:
    ///
    /// ```
    /// use rituals::Failure;
    /// use rituals_compose::rollback::{self, Wording};
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-rollback-rename-{}", std::process::id()));
    /// let from = directory.join("tasks/lint");
    /// let to = directory.join(".rituals/lint");
    /// # std::fs::create_dir_all(from.join("target"))?;
    /// std::fs::write(from.join("target/lint.d"), "built\n")?;
    ///
    /// let wording = Wording::project(&directory, "running `cargo ritual migrate` again");
    /// let outcome = rollback::attempt(wording, |changes| {
    ///     changes.rename(&from, &to)?;
    ///     Err::<(), _>(Failure::new("the project no longer builds"))
    /// });
    ///
    /// assert!(outcome.is_err());
    /// assert!(from.join("target/lint.d").is_file());
    /// assert!(!to.exists());
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming `to` if anything is there, a link to
    /// nothing included, and one naming `from` if there is nothing to move;
    /// in both cases nothing is moved and nothing is created. Returns a
    /// [`Failure`] naming the directory that could not be created, or the
    /// move, if either fails; the directories above `to` that this call did
    /// create are already recorded, so a failed run removes them.
    ///
    /// # Panics
    ///
    /// Panics if either path is not absolute, since a relative move depends
    /// on a working directory neither carries, or if `to` is under `from`,
    /// which cannot be moved into itself.
    pub fn rename(&mut self, from: &Path, to: &Path) -> Result<(), Failure> {
        assert!(
            from.is_absolute(),
            "a rename moves from an absolute path, got {}",
            from.display()
        );
        assert!(
            to.is_absolute(),
            "a rename moves to an absolute path, got {}",
            to.display()
        );
        assert!(
            !to.starts_with(from),
            "{} cannot be moved into itself, at {}",
            from.display(),
            to.display()
        );

        if is_present(to)? {
            return Err(Failure::new(format!(
                "{} already exists, so ritual will not move {} onto it",
                to.display(),
                from.display()
            )));
        }
        if !is_present(from)? {
            return Err(Failure::new(format!(
                "{} does not exist, so ritual has nothing to move",
                from.display()
            )));
        }

        self.create_missing_parents_of(to)?;
        std::fs::rename(from, to).map_err(|error| {
            Failure::new(format!(
                "moving {} to {} failed",
                from.display(),
                to.display()
            ))
            .caused_by(error)
        })?;
        self.steps.push(Step::Rename {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        });
        Ok(())
    }

    /// Removes the directory at `path` if it is empty, and records it so a
    /// failed run creates it again, empty, with the permissions it had.
    ///
    /// For a run whose result depends on a directory being gone, such as one
    /// that empties a directory by moving everything out of it and then asks
    /// Cargo whether a glob still matches it: removing the directory inside
    /// the run lets that question be asked of the project the run leaves.
    ///
    /// Undone in the reverse of the order it was recorded in, like every
    /// change, so a directory removed after a [`Changes::rename`] emptied it
    /// exists again before the undo moves anything back into it.
    ///
    /// The error is the file system's own, so a caller can tell a directory
    /// that is not empty, or no longer there, from one that could not be
    /// removed: only a removal that happened is recorded.
    ///
    /// # Examples
    ///
    /// A task's directory moves out of `tasks/`, `tasks/` is removed because
    /// nothing is left in it, a later step fails, and both are back:
    ///
    /// ```
    /// use rituals::Failure;
    /// use rituals_compose::rollback::{self, Wording};
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-rollback-remove-{}", std::process::id()));
    /// let from = directory.join("tasks/lint");
    /// let to = directory.join(".rituals/lint");
    /// # std::fs::create_dir_all(&from)?;
    /// std::fs::write(from.join("Cargo.toml"), "[package]\n")?;
    ///
    /// let wording = Wording::project(&directory, "running `cargo ritual migrate` again");
    /// let outcome = rollback::attempt(wording, |changes| {
    ///     changes.rename(&from, &to)?;
    ///     changes
    ///         .remove_empty_directory(&directory.join("tasks"))
    ///         .map_err(|error| Failure::new("removing tasks/ failed").caused_by(error))?;
    ///     Err::<(), _>(Failure::new("the project no longer loads"))
    /// });
    ///
    /// assert!(outcome.is_err());
    /// assert!(from.join("Cargo.toml").is_file());
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the file system's error if `path` cannot be read, is not
    /// empty, or cannot be removed; nothing is recorded and nothing is
    /// removed.
    pub fn remove_empty_directory(&mut self, path: &Path) -> std::io::Result<()> {
        let permissions = std::fs::metadata(path)?.permissions();
        std::fs::remove_dir(path)?;
        self.steps.push(Step::Removed {
            path: path.to_path_buf(),
            permissions,
        });
        Ok(())
    }

    /// Creates every directory above `path` that does not exist yet,
    /// outermost first, and records each one it created as a parent, so the
    /// undo removes them, innermost first, once they are empty. One that
    /// appears in the meantime is used and left alone, because this call
    /// did not create it.
    fn create_missing_parents_of(&mut self, path: &Path) -> Result<(), Failure> {
        let mut missing_parents: Vec<&Path> = path
            .ancestors()
            .skip(1)
            .take_while(|ancestor| !ancestor.as_os_str().is_empty() && !ancestor.exists())
            .collect();
        missing_parents.reverse();
        for parent in missing_parents {
            match std::fs::create_dir(parent) {
                Ok(()) => self.steps.push(Step::Parent {
                    path: parent.to_path_buf(),
                }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(creating_failed(parent, error)),
            }
        }
        Ok(())
    }

    /// Records the bytes at `path`, or that there is no file there, unless
    /// this run has already recorded `path`: the first record is the one
    /// that holds what the project had before the run.
    ///
    /// A symbolic link to nothing is refused rather than recorded as absent:
    /// writing through it would create its target, and undoing that would
    /// remove the link and leave the new target behind.
    fn record_file(&mut self, path: &Path) -> Result<(), Failure> {
        let already_recorded = self
            .steps
            .iter()
            .any(|step| matches!(step, Step::File { path: recorded, .. } if recorded == path));
        if already_recorded {
            return Ok(());
        }
        let original = match std::fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                refuse_a_link_to_nothing(path)?;
                None
            }
            Err(error) => {
                return Err(
                    Failure::new(format!("reading {} failed", path.display())).caused_by(error)
                );
            }
        };
        self.steps.push(Step::File {
            path: path.to_path_buf(),
            original,
        });
        Ok(())
    }

    /// Whether this run recorded `path` and found no file there, so that a
    /// file now at `path` is one the run made, whoever wrote it.
    ///
    /// Answers for what the run found when it first recorded `path`, however
    /// many times it has written it since, and is false for a path the run
    /// never recorded.
    ///
    /// # Examples
    ///
    /// A run reports a lockfile as created, not updated, when there was none
    /// before it:
    ///
    /// ```
    /// use rituals_compose::rollback::{self, Wording};
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-rollback-absent-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let lockfile = directory.join("Cargo.lock");
    ///
    /// let wording = Wording::project(&directory, "running `cargo ritual import lint` again");
    /// let verb = rollback::attempt(wording, |changes| {
    ///     changes.write(&lockfile, "version = 4\n")?;
    ///     Ok(if changes.recorded_as_absent(&lockfile) { "created" } else { "updated" })
    /// })?;
    ///
    /// assert_eq!(verb, "created");
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn recorded_as_absent(&self, path: &Path) -> bool {
        self.steps.iter().any(|step| {
            matches!(
                step,
                Step::File { path: recorded, original: None } if recorded == path
            )
        })
    }

    /// Undoes every recorded change, most recent first, and returns
    /// `failure` extended with what happened to the project, in the words of
    /// `wording`. Every step is attempted whatever happened to the ones
    /// before it, and each one that fails is named.
    ///
    /// A failure the undo put nothing back for is returned unchanged: every
    /// step was already as found, or there were none, so there is nothing to
    /// report and a claim of recovery would be false.
    fn undo(self, failure: Failure, wording: Wording<'_>) -> Failure {
        self.assert_names_the_reserved_directory(wording);

        // Where each thing that could not be put back is, and how the report
        // names it.
        let mut left: Vec<&Path> = Vec::new();
        let mut not_restored: Vec<String> = Vec::new();
        let mut any_put_back = false;
        for step in self.steps.iter().rev() {
            match step.undo() {
                Ok(Undone::PutBack) => any_put_back = true,
                Ok(Undone::AsFound) => {}
                // A parent is undone only once it is empty, so one that still
                // holds something already named cannot be removed for that
                // reason alone, and checking what it holds is checking it.
                Err(_) if step.holds_any_of(&left) => {}
                Err(_) => {
                    left.push(step.location());
                    not_restored.push(step.describe(wording));
                }
            }
        }

        match (not_restored.is_empty(), any_put_back) {
            (false, true) => continued(&failure, &wording.partly_restored(&not_restored)),
            (false, false) => continued(&failure, &wording.nothing_restored(&not_restored)),
            (true, true) => continued(&failure, &wording.restored()),
            (true, false) => failure,
        }
    }

    /// Asserts that a [`Wording::fresh_directory`] names the directory the run
    /// reserved first, which is the one the report says was removed.
    ///
    /// Pairs with the reservation: the wording's directory and the reserved
    /// one are supplied separately by the caller, and nothing else ties them
    /// together. Directories created only to hold the reserved one come
    /// first, so they are skipped.
    fn assert_names_the_reserved_directory(&self, wording: Wording<'_>) {
        let Some(directory) = wording.reserved_directory() else {
            return;
        };
        let first = self
            .steps
            .iter()
            .find(|step| !matches!(step, Step::Parent { .. }));
        if let Some(first) = first {
            assert!(
                matches!(first, Step::Directory { path } if *path == directory),
                "a run worded as making {} must reserve that directory before recording \
                 anything else; it recorded {first:?} first",
                directory.display()
            );
        }
    }
}

/// `failure`, every cause included, with `clause` continuing its sentence,
/// exiting with the status `failure` chose. The full stop that ended the
/// failure, which several refusals carry, goes first, because the clause
/// follows a semicolon.
fn continued(failure: &Failure, clause: &str) -> Failure {
    let status = failure.status();
    let failure = failure.with_causes().to_string();
    let failure = failure.strip_suffix('.').unwrap_or(&failure);
    Failure::new(format!("{failure}{clause}")).exiting_with(status)
}

/// What undoing one step did to the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Undone {
    /// The step changed something back to what the run found.
    PutBack,
    /// What the step guards was already as found, so nothing was touched.
    AsFound,
}

impl Step {
    /// What could not be put back, for the report that says so: the path,
    /// and for a move that could not be undone, where it belongs and what
    /// puts it there, each spelled as `wording` spells a path.
    fn describe(&self, wording: Wording<'_>) -> String {
        match self {
            Self::File { path, .. }
            | Self::Directory { path }
            | Self::Parent { path }
            | Self::Removed { path, .. } => wording.spell(path),
            Self::Rename { from, to } => describe_a_move_not_undone(from, to, wording),
        }
    }

    /// Where what this step guards is now: its path, or for a move, where
    /// the moved path went.
    fn location(&self) -> &Path {
        match self {
            Self::File { path, .. }
            | Self::Directory { path }
            | Self::Parent { path }
            | Self::Removed { path, .. } => path,
            Self::Rename { to, .. } => to,
        }
    }

    /// Whether this is a parent that holds one of `left`, which the undo
    /// could not remove from it.
    fn holds_any_of(&self, left: &[&Path]) -> bool {
        matches!(self, Self::Parent { path } if left.iter().any(|inside| inside.starts_with(path)))
    }

    /// Puts this step's path back as it was, doing nothing when it already
    /// is, so a change the run never got as far as making is not reported
    /// as one that could not be undone. Says which of the two it did.
    fn undo(&self) -> Result<Undone, Failure> {
        match self {
            Self::File {
                path,
                original: Some(original),
            } => restore_file(path, original),
            Self::File {
                path,
                original: None,
            } => remove_created_file(path),
            Self::Directory { path } => {
                if !path.exists() {
                    return Ok(Undone::AsFound);
                }
                std::fs::remove_dir_all(path)
                    .map(|()| Undone::PutBack)
                    .map_err(|error| {
                        Failure::new(format!("removing {} failed", path.display())).caused_by(error)
                    })
            }
            // Plain `remove_dir`, which refuses a directory that is not
            // empty: that refusal is the check that everything the run put
            // inside it really is gone.
            Self::Parent { path } => {
                if !path.exists() {
                    return Ok(Undone::AsFound);
                }
                std::fs::remove_dir(path)
                    .map(|()| Undone::PutBack)
                    .map_err(|error| {
                        Failure::new(format!("removing {} failed", path.display())).caused_by(error)
                    })
            }
            Self::Rename { from, to } => move_back(from, to),
            Self::Removed { path, permissions } => create_again(path, permissions),
        }
    }
}

/// Creates the directory a run removed, empty, with the permissions it had,
/// doing nothing when it is there again: one that exists already is either
/// the run's own, never removed, or something the undo must not replace.
fn create_again(path: &Path, permissions: &std::fs::Permissions) -> Result<Undone, Failure> {
    if is_present(path)? {
        return Ok(Undone::AsFound);
    }
    std::fs::create_dir(path)
        .and_then(|()| std::fs::set_permissions(path, permissions.clone()))
        .map(|()| Undone::PutBack)
        .map_err(|error| {
            Failure::new(format!("creating {} again failed", path.display())).caused_by(error)
        })
}

/// Moves `to` back to `from`, doing nothing when the move was never made.
///
/// Only the one state a move leaves behind is undone: `to` holds the
/// directory and `from` is empty. `from` holding something again means `to`
/// would move inside it, and neither holding anything means there is
/// nothing to move, so each of those is left as it is and reported.
fn move_back(from: &Path, to: &Path) -> Result<Undone, Failure> {
    match (is_present(from)?, is_present(to)?) {
        (false, true) => std::fs::rename(to, from)
            .map(|()| Undone::PutBack)
            .map_err(|error| {
                Failure::new(format!(
                    "moving {} back to {} failed",
                    to.display(),
                    from.display()
                ))
                .caused_by(error)
            }),
        (true, false) => Ok(Undone::AsFound),
        (true, true) => Err(Failure::new(format!(
            "{} and {} both exist, so {} cannot be moved back",
            from.display(),
            to.display(),
            to.display()
        ))),
        (false, false) => Err(Failure::new(format!(
            "neither {} nor {} exists, so there is nothing to move back",
            from.display(),
            to.display()
        ))),
    }
}

/// The report line for a move [`move_back`] could not undo, which names both
/// paths and says what to do about it, for the state it was left in.
///
/// The command is offered only when it is safe: `to` holds the directory and
/// `from` is empty, where `mv` puts it back. A `from` that exists again would
/// take it in as a child. The command's paths are spelled as the report's
/// are, so it says where to run it from.
fn describe_a_move_not_undone(from: &Path, to: &Path, wording: Wording<'_>) -> String {
    let (from_text, to_text) = (wording.spell(from), wording.spell(to));
    match (is_present(from), is_present(to)) {
        (Ok(false), Ok(true)) => {
            let command =
                crate::shell::join(["mv".to_string(), to_text.clone(), from_text.clone()]);
            format!(
                "{to_text}, which belongs at {from_text} (`{command}`, run {}, puts it back)",
                wording.where_commands_run()
            )
        }
        (Ok(true), Ok(true)) => format!(
            "{to_text}, which belongs at {from_text}, where something else now is, so ritual \
             moved nothing back"
        ),
        _ => format!(
            "{to_text}, which belongs at {from_text}; ritual could not tell where it is now, so \
             it moved nothing back"
        ),
    }
}

/// Whether anything is at `path`, a link to nothing included, which
/// `Path::exists` would call absent.
fn is_present(path: &Path) -> Result<bool, Failure> {
    match path.symlink_metadata() {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(Failure::new(format!("reading {} failed", path.display())).caused_by(error))
        }
    }
}

/// Writes `original` back to `path` unless the file already holds exactly
/// those bytes, so a file the run never got as far as writing is not
/// written to at all.
fn restore_file(path: &Path, original: &[u8]) -> Result<Undone, Failure> {
    match std::fs::read(path) {
        Ok(current) if current == original => return Ok(Undone::AsFound),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(Failure::new(format!("reading {} failed", path.display())).caused_by(error));
        }
    }
    std::fs::write(path, original)
        .map(|()| Undone::PutBack)
        .map_err(|error| {
            Failure::new(format!("writing {} failed", path.display())).caused_by(error)
        })
}

/// Refuses `path`, which could not be read because nothing is there, if it
/// is a symbolic link: following it finds nothing, but the link itself is
/// there.
fn refuse_a_link_to_nothing(path: &Path) -> Result<(), Failure> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(Failure::new(format!(
            "{} is a symbolic link to nothing, so ritual will not write through it",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(Failure::new(format!("reading {} failed", path.display())).caused_by(error))
        }
    }
}

/// The failure for a directory [`Changes::reserve_directory`] could not
/// create.
fn creating_failed(path: &Path, error: std::io::Error) -> Failure {
    Failure::new(format!("creating {} failed", path.display())).caused_by(error)
}

/// Removes the file the run created at `path`, if it is there.
fn remove_created_file(path: &Path) -> Result<Undone, Failure> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(Undone::PutBack),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Undone::AsFound),
        Err(error) => {
            Err(Failure::new(format!("removing {} failed", path.display())).caused_by(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rituals::{Failure, RefusalStatus};

    use super::{Changes, Wording, attempt};
    use crate::test_support::{ScratchDir, TestOutcome};

    /// The retry wording every test here hands [`attempt`], so an assertion
    /// on a whole message can name it.
    const RETRY: &str = "running `demo` again";

    /// The wording for a run that changes the project at `root`, with
    /// [`RETRY`], so a report names a path from `root`.
    fn wording(root: &Path) -> Wording<'_> {
        Wording::project(root, RETRY)
    }

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    /// Runs `steps` inside [`attempt`] and then fails, returning the
    /// failure `attempt` reports. Asserts the failure is the simulated one,
    /// so a setup step that failed inside `steps` is never mistaken for the
    /// run's own failure.
    fn fail_after(root: &Path, steps: impl FnOnce(&mut Changes) -> Result<(), Failure>) -> Failure {
        let outcome = attempt(wording(root), |changes| {
            steps(changes)?;
            Err::<(), _>(Failure::new("simulated failure"))
        });
        let Err(reported) = outcome else {
            unreachable!("a run that always ends in Err cannot succeed");
        };
        assert!(
            reported.to_string().starts_with("simulated failure"),
            "a setup step failed inside the run: {reported}"
        );
        reported
    }

    /// Undoes every recorded step, most recent first, stopping at the first
    /// one that fails: the one step a test recorded, undone on its own.
    fn undo_steps(changes: &Changes) -> Result<(), Failure> {
        for step in changes.steps.iter().rev() {
            step.undo()?;
        }
        Ok(())
    }

    /// Whether this process is held to a directory's missing write bit.
    /// Root is not, so a test that relies on a removal failing there
    /// reports a skip rather than a pass it did not earn.
    fn write_permission_is_enforced(directory: &Path) -> bool {
        let probe = directory.join("probe");
        let enforced = std::fs::File::create(&probe).is_err();
        if !enforced {
            let _ = std::fs::remove_file(&probe);
        }
        enforced
    }

    #[test]
    #[should_panic(expected = "pass the outer run's &mut Changes down instead")]
    fn an_attempt_inside_an_attempt_is_refused() {
        let _ = attempt(wording(Path::new("/work")), |_outer| {
            attempt(wording(Path::new("/work")), |_inner| Ok(()))
        });
    }

    #[test]
    fn attempts_in_sequence_on_one_thread_each_run() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-sequence")?;
        let path = scratch.path().join("Cargo.toml");

        attempt(wording(scratch.path()), |changes| {
            changes.write(&path, "first\n")
        })?;
        let _ = fail_after(scratch.path(), |changes| changes.write(&path, "second\n"));
        attempt(wording(scratch.path()), |changes| {
            changes.write(&path, "third\n")
        })?;

        assert_eq!(std::fs::read_to_string(&path)?, "third\n");
        Ok(())
    }

    #[test]
    fn an_attempt_after_one_that_panicked_still_runs() {
        // The refused nested call panics inside the outer run, so the outer
        // run unwinds without returning: the mark has to be cleared on the
        // way out regardless.
        let unwound = std::panic::catch_unwind(|| {
            let _ = attempt(wording(Path::new("/work")), |_outer| {
                attempt(wording(Path::new("/work")), |_inner| Ok(()))
            });
        });
        assert!(unwound.is_err(), "the nested attempt must have panicked");

        let outcome = attempt(wording(Path::new("/work")), |_changes| Ok(()));
        assert!(
            outcome.is_ok(),
            "expected the later run to succeed: {outcome:?}"
        );
    }

    #[test]
    fn changes_is_send_and_sync() {
        assert_send::<Changes>();
        assert_sync::<Changes>();
    }

    #[test]
    fn a_changed_file_is_put_back_byte_for_byte_whatever_it_holds() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-bytes")?;
        let path = scratch.path().join("Cargo.lock");
        // Not UTF-8, and not TOML: the restore is of bytes, not of text.
        let original: &[u8] = b"version = 4\r\n\xff\xfe\x00no trailing newline";
        std::fs::write(&path, original)?;

        let reported = fail_after(scratch.path(), |changes| {
            changes.write(&path, "version = 4\n")
        });

        assert_eq!(
            reported.to_string(),
            "simulated failure; ritual put the project back as it found it"
        );
        assert_eq!(std::fs::read(&path)?, original);
        Ok(())
    }

    #[test]
    fn a_file_the_run_created_is_removed() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-created-file")?;
        let path = scratch.path().join("Cargo.lock");

        let reported = fail_after(scratch.path(), |changes| {
            changes.write(&path, "version = 4\n")
        });

        assert!(
            reported
                .to_string()
                .ends_with("; ritual put the project back as it found it")
        );
        assert!(!path.exists(), "a file that was not there must be gone");
        Ok(())
    }

    #[test]
    fn a_file_written_twice_goes_back_to_what_it_held_before_the_first_write() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-twice")?;
        let path = scratch.path().join("main.rs");
        std::fs::write(&path, "original\n")?;

        let _ = fail_after(scratch.path(), |changes| {
            changes.write(&path, "first\n")?;
            changes.write(&path, "second\n")
        });

        assert_eq!(std::fs::read_to_string(&path)?, "original\n");
        Ok(())
    }

    #[test]
    fn a_file_written_twice_that_cannot_be_put_back_is_named_once() -> TestOutcome {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::new("rollback-twice-fails")?;
        let path = scratch.path().join("main.rs");
        std::fs::write(&path, "original\n")?;
        let mut enforced = true;

        let reported = fail_after(scratch.path(), |changes| {
            changes.write(&path, "first\n")?;
            changes.write(&path, "second\n")?;
            // Read-only, so writing the original back fails.
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444))
                .map_err(|error| Failure::new("setup").caused_by(error))?;
            enforced = std::fs::OpenOptions::new().write(true).open(&path).is_err();
            Ok(())
        });

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;

        if !enforced {
            crate::test_support::report_skip(
                "a_file_written_twice_that_cannot_be_put_back_is_named_once could not \
                 demonstrate a failed restore because this process does not honour the \
                 read-only permission bit",
            );
            return Ok(());
        }

        assert_eq!(
            reported.to_string(),
            "simulated failure; ritual could not put back main.rs — check it before running \
             `demo` again"
        );
        Ok(())
    }

    #[test]
    fn files_another_process_changed_and_created_are_put_back() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-run-changing")?;
        let manifest = scratch.path().join("Cargo.toml");
        let lockfile = scratch.path().join("Cargo.lock");
        std::fs::write(
            &manifest,
            "[dependencies]\n# a comment cargo remove would drop\n",
        )?;

        let _ = fail_after(scratch.path(), |changes| {
            changes.run_changing(&[manifest.as_path(), lockfile.as_path()], || {
                std::fs::write(&manifest, "[dependencies]\nlint = \"0.1\"\n")
                    .and_then(|()| std::fs::write(&lockfile, "version = 4\n"))
                    .map_err(|error| Failure::new("stand-in for cargo add").caused_by(error))
            })
        });

        assert_eq!(
            std::fs::read_to_string(&manifest)?,
            "[dependencies]\n# a comment cargo remove would drop\n"
        );
        assert!(
            !lockfile.exists(),
            "a lockfile the run created must be gone"
        );
        Ok(())
    }

    #[test]
    fn run_changing_does_not_run_the_change_when_a_file_cannot_be_read() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-unreadable")?;
        // A directory where a file is expected reads as an error other than
        // "not found", so there is nothing the change could be undone to.
        let unreadable = scratch.path().join("Cargo.lock");
        std::fs::create_dir(&unreadable)?;
        let mut ran = false;

        let outcome = attempt(wording(scratch.path()), |changes| {
            changes.run_changing(&[unreadable.as_path()], || {
                ran = true;
                Ok(())
            })
        });

        assert!(!ran, "the change must not run without its snapshot");
        let Err(reported) = outcome else {
            unreachable!("reading a directory as a file cannot succeed");
        };
        assert!(
            reported
                .with_causes()
                .to_string()
                .starts_with(&format!("reading {} failed", unreadable.display())),
            "expected the unreadable path to be named: {reported}"
        );
        Ok(())
    }

    #[test]
    fn writing_through_a_symbolic_link_to_nothing_is_refused() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-dangling-link")?;
        let target = scratch.path().join("real.lock");
        let link = scratch.path().join("Cargo.lock");
        std::os::unix::fs::symlink(&target, &link)?;

        let outcome = attempt(wording(scratch.path()), |changes| {
            changes.write(&link, "version = 4\n")
        });

        let Err(reported) = outcome else {
            unreachable!("writing through a link to nothing must be refused");
        };
        assert_eq!(
            reported.to_string(),
            format!(
                "{} is a symbolic link to nothing, so ritual will not write through it",
                link.display()
            )
        );
        assert!(
            std::fs::symlink_metadata(&link)?.file_type().is_symlink(),
            "the link must still be there"
        );
        assert!(
            std::fs::symlink_metadata(&target).is_err(),
            "its target must still be absent"
        );
        Ok(())
    }

    #[test]
    fn a_symbolic_link_to_a_file_is_written_through_and_put_back() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-live-link")?;
        let target = scratch.path().join("real.lock");
        let link = scratch.path().join("Cargo.lock");
        std::fs::write(&target, "version = 3\n")?;
        std::os::unix::fs::symlink(&target, &link)?;

        let reported = fail_after(scratch.path(), |changes| {
            changes.write(&link, "version = 4\n")
        });

        assert_eq!(
            reported.to_string(),
            "simulated failure; ritual put the project back as it found it"
        );
        assert!(std::fs::symlink_metadata(&link)?.file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&target)?, "version = 3\n");
        Ok(())
    }

    #[test]
    fn a_reserved_directory_goes_with_everything_in_it_and_the_parents_it_needed() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-reserved")?;
        let tasks = scratch.path().join("tasks");
        let task = tasks.join("lint");

        let reported = fail_after(scratch.path(), |changes| {
            changes.reserve_directory(&task)?;
            std::fs::create_dir_all(task.join("src"))
                .and_then(|()| std::fs::write(task.join("src/lib.rs"), "// lint\n"))
                .map_err(|error| Failure::new("setup").caused_by(error))
        });

        assert!(
            reported
                .to_string()
                .ends_with("; ritual put the project back as it found it")
        );
        assert!(!tasks.exists(), "tasks/ was created by the run and must go");
        Ok(())
    }

    #[test]
    fn reserving_a_directory_creates_it_empty_with_the_parents_it_needs() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-reserve-creates")?;
        let tasks = scratch.path().join("tasks");
        let task = tasks.join("lint");

        attempt(wording(scratch.path()), |changes| {
            changes.reserve_directory(&task)
        })?;

        assert!(task.is_dir(), "the reserved directory must exist");
        assert_eq!(std::fs::read_dir(&task)?.count(), 0, "and be empty");
        Ok(())
    }

    #[test]
    fn two_directories_reserved_under_one_new_parent_are_put_back_cleanly() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-siblings")?;
        let tasks = scratch.path().join("tasks");
        let lint = tasks.join("lint");
        let format = tasks.join("format");

        let reported = fail_after(scratch.path(), |changes| {
            changes.reserve_directory(&lint)?;
            changes.reserve_directory(&format)?;
            // As `create` fills its directory: creating what is already there
            // is not an error.
            std::fs::create_dir_all(&lint)
                .and_then(|()| std::fs::create_dir_all(&format))
                .and_then(|()| std::fs::write(lint.join("Cargo.toml"), "[package]\n"))
                .and_then(|()| std::fs::write(format.join("Cargo.toml"), "[package]\n"))
                .map_err(|error| Failure::new("setup").caused_by(error))
        });

        assert_eq!(
            reported.to_string(),
            "simulated failure; ritual put the project back as it found it"
        );
        assert!(!tasks.exists(), "tasks/ was created by the run and must go");
        Ok(())
    }

    #[test]
    fn a_directory_reserved_inside_another_is_put_back_cleanly() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-nested-reserve")?;
        let outer = scratch.path().join("a");
        let inner = outer.join("b");

        let reported = fail_after(scratch.path(), |changes| {
            changes.reserve_directory(&outer)?;
            changes.reserve_directory(&inner)?;
            std::fs::create_dir_all(&inner)
                .and_then(|()| std::fs::write(outer.join("outer.txt"), "outer\n"))
                .and_then(|()| std::fs::write(inner.join("inner.txt"), "inner\n"))
                .map_err(|error| Failure::new("setup").caused_by(error))
        });

        assert_eq!(
            reported.to_string(),
            "simulated failure; ritual put the project back as it found it"
        );
        assert!(!outer.exists(), "a was created by the run and must go");
        Ok(())
    }

    #[test]
    fn a_parent_that_was_already_there_is_left_alone() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-existing-parent")?;
        let tasks = scratch.path().join("tasks");
        std::fs::create_dir(&tasks)?;
        std::fs::write(tasks.join(".keep"), "predates the run\n")?;
        let task = tasks.join("lint");

        let _ = fail_after(scratch.path(), |changes| {
            changes.reserve_directory(&task)?;
            std::fs::create_dir_all(&task)
                .and_then(|()| std::fs::write(task.join("Cargo.toml"), "[package]\n"))
                .map_err(|error| Failure::new("setup").caused_by(error))
        });

        assert!(tasks.join(".keep").is_file(), "tasks/ predates the run");
        assert!(!task.exists(), "the reserved directory must still go");
        Ok(())
    }

    #[test]
    fn reserving_a_directory_that_already_exists_is_refused_and_leaves_it() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-reserve-existing")?;
        let task = scratch.path().join("tasks/lint");
        std::fs::create_dir_all(&task)?;

        let outcome = attempt(wording(scratch.path()), |changes| {
            changes.reserve_directory(&task)
        });

        let Err(reported) = outcome else {
            unreachable!("reserving a directory that exists must be refused");
        };
        assert_eq!(
            reported.to_string(),
            format!(
                "{} already exists, so ritual will not create it",
                task.display()
            )
        );
        assert!(task.exists(), "a directory the run did not create stays");
        Ok(())
    }

    /// `top_level`'s refusals end in a full stop, and the rollback's report
    /// continues the sentence after a semicolon, so a stop left in place
    /// reads `.;`. A file is written first, so there is something to put
    /// back and a report to continue the sentence with.
    #[test]
    fn a_failure_ending_in_a_full_stop_is_continued_without_one() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-full-stop")?;
        let path = scratch.path().join("Cargo.lock");

        let outcome = attempt(wording(scratch.path()), |changes| {
            changes.write(&path, "version = 4\n")?;
            Err::<(), _>(Failure::new("`add` would be a top-level command twice."))
        });

        let reported = outcome.err().ok_or("expected the run to fail")?;
        assert_eq!(
            reported.to_string(),
            "`add` would be a top-level command twice; ritual put the project back as it \
             found it"
        );
        Ok(())
    }

    /// A failure with a cause is continued after the cause, so the report
    /// keeps everything the refusal line would have named.
    #[test]
    fn a_failure_with_a_cause_is_continued_after_its_cause() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-caused-continued")?;
        let path = scratch.path().join("Cargo.lock");

        let outcome = attempt(wording(scratch.path()), |changes| {
            changes.write(&path, "version = 4\n")?;
            Err::<(), _>(
                Failure::new("writing Cargo.toml failed")
                    .caused_by(std::io::Error::other("disk full")),
            )
        });

        let reported = outcome.err().ok_or("expected the run to fail")?;
        assert_eq!(
            reported.with_causes().to_string(),
            "writing Cargo.toml failed: disk full; ritual put the project back as it found it"
        );
        Ok(())
    }

    /// A rolled-back failure exits with the status the run refused with,
    /// though its report is a new sentence.
    #[test]
    fn a_continued_failure_keeps_the_status_it_was_raised_with() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-continued-status")?;
        let path = scratch.path().join("Cargo.lock");
        let status = RefusalStatus::new(3)?;

        let outcome = attempt(wording(scratch.path()), |changes| {
            changes.write(&path, "version = 4\n")?;
            Err::<(), _>(Failure::new("3 tasks differ").exiting_with(status))
        });

        let reported = outcome.err().ok_or("expected the run to fail")?;
        assert_eq!(
            reported.to_string(),
            "3 tasks differ; ritual put the project back as it found it"
        );
        assert_eq!(reported.status(), status);
        Ok(())
    }

    /// The same join when something could not be put back, where the report
    /// continues with what could not be instead.
    #[test]
    fn a_failure_ending_in_a_full_stop_is_continued_without_one_when_a_restore_fails() -> TestOutcome
    {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::new("rollback-full-stop-restore-fails")?;
        let locked = scratch.path().join("locked");
        std::fs::create_dir(&locked)?;
        let created = locked.join("Cargo.lock");
        let mut enforced = true;

        let outcome = attempt(wording(scratch.path()), |changes| {
            changes.write(&created, "version = 4\n")?;
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555))
                .map_err(|error| Failure::new("setup").caused_by(error))?;
            enforced = write_permission_is_enforced(&locked);
            Err::<(), _>(Failure::new("refused."))
        });

        // Before any assertion can return early, so the scratch directory
        // can still be removed on drop.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))?;

        if !enforced {
            crate::test_support::report_skip(
                "a_failure_ending_in_a_full_stop_is_continued_without_one_when_a_restore_fails \
                 could not demonstrate a failed removal because this process does not honour \
                 directory write permissions",
            );
            return Ok(());
        }
        let reported = outcome.err().ok_or("expected the run to fail")?.to_string();
        assert!(
            reported.starts_with("refused; ritual could not put back "),
            "expected the full stop trimmed before the continuation; got: {reported}"
        );
        Ok(())
    }

    #[test]
    fn a_file_recorded_as_absent_is_reported_absent_and_one_that_was_there_is_not() -> TestOutcome {
        // Verifies `recorded_as_absent` answers what the run found when it
        // recorded each path: a missing file is absent, a present one is
        // not, and a path the run never recorded says nothing either way.
        let scratch = ScratchDir::new("rollback-recorded-as-absent")?;
        let missing = scratch.path().join("Cargo.lock");
        let present = scratch.path().join("Cargo.toml");
        let unrecorded = scratch.path().join("notes.txt");
        std::fs::write(&present, "[workspace]\n")?;

        let mut changes = Changes::new();
        changes.record_file(&missing)?;
        changes.record_file(&present)?;

        assert!(changes.recorded_as_absent(&missing));
        assert!(!changes.recorded_as_absent(&present));
        assert!(!changes.recorded_as_absent(&unrecorded));
        Ok(())
    }

    /// The first record is the one that holds what the project had, so a
    /// file the run wrote after recording it as absent is still absent.
    #[test]
    fn a_file_the_run_has_since_written_is_still_recorded_as_absent() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-absent-then-written")?;
        let path = scratch.path().join("Cargo.lock");

        let mut changes = Changes::new();
        changes.write(&path, "version = 4\n")?;

        assert!(changes.recorded_as_absent(&path));
        Ok(())
    }

    #[test]
    fn a_run_that_succeeds_keeps_its_changes() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-success")?;
        let path = scratch.path().join("Cargo.toml");
        let task = scratch.path().join("tasks/lint");

        let outcome = attempt(wording(scratch.path()), |changes| {
            changes.write(&path, "[workspace]\n")?;
            changes.reserve_directory(&task)?;
            std::fs::create_dir_all(&task).map_err(|error| Failure::new("setup").caused_by(error))
        });

        assert!(outcome.is_ok(), "expected the run to succeed: {outcome:?}");
        assert_eq!(std::fs::read_to_string(&path)?, "[workspace]\n");
        assert!(task.is_dir());
        Ok(())
    }

    #[test]
    fn a_restore_that_fails_is_named_with_the_callers_retry() -> TestOutcome {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::new("rollback-restore-fails")?;
        let locked = scratch.path().join("locked");
        std::fs::create_dir(&locked)?;
        let created = locked.join("Cargo.lock");
        let mut enforced = true;

        let reported = fail_after(scratch.path(), |changes| {
            changes.write(&created, "version = 4\n")?;
            // Without write permission on its directory, the file cannot be
            // removed, so the undo has to report it.
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555))
                .map_err(|error| Failure::new("setup").caused_by(error))?;
            enforced = write_permission_is_enforced(&locked);
            Ok(())
        });

        // Before any assertion can return early, so the scratch directory
        // can still be removed on drop.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))?;

        if !enforced {
            crate::test_support::report_skip(
                "a_restore_that_fails_is_named_with_the_callers_retry could not demonstrate a \
                 failed removal because this process does not honour directory write \
                 permissions",
            );
            return Ok(());
        }

        assert_eq!(
            reported.to_string(),
            "simulated failure; ritual could not put back locked/Cargo.lock — check it before \
             running `demo` again"
        );
        assert!(
            created.exists(),
            "the file the undo could not remove is still there"
        );
        Ok(())
    }

    /// A reserved directory the undo cannot remove is the only thing named,
    /// from the project root: the parent that stays because it holds it is
    /// not named again, and with nothing put back the report does not say
    /// the project was.
    #[test]
    fn a_reserved_directory_that_cannot_be_removed_is_named_alone_and_nothing_claims_a_recovery()
    -> TestOutcome {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::new("rollback-restores-fail")?;
        let tasks = scratch.path().join("tasks");
        let task = tasks.join("lint");
        let mut enforced = true;

        let reported = fail_after(scratch.path(), |changes| {
            changes.reserve_directory(&task)?;
            std::fs::create_dir_all(task.join("src"))
                .and_then(|()| {
                    std::fs::set_permissions(&task, std::fs::Permissions::from_mode(0o555))
                })
                .map_err(|error| Failure::new("setup").caused_by(error))?;
            enforced = write_permission_is_enforced(&task);
            Ok(())
        });

        // The directory is still there only when the undo could not remove
        // it. A process that ignores the missing write bit (root) removed
        // it, and has nothing to restore permissions on.
        if !enforced {
            crate::test_support::report_skip(
                "a_reserved_directory_that_cannot_be_removed_is_named_alone_and_nothing_claims_a_\
                 recovery could not demonstrate a failed removal because this process does not \
                 honour directory write permissions",
            );
            return Ok(());
        }

        // Before any assertion can return early, so the scratch directory
        // can still be removed on drop.
        std::fs::set_permissions(&task, std::fs::Permissions::from_mode(0o755))?;

        // `tasks/lint` cannot lose `src`, so it stays, and `tasks/` is not
        // empty, so it stays too, and is not deleted out from under it. Only
        // `tasks/lint` is named, from the project root: `tasks/` stays only
        // because it holds it. Nothing was put back, so nothing says the
        // project was.
        assert_eq!(
            reported.to_string(),
            "simulated failure; ritual could not put back tasks/lint — check it before running \
             `demo` again"
        );
        assert!(task.join("src").is_dir());
        assert!(tasks.is_dir());
        Ok(())
    }

    /// When some changes were put back and one could not be, the report says
    /// the project was put back except for that one, named from the project
    /// root.
    #[test]
    fn a_run_that_put_some_back_names_what_it_could_not_after_except_for() -> TestOutcome {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::new("rollback-some-put-back")?;
        let manifest = scratch.path().join("Cargo.toml");
        std::fs::write(&manifest, "[workspace]\n")?;
        let locked = scratch.path().join("locked");
        std::fs::create_dir(&locked)?;
        let created = locked.join("Cargo.lock");
        let mut enforced = true;

        let reported = fail_after(scratch.path(), |changes| {
            changes.write(&manifest, "[workspace]\nmembers = []\n")?;
            changes.write(&created, "version = 4\n")?;
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555))
                .map_err(|error| Failure::new("setup").caused_by(error))?;
            enforced = write_permission_is_enforced(&locked);
            Ok(())
        });

        // Before any assertion can return early, so the scratch directory
        // can still be removed on drop.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755))?;

        if !enforced {
            crate::test_support::report_skip(
                "a_run_that_put_some_back_names_what_it_could_not_after_except_for could not \
                 demonstrate a failed removal because this process does not honour directory \
                 write permissions",
            );
            return Ok(());
        }

        assert_eq!(
            reported.to_string(),
            "simulated failure; ritual put the project back except for locked/Cargo.lock — check \
             it before running `demo` again"
        );
        assert_eq!(std::fs::read_to_string(&manifest)?, "[workspace]\n");
        Ok(())
    }

    #[test]
    fn restoring_a_recorded_file_puts_back_the_original_bytes() -> TestOutcome {
        let scratch = ScratchDir::new("restore-changed")?;
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(&path, "[workspace]\nmembers = [\"ritual\"]\n")?;

        let mut changes = Changes::new();
        changes.record_file(&path)?;
        std::fs::write(
            &path,
            "[workspace]\nmembers = [\"ritual\", \"tasks/lint\"]\n",
        )?;

        let restored = undo_steps(&changes);
        assert!(
            restored.is_ok(),
            "expected the restore to succeed: {restored:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&path)?,
            "[workspace]\nmembers = [\"ritual\"]\n"
        );
        Ok(())
    }

    #[test]
    fn restoring_an_unchanged_file_does_not_write_to_it() -> TestOutcome {
        let scratch = ScratchDir::new("restore-unchanged")?;
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(&path, "[workspace]\nmembers = [\"ritual\"]\n")?;

        let mut changes = Changes::new();
        changes.record_file(&path)?;

        // Read-only: if the restore tried to write despite the file already
        // matching, this would turn that attempt into a failure instead of
        // silently succeeding either way.
        let mut permissions = std::fs::metadata(&path)?.permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions)?;

        // A process that ignores the read-only bit (root, on Unix) can open
        // the file for writing anyway, which would make the assertion below
        // pass whether or not the restore actually attempted a write. This
        // probe opens for write without writing anything, so it tells the
        // two cases apart without disturbing the file's contents.
        let permission_is_enforced = std::fs::OpenOptions::new().write(true).open(&path).is_err();

        let restored = if permission_is_enforced {
            undo_steps(&changes)
        } else {
            Ok(())
        };

        // Cleanup, on a scratch file this test alone created and is about to
        // delete — not a security boundary `set_readonly(false)`'s
        // world-writable warning is guarding here.
        let mut permissions = std::fs::metadata(&path)?.permissions();
        #[expect(
            clippy::permissions_set_readonly_false,
            reason = "restoring a scratch file's own permissions before removing it, not \
                      granting access to anything"
        )]
        {
            permissions.set_readonly(false);
        }
        std::fs::set_permissions(&path, permissions)?;

        if !permission_is_enforced {
            crate::test_support::report_skip(
                "restoring_an_unchanged_file_does_not_write_to_it could not demonstrate a \
                     blocked write because this process does not honour the read-only \
                     permission bit",
            );
            return Ok(());
        }

        assert!(
            restored.is_ok(),
            "expected no write attempt against an unchanged, read-only file: {restored:?}"
        );
        Ok(())
    }

    /// A directory a run moved goes back to where it was when the run
    /// fails, with everything inside it: a nested file stands in for an
    /// ignored `target/`, which git could never give back.
    #[test]
    fn a_failed_run_moves_a_renamed_directory_back_with_everything_in_it() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-rename-back")?;
        let from = scratch.path().join("tasks/lint");
        let to = scratch.path().join(".rituals/lint");
        std::fs::create_dir_all(from.join("target/debug"))?;
        std::fs::write(from.join("target/debug/lint.d"), "built\n")?;
        std::fs::write(from.join("Cargo.toml"), "[package]\n")?;

        let reported = fail_after(scratch.path(), |changes| {
            changes.rename(&from, &to)?;
            assert!(
                to.join("target/debug/lint.d").is_file(),
                "the directory must have moved, whole, before the undo"
            );
            Ok(())
        });

        assert_eq!(
            reported.to_string(),
            "simulated failure; ritual put the project back as it found it"
        );
        assert_eq!(
            std::fs::read_to_string(from.join("target/debug/lint.d"))?,
            "built\n"
        );
        assert_eq!(
            std::fs::read_to_string(from.join("Cargo.toml"))?,
            "[package]\n"
        );
        assert!(!to.exists(), "the destination must be empty again");
        Ok(())
    }

    /// A rename into a directory that does not exist creates it, and the
    /// undo removes what the run created while leaving a parent that was
    /// already there alone.
    #[test]
    fn the_parents_a_rename_created_go_and_one_that_was_already_there_stays() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-rename-parents")?;
        let from = scratch.path().join("tasks/lint");
        std::fs::create_dir_all(&from)?;
        let existing = scratch.path().join("existing");
        std::fs::create_dir(&existing)?;
        let to = existing.join("new/deeper/lint");

        let _ = fail_after(scratch.path(), |changes| {
            changes.rename(&from, &to)?;
            assert!(to.is_dir(), "the move must have happened before the undo");
            Ok(())
        });

        assert!(from.is_dir(), "the directory must be back");
        assert!(existing.is_dir(), "a parent that predates the run stays");
        assert!(
            !existing.join("new").exists(),
            "the parents the run created must go"
        );
        Ok(())
    }

    #[test]
    fn a_rename_onto_a_directory_a_file_or_a_dangling_link_is_refused_and_moves_nothing()
    -> TestOutcome {
        let scratch = ScratchDir::new("rollback-rename-occupied")?;
        let from = scratch.path().join("tasks/lint");
        std::fs::create_dir_all(&from)?;
        std::fs::write(from.join("Cargo.toml"), "[package]\n")?;
        let occupied_by_directory = scratch.path().join("a-directory");
        std::fs::create_dir(&occupied_by_directory)?;
        let occupied_by_file = scratch.path().join("a-file");
        std::fs::write(&occupied_by_file, "in the way\n")?;
        let occupied_by_link = scratch.path().join("a-link");
        std::os::unix::fs::symlink(scratch.path().join("nothing"), &occupied_by_link)?;

        for to in [occupied_by_directory, occupied_by_file, occupied_by_link] {
            let outcome = attempt(wording(scratch.path()), |changes| {
                changes.rename(&from, &to)
            });

            let Err(reported) = outcome else {
                unreachable!("a rename onto {} must be refused", to.display());
            };
            assert_eq!(
                reported.to_string(),
                format!(
                    "{} already exists, so ritual will not move {} onto it",
                    to.display(),
                    from.display()
                )
            );
            assert_eq!(
                std::fs::read_to_string(from.join("Cargo.toml"))?,
                "[package]\n",
                "{} must stay where it was",
                from.display()
            );
        }
        Ok(())
    }

    #[test]
    fn a_rename_of_something_that_is_not_there_is_refused_and_creates_nothing() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-rename-missing")?;
        let from = scratch.path().join("tasks/lint");
        let to = scratch.path().join(".rituals/lint");

        let outcome = attempt(wording(scratch.path()), |changes| {
            changes.rename(&from, &to)
        });

        let Err(reported) = outcome else {
            unreachable!("a rename of a missing path must be refused");
        };
        assert_eq!(
            reported.to_string(),
            format!(
                "{} does not exist, so ritual has nothing to move",
                from.display()
            )
        );
        assert!(
            !scratch.path().join(".rituals").exists(),
            "a refused rename must not leave the parents it would have needed"
        );
        Ok(())
    }

    /// A manifest the run edited before the directory holding it moved is
    /// put back at the path it was written at, with its original bytes,
    /// because the directory goes back first.
    #[test]
    fn a_file_written_and_then_carried_by_a_rename_gets_its_original_bytes_back() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-rename-carries-a-write")?;
        let from = scratch.path().join("tasks/lint");
        let to = scratch.path().join(".rituals/lint");
        std::fs::create_dir_all(&from)?;
        let manifest = from.join("Cargo.toml");
        std::fs::write(&manifest, "[dependencies]\n# a comment\n")?;

        let _ = fail_after(scratch.path(), |changes| {
            changes.write(&manifest, "[dependencies]\n")?;
            changes.rename(&from, &to)?;
            assert!(
                std::fs::read_to_string(to.join("Cargo.toml"))
                    .is_ok_and(|carried| carried == "[dependencies]\n"),
                "the edited manifest must have been carried by the move"
            );
            Ok(())
        });

        assert_eq!(
            std::fs::read_to_string(&manifest)?,
            "[dependencies]\n# a comment\n"
        );
        assert!(!to.exists());
        Ok(())
    }

    #[test]
    fn a_run_that_succeeds_keeps_a_rename() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-rename-success")?;
        let from = scratch.path().join("tasks/lint");
        let to = scratch.path().join(".rituals/lint");
        std::fs::create_dir_all(&from)?;
        std::fs::write(from.join("Cargo.toml"), "[package]\n")?;

        attempt(wording(scratch.path()), |changes| {
            changes.rename(&from, &to)
        })?;

        assert_eq!(
            std::fs::read_to_string(to.join("Cargo.toml"))?,
            "[package]\n"
        );
        assert!(!from.exists());
        Ok(())
    }

    /// When the directory cannot be moved back because the directory it
    /// belongs in is read-only, the report names both paths from the project
    /// root and a command that puts it back from there, and the directory
    /// stays where the run put it.
    #[test]
    fn a_rename_that_cannot_be_undone_names_both_paths_and_the_command_that_puts_it_back()
    -> TestOutcome {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::new("rollback-rename-undo-fails")?;
        let tasks = scratch.path().join("tasks");
        let from = tasks.join("lint");
        // Already there, so no parent of the destination is the run's own to
        // remove and the report names the move alone.
        let to_parent = scratch.path().join(".rituals");
        let to = to_parent.join("lint");
        std::fs::create_dir_all(&from)?;
        std::fs::create_dir(&to_parent)?;
        let mut enforced = true;

        let reported = fail_after(scratch.path(), |changes| {
            changes.rename(&from, &to)?;
            // Without write permission on `tasks`, nothing can be moved
            // back into it.
            std::fs::set_permissions(&tasks, std::fs::Permissions::from_mode(0o555))
                .map_err(|error| Failure::new("setup").caused_by(error))?;
            enforced = write_permission_is_enforced(&tasks);
            Ok(())
        });

        // Before any assertion can return early, so the scratch directory
        // can still be removed on drop.
        std::fs::set_permissions(&tasks, std::fs::Permissions::from_mode(0o755))?;

        if !enforced {
            crate::test_support::report_skip(
                "a_rename_that_cannot_be_undone_names_both_paths_and_the_command_that_puts_it_back \
                 could not demonstrate a failed move back because this process does not honour \
                 directory write permissions",
            );
            return Ok(());
        }

        assert_eq!(
            reported.to_string(),
            "simulated failure; ritual could not put back .rituals/lint, which belongs at \
             tasks/lint (`mv .rituals/lint tasks/lint`, run from the project root, puts it back) \
             — check it before running `demo` again"
        );
        assert!(to.is_dir(), "the directory stays where the run put it");
        Ok(())
    }

    /// Moving `to` back onto a `from` that exists again would put it inside
    /// `from`, so nothing is moved and no command is offered.
    #[test]
    fn an_undo_that_finds_both_paths_present_moves_nothing_and_offers_no_command() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-rename-both-present")?;
        let from = scratch.path().join("tasks/lint");
        let to = scratch.path().join(".rituals/lint");
        std::fs::create_dir_all(&from)?;
        std::fs::write(from.join("Cargo.toml"), "[package]\n")?;

        let reported = fail_after(scratch.path(), |changes| {
            changes.rename(&from, &to)?;
            std::fs::create_dir(&from).map_err(|error| Failure::new("setup").caused_by(error))
        });

        let reported = reported.to_string();
        assert!(
            reported.contains(
                ".rituals/lint, which belongs at tasks/lint, where something else now is"
            ),
            "expected both paths named: {reported}"
        );
        assert!(
            reported.contains("ritual moved nothing back"),
            "expected the report to say nothing was moved back: {reported}"
        );
        assert!(
            !reported.contains("`mv "),
            "a command that would nest one directory in the other must not be offered: \
             {reported}"
        );
        assert!(to.join("Cargo.toml").is_file(), "the moved directory stays");
        assert_eq!(
            std::fs::read_dir(&from)?.count(),
            0,
            "and so does the new one"
        );
        Ok(())
    }

    /// With neither path there, there is nothing to move and nothing to
    /// base a command on, so the report names both and offers none.
    #[test]
    fn an_undo_that_finds_neither_path_present_names_both_and_offers_no_command() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-rename-neither-present")?;
        let from = scratch.path().join("tasks/lint");
        let to = scratch.path().join(".rituals/lint");
        std::fs::create_dir_all(&from)?;

        let reported = fail_after(scratch.path(), |changes| {
            changes.rename(&from, &to)?;
            std::fs::remove_dir(&to).map_err(|error| Failure::new("setup").caused_by(error))
        });

        let reported = reported.to_string();
        assert!(
            reported.contains(
                ".rituals/lint, which belongs at tasks/lint; ritual could not tell where it is \
                 now, so it moved nothing back"
            ),
            "expected both paths named: {reported}"
        );
        assert!(
            !reported.contains("`mv "),
            "no command is offered: {reported}"
        );
        Ok(())
    }

    #[test]
    #[should_panic(expected = "a rename moves from an absolute path")]
    fn a_rename_between_relative_paths_is_a_bug() {
        let _ = attempt(wording(Path::new("/work")), |changes| {
            changes.rename(Path::new("tasks/lint"), Path::new(".rituals/lint"))
        });
    }

    #[test]
    #[should_panic(expected = "a rename moves to an absolute path")]
    fn a_rename_to_a_relative_path_is_a_bug() {
        let _ = attempt(wording(Path::new("/work")), |changes| {
            changes.rename(Path::new("/project/tasks/lint"), Path::new(".rituals/lint"))
        });
    }

    #[test]
    #[should_panic(expected = "cannot be moved into itself")]
    fn a_rename_into_the_directory_being_moved_is_a_bug() {
        let _ = attempt(wording(Path::new("/work")), |changes| {
            changes.rename(
                Path::new("/project/tasks"),
                Path::new("/project/tasks/lint"),
            )
        });
    }

    /// A run that empties directories by moving out of them and removes
    /// them, deepest first, then fails, has every one back, with the
    /// permissions it had, and the moved directory back inside them.
    #[test]
    fn directories_a_run_emptied_and_removed_are_back_before_the_move_is_undone() -> TestOutcome {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::new("rollback-remove-empty")?;
        let tasks = scratch.path().join("tasks");
        let group = tasks.join("group");
        let from = group.join("lint");
        let to = scratch.path().join(".rituals/group/lint");
        std::fs::create_dir_all(&from)?;
        std::fs::write(from.join("Cargo.toml"), "[package]\n")?;
        std::fs::set_permissions(&group, std::fs::Permissions::from_mode(0o750))?;

        let reported = fail_after(scratch.path(), |changes| {
            changes.rename(&from, &to)?;
            for emptied in [&group, &tasks] {
                changes.remove_empty_directory(emptied).map_err(|error| {
                    Failure::new("removing an emptied directory failed").caused_by(error)
                })?;
            }
            assert!(!tasks.exists(), "the run removed tasks/ before failing");
            Ok(())
        });

        assert_eq!(
            reported.to_string(),
            "simulated failure; ritual put the project back as it found it"
        );
        assert_eq!(
            std::fs::read_to_string(from.join("Cargo.toml"))?,
            "[package]\n"
        );
        assert_eq!(
            std::fs::metadata(&group)?.permissions().mode() & 0o777,
            0o750
        );
        assert!(!scratch.path().join(".rituals").exists());
        Ok(())
    }

    /// A directory that still holds something is not removed, and nothing
    /// is recorded for it, so the file system's own answer reaches the
    /// caller and the undo has nothing to create.
    #[test]
    fn a_directory_that_is_not_empty_is_left_and_nothing_is_recorded() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-remove-not-empty")?;
        let tasks = scratch.path().join("tasks");
        std::fs::create_dir_all(&tasks)?;
        std::fs::write(tasks.join("notes.md"), "kept\n")?;

        let kind = attempt(wording(scratch.path()), |changes| {
            let kind = changes
                .remove_empty_directory(&tasks)
                .err()
                .map(|error| error.kind());
            assert!(changes.steps.is_empty(), "nothing was removed to record");
            Ok(kind)
        })?;

        assert_eq!(kind, Some(std::io::ErrorKind::DirectoryNotEmpty));
        assert_eq!(std::fs::read_to_string(tasks.join("notes.md"))?, "kept\n");
        Ok(())
    }

    /// A directory that is there again when the undo reaches it is left as
    /// it is: the undo creates only what is missing.
    #[test]
    fn a_removed_directory_that_is_back_already_is_left_alone() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-remove-back-already")?;
        let tasks = scratch.path().join("tasks");
        std::fs::create_dir_all(&tasks)?;

        let reported = fail_after(scratch.path(), |changes| {
            changes
                .remove_empty_directory(&tasks)
                .map_err(|error| Failure::new("removing tasks/ failed").caused_by(error))?;
            std::fs::create_dir(&tasks)
                .map_err(|error| Failure::new("creating tasks/ failed").caused_by(error))?;
            std::fs::write(tasks.join("new.md"), "written after\n")
                .map_err(|error| Failure::new("writing failed").caused_by(error))
        });

        // The removed directory was there again, so the undo touched
        // nothing and has nothing to report.
        assert_eq!(reported.to_string(), "simulated failure");
        assert_eq!(
            std::fs::read_to_string(tasks.join("new.md"))?,
            "written after\n"
        );
        Ok(())
    }

    /// A refusal raised before the run recorded anything says nothing about
    /// putting the project back, and keeps its full stop, because no clause
    /// continues its sentence.
    #[test]
    fn a_failure_with_nothing_recorded_is_returned_exactly_as_it_was_raised() {
        let outcome = attempt(wording(Path::new("/work")), |_changes| {
            Err::<(), _>(Failure::new("`add` would be a top-level command twice."))
        });

        let Err(reported) = outcome else {
            unreachable!("a run that always ends in Err cannot succeed");
        };
        assert_eq!(
            reported.to_string(),
            "`add` would be a top-level command twice."
        );
    }

    /// A file the run recorded and never changed is already as found, so the
    /// undo puts nothing back and the refusal stands alone: the case of a
    /// refusal that comes after a lockfile was recorded.
    #[test]
    fn a_failure_after_a_recorded_file_that_never_changed_is_returned_exactly_as_it_was_raised()
    -> TestOutcome {
        let scratch = ScratchDir::new("rollback-recorded-unchanged")?;
        let lockfile = scratch.path().join("Cargo.lock");
        std::fs::write(&lockfile, "version = 4\n")?;

        let outcome = attempt(wording(scratch.path()), |changes| {
            changes.run_changing(&[lockfile.as_path()], || Ok(()))?;
            Err::<(), _>(Failure::new("the work tree has changes."))
        });

        let reported = outcome.err().ok_or("expected the run to fail")?;
        assert_eq!(reported.to_string(), "the work tree has changes.");
        assert_eq!(std::fs::read_to_string(&lockfile)?, "version = 4\n");
        Ok(())
    }

    /// A file the run recorded as absent and never created is as found too.
    #[test]
    fn a_failure_after_a_recorded_absent_file_that_was_never_created_is_returned_as_raised()
    -> TestOutcome {
        let scratch = ScratchDir::new("rollback-recorded-absent-unchanged")?;
        let lockfile = scratch.path().join("Cargo.lock");

        let outcome = attempt(wording(scratch.path()), |changes| {
            changes.run_changing(&[lockfile.as_path()], || Ok(()))?;
            Err::<(), _>(Failure::new("refused"))
        });

        let reported = outcome.err().ok_or("expected the run to fail")?;
        assert_eq!(reported.to_string(), "refused");
        Ok(())
    }

    /// A run that makes a directory from nothing and fails says the
    /// directory was removed, spelled as the wording was given it.
    #[test]
    fn a_fresh_directory_that_is_removed_is_named_as_it_was_typed() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-fresh-removed")?;
        let made = scratch.path().join("demo");
        let wording = Wording::fresh_directory(
            scratch.path(),
            Path::new("demo"),
            "running `new demo` again",
        );

        let outcome = attempt(wording, |changes| {
            changes.reserve_directory(&made)?;
            std::fs::write(made.join("Cargo.toml"), "[workspace]\n")
                .map_err(|error| Failure::new("writing demo/Cargo.toml failed").caused_by(error))?;
            Err::<(), _>(Failure::new("writing demo/ritual/Cargo.toml failed"))
        });

        let reported = outcome.err().ok_or("expected the run to fail")?;
        assert_eq!(
            reported.to_string(),
            "writing demo/ritual/Cargo.toml failed; ritual removed demo so a retry starts clean"
        );
        assert!(!made.exists(), "the reserved directory must be gone");
        Ok(())
    }

    /// A directory the undo cannot remove is named as it was typed, the way
    /// the run's success line names it, and the retry is the caller's.
    #[test]
    fn a_fresh_directory_that_cannot_be_removed_is_named_with_the_callers_retry() -> TestOutcome {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::new("rollback-fresh-not-removed")?;
        let made = scratch.path().join("demo");
        let wording = Wording::fresh_directory(
            scratch.path(),
            Path::new("demo"),
            "running `new demo` again",
        );
        let mut enforced = true;

        let outcome = attempt(wording, |changes| {
            changes.reserve_directory(&made)?;
            std::fs::create_dir(made.join("src"))
                .and_then(|()| {
                    std::fs::set_permissions(&made, std::fs::Permissions::from_mode(0o555))
                })
                .map_err(|error| Failure::new("setup").caused_by(error))?;
            enforced = write_permission_is_enforced(&made);
            Err::<(), _>(Failure::new("writing demo/Cargo.toml failed"))
        });

        // A process that ignores the write bit, such as root, removed `made`
        // in the undo, so there is nothing to restore and nothing to show.
        if !enforced {
            crate::test_support::report_skip(
                "a_fresh_directory_that_cannot_be_removed_is_named_with_the_callers_retry could \
                 not demonstrate a failed removal because this process does not honour \
                 directory write permissions",
            );
            return Ok(());
        }

        // Before any assertion can return early, so the scratch directory
        // can still be removed on drop.
        std::fs::set_permissions(&made, std::fs::Permissions::from_mode(0o755))?;

        let reported = outcome.err().ok_or("expected the run to fail")?;
        assert_eq!(
            reported.to_string(),
            "writing demo/Cargo.toml failed; ritual could not remove demo — check it before \
             running `new demo` again"
        );
        Ok(())
    }

    /// Directories created only to hold the reserved one come before it in
    /// the record, and are not what the wording names.
    #[test]
    fn a_fresh_directory_under_parents_the_run_made_is_still_the_one_named() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-fresh-parents")?;
        let current_dir = scratch.path().join("new/parents");
        let made = current_dir.join("lint");
        let wording = Wording::fresh_directory(
            &current_dir,
            Path::new("lint"),
            "running `create lint` again",
        );

        let outcome = attempt(wording, |changes| {
            changes.reserve_directory(&made)?;
            Err::<(), _>(Failure::new("writing failed"))
        });

        let reported = outcome.err().ok_or("expected the run to fail")?;
        assert_eq!(
            reported.to_string(),
            "writing failed; ritual removed lint so a retry starts clean"
        );
        assert!(!scratch.path().join("new").exists());
        Ok(())
    }

    /// A wording that names one directory while the run reserves another
    /// would report the wrong one as removed, so the undo refuses to.
    #[test]
    #[should_panic(
        expected = "must reserve that directory before recording anything else; it recorded Directory"
    )]
    fn a_wording_naming_a_directory_the_run_did_not_reserve_is_a_bug() {
        let scratch = ScratchDir::new("rollback-fresh-mismatch")
            .expect("a scratch directory can be made under the temp root");
        let wording =
            Wording::fresh_directory(scratch.path(), Path::new("a"), "running `new a` again");

        let _ = attempt(wording, |changes| {
            changes.reserve_directory(&scratch.path().join("b"))?;
            Err::<(), _>(Failure::new("writing failed"))
        });
    }

    /// A wording that names a directory is for a run that reserves it first:
    /// a file recorded before the reservation is a bug in the caller too.
    #[test]
    #[should_panic(expected = "must reserve that directory before recording anything else")]
    fn a_fresh_directory_run_that_records_a_file_first_is_a_bug() {
        let scratch = ScratchDir::new("rollback-fresh-file-first")
            .expect("a scratch directory can be made under the temp root");
        let wording =
            Wording::fresh_directory(scratch.path(), Path::new("a"), "running `new a` again");

        let _ = attempt(wording, |changes| {
            changes.write(&scratch.path().join("Cargo.lock"), "version = 4\n")?;
            changes.reserve_directory(&scratch.path().join("a"))?;
            Err::<(), _>(Failure::new("writing failed"))
        });
    }
}
