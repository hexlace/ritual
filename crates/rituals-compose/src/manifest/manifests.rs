//! The two manifests a task edits in a project: the composed command line's,
//! and the workspace's, which are one file when the command line is the
//! workspace root.

use std::path::PathBuf;

use rituals::Failure;

use super::Manifest;
use crate::metadata::Project;

/// Where the two manifests a task edits are on disk.
///
/// A plain carrier of two paths of one type, with named fields so that
/// swapping them is visible at the call: the composed command line's
/// manifest and the workspace's. They are the same path when the command
/// line is the workspace root.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ManifestPaths {
    /// The composed command line crate's `Cargo.toml`.
    pub cli: PathBuf,
    /// The workspace root's `Cargo.toml`, the one holding `[workspace]`.
    pub workspace: PathBuf,
}

impl ManifestPaths {
    /// Returns where `project`'s two manifests are.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::manifest::{ManifestPaths, Manifests};
    /// use rituals_compose::metadata;
    ///
    /// // `Project` is only ever built by `Metadata::locate_project`, which
    /// // needs a real `cargo metadata` call, so this example stays `no_run`.
    /// let document = metadata::fetch(Path::new("."))?;
    /// let project = document.locate_project("demo-ritual")?;
    /// let manifests = Manifests::read(&ManifestPaths::of(&project))?;
    /// println!("the workspace is declared in {}", manifests.workspace().path().display());
    /// # Ok::<(), rituals::Failure>(())
    /// ```
    #[must_use]
    pub fn of(project: &Project<'_>) -> Self {
        Self {
            cli: project.manifest_path().to_path_buf(),
            workspace: project.workspace_root().join("Cargo.toml"),
        }
    }
}

/// The manifests a task edits: the composed command line's, and the
/// workspace's.
///
/// They are one file when the composed command line is the workspace root.
/// Two documents read from one file would each be written back whole, and the
/// second write would discard the first's edits, so then there is one
/// document and the workspace's edits go to it.
///
/// # Examples
///
/// A project whose command line is its workspace root has one document, so
/// there is one file to write:
///
/// ```
/// use rituals_compose::manifest::{ManifestPaths, Manifests};
///
/// # let directory = std::env::temp_dir()
/// #     .join(format!("rituals-compose-doctest-manifests-{}", std::process::id()));
/// # std::fs::create_dir_all(&directory)?;
/// let path = directory.join("Cargo.toml");
/// # std::fs::write(&path, "[package]\nname = \"demo\"\n\n[workspace]\nmembers = []\n")?;
/// let manifests = Manifests::read(&ManifestPaths { cli: path.clone(), workspace: path })?;
///
/// assert!(manifests.separate_workspace().is_none());
/// # std::fs::remove_dir_all(&directory)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct Manifests {
    cli: Manifest,
    workspace: Option<Manifest>,
}

impl Manifests {
    /// Reads the composed command line's manifest, and the workspace's too
    /// when it is a different file.
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming the path of whichever manifest cannot be
    /// read or does not parse.
    pub fn read(paths: &ManifestPaths) -> Result<Self, Failure> {
        let cli = Manifest::read(&paths.cli)?;
        let workspace = if paths.cli == paths.workspace {
            None
        } else {
            Some(Manifest::read(&paths.workspace)?)
        };
        Ok(Self { cli, workspace })
    }

    /// The manifest that holds `[workspace]`.
    #[must_use]
    pub const fn workspace(&self) -> &Manifest {
        match &self.workspace {
            Some(workspace) => workspace,
            None => &self.cli,
        }
    }

    /// The manifest that holds `[workspace]`, to edit.
    #[must_use]
    pub fn workspace_mut(&mut self) -> &mut Manifest {
        self.workspace.as_mut().unwrap_or(&mut self.cli)
    }

    /// The composed command line's manifest.
    #[must_use]
    pub const fn cli(&self) -> &Manifest {
        &self.cli
    }

    /// The composed command line's manifest, to edit.
    #[must_use]
    pub const fn cli_mut(&mut self) -> &mut Manifest {
        &mut self.cli
    }

    /// The workspace's manifest when it is a different file from the
    /// command line's: the one a caller writes in addition to [`cli`].
    ///
    /// [`cli`]: Manifests::cli
    #[must_use]
    pub const fn separate_workspace(&self) -> Option<&Manifest> {
        self.workspace.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use toml_edit::DocumentMut;

    use super::{Manifest, ManifestPaths, Manifests};
    use crate::rollback::{self, Wording};
    use crate::test_support::{ScratchDir, TestOutcome};

    const CLI: &str = "[package]\nname = \"demo-ritual\"\n";
    const WORKSPACE: &str = "[workspace]\nmembers = []\n";

    fn write(path: &Path, text: &str) -> TestOutcome {
        std::fs::write(path, text)?;
        Ok(())
    }

    /// Writes `manifest` the way a task does, inside a run, so the file on
    /// disk is what a person would see afterwards.
    fn write_through_a_run(manifest: &Manifest) -> TestOutcome {
        rollback::attempt(Wording::project("running `create lint` again"), |changes| {
            manifest.write(changes)
        })?;
        Ok(())
    }

    /// One path for both manifests reads one document: an edit made through
    /// the workspace accessor lands in the file `cli()` writes, whole, with
    /// the package table beside it, and there is no second file to write. The
    /// file is read back with `toml_edit`, the grammar Cargo's manifests are
    /// written in.
    #[test]
    fn one_path_for_both_gives_one_document_written_once() -> TestOutcome {
        let scratch = ScratchDir::new("manifests-one-file")?;
        let path = scratch.path().join("Cargo.toml");
        write(&path, &format!("{CLI}\n{WORKSPACE}"))?;
        let mut manifests = Manifests::read(&ManifestPaths {
            cli: path.clone(),
            workspace: path.clone(),
        })?;

        manifests
            .workspace_mut()
            .append_workspace_member(".rituals/lint")?;
        write_through_a_run(manifests.cli())?;

        assert!(manifests.separate_workspace().is_none());
        let written: DocumentMut = std::fs::read_to_string(&path)?.parse()?;
        assert_eq!(
            written["package"]["name"].as_str(),
            Some("demo-ritual"),
            "the package table is still there"
        );
        let members: Vec<&str> = written["workspace"]["members"]
            .as_array()
            .map(|members| {
                members
                    .iter()
                    .filter_map(toml_edit::Value::as_str)
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(members, [".rituals/lint"]);
        Ok(())
    }

    /// Two paths read two documents, and an edit through the workspace
    /// accessor reaches the workspace's file and not the command line's.
    #[test]
    fn two_paths_give_two_documents() -> TestOutcome {
        let scratch = ScratchDir::new("manifests-two-files")?;
        let cli = scratch.path().join("ritual.toml");
        let workspace = scratch.path().join("Cargo.toml");
        write(&cli, CLI)?;
        write(&workspace, WORKSPACE)?;
        let mut manifests = Manifests::read(&ManifestPaths {
            cli: cli.clone(),
            workspace: workspace.clone(),
        })?;

        manifests
            .workspace_mut()
            .append_workspace_member(".rituals/lint")?;
        let separate = manifests
            .separate_workspace()
            .ok_or("two paths give a separate workspace manifest")?;
        write_through_a_run(separate)?;
        write_through_a_run(manifests.cli())?;

        assert_eq!(manifests.cli().path(), cli);
        assert_eq!(manifests.workspace().path(), workspace);
        assert!(std::fs::read_to_string(&workspace)?.contains(".rituals/lint"));
        assert_eq!(std::fs::read_to_string(&cli)?, CLI);
        Ok(())
    }

    #[test]
    fn a_manifest_that_cannot_be_read_is_a_failure_naming_it() -> TestOutcome {
        let scratch = ScratchDir::new("manifests-missing")?;
        let cli = scratch.path().join("Cargo.toml");
        write(&cli, CLI)?;
        let missing = scratch.path().join("missing/Cargo.toml");

        let refused = Manifests::read(&ManifestPaths {
            cli,
            workspace: missing.clone(),
        });

        assert!(
            refused.err().is_some_and(|failure| failure
                .to_string()
                .contains(&missing.display().to_string())),
            "expected the failure to name {}",
            missing.display()
        );
        Ok(())
    }
}
