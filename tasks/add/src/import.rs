//! Writing everything `add` has prepared, and putting the project back if a
//! write fails partway.
//
// `redundant_pub_crate` (clippy nursery) wants `pub` here because this
// module is private, but `pub(crate)` is the visibility that is actually
// true — it stays correct if this module is ever re-exported at a different
// level — so the nursery lint gives way.
#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) reflects this module's actual visibility even though the enclosing \
              module is private; see the note above"
)]

use std::path::{Path, PathBuf};

use rituals::{CommandLine, Failure, Name, Outcome, report};
use rituals_compose::manifest::Manifest;
use rituals_compose::source::Source;
use rituals_compose::{generated_file, task_crate, top_level};

/// Everything `add` is about to write, captured before the first write.
///
/// A run that reaches [`Import::write`] and fails partway is put back by
/// [`Import::undo`] — extending the same promise from refusals to failures:
/// a run of `add` that does not finish leaves the project exactly as it
/// found it, and says so.
pub(crate) struct Import {
    pub(crate) name: Name,
    pub(crate) task_crate_dir: PathBuf,
    /// `add` creates `tasks/` when this is the project's first task, and
    /// removes it again if the run does not finish.
    pub(crate) tasks_directory_was_created: bool,
    pub(crate) workspace_manifest: Manifest,
    pub(crate) cli_manifest: Manifest,
    pub(crate) dependency_path: String,
    pub(crate) workspace_root: PathBuf,
}

impl Import {
    /// The four writes, in the order they are reported: the task crate's own
    /// two files, then the two manifests. Stops at the first failure.
    fn write(&mut self) -> Outcome {
        write_task_crate(&self.task_crate_dir, &self.name)?;

        let member = format!("tasks/{}", self.name);
        self.workspace_manifest.append_workspace_member(&member)?;
        self.workspace_manifest.write()?;
        report(format!(
            "updated {}",
            relative_to(self.workspace_manifest.path(), &self.workspace_root).display()
        ));

        self.cli_manifest
            .import_task(&self.name, &self.dependency_path)?;
        self.cli_manifest.write()?;
        report(format!(
            "updated {}",
            relative_to(self.cli_manifest.path(), &self.workspace_root).display()
        ));

        Ok(())
    }

    /// Puts back whatever is on disk that should not be, and returns the
    /// failure to report. Carries no progress marker: every step is
    /// conditioned on what it finds on disk, so it is correct whichever
    /// write failed.
    ///
    /// Manifests are restored before the directory is removed: a member
    /// entry pointing at a directory that is gone is a worse state than a
    /// directory nothing points at, so if the restore is the step that
    /// fails, the directory is still there. Every step's result is checked
    /// the same way — a removal that fails leaves the project exactly as
    /// unrestored as a manifest that could not be written back, and saying
    /// so is the whole point of this function.
    fn undo(&self, failure: &Failure) -> Failure {
        let mut not_restored: Vec<String> = Vec::new();

        if self.cli_manifest.restore_if_changed().is_err() {
            not_restored.push(self.cli_manifest.path().display().to_string());
        }
        if self.workspace_manifest.restore_if_changed().is_err() {
            not_restored.push(self.workspace_manifest.path().display().to_string());
        }
        if self.task_crate_dir.exists() && std::fs::remove_dir_all(&self.task_crate_dir).is_err() {
            not_restored.push(self.task_crate_dir.display().to_string());
        }
        let tasks_directory = self.workspace_root.join("tasks");
        // Plain `remove_dir`, which refuses a non-empty directory — that is
        // the check that the crate directory really is gone, not an extra
        // one. `tasks_directory.exists()` guards against a run that failed
        // before `tasks/` itself was ever created: `remove_dir` on a path
        // that was never there is not a restoration failure, it is nothing
        // to restore.
        if self.tasks_directory_was_created
            && tasks_directory.exists()
            && std::fs::remove_dir(&tasks_directory).is_err()
        {
            not_restored.push(tasks_directory.display().to_string());
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
            "{failure}; ritual put the project back except for {} — {pronoun} before running \
             `add {}` again",
            rituals_compose::sentence::join_with_and(&not_restored),
            self.name
        ))
    }
}

/// Writes everything `import` describes, and either finishes by running the
/// same path `regenerate` does, or puts the project back and reports why.
pub(crate) fn finish(command_line: &CommandLine, mut import: Import) -> Outcome {
    match import.write() {
        Ok(()) => finish_by_regenerating(command_line, &import),
        Err(failure) => Err(import.undo(&failure)),
    }
}

/// `add` has no idea of the task list of its own; it writes the manifests
/// and then runs the regenerate path, so the two can never drift. Ends by
/// naming the file to edit and the command that runs it.
fn finish_by_regenerating(command_line: &CommandLine, import: &Import) -> Outcome {
    generated_file::regenerate(command_line).map_err(|failure| {
        Failure::new(format!(
            "{failure}; `{}` is imported — run `{}` to finish",
            import.name,
            top_level::management_command(command_line, "regenerate")
        ))
    })?;
    report(next_step(
        &import.name,
        command_line.identity().binary_name(),
    ));
    Ok(())
}

/// The line `add` ends on: the file the new task's code goes in, and what
/// a person types to run it. A task imported from the project's own
/// manifest is always a top-level command under its own key.
fn next_step(name: &Name, binary_name: &str) -> String {
    format!("next: edit tasks/{name}/src/lib.rs, then run cargo {binary_name} {name}")
}

/// Writes `tasks/<name>/Cargo.toml` and `tasks/<name>/src/lib.rs`, and
/// reports both.
fn write_task_crate(task_crate_dir: &Path, name: &Name) -> Outcome {
    let source_directory = task_crate_dir.join("src");
    std::fs::create_dir_all(&source_directory).map_err(|error| {
        Failure::new(format!("creating {} failed", source_directory.display())).caused_by(error)
    })?;

    let manifest_path = task_crate_dir.join("Cargo.toml");
    let manifest_text = task_crate::manifest(name, &Source::Inherited);
    std::fs::write(&manifest_path, manifest_text).map_err(|error| {
        Failure::new(format!("writing {} failed", manifest_path.display())).caused_by(error)
    })?;
    report(format!("created tasks/{name}/Cargo.toml"));

    let lib_path = source_directory.join("lib.rs");
    let lib_text = task_crate::lib(name);
    std::fs::write(&lib_path, lib_text).map_err(|error| {
        Failure::new(format!("writing {} failed", lib_path.display())).caused_by(error)
    })?;
    report(format!("created tasks/{name}/src/lib.rs"));

    Ok(())
}

/// Renders `path` relative to `workspace_root`, the shape `add` and
/// `regenerate` report paths in (they are about a project; `new` and
/// `create`, which are not, report relative to the working directory
/// instead).
fn relative_to<'a>(path: &'a Path, workspace_root: &Path) -> &'a Path {
    path.strip_prefix(workspace_root).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::PathBuf;

    use rituals::Name;
    use rituals_compose::manifest::Manifest;

    use super::{Import, next_step};
    use crate::test_support::{ScratchDir, TestOutcome};

    #[test]
    fn the_next_step_names_the_file_to_edit_and_the_command_that_runs_it() {
        let name = Name::new("hello").expect("hello is a valid name");
        assert_eq!(
            next_step(&name, "acme"),
            "next: edit tasks/hello/src/lib.rs, then run cargo acme hello"
        );
    }

    /// Every file under a directory tree, as a path paired with its bytes,
    /// sorted by path — a before/after diff for `undo` to be checked
    /// against.
    type Snapshot = Vec<(PathBuf, Vec<u8>)>;

    /// A scratch project: a workspace manifest with one member and a
    /// composed CLI manifest with one task already imported, ready for
    /// `Import::write`/`undo` to be exercised directly against real files.
    struct ScratchProject {
        _root: ScratchDir,
        workspace_manifest_path: PathBuf,
        cli_manifest_path: PathBuf,
        workspace_root: PathBuf,
    }

    impl ScratchProject {
        fn new(tag: &str) -> Result<Self, Box<dyn Error>> {
            let root = ScratchDir::new(tag)?;
            let workspace_root = root.path().to_path_buf();
            std::fs::create_dir_all(workspace_root.join("ritual/src"))?;

            let workspace_manifest_path = workspace_root.join("Cargo.toml");
            std::fs::write(
                &workspace_manifest_path,
                "[workspace]\nmembers = [\n    \"ritual\",\n]\nresolver = \"3\"\n",
            )?;

            let cli_manifest_path = workspace_root.join("ritual/Cargo.toml");
            std::fs::write(
                &cli_manifest_path,
                "[package]\nname = \"demo-ritual\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
                 [dependencies]\nrituals.workspace = true\n\n\
                 [package.metadata.ritual]\ntasks = []\n",
            )?;

            Ok(Self {
                _root: root,
                workspace_manifest_path,
                cli_manifest_path,
                workspace_root,
            })
        }

        fn import(&self, name: &str) -> Result<Import, Box<dyn Error>> {
            Ok(Import {
                name: Name::new(name)?,
                task_crate_dir: self.workspace_root.join("tasks").join(name),
                tasks_directory_was_created: true,
                workspace_manifest: Manifest::read(&self.workspace_manifest_path)?,
                cli_manifest: Manifest::read(&self.cli_manifest_path)?,
                dependency_path: format!("../tasks/{name}"),
                workspace_root: self.workspace_root.clone(),
            })
        }

        fn snapshot(&self) -> Result<Snapshot, Box<dyn Error>> {
            let mut files = Vec::new();
            let mut pending = vec![self.workspace_root.clone()];
            while let Some(directory) = pending.pop() {
                for entry in std::fs::read_dir(&directory)? {
                    let entry = entry?;
                    let path = entry.path();
                    if entry.file_type()?.is_dir() {
                        pending.push(path);
                    } else {
                        let bytes = std::fs::read(&path)?;
                        files.push((path, bytes));
                    }
                }
            }
            files.sort_by(|left, right| left.0.cmp(&right.0));
            Ok(files)
        }
    }

    #[test]
    fn undo_after_only_the_crate_directory_was_written() -> TestOutcome {
        let project = ScratchProject::new("undo-crate-only")?;
        let before = project.snapshot()?;

        let import = project.import("lint")?;
        super::write_task_crate(&import.task_crate_dir, &import.name)?;
        // Simulate the workspace-manifest write failing: nothing else has
        // happened yet.
        let failure = rituals::Failure::new("simulated failure");
        let reported = import.undo(&failure);

        assert!(
            reported
                .to_string()
                .contains("put the project back as it found it")
        );
        assert_eq!(
            project.snapshot()?,
            before,
            "the tree must be exactly as it started"
        );
        assert!(
            !project.workspace_root.join("tasks").exists(),
            "tasks/ must be gone too"
        );
        Ok(())
    }

    #[test]
    fn undo_after_the_workspace_manifest_was_also_rewritten() -> TestOutcome {
        let project = ScratchProject::new("undo-workspace-too")?;
        let before = project.snapshot()?;

        let mut import = project.import("lint")?;
        super::write_task_crate(&import.task_crate_dir, &import.name)?;
        import
            .workspace_manifest
            .append_workspace_member("tasks/lint")?;
        import.workspace_manifest.write()?;

        let failure = rituals::Failure::new("simulated failure");
        let reported = import.undo(&failure);

        assert!(
            reported
                .to_string()
                .contains("put the project back as it found it")
        );
        assert_eq!(
            project.snapshot()?,
            before,
            "the tree must be exactly as it started"
        );
        Ok(())
    }

    #[test]
    fn undo_after_the_cli_manifest_was_also_rewritten() -> TestOutcome {
        let project = ScratchProject::new("undo-cli-too")?;
        let before = project.snapshot()?;

        let mut import = project.import("lint")?;
        super::write_task_crate(&import.task_crate_dir, &import.name)?;
        import
            .workspace_manifest
            .append_workspace_member("tasks/lint")?;
        import.workspace_manifest.write()?;
        import
            .cli_manifest
            .import_task(&import.name, &import.dependency_path)?;
        import.cli_manifest.write()?;

        let failure = rituals::Failure::new("simulated failure");
        let reported = import.undo(&failure);

        assert!(
            reported
                .to_string()
                .contains("put the project back as it found it")
        );
        assert_eq!(
            project.snapshot()?,
            before,
            "the tree must be exactly as it started"
        );
        Ok(())
    }

    #[test]
    fn undo_leaves_tasks_directory_when_add_did_not_create_it() -> TestOutcome {
        let project = ScratchProject::new("undo-preexisting-tasks")?;
        std::fs::create_dir_all(project.workspace_root.join("tasks"))?;
        std::fs::write(
            project.workspace_root.join("tasks/.keep"),
            "a file that predates this add run\n",
        )?;

        let mut import = project.import("lint")?;
        import.tasks_directory_was_created = false;
        super::write_task_crate(&import.task_crate_dir, &import.name)?;

        let failure = rituals::Failure::new("simulated failure");
        let _ = import.undo(&failure);

        assert!(
            project.workspace_root.join("tasks/.keep").is_file(),
            "tasks/ must survive when add did not create it"
        );
        assert!(
            !import.task_crate_dir.exists(),
            "the crate directory add did create must still go"
        );
        Ok(())
    }

    /// A directory without write permission refuses to have entries removed
    /// from it — that is what write permission controls on a directory, not
    /// on the entries inside it — so `chmod 0o555` on the crate directory
    /// makes `remove_dir_all` fail without needing anything to hold the
    /// directory open. Verifies `undo` reports the directory by name and
    /// leaves it on disk rather than claiming full restoration. Unix-only:
    /// this permission model has no direct Windows equivalent.
    ///
    /// Root ignores a directory's own write bit, so as uid 0 the removal this
    /// test relies on failing would succeed instead, and the second
    /// `set_permissions` below would fail with `NotFound` on a directory
    /// `remove_dir_all` had already removed. A probe write right after the
    /// `chmod` tells the two cases apart, so a run under that condition
    /// reports "could not demonstrate" instead of a false pass or a spurious
    /// failure.
    #[cfg(unix)]
    #[test]
    fn undo_reports_the_crate_directory_when_removing_it_fails() -> TestOutcome {
        use std::os::unix::fs::PermissionsExt;

        let project = ScratchProject::new("undo-crate-removal-fails")?;
        let import = project.import("lint")?;
        super::write_task_crate(&import.task_crate_dir, &import.name)?;

        std::fs::set_permissions(
            &import.task_crate_dir,
            std::fs::Permissions::from_mode(0o555),
        )?;

        // Root ignores a directory's missing write bit entirely, so a probe
        // write tells whether this process is actually subject to the
        // permission above. If it is not, `remove_dir_all` below would
        // succeed despite the `0o555`, and the scenario this test
        // demonstrates cannot occur for this user: restore and return early,
        // so a skipped check says so rather than passing silently.
        let probe = import.task_crate_dir.join("probe");
        let permission_is_enforced = std::fs::File::create(&probe).is_err();
        if !permission_is_enforced {
            let _ = std::fs::remove_file(&probe);
            std::fs::set_permissions(
                &import.task_crate_dir,
                std::fs::Permissions::from_mode(0o755),
            )?;
            crate::test_support::report_skip(
                "undo_reports_the_crate_directory_when_removing_it_fails could \
                     not demonstrate a permission-denied removal because this process does \
                     not honour directory write permissions",
            );
            return Ok(());
        }

        let failure = rituals::Failure::new("simulated failure");
        let reported = import.undo(&failure);

        // Restore permissions before any assertion can return early, so the
        // scratch directory this test made is still removable on drop
        // whether or not the assertions below pass.
        std::fs::set_permissions(
            &import.task_crate_dir,
            std::fs::Permissions::from_mode(0o755),
        )?;

        assert!(
            reported
                .to_string()
                .contains(&import.task_crate_dir.display().to_string()),
            "expected the message to name the directory that could not be removed: {reported}"
        );
        assert!(
            import.task_crate_dir.exists(),
            "the directory undo could not remove must still be there"
        );
        Ok(())
    }
}
