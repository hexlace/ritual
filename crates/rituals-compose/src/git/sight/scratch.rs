//! A work tree of copies, standing where the project's ignore and attribute
//! files will be once the directories have moved.
//!
//! Git decides what to ignore and which attributes to give a path from the
//! files in the work tree, and a moved directory's own `.gitignore` and
//! `.gitattributes` move with it. Asked before the move, git cannot see them
//! at the new place, so it is asked about a work tree that already holds them
//! there, with the repository's own git directory beside it: its
//! `info/exclude`, its configuration and its index are read exactly as they
//! are for the real tree.

use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};

use crate::git::Unanswered;

/// The files in a directory that decide what git ignores there and which
/// attributes it gives, by name.
const RULE_FILES: [&str; 2] = [".gitignore", ".gitattributes"];

/// How many names are tried for the scratch directory before giving up. A
/// name is taken by a directory a process of the same number left behind, or
/// by another check running in this process at the same moment, as a test run
/// has many; so a few dozen cover the checks that can overlap, and a bound
/// keeps a full temporary directory from being retried for ever.
const ATTEMPTS_MAX: u32 = 64;

/// The permissions of the scratch directory: the owner's alone, since it
/// holds copies of the project's own rules.
const OWNER_ONLY: u32 = 0o700;

/// A directory that removes itself, and everything in it, on drop.
#[derive(Debug)]
pub(super) struct Scratch {
    path: PathBuf,
}

impl Scratch {
    /// Creates a fresh, empty directory in the system's temporary one, named
    /// for this process.
    pub(super) fn create() -> Result<Self, Unanswered> {
        Self::create_in(&std::env::temp_dir(), std::process::id())
    }

    /// Creates a fresh, empty directory in `parent`, named by
    /// [`name`] from `process_id` and the first attempt whose name is free.
    fn create_in(parent: &Path, process_id: u32) -> Result<Self, Unanswered> {
        for attempt in 0..ATTEMPTS_MAX {
            let path = parent.join(name(process_id, attempt));
            match std::fs::DirBuilder::new().mode(OWNER_ONLY).create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(Unanswered::Failed(format!(
                        "creating {} failed: {error}",
                        path.display()
                    )));
                }
            }
        }
        Err(Unanswered::Failed(format!(
            "every name {} to {} in {} is taken",
            name(process_id, 0),
            name(process_id, ATTEMPTS_MAX - 1),
            parent.display()
        )))
    }

    /// The directory.
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Copies each `(from, to)`, the file at `from` under `source_root` to
    /// `to` under this directory, creating the directories between. A `from`
    /// that is not a regular file is skipped: git does not follow a symbolic
    /// link for these files, so a copy of what one leads to would be a rule
    /// git never reads.
    pub(super) fn fill(
        &self,
        source_root: &Path,
        copies: &[(String, String)],
    ) -> Result<(), Unanswered> {
        for (from, to) in copies {
            let source = source_root.join(from);
            let is_a_file = source
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.file_type().is_file());
            if !is_a_file {
                continue;
            }
            let target = self.path.join(to);
            let written = target
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::copy(&source, &target).map(drop));
            written.map_err(|error| {
                Unanswered::Failed(format!(
                    "copying {} for git to read failed: {error}",
                    source.display()
                ))
            })?;
        }
        Ok(())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // A directory that cannot be removed is left in the temporary
        // directory, which is the system's to clear; a destructor has no one
        // to tell, and the answer this directory was made for is already
        // given.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The name of the scratch directory for `process_id`'s `attempt`th try.
///
/// Pure, so that two processes never try the same name and one process's
/// tries never repeat: the process number tells processes apart, and the
/// attempt tells one process's tries apart.
fn name(process_id: u32, attempt: u32) -> String {
    format!("ritual-sight-{process_id}-{attempt}")
}

/// Which copies put the rule files where git will look for them once
/// `moved_directories`, pairs of where each is now and where it will be,
/// have moved, each as `(from, to)`, both spelled from the top level.
///
/// - The rule files in every directory above a moved directory's new place,
///   from the top level down, stay where they are: they decide for the new
///   place as they do for the old one.
/// - The rule files of `entries`, every file under a moved directory,
///   move with it, to the place `destination` says.
///
/// Those in the first group come first, so a copy from the second wins if a
/// directory is in both.
pub(super) fn rule_file_copies(
    moved_directories: &[(String, String)],
    entries: &[&str],
    destination: impl Fn(&str) -> String,
) -> Vec<(String, String)> {
    let mut copies: Vec<(String, String)> = Vec::new();
    for (_before, after) in moved_directories {
        // The top level, then each directory down to the one the moved
        // directory is in, each with its trailing slash.
        let above =
            std::iter::once("").chain(after.match_indices('/').map(|(slash, _)| &after[..=slash]));
        for directory in above {
            for file in RULE_FILES {
                copies.push((format!("{directory}{file}"), format!("{directory}{file}")));
            }
        }
    }
    copies.sort();
    copies.dedup();
    for entry in entries {
        let file_name = entry.rsplit('/').next().unwrap_or(entry);
        if RULE_FILES.contains(&file_name) {
            copies.push(((*entry).to_string(), destination(entry)));
        }
    }
    copies
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{ATTEMPTS_MAX, Scratch, name, rule_file_copies};
    use crate::git::Unanswered;
    use crate::test_support::{ScratchDir, TestOutcome};

    #[test]
    fn a_name_is_made_from_the_process_and_the_attempt_alone() {
        assert_eq!(name(4242, 0), "ritual-sight-4242-0");
        assert_eq!(name(4242, 3), "ritual-sight-4242-3");
        assert_ne!(name(4242, 0), name(4243, 0), "another process");
        assert_ne!(name(4242, 0), name(4242, 1), "another attempt");
        assert_eq!(name(4242, 1), name(4242, 1), "the same inputs");
    }

    #[test]
    fn a_name_taken_is_skipped_for_the_next_attempt() -> TestOutcome {
        let parent = ScratchDir::new("sight-scratch-taken")?;
        std::fs::create_dir(parent.path().join(name(77, 0)))?;
        std::fs::write(parent.path().join(name(77, 1)), "a file in the way")?;

        let scratch = Scratch::create_in(parent.path(), 77)?;

        assert_eq!(scratch.path(), parent.path().join(name(77, 2)));
        assert!(scratch.path().is_dir());
        Ok(())
    }

    #[test]
    fn every_name_taken_is_a_failure_after_a_bounded_number_of_tries() -> TestOutcome {
        let parent = ScratchDir::new("sight-scratch-exhausted")?;
        for attempt in 0..ATTEMPTS_MAX {
            std::fs::create_dir(parent.path().join(name(77, attempt)))?;
        }

        let result = Scratch::create_in(parent.path(), 77);

        assert!(
            matches!(&result, Err(Unanswered::Failed(message)) if message.contains("is taken")),
            "expected a failure naming the names taken, got {result:?}"
        );
        Ok(())
    }

    #[test]
    fn the_directory_and_what_is_in_it_go_when_it_is_dropped() -> TestOutcome {
        let parent = ScratchDir::new("sight-scratch-dropped")?;
        let scratch = Scratch::create_in(parent.path(), 77)?;
        let path = scratch.path().to_path_buf();
        std::fs::create_dir_all(path.join("deep/er"))?;
        std::fs::write(path.join("deep/er/.gitignore"), "x\n")?;

        drop(scratch);

        assert!(!path.exists());
        Ok(())
    }

    /// Only regular files are copied, to where they are told, with the
    /// directories made on the way; a link, and a file that is not there,
    /// are left out.
    #[test]
    fn only_regular_files_are_copied() -> TestOutcome {
        let parent = ScratchDir::new("sight-scratch-fill")?;
        let source = parent.path().join("source");
        std::fs::create_dir_all(source.join("tasks/greet"))?;
        std::fs::write(source.join("tasks/greet/.gitignore"), ".env\n")?;
        std::fs::write(source.join("real"), "ignored\n")?;
        std::os::unix::fs::symlink("../../real", source.join("tasks/greet/.gitattributes"))?;
        let scratch = Scratch::create_in(parent.path(), 77)?;

        scratch.fill(
            &source,
            &[
                (
                    "tasks/greet/.gitignore".to_string(),
                    ".rituals/greet/.gitignore".to_string(),
                ),
                (
                    "tasks/greet/.gitattributes".to_string(),
                    ".rituals/greet/.gitattributes".to_string(),
                ),
                ("absent".to_string(), "absent".to_string()),
            ],
        )?;

        assert_eq!(
            std::fs::read_to_string(scratch.path().join(".rituals/greet/.gitignore"))?,
            ".env\n"
        );
        assert!(
            scratch
                .path()
                .join(".rituals/greet/.gitattributes")
                .symlink_metadata()
                .is_err(),
            "a link is not copied"
        );
        assert!(!Path::new(&scratch.path().join("absent")).exists());
        Ok(())
    }

    /// The rule files above a new place stay where they are, and the ones
    /// under a moved directory go to its new place.
    #[test]
    fn the_rule_files_above_stay_and_the_ones_inside_go_along() {
        let moved = [("tasks/greet".to_string(), ".rituals/greet".to_string())];
        let destination = |entry: &str| entry.replacen("tasks/greet", ".rituals/greet", 1);

        let copies = rule_file_copies(
            &moved,
            &[
                "tasks/greet/.gitignore",
                "tasks/greet/Cargo.toml",
                "tasks/greet/sub/.gitattributes",
            ],
            destination,
        );

        let copy = |from: &str, to: &str| (from.to_string(), to.to_string());
        assert_eq!(
            copies,
            [
                copy(".gitattributes", ".gitattributes"),
                copy(".gitignore", ".gitignore"),
                copy(".rituals/.gitattributes", ".rituals/.gitattributes"),
                copy(".rituals/.gitignore", ".rituals/.gitignore"),
                copy("tasks/greet/.gitignore", ".rituals/greet/.gitignore"),
                copy(
                    "tasks/greet/sub/.gitattributes",
                    ".rituals/greet/sub/.gitattributes"
                ),
            ]
        );
    }
}
