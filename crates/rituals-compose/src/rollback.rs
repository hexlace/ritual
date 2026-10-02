//! Putting a project back exactly as a run found it, when the run does not
//! finish.
//!
//! A management task that writes to a project promises that a run which
//! fails partway leaves the project as it was, and says so when it cannot.
//! [`attempt`] keeps that promise for every task that writes: the run gets a
//! [`Changes`], records each change through it before making it, and if the
//! run returns a failure, every recorded change is undone, the most recent
//! first, and the failure says what could not be put back.
//!
//! The only way to hold a [`Changes`] is inside [`attempt`], so a run cannot
//! forget to undo, and nothing can be recorded once the undo has started.
//! Each way of recording takes its snapshot before it lets the change
//! happen: [`Changes::write`] writes the file itself, [`Changes::run_changing`]
//! runs the change it guards, and [`Changes::reserve_directory`] refuses a
//! directory that already exists. None of them can be handed a file after it
//! has been changed and take the changed bytes for the original.
//!
//! # Examples
//!
//! A run that writes two files and then fails puts both back: the one that
//! existed gets its bytes back, and the one it created is gone.
//!
//! ```
//! use rituals::Failure;
//! use rituals_compose::rollback;
//!
//! # let directory = std::env::temp_dir()
//! #     .join(format!("rituals-compose-doctest-rollback-module-{}", std::process::id()));
//! # std::fs::create_dir_all(&directory)?;
//! let lockfile = directory.join("Cargo.lock");
//! let created = directory.join("notes.txt");
//! std::fs::write(&lockfile, "version = 4\n")?;
//!
//! let outcome = rollback::attempt("running `import lint` again", |changes| {
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

use std::path::{Path, PathBuf};

use rituals::Failure;

/// Runs `run`, and if it fails, undoes every change it recorded in the
/// [`Changes`] it was handed, most recent first.
///
/// On success the changes are kept and `run`'s value is returned. On
/// failure the failure is returned with what happened to the project added
/// to it: `"<failure>; ritual put the project back as it found it"` when
/// every change was undone, or, when something could not be, every path
/// that was not put back, followed by `"— check it before <retry>"`.
/// `retry` is the caller's own, because only the caller knows what a person
/// types to try again: `add` passes ``"running `add lint` again"``.
///
/// # Examples
///
/// A run that creates a task's directory and fails before finishing leaves
/// no directory behind:
///
/// ```
/// use rituals::Failure;
/// use rituals_compose::rollback;
///
/// # let directory = std::env::temp_dir()
/// #     .join(format!("rituals-compose-doctest-rollback-attempt-{}", std::process::id()));
/// # std::fs::create_dir_all(&directory)?;
/// let task_directory = directory.join("tasks/lint");
///
/// let outcome = rollback::attempt("running `add lint` again", |changes| {
///     changes.reserve_directory(&task_directory)?;
///     std::fs::create_dir_all(task_directory.join("src")).map_err(|error| {
///         Failure::new("creating the task's directory failed").caused_by(error)
///     })?;
///     Err::<(), _>(Failure::new("writing the workspace manifest failed"))
/// });
///
/// assert!(outcome.is_err());
/// assert!(!directory.join("tasks").exists());
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
/// Panics if called inside another [`attempt`] on the same thread. A nested
/// run's changes would be kept when it succeeds, and then survive the outer
/// run's failure while that failure says the project was put back. Code that
/// writes inside a run takes the outer run's `&mut Changes` instead.
pub fn attempt<T>(
    retry: &str,
    run: impl FnOnce(&mut Changes) -> Result<T, Failure>,
) -> Result<T, Failure> {
    let _inside = InsideAttempt::enter();
    let mut changes = Changes::new();
    run(&mut changes).map_err(|failure| changes.undo(&failure, retry))
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
    /// use rituals_compose::rollback;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-rollback-write-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let generated = directory.join("main.rs");
    /// let original = "fn main() { rituals::run(); }\n";
    /// std::fs::write(&generated, original)?;
    ///
    /// let outcome = rollback::attempt("running `remove lint` again", |changes| {
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
    /// in which case nothing is written, or if writing it fails.
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
    /// use rituals_compose::rollback;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-rollback-run-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let lockfile = directory.join("Cargo.lock");
    ///
    /// let outcome = rollback::attempt("running `import lint` again", |changes| {
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
    /// read, in which case `change` does not run, or `change`'s own failure.
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

    /// Claims `path` as a directory this run will create, and records it and
    /// every directory above it that does not exist yet, so a failed run
    /// removes them all. Creates nothing: the caller creates the directory
    /// and fills it however it likes.
    ///
    /// The reserved directory is removed with everything in it. A directory
    /// above it is removed only if it is empty by then, so if the reserved
    /// directory could not be removed, both are reported rather than one
    /// being deleted out from under the other.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::rollback;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-rollback-reserve-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let existing = directory.join("tasks/lint");
    /// std::fs::create_dir_all(&existing)?;
    ///
    /// // A directory that already exists is not the run's to remove.
    /// let outcome = rollback::attempt("running `add lint` again", |changes| {
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
    /// that was there before the run is not the run's to remove.
    pub fn reserve_directory(&mut self, path: &Path) -> Result<(), Failure> {
        if path.exists() {
            return Err(Failure::new(format!(
                "{} already exists, so ritual will not create it",
                path.display()
            )));
        }
        let mut missing_parents: Vec<&Path> = path
            .ancestors()
            .skip(1)
            .take_while(|ancestor| !ancestor.as_os_str().is_empty() && !ancestor.exists())
            .collect();
        missing_parents.reverse();
        for parent in missing_parents {
            self.steps.push(Step::Parent {
                path: parent.to_path_buf(),
            });
        }
        self.steps.push(Step::Directory {
            path: path.to_path_buf(),
        });
        Ok(())
    }

    /// Records the bytes at `path`, or that there is no file there, unless
    /// this run has already recorded `path`: the first record is the one
    /// that holds what the project had before the run.
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
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

    /// Undoes every recorded change, most recent first, and returns
    /// `failure` extended with what happened to the project. Every step is
    /// attempted whatever happened to the ones before it, and each one that
    /// fails is named.
    fn undo(self, failure: &Failure, retry: &str) -> Failure {
        let mut not_restored: Vec<String> = Vec::new();
        for step in self.steps.iter().rev() {
            if step.undo().is_err() {
                not_restored.push(step.path().display().to_string());
            }
        }

        if not_restored.is_empty() {
            return Failure::new(format!(
                "{failure}; ritual put the project back as it found it"
            ));
        }

        let pronoun = if not_restored.len() == 1 {
            "check it"
        } else {
            "check them"
        };
        Failure::new(format!(
            "{failure}; ritual put the project back except for {} — {pronoun} before {retry}",
            crate::sentence::join_with_and(&not_restored),
        ))
    }
}

impl Step {
    fn path(&self) -> &Path {
        match self {
            Self::File { path, .. } | Self::Directory { path } | Self::Parent { path } => path,
        }
    }

    /// Puts this step's path back as it was, doing nothing when it already
    /// is, so a change the run never got as far as making is not reported
    /// as one that could not be undone.
    fn undo(&self) -> Result<(), Failure> {
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
                    return Ok(());
                }
                std::fs::remove_dir_all(path).map_err(|error| {
                    Failure::new(format!("removing {} failed", path.display())).caused_by(error)
                })
            }
            // Plain `remove_dir`, which refuses a directory that is not
            // empty: that refusal is the check that everything the run put
            // inside it really is gone.
            Self::Parent { path } => {
                if !path.exists() {
                    return Ok(());
                }
                std::fs::remove_dir(path).map_err(|error| {
                    Failure::new(format!("removing {} failed", path.display())).caused_by(error)
                })
            }
        }
    }
}

/// Writes `original` back to `path` unless the file already holds exactly
/// those bytes, so a file the run never got as far as writing is not
/// written to at all.
fn restore_file(path: &Path, original: &[u8]) -> Result<(), Failure> {
    match std::fs::read(path) {
        Ok(current) if current == original => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(Failure::new(format!("reading {} failed", path.display())).caused_by(error));
        }
    }
    std::fs::write(path, original).map_err(|error| {
        Failure::new(format!("writing {} failed", path.display())).caused_by(error)
    })
}

/// Removes the file the run created at `path`, if it is there.
fn remove_created_file(path: &Path) -> Result<(), Failure> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(Failure::new(format!("removing {} failed", path.display())).caused_by(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rituals::Failure;

    use super::{Changes, attempt};
    use crate::test_support::{ScratchDir, TestOutcome};

    /// The retry wording every test here hands [`attempt`], so an assertion
    /// on a whole message can name it.
    const RETRY: &str = "running `demo` again";

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    /// Runs `steps` inside [`attempt`] and then fails, returning the
    /// failure `attempt` reports. Asserts the failure is the simulated one,
    /// so a setup step that failed inside `steps` is never mistaken for the
    /// run's own failure.
    fn fail_after(steps: impl FnOnce(&mut Changes) -> Result<(), Failure>) -> Failure {
        let outcome = attempt(RETRY, |changes| {
            steps(changes)?;
            Err::<(), _>(Failure::new("simulated failure"))
        });
        let Err(reported) = outcome else {
            unreachable!("a run that always ends in Err cannot succeed");
        };
        assert!(
            reported.to_string().starts_with("simulated failure;"),
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
        let _ = attempt(RETRY, |_outer| attempt(RETRY, |_inner| Ok(())));
    }

    #[test]
    fn attempts_in_sequence_on_one_thread_each_run() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-sequence")?;
        let path = scratch.path().join("Cargo.toml");

        attempt(RETRY, |changes| changes.write(&path, "first\n"))?;
        let _ = fail_after(|changes| changes.write(&path, "second\n"));
        attempt(RETRY, |changes| changes.write(&path, "third\n"))?;

        assert_eq!(std::fs::read_to_string(&path)?, "third\n");
        Ok(())
    }

    #[test]
    fn an_attempt_after_one_that_panicked_still_runs() {
        // The refused nested call panics inside the outer run, so the outer
        // run unwinds without returning: the mark has to be cleared on the
        // way out regardless.
        let unwound = std::panic::catch_unwind(|| {
            let _ = attempt(RETRY, |_outer| attempt(RETRY, |_inner| Ok(())));
        });
        assert!(unwound.is_err(), "the nested attempt must have panicked");

        let outcome = attempt(RETRY, |_changes| Ok(()));
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

        let reported = fail_after(|changes| changes.write(&path, "version = 4\n"));

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

        let reported = fail_after(|changes| changes.write(&path, "version = 4\n"));

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

        let _ = fail_after(|changes| {
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

        let reported = fail_after(|changes| {
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
            format!(
                "simulated failure; ritual put the project back except for {} — check it \
                 before running `demo` again",
                path.display()
            )
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

        let _ = fail_after(|changes| {
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

        let outcome = attempt(RETRY, |changes| {
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
                .to_string()
                .starts_with(&format!("reading {} failed", unreadable.display())),
            "expected the unreadable path to be named: {reported}"
        );
        Ok(())
    }

    #[test]
    fn a_reserved_directory_goes_with_everything_in_it_and_the_parents_it_needed() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-reserved")?;
        let tasks = scratch.path().join("tasks");
        let task = tasks.join("lint");

        let reported = fail_after(|changes| {
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
    fn a_parent_that_was_already_there_is_left_alone() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-existing-parent")?;
        let tasks = scratch.path().join("tasks");
        std::fs::create_dir(&tasks)?;
        std::fs::write(tasks.join(".keep"), "predates the run\n")?;
        let task = tasks.join("lint");

        let _ = fail_after(|changes| {
            changes.reserve_directory(&task)?;
            std::fs::create_dir(&task).map_err(|error| Failure::new("setup").caused_by(error))
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

        let outcome = attempt(RETRY, |changes| changes.reserve_directory(&task));

        let Err(reported) = outcome else {
            unreachable!("reserving a directory that exists must be refused");
        };
        assert_eq!(
            reported.to_string(),
            format!(
                "{} already exists, so ritual will not create it; ritual put the project back \
                 as it found it",
                task.display()
            )
        );
        assert!(task.exists(), "a directory the run did not create stays");
        Ok(())
    }

    #[test]
    fn a_run_that_succeeds_keeps_its_changes() -> TestOutcome {
        let scratch = ScratchDir::new("rollback-success")?;
        let path = scratch.path().join("Cargo.toml");
        let task = scratch.path().join("tasks/lint");

        let outcome = attempt(RETRY, |changes| {
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

        let reported = fail_after(|changes| {
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
            format!(
                "simulated failure; ritual put the project back except for {} — check it \
                 before running `demo` again",
                created.display()
            )
        );
        assert!(
            created.exists(),
            "the file the undo could not remove is still there"
        );
        Ok(())
    }

    #[test]
    fn every_restore_that_fails_is_named_most_recent_first() -> TestOutcome {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::new("rollback-restores-fail")?;
        let tasks = scratch.path().join("tasks");
        let task = tasks.join("lint");
        let mut enforced = true;

        let reported = fail_after(|changes| {
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
                "every_restore_that_fails_is_named_most_recent_first could not demonstrate a \
                 failed removal because this process does not honour directory write \
                 permissions",
            );
            return Ok(());
        }

        // Before any assertion can return early, so the scratch directory
        // can still be removed on drop.
        std::fs::set_permissions(&task, std::fs::Permissions::from_mode(0o755))?;

        // `tasks/lint` cannot lose `src`, so it stays, and `tasks/` is not
        // empty, so it stays too: both are named, and neither is deleted
        // out from under the other.
        assert_eq!(
            reported.to_string(),
            format!(
                "simulated failure; ritual put the project back except for {} and {} — check \
                 them before running `demo` again",
                task.display(),
                tasks.display()
            )
        );
        assert!(task.join("src").is_dir());
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
}
