//! Task crates to import from outside a project: a directory on disk, a git
//! repository, and a registry that is only a directory.
//!
//! A task crate depends on `rituals`, and a project builds against the
//! checkout's own `rituals`. Two copies of that crate in one build are two
//! different crates to the compiler, so a task crate the project imports
//! must resolve `rituals` to the very package the project already uses: the
//! checkout's, by path, for a crate in a directory; the registry name patched
//! to the checkout, for a crate in a git repository or a registry, where a
//! path dependency would be re-read as part of that source.
//!
//! The registry is a Cargo `directory` source standing in for crates.io, so
//! an import by name and version never reaches the network. A directory
//! source has to hold every registry crate the project builds with, which is
//! what `cargo vendor` writes; it is filled from the project's own
//! dependencies once, then the story's task crates are added beside them.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::git::isolate_from_the_machine;
use super::{
    Checkout, Outcome, Project, ResultContext, TestOutcome, crates, path_to_str, read_text,
    write_text,
};

/// The text of a task crate's manifest, for a crate that declares itself a
/// task and depends on `rituals` as `rituals_dependency` says (the right
/// hand side of `rituals = …`).
fn task_manifest(crate_name: &str, version: &str, rituals_dependency: &str) -> String {
    format!(
        "[package]\nname = \"{crate_name}\"\nversion = \"{version}\"\nedition = \"2024\"\n\n\
         [dependencies]\nrituals = {rituals_dependency}\n\n\
         [package.metadata.ritual]\ntask = true\n"
    )
}

/// A task crate's source: a leaf that prints `<crate_name> <version> ran`,
/// so a story can tell which release of a crate answered.
fn task_lib(crate_name: &str, version: &str) -> String {
    crates::leaf_lib(&format!("{crate_name} {version}"))
}

/// What a task crate prints when it runs.
pub(crate) fn task_output(crate_name: &str, version: &str) -> String {
    crates::ran_line(&format!("{crate_name} {version}"))
}

/// Writes a task crate at `directory` that depends on the checkout's own
/// `rituals` by path. For a crate imported with `--path`.
pub(crate) fn write_path_task(
    directory: &Path,
    checkout: &Checkout,
    crate_name: &str,
    version: &str,
) -> TestOutcome {
    let rituals = checkout.root().join("crates").join("rituals");
    let dependency = format!("{{ path = {:?} }}", path_to_str(&rituals)?);
    crates::write_crate(
        directory,
        &task_manifest(crate_name, version, &dependency),
        &task_lib(crate_name, version),
    )
}

/// Writes a crate called `rituals` at `directory`, at `version`: a package
/// of the same name as the checkout's own `rituals`, which Rust reads as a
/// different crate whatever its version. Nothing builds it; a story that
/// uses it is about what ritual refuses before anything is built.
pub(crate) fn write_other_rituals(directory: &Path, version: &str) -> TestOutcome {
    crates::write_crate(
        directory,
        &format!("[package]\nname = \"rituals\"\nversion = \"{version}\"\nedition = \"2024\"\n"),
        "//! A crate called rituals that is not the checkout's.\n",
    )
}

/// Writes a task crate at `directory` that depends by path on the
/// `rituals` at `rituals_directory`, rather than on the checkout's own.
pub(crate) fn write_path_task_built_on(
    directory: &Path,
    rituals_directory: &Path,
    crate_name: &str,
    version: &str,
) -> TestOutcome {
    let dependency = format!("{{ path = {:?} }}", path_to_str(rituals_directory)?);
    crates::write_crate(
        directory,
        &task_manifest(crate_name, version, &dependency),
        &task_lib(crate_name, version),
    )
}

/// Writes a task crate at `directory` that names no `rituals` of its own:
/// it depends by path only on the task crate called `reexported` at
/// `reexported_directory`, and hands over that crate's task as its own.
pub(crate) fn write_facade_task(
    directory: &Path,
    crate_name: &str,
    reexported_directory: &Path,
    reexported: &str,
) -> TestOutcome {
    crates::write_crate(
        directory,
        &format!(
            "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [dependencies]\n{reexported} = {{ path = {:?} }}\n\n\
             [package.metadata.ritual]\ntask = true\n",
            path_to_str(reexported_directory)?
        ),
        &format!("//! A task re-exported from `{reexported}`.\n\npub use {reexported}::task;\n"),
    )
}

/// Writes a task crate at `directory` that depends on the `rituals` at
/// `rituals_directory` by path, as its own, and also on the crate called
/// `reexported` at `reexported_directory`, whose task it hands over as its
/// own: a facade whose direct `rituals` says nothing about the task's.
pub(crate) fn write_facade_task_with_own_rituals(
    directory: &Path,
    crate_name: &str,
    rituals_directory: &Path,
    reexported_directory: &Path,
    reexported: &str,
) -> TestOutcome {
    crates::write_crate(
        directory,
        &format!(
            "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nrituals = {{ path = {:?} }}\n{reexported} = {{ path = {:?} }}\n\n\
             [package.metadata.ritual]\ntask = true\n",
            path_to_str(rituals_directory)?,
            path_to_str(reexported_directory)?
        ),
        &format!("//! A task re-exported from `{reexported}`.\n\npub use {reexported}::task;\n"),
    )
}

/// Writes a crate at `directory` that declares itself a task but depends
/// on nothing, so no `rituals` is anywhere in what it is built with.
pub(crate) fn write_task_crate_without_rituals(directory: &Path, crate_name: &str) -> TestOutcome {
    crates::write_crate(
        directory,
        &format!(
            "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [package.metadata.ritual]\ntask = true\n"
        ),
        "//! A crate that declares itself a task and depends on nothing.\n",
    )
}

/// A git repository holding a crate called `rituals`, and the commit a
/// dependency pins it to.
pub(crate) struct GitRituals {
    /// The `file://` URL that clones the repository.
    pub(crate) url: String,
    /// The full id of the one commit in it.
    pub(crate) rev: String,
}

/// Creates a git repository at `directory` holding, at its root, a crate
/// called `rituals` at `version` (see [`write_other_rituals`]), committed.
pub(crate) fn write_git_rituals_repository(directory: &Path, version: &str) -> Outcome<GitRituals> {
    write_other_rituals(directory, version)?;
    git(directory, &["init", "--quiet", "--initial-branch", "main"])?;
    git(directory, &["add", "--all"])?;
    git(directory, &["commit", "--quiet", "--message", "a rituals"])?;
    let rev_parse = super::git::git(directory, &["rev-parse", "HEAD"])?;
    rev_parse.expect_success("`git rev-parse HEAD` in a rituals repository");
    Ok(GitRituals {
        url: format!("file://{}", path_to_str(directory)?),
        rev: rev_parse.stdout.trim().to_string(),
    })
}

/// Writes a task crate at `directory` that depends on the `rituals` in
/// `rituals`'s repository, pinned to its commit with `rev`.
pub(crate) fn write_task_built_on_git_rituals(
    directory: &Path,
    rituals: &GitRituals,
    crate_name: &str,
    version: &str,
) -> TestOutcome {
    let dependency = format!("{{ git = {:?}, rev = {:?} }}", rituals.url, rituals.rev);
    crates::write_crate(
        directory,
        &task_manifest(crate_name, version, &dependency),
        &task_lib(crate_name, version),
    )
}

/// Writes a plain crate at `directory` that never declares itself a task:
/// no `[package.metadata.ritual]` table, and no dependencies.
pub(crate) fn write_unmarked_crate(directory: &Path, crate_name: &str) -> TestOutcome {
    crates::write_crate(
        directory,
        &format!("[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        "//! A plain crate that is not a task.\n\n/// Does nothing.\npub fn placeholder() {}\n",
    )
}

/// The `rituals` requirement a registry or git task crate declares: the
/// version this suite was built against, which the project's patch to the
/// checkout satisfies.
fn registry_dependency() -> String {
    format!("{:?}", rituals::VERSION)
}

/// Runs `git <arguments…>` in `directory`, reading nothing from the machine
/// (see [`isolate_from_the_machine`]), so a story's repository is the same on
/// every machine and never waits on a signing prompt.
fn git(directory: &Path, arguments: &[&str]) -> TestOutcome {
    let output = isolate_from_the_machine(
        Command::new("git")
            .args(["-c", "user.name=ritual-tests"])
            .args(["-c", "user.email=ritual-tests@example.invalid"])
            .args(arguments)
            .current_dir(directory)
            .stdin(std::process::Stdio::null()),
    )
    .output()
    .context("spawning git failed")?;
    assert!(
        output.status.success(),
        "`git {}` failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

/// Creates a git repository at `directory` holding one task crate at its
/// root, committed, and returns the `file://` URL that clones it.
///
/// The crate depends on `rituals` by registry name, for the project's patch
/// to point at the checkout; see the module documentation.
pub(crate) fn write_git_task_repository(
    directory: &Path,
    crate_name: &str,
    version: &str,
) -> Outcome<String> {
    crates::write_crate(
        directory,
        &task_manifest(crate_name, version, &registry_dependency()),
        &task_lib(crate_name, version),
    )?;
    git(directory, &["init", "--quiet", "--initial-branch", "main"])?;
    git(directory, &["add", "--all"])?;
    git(
        directory,
        &["commit", "--quiet", "--message", "a task crate"],
    )?;
    Ok(format!("file://{}", path_to_str(directory)?))
}

/// A directory standing in for crates.io for one project.
pub(crate) struct LocalRegistry {
    directory: PathBuf,
}

impl LocalRegistry {
    /// Fills `directory` with every registry crate `project` builds with,
    /// points the project's Cargo at it in place of crates.io, and patches
    /// the registry's `rituals` to the checkout's so a crate that names it
    /// by version builds against the same `rituals` the project does.
    ///
    /// Resolving the project's own dependencies to fill the directory is
    /// the one step that needs crates.io or a warm Cargo cache, as every
    /// story that builds a project does; nothing after it does.
    pub(crate) fn install(
        project: &Project,
        checkout: &Checkout,
        directory: &Path,
    ) -> Outcome<Self> {
        let registry = Self::install_unpatched(project, directory)?;

        let rituals = checkout.root().join("crates").join("rituals");
        let manifest_path = project.workspace_manifest_path();
        let manifest = read_text(&manifest_path)?;
        write_text(
            &manifest_path,
            &format!(
                "{manifest}\n[patch.crates-io]\nrituals = {{ path = {:?} }}\n",
                path_to_str(&rituals)?
            ),
        )?;

        Ok(registry)
    }

    /// [`Self::install`] without the patch: a registry crate that names
    /// `rituals` by version resolves it from the registry, as a project
    /// whose own `rituals` comes from a path would see a task from
    /// crates.io do. The two are then two packages of one name.
    pub(crate) fn install_unpatched(project: &Project, directory: &Path) -> Outcome<Self> {
        project
            .cargo(&["vendor", path_to_str(directory)?])?
            .expect_success("`cargo vendor` of the project's own dependencies");

        let config_path = project.root().join(".cargo/config.toml");
        let config = read_text(&config_path)?;
        write_text(
            &config_path,
            &format!(
                "{config}\n[source.crates-io]\nreplace-with = \"local-registry\"\n\n\
                 [source.local-registry]\ndirectory = {:?}\n",
                path_to_str(directory)?
            ),
        )?;

        Ok(Self {
            directory: directory.to_path_buf(),
        })
    }

    /// Publishes one release of a task crate into the registry.
    pub(crate) fn publish_task(&self, crate_name: &str, version: &str) -> TestOutcome {
        self.publish(
            crate_name,
            version,
            &task_manifest(crate_name, version, &registry_dependency()),
            &task_lib(crate_name, version),
        )
    }

    /// Publishes a release of `rituals` into the registry, and one release
    /// of a task crate built for exactly that release. With the project's
    /// patch to the checkout, a release other than the one this suite was
    /// built against is one the patch does not reach; without it, any
    /// release is the registry's own.
    pub(crate) fn publish_task_built_for_rituals(
        &self,
        crate_name: &str,
        version: &str,
        rituals_version: &str,
    ) -> TestOutcome {
        self.publish(
            "rituals",
            rituals_version,
            &format!(
                "[package]\nname = \"rituals\"\nversion = \"{rituals_version}\"\n\
                 edition = \"2024\"\n"
            ),
            "//! A release of rituals this suite was not built against.\n",
        )?;
        self.publish(
            crate_name,
            version,
            &task_manifest(crate_name, version, &format!("\"={rituals_version}\"")),
            &task_lib(crate_name, version),
        )
    }

    /// Writes one crate into the registry, as `cargo vendor` lays it out.
    fn publish(&self, crate_name: &str, version: &str, manifest: &str, lib: &str) -> TestOutcome {
        let crate_dir = self.directory.join(format!("{crate_name}-{version}"));
        crates::write_crate(&crate_dir, manifest, lib)?;
        // A directory source reads this file to tell a vendored crate from a
        // hand-edited one; a null package checksum is what a crate with no
        // registry origin carries.
        fs::write(
            crate_dir.join(".cargo-checksum.json"),
            "{\"files\":{},\"package\":null}\n",
        )
        .context("writing the crate's .cargo-checksum.json failed")?;
        Ok(())
    }
}

/// Commits a later release of the task crate in the repository at
/// `directory`, replacing the files [`write_git_task_repository`] wrote.
pub(crate) fn commit_task_release(
    directory: &Path,
    crate_name: &str,
    version: &str,
) -> TestOutcome {
    crates::write_crate(
        directory,
        &task_manifest(crate_name, version, &registry_dependency()),
        &task_lib(crate_name, version),
    )?;
    git(directory, &["add", "--all"])?;
    git(
        directory,
        &["commit", "--quiet", "--message", "a later release"],
    )
}

/// Tags the repository's current commit as `tag`.
pub(crate) fn tag_current_commit(directory: &Path, tag: &str) -> TestOutcome {
    git(directory, &["tag", tag])
}
