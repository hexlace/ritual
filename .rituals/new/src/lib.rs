//! The `new` task: scaffold a project with a command line of its own.

mod render;

use rituals::{Failure, Name, Outcome, Task, clap, report};
use rituals_compose::generated_file;
use rituals_compose::source::{Source, SourceArguments, assert_is_a_ritual_checkout};
use rituals_compose::workspace;

/// `new`'s arguments: the project directory to create, the name its own
/// command line answers to, and where ritual's own crates come from.
#[derive(clap::Args)]
struct Arguments {
    /// the project directory to create, in the working directory
    #[arg(value_name = "PROJECT")]
    name: String,

    // This value becomes two things at once, never a directory: the
    // `[[bin]] name`, and the `cargo <name>` alias key, which is also what
    // the string dispatch compares every mount key against at startup. The
    // composed CLI crate's own directory is fixed at `ritual/` regardless
    // of what this names — that is `write_project`'s to decide, not this
    // argument's — so nothing here ever becomes a path component. It is
    // still validated as a `rituals::Name`, the same rule every mount key
    // already obeys, because the alias key and the bin name both have to
    // be spelled as one.
    //
    // No name is reserved: nothing on disk is named after this value, so
    // there is nothing here for Cargo, or anyone else, to delete or
    // collide with. One case is worth knowing about: a value that is also
    // a built-in `cargo` subcommand (`check`, `build`, `test`, `run`,
    // `new`, `add`, …) gets an alias Cargo ignores — `warning: user-defined
    // alias `check` is ignored, because it is shadowed by a built-in
    // command` — so the binary works and `cargo <name>` does not. `cargo
    // run --package <project>-ritual --` still reaches it, and renaming is
    // the fix.
    /// the name this project's own command line answers to, and its cargo alias
    #[arg(long, value_name = "NAME", default_value = "ritual")]
    cli: String,

    #[command(flatten)]
    source: SourceArguments,
}

/// This task, for a command line to mount under whatever name imports it.
///
/// Scaffolds a project with a command line of its own: a workspace, the
/// composed CLI crate that imports ritual's own tasks, its generated file,
/// and the `cargo <cli>` alias that runs it.
//
// No `# Examples` section: a `task()` function has exactly one call-site
// shape — a mount line in a generated file — and an example here could only
// restate `let task = new::task();`, which shows nothing a reader needs.
#[must_use]
pub fn task() -> Task {
    Task::new(
        "scaffold a project with a command line of its own",
        |arguments: Arguments| run(&arguments),
    )
}

fn run(arguments: &Arguments) -> Outcome {
    let name = Name::new(&arguments.name)?;
    let cli = Name::new(&arguments.cli)?;
    let source = arguments.source.resolve();
    let source = validate_source(source)?;

    let current_dir = std::env::current_dir()
        .map_err(|error| Failure::new("reading the current directory failed").caused_by(error))?;
    // Runs after both names and the source are validated, so a caller with
    // two mistakes hears about the one they can fix without moving — and
    // before anything below is created, so a refused run leaves the tree
    // byte-identical.
    workspace::ensure_the_directory_stands_alone(&current_dir, &refusal(&name))?;

    let project_dir = current_dir.join(name.as_str());
    if project_dir.exists() {
        return Err(Failure::new(format!(
            "refusing to create {name}: it already exists"
        )));
    }

    std::fs::create_dir(&project_dir).map_err(|error| {
        Failure::new(format!("creating {} failed", project_dir.display())).caused_by(error)
    })?;

    if let Err(failure) = write_project(&project_dir, &name, &cli, &source) {
        return Err(undo(&project_dir, &name, &failure));
    }

    // Name the obvious next step after the `created` lines.
    report(format!("next: cd {name} && cargo {cli} --help"));

    Ok(())
}

/// Writes every file `new` scaffolds into `project_dir`, which the caller
/// has already created and refused to reuse. Stops at the first failure —
/// [`run`] removes `project_dir` wholesale on one, rather than this function
/// trying to undo file by file.
fn write_project(
    project_dir: &std::path::Path,
    name: &Name,
    cli: &Name,
    source: &Source,
) -> Outcome {
    let cli_package = format!("{name}-ritual");
    let cli_dir = project_dir.join("ritual");
    let cli_source_directory = cli_dir.join("src");
    let cargo_directory = project_dir.join(".cargo");

    std::fs::create_dir_all(&cli_source_directory).map_err(|error| {
        Failure::new(format!(
            "creating {} failed",
            cli_source_directory.display()
        ))
        .caused_by(error)
    })?;
    std::fs::create_dir_all(&cargo_directory).map_err(|error| {
        Failure::new(format!("creating {} failed", cargo_directory.display())).caused_by(error)
    })?;

    write_file(
        &project_dir.join("Cargo.toml"),
        &render::workspace_manifest(source),
        &format!("{name}/Cargo.toml"),
    )?;
    // Cargo builds into `target/` beside the workspace manifest, and a
    // project's first commit should not pick it up.
    write_file(
        &project_dir.join(".gitignore"),
        "/target\n",
        &format!("{name}/.gitignore"),
    )?;
    write_file(
        &cargo_directory.join("config.toml"),
        &render::cargo_alias(cli, &cli_package),
        &format!("{name}/.cargo/config.toml"),
    )?;
    write_file(
        &cli_dir.join("Cargo.toml"),
        &render::cli_manifest(cli, &cli_package, source),
        &format!("{name}/ritual/Cargo.toml"),
    )?;
    // The command name and the extern-crate identifier coincide here — the
    // same value twice is not a slip. An Entry's identifier is normally
    // read from `cargo metadata` rather than derived (see Entry's own doc),
    // which `new` cannot do because the project does not exist yet; the
    // bundle's own dependency key, `ritual`, already is a valid Rust
    // identifier, so the two coincide.
    let entries = [generated_file::Entry::new(
        render::MANAGEMENT_BUNDLE.key,
        render::MANAGEMENT_BUNDLE.key,
    )];
    write_file(
        &cli_source_directory.join("main.rs"),
        &generated_file::render(&cli_package, &entries),
        &format!("{name}/ritual/src/main.rs"),
    )?;

    Ok(())
}

/// Writes `content` to `path`, reports `created <reported_path>`.
fn write_file(path: &std::path::Path, content: &str, reported_path: &str) -> Outcome {
    std::fs::write(path, content).map_err(|error| {
        Failure::new(format!("writing {} failed", path.display())).caused_by(error)
    })?;
    report(format!("created {reported_path}"));
    Ok(())
}

/// Removes `project_dir` after [`write_project`] fails partway, and folds
/// the removal's own outcome into the refusal — parity with `add`'s own
/// undo: a run that does not finish leaves nothing behind, and says whether
/// it managed to.
///
/// `failure`'s own `Display` already carries its attached cause, so
/// formatting `{failure}` directly here is enough — this needs no local
/// rendering of its own.
fn undo(project_dir: &std::path::Path, name: &Name, failure: &Failure) -> Failure {
    match std::fs::remove_dir_all(project_dir) {
        Ok(()) => Failure::new(format!(
            "{failure}; ritual removed {name} so a retry starts clean"
        )),
        Err(_removal_error) => Failure::new(format!(
            "{failure}; ritual could not remove {name} — check it before running `new {name}` \
             again"
        )),
    }
}

/// Turns a `--path` source into the absolute checkout root, after checking
/// it is a real one; passes a registry or `--git` source through unchanged.
///
/// Unlike `create`'s `validate_source`, this does not narrow the path to
/// `rituals`'s own crate directory: `new` names two crates from the same
/// checkout — `rituals` in the workspace manifest and `rituals-core`, ritual's
/// own bundle, in the composed CLI's manifest — so
/// [`render::workspace_manifest`] and [`render::cli_manifest`] each join their
/// crate's directory on themselves.
///
/// # Errors
///
/// Returns a [`Failure`] naming the given `--path` when it does not
/// contain `rituals`'s manifest.
fn validate_source(source: Source) -> Result<Source, Failure> {
    let Source::Path(checkout_root) = source else {
        return Ok(source);
    };

    assert_is_a_ritual_checkout(&checkout_root)?;

    let absolute_checkout_root = std::path::absolute(&checkout_root).map_err(|error| {
        Failure::new(format!("resolving {} failed", checkout_root.display())).caused_by(error)
    })?;

    Ok(Source::Path(absolute_checkout_root))
}

/// What `new` puts into whichever refusal
/// [`workspace::ensure_the_directory_stands_alone`] produces.
///
/// `name` is the input a person already typed, already validated by
/// [`Name::new`] before this is called. The remedy names *the enclosing
/// project's own* `add` rather than a binary, for the same reason
/// `create`'s own `refusal` does: the project being stood in may call its
/// command line anything, and `new` has not read its manifest to find out.
fn refusal(name: &Name) -> workspace::Refusal {
    workspace::Refusal {
        attempted_command: format!("new {name}"),
        why_not_here: "a project does not belong inside another project's workspace".to_string(),
        what_to_do_instead: format!(
            "run new outside any Cargo workspace, or, if you meant a new command rather than \
             a new project, add it with the enclosing project's own `add {name}`"
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

    use super::validate_source;

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

    /// A directory under the system temp root that removes itself on drop —
    /// this crate's own copy of `rituals-compose`'s `test_support::ScratchDir`
    /// and `tasks/create`'s copy of the same: a different crate is a genuine
    /// boundary none of the three can share across.
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(tag: &str) -> std::io::Result<Self> {
            let path = std::env::temp_dir().join(format!(
                "ritual-new-{tag}-{}-{}",
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

    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn demo_name() -> Name {
        Name::new("demo").expect("demo is a valid name")
    }

    fn ritual_name() -> Name {
        Name::new("ritual").expect("ritual is a valid name")
    }

    /// A directory in place of `Cargo.toml` makes `std::fs::write` fail with
    /// `EISDIR` — the poison [`write_project`](super::write_project)
    /// exercises here and in the sibling test below, mirroring how a real
    /// write can fail partway through scaffolding.
    fn poison_workspace_manifest_path(
        project_dir: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(project_dir.join("Cargo.toml"))?;
        Ok(())
    }

    #[test]
    fn a_write_failure_removes_the_root_and_names_the_path_in_the_report()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new("write-failure")?;
        let project_dir = scratch.path().join("demo");
        poison_workspace_manifest_path(&project_dir)?;
        let name = demo_name();

        let write_result = super::write_project(
            &project_dir,
            &name,
            &ritual_name(),
            &Source::Git("https://example.invalid/x".to_string()),
        );
        assert!(
            write_result.is_err(),
            "expected the poisoned manifest path to fail the write"
        );

        if let Err(failure) = write_result {
            let reported = super::undo(&project_dir, &name, &failure);
            let message = reported.to_string();
            assert!(
                message.contains(&project_dir.join("Cargo.toml").display().to_string()),
                "expected the message to name the path that failed to write: {message}"
            );
            assert!(
                message.contains("ritual removed demo so a retry starts clean"),
                "expected the message to say the root was removed: {message}"
            );
        }
        assert!(
            !project_dir.exists(),
            "the root must be gone after a successful removal"
        );
        Ok(())
    }

    /// A directory without write permission refuses to have entries removed
    /// from it — `chmod 0o555` on `project_dir` after the poisoned write has
    /// already failed makes `undo`'s `remove_dir_all` fail in turn, with no
    /// need for anything to hold the directory open. Unix-only: this
    /// permission model has no direct Windows equivalent.
    #[cfg(unix)]
    #[test]
    fn a_write_failure_reports_the_root_when_removal_also_fails()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt;

        let scratch = ScratchDir::new("write-failure-unremovable")?;
        let project_dir = scratch.path().join("demo");
        poison_workspace_manifest_path(&project_dir)?;
        let name = demo_name();

        let write_result = super::write_project(
            &project_dir,
            &name,
            &ritual_name(),
            &Source::Git("https://example.invalid/x".to_string()),
        );
        assert!(
            write_result.is_err(),
            "expected the poisoned manifest path to fail the write"
        );

        std::fs::set_permissions(&project_dir, std::fs::Permissions::from_mode(0o555))?;

        // Root ignores a directory's missing write bit, so this probe tells
        // whether the permission below actually blocks this process. If it
        // does not, the scenario cannot be demonstrated: restore and return
        // rather than asserting a state that never occurred, and say so
        // rather than passing silently.
        let probe = project_dir.join("probe");
        let permission_is_enforced = std::fs::File::create(&probe).is_err();
        if !permission_is_enforced {
            let _ = std::fs::remove_file(&probe);
            std::fs::set_permissions(&project_dir, std::fs::Permissions::from_mode(0o755))?;
            report_skip(
                "a_write_failure_reports_the_root_when_removal_also_fails could \
                     not demonstrate a permission-denied removal because this process does \
                     not honour directory write permissions",
            );
            return Ok(());
        }

        if let Err(failure) = write_result {
            let reported = super::undo(&project_dir, &name, &failure);

            // Restore permissions before any assertion can return early, so
            // the scratch directory this test made is still removable on
            // drop whether or not the assertions below pass.
            std::fs::set_permissions(&project_dir, std::fs::Permissions::from_mode(0o755))?;

            let message = reported.to_string();
            assert!(
                message.contains(&project_dir.display().to_string()),
                "expected the message to name the root that could not be removed: {message}"
            );
            assert!(
                project_dir.exists(),
                "the root undo could not remove must still be there"
            );
        }
        Ok(())
    }
}
