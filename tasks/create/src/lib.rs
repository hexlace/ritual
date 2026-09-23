//! The `create` task: scaffold a task crate on its own, for a project to
//! import later — no enclosing Cargo workspace required, and refused if one
//! is found anyway.

use std::path::Path;

use rituals::{Failure, Name, Outcome, Task, clap, report};
use rituals_compose::source::{
    Source, SourceArguments, assert_is_a_ritual_checkout, escape_toml_string,
};
use rituals_compose::{task_crate, workspace};

/// `create`'s arguments: the name of the crate to scaffold, and where
/// ritual's own crates come from.
#[derive(clap::Args)]
struct Arguments {
    /// the task crate directory to create, in the working directory
    name: String,

    #[command(flatten)]
    source: SourceArguments,
}

/// This task, for a command line to mount under whatever name imports it.
///
/// Scaffolds a task crate that stands alone, outside any project, for a
/// project to import later — the one thing `add`, which only ever scaffolds
/// into the project it runs in, cannot do.
//
// No `# Examples` section: a `task()` function has exactly one call-site
// shape — a mount line in a generated file — and an example here could only
// restate `let task = create::task();`, which shows nothing a reader needs.
#[must_use]
pub fn task() -> Task {
    Task::new(
        "scaffold a task crate on its own, for a project to import later",
        |arguments: Arguments| run(&arguments),
    )
}

fn run(arguments: &Arguments) -> Outcome {
    let name = Name::new(&arguments.name)?;
    let source = arguments.source.resolve();
    let source = validate_source(source)?;

    let current_dir = std::env::current_dir()
        .map_err(|error| Failure::new("reading the current directory failed").caused_by(error))?;
    workspace::ensure_the_directory_stands_alone(&current_dir, &refusal(&name))?;

    let target_dir = current_dir.join(name.as_str());
    if target_dir.exists() {
        return Err(Failure::new(format!(
            "refusing to create {name}: it already exists"
        )));
    }

    std::fs::create_dir(&target_dir).map_err(|error| {
        Failure::new(format!("creating {} failed", target_dir.display())).caused_by(error)
    })?;

    if let Err(failure) = write_crate(&target_dir, &name, &source) {
        return Err(undo(&target_dir, &name, &failure));
    }

    for line in next_steps(&name, &target_dir) {
        report(line);
    }

    Ok(())
}

/// The lines `create` ends on: the dependency line that imports the new
/// crate, on a line of its own so it pastes as TOML, and the one edit
/// after it.
///
/// The path is absolute so the line works from any project on this machine;
/// a crate moved into a shared repository changes only that field. A path
/// that is not valid UTF-8 cannot be written in a Cargo manifest at all, so
/// then the line says that instead of printing one that points nowhere.
//
// No `[dependencies]` header line: the manifest the line goes into already
// has that table, and pasting a second header would make it invalid.
fn next_steps(name: &Name, crate_dir: &Path) -> Vec<String> {
    let Some(path) = crate_dir.to_str() else {
        return vec![format!(
            "next: {name} was created, but its path is not valid UTF-8, so no Cargo manifest \
             can name it; move it to a path that is before importing it"
        )];
    };
    vec![
        "next: to import it, add this under [dependencies] in a project's ritual/Cargo.toml:"
            .to_string(),
        format!("{name} = {{ path = \"{}\" }}", escape_toml_string(path)),
        // `create` runs outside any project, so it cannot know what the
        // importing project calls its command line: both spellings, as the
        // refusals give them.
        format!(
            "then add \"{name}\" to [package.metadata.ritual] tasks and run \
             cargo ritual regenerate there (or cargo <name> ritual regenerate if it was made \
             with --cli <name>)"
        ),
    ]
}

/// Writes every file `create` scaffolds into `target_dir`, which the caller
/// has already created and refused to reuse. Stops at the first failure —
/// [`run`] removes `target_dir` wholesale on one, rather than this function
/// trying to undo file by file.
fn write_crate(target_dir: &Path, name: &Name, source: &Source) -> Outcome {
    let source_directory = target_dir.join("src");
    std::fs::create_dir_all(&source_directory).map_err(|error| {
        Failure::new(format!("creating {} failed", source_directory.display())).caused_by(error)
    })?;

    let manifest_path = target_dir.join("Cargo.toml");
    std::fs::write(&manifest_path, task_crate::manifest(name, source)).map_err(|error| {
        Failure::new(format!("writing {} failed", manifest_path.display())).caused_by(error)
    })?;
    report(format!("created {name}/Cargo.toml"));

    let lib_path = source_directory.join("lib.rs");
    std::fs::write(&lib_path, task_crate::lib(name)).map_err(|error| {
        Failure::new(format!("writing {} failed", lib_path.display())).caused_by(error)
    })?;
    report(format!("created {name}/src/lib.rs"));

    Ok(())
}

/// Removes `target_dir` after [`write_crate`] fails partway, and folds the
/// removal's own outcome into the refusal — parity with `add`'s own undo:
/// a run that does not finish leaves nothing behind, and says whether it
/// managed to.
///
/// `failure`'s own `Display` already carries its attached cause, so
/// formatting `{failure}` directly here is enough — this needs no local
/// rendering of its own.
fn undo(target_dir: &Path, name: &Name, failure: &Failure) -> Failure {
    match std::fs::remove_dir_all(target_dir) {
        Ok(()) => Failure::new(format!(
            "{failure}; ritual removed {name} so a retry starts clean"
        )),
        Err(_removal_error) => Failure::new(format!(
            "{failure}; ritual could not remove {name} — check it before running `create \
             {name}` again"
        )),
    }
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

    assert_is_a_ritual_checkout(&checkout_root)?;

    let absolute_checkout_root = std::path::absolute(&checkout_root).map_err(|error| {
        Failure::new(format!("resolving {} failed", checkout_root.display())).caused_by(error)
    })?;

    Ok(Source::Path(absolute_checkout_root.join("crates/rituals")))
}

/// What `create` puts into whichever refusal
/// [`workspace::ensure_the_directory_stands_alone`] produces.
///
/// `name` is the input a person already typed, already validated by
/// [`Name::new`] before this is called. The wording belongs to `create`
/// rather than to the shared check because the situation the check decides
/// is the same for every caller but what a person should do about it is
/// not: `create`'s crate is a `cargo new --lib` crate plus the mark and the
/// dependency, so it needs an enclosing project that already imports tasks
/// to hand it to — and that project's own command line may be named
/// anything, so the remedy names its own `add` rather than any particular
/// binary, which would only be true in one project's alias.
fn refusal(name: &Name) -> workspace::Refusal {
    workspace::Refusal {
        attempted_command: format!("create {name}"),
        why_not_here: "a crate scaffolded there does not build on its own".to_string(),
        what_to_do_instead: format!(
            "run create outside any Cargo workspace, or, if the enclosing project is a \
             ritual project, add the task with that project's own `add {name}`"
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// Says, on the test process's own stderr, that a check was skipped and
    /// why. Written straight to the stream rather than through `eprintln!`,
    /// which the test harness captures and discards for a passing test —
    /// where a skip would read exactly like a pass.
    fn report_skip(message: &str) {
        use std::io::Write as _;
        // Best effort: a notice that cannot be written changes nothing about
        // the result, and a failed write to stderr has nowhere better to go.
        drop(writeln!(std::io::stderr(), "SKIPPED {message}"));
    }

    use rituals::Name;
    use rituals_compose::source::Source;
    use rituals_compose::workspace;

    use super::{next_steps, refusal, validate_source};

    #[test]
    fn the_next_steps_name_both_edits_with_a_dependency_line_ready_to_paste() {
        let name = Name::new("lint").expect("lint is a valid name");
        let lines = next_steps(&name, Path::new("/work/lint"));
        assert_eq!(
            lines,
            [
                "next: to import it, add this under [dependencies] in a project's \
                 ritual/Cargo.toml:",
                "lint = { path = \"/work/lint\" }",
                "then add \"lint\" to [package.metadata.ritual] tasks and run cargo ritual \
                 regenerate there (or cargo <name> ritual regenerate if it was made with --cli \
                 <name>)",
            ]
        );
    }

    /// A path that is not valid UTF-8 cannot appear in a Cargo manifest, so
    /// no dependency line is printed for it, lossy or otherwise.
    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_utf8_gets_a_plain_statement_instead_of_a_dependency_line() {
        use std::os::unix::ffi::OsStrExt;

        let name = Name::new("lint").expect("lint is a valid name");
        let crate_dir = Path::new(std::ffi::OsStr::from_bytes(b"/work/\xff/lint"));
        assert_eq!(
            next_steps(&name, crate_dir),
            [
                "next: lint was created, but its path is not valid UTF-8, so no Cargo manifest \
                 can name it; move it to a path that is before importing it"
            ]
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

    /// Walks up from this crate looking for `.git`, bounded to a fixed
    /// depth — a real checkout's root is a handful of directories above any
    /// crate inside it, and a loop with no upper bound at all is a hazard
    /// this small climb does not need to accept. Returns `None` rather
    /// than falling back to this crate's own directory when no `.git`
    /// turns up within that bound: a fallback that returns some path
    /// regardless would run the test that calls this against the wrong
    /// directory instead of saying it could not find a checkout at all.
    fn repository_root() -> Option<PathBuf> {
        const MAX_ANCESTORS: u8 = 16;

        let mut candidate = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
        for _ in 0..MAX_ANCESTORS {
            if candidate.join(".git").exists() {
                return Some(candidate);
            }
            candidate = candidate.parent()?.to_path_buf();
        }
        None
    }

    /// A directory under the system temp root that removes itself on drop.
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(tag: &str) -> std::io::Result<Self> {
            let path = std::env::temp_dir().join(format!(
                "ritual-create-workspace-check-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path)?;
            Ok(Self(path))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Names each scratch directory `<prefix>-<pid>-<counter>`, with no
    /// clock in it, so two concurrent test binaries cannot collide.
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn demo_name() -> Name {
        Name::new("demo").expect("demo is a valid name")
    }

    #[test]
    fn running_inside_this_repositorys_own_workspace_is_refused() {
        let Some(root) = repository_root() else {
            report_skip(
                "running_inside_this_repositorys_own_workspace_is_refused needs \
                     to run from inside a real ritual checkout, and none was found within 16 \
                     ancestors of this crate",
            );
            return;
        };

        let result = workspace::ensure_the_directory_stands_alone(&root, &refusal(&demo_name()));
        assert!(
            result.is_err(),
            "expected this repository's own root to be refused"
        );
        if let Err(error) = result {
            let message = error.to_string();
            assert!(message.contains(&root.display().to_string()));
            assert!(message.contains("that project's own `add demo`"));
        }
    }

    /// A directory in place of `Cargo.toml` makes `std::fs::write` fail with
    /// `EISDIR` — the poison [`write_crate`](super::write_crate) exercises
    /// here and in the sibling test below, mirroring how a real write can
    /// fail partway through scaffolding.
    fn poison_manifest_path(target_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(target_dir.join("Cargo.toml"))?;
        Ok(())
    }

    #[test]
    fn a_write_failure_removes_the_root_and_names_the_path_in_the_report()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new("write-failure")?;
        let target_dir = scratch.path().join("demo");
        poison_manifest_path(&target_dir)?;
        let name = demo_name();

        let write_result = super::write_crate(&target_dir, &name, &Source::Inherited);
        assert!(
            write_result.is_err(),
            "expected the poisoned manifest path to fail the write"
        );

        if let Err(failure) = write_result {
            let reported = super::undo(&target_dir, &name, &failure);
            let message = reported.to_string();
            assert!(
                message.contains(&target_dir.join("Cargo.toml").display().to_string()),
                "expected the message to name the path that failed to write: {message}"
            );
            assert!(
                message.contains("ritual removed demo so a retry starts clean"),
                "expected the message to say the root was removed: {message}"
            );
        }
        assert!(
            !target_dir.exists(),
            "the root must be gone after a successful removal"
        );
        Ok(())
    }

    /// A directory without write permission refuses to have entries removed
    /// from it — `chmod 0o555` on `target_dir` after the poisoned write has
    /// already failed makes `undo`'s `remove_dir_all` fail in turn, with no
    /// need for anything to hold the directory open. Unix-only: this
    /// permission model has no direct Windows equivalent.
    #[cfg(unix)]
    #[test]
    fn a_write_failure_reports_the_root_when_removal_also_fails()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::new("write-failure-unremovable")?;
        let target_dir = scratch.path().join("demo");
        poison_manifest_path(&target_dir)?;
        let name = demo_name();

        let write_result = super::write_crate(&target_dir, &name, &Source::Inherited);
        assert!(
            write_result.is_err(),
            "expected the poisoned manifest path to fail the write"
        );

        std::fs::set_permissions(&target_dir, std::fs::Permissions::from_mode(0o555))?;

        // Root ignores a directory's missing write bit, so this probe tells
        // whether the permission below actually blocks this process. If it
        // does not, the scenario cannot be demonstrated: restore and return
        // rather than asserting a state that never occurred, and say so
        // rather than passing silently.
        let probe = target_dir.join("probe");
        let permission_is_enforced = std::fs::File::create(&probe).is_err();
        if !permission_is_enforced {
            let _ = std::fs::remove_file(&probe);
            std::fs::set_permissions(&target_dir, std::fs::Permissions::from_mode(0o755))?;
            report_skip(
                "a_write_failure_reports_the_root_when_removal_also_fails could \
                     not demonstrate a permission-denied removal because this process does \
                     not honour directory write permissions",
            );
            return Ok(());
        }

        if let Err(failure) = write_result {
            let reported = super::undo(&target_dir, &name, &failure);

            // Restore permissions before any assertion can return early, so
            // the scratch directory this test made is still removable on
            // drop whether or not the assertions below pass.
            std::fs::set_permissions(&target_dir, std::fs::Permissions::from_mode(0o755))?;

            let message = reported.to_string();
            assert!(
                message.contains(&target_dir.display().to_string()),
                "expected the message to name the root that could not be removed: {message}"
            );
            assert!(
                target_dir.exists(),
                "the root undo could not remove must still be there"
            );
        }
        Ok(())
    }
}
