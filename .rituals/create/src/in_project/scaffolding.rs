//! Writing everything `create` has prepared inside a project.
//!
//! The run is one [`rollback::attempt`] that [`super::scaffold`] holds, so a
//! run that fails partway puts the project back as it found it.
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

use rituals::{CommandLine, Failure};
use rituals_compose::generated_file::{self, TaskKey};
use rituals_compose::layout::TaskPlace;
use rituals_compose::manifest::Manifests;
use rituals_compose::rollback::Changes;
use rituals_compose::source::Source;
use rituals_compose::task_crate::{self, Audience};

/// Everything `create` is about to write, captured before the first write.
pub(crate) struct Scaffolding {
    pub(crate) name: TaskKey,
    pub(crate) place: TaskPlace,
    pub(crate) audience: Audience,
    pub(crate) manifests: Manifests,
    pub(crate) dependency_path: String,
    pub(crate) workspace_root: PathBuf,
}

impl Scaffolding {
    /// Every write, in the order they are reported, and the lines that
    /// report them: the task crate's own two files, the manifests, then the
    /// regenerated file. Stops at the first failure.
    ///
    /// The lines come back rather than being printed as the run goes, so a
    /// failed run names nothing it put back. The task crate's directory is
    /// reserved before anything is written, and `.rituals/` with it when
    /// this is the project's first task, so a failed run removes both. Each
    /// manifest is written through `changes`, so a failed run puts its bytes
    /// back; the undo runs in the reverse order, so the manifests are
    /// restored before the directory is removed. A member entry pointing at
    /// a directory that is gone is a worse state than a directory nothing
    /// points at, so if the restore is the step that fails, the directory is
    /// still there. The generated file is regenerated last, inside the same
    /// run, so a regenerate that refuses leaves nothing half-written.
    pub(crate) fn write(
        &mut self,
        changes: &mut Changes,
        command_line: &CommandLine,
    ) -> Result<Vec<String>, Failure> {
        changes.reserve_directory(self.place.directory())?;
        let mut lines = self.write_task_crate()?.to_vec();
        lines.extend(self.write_manifests(changes)?);
        let regenerated =
            generated_file::regenerate_recording(changes, command_line, &self.workspace_root)?;
        lines.push(regenerated.to_string());
        Ok(lines)
    }

    /// The line `create` ends on: the file the new task's code goes in, and
    /// what a person types to run it. A task imported from the project's own
    /// manifest is always a top-level command under its own key.
    pub(crate) fn next_step(&self, binary_name: &str) -> String {
        let directory = self.place.from_the_root();
        let name = self.name.as_name();
        format!("next: edit {directory}/src/lib.rs, then run cargo {binary_name} {name}")
    }

    /// Writes the task crate's `Cargo.toml` and `src/lib.rs`, and returns the
    /// two lines that report them.
    fn write_task_crate(&self) -> Result<[String; 2], Failure> {
        let directory = self.place.from_the_root();
        let source_directory = self.place.directory().join("src");
        std::fs::create_dir_all(&source_directory).map_err(|error| {
            Failure::new(format!("creating {} failed", source_directory.display())).caused_by(error)
        })?;

        let manifest_path = self.place.directory().join("Cargo.toml");
        let manifest_text =
            task_crate::manifest(self.name.as_name(), &Source::Inherited, self.audience);
        std::fs::write(&manifest_path, manifest_text).map_err(|error| {
            Failure::new(format!("writing {} failed", manifest_path.display())).caused_by(error)
        })?;

        let lib_path = source_directory.join("lib.rs");
        std::fs::write(&lib_path, task_crate::lib(self.name.as_name())).map_err(|error| {
            Failure::new(format!("writing {} failed", lib_path.display())).caused_by(error)
        })?;

        Ok([
            format!("created {directory}/Cargo.toml"),
            format!("created {directory}/src/lib.rs"),
        ])
    }

    /// Adds the task to the workspace's members and to the command line
    /// crate's dependencies and task list, writes what changed, and returns
    /// the lines that report it.
    ///
    /// When the two manifests are one file it is written, and reported,
    /// once: [`Manifests::separate_workspace`] is `None` then.
    fn write_manifests(&mut self, changes: &mut Changes) -> Result<Vec<String>, Failure> {
        self.manifests
            .workspace_mut()
            .append_workspace_member(self.place.from_the_root())?;
        self.manifests
            .cli_mut()
            .import_task(&self.name, &self.dependency_path)?;

        let mut lines = Vec::new();
        if let Some(workspace) = self.manifests.separate_workspace() {
            workspace.write(changes)?;
            lines.push(updated(workspace.path(), &self.workspace_root));
        }
        self.manifests.cli().write(changes)?;
        lines.push(updated(self.manifests.cli().path(), &self.workspace_root));
        Ok(lines)
    }
}

/// The line that reports `path` as written, spelled from `workspace_root`.
fn updated(path: &Path, workspace_root: &Path) -> String {
    format!(
        "updated {}",
        path.strip_prefix(workspace_root).unwrap_or(path).display()
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::PathBuf;

    use rituals::{Failure, Name};
    use rituals_compose::generated_file::TaskKey;
    use rituals_compose::layout;
    use rituals_compose::manifest::{self, ManifestPaths, Manifests};
    use rituals_compose::rollback::{self, Changes, Wording};
    use rituals_compose::task_crate::Audience;

    use super::Scaffolding;
    use crate::test_support::{ScratchDir, TestOutcome};

    /// Every file under a directory tree, as a path paired with its bytes,
    /// sorted by path — a before/after diff for the undo to be checked
    /// against.
    type Snapshot = Vec<(PathBuf, Vec<u8>)>;

    const WORKSPACE_MANIFEST: &str =
        "[workspace]\nmembers = [\n    \"ritual\",\n]\nresolver = \"3\"\n";

    const CLI_MANIFEST: &str = "[package]\nname = \"demo-ritual\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
         [dependencies]\nrituals.workspace = true\n\n\
         [package.metadata.ritual]\ntasks = []\n";

    /// A scratch project: a workspace manifest with one member and a
    /// composed CLI manifest with no task imported yet, ready for the steps
    /// of `Scaffolding::write`, and the undo of a run that fails after
    /// them, to be exercised directly against real files.
    struct ScratchProject {
        _root: ScratchDir,
        paths: ManifestPaths,
        workspace_root: PathBuf,
    }

    impl ScratchProject {
        fn new(tag: &str) -> Result<Self, Box<dyn Error>> {
            let root = ScratchDir::new(tag)?;
            let workspace_root = root.path().to_path_buf();
            std::fs::create_dir_all(workspace_root.join("ritual/src"))?;

            let workspace = workspace_root.join("Cargo.toml");
            std::fs::write(&workspace, WORKSPACE_MANIFEST)?;
            let cli = workspace_root.join("ritual/Cargo.toml");
            std::fs::write(&cli, CLI_MANIFEST)?;

            Ok(Self {
                _root: root,
                paths: ManifestPaths { cli, workspace },
                workspace_root,
            })
        }

        /// A project whose composed CLI is the workspace root, so both
        /// manifests are one file.
        fn single_manifest(tag: &str) -> Result<Self, Box<dyn Error>> {
            let project = Self::new(tag)?;
            let combined = format!("{CLI_MANIFEST}\n{WORKSPACE_MANIFEST}");
            std::fs::write(&project.paths.workspace, combined)?;
            Ok(Self {
                paths: ManifestPaths {
                    cli: project.paths.workspace.clone(),
                    workspace: project.paths.workspace.clone(),
                },
                ..project
            })
        }

        fn scaffolding(
            &self,
            name: &str,
            audience: Audience,
        ) -> Result<Scaffolding, Box<dyn Error>> {
            let name = TaskKey::new(Name::new(name)?)?;
            let place = layout::place_for(&self.workspace_root, name.as_name());
            Ok(Scaffolding {
                dependency_path: manifest::dependency_path(&self.paths.cli, place.directory()),
                name,
                place,
                audience,
                manifests: Manifests::read(&self.paths)?,
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

    /// Runs `steps` — the first few of `Scaffolding::write`'s — inside the
    /// same rollback `scaffold` runs it in, with the same retry wording,
    /// then fails the way the next step would, and returns the failure the
    /// rollback reports. Asserts it is the simulated failure, so a step that
    /// failed is never mistaken for the one simulated after it.
    fn fail_after(
        scaffolding: &mut Scaffolding,
        steps: impl FnOnce(&mut Scaffolding, &mut Changes) -> Result<(), Failure>,
    ) -> Failure {
        let outcome =
            rollback::attempt(Wording::project("running `create lint` again"), |changes| {
                steps(scaffolding, changes)?;
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
    fn the_next_step_names_the_file_to_edit_and_the_command_that_runs_it() -> TestOutcome {
        let project = ScratchProject::new("next-step")?;
        let scaffolding = project.scaffolding("hello", Audience::Private)?;
        assert_eq!(
            scaffolding.next_step("acme"),
            "next: edit .rituals/hello/src/lib.rs, then run cargo acme hello"
        );
        Ok(())
    }

    /// The task crate's two files are reported as created, spelled from the
    /// workspace root, and the manifest says who the ritual is for.
    #[test]
    fn the_crate_is_written_for_its_audience_and_reported_from_the_root() -> TestOutcome {
        for (audience, publish_line) in [(Audience::Private, true), (Audience::Public, false)] {
            let project = ScratchProject::new("task-crate-audience")?;
            let scaffolding = project.scaffolding("lint", audience)?;
            std::fs::create_dir_all(scaffolding.place.directory())?;

            let lines = scaffolding.write_task_crate()?;

            assert_eq!(
                lines,
                [
                    "created .rituals/lint/Cargo.toml",
                    "created .rituals/lint/src/lib.rs"
                ]
            );
            let manifest =
                std::fs::read_to_string(scaffolding.place.directory().join("Cargo.toml"))?;
            assert_eq!(
                manifest.contains("publish = false\n"),
                publish_line,
                "{manifest}"
            );
            assert!(manifest.contains("rituals.workspace = true"), "{manifest}");
        }
        Ok(())
    }

    /// Two manifests are two writes, reported workspace first, as `add`
    /// always has, with both spelled from the workspace root.
    #[test]
    fn two_manifests_are_written_and_reported_workspace_first() -> TestOutcome {
        let project = ScratchProject::new("two-manifests")?;
        let mut scaffolding = project.scaffolding("lint", Audience::Private)?;

        let lines =
            rollback::attempt(Wording::project("running `create lint` again"), |changes| {
                scaffolding.write_manifests(changes)
            })?;

        assert_eq!(lines, ["updated Cargo.toml", "updated ritual/Cargo.toml"]);
        let workspace = std::fs::read_to_string(&project.paths.workspace)?;
        assert!(workspace.contains(".rituals/lint"), "{workspace}");
        let cli = std::fs::read_to_string(&project.paths.cli)?;
        assert!(
            cli.contains("lint = { path = \"../.rituals/lint\" }"),
            "the dependency must land: {cli}"
        );
        assert!(
            cli.contains("tasks = [\"lint\"]"),
            "the task must be listed: {cli}"
        );
        Ok(())
    }

    /// When the command line is the workspace root there is one document:
    /// both edits land in it, and the file is written and reported once.
    #[test]
    fn one_manifest_is_written_and_reported_once_with_both_edits() -> TestOutcome {
        let project = ScratchProject::single_manifest("one-manifest")?;
        let mut scaffolding = project.scaffolding("lint", Audience::Private)?;

        let lines =
            rollback::attempt(Wording::project("running `create lint` again"), |changes| {
                scaffolding.write_manifests(changes)
            })?;

        assert_eq!(lines, ["updated Cargo.toml"]);
        let root = std::fs::read_to_string(&project.paths.workspace)?;
        assert!(
            root.contains("\".rituals/lint\""),
            "the member entry must land: {root}"
        );
        assert!(
            root.contains("lint = { path = \".rituals/lint\" }"),
            "the dependency must land: {root}"
        );
        assert!(
            root.contains("tasks = [\"lint\"]"),
            "the task must be listed: {root}"
        );
        assert!(
            root.contains("[package]"),
            "the package table must survive: {root}"
        );
        Ok(())
    }

    #[test]
    fn undo_after_only_the_crate_directory_was_written() -> TestOutcome {
        let project = ScratchProject::new("undo-crate-only")?;
        let before = project.snapshot()?;

        let mut scaffolding = project.scaffolding("lint", Audience::Private)?;
        // Simulate the workspace-manifest write failing: nothing else has
        // happened yet.
        let reported = fail_after(&mut scaffolding, |scaffolding, changes| {
            changes.reserve_directory(scaffolding.place.directory())?;
            scaffolding.write_task_crate().map(drop)
        });

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
            !project.workspace_root.join(".rituals").exists(),
            ".rituals/ must be gone too"
        );
        Ok(())
    }

    #[test]
    fn undo_after_both_manifests_were_also_rewritten() -> TestOutcome {
        let project = ScratchProject::new("undo-manifests-too")?;
        let before = project.snapshot()?;

        let mut scaffolding = project.scaffolding("lint", Audience::Private)?;
        let reported = fail_after(&mut scaffolding, |scaffolding, changes| {
            changes.reserve_directory(scaffolding.place.directory())?;
            scaffolding.write_task_crate()?;
            scaffolding.write_manifests(changes).map(drop)
        });

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

    /// The same promise when the two manifests are one file: the single
    /// write is put back.
    #[test]
    fn undo_after_the_one_manifest_was_rewritten() -> TestOutcome {
        let project = ScratchProject::single_manifest("undo-one-manifest")?;
        let before = project.snapshot()?;

        let mut scaffolding = project.scaffolding("lint", Audience::Public)?;
        let _ = fail_after(&mut scaffolding, |scaffolding, changes| {
            changes.reserve_directory(scaffolding.place.directory())?;
            scaffolding.write_task_crate()?;
            scaffolding.write_manifests(changes).map(drop)
        });

        assert_eq!(project.snapshot()?, before);
        Ok(())
    }

    #[test]
    fn undo_leaves_the_tasks_directory_when_create_did_not_make_it() -> TestOutcome {
        let project = ScratchProject::new("undo-preexisting-tasks")?;
        std::fs::create_dir_all(project.workspace_root.join(".rituals"))?;
        std::fs::write(
            project.workspace_root.join(".rituals/.keep"),
            "a file that predates this create run\n",
        )?;

        let mut scaffolding = project.scaffolding("lint", Audience::Private)?;
        let _ = fail_after(&mut scaffolding, |scaffolding, changes| {
            changes.reserve_directory(scaffolding.place.directory())?;
            scaffolding.write_task_crate().map(drop)
        });

        assert!(
            project.workspace_root.join(".rituals/.keep").is_file(),
            ".rituals/ must survive when create did not make it"
        );
        assert!(
            !scaffolding.place.directory().exists(),
            "the crate directory create did make must still go"
        );
        Ok(())
    }

    /// A directory without write permission refuses to have entries removed
    /// from it — that is what write permission controls on a directory, not
    /// on the entries inside it — so `chmod 0o555` on the crate directory
    /// makes `remove_dir_all` fail without needing anything to hold the
    /// directory open. Verifies the undo reports the directory by name and
    /// leaves it on disk rather than claiming full restoration.
    ///
    /// Root ignores a directory's own write bit, so as uid 0 the removal this
    /// test relies on failing would succeed instead, and the second
    /// `set_permissions` below would fail with `NotFound` on a directory
    /// the undo had already removed. A probe write right after the
    /// `chmod` tells the two cases apart, so a run under that condition
    /// reports "could not demonstrate" instead of a false pass or a spurious
    /// failure.
    #[test]
    fn undo_reports_the_crate_directory_when_removing_it_fails() -> TestOutcome {
        use std::os::unix::fs::PermissionsExt;

        let project = ScratchProject::new("undo-crate-removal-fails")?;
        let mut scaffolding = project.scaffolding("lint", Audience::Private)?;
        let mut permission_is_enforced = true;

        let reported = fail_after(&mut scaffolding, |scaffolding, changes| {
            changes.reserve_directory(scaffolding.place.directory())?;
            scaffolding.write_task_crate()?;

            let setup = |error| Failure::new("changing permissions failed").caused_by(error);
            std::fs::set_permissions(
                scaffolding.place.directory(),
                std::fs::Permissions::from_mode(0o555),
            )
            .map_err(setup)?;

            // Root ignores a directory's missing write bit entirely, so a
            // probe write tells whether this process is actually subject to
            // the permission above. If it is not, the removal the undo makes
            // would succeed despite the `0o555`, and the scenario this test
            // demonstrates cannot occur for this user: restore before the
            // undo runs, so a skipped check says so rather than passing
            // silently.
            let probe = scaffolding.place.directory().join("probe");
            permission_is_enforced = std::fs::File::create(&probe).is_err();
            if !permission_is_enforced {
                let _ = std::fs::remove_file(&probe);
                std::fs::set_permissions(
                    scaffolding.place.directory(),
                    std::fs::Permissions::from_mode(0o755),
                )
                .map_err(setup)?;
            }
            Ok(())
        });

        if !permission_is_enforced {
            crate::test_support::report_skip(
                "undo_reports_the_crate_directory_when_removing_it_fails could \
                     not demonstrate a permission-denied removal because this process does \
                     not honour directory write permissions",
            );
            return Ok(());
        }

        // Restore permissions before any assertion can return early, so the
        // scratch directory this test made is still removable on drop
        // whether or not the assertions below pass.
        std::fs::set_permissions(
            scaffolding.place.directory(),
            std::fs::Permissions::from_mode(0o755),
        )?;

        assert!(
            reported
                .to_string()
                .contains(&scaffolding.place.directory().display().to_string()),
            "expected the message to name the directory that could not be removed: {reported}"
        );
        assert!(
            scaffolding.place.directory().exists(),
            "the directory undo could not remove must still be there"
        );
        Ok(())
    }
}
