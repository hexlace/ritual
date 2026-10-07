//! `create` where a crate builds on its own, outside any project: scaffold a
//! task crate on its own, in the current directory, for a project to import
//! later.
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

use std::path::Path;

use rituals::{Failure, Name, Outcome, report};
use rituals_compose::rollback::{self, Wording};
use rituals_compose::shell;
use rituals_compose::source::{Source, SourceArguments, ensure_a_ritual_checkout};
use rituals_compose::task_crate::Audience;

use crate::arguments::{NameOrPath, ScaffoldArguments};
use crate::crate_files;

/// Scaffolds the task crate `arguments` names in `current_dir`, taking
/// `rituals` from `source`, and ends on the `import` command that brings it
/// into the project that will use it.
///
/// Outside a project there is no `.rituals/` to place a path in, so only a
/// bare name is taken: the crate is made in the current directory.
pub(crate) fn scaffold(
    arguments: &ScaffoldArguments,
    source: &SourceArguments,
    current_dir: &Path,
) -> Outcome {
    let name = match arguments.name_or_path() {
        NameOrPath::Name(text) => Name::new(text)?,
        NameOrPath::Path(path) => return Err(path_refusal(path)),
    };
    let source = validate_source(source.resolve())?;

    let target_dir = current_dir.join(name.as_str());
    if target_dir.exists() {
        return Err(Failure::new(format!(
            "refusing to create {name}: it already exists"
        )));
    }

    // The directory is spelled as typed in the report, because that is what
    // a person looks for next to where they ran `create`.
    let retry = format!("running `create {name}` again");
    rollback::attempt(
        Wording::fresh_directory(current_dir, Path::new(name.as_str()), &retry),
        |changes| {
            changes.reserve_directory(&target_dir)?;
            write_crate(&target_dir, &name, &source, arguments.audience())
        },
    )?;

    report(next_step(&name, &target_dir));
    Ok(())
}

/// The refusal for a path, which has nowhere to go outside a project.
fn path_refusal(typed: &Path) -> Failure {
    Failure::new(format!(
        "refusing to create {}: outside a project, create makes the crate in the current \
         directory and takes a bare name, not a path",
        typed.display()
    ))
}

/// The line `create` ends on: the `import` command that brings the new
/// crate into the project that will use it.
///
/// The path is absolute so the command works from any project on this
/// machine, and the whole command is rendered by [`shell::join`] so a path
/// with a space in it still pastes.
fn next_step(name: &Name, crate_dir: &Path) -> String {
    let path = crate_dir.to_string_lossy();
    let import = shell::join(["import", name.as_str(), "--path", &path]);
    // `create` runs outside any project, so it cannot know what the
    // importing project calls its command line: both spellings, as the
    // refusals give them. The place comes first because that is the order a
    // person acts in.
    format!(
        "next: in the project that will use it, run cargo ritual {import} (or cargo <name> \
         ritual {import} if it was made with --cli <name>)"
    )
}

/// Writes every file `create` scaffolds into `target_dir`, which the caller
/// has already reserved and refused to reuse, and reports each as created
/// under `name`, as typed. Stops at the first failure — the rollback
/// [`scaffold`] holds removes `target_dir` wholesale on one, rather than this
/// function trying to undo file by file.
fn write_crate(target_dir: &Path, name: &Name, source: &Source, audience: Audience) -> Outcome {
    for file in crate_files::write(target_dir, name, source, audience)? {
        report(format!("created {name}/{file}"));
    }
    Ok(())
}

/// Turns a `--path` source into the absolute path of the `rituals`
/// crate directory inside that checkout, after checking the checkout is a
/// real one; passes a registry or `--git` source through unchanged.
///
/// # Errors
///
/// Returns a [`Failure`] naming the given `--path` when it does not contain
/// `rituals_compose::source::RITUALS_MANIFEST_IN_CHECKOUT`.
fn validate_source(source: Source) -> Result<Source, Failure> {
    let Source::Path(checkout_root) = source else {
        return Ok(source);
    };

    ensure_a_ritual_checkout(&checkout_root)?;

    let absolute_checkout_root = std::path::absolute(&checkout_root).map_err(|error| {
        Failure::new(format!("resolving {} failed", checkout_root.display())).caused_by(error)
    })?;

    Ok(Source::Path(absolute_checkout_root.join("crates/rituals")))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rituals::{Failure, Name};
    use rituals_compose::rollback::{self, Wording};
    use rituals_compose::source::Source;
    use rituals_compose::task_crate::Audience;

    use super::{next_step, path_refusal, validate_source, write_crate};
    use crate::test_support::{ScratchDir, TestOutcome, report_skip};

    fn demo_name() -> Name {
        Name::new("demo").expect("demo is a valid name")
    }

    #[test]
    fn the_next_step_is_the_import_command_to_run_in_the_project_that_will_use_it() {
        let name = Name::new("lint").expect("lint is a valid name");
        assert_eq!(
            next_step(&name, Path::new("/work/lint")),
            "next: in the project that will use it, run cargo ritual import lint --path \
             /work/lint (or cargo <name> ritual import lint --path /work/lint if it was \
             made with --cli <name>)"
        );
    }

    /// The path is typed into a shell, so one with a space in it is quoted
    /// the way the shell will read it back.
    #[test]
    fn a_path_with_a_space_is_quoted_so_the_command_pastes_as_written() {
        let name = Name::new("lint").expect("lint is a valid name");
        assert_eq!(
            next_step(&name, Path::new("/my work/lint")),
            "next: in the project that will use it, run cargo ritual import lint --path \
             '/my work/lint' (or cargo <name> ritual import lint --path '/my work/lint' if \
             it was made with --cli <name>)"
        );
    }

    #[test]
    fn a_path_is_refused_naming_what_was_typed_and_what_to_give_instead() {
        assert_eq!(
            path_refusal(Path::new("private/lint")).to_string(),
            "refusing to create private/lint: outside a project, create makes the crate in \
             the current directory and takes a bare name, not a path"
        );
    }

    #[test]
    fn a_path_missing_the_core_manifest_is_refused_naming_the_manifest_it_expects() {
        let result = validate_source(Source::Path(std::env::temp_dir()));
        assert!(result.is_err(), "expected a bad --path to be refused");
        if let Err(error) = result {
            let message = error.to_string();
            assert!(message.to_lowercase().contains("ritual checkout"));
            assert!(message.contains("crates/rituals/Cargo.toml"));
        }
    }

    #[test]
    fn a_git_source_passes_through_unchanged() {
        let result = validate_source(Source::Git("https://example.invalid/x".to_string()));
        assert!(
            result.is_ok(),
            "expected a --git source to pass through: {result:?}"
        );
        if let Ok(source) = result {
            assert_eq!(source, Source::Git("https://example.invalid/x".to_string()));
        }
    }

    /// Runs `create`'s write inside the rollback it runs in: reserves
    /// `target_dir`, makes a directory where `Cargo.toml` is to go, so the
    /// first file write fails with `EISDIR` the way a real write can fail
    /// partway through scaffolding, and then lets `after_failed_write` act
    /// before the rollback undoes the run.
    fn failed_create(
        target_dir: &Path,
        after_failed_write: impl FnOnce() -> std::io::Result<()>,
    ) -> Result<(), Failure> {
        let name = demo_name();
        let retry = "running `create demo` again";
        let current_dir = target_dir
            .parent()
            .ok_or_else(|| Failure::new("a scratch directory is inside another"))?;
        rollback::attempt(
            Wording::fresh_directory(current_dir, Path::new("demo"), retry),
            |changes| {
                changes.reserve_directory(target_dir)?;
                std::fs::create_dir(target_dir.join("Cargo.toml")).map_err(|error| {
                    Failure::new("making the poisoned manifest path failed").caused_by(error)
                })?;
                let written = write_crate(target_dir, &name, &Source::Inherited, Audience::Private);
                after_failed_write().map_err(|error| Failure::new("setup").caused_by(error))?;
                written
            },
        )
    }

    #[test]
    fn a_write_failure_removes_the_root_and_names_the_path_in_the_report() -> TestOutcome {
        let scratch = ScratchDir::new("write-failure")?;
        let target_dir = scratch.path().join("demo");

        let outcome = failed_create(&target_dir, || Ok(()));

        let failure = outcome
            .err()
            .ok_or("expected the poisoned manifest path to fail the write")?;
        let message = failure.to_string();
        assert!(
            message.contains(&target_dir.join("Cargo.toml").display().to_string()),
            "expected the message to name the path that failed to write: {message}"
        );
        assert!(
            message.ends_with("; ritual removed demo so a retry starts clean"),
            "expected the message to say the root was removed: {message}"
        );
        assert!(
            !target_dir.exists(),
            "the root must be gone after a successful removal"
        );
        Ok(())
    }

    /// A directory without write permission refuses to have entries removed
    /// from it: `chmod 0o555` on `target_dir` after the poisoned write has
    /// already failed makes the rollback's removal fail in turn, with no
    /// need for anything to hold the directory open.
    #[test]
    fn a_write_failure_reports_the_root_when_removal_also_fails() -> TestOutcome {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::new("write-failure-unremovable")?;
        let target_dir = scratch.path().join("demo");
        let mut permission_is_enforced = false;

        let outcome = failed_create(&target_dir, || {
            std::fs::set_permissions(&target_dir, std::fs::Permissions::from_mode(0o555))?;
            // Root ignores a directory's missing write bit, so this probe
            // tells whether the permission above actually blocks this
            // process.
            permission_is_enforced = std::fs::File::create(target_dir.join("probe")).is_err();
            Ok(())
        });

        // A process that ignores the write bit, such as root, removed the
        // directory in the undo, so there is nothing to restore and nothing
        // to show.
        if !permission_is_enforced {
            report_skip(
                "a_write_failure_reports_the_root_when_removal_also_fails could \
                     not demonstrate a permission-denied removal because this process does \
                     not honour directory write permissions",
            );
            return Ok(());
        }

        // Restore permissions before any assertion can return early, so the
        // scratch directory this test made is still removable on drop
        // whether or not the assertions below pass.
        std::fs::set_permissions(&target_dir, std::fs::Permissions::from_mode(0o755))?;

        let failure = outcome
            .err()
            .ok_or("expected the poisoned manifest path to fail the write")?;
        let message = failure.to_string();
        assert!(
            message.ends_with(
                "; ritual could not remove demo — check it before running `create demo` again"
            ),
            "expected the message to name the root that could not be removed, as typed: \
             {message}"
        );
        assert!(
            target_dir.exists(),
            "the root the rollback could not remove must still be there"
        );
        Ok(())
    }
}
