//! Writing what `import` has prepared, and putting the project back if a
//! step fails.
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
use rituals_compose::generated_file::TaskKey;
use rituals_compose::manifest::Manifest;
use rituals_compose::rollback::Changes;
use rituals_compose::{generated_file, metadata, top_level};

use crate::cargo_add;

/// Everything `import` is about to do, captured before the first write.
///
/// [`Import::run`] runs inside the [`rollback::attempt`] that began at the
/// first subprocess, so a run that is refused or fails partway puts both
/// files `cargo add` can change, the command line crate's manifest and the
/// workspace's `Cargo.lock`, back byte for byte, and says so.
pub(crate) struct Import {
    pub(crate) package: String,
    pub(crate) key: TaskKey,
    pub(crate) current_dir: PathBuf,
    pub(crate) workspace_root: PathBuf,
    pub(crate) cli_manifest_path: PathBuf,
    pub(crate) lockfile_path: PathBuf,
    pub(crate) cargo_add_arguments: Vec<String>,
}

impl Import {
    /// The steps, in order: `cargo add`, whether what it added is a task, and
    /// the key joining the task list. Stops at the first failure.
    ///
    /// Both files `cargo add` can change are recorded before it runs, and
    /// the task check and the append come after it, so a refusal at either
    /// has something to put back. The manifest is read again after `cargo
    /// add`, which has rewritten it, and written through `changes` like any
    /// other. Reports each file changed, once all three steps have gone
    /// through.
    ///
    /// Whether the lockfile is reported as created is asked of `changes`,
    /// which recorded it before the first `cargo metadata` wrote it: asking
    /// the disk here would find the one that `cargo metadata` made.
    pub(crate) fn run(&self, changes: &mut Changes) -> Outcome {
        changes.run_changing(&[&self.cli_manifest_path, &self.lockfile_path], || {
            cargo_add::run(&self.current_dir, &self.cargo_add_arguments)
        })?;

        // A fresh `cargo metadata`, because what a dependency declares about
        // itself is only known once `cargo add` has resolved it, wherever it
        // came from. Its refusal is returned as it is: it already says what
        // is wrong and what to do, and the rollback adds that nothing changed.
        // The project is known to be this one by now, so there is nothing
        // more to check about where it runs.
        metadata::fetch_recording(changes, &self.current_dir)?
            .ensure_dependency_is_a_task(&self.package, self.key.as_name())?;

        let mut cli_manifest = Manifest::read(&self.cli_manifest_path)?;
        cli_manifest.append_task(&self.key)?;
        cli_manifest.write(changes)?;

        for line in reported_lines(
            &self.cli_manifest_path,
            &self.workspace_root,
            if changes.recorded_as_absent(&self.lockfile_path) {
                LockfileChange::Created
            } else {
                LockfileChange::Updated
            },
        ) {
            report(line);
        }
        Ok(())
    }
}

/// What a person runs again once they have checked whatever a failed run
/// could not put back, for the words `again` that run the import.
pub(crate) fn retry(again: &str) -> String {
    format!("running `import {again}` again")
}

/// `import` has no idea of the task list of its own: it writes the manifest
/// and then runs the regenerate path, so the two can never drift. Ends by
/// naming the command that runs what was imported. Runs after the attempt
/// that wrote the manifest has committed, so a failure here leaves the task
/// imported and says what finishes it.
pub(crate) fn finish_by_regenerating(command_line: &CommandLine, import: &Import) -> Outcome {
    generated_file::regenerate(command_line).map_err(|failure| {
        imported_but_not_regenerated(
            &failure,
            import.key.as_name(),
            &top_level::management_command(command_line, "regenerate"),
        )
    })?;
    report(next_step(
        import.key.as_name(),
        command_line.identity().binary_name(),
    ));
    Ok(())
}

/// What `import` did to the lockfile: `cargo add` made it when the project
/// had none, and otherwise changed the one it had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LockfileChange {
    Created,
    Updated,
}

/// The lines `import` reports once it has written: the manifest it appended
/// to, then the lockfile.
fn reported_lines(
    cli_manifest_path: &Path,
    workspace_root: &Path,
    lockfile: LockfileChange,
) -> [String; 2] {
    let manifest = cli_manifest_path
        .strip_prefix(workspace_root)
        .unwrap_or(cli_manifest_path);
    let lockfile = match lockfile {
        LockfileChange::Created => "created Cargo.lock",
        LockfileChange::Updated => "updated Cargo.lock",
    };
    [
        format!("updated {}", manifest.display()),
        lockfile.to_string(),
    ]
}

/// The failure for a run whose writes all went through but whose regenerate
/// did not: the task is imported, so what is left is the one command that
/// finishes it.
fn imported_but_not_regenerated(failure: &Failure, key: &Name, regenerate: &str) -> Failure {
    Failure::new(format!(
        "{failure}; `{key}` is imported — run `{regenerate}` to finish"
    ))
}

/// The line `import` ends on: what a person types to run the task it
/// imported, which is always a top-level command under its own key.
fn next_step(key: &Name, binary_name: &str) -> String {
    format!("next: run cargo {binary_name} {key}")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rituals::{Failure, Name};
    use rituals_compose::generated_file::TaskKey;
    use rituals_compose::rollback::{self, Wording};

    use super::{
        Import, LockfileChange, imported_but_not_regenerated, next_step, reported_lines, retry,
    };
    use crate::cargo_add;
    use crate::test_support::{RITUALS_DEPENDENCY, ScratchProject, TestOutcome, typed_arguments};

    fn valid_name(value: &str) -> Name {
        Name::new(value).expect("a test passes only names it knows are valid")
    }

    #[test]
    fn the_next_step_names_the_command_that_runs_the_imported_task() {
        assert_eq!(
            next_step(&valid_name("hail"), "acme"),
            "next: run cargo acme hail"
        );
    }

    #[test]
    fn a_failed_regenerate_says_the_task_is_imported_and_what_finishes_it() {
        let failure = imported_but_not_regenerated(
            &Failure::new("cargo metadata failed: no space left"),
            &valid_name("hail"),
            "cargo ritual regenerate",
        );

        assert_eq!(
            failure.to_string(),
            "cargo metadata failed: no space left; `hail` is imported — run `cargo ritual \
             regenerate` to finish"
        );
    }

    #[test]
    fn the_reported_lines_name_the_manifest_relative_to_the_workspace_and_the_lockfile() {
        let root = Path::new("/work/demo");
        let manifest = root.join("ritual/Cargo.toml");

        assert_eq!(
            reported_lines(&manifest, root, LockfileChange::Updated),
            ["updated ritual/Cargo.toml", "updated Cargo.lock"]
        );
        assert_eq!(
            reported_lines(&manifest, root, LockfileChange::Created),
            ["updated ritual/Cargo.toml", "created Cargo.lock"]
        );
    }

    /// An `Import` of `key` for `words` typed as `import`'s arguments, run
    /// from the scratch project's CLI directory.
    fn import_in(
        project: &ScratchProject,
        key: &str,
        words: &[&str],
    ) -> Result<Import, Box<dyn std::error::Error>> {
        let key = TaskKey::new(Name::new(key)?)?;
        Ok(Import {
            package: "demo-ritual".to_string(),
            current_dir: project.cli_dir(),
            workspace_root: project.root().to_path_buf(),
            cli_manifest_path: project.cli_manifest_path(),
            lockfile_path: project.lockfile_path(),
            cargo_add_arguments: cargo_add::arguments(
                "demo-ritual",
                key.as_name(),
                &typed_arguments(words),
            ),
            key,
        })
    }

    fn run_in_attempt(import: &Import) -> Result<(), Failure> {
        rollback::attempt(Wording::project(&retry("greeter")), |changes| {
            import.run(changes)
        })
    }

    #[test]
    fn the_retry_names_import_and_the_words_that_run_it_again() {
        assert_eq!(
            retry("greeter --path /w/greeter"),
            "running `import greeter --path /w/greeter` again"
        );
    }

    #[test]
    fn a_task_crate_is_added_and_its_key_joins_the_task_list() -> TestOutcome {
        let project = ScratchProject::new("run-adds-a-task")?;
        project.write_crate("greeter", true)?;
        let import = import_in(
            &project,
            "hail",
            &["greeter", "hail", "--path", "../greeter"],
        )?;
        assert!(
            !project.lockfile_path().exists(),
            "this story starts with no lockfile, so cargo has one to make"
        );

        run_in_attempt(&import)?;

        let manifest = std::fs::read_to_string(project.cli_manifest_path())?;
        assert!(
            manifest.contains("hail = {"),
            "expected a dependency under the key; manifest was:\n{manifest}"
        );
        assert!(
            manifest.contains("package = \"greeter\""),
            "expected the dependency to be the crate renamed; manifest was:\n{manifest}"
        );
        assert!(
            manifest.contains("tasks = [\"hail\"]"),
            "expected the key in the task list; manifest was:\n{manifest}"
        );
        assert!(project.lockfile_path().exists(), "cargo wrote a lockfile");
        Ok(())
    }

    /// The run is refused or fails, and every file is exactly what it was.
    /// Returns the failure's message, as the run reports it, for the caller
    /// to say what it was refused for and whether the project was put back.
    fn refused_with_every_file_as_it_was(
        project: &ScratchProject,
        import: &Import,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let before = project.snapshot()?;

        let outcome = run_in_attempt(import);

        assert_eq!(project.snapshot()?, before, "every file must be as it was");
        let failure = outcome.err().ok_or("expected the run to be refused")?;
        Ok(failure.to_string())
    }

    /// The common shape of the tests below: the run is refused or fails
    /// after changing something, the failure says the project was put back,
    /// and every file is exactly what it was. Returns the failure's message,
    /// for the caller to say what it was refused for.
    fn refused_and_put_back(
        project: &ScratchProject,
        import: &Import,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let message = refused_with_every_file_as_it_was(project, import)?;
        assert!(
            message.ends_with("; ritual put the project back as it found it"),
            "expected the rollback's report; got: {message}"
        );
        Ok(message)
    }

    #[test]
    fn a_crate_that_is_not_a_task_is_refused_and_everything_is_put_back() -> TestOutcome {
        let project = ScratchProject::new("run-not-a-task")?;
        project.write_crate("plain", false)?;
        let import = import_in(&project, "plain", &["plain", "--path", "../plain"])?;

        let message = refused_and_put_back(&project, &import)?;

        assert!(
            message.starts_with("`plain` is not a task crate"),
            "expected the task check's refusal; got: {message}"
        );
        Ok(())
    }

    #[test]
    fn a_lockfile_that_was_there_is_put_back_byte_for_byte() -> TestOutcome {
        // A genuine lockfile, made by cargo, for `cargo add` to rewrite.
        let project = ScratchProject::new("run-lockfile-restored")?.with_a_lockfile()?;
        project.write_crate("plain", false)?;
        let before = std::fs::read(project.lockfile_path())?;
        let import = import_in(&project, "plain", &["plain", "--path", "../plain"])?;

        let message = refused_and_put_back(&project, &import)?;

        assert!(
            message.starts_with("`plain` is not a task crate"),
            "expected the task check's refusal, which is after cargo changed the lockfile; \
             got: {message}"
        );
        assert_eq!(std::fs::read(project.lockfile_path())?, before);
        Ok(())
    }

    #[test]
    fn a_cargo_add_that_fails_leaves_everything_as_it_was() -> TestOutcome {
        let project = ScratchProject::new("run-cargo-add-fails")?;
        let import = import_in(&project, "ghost", &["ghost", "--path", "../ghost"])?;

        let message = refused_with_every_file_as_it_was(&project, &import)?;

        assert!(
            message.starts_with("cargo add failed: "),
            "expected cargo's own failure; got: {message}"
        );
        assert!(
            !message.contains("put the project back"),
            "cargo add changed nothing before it failed, so nothing was put back; got: \
             {message}"
        );
        Ok(())
    }

    /// The task list is missing in a manifest this run is handed, which
    /// `prepare` refuses before anything is written, so this reaches the
    /// append only by being given such a project directly. The refusal is
    /// the method's own, and what `cargo add` had already changed goes back.
    #[test]
    fn an_append_that_is_refused_puts_back_what_cargo_add_changed() -> TestOutcome {
        let project = ScratchProject::new("run-append-refused")?;
        project.write_crate("greeter", true)?;
        std::fs::write(
            project.cli_manifest_path(),
            format!(
                "[package]\nname = \"demo-ritual\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
                 [dependencies]\n{RITUALS_DEPENDENCY}\n\n[package.metadata.ritual]\ntask = true\n"
            ),
        )?;
        let import = import_in(&project, "greeter", &["greeter", "--path", "../greeter"])?;

        let message = refused_and_put_back(&project, &import)?;

        assert!(
            message.contains("has no [package.metadata.ritual] tasks list to add `greeter` to"),
            "expected the append's own refusal; got: {message}"
        );
        Ok(())
    }
}
