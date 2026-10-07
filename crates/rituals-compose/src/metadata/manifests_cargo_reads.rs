//! Every manifest Cargo reads to make sense of a project.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rituals::Failure;

use super::Metadata;
use super::manifest_walk::walk_manifests;
use crate::paths::normalize;

impl Metadata {
    /// Lists the manifest of every crate Cargo reads to understand this
    /// project, sorted, each once: the workspace's own, every package at a
    /// path on disk, and every crate those reach through a path dependency
    /// of any kind, however far. A `[workspace.dependencies]`, `[patch]` or
    /// `[replace]` entry is followed only from the workspace's own manifest,
    /// because Cargo ignores one written anywhere else.
    ///
    /// Cargo reads more than `cargo metadata` lists. A crate outside the
    /// workspace, reached only through an optional dependency no feature
    /// turns on, is read when Cargo resolves the lockfile and is not among
    /// the packages; neither is a crate that crate reaches in turn. Their
    /// dependencies are followed here from the TOML each one is written in,
    /// because `cargo metadata --no-deps` refuses a crate that sits under the
    /// workspace's root without being a member.
    ///
    /// A path dependency whose directory holds no `Cargo.toml` is left out:
    /// Cargo did not need it to read the project, or this document could not
    /// have been produced. A package from a registry or a git repository is
    /// not the project's own, however its files were unpacked, and neither
    /// are its dependencies.
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming the manifest when one of these exists
    /// but cannot be read or does not parse as TOML.
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
    /// for manifest in document.manifests_cargo_reads()? {
    ///     println!("Cargo reads {}", manifest.display());
    /// }
    /// # Ok::<(), rituals::Failure>(())
    /// ```
    pub fn manifests_cargo_reads(&self) -> Result<Vec<PathBuf>, Failure> {
        let root_manifest = normalize(&self.workspace_root.join("Cargo.toml"));
        let mut starts: Vec<PathBuf> = vec![root_manifest.clone()];
        starts.extend(
            self.path_package_manifests()
                .into_iter()
                .map(Path::to_path_buf),
        );

        let mut read: BTreeSet<PathBuf> = BTreeSet::new();
        walk_manifests(&root_manifest, starts, BTreeSet::new(), None, |reached| {
            read.insert(reached.path.to_path_buf());
        })?;
        Ok(read.into_iter().collect())
    }

    /// The manifest of every package that lives at a path on disk, sorted,
    /// each once.
    ///
    /// A path package is one with no source: a workspace member, and a path
    /// dependency that is not a member, such as one reached from outside the
    /// workspace's own directory. A package from a registry or a git
    /// repository is not the project's own, however its files were unpacked,
    /// and is left out.
    pub(super) fn path_package_manifests(&self) -> Vec<&Path> {
        let mut manifests: Vec<&Path> = self
            .packages
            .iter()
            .filter(|package| package.source.is_none())
            .map(|package| package.manifest_path.as_path())
            .collect();
        manifests.sort_unstable();
        manifests.dedup();
        manifests
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::fetch;
    use crate::test_support::{ScratchDir, TestOutcome};

    fn write(root: &Path, files: &[(&str, &str)]) -> TestOutcome {
        for (path, contents) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().ok_or("a fixture file has a directory")?)?;
            std::fs::write(path, contents)?;
        }
        Ok(())
    }

    fn package(name: &str, extra: &str) -> String {
        format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n{extra}")
    }

    /// What Cargo reads in the project at `root`, spelled from `root`.
    fn read_in(root: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let root = std::fs::canonicalize(root)?;
        let manifests = fetch(&root)?.manifests_cargo_reads()?;
        manifests
            .iter()
            .map(|manifest| Ok(manifest.strip_prefix(&root)?.display().to_string()))
            .collect()
    }

    /// A workspace of one member, `cli`, that depends on `vendor/x` through
    /// an optional dependency. `vendor/x` is excluded from the workspace, so
    /// `cargo metadata` does not list it, and it depends on `tasks/greet`.
    fn project_with_an_excluded_optional_crate(root: &Path) -> TestOutcome {
        write(
            root,
            &[
                (
                    "Cargo.toml",
                    "[workspace]\nmembers = [\"cli\", \"tasks/greet\"]\nexclude = [\"vendor/x\"]\n\
                     resolver = \"3\"\n",
                ),
                (
                    "cli/Cargo.toml",
                    &package(
                        "cli",
                        "\n[dependencies]\nx = { path = \"../vendor/x\", optional = true }\n",
                    ),
                ),
                ("cli/src/lib.rs", ""),
                ("tasks/greet/Cargo.toml", &package("greet", "")),
                ("tasks/greet/src/lib.rs", ""),
                (
                    "vendor/x/Cargo.toml",
                    &package(
                        "x",
                        "\n[dependencies]\ngreet = { path = \"../../tasks/greet\" }\n",
                    ),
                ),
                ("vendor/x/src/lib.rs", ""),
            ],
        )
    }

    /// An excluded crate reached only through an optional dependency is not
    /// in `cargo metadata`'s packages, and Cargo still reads it, so it is
    /// listed.
    #[test]
    fn a_crate_outside_the_workspace_reached_by_an_optional_dependency_is_read() -> TestOutcome {
        let scratch = ScratchDir::new("reads-excluded-optional")?;
        project_with_an_excluded_optional_crate(scratch.path())?;
        let root = std::fs::canonicalize(scratch.path())?;
        let listed = fetch(&root)?.workspace_members().len();
        assert_eq!(listed, 2, "fixture precondition: x is not a member");

        assert_eq!(
            read_in(scratch.path())?,
            [
                "Cargo.toml",
                "cli/Cargo.toml",
                "tasks/greet/Cargo.toml",
                "vendor/x/Cargo.toml"
            ]
        );
        Ok(())
    }

    /// `vendor/y` is reached by `vendor/x`, sits under the root and is not
    /// excluded, so asking `cargo metadata --no-deps` about it fails with
    /// `current package believes it's in a workspace when it's not`. It is
    /// listed anyway, because the walk reads TOML.
    #[test]
    fn a_crate_reached_through_another_outside_the_workspace_is_read() -> TestOutcome {
        let scratch = ScratchDir::new("reads-chain")?;
        let root = scratch.path();
        project_with_an_excluded_optional_crate(root)?;
        write(
            root,
            &[
                (
                    "vendor/x/Cargo.toml",
                    &package("x", "\n[dependencies]\ny = { path = \"../y\" }\n"),
                ),
                (
                    "vendor/y/Cargo.toml",
                    &package(
                        "y",
                        "\n[dependencies]\ngreet = { path = \"../../tasks/greet\" }\n",
                    ),
                ),
                ("vendor/y/src/lib.rs", ""),
            ],
        )?;

        assert_eq!(
            read_in(root)?,
            [
                "Cargo.toml",
                "cli/Cargo.toml",
                "tasks/greet/Cargo.toml",
                "vendor/x/Cargo.toml",
                "vendor/y/Cargo.toml"
            ]
        );
        Ok(())
    }

    /// Cargo does not read a non-member's dev-dependencies, and a repoint
    /// that left them would leave them stale, so they are listed too.
    #[test]
    fn a_target_specific_dev_dependency_of_an_outside_crate_is_followed() -> TestOutcome {
        let scratch = ScratchDir::new("reads-dev")?;
        let root = scratch.path();
        project_with_an_excluded_optional_crate(root)?;
        write(
            root,
            &[
                (
                    "vendor/x/Cargo.toml",
                    &package(
                        "x",
                        "\n[target.'cfg(unix)'.dev-dependencies]\n\
                         tools = { path = \"../tools\" }\n",
                    ),
                ),
                ("vendor/tools/Cargo.toml", &package("tools", "")),
                ("vendor/tools/src/lib.rs", ""),
            ],
        )?;

        assert!(
            read_in(root)?.contains(&"vendor/tools/Cargo.toml".to_string()),
            "a dev-dependency under a target table is followed"
        );
        Ok(())
    }

    /// A `[patch]` crate nothing uses is not read by Cargo's resolution, and
    /// its own path dependencies still lead to what a move changes.
    #[test]
    fn an_unused_patch_crate_and_what_it_reaches_are_read() -> TestOutcome {
        let scratch = ScratchDir::new("reads-patch")?;
        let root = scratch.path();
        project_with_an_excluded_optional_crate(root)?;
        write(
            root,
            &[
                (
                    "Cargo.toml",
                    "[workspace]\nmembers = [\"cli\", \"tasks/greet\"]\nexclude = [\"vendor/x\"]\n\
                     resolver = \"3\"\n\n[patch.crates-io]\n\
                     unused = { path = \"patches/unused\" }\n",
                ),
                (
                    "patches/unused/Cargo.toml",
                    &package(
                        "unused",
                        "\n[dependencies]\ngreet = { path = \"../../tasks/greet\" }\n",
                    ),
                ),
                ("patches/unused/src/lib.rs", ""),
            ],
        )?;

        assert!(
            read_in(root)?.contains(&"patches/unused/Cargo.toml".to_string()),
            "a patch crate is read"
        );
        Ok(())
    }

    /// Cargo reads `[patch]` and `[replace]` in the workspace's root manifest
    /// alone, and ignores them in a member's and in a crate outside the
    /// workspace, so the crates only those reach are not read.
    #[test]
    fn a_patch_or_replace_outside_the_root_manifest_is_not_followed() -> TestOutcome {
        let scratch = ScratchDir::new("reads-ignored-patch")?;
        let root = scratch.path();
        project_with_an_excluded_optional_crate(root)?;
        write(
            root,
            &[
                (
                    "cli/Cargo.toml",
                    &package(
                        "cli",
                        "\n[dependencies]\nx = { path = \"../vendor/x\", optional = true }\n\
                         \n[patch.crates-io]\npatched = { path = \"../ignored/patched\" }\n",
                    ),
                ),
                (
                    "vendor/x/Cargo.toml",
                    &package(
                        "x",
                        "\n[replace]\n\"replaced:0.1.0\" = { path = \"../../ignored/replaced\" }\n",
                    ),
                ),
                ("ignored/patched/Cargo.toml", &package("patched", "")),
                ("ignored/patched/src/lib.rs", ""),
                ("ignored/replaced/Cargo.toml", &package("replaced", "")),
                ("ignored/replaced/src/lib.rs", ""),
            ],
        )?;

        assert_eq!(
            read_in(root)?,
            [
                "Cargo.toml",
                "cli/Cargo.toml",
                "tasks/greet/Cargo.toml",
                "vendor/x/Cargo.toml"
            ]
        );
        Ok(())
    }

    /// A path dependency nothing is at is skipped: Cargo read the project
    /// without it.
    #[test]
    fn a_path_dependency_with_no_manifest_is_skipped() -> TestOutcome {
        let scratch = ScratchDir::new("reads-missing")?;
        let root = scratch.path();
        project_with_an_excluded_optional_crate(root)?;
        write(
            root,
            &[(
                "vendor/x/Cargo.toml",
                &package("x", "\n[dev-dependencies]\ngone = { path = \"../gone\" }\n"),
            )],
        )?;

        let manifests = read_in(root)?;

        assert!(
            manifests
                .iter()
                .all(|manifest| !manifest.starts_with("vendor/gone")),
            "{manifests:?}"
        );
        assert!(manifests.contains(&"vendor/x/Cargo.toml".to_string()));
        Ok(())
    }

    /// A manifest Cargo reaches that does not parse is a failure that names
    /// it, rather than a crate quietly left stale.
    #[test]
    fn a_manifest_that_does_not_parse_is_a_failure_naming_it() -> TestOutcome {
        let scratch = ScratchDir::new("reads-unparsable")?;
        let root = scratch.path();
        project_with_an_excluded_optional_crate(root)?;
        write(
            root,
            &[
                (
                    "vendor/x/Cargo.toml",
                    &package(
                        "x",
                        "\n[dev-dependencies]\nbroken = { path = \"../broken\" }\n",
                    ),
                ),
                ("vendor/broken/Cargo.toml", "[package\nname = \n"),
            ],
        )?;
        let root = std::fs::canonicalize(root)?;
        let document = fetch(&root)?;

        let failure = document
            .manifests_cargo_reads()
            .err()
            .ok_or("expected the unparsable manifest to be refused")?;

        assert!(
            failure
                .with_causes()
                .to_string()
                .contains(&root.join("vendor/broken/Cargo.toml").display().to_string()),
            "{failure}"
        );
        Ok(())
    }
}
