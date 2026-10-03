//! Preparing and writing everything `remove` does, putting the project back
//! if it is refused or a write fails, and deleting the task's directory once
//! nothing can.
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

use std::fmt::Display;
use std::path::{Path, PathBuf};

use rituals::{CommandLine, Failure, Name, Outcome, report};
use rituals_compose::generated_file::{self, Regenerated};
use rituals_compose::manifest::Manifest;
use rituals_compose::metadata;
use rituals_compose::rollback::{self, Changes};

/// The manifests `remove` edits: the composed CLI's, and the workspace's.
///
/// They are one file when the composed CLI is the workspace root. Two
/// documents read from one file would each be written back whole, and the
/// second write would discard the first's edits, so then there is one
/// document and the workspace's edits go to it.
pub(crate) struct Manifests {
    cli: Manifest,
    workspace: Option<Manifest>,
}

impl Manifests {
    /// Reads the composed CLI's manifest, and the workspace's too when it is
    /// a different file.
    pub(crate) fn read(cli_path: &Path, workspace_path: &Path) -> Result<Self, Failure> {
        let cli = Manifest::read(cli_path)?;
        let workspace = if cli_path == workspace_path {
            None
        } else {
            Some(Manifest::read(workspace_path)?)
        };
        Ok(Self { cli, workspace })
    }

    /// The manifest that holds `[workspace]`.
    pub(crate) const fn workspace(&self) -> &Manifest {
        match &self.workspace {
            Some(workspace) => workspace,
            None => &self.cli,
        }
    }

    /// The manifest that holds `[workspace]`, to edit.
    fn workspace_mut(&mut self) -> &mut Manifest {
        self.workspace.as_mut().unwrap_or(&mut self.cli)
    }

    /// The composed CLI's manifest.
    pub(crate) const fn cli(&self) -> &Manifest {
        &self.cli
    }

    /// The composed CLI's manifest, to edit.
    pub(crate) const fn cli_mut(&mut self) -> &mut Manifest {
        &mut self.cli
    }
}

/// A workspace member whose directory `remove` deletes.
pub(crate) struct Member {
    /// The directory, as `cargo metadata` gives it.
    pub(crate) directory: PathBuf,
    /// The directory from the workspace root, as the report names it.
    pub(crate) relative: String,
    /// The directory from git's top level, which the advice for getting it
    /// back spells with the `:/` pathspec magic so it works from anywhere in
    /// the project.
    pub(crate) from_top_level: PathBuf,
}

/// Everything `remove` is about to write, captured before the first write.
///
/// [`finish`] prepares it and runs the writes inside one
/// [`rollback::attempt`], so a run that is refused or fails partway is put
/// back: a `remove` that does not finish leaves the project exactly as it
/// found it, and says so.
pub(crate) struct Removal {
    /// The key in `[package.metadata.ritual] tasks`.
    pub(crate) key: String,
    pub(crate) manifests: Manifests,
    pub(crate) workspace_root: PathBuf,
    /// Whether a normal dependency of the composed CLI has this key.
    pub(crate) has_dependency: bool,
    /// Whether the `[workspace.dependencies]` entry the dependency inherits
    /// goes too, because nothing else in the workspace depends on it.
    pub(crate) drops_inherited_entry: bool,
    /// The member whose directory is deleted, if the key imports one.
    pub(crate) member: Option<Member>,
}

/// What a finished run wrote, for the report.
struct Written {
    regenerated: Regenerated,
    /// Whether the workspace manifest, a different file from the composed
    /// CLI's, was written.
    workspace_manifest: bool,
}

/// What a person runs again once they have checked whatever a failed run
/// could not put back, given what they typed as the task's name.
pub(crate) fn retry(argument: &str) -> String {
    format!("running `remove {argument}` again")
}

impl Removal {
    /// The writes, in the order that keeps the project building at every
    /// step: the key leaves `tasks` and the generated file is rewritten
    /// without it before the dependency it mounted is removed, because a
    /// generated file that mounts a dependency that is gone no longer
    /// compiles, and nothing could then regenerate it.
    ///
    /// Ends by asking Cargo to read the edited manifests again, which
    /// refuses a result that no longer resolves and leaves `Cargo.lock`
    /// current for what remains.
    fn write(
        &mut self,
        changes: &mut Changes,
        command_line: &CommandLine,
    ) -> Result<Written, Failure> {
        self.unlist(changes)?;
        let regenerated =
            generated_file::regenerate_recording(changes, command_line, &self.workspace_root)?;
        let workspace_manifest = self.take_out_dependency(changes)?;
        self.ensure_it_still_resolves(changes, command_line.identity().package_name())?;
        Ok(Written {
            regenerated,
            workspace_manifest,
        })
    }

    /// Takes the key out of `tasks` and writes the composed CLI's manifest.
    fn unlist(&mut self, changes: &mut Changes) -> Outcome {
        self.manifests.cli_mut().unlist_task(&self.key)?;
        self.manifests.cli().write(changes)
    }

    /// Removes the key's dependency, the `[workspace.dependencies]` entry
    /// it inherits when nothing else uses it, and a member's `members`
    /// entry, and writes what changed. Returns whether a workspace manifest
    /// that is a different file from the composed CLI's was written.
    fn take_out_dependency(&mut self, changes: &mut Changes) -> Result<bool, Failure> {
        if !self.has_dependency {
            return Ok(false);
        }
        if !self.manifests.cli_mut().remove_dependency(&self.key) {
            return Err(Failure::new(format!(
                "{} has no dependency called `{}` to remove",
                self.manifests.cli().path().display(),
                self.key
            )));
        }

        let workspace = self.manifests.workspace_mut();
        let mut edited = false;
        if self.drops_inherited_entry {
            edited |= workspace.remove_workspace_dependency(&self.key);
        }
        if let Some(member) = &self.member {
            edited |= workspace.remove_workspace_member(&member.directory);
        }

        self.manifests.cli().write(changes)?;
        match (&self.manifests.workspace, edited) {
            (Some(workspace), true) => {
                workspace.write(changes)?;
                Ok(true)
            }
            (Some(_) | None, _) => Ok(false),
        }
    }

    /// Reads the project the way Cargo now does, inside the run so that a
    /// failure puts everything back, and checks that the key is gone.
    ///
    /// Reading can rewrite `Cargo.lock`, which is recorded first. A
    /// dependency line removed from a manifest leaves the lock stale, and a
    /// build that must not change it fails until something brings it up to
    /// date.
    fn ensure_it_still_resolves(&self, changes: &mut Changes, package: &str) -> Outcome {
        let document = metadata::fetch_recording(changes, &self.workspace_root)?;

        let still_listed = document
            .task_imports(package)?
            .iter()
            .any(|task| task.key() == self.key);
        // A key that is not a valid name could not have been imported by
        // `add` or by a person following the design, and its dependency is
        // not checked here; the generated file already no longer mounts it.
        let still_a_dependency = match Name::new(&self.key) {
            Ok(name) => document
                .locate_project(package)?
                .declares_dependency_key(&name),
            Err(_) => false,
        };
        ensure_it_is_gone(&self.key, still_listed, still_a_dependency)
    }

    /// Deletes the member's directory, if there is one, and reports it.
    ///
    /// Runs after the writes have all succeeded and cannot be undone by the
    /// rollback; git is the way back. If it fails partway, the manifests are
    /// already updated, and the failure says so and says how to get back
    /// what was deleted.
    fn delete_the_directory(&self) -> Outcome {
        let Some(member) = &self.member else {
            return Ok(());
        };
        std::fs::remove_dir_all(&member.directory)
            .map_err(|error| deletion_failure(&member.relative, &member.from_top_level, error))?;
        report(format!("deleted {}", member.relative));
        Ok(())
    }
}

/// Prepares a removal with `prepare` and writes everything it describes:
/// either every write, then the deletion, or none of the writes and a
/// report of why.
///
/// `prepare` runs inside the same [`rollback::attempt`] as the writes, so
/// whatever it records, such as the lockfile its `cargo metadata` may
/// write, is put back when it refuses. `argument` is what the person typed
/// as the task's name, for the retry a failure names.
pub(crate) fn finish(
    command_line: &CommandLine,
    argument: &str,
    prepare: impl FnOnce(&mut Changes) -> Result<Removal, Failure>,
) -> Outcome {
    let (removal, written) = rollback::attempt(&retry(argument), |changes| {
        let mut removal = prepare(changes)?;
        let written = removal.write(changes, command_line)?;
        Ok((removal, written))
    })?;

    let relative_cli_manifest =
        relative_to(removal.manifests.cli().path(), &removal.workspace_root);
    let relative_workspace_manifest = written.workspace_manifest.then(|| {
        relative_to(
            removal.manifests.workspace().path(),
            &removal.workspace_root,
        )
    });
    for line in report_lines(
        relative_cli_manifest,
        &written.regenerated,
        relative_workspace_manifest,
    ) {
        report(line);
    }

    removal.delete_the_directory()
}

/// The lines reported once a run has written everything, one per file and
/// in the order they were written: the composed CLI's manifest, the
/// generated file, then the workspace manifest if it changed.
///
/// The composed CLI's manifest is written twice, and the report is made
/// after the run rather than as it goes so that it names the file once and
/// names nothing a failed run put back.
fn report_lines(
    cli_manifest: &Path,
    regenerated: &impl Display,
    workspace_manifest: Option<&Path>,
) -> Vec<String> {
    let mut lines = vec![
        format!("updated {}", cli_manifest.display()),
        regenerated.to_string(),
    ];
    if let Some(workspace_manifest) = workspace_manifest {
        lines.push(format!("updated {}", workspace_manifest.display()));
    }
    lines
}

/// Fails if the key is still in `tasks` or is still a dependency after the
/// run's edits, which means an edit did not do what it was meant to.
fn ensure_it_is_gone(key: &str, still_listed: bool, still_a_dependency: bool) -> Outcome {
    match (still_listed, still_a_dependency) {
        (false, false) => Ok(()),
        (true, false) => Err(Failure::new(format!(
            "`{key}` is still in [package.metadata.ritual] tasks after it was removed"
        ))),
        (false, true) => Err(Failure::new(format!(
            "`{key}` is still a dependency after it was removed"
        ))),
        (true, true) => Err(Failure::new(format!(
            "`{key}` is still in [package.metadata.ritual] tasks and still a dependency after \
             it was removed"
        ))),
    }
}

/// The failure for a deletion that did not finish, after the manifests were
/// already updated.
///
/// It says what is done and what is not, in that order, because the rollback
/// has nothing to put back here and must not be taken to have. The advice
/// names the directory from git's top level with the `:/` pathspec magic,
/// since a person may run it from any directory of the project.
fn deletion_failure(directory: &str, from_top_level: &Path, error: std::io::Error) -> Failure {
    Failure::new(format!(
        "deleting {directory} failed partway, and the manifests are already updated; git can \
         give back anything that was deleted (`git checkout -- :/{}`), or delete what is left \
         by hand",
        from_top_level.display()
    ))
    .caused_by(error)
}

/// Renders `path` relative to `workspace_root`, the shape `remove` reports
/// paths in.
fn relative_to<'a>(path: &'a Path, workspace_root: &Path) -> &'a Path {
    path.strip_prefix(workspace_root).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::PathBuf;

    use rituals::{Failure, Outcome};
    use rituals_compose::rollback::{self, Changes};

    use super::{
        Manifests, Member, Removal, deletion_failure, ensure_it_is_gone, relative_to, report_lines,
        retry,
    };
    use crate::test_support::{ScratchDir, TestOutcome};

    /// Every file under a directory tree, as a path paired with its bytes,
    /// sorted by path — a before/after diff for the undo to be checked
    /// against.
    type Snapshot = Vec<(PathBuf, Vec<u8>)>;

    /// A scratch project: a workspace with the composed CLI and one task
    /// crate `lint`, which the CLI imports by inheriting it from
    /// `[workspace.dependencies]`, ready for the steps of
    /// `Removal::write`, and the undo of a run that fails after them, to be
    /// exercised directly against real files.
    struct ScratchProject {
        _root: ScratchDir,
        workspace_root: PathBuf,
        cli_manifest_path: PathBuf,
        workspace_manifest_path: PathBuf,
    }

    const WORKSPACE_MANIFEST: &str = "[workspace]\nmembers = [\n    \"ritual\",\n    \
        \"tasks/lint\",\n]\nresolver = \"3\"\n\n[workspace.dependencies]\n\
        lint = { path = \"tasks/lint\" }\n";

    const CLI_MANIFEST: &str = "[package]\nname = \"demo-ritual\"\nversion = \"0.1.0\"\n\n\
        [dependencies]\nlint.workspace = true\n\n\
        [package.metadata.ritual]\ntasks = [\n    \"lint\",\n]\n";

    impl ScratchProject {
        fn new(tag: &str) -> Result<Self, Box<dyn Error>> {
            let root = ScratchDir::new(tag)?;
            let workspace_root = root.path().to_path_buf();
            std::fs::create_dir_all(workspace_root.join("ritual/src"))?;
            std::fs::create_dir_all(workspace_root.join("tasks/lint/src"))?;
            std::fs::write(workspace_root.join("tasks/lint/src/lib.rs"), "// lint\n")?;

            let workspace_manifest_path = workspace_root.join("Cargo.toml");
            std::fs::write(&workspace_manifest_path, WORKSPACE_MANIFEST)?;
            let cli_manifest_path = workspace_root.join("ritual/Cargo.toml");
            std::fs::write(&cli_manifest_path, CLI_MANIFEST)?;

            Ok(Self {
                _root: root,
                workspace_root,
                cli_manifest_path,
                workspace_manifest_path,
            })
        }

        /// A project whose composed CLI is the workspace root, so both
        /// manifests are one file.
        fn single_manifest(tag: &str) -> Result<Self, Box<dyn Error>> {
            let project = Self::new(tag)?;
            let combined = format!(
                "[package]\nname = \"demo-ritual\"\nversion = \"0.1.0\"\n\n\
                 [dependencies]\nlint.workspace = true\n\n\
                 [package.metadata.ritual]\ntasks = [\n    \"lint\",\n]\n\n\
                 {WORKSPACE_MANIFEST}"
            );
            std::fs::write(&project.workspace_manifest_path, combined)?;
            Ok(Self {
                cli_manifest_path: project.workspace_manifest_path.clone(),
                ..project
            })
        }

        fn removal(&self) -> Result<Removal, Box<dyn Error>> {
            Ok(Removal {
                key: "lint".to_string(),
                manifests: Manifests::read(&self.cli_manifest_path, &self.workspace_manifest_path)?,
                workspace_root: self.workspace_root.clone(),
                has_dependency: true,
                drops_inherited_entry: true,
                member: Some(Member {
                    directory: self.workspace_root.join("tasks/lint"),
                    relative: "tasks/lint".to_string(),
                    from_top_level: PathBuf::from("project/tasks/lint"),
                }),
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

    /// Runs `steps` — the first few of `Removal::write`'s — inside the same
    /// rollback `finish` runs it in, with the same retry wording, then fails
    /// the way the next step would, and returns the failure the rollback
    /// reports. Asserts it is the simulated failure, so a step that failed
    /// is never mistaken for the one simulated after it.
    fn fail_after(
        removal: &mut Removal,
        steps: impl FnOnce(&mut Removal, &mut Changes) -> Outcome,
    ) -> Failure {
        let outcome = rollback::attempt(&retry("lint"), |changes| {
            steps(removal, changes)?;
            Err::<(), _>(Failure::new("simulated failure"))
        });
        let Err(reported) = outcome else {
            unreachable!("a run that always ends in Err cannot succeed");
        };
        assert!(
            reported.to_string().starts_with("simulated failure;"),
            "a step before the simulated failure failed: {reported}"
        );
        reported
    }

    #[test]
    fn the_retry_names_remove_and_what_the_person_typed() {
        assert_eq!(
            retry("rituals-core-lint"),
            "running `remove rituals-core-lint` again"
        );
    }

    #[test]
    fn undo_after_only_the_key_was_unlisted() -> TestOutcome {
        let project = ScratchProject::new("undo-unlisted")?;
        let before = project.snapshot()?;

        let mut removal = project.removal()?;
        let reported = fail_after(&mut removal, Removal::unlist);

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
    fn undo_after_the_dependency_and_the_member_entry_were_also_removed() -> TestOutcome {
        let project = ScratchProject::new("undo-dependency-removed")?;
        let before = project.snapshot()?;

        let mut removal = project.removal()?;
        let reported = fail_after(&mut removal, |removal, changes| {
            removal.unlist(changes)?;
            removal.take_out_dependency(changes).map(drop)
        });

        assert!(
            reported
                .to_string()
                .contains("put the project back as it found it")
        );
        assert_eq!(
            project.snapshot()?,
            before,
            "both manifests must be exactly as they started"
        );
        assert!(
            project
                .workspace_root
                .join("tasks/lint/src/lib.rs")
                .is_file(),
            "the directory is deleted only after the run, so the undo has none to restore"
        );
        Ok(())
    }

    /// The steps `remove` takes between the generated file and the final
    /// read: with a separate workspace manifest, the key leaves `tasks`,
    /// the dependency line goes, and so do the inherited
    /// `[workspace.dependencies]` entry and the `members` entry.
    #[test]
    fn the_key_its_dependency_and_its_workspace_entries_are_taken_out() -> TestOutcome {
        let project = ScratchProject::new("take-out")?;
        let mut removal = project.removal()?;

        let workspace_written = rollback::attempt("retry", |changes| {
            removal.unlist(changes)?;
            removal.take_out_dependency(changes)
        })?;

        assert!(workspace_written, "the workspace manifest changed");
        assert_eq!(
            std::fs::read_to_string(&project.cli_manifest_path)?,
            "[package]\nname = \"demo-ritual\"\nversion = \"0.1.0\"\n\n[dependencies]\n\n\
             [package.metadata.ritual]\ntasks = [\n]\n"
        );
        assert_eq!(
            std::fs::read_to_string(&project.workspace_manifest_path)?,
            "[workspace]\nmembers = [\n    \"ritual\",\n]\nresolver = \"3\"\n\n\
             [workspace.dependencies]\n"
        );
        Ok(())
    }

    /// Another workspace package that depends on the crate keeps the
    /// `[workspace.dependencies]` entry its own dependency inherits.
    #[test]
    fn the_inherited_entry_stays_when_something_else_uses_it() -> TestOutcome {
        let project = ScratchProject::new("inherited-entry-stays")?;
        let mut removal = project.removal()?;
        removal.drops_inherited_entry = false;
        removal.member = None;

        rollback::attempt("retry", |changes| {
            removal.unlist(changes)?;
            removal.take_out_dependency(changes)
        })?;

        let workspace = std::fs::read_to_string(&project.workspace_manifest_path)?;
        assert!(
            workspace.contains("lint = { path = \"tasks/lint\" }"),
            "the entry is still declared: {workspace}"
        );
        assert!(
            workspace.contains("\"tasks/lint\","),
            "no member was removed either: {workspace}"
        );
        Ok(())
    }

    /// A key that lists no dependency is only taken out of `tasks`: the
    /// manifests have nothing else of it to remove.
    #[test]
    fn a_key_with_no_dependency_leaves_the_manifests_alone() -> TestOutcome {
        let project = ScratchProject::new("no-dependency")?;
        let mut removal = project.removal()?;
        removal.has_dependency = false;
        removal.member = None;
        let workspace_before = std::fs::read_to_string(&project.workspace_manifest_path)?;

        let workspace_written = rollback::attempt("retry", |changes| {
            removal.unlist(changes)?;
            removal.take_out_dependency(changes)
        })?;

        assert!(!workspace_written);
        assert_eq!(
            std::fs::read_to_string(&project.workspace_manifest_path)?,
            workspace_before
        );
        let cli = std::fs::read_to_string(&project.cli_manifest_path)?;
        assert!(cli.contains("lint.workspace = true"), "{cli}");
        Ok(())
    }

    /// A manifest with no such dependency, though the project said there was
    /// one, fails the run instead of writing as if it had been removed.
    #[test]
    fn a_dependency_that_is_not_in_the_manifest_fails_the_run() -> TestOutcome {
        let project = ScratchProject::new("dependency-missing")?;
        std::fs::write(
            &project.cli_manifest_path,
            CLI_MANIFEST.replace("lint.workspace = true\n", ""),
        )?;
        let before = project.snapshot()?;
        let mut removal = project.removal()?;

        let reported = fail_after_taking_out_a_missing_dependency(&mut removal);

        assert!(
            reported.contains("no dependency called `lint`"),
            "{reported}"
        );
        assert_eq!(project.snapshot()?, before);
        Ok(())
    }

    fn fail_after_taking_out_a_missing_dependency(removal: &mut Removal) -> String {
        let outcome = rollback::attempt(&retry("lint"), |changes| {
            removal.unlist(changes)?;
            removal.take_out_dependency(changes)
        });
        match outcome {
            Err(failure) => failure.to_string(),
            Ok(_) => unreachable!("the manifest has no such dependency"),
        }
    }

    /// With the composed CLI as the workspace root there is one manifest,
    /// and every edit must land in it. Two documents read from the one file
    /// would leave only the last one written.
    #[test]
    fn a_composed_cli_that_is_the_workspace_root_has_all_its_edits_in_one_file() -> TestOutcome {
        let project = ScratchProject::single_manifest("single-manifest")?;
        let mut removal = project.removal()?;

        let workspace_written = rollback::attempt("retry", |changes| {
            removal.unlist(changes)?;
            removal.take_out_dependency(changes)
        })?;

        assert!(
            !workspace_written,
            "there is no second manifest to have written"
        );
        assert_eq!(
            std::fs::read_to_string(&project.cli_manifest_path)?,
            "[package]\nname = \"demo-ritual\"\nversion = \"0.1.0\"\n\n[dependencies]\n\n\
             [package.metadata.ritual]\ntasks = [\n]\n\n\
             [workspace]\nmembers = [\n    \"ritual\",\n]\nresolver = \"3\"\n\n\
             [workspace.dependencies]\n"
        );
        Ok(())
    }

    #[test]
    fn the_report_names_each_file_once() {
        let lines = report_lines(
            std::path::Path::new("ritual/Cargo.toml"),
            &"updated ritual/src/main.rs (tasks: ritual)",
            Some(std::path::Path::new("Cargo.toml")),
        );
        assert_eq!(
            lines,
            [
                "updated ritual/Cargo.toml",
                "updated ritual/src/main.rs (tasks: ritual)",
                "updated Cargo.toml",
            ]
        );

        let without_workspace = report_lines(
            std::path::Path::new("Cargo.toml"),
            &"Cargo.toml is already up to date",
            None,
        );
        assert_eq!(
            without_workspace,
            ["updated Cargo.toml", "Cargo.toml is already up to date"]
        );
    }

    #[test]
    fn a_path_outside_the_workspace_is_reported_as_it_is() {
        assert_eq!(
            relative_to(
                std::path::Path::new("/elsewhere/Cargo.toml"),
                std::path::Path::new("/project")
            ),
            std::path::Path::new("/elsewhere/Cargo.toml")
        );
        assert_eq!(
            relative_to(
                std::path::Path::new("/project/ritual/Cargo.toml"),
                std::path::Path::new("/project")
            ),
            std::path::Path::new("ritual/Cargo.toml")
        );
    }

    /// The directory is deleted last, with everything in it.
    #[test]
    fn the_directory_is_deleted_with_everything_in_it() -> TestOutcome {
        let project = ScratchProject::new("delete")?;
        let removal = project.removal()?;

        removal.delete_the_directory()?;

        assert!(!project.workspace_root.join("tasks/lint").exists());
        assert!(
            project.workspace_root.join("ritual/Cargo.toml").is_file(),
            "nothing else is deleted"
        );
        Ok(())
    }

    #[test]
    fn a_key_with_no_member_deletes_nothing() -> TestOutcome {
        let project = ScratchProject::new("delete-nothing")?;
        let mut removal = project.removal()?;
        removal.member = None;
        let before = project.snapshot()?;

        removal.delete_the_directory()?;

        assert_eq!(project.snapshot()?, before);
        Ok(())
    }

    /// A directory that cannot be deleted must not be reported as put back:
    /// the manifests are already written by then, and the message has to say
    /// so and say how to get back what was deleted. A directory that is not
    /// there makes `remove_dir_all` fail without needing permissions.
    #[test]
    fn a_deletion_that_fails_says_the_manifests_are_updated_and_how_to_get_the_files_back()
    -> TestOutcome {
        let project = ScratchProject::new("delete-fails")?;
        std::fs::remove_dir_all(project.workspace_root.join("tasks/lint"))?;
        let removal = project.removal()?;

        let Err(failure) = removal.delete_the_directory() else {
            return Err("deleting a missing directory should fail".into());
        };

        let message = failure.to_string();
        assert!(message.contains("deleting tasks/lint failed"), "{message}");
        assert!(
            message.contains("the manifests are already updated"),
            "{message}"
        );
        assert!(
            message.contains("git checkout -- :/project/tasks/lint"),
            "{message}"
        );
        assert!(
            !message.contains("put the project back"),
            "a failed deletion must not claim the rollback ran: {message}"
        );
        Ok(())
    }

    #[test]
    fn the_deletion_failure_keeps_the_error_that_caused_it() {
        let failure = deletion_failure(
            "tasks/lint",
            std::path::Path::new("tasks/lint"),
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        );
        assert!(
            std::error::Error::source(&failure).is_some_and(|cause| cause.to_string() == "denied"),
            "the failure should carry its cause: {failure:?}"
        );
    }

    #[test]
    fn a_key_that_is_gone_is_accepted() {
        assert!(ensure_it_is_gone("lint", false, false).is_ok());
    }

    #[test]
    fn a_key_still_listed_or_still_a_dependency_is_a_failure_that_says_which() {
        let cases = [
            (
                true,
                false,
                "`lint` is still in [package.metadata.ritual] tasks after",
            ),
            (false, true, "`lint` is still a dependency after"),
            (
                true,
                true,
                "still in [package.metadata.ritual] tasks and still a dependency",
            ),
        ];
        for (listed, dependency, expected) in cases {
            let result = ensure_it_is_gone("lint", listed, dependency);
            assert!(
                matches!(&result, Err(failure) if failure.to_string().contains(expected)),
                "listed={listed} dependency={dependency}: {result:?}"
            );
        }
    }
}
