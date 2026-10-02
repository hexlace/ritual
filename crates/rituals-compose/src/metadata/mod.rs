//! The resolved dependency graph, and every question this framework asks of
//! it.
//!
//! `cargo metadata --format-version 1` answers every question ritual needs
//! about a project in one call: where the workspace is, which package is
//! its composed CLI, what that package's dependencies resolve to, what each
//! of those declares about itself, and what extern-crate name rustc gives
//! each one. Ritual never reads a dependency's manifest itself and never
//! walks a directory looking for anything.

mod schema;

use std::path::Path;

use rituals::{Failure, Name, Outcome};

use crate::cargo;
use crate::generated_file::Entry;
use crate::rollback::Changes;
use crate::workspace::{Located, locate_project};

pub use schema::Metadata;
pub(crate) use schema::{DepKind, Node, Package};

/// A composed CLI crate, found among a workspace's members. Declared in the
/// private `crate::project` module and re-exported here, at the path a
/// caller reaches it by: only ever built by [`Metadata::locate_project`].
#[doc(inline)]
pub use crate::project::Project;
// `Dependency` has no production reader outside this module — `Project`'s
// own accessors read `Package::dependencies`' elements by field, never
// construct one — but this crate's tests build one directly, to exercise
// those accessors without a real `cargo metadata` call.
#[cfg(test)]
pub(crate) use schema::Dependency;

/// The `cargo metadata` schema version this framework was written against.
///
/// Asked for explicitly on every call, and checked against the response, so
/// a future Cargo defaulting to a new schema fails loudly here rather than
/// silently misreading a field.
const SUPPORTED_FORMAT_VERSION: u64 = 1;

/// Runs `cargo metadata --format-version 1` in `current_dir` and parses its
/// output.
///
/// Cargo writes `Cargo.lock` here when the workspace has none, and rewrites
/// it when it is stale. A task that promises to leave the project as it found
/// it reaches Cargo through [`fetch_in_its_own_project`], which records the
/// lockfile first; a caller of this function that is already inside a run has
/// recorded it itself.
///
/// Invokes the cargo that launched this process, through [`cargo::command`],
/// so a nested call never uses a different cargo than the one in charge. No flags beyond
/// `--format-version 1`: `--offline` would break a project whose
/// dependencies are not yet fetched, and `--locked` would break `add`,
/// which must update the lockfile.
///
/// # Errors
///
/// Returns a [`Failure`] naming `cargo metadata`'s own stderr when the
/// subprocess could not be run or exited unsuccessfully, or one naming a
/// parse or version problem when its output is not what this framework
/// understands.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::metadata;
///
/// // Shells out to a real `cargo metadata` and needs a workspace on disk
/// // to run against, so this example is `no_run` rather than
/// // compiled-and-executed.
/// let document = metadata::fetch(Path::new("."))?;
/// let project = document.locate_project("demo-ritual")?;
/// println!("crate manifest: {}", project.manifest_path().display());
/// # Ok::<(), rituals::Failure>(())
/// ```
pub fn fetch(current_dir: &Path) -> Result<Metadata, Failure> {
    let output = cargo::command()
        .args(["metadata", "--format-version", "1"])
        .current_dir(current_dir)
        .output()
        .map_err(|error| Failure::new("running `cargo metadata` failed").caused_by(error))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Failure::new(format!(
            "cargo metadata failed: {}",
            stderr.trim_end()
        )));
    }

    parse(&output.stdout)
}

/// Fetches the metadata of the project `current_dir` is in, recording the
/// lockfile that fetch may write, and refuses unless that project is the one
/// `package_name` belongs to.
///
/// [`fetch`] followed by [`Metadata::ensure_runs_in_its_own_project`], for a
/// task that promises to leave the project as it found it: `cargo metadata`
/// creates `Cargo.lock` when there is none and rewrites it when it is stale,
/// so the workspace's lockfile is recorded in `changes` before the fetch, and
/// a run that fails afterwards puts it back. Taking `changes` is what makes
/// the call impossible to make without the record.
///
/// The workspace is found first, with `cargo locate-project`, which writes
/// nothing: the lockfile is the root's, and the root is only known from
/// Cargo. Where Cargo finds no workspace, `cargo metadata` is not run at all,
/// so nothing unrecorded can be written. If it finds no manifest either,
/// there is no Cargo project, and the refusal is the one a project that is
/// not this one gets, which names the command to run inside the right one,
/// where Cargo's own words say what is missing and nothing about what to do.
/// A manifest Cargo finds but cannot place in a workspace keeps Cargo's own
/// words, which say what is wrong with it.
///
/// `command` and `arguments` are what the person typed after the binary's
/// name, as for [`Metadata::ensure_runs_in_its_own_project`].
///
/// # Errors
///
/// Returns a [`Failure`] that names the command to run instead when there is
/// no Cargo manifest at or above `current_dir`, or when no workspace member
/// is called `package_name`, one carrying Cargo's own words when a manifest
/// is found but no workspace for it, and [`fetch`]'s own failure when `cargo
/// metadata` cannot be run, fails for another reason, or answers with
/// something this framework does not understand.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::{metadata, rollback};
///
/// // Shells out to a real `cargo metadata` and needs a workspace on disk
/// // to run against, so this example is `no_run`.
/// let key = rollback::attempt("running `import greeter` again", |changes| {
///     let document = metadata::fetch_in_its_own_project(
///         changes,
///         Path::new("."),
///         "demo-ritual",
///         "import",
///         "greeter",
///     )?;
///     let project = document.locate_project("demo-ritual")?;
///     Ok(project.workspace_root().to_path_buf())
/// })?;
/// println!("workspace root: {}", key.display());
/// # Ok::<(), rituals::Failure>(())
/// ```
pub fn fetch_in_its_own_project(
    changes: &mut Changes,
    current_dir: &Path,
    package_name: &str,
    command: &str,
    arguments: &str,
) -> Result<Metadata, Failure> {
    match locate_project(current_dir, true)? {
        Located::Found(workspace_manifest) => {
            let lockfile = workspace_manifest.with_file_name("Cargo.lock");
            let document = changes.run_changing(&[lockfile.as_path()], || fetch(current_dir))?;
            document.ensure_runs_in_its_own_project(package_name, command, arguments)?;
            Ok(document)
        }
        Located::NotFound(no_workspace) => match locate_project(current_dir, false)? {
            Located::Found(_nearest_manifest) => Err(Failure::new(format!(
                "cargo locate-project failed: {no_workspace}"
            ))),
            Located::NotFound(_no_manifest) => Err(outside_its_project_refusal(command, arguments)),
        },
    }
}

/// Parses `cargo metadata --format-version 1`'s JSON output.
///
/// Split from [`fetch`] so a unit test can exercise parsing directly against
/// a committed fixture without spawning `cargo`.
///
/// # Errors
///
/// Returns a [`Failure`] when `document` is not valid JSON in the shape
/// expected, or when its `version` is not [`SUPPORTED_FORMAT_VERSION`].
pub(crate) fn parse(document: &[u8]) -> Result<Metadata, Failure> {
    let metadata: Metadata = serde_json::from_slice(document)
        .map_err(|error| Failure::new("parsing cargo metadata output failed").caused_by(error))?;

    if metadata.version != SUPPORTED_FORMAT_VERSION {
        return Err(Failure::new(format!(
            "cargo metadata returned format version {}, but this framework understands only \
             version {SUPPORTED_FORMAT_VERSION}",
            metadata.version
        )));
    }

    Ok(metadata)
}

impl Metadata {
    /// Finds the workspace member named `package_name` in this document,
    /// and its single binary target.
    ///
    /// `package_name` normally arrives through the command line a task
    /// asked to receive — `CommandLine::identity().package_name()` —
    /// rather than being read directly: `cargo metadata` walks up from the
    /// current directory to find the workspace root, and this method then
    /// identifies the caller among the workspace's members by name, so a
    /// task built with `Task::receiving_command_line` works from any
    /// subdirectory of a project.
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming `package_name` and the workspace root
    /// when no workspace member has that name, and one naming
    /// `package_name` when its manifest declares no `[[bin]]` target or
    /// more than one.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::metadata;
    ///
    /// // `locate_project` reads a document `fetch` already produced from a
    /// // real `cargo metadata` call, so this example stays `no_run`.
    /// let document = metadata::fetch(Path::new("."))?;
    /// let project = document.locate_project("demo-ritual")?;
    /// println!("workspace root: {}", project.workspace_root().display());
    /// # Ok::<(), rituals::Failure>(())
    /// ```
    pub fn locate_project(&self, package_name: &str) -> Result<Project<'_>, Failure> {
        crate::project::locate(self, package_name)
    }

    /// Reads `package_name`'s `[package.metadata.ritual] tasks` list from
    /// this document and resolves every name in it to an [`Entry`], in the
    /// order the manifest names them.
    ///
    /// This is a pure read: nothing is written, and each check runs only
    /// once the one before it has succeeded — `package_name` found as a
    /// workspace member with one binary target, `tasks` present and a list of
    /// strings, each name valid, no duplicates, a matching normal dependency
    /// present on every target, that dependency resolved, and the resolved
    /// crate marked `task = true`.
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming the specific problem: a project that
    /// cannot be located, an absent or wrongly-shaped `tasks` list, an invalid
    /// or duplicated name, a name with no matching dependency, a dependency
    /// declared only under a `cfg(...)` target, or a dependency that resolves
    /// to a crate whose `task` value is missing, `false`, or not a boolean.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::metadata;
    ///
    /// // Reads a document `fetch` already produced from a real
    /// // `cargo metadata` call, so this example stays `no_run`.
    /// let document = metadata::fetch(Path::new("."))?;
    /// let entries = document.resolve_task_list("demo-ritual")?;
    /// println!("{} tasks imported", entries.len());
    /// # Ok::<(), rituals::Failure>(())
    /// ```
    pub fn resolve_task_list(&self, package_name: &str) -> Result<Vec<Entry>, Failure> {
        crate::task_list::resolve(self, package_name)
    }

    /// Refuses unless the dependency `package_name` declares under `key` is a
    /// task: a normal dependency, present on every target, that resolves to
    /// a crate declaring `[package.metadata.ritual] task = true`.
    ///
    /// This is the question an import asks of a dependency somebody else
    /// wrote, once Cargo has declared it and before it joins the task list.
    /// The dependency is found the way [`Metadata::resolve_task_list`] finds
    /// the one a listed name stands for, by the extern-crate name rustc gives
    /// `key` (the key with `-` read as `_`), and judged by the same rule, so
    /// whatever this accepts the resolver accepts once `key` is in the list.
    /// Nothing is read from the dependency's own manifest: what the crate
    /// declares about itself is in this document, whether it came from a
    /// registry, a git repository or a path.
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] saying what is wrong and what to do about it
    /// when `package_name` cannot be located, when it has no dependency
    /// under `key`, when that dependency is only a dev- or build-dependency
    /// or only present under a `cfg(...)` target, when the crate it
    /// resolves to does not declare `task = true`, or when it declares
    /// `task` as something other than a boolean.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals::Name;
    /// use rituals_compose::metadata;
    ///
    /// // Reads a document `fetch` already produced from a real
    /// // `cargo metadata` call, so this example stays `no_run`.
    /// let document = metadata::fetch(Path::new("."))?;
    /// document.ensure_dependency_is_a_task("demo-ritual", &Name::new("hail")?)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn ensure_dependency_is_a_task(&self, package_name: &str, key: &Name) -> Outcome {
        crate::task_list::ensure_dependency_is_a_task(self, package_name, key)
    }

    /// Reports whether any workspace member in this document is already
    /// named `name` — a task's crate is named after the command, so this is
    /// what tells `add` a name is already taken by an unrelated package.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals::Name;
    /// use rituals_compose::metadata;
    ///
    /// // Reads a document `fetch` already produced from a real
    /// // `cargo metadata` call, so this example stays `no_run`.
    /// let document = metadata::fetch(Path::new("."))?;
    /// let name = Name::new("lint")?;
    /// if document.has_workspace_member(&name) {
    ///     println!("`lint` already names a workspace package");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn has_workspace_member(&self, name: &Name) -> bool {
        self.has_member_package(name.as_str())
    }

    /// Refuses when no member of this workspace is a package called
    /// `package_name`.
    ///
    /// A command line is identified with its project by package name, and
    /// nothing more. That catches the global binary's `add` run inside a
    /// project, and a project's own command line run inside another project
    /// whose CLI crate has a different package name.
    ///
    /// Two projects whose CLI crates share a package name pass this check:
    /// one project's built binary, run by hand inside the other, is taken
    /// for that project's own. The writes land in the current workspace, but
    /// the collision check before writing reads the running binary's top
    /// level, which is the other project's. So the check can be wrong both
    /// ways:
    ///
    /// - A false accept: a name that collides with one of the current
    ///   project's own top-level commands is written. The project's command
    ///   line then refuses to start, naming both commands, and because
    ///   `regenerate` is unreachable until it starts, the fix is a hand edit:
    ///   drop the entry from `[package.metadata.ritual] tasks` and its mount
    ///   line from the generated file.
    /// - A false refusal: a name that is free in the current project is
    ///   refused because it collides in the other.
    ///
    /// Both are accepted. Only a built binary run by hand, inside a
    /// different project whose CLI crate has the same package name, can
    /// reach them; nothing in ordinary use through a project's own alias
    /// does. Both fail loudly and say what to change. It is the same kind of
    /// case as the bundle mounted by hand under the bin's name, which no
    /// check before writing can see either and which surfaces at the next
    /// startup.
    ///
    /// `command` and `arguments` are what the person typed after the
    /// binary's name, such as `add` and `lint`; the refusal hands them back
    /// as the command to run in their own project. `arguments` is empty for
    /// a command that takes none.
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] that names the command to run instead when no
    /// workspace member is called `package_name`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::metadata;
    ///
    /// // Reads a document `fetch` already produced from a real
    /// // `cargo metadata` call, so this example stays `no_run`.
    /// let document = metadata::fetch(Path::new("."))?;
    /// document.ensure_runs_in_its_own_project("demo-ritual", "add", "lint")?;
    /// # Ok::<(), rituals::Failure>(())
    /// ```
    pub fn ensure_runs_in_its_own_project(
        &self,
        package_name: &str,
        command: &str,
        arguments: &str,
    ) -> Result<(), Failure> {
        if self.has_member_package(package_name) {
            return Ok(());
        }
        Err(outside_its_project_refusal(command, arguments))
    }

    /// Whether any workspace member in this document is called
    /// `package_name`.
    fn has_member_package(&self, package_name: &str) -> bool {
        self.packages.iter().any(|candidate| {
            self.workspace_members.contains(&candidate.id) && candidate.name == package_name
        })
    }
}

/// The refusal for a command line run outside the project it belongs to.
///
/// The running binary cannot see what the project the person stands in
/// calls its own command line, so the remedy names the default, `ritual`,
/// and the shape a `--cli` project uses.
fn outside_its_project_refusal(command: &str, arguments: &str) -> Failure {
    let typed = if arguments.is_empty() {
        command.to_string()
    } else {
        format!("{command} {arguments}")
    };
    Failure::new(format!(
        "`{command}` works inside the project this command line belongs to; in your project, \
         run `cargo ritual {typed}` (or `cargo <name> ritual {typed}` if it was made with \
         `--cli <name>`)"
    ))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rituals::{Failure, Name};

    use super::{Metadata, fetch_in_its_own_project, outside_its_project_refusal, parse};
    use crate::rollback::attempt;
    use crate::test_support::{ScratchDir, TestOutcome};

    #[test]
    fn the_outside_project_refusal_hands_back_the_command_to_run() {
        assert_eq!(
            outside_its_project_refusal("add", "lint").to_string(),
            "`add` works inside the project this command line belongs to; in your project, run \
             `cargo ritual add lint` (or `cargo <name> ritual add lint` if it was made with \
             `--cli <name>`)"
        );
        assert_eq!(
            outside_its_project_refusal("regenerate", "").to_string(),
            "`regenerate` works inside the project this command line belongs to; in your \
             project, run `cargo ritual regenerate` (or `cargo <name> ritual regenerate` if it \
             was made with `--cli <name>`)"
        );
    }

    /// A workspace with one package, `demo`, that has no dependencies, so
    /// `cargo metadata` runs against it with nothing to fetch.
    fn scratch_package(tag: &str) -> Result<ScratchDir, Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new(tag)?;
        std::fs::create_dir_all(scratch.path().join("src"))?;
        std::fs::write(
            scratch.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        std::fs::write(scratch.path().join("src/lib.rs"), "")?;
        Ok(scratch)
    }

    /// What the rollback appends to every failure of a run, which a task
    /// that fetches inside [`attempt`] sees on each refusal below.
    const PUT_BACK: &str = "; ritual put the project back as it found it";

    /// [`fetch_in_its_own_project`] as a task calls it: inside a run that
    /// records what `cargo metadata` writes. The failure is returned without
    /// the rollback's report, which is asserted to be there.
    fn fetch_in_a_run(directory: &Path, package: &str) -> Result<Metadata, String> {
        let outcome = attempt("running `import greeter` again", |changes| {
            fetch_in_its_own_project(changes, directory, package, "import", "greeter")
        });
        outcome.map_err(|failure| {
            let message = failure.to_string();
            message
                .strip_suffix(PUT_BACK)
                .unwrap_or_else(|| unreachable!("the rollback reports every failure: {message}"))
                .to_string()
        })
    }

    #[test]
    fn outside_any_workspace_the_refusal_says_to_work_inside_a_project() -> TestOutcome {
        // Cargo's own words here are "could not find `Cargo.toml`", which
        // says nothing about what to do; the person is told the command to
        // run inside their project instead.
        let scratch = ScratchDir::new("fetch-outside-any-workspace")?;

        let outcome = fetch_in_a_run(scratch.path(), "demo-ritual");

        assert_eq!(
            outcome.err().as_deref(),
            Some(
                outside_its_project_refusal("import", "greeter")
                    .to_string()
                    .as_str()
            ),
            "expected a directory with no project to be refused"
        );
        Ok(())
    }

    #[test]
    fn inside_another_projects_workspace_the_refusal_is_the_same() -> TestOutcome {
        let scratch = scratch_package("fetch-another-workspace")?;

        let outcome = fetch_in_a_run(scratch.path(), "demo-ritual");

        assert_eq!(
            outcome.err().as_deref(),
            Some(
                outside_its_project_refusal("import", "greeter")
                    .to_string()
                    .as_str()
            ),
            "expected a workspace without the package to be refused"
        );
        assert!(
            !scratch.path().join("Cargo.lock").exists(),
            "a refused fetch leaves no lockfile behind"
        );
        Ok(())
    }

    #[test]
    fn inside_the_workspace_the_package_belongs_to_the_document_is_returned() -> TestOutcome {
        let scratch = scratch_package("fetch-own-workspace")?;

        let document = fetch_in_a_run(scratch.path(), "demo")?;

        assert!(document.has_workspace_member(&valid_name("demo")));
        Ok(())
    }

    /// A manifest under a workspace root that does not list it has a
    /// project, but one Cargo cannot read. Cargo's own refusal says what is
    /// wrong with it, which "work inside a project" would hide.
    #[test]
    fn a_workspace_cargo_cannot_read_keeps_cargos_own_refusal() -> TestOutcome {
        let scratch = ScratchDir::new("fetch-unreadable-workspace")?;
        let member = scratch.path().join("not-listed");
        std::fs::create_dir_all(member.join("src"))?;
        std::fs::write(
            scratch.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\nresolver = \"3\"\n",
        )?;
        std::fs::write(
            member.join("Cargo.toml"),
            "[package]\nname = \"not-listed\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        std::fs::write(member.join("src/lib.rs"), "")?;

        let outcome = fetch_in_a_run(&member, "not-listed");

        let message = outcome
            .err()
            .ok_or("expected cargo to refuse this directory")?;
        assert!(
            message.starts_with("cargo locate-project failed: "),
            "expected cargo's own refusal; got: {message}"
        );
        assert!(
            !scratch.path().join("Cargo.lock").exists(),
            "a workspace cargo cannot read is refused without asking it for metadata, which \
             would write a lockfile"
        );
        Ok(())
    }

    /// Runs `fetch_in_its_own_project` in `directory`, which is the project
    /// of package `demo`, inside a run that then ends as `ending`, and returns
    /// what the run reported.
    fn fetch_then(directory: &Path, ending: Result<(), Failure>) -> Result<(), Failure> {
        attempt("running `import greeter` again", |changes| {
            fetch_in_its_own_project(changes, directory, "demo", "import", "greeter")?;
            ending
        })
    }

    /// `cargo metadata` creates the lockfile a project lacks, so a run that
    /// fetches and is then refused has to take it back out. The control is
    /// the same fetch in a run that succeeds, which keeps the lockfile and so
    /// proves that cargo wrote it.
    #[test]
    fn a_lockfile_cargo_creates_is_removed_when_the_run_fails() -> TestOutcome {
        let scratch = scratch_package("fetch-creates-lockfile")?;
        let lockfile = scratch.path().join("Cargo.lock");

        fetch_then(scratch.path(), Ok(()))?;
        assert!(
            lockfile.exists(),
            "cargo metadata is expected to write a lockfile here; if it did not, this test \
             proves nothing about removing one"
        );
        std::fs::remove_file(&lockfile)?;

        let outcome = fetch_then(scratch.path(), Err(Failure::new("refused")));

        assert!(outcome.is_err(), "expected the run to fail");
        assert!(!lockfile.exists(), "the lockfile the run made must be gone");
        Ok(())
    }

    /// A lockfile that no longer matches the manifest is rewritten by `cargo
    /// metadata`, so the run puts its old bytes back when it fails.
    #[test]
    fn a_stale_lockfile_cargo_rewrites_is_restored_when_the_run_fails() -> TestOutcome {
        let scratch = scratch_package("fetch-rewrites-lockfile")?;
        let lockfile = scratch.path().join("Cargo.lock");
        let generated = crate::cargo::command()
            .arg("generate-lockfile")
            .current_dir(scratch.path())
            .output()?;
        assert!(generated.status.success(), "cargo made a lockfile");
        // A dependency the lockfile does not know about makes it stale.
        let helper = scratch.path().join("helper");
        std::fs::create_dir_all(helper.join("src"))?;
        std::fs::write(
            helper.join("Cargo.toml"),
            "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        std::fs::write(helper.join("src/lib.rs"), "")?;
        std::fs::write(
            scratch.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nhelper = { path = \"helper\" }\n",
        )?;
        let stale = std::fs::read(&lockfile)?;

        fetch_then(scratch.path(), Ok(()))?;
        assert_ne!(
            std::fs::read(&lockfile)?,
            stale,
            "cargo metadata is expected to rewrite a stale lockfile; if it did not, this test \
             proves nothing about restoring one"
        );
        std::fs::write(&lockfile, &stale)?;

        let outcome = fetch_then(scratch.path(), Err(Failure::new("refused")));

        assert!(outcome.is_err(), "expected the run to fail");
        assert_eq!(
            std::fs::read(&lockfile)?,
            stale,
            "the old bytes must be back"
        );
        Ok(())
    }

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[test]
    fn metadata_is_send_and_sync() {
        assert_send::<Metadata>();
        assert_sync::<Metadata>();
    }

    /// A `Name` from a literal already known to be valid.
    fn valid_name(value: &str) -> Name {
        Name::new(value).expect("a test passes only names it knows are valid")
    }

    /// A `cargo metadata --format-version 1` document captured from a
    /// scratch workspace built to exercise every shape this module and
    /// `project.rs` read: a composed CLI with `[package.metadata.ritual]
    /// tasks = [...]`, a task crate with no metadata table at all, one with
    /// `task = true`, one reached through a Cargo dependency rename, one
    /// reached through a dependency key that is a Rust keyword (`move`),
    /// one that is a dev-dependency only, and one with a malformed
    /// `[package.metadata.ritual]` table (`task = "true"`, a string). The
    /// capturing machine's absolute paths are replaced with
    /// `/scrubbed/checkout` throughout.
    const DEMO_WORKSPACE: &str = include_str!("fixtures/demo-workspace.json");

    #[test]
    fn parses_the_document_version() {
        let metadata = parse(DEMO_WORKSPACE.as_bytes());
        assert!(
            metadata.is_ok(),
            "expected the fixture to parse: {metadata:?}"
        );
        if let Ok(metadata) = metadata {
            assert_eq!(metadata.version, 1);
        }
    }

    #[test]
    fn a_future_format_version_is_refused() {
        let future_version = DEMO_WORKSPACE.replacen("\"version\": 1,", "\"version\": 2,", 1);
        let metadata = parse(future_version.as_bytes());
        assert!(
            metadata.is_err(),
            "expected an unsupported version to be refused"
        );
        if let Err(error) = metadata {
            assert!(error.to_string().contains('2'));
        }
    }

    #[test]
    fn a_package_with_no_metadata_table_reports_null() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let package = metadata
            .packages
            .iter()
            .find(|package| package.name == "task-null");
        assert!(
            package.is_some(),
            "expected a task-null package in the fixture"
        );
        if let Some(package) = package {
            assert!(package.metadata.is_null());
        }
        Ok(())
    }

    #[test]
    fn a_package_with_task_true_reports_it() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let package = metadata
            .packages
            .iter()
            .find(|package| package.name == "task-true");
        assert!(
            package.is_some(),
            "expected a task-true package in the fixture"
        );
        if let Some(package) = package {
            assert_eq!(package.metadata["ritual"]["task"], serde_json::json!(true));
        }
        Ok(())
    }

    #[test]
    fn a_malformed_ritual_table_is_still_captured_as_a_raw_value() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let package = metadata
            .packages
            .iter()
            .find(|package| package.name == "task-malformed");
        assert!(
            package.is_some(),
            "expected a task-malformed package in the fixture"
        );
        if let Some(package) = package {
            // A string, not the boolean the framework requires — parsing
            // must not fail on this; the semantic check belongs to the
            // resolver that reads this value, not to this module.
            assert_eq!(
                package.metadata["ritual"]["task"],
                serde_json::json!("true")
            );
        }
        Ok(())
    }

    #[test]
    fn the_composed_cli_packages_tasks_list_is_read() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let cli = metadata
            .packages
            .iter()
            .find(|package| package.name == "demo-ritual");
        assert!(
            cli.is_some(),
            "expected a demo-ritual package in the fixture"
        );
        if let Some(cli) = cli {
            assert_eq!(
                cli.metadata["ritual"]["tasks"],
                serde_json::json!(["task-true", "renamed", "move"])
            );
        }
        Ok(())
    }

    #[test]
    fn a_renamed_dependency_carries_its_rename() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let cli = metadata
            .packages
            .iter()
            .find(|package| package.name == "demo-ritual");
        assert!(
            cli.is_some(),
            "expected a demo-ritual package in the fixture"
        );
        if let Some(cli) = cli {
            let renamed = cli
                .dependencies
                .iter()
                .find(|dependency| dependency.name == "task-renamed-source");
            assert!(
                renamed.is_some(),
                "expected the renamed dependency in the fixture"
            );
            if let Some(renamed) = renamed {
                assert_eq!(renamed.rename.as_deref(), Some("renamed"));
            }
        }
        Ok(())
    }

    #[test]
    fn a_keyword_dependency_key_is_reported_without_a_raw_prefix() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let node = metadata
            .resolve
            .nodes
            .iter()
            .find(|node| node.id.contains("demo-ritual"));
        assert!(
            node.is_some(),
            "expected the composed CLI's resolve node in the fixture"
        );
        if let Some(node) = node {
            let keyword_dependency = node
                .deps
                .iter()
                .find(|dependency| dependency.name == "move");
            assert!(
                keyword_dependency.is_some(),
                "expected the `move` extern-crate name in the resolved graph"
            );
        }
        Ok(())
    }

    #[test]
    fn a_dev_dependency_is_reported_with_a_dev_kind() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let cli = metadata
            .packages
            .iter()
            .find(|package| package.name == "demo-ritual");
        assert!(
            cli.is_some(),
            "expected a demo-ritual package in the fixture"
        );
        if let Some(cli) = cli {
            let dev_dependency = cli
                .dependencies
                .iter()
                .find(|dependency| dependency.name == "task-dev-only");
            assert!(
                dev_dependency.is_some(),
                "expected the dev dependency in the fixture"
            );
            if let Some(dev_dependency) = dev_dependency {
                assert_eq!(dev_dependency.kind.as_deref(), Some("dev"));
            }
        }
        Ok(())
    }

    #[test]
    fn has_workspace_member_finds_a_real_member_and_not_an_unrelated_name() {
        let metadata = parse(DEMO_WORKSPACE.as_bytes());
        assert!(
            metadata.is_ok(),
            "expected the fixture to parse: {metadata:?}"
        );
        if let Ok(metadata) = metadata {
            assert!(metadata.has_workspace_member(&valid_name("demo-ritual")));
            assert!(metadata.has_workspace_member(&valid_name("task-true")));
            assert!(!metadata.has_workspace_member(&valid_name("no-such-package")));
        }
    }
}
