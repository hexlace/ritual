//! The directories a manifest's path dependencies lead to.
//!
//! Cargo reads a path dependency from a good many places, and a crate it
//! reaches that way can be one the workspace never lists. Some of those
//! places it reads only in the workspace's root manifest. The places are the
//! one list in [`super::dependency_places`], shared by this and by what
//! repoints them.

use std::collections::BTreeSet;
use std::path::PathBuf;

use toml_edit::{Item, TableLike};

use super::Manifest;
use super::dependency_places::{ManifestRole, places_cargo_reads};
use crate::paths::normalize;

impl Manifest {
    /// Lists the directory every path dependency Cargo reads in this
    /// manifest leads to, when the manifest is `role` to its workspace, each
    /// spelled from this manifest's own directory and normalised, sorted,
    /// each once.
    ///
    /// A path dependency is read wherever Cargo reads one: a dependency's
    /// `path` in `dependencies`, `dev-dependencies` and `build-dependencies`
    /// and their underscore spellings, at the top level and in every
    /// `[target.<t>]` table, optional or not; and, in the workspace's root
    /// manifest alone, `[workspace.dependencies]`, every `[patch.<source>]`
    /// entry and every `[replace]` entry, which Cargo ignores anywhere else.
    /// A dependency with no `path`, or one that is not a string, leads
    /// nowhere and is left out.
    ///
    /// The directories are those the manifest names. Whether anything is
    /// there is not asked.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::PathBuf;
    ///
    /// use rituals_compose::manifest::{Manifest, ManifestRole};
    ///
    /// # let directory = std::env::temp_dir().join(format!(
    /// #     "rituals-compose-doctest-path-dependencies-{}",
    /// #     std::process::id()
    /// # ));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(
    /// #     &manifest_path,
    /// #     "[dependencies]\nserde = \"1\"\ngreet = { path = \"tasks/greet\", optional = true }\n\
    /// #      \n[target.'cfg(unix)'.dev-dependencies]\n\
    /// #      shout = { path = \"tasks/../tasks/shout\" }\n",
    /// # )?;
    /// let manifest = Manifest::read(&manifest_path)?;
    ///
    /// assert_eq!(
    ///     manifest.path_dependency_directories(ManifestRole::Other),
    ///     [directory.join("tasks/greet"), directory.join("tasks/shout")]
    /// );
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn path_dependency_directories(&self, role: ManifestRole) -> Vec<PathBuf> {
        let base = self.directory();
        let root = self.document.as_table();
        let mut written: Vec<&str> = Vec::new();

        for place in places_cargo_reads(role) {
            for located in place.tables(root) {
                paths_of(located.table, &mut written);
            }
        }

        written
            .into_iter()
            .map(|path| normalize(&base.join(path)))
            .collect::<BTreeSet<PathBuf>>()
            .into_iter()
            .collect()
    }
}

/// Collects the string `path` of every declaration in `declarations`.
fn paths_of<'a>(declarations: &'a dyn TableLike, written: &mut Vec<&'a str>) {
    for (_key, declaration) in declarations.iter() {
        if let Some(path) = declaration
            .as_table_like()
            .and_then(|declaration| declaration.get("path"))
            .and_then(Item::as_str)
        {
            written.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Manifest, ManifestRole};
    use crate::test_support::{ScratchDir, TestOutcome};

    /// The directories a manifest holding `content`, at `at` in a scratch
    /// directory and `role` to its workspace, leads to, each spelled from the
    /// scratch directory.
    fn led_to(
        tag: &str,
        at: &str,
        role: ManifestRole,
        content: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new(tag)?;
        let path = scratch.path().join(at);
        std::fs::create_dir_all(path.parent().ok_or("a manifest has a directory")?)?;
        std::fs::write(&path, content)?;
        let directories = Manifest::read(&path)?.path_dependency_directories(role);
        directories
            .iter()
            .map(|directory| {
                let relative = directory.strip_prefix(scratch.path())?;
                Ok(relative.display().to_string())
            })
            .collect()
    }

    /// A dependency in every place a manifest writes one: each table, a
    /// `[target.<t>]` table, the workspace's, a patch and a replacement, and
    /// the three shapes a dependency is written in.
    const EVERY_PLACE: &str = "[dependencies]\n\
             normal = { path = \"normal\" }\n\
             dotted.path = \"dotted\"\n\
             serde = \"1\"\n\
             \n\
             [dev-dependencies]\n\
             developed = { path = \"developed\" }\n\
             \n\
             [build-dependencies.built]\n\
             path = \"built\"\n\
             \n\
             [dev_dependencies]\n\
             underscored = { path = \"underscored\" }\n\
             \n\
             [target.'cfg(unix)'.dependencies]\n\
             targeted = { path = \"targeted\" }\n\
             \n\
             [target.'cfg(unix)'.dev-dependencies]\n\
             targeted_dev = { path = \"targeted_dev\" }\n\
             \n\
             [workspace.dependencies]\n\
             shared = { path = \"shared\" }\n\
             \n\
             [patch.crates-io]\n\
             patched = { path = \"patched\" }\n\
             \n\
             [replace]\n\
             \"replaced:0.1.0\" = { path = \"replaced\" }\n";

    /// Verifies that every place Cargo reads a dependency's `path` from in
    /// the workspace's root manifest is read here, by writing one dependency
    /// in each and listing them all.
    #[test]
    fn every_place_cargo_reads_in_the_root_manifest_is_read() -> TestOutcome {
        let directories = led_to(
            "path-dependencies-every-place",
            "Cargo.toml",
            ManifestRole::WorkspaceRoot,
            EVERY_PLACE,
        )?;

        assert_eq!(
            directories,
            [
                "built",
                "developed",
                "dotted",
                "normal",
                "patched",
                "replaced",
                "shared",
                "targeted",
                "targeted_dev",
                "underscored",
            ]
        );
        Ok(())
    }

    /// The same manifest anywhere but the workspace's root: Cargo ignores its
    /// `[workspace.dependencies]`, `[patch]` and `[replace]`, so nothing they
    /// name is reached through it.
    #[test]
    fn a_manifest_other_than_the_root_leads_nowhere_through_the_root_only_places() -> TestOutcome {
        let directories = led_to(
            "path-dependencies-not-the-root",
            "Cargo.toml",
            ManifestRole::Other,
            EVERY_PLACE,
        )?;

        assert_eq!(
            directories,
            [
                "built",
                "developed",
                "dotted",
                "normal",
                "targeted",
                "targeted_dev",
                "underscored",
            ]
        );
        Ok(())
    }

    #[test]
    fn an_optional_dependency_is_read_like_any_other() -> TestOutcome {
        let directories = led_to(
            "path-dependencies-optional",
            "Cargo.toml",
            ManifestRole::Other,
            "[dependencies]\nx = { path = \"vendor/x\", optional = true }\n",
        )?;

        assert_eq!(directories, ["vendor/x"]);
        Ok(())
    }

    /// A path is read from the manifest's own directory, and two spellings of
    /// one directory are one.
    #[test]
    fn a_path_is_read_from_the_manifests_directory_and_normalised_once() -> TestOutcome {
        let directories = led_to(
            "path-dependencies-normalised",
            "crates/cli/Cargo.toml",
            ManifestRole::Other,
            "[dependencies]\n\
             a = { path = \"../../tasks/greet\" }\n\
             b = { path = \"../cli/../../tasks/./greet\" }\n\
             c = { path = \"../sibling\" }\n",
        )?;

        assert_eq!(directories, ["crates/sibling", "tasks/greet"]);
        Ok(())
    }

    /// Dependencies that lead nowhere on disk are not path dependencies: a
    /// version, a git source, and a `path` that is not a string.
    #[test]
    fn a_dependency_with_no_string_path_leads_nowhere() -> TestOutcome {
        let directories = led_to(
            "path-dependencies-none",
            "Cargo.toml",
            ManifestRole::Other,
            "[dependencies]\n\
             serde = \"1\"\n\
             versioned = { version = \"1\" }\n\
             git_sourced = { git = \"https://example.invalid/x\" }\n\
             malformed = { path = 3 }\n\
             workspace_sourced.workspace = true\n\
             \n\
             [package]\n\
             name = \"x\"\n\
             build = \"build.rs\"\n",
        )?;

        assert_eq!(directories, Vec::<String>::new());
        Ok(())
    }

    #[test]
    fn a_manifest_with_no_dependencies_leads_nowhere() -> TestOutcome {
        let directories = led_to(
            "path-dependencies-empty",
            "Cargo.toml",
            ManifestRole::Other,
            "[package]\nname = \"x\"\n",
        )?;

        assert_eq!(directories, Vec::<String>::new());
        Ok(())
    }
}
