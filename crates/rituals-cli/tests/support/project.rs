//! A project scaffolded by `ritual new`, and the ways a story acts on it.

use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

use super::crates::{self, Child};
use super::process::{built_binary_path, cargo, run_binary, run_ritual};
use super::{Checkout, Outcome, RunOutput, TestOutcome, failure, manifest, path_to_str, read_text};

/// A project `ritual new` scaffolded inside a story's own temporary
/// directory.
pub(crate) struct Project {
    root: PathBuf,
    composed_cli_dir: PathBuf,
}

impl Project {
    /// Runs `ritual new <name> --path <checkout> <options…>` in
    /// `working_dir`, asserts it succeeded, and returns the project it
    /// scaffolded at `working_dir/<name>`.
    pub(crate) fn scaffold(
        checkout: &Checkout,
        working_dir: &Path,
        name: &str,
        options: &[&str],
    ) -> Outcome<Self> {
        let mut arguments = vec!["new", name, "--path", checkout.path_argument()?];
        arguments.extend_from_slice(options);
        run_ritual(working_dir, &arguments)?
            .expect_success(&format!("`ritual {}`", arguments.join(" ")));

        Self::open(&working_dir.join(name))
    }

    /// The project `ritual new` already scaffolded at `root`, for a story
    /// that ran `new` itself to read what it printed.
    pub(crate) fn open(root: &Path) -> Outcome<Self> {
        let composed_cli_dir = find_composed_cli_crate_dir(root)?;
        Ok(Self {
            root: root.to_path_buf(),
            composed_cli_dir,
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The composed CLI crate's own directory, found by role — the one crate
    /// whose manifest declares a `[[bin]]` target — rather than by name.
    pub(crate) fn composed_cli_dir(&self) -> &Path {
        &self.composed_cli_dir
    }

    /// Where this project's builds put their final binaries: its own
    /// `target/`, the directory Cargo would use by default, which belongs
    /// to this one project in this one test.
    pub(crate) fn target_dir(&self) -> PathBuf {
        self.root.join("target")
    }

    pub(crate) fn workspace_manifest_path(&self) -> PathBuf {
        self.root.join("Cargo.toml")
    }

    pub(crate) fn cli_manifest_path(&self) -> PathBuf {
        self.composed_cli_dir.join("Cargo.toml")
    }

    /// The composed CLI's generated command-line file.
    pub(crate) fn generated_file_path(&self) -> PathBuf {
        self.composed_cli_dir.join("src/main.rs")
    }

    pub(crate) fn workspace_manifest(&self) -> Outcome<DocumentMut> {
        manifest::read(&self.workspace_manifest_path())
    }

    pub(crate) fn cli_manifest(&self) -> Outcome<DocumentMut> {
        manifest::read(&self.cli_manifest_path())
    }

    pub(crate) fn generated_file(&self) -> Outcome<String> {
        read_text(&self.generated_file_path())
    }

    /// The project's `.cargo/config.toml`, where `new` writes its alias.
    pub(crate) fn cargo_config(&self) -> Outcome<DocumentMut> {
        manifest::read(&self.root.join(".cargo/config.toml"))
    }

    /// The name the composed CLI's manifest gives its one `[[bin]]` target,
    /// read fresh each time — a story that renames it by hand gets the new
    /// name from here as soon as the manifest says so.
    pub(crate) fn bin_name(&self) -> Outcome<String> {
        manifest::sole_bin_name(&self.cli_manifest()?)
    }

    /// Runs `cargo <arguments>` at the project root, building into
    /// [`Self::target_dir`].
    pub(crate) fn cargo(&self, arguments: &[&str]) -> Outcome<RunOutput> {
        cargo(&self.root, &self.target_dir(), arguments)
    }

    /// Runs the project's own `cargo <bin> <arguments>` alias — the command a
    /// person working in the project types.
    ///
    /// The alias is `cargo run`, so its stderr carries Cargo's own progress
    /// lines ahead of the program's: read stdout and the exit code from it,
    /// and take any claim about the program's stderr from [`Self::run_cli`].
    pub(crate) fn alias(&self, arguments: &[&str]) -> Outcome<RunOutput> {
        let bin_name = self.bin_name()?;
        let mut full_arguments = vec![bin_name.as_str()];
        full_arguments.extend_from_slice(arguments);
        self.cargo(&full_arguments)
    }

    /// Builds the composed CLI as the manifest currently names it and
    /// returns the built binary's path, asserting the build succeeded.
    pub(crate) fn build(&self) -> Outcome<PathBuf> {
        let bin_name = self.bin_name()?;
        self.cargo(&["build", "--bin", &bin_name])?
            .expect_success(&format!("building the project's `{bin_name}` binary"));
        Ok(built_binary_path(&self.target_dir(), &bin_name))
    }

    /// Builds the composed CLI, then runs the built binary directly with
    /// `arguments` at the project root.
    ///
    /// The same thing `cargo run` does, except that what comes back is the
    /// program's own output alone: this is the call to make whenever a story
    /// reads stderr.
    pub(crate) fn run_cli(&self, arguments: &[&str]) -> Outcome<RunOutput> {
        let binary = self.build()?;
        run_binary(&binary, &self.root, arguments)
    }

    /// Writes a hand-written leaf crate under `tasks/<crate_name>` and adds
    /// it to the workspace's members. Returns the crate's directory.
    pub(crate) fn write_leaf(&self, crate_name: &str) -> Outcome<PathBuf> {
        self.write_member(
            crate_name,
            &crates::leaf_manifest(crate_name),
            &crates::leaf_lib(crate_name),
        )
    }

    /// Writes a hand-written bundle crate under `tasks/<crate_name>`,
    /// described by `about` and grouping `children`, and adds it to the
    /// workspace's members. Any [`Child::Crate`] must already be a member.
    /// Returns the crate's directory.
    pub(crate) fn write_bundle(
        &self,
        crate_name: &str,
        about: &str,
        children: &[Child<'_>],
    ) -> Outcome<PathBuf> {
        self.write_member(
            crate_name,
            &crates::bundle_manifest(crate_name, children),
            &crates::bundle_lib(about, children),
        )
    }

    fn write_member(&self, crate_name: &str, manifest: &str, lib: &str) -> Outcome<PathBuf> {
        let member = format!("tasks/{crate_name}");
        let crate_dir = self.root.join(&member);
        crates::write_crate(&crate_dir, manifest, lib)?;
        manifest::edit(&self.workspace_manifest_path(), |document| {
            manifest::push_member(document, &member)
        })?;
        Ok(crate_dir)
    }

    /// Mounts the workspace member at `crate_dir`, package `crate_name`, on
    /// the composed CLI under `mount_key`: a dependency under that key, and
    /// the key appended to `[package.metadata.ritual] tasks`.
    ///
    /// The two edits `add` makes, made here by hand — `add` only mounts a
    /// crate it scaffolded itself, and only under that crate's own name. The
    /// dependency goes in through `cargo add`, with `--rename` when the key
    /// differs from the package name, the ordinary way to import a crate
    /// under another name. The generated file is left for `regenerate`.
    pub(crate) fn mount(&self, crate_dir: &Path, mount_key: &str, crate_name: &str) -> TestOutcome {
        let crate_dir_argument = path_to_str(crate_dir)?;
        let mut arguments = vec!["add", "--path", crate_dir_argument];
        if mount_key != crate_name {
            arguments.extend(["--rename", mount_key]);
        }
        cargo(&self.composed_cli_dir, &self.target_dir(), &arguments)?
            .expect_success(&format!("`cargo {}`", arguments.join(" ")));

        manifest::edit(&self.cli_manifest_path(), |document| {
            manifest::push_task(document, mount_key)
        })
    }
}

/// Finds the one crate under `project_root` whose manifest declares a
/// `[[bin]]` target — the composed CLI — and returns its directory.
fn find_composed_cli_crate_dir(project_root: &Path) -> Outcome<PathBuf> {
    let mut composed_cli_dirs = Vec::new();
    for path in super::tree::files_under(project_root)? {
        if path.file_name().is_some_and(|name| name == "Cargo.toml")
            && !manifest::bin_names(&manifest::read(&path)?).is_empty()
        {
            composed_cli_dirs.push(super::process::parent_of(&path)?.to_path_buf());
        }
    }
    match composed_cli_dirs.as_slice() {
        [only] => Ok(only.clone()),
        other => failure(format!(
            "expected exactly one crate with a [[bin]] target under {}, found {other:?}",
            project_root.display()
        )),
    }
}
