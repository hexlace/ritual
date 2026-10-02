//! Taking things out of a manifest: a task's key from the list, its
//! dependency line, the workspace's declaration of it, and its member
//! entry — each as an edit in place that leaves the rest of the file
//! as a human wrote it.
//!
//! The counterpart of what `add` appends. Each edit that takes an entry out
//! of an array removes the entry's own line, including a comment written on
//! it, and every line the entry did not own stays.

use std::path::{Path, PathBuf};

use toml_edit::{Array, Item, TableLike, Value};

use rituals::Failure;

use super::Manifest;
use super::entry_removal::remove_matching_style;
use crate::paths::{lies_under, normalize};

/// The characters that make a `members` entry a glob rather than a path.
const GLOB_CHARACTERS: [char; 3] = ['*', '?', '['];

impl Manifest {
    /// Takes `key` out of `[package.metadata.ritual] tasks`, every time it
    /// appears, leaving the lines around it as they were.
    ///
    /// The entry's own line goes with it, including a comment written on that
    /// line. A comment on a line of its own above the entry stays.
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming [`Manifest::path`] when there is no
    /// `[package.metadata.ritual] tasks` list, or when it does not hold
    /// `key`. The document is unchanged when it refuses.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::manifest::Manifest;
    /// use rituals_compose::rollback;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-manifest-unlist-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(
    /// #     &manifest_path,
    /// #     "[package.metadata.ritual]\ntasks = [\n    \"ritual\",\n    \"lint\", # style\n]\n",
    /// # )?;
    /// let mut manifest = Manifest::read(&manifest_path)?;
    ///
    /// manifest.unlist_task("lint")?;
    /// rollback::attempt("running `remove lint` again", |changes| manifest.write(changes))?;
    ///
    /// let on_disk = std::fs::read_to_string(&manifest_path)?;
    /// assert_eq!(on_disk, "[package.metadata.ritual]\ntasks = [\n    \"ritual\",\n]\n");
    /// assert!(manifest.unlist_task("lint").is_err());
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn unlist_task(&mut self, key: &str) -> Result<(), Failure> {
        let path = self.path.display().to_string();
        let tasks = self
            .document
            .get_mut("package")
            .and_then(Item::as_table_like_mut)
            .and_then(|package| package.get_mut("metadata"))
            .and_then(Item::as_table_like_mut)
            .and_then(|metadata| metadata.get_mut("ritual"))
            .and_then(Item::as_table_like_mut)
            .and_then(|ritual| ritual.get_mut("tasks"))
            .and_then(Item::as_array_mut)
            .ok_or_else(|| {
                Failure::new(format!(
                    "{path} has no [package.metadata.ritual] tasks list to take `{key}` out of"
                ))
            })?;

        let positions = positions_of(tasks, |entry| entry == key);
        if positions.is_empty() {
            return Err(Failure::new(format!(
                "`{key}` is not in the [package.metadata.ritual] tasks list of {path}"
            )));
        }
        remove_all(tasks, &positions);
        Ok(())
    }

    /// Removes the dependency declared under `key` from `[dependencies]` and
    /// from every `[target.<…>.dependencies]` table, whatever shape it is
    /// written in: an inline table, a dotted `key.workspace = true`, a
    /// `[dependencies.key]` table of its own, or a bare version string.
    ///
    /// The dependency's own line goes, with a comment written on it and one
    /// written directly above it, which `toml_edit` holds as part of the
    /// line; the comments around its neighbours stay. Reports whether it
    /// removed anything. `[dev-dependencies]` and
    /// `[build-dependencies]` are not touched: a dependency under the same
    /// key there is a different declaration that nothing here owns.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::manifest::Manifest;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-manifest-dependency-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(
    /// #     &manifest_path,
    /// #     "[dependencies]\nlint = { path = \"tasks/lint\" }\nritual.workspace = true\n",
    /// # )?;
    /// let mut manifest = Manifest::read(&manifest_path)?;
    ///
    /// assert!(manifest.remove_dependency("lint"));
    /// assert!(!manifest.remove_dependency("lint"));
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn remove_dependency(&mut self, key: &str) -> bool {
        let root = self.document.as_table_mut();
        let mut removed = root
            .get_mut("dependencies")
            .and_then(Item::as_table_like_mut)
            .is_some_and(|dependencies| dependencies.remove(key).is_some());

        let Some(targets) = root.get_mut("target").and_then(Item::as_table_like_mut) else {
            return removed;
        };
        for (_target, declarations) in targets.iter_mut() {
            let Some(dependencies) = declarations
                .as_table_like_mut()
                .and_then(|declarations| declarations.get_mut("dependencies"))
                .and_then(Item::as_table_like_mut)
            else {
                continue;
            };
            if dependencies.remove(key).is_some() {
                removed = true;
            }
        }
        removed
    }

    /// Reports whether the dependency declared under `key` takes its
    /// definition from the workspace, as `key.workspace = true`, in
    /// `[dependencies]` or in any `[target.<…>.dependencies]` table.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::manifest::Manifest;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-manifest-inherits-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(
    /// #     &manifest_path,
    /// #     "[dependencies]\nlint.workspace = true\nformat = { path = \"tasks/format\" }\n",
    /// # )?;
    /// let manifest = Manifest::read(&manifest_path)?;
    ///
    /// assert!(manifest.inherits_workspace_dependency("lint"));
    /// assert!(!manifest.inherits_workspace_dependency("format"));
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn inherits_workspace_dependency(&self, key: &str) -> bool {
        let root = self.document.as_table();
        let in_dependencies = root
            .get("dependencies")
            .and_then(Item::as_table_like)
            .is_some_and(|dependencies| inherits(dependencies, key));
        let in_a_target = root
            .get("target")
            .and_then(Item::as_table_like)
            .is_some_and(|targets| {
                targets.iter().any(|(_target, declarations)| {
                    declarations
                        .as_table_like()
                        .and_then(|declarations| declarations.get("dependencies"))
                        .and_then(Item::as_table_like)
                        .is_some_and(|dependencies| inherits(dependencies, key))
                })
            });
        in_dependencies || in_a_target
    }

    /// Removes `key` from `[workspace.dependencies]`, and reports whether
    /// there was an entry to remove.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::manifest::Manifest;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!(
    /// #         "rituals-compose-doctest-manifest-workspace-dependency-{}",
    /// #         std::process::id()
    /// #     ));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(
    /// #     &manifest_path,
    /// #     "[workspace.dependencies]\nlint = { path = \"tasks/lint\" }\n",
    /// # )?;
    /// let mut manifest = Manifest::read(&manifest_path)?;
    ///
    /// assert!(manifest.remove_workspace_dependency("lint"));
    /// assert!(!manifest.declares_workspace_dependency("lint"));
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn remove_workspace_dependency(&mut self, key: &str) -> bool {
        self.document
            .get_mut("workspace")
            .and_then(Item::as_table_like_mut)
            .and_then(|workspace| workspace.get_mut("dependencies"))
            .and_then(Item::as_table_like_mut)
            .is_some_and(|dependencies| dependencies.remove(key).is_some())
    }

    /// Removes every entry of `[workspace] members` and of `default-members`
    /// that names `directory`, and reports whether it removed any.
    ///
    /// An entry names it when Cargo would read it as that directory: joined
    /// to this manifest's own directory, the workspace root, with `.` and
    /// `..` components and repeated separators removed as text, the way
    /// Cargo reads them. So `tasks/lint`, `./tasks/lint/`, `tasks//lint`,
    /// `x/../tasks/lint` and the absolute path all name `tasks/lint`. A
    /// relative `directory` is taken from the workspace root too.
    ///
    /// A glob such as `tasks/*` is never touched, because it names more than
    /// this directory; [`Manifest::globs_left_matching_nothing`] says when
    /// deleting the directory would leave one matching nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    ///
    /// use rituals_compose::manifest::Manifest;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-manifest-member-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(
    /// #     &manifest_path,
    /// #     "[workspace]\nmembers = [\"crates/cli\", \"./tasks/x/../lint\", \"tasks/*\"]\n",
    /// # )?;
    /// let mut manifest = Manifest::read(&manifest_path)?;
    ///
    /// assert!(manifest.remove_workspace_member(Path::new("tasks/lint")));
    /// // The glob stays: it is not an entry for this directory.
    /// assert!(!manifest.remove_workspace_member(Path::new("tasks/*")));
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn remove_workspace_member(&mut self, directory: &Path) -> bool {
        let target = self.joined_to_the_workspace_root(directory);
        let root = self.workspace_root();
        let Some(workspace) = self
            .document
            .get_mut("workspace")
            .and_then(Item::as_table_like_mut)
        else {
            return false;
        };

        let mut removed = false;
        for list in MEMBER_LISTS {
            let Some(entries) = workspace.get_mut(list).and_then(Item::as_array_mut) else {
                continue;
            };
            let positions = positions_of(entries, |entry| names(&root, entry, &target));
            remove_all(entries, &positions);
            if !positions.is_empty() {
                removed = true;
            }
        }
        removed
    }

    /// Reports whether removing `directory`'s entries would leave
    /// `[workspace] default-members` empty, when it is not empty now.
    ///
    /// Entries name `directory` the way
    /// [`Manifest::remove_workspace_member`] reads them.
    ///
    /// Cargo refuses a workspace whose `default-members` names nothing, so
    /// a caller checks this before it writes anything, rather than learning
    /// it from the build afterwards.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    ///
    /// use rituals_compose::manifest::Manifest;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!(
    /// #         "rituals-compose-doctest-manifest-default-members-{}",
    /// #         std::process::id()
    /// #     ));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(
    /// #     &manifest_path,
    /// #     "[workspace]\nmembers = [\"tasks/lint\"]\ndefault-members = [\"tasks/lint\"]\n",
    /// # )?;
    /// let manifest = Manifest::read(&manifest_path)?;
    ///
    /// assert!(manifest.empties_default_members(Path::new("tasks/lint")));
    /// assert!(!manifest.empties_default_members(Path::new("tasks/format")));
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn empties_default_members(&self, directory: &Path) -> bool {
        let target = self.joined_to_the_workspace_root(directory);
        let root = self.workspace_root();
        let Some(entries) = self
            .document
            .get("workspace")
            .and_then(Item::as_table_like)
            .and_then(|workspace| workspace.get("default-members"))
            .and_then(Item::as_array)
        else {
            return false;
        };

        let positions = positions_of(entries, |entry| names(&root, entry, &target));
        !entries.is_empty() && positions.len() == entries.len()
    }

    /// Returns every glob in `[workspace] members` and `default-members` that
    /// matches something now and would match nothing once `directory` is
    /// deleted, as written.
    ///
    /// Cargo expands a glob against the filesystem, and reads one that
    /// matches nothing as a literal path, so deleting the last directory a
    /// glob matched leaves a workspace Cargo cannot load. Expanded here the
    /// way Cargo expands it, joined to the workspace root with the `glob`
    /// crate Cargo uses, and every match under `directory` set aside. A
    /// relative `directory` is taken from the workspace root.
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming the glob when it is not a valid pattern
    /// or a path it reaches cannot be read, the cases in which Cargo fails to
    /// expand it too.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    ///
    /// use rituals_compose::manifest::Manifest;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-manifest-globs-{}", std::process::id()));
    /// # std::fs::create_dir_all(directory.join("tasks/lint"))?;
    /// # std::fs::create_dir_all(directory.join("crates/cli"))?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(&manifest_path, "[workspace]\nmembers = [\"crates/*\", \"tasks/*\"]\n")?;
    /// let manifest = Manifest::read(&manifest_path)?;
    ///
    /// // `tasks/lint` is the only match `tasks/*` has.
    /// let left_empty = manifest.globs_left_matching_nothing(Path::new("tasks/lint"))?;
    /// assert_eq!(left_empty, ["tasks/*"]);
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn globs_left_matching_nothing(&self, directory: &Path) -> Result<Vec<String>, Failure> {
        let target = self.joined_to_the_workspace_root(directory);
        let root = self.workspace_root();
        let Some(workspace) = self.document.get("workspace").and_then(Item::as_table_like) else {
            return Ok(Vec::new());
        };

        let mut left_empty = Vec::new();
        for list in MEMBER_LISTS {
            let Some(entries) = workspace.get(list).and_then(Item::as_array) else {
                continue;
            };
            for entry in entries.iter().filter_map(Value::as_str) {
                if !entry.contains(GLOB_CHARACTERS) || left_empty.iter().any(|seen| seen == entry) {
                    continue;
                }
                let matches = expand(&root, entry)?;
                let survivors = matches.iter().filter(|path| !lies_under(path, &target));
                if !matches.is_empty() && survivors.count() == 0 {
                    left_empty.push(entry.to_string());
                }
            }
        }
        Ok(left_empty)
    }

    /// Returns every `[patch.<source>]`, `[replace]` and
    /// `[workspace.dependencies]` entry whose `path` is `directory` or lies
    /// under it, named as a person would find it, such as
    /// `[patch.crates-io] lint`.
    ///
    /// Cargo reads each of them whether or not anything uses it, so deleting
    /// the directory leaves a workspace that refers to nothing. The
    /// `[workspace.dependencies]` entry under `dropped` is left out, because
    /// the caller is taking it out. Paths are joined to this manifest's
    /// directory, the workspace root, as Cargo joins them; so is a relative
    /// `directory`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::path::Path;
    ///
    /// use rituals_compose::manifest::Manifest;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-manifest-references-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(
    /// #     &manifest_path,
    /// #     "[workspace.dependencies]\nlint = { path = \"tasks/lint\" }\n\n\
    /// #      [patch.crates-io]\nhelper = { path = \"tasks/lint/helper\" }\n",
    /// # )?;
    /// let manifest = Manifest::read(&manifest_path)?;
    ///
    /// let entries = manifest.entries_pointing_under(Path::new("tasks/lint"), Some("lint"));
    /// assert_eq!(entries, ["[patch.crates-io] helper"]);
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn entries_pointing_under(&self, directory: &Path, dropped: Option<&str>) -> Vec<String> {
        let target = self.joined_to_the_workspace_root(directory);
        let root = self.workspace_root();
        let points_under = |declaration: &Item| {
            declaration
                .as_table_like()
                .and_then(|declaration| declaration.get("path"))
                .and_then(Item::as_str)
                .is_some_and(|path| lies_under(&root.join(path), &target))
        };

        let mut entries = Vec::new();
        let workspace_dependencies = self
            .document
            .get("workspace")
            .and_then(Item::as_table_like)
            .and_then(|workspace| workspace.get("dependencies"));
        for (name, declaration) in entries_of(workspace_dependencies) {
            if Some(name) != dropped && points_under(declaration) {
                entries.push(format!("[workspace.dependencies] {name}"));
            }
        }
        for (source, patches) in entries_of(self.document.get("patch")) {
            for (name, declaration) in entries_of(Some(patches)) {
                if points_under(declaration) {
                    entries.push(format!("[patch.{source}] {name}"));
                }
            }
        }
        for (specification, declaration) in entries_of(self.document.get("replace")) {
            if points_under(declaration) {
                entries.push(format!("[replace] \"{specification}\""));
            }
        }
        entries
    }

    /// The directory this manifest is in, which for a workspace's manifest
    /// is the root every path in it is read from.
    fn workspace_root(&self) -> PathBuf {
        self.path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }

    /// `directory` joined to [`Manifest::workspace_root`], which leaves an
    /// absolute one as it is.
    fn joined_to_the_workspace_root(&self, directory: &Path) -> PathBuf {
        self.workspace_root().join(directory)
    }
}

/// The two `[workspace]` lists whose entries name member directories.
const MEMBER_LISTS: [&str; 2] = ["members", "default-members"];

/// Whether `dependencies` declares `key` as taking its definition from the
/// workspace: `key.workspace = true`, `key = { workspace = true }` and a
/// `[dependencies.key]` table with `workspace = true` all read the same.
fn inherits(dependencies: &dyn TableLike, key: &str) -> bool {
    dependencies
        .get(key)
        .and_then(Item::as_table_like)
        .and_then(|declaration| declaration.get("workspace"))
        .and_then(Item::as_value)
        .and_then(Value::as_bool)
        == Some(true)
}

/// Whether the `members` entry `entry`, read from `root`, names `directory`:
/// not a glob, and the same path once both are normalised the way Cargo
/// normalises a member entry.
fn names(root: &Path, entry: &str, directory: &Path) -> bool {
    !entry.contains(GLOB_CHARACTERS) && normalize(&root.join(entry)) == normalize(directory)
}

/// Every key and value of `item`, when it is a table of any kind, and
/// nothing otherwise.
fn entries_of(item: Option<&Item>) -> impl Iterator<Item = (&str, &Item)> {
    item.and_then(Item::as_table_like)
        .into_iter()
        .flat_map(TableLike::iter)
}

/// Every path the glob `entry`, joined to `root`, matches on disk, the way
/// Cargo expands a `members` glob.
fn expand(root: &Path, entry: &str) -> Result<Vec<PathBuf>, Failure> {
    let pattern = root.join(entry);
    let unreadable = |error: &dyn std::fmt::Display| {
        Failure::new(format!(
            "the [workspace] members glob `{entry}` could not be expanded: {error}"
        ))
    };
    let pattern = pattern
        .to_str()
        .ok_or_else(|| unreadable(&"its path is not UTF-8"))?;
    glob::glob(pattern)
        .map_err(|error| unreadable(&error))?
        .map(|matched| matched.map_err(|error| unreadable(&error)))
        .collect()
}

/// The positions in `array`, in order, of the string entries `matches`
/// accepts. An entry that is not a string never matches.
fn positions_of(array: &Array, matches: impl Fn(&str) -> bool) -> Vec<usize> {
    array
        .iter()
        .enumerate()
        .filter(|(_position, value)| value.as_str().is_some_and(&matches))
        .map(|(position, _value)| position)
        .collect()
}

/// Removes the entries at `positions`, given in increasing order, from the
/// last to the first, so that each removal leaves the positions still to
/// come where they were.
fn remove_all(array: &mut Array, positions: &[usize]) {
    for position in positions.iter().rev() {
        remove_matching_style(array, *position);
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::Path;

    use super::Manifest;
    use crate::test_support::{ScratchDir, TestOutcome};

    /// Writes `content` to a scratch manifest and reads it back.
    fn read_manifest(tag: &str, content: &str) -> Result<(ScratchDir, Manifest), Box<dyn Error>> {
        let scratch = ScratchDir::new(tag)?;
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(&path, content)?;
        let manifest = Manifest::read(&path)?;
        Ok((scratch, manifest))
    }

    // `unlist_task`.

    #[test]
    fn unlisting_a_task_takes_its_line_and_leaves_the_rest_of_the_file() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "unlist",
            "[package]\nname = \"cli\"\n\n# the tasks\n[package.metadata.ritual]\n\
             tasks = [\n    \"ritual\",\n    \"lint\", # style\n    \"format\",\n]\n",
        )?;

        manifest.unlist_task("lint")?;

        assert_eq!(
            manifest.document.to_string(),
            "[package]\nname = \"cli\"\n\n# the tasks\n[package.metadata.ritual]\n\
             tasks = [\n    \"ritual\",\n    \"format\",\n]\n"
        );
        Ok(())
    }

    #[test]
    fn unlisting_a_task_listed_twice_takes_both() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "unlist-twice",
            "[package.metadata.ritual]\ntasks = [\"lint\", \"ritual\", \"lint\"]\n",
        )?;

        manifest.unlist_task("lint")?;

        assert_eq!(
            manifest.document.to_string(),
            "[package.metadata.ritual]\ntasks = [\"ritual\"]\n"
        );
        Ok(())
    }

    #[test]
    fn unlisting_a_task_that_is_not_listed_is_refused_and_changes_nothing() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "unlist-absent-key",
            "[package.metadata.ritual]\ntasks = [\"ritual\"]\n",
        )?;
        let before = manifest.document.to_string();

        let failure = manifest
            .unlist_task("lint")
            .err()
            .ok_or("an unlisted key was meant to be refused")?;

        assert!(
            failure.to_string().contains("`lint` is not in"),
            "{failure}"
        );
        assert!(failure.to_string().contains("Cargo.toml"), "{failure}");
        assert_eq!(manifest.document.to_string(), before);
        Ok(())
    }

    #[test]
    fn unlisting_from_a_manifest_with_no_list_is_refused_and_changes_nothing() -> TestOutcome {
        for (tag, content) in [
            ("unlist-no-ritual", "[package]\nname = \"cli\"\n"),
            (
                "unlist-no-tasks",
                "[package.metadata.ritual]\ntask = true\n",
            ),
            (
                "unlist-not-a-list",
                "[package.metadata.ritual]\ntasks = \"lint\"\n",
            ),
        ] {
            let (_scratch, mut manifest) = read_manifest(tag, content)?;
            let before = manifest.document.to_string();

            let failure = manifest
                .unlist_task("lint")
                .err()
                .ok_or("a manifest with no list was meant to be refused")?;

            assert!(
                failure
                    .to_string()
                    .contains("has no [package.metadata.ritual] tasks list"),
                "{tag}: {failure}"
            );
            assert_eq!(manifest.document.to_string(), before, "{tag}");
        }
        Ok(())
    }

    // `remove_dependency` and `inherits_workspace_dependency`.

    #[test]
    fn a_dependency_is_removed_in_every_shape_it_can_be_written() -> TestOutcome {
        for (shape, dependency) in [
            ("inline table", "lint = { path = \"tasks/lint\" }\n"),
            ("dotted key", "lint.workspace = true\n"),
            ("version string", "lint = \"1.0\"\n"),
            (
                "table of its own",
                "\n[dependencies.lint]\npath = \"tasks/lint\"\n",
            ),
        ] {
            let (_scratch, mut manifest) = read_manifest(
                "remove-dependency-shapes",
                &format!("[dependencies]\nritual.workspace = true\n{dependency}"),
            )?;

            assert!(manifest.remove_dependency("lint"), "{shape}");

            assert_eq!(
                manifest.document.to_string(),
                "[dependencies]\nritual.workspace = true\n",
                "{shape}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_dependency_after_the_one_removed_keeps_its_line_and_comment() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "remove-dependency-neighbours",
            "[dependencies]\nlint = { path = \"tasks/lint\" }\n\
             # the formatter\nformat = { path = \"tasks/format\" } # local\n",
        )?;

        assert!(manifest.remove_dependency("lint"));

        assert_eq!(
            manifest.document.to_string(),
            "[dependencies]\n# the formatter\nformat = { path = \"tasks/format\" } # local\n"
        );
        Ok(())
    }

    #[test]
    fn a_comment_on_the_line_above_a_removed_dependency_goes_with_it() -> TestOutcome {
        // `toml_edit` keeps a comment written directly above a key as part of
        // that key, so it is removed with it. The comment above the entry
        // after it is that entry's, and stays.
        let (_scratch, mut manifest) = read_manifest(
            "remove-dependency-comment-above",
            "[dependencies]\n# the linter\nlint = \"1\" # pinned\n# the formatter\n\
             format = \"1\"\n",
        )?;

        assert!(manifest.remove_dependency("lint"));

        assert_eq!(
            manifest.document.to_string(),
            "[dependencies]\n# the formatter\nformat = \"1\"\n"
        );
        Ok(())
    }

    #[test]
    fn a_dependency_is_removed_from_every_target_table_too() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "remove-dependency-targets",
            "[dependencies]\nlint = { path = \"tasks/lint\" }\n\n\
             [target.'cfg(unix)'.dependencies]\nlint = { path = \"tasks/lint\" }\nother = \"1\"\n\n\
             [target.'cfg(debug_assertions)'.dependencies]\nlint = \"1\"\n",
        )?;

        assert!(manifest.remove_dependency("lint"));

        assert_eq!(
            manifest.document.to_string(),
            "[dependencies]\n\n\
             [target.'cfg(unix)'.dependencies]\nother = \"1\"\n\n\
             [target.'cfg(debug_assertions)'.dependencies]\n"
        );
        Ok(())
    }

    #[test]
    fn a_dependency_only_under_a_target_is_removed() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "remove-dependency-only-target",
            "[dependencies]\n\n[target.'cfg(unix)'.dependencies]\nlint = \"1\"\n",
        )?;

        assert!(manifest.remove_dependency("lint"));
        Ok(())
    }

    #[test]
    fn a_dev_dependency_under_the_same_key_is_not_touched() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "remove-dependency-dev",
            "[dependencies]\nlint = \"1\"\n\n[dev-dependencies]\nlint = \"1\"\n",
        )?;

        assert!(manifest.remove_dependency("lint"));

        assert_eq!(
            manifest.document.to_string(),
            "[dependencies]\n\n[dev-dependencies]\nlint = \"1\"\n"
        );
        Ok(())
    }

    #[test]
    fn removing_a_dependency_that_is_not_there_reports_false_and_changes_nothing() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "remove-dependency-absent",
            "[dependencies]\nother = \"1\"\n\n[target.'cfg(unix)'.dependencies]\nmore = \"1\"\n",
        )?;
        let before = manifest.document.to_string();

        assert!(!manifest.remove_dependency("lint"));
        assert_eq!(manifest.document.to_string(), before);

        let (_scratch, mut bare) = manifest_without_dependencies()?;
        assert!(!bare.remove_dependency("lint"));
        Ok(())
    }

    fn manifest_without_dependencies() -> Result<(ScratchDir, Manifest), Box<dyn Error>> {
        read_manifest("remove-dependency-none", "[package]\nname = \"cli\"\n")
    }

    #[test]
    fn a_dependency_reads_as_inheriting_in_every_shape_that_says_so() -> TestOutcome {
        for (shape, dependencies) in [
            ("dotted key", "[dependencies]\nlint.workspace = true\n"),
            (
                "inline table",
                "[dependencies]\nlint = { workspace = true }\n",
            ),
            (
                "table of its own",
                "[dependencies.lint]\nworkspace = true\n",
            ),
            (
                "inline table with more",
                "[dependencies]\nlint = { workspace = true, optional = true }\n",
            ),
            (
                "a target table",
                "[dependencies]\n\n[target.'cfg(unix)'.dependencies]\nlint.workspace = true\n",
            ),
        ] {
            let (_scratch, manifest) = read_manifest("inherits-yes", dependencies)?;
            assert!(manifest.inherits_workspace_dependency("lint"), "{shape}");
        }
        Ok(())
    }

    #[test]
    fn a_dependency_that_does_not_say_workspace_true_does_not_inherit() -> TestOutcome {
        for (shape, dependencies) in [
            (
                "a path",
                "[dependencies]\nlint = { path = \"tasks/lint\" }\n",
            ),
            ("a version", "[dependencies]\nlint = \"1\"\n"),
            (
                "workspace false",
                "[dependencies]\nlint.workspace = false\n",
            ),
            ("another key", "[dependencies]\nother.workspace = true\n"),
            ("no dependencies", "[package]\nname = \"cli\"\n"),
            (
                "a dev-dependency",
                "[dev-dependencies]\nlint.workspace = true\n",
            ),
        ] {
            let (_scratch, manifest) = read_manifest("inherits-no", dependencies)?;
            assert!(!manifest.inherits_workspace_dependency("lint"), "{shape}");
        }
        Ok(())
    }

    // `remove_workspace_dependency`.

    #[test]
    fn a_workspace_dependency_is_removed_and_the_others_stay() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "remove-workspace-dependency",
            "[workspace.dependencies]\nlint = { path = \"tasks/lint\", version = \"0.1.1\" }\n\
             format = { path = \"tasks/format\" }\n",
        )?;

        assert!(manifest.remove_workspace_dependency("lint"));

        assert_eq!(
            manifest.document.to_string(),
            "[workspace.dependencies]\nformat = { path = \"tasks/format\" }\n"
        );
        assert!(!manifest.remove_workspace_dependency("lint"));
        Ok(())
    }

    #[test]
    fn a_workspace_dependency_that_is_not_declared_reports_false() -> TestOutcome {
        let (_scratch, mut without_table) = read_manifest(
            "remove-workspace-dependency-none",
            "[workspace]\nmembers = []\n",
        )?;
        let before = without_table.document.to_string();
        assert!(!without_table.remove_workspace_dependency("lint"));
        assert_eq!(without_table.document.to_string(), before);

        let (_scratch, mut not_a_workspace) = read_manifest(
            "remove-workspace-dependency-package",
            "[package]\nname = \"cli\"\n",
        )?;
        assert!(!not_a_workspace.remove_workspace_dependency("lint"));
        Ok(())
    }

    // `remove_workspace_member` and `empties_default_members`.

    const A_WORKSPACE: &str = "[workspace]\n# the crates\nmembers = [\n    \"crates/cli\",\n    \
                               \"tasks/lint\", # style\n    \"tasks/format\",\n]\n\
                               resolver = \"3\"\n";

    #[test]
    fn a_member_is_removed_with_its_line_and_the_rest_of_the_file_stays() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest("remove-member", A_WORKSPACE)?;

        assert!(manifest.remove_workspace_member(Path::new("tasks/lint")));

        assert_eq!(
            manifest.document.to_string(),
            "[workspace]\n# the crates\nmembers = [\n    \"crates/cli\",\n    \
             \"tasks/format\",\n]\nresolver = \"3\"\n"
        );
        Ok(())
    }

    #[test]
    fn a_member_is_found_however_it_is_spelled() -> TestOutcome {
        for spelling in ["tasks/lint", "./tasks/lint", "tasks/lint/", "./tasks/lint/"] {
            let (_scratch, mut manifest) = read_manifest(
                "remove-member-spelling",
                &format!("[workspace]\nmembers = [\"crates/cli\", \"{spelling}\"]\n"),
            )?;

            assert!(
                manifest.remove_workspace_member(Path::new("tasks/lint")),
                "{spelling}"
            );
            assert!(
                !manifest.remove_workspace_member(Path::new("tasks/lint")),
                "{spelling}: already removed"
            );

            assert_eq!(
                manifest.document.to_string(),
                "[workspace]\nmembers = [\"crates/cli\"]\n",
                "{spelling}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_directory_is_matched_however_it_is_spelled() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "remove-member-directory-spelling",
            "[workspace]\nmembers = [\"crates/cli\", \"tasks/lint\"]\n",
        )?;

        assert!(manifest.remove_workspace_member(Path::new("./tasks/lint/")));
        Ok(())
    }

    #[test]
    fn a_glob_is_never_removed() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "remove-member-glob",
            "[workspace]\nmembers = [\"crates/cli\", \"tasks/*\", \"tools/[a-z]*\", \"x/?\"]\n",
        )?;
        let before = manifest.document.to_string();

        for directory in ["tasks/lint", "tasks/*", "tools/[a-z]*", "x/?", "x/a"] {
            assert!(
                !manifest.remove_workspace_member(Path::new(directory)),
                "{directory}"
            );
        }
        assert_eq!(manifest.document.to_string(), before);
        Ok(())
    }

    #[test]
    fn a_member_whose_name_only_starts_the_same_is_not_removed() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "remove-member-prefix",
            "[workspace]\nmembers = [\"tasks/lint-extra\", \"tasks/lin\", \"other/tasks/lint\"]\n",
        )?;
        let before = manifest.document.to_string();

        assert!(!manifest.remove_workspace_member(Path::new("tasks/lint")));
        assert_eq!(manifest.document.to_string(), before);
        Ok(())
    }

    /// Cargo reads a member entry as a path joined to the workspace root,
    /// so every spelling it reads as the directory is taken out: repeated
    /// separators, `.` and `..` components, and the absolute path.
    #[test]
    fn a_member_is_found_however_cargo_would_read_it() -> TestOutcome {
        for spelling in ["tasks/./lint", "tasks//lint", "x/../tasks/lint", "ABSOLUTE"] {
            let scratch = ScratchDir::new("remove-member-as-a-path")?;
            let absolute = scratch.path().join("tasks/lint");
            let spelling = spelling.replace("ABSOLUTE", &absolute.display().to_string());
            let path = scratch.path().join("Cargo.toml");
            std::fs::write(
                &path,
                format!("[workspace]\nmembers = [\"crates/cli\", \"{spelling}\"]\n"),
            )?;
            let mut manifest = Manifest::read(&path)?;

            assert!(
                manifest.remove_workspace_member(Path::new("tasks/lint")),
                "{spelling}"
            );
            assert_eq!(
                manifest.document.to_string(),
                "[workspace]\nmembers = [\"crates/cli\"]\n",
                "{spelling}"
            );
        }
        Ok(())
    }

    /// An empty entry and `./` name the workspace root itself, as they do to
    /// Cargo, and so does an empty directory; neither names a member below
    /// it.
    #[test]
    fn the_root_is_named_only_by_entries_that_name_the_root() -> TestOutcome {
        let (_scratch, manifest) = read_manifest(
            "remove-member-root",
            "[workspace]\nmembers = [\"\", \"crates/cli\"]\ndefault-members = [\"./\"]\n",
        )?;

        assert!(manifest.empties_default_members(Path::new("")));
        assert!(!manifest.empties_default_members(Path::new("crates")));
        let mut manifest = manifest;
        assert!(!manifest.remove_workspace_member(Path::new("crates")));
        assert!(manifest.remove_workspace_member(Path::new("")));
        assert_eq!(
            manifest.document.to_string(),
            "[workspace]\nmembers = [\"crates/cli\"]\ndefault-members = []\n"
        );
        Ok(())
    }

    #[test]
    fn default_members_loses_the_entry_too() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "remove-member-default",
            "[workspace]\nmembers = [\"crates/cli\", \"tasks/lint\"]\n\
             default-members = [\"crates/cli\", \"./tasks/lint\"]\n",
        )?;

        assert!(manifest.remove_workspace_member(Path::new("tasks/lint")));

        assert_eq!(
            manifest.document.to_string(),
            "[workspace]\nmembers = [\"crates/cli\"]\ndefault-members = [\"crates/cli\"]\n"
        );
        Ok(())
    }

    #[test]
    fn an_entry_only_in_default_members_still_counts_as_removed() -> TestOutcome {
        let (_scratch, mut manifest) = read_manifest(
            "remove-member-default-only",
            "[workspace]\nmembers = [\"tasks/*\"]\n\
             default-members = [\"crates/cli\", \"tasks/lint\"]\n",
        )?;

        assert!(manifest.remove_workspace_member(Path::new("tasks/lint")));

        assert_eq!(
            manifest.document.to_string(),
            "[workspace]\nmembers = [\"tasks/*\"]\ndefault-members = [\"crates/cli\"]\n"
        );
        Ok(())
    }

    #[test]
    fn removing_a_member_from_a_manifest_with_no_workspace_reports_false() -> TestOutcome {
        let (_scratch, mut manifest) =
            read_manifest("remove-member-no-workspace", "[package]\nname = \"cli\"\n")?;
        let before = manifest.document.to_string();

        assert!(!manifest.remove_workspace_member(Path::new("tasks/lint")));
        assert_eq!(manifest.document.to_string(), before);

        let (_scratch, mut no_list) =
            read_manifest("remove-member-no-list", "[workspace]\nresolver = \"3\"\n")?;
        assert!(!no_list.remove_workspace_member(Path::new("tasks/lint")));
        Ok(())
    }

    #[test]
    fn default_members_is_emptied_only_when_every_entry_would_go() -> TestOutcome {
        let (_scratch, only) = read_manifest(
            "empties-only",
            "[workspace]\ndefault-members = [\"tasks/lint\"]\n",
        )?;
        assert!(only.empties_default_members(Path::new("tasks/lint")));
        assert!(only.empties_default_members(Path::new("./tasks/lint/")));

        let (_scratch, twice) = read_manifest(
            "empties-twice",
            "[workspace]\ndefault-members = [\"tasks/lint\", \"./tasks/lint\"]\n",
        )?;
        assert!(twice.empties_default_members(Path::new("tasks/lint")));

        let (_scratch, others) = read_manifest(
            "empties-others",
            "[workspace]\ndefault-members = [\"crates/cli\", \"tasks/lint\"]\n",
        )?;
        assert!(!others.empties_default_members(Path::new("tasks/lint")));

        let (_scratch, elsewhere) = read_manifest(
            "empties-elsewhere",
            "[workspace]\ndefault-members = [\"crates/cli\"]\n",
        )?;
        assert!(!elsewhere.empties_default_members(Path::new("tasks/lint")));
        Ok(())
    }

    #[test]
    fn default_members_that_is_absent_or_already_empty_is_not_emptied() -> TestOutcome {
        let (_scratch, absent) = read_manifest("empties-absent", "[workspace]\nmembers = []\n")?;
        assert!(!absent.empties_default_members(Path::new("tasks/lint")));

        let (_scratch, empty) =
            read_manifest("empties-empty", "[workspace]\ndefault-members = []\n")?;
        assert!(!empty.empties_default_members(Path::new("tasks/lint")));

        let (_scratch, no_workspace) =
            read_manifest("empties-no-workspace", "[package]\nname = \"cli\"\n")?;
        assert!(!no_workspace.empties_default_members(Path::new("tasks/lint")));
        Ok(())
    }

    // `globs_left_matching_nothing`.

    /// A scratch workspace with `Cargo.toml` holding `members`, and each of
    /// `directories` made under it.
    fn workspace_with(
        tag: &str,
        members: &str,
        directories: &[&str],
    ) -> Result<(ScratchDir, Manifest), Box<dyn Error>> {
        let scratch = ScratchDir::new(tag)?;
        for directory in directories {
            std::fs::create_dir_all(scratch.path().join(directory))?;
        }
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(&path, format!("[workspace]\n{members}\n"))?;
        let manifest = Manifest::read(&path)?;
        Ok((scratch, manifest))
    }

    #[test]
    fn a_glob_whose_only_match_is_the_directory_is_left_matching_nothing() -> TestOutcome {
        let (_scratch, manifest) = workspace_with(
            "globs-last-match",
            "members = [\"ritual\", \"tasks/*\"]\ndefault-members = [\"tasks/l*\"]",
            &["ritual", "tasks/lint"],
        )?;

        assert_eq!(
            manifest.globs_left_matching_nothing(Path::new("tasks/lint"))?,
            ["tasks/*", "tasks/l*"]
        );
        Ok(())
    }

    #[test]
    fn a_glob_with_another_match_is_not_left_matching_nothing() -> TestOutcome {
        let (_scratch, manifest) = workspace_with(
            "globs-other-match",
            "members = [\"ritual\", \"tasks/*\"]",
            &["ritual", "tasks/lint", "tasks/format"],
        )?;

        assert!(
            manifest
                .globs_left_matching_nothing(Path::new("tasks/lint"))?
                .is_empty()
        );
        Ok(())
    }

    /// Cargo counts any match toward a glob, a file included, and reads only
    /// a glob that matches nothing as a literal path.
    #[test]
    fn a_glob_that_still_matches_a_file_is_not_left_matching_nothing() -> TestOutcome {
        let (scratch, manifest) = workspace_with(
            "globs-file-match",
            "members = [\"tasks/*\"]",
            &["tasks/lint"],
        )?;
        std::fs::write(scratch.path().join("tasks/.DS_Store"), "")?;

        assert!(
            manifest
                .globs_left_matching_nothing(Path::new("tasks/lint"))?
                .is_empty()
        );
        Ok(())
    }

    /// A glob that matches nothing already is how the workspace was found,
    /// not something deleting the directory does to it.
    #[test]
    fn a_glob_that_matches_nothing_already_is_not_reported() -> TestOutcome {
        let (_scratch, manifest) = workspace_with(
            "globs-no-match",
            "members = [\"other/*\", \"tasks/lint\"]",
            &["tasks/lint"],
        )?;

        assert!(
            manifest
                .globs_left_matching_nothing(Path::new("tasks/lint"))?
                .is_empty()
        );
        Ok(())
    }

    /// A glob matching only inside the directory loses every match with it.
    #[test]
    fn a_glob_matching_inside_the_directory_is_left_matching_nothing() -> TestOutcome {
        let (_scratch, manifest) = workspace_with(
            "globs-inside",
            "members = [\"tasks/lint/*\"]",
            &["tasks/lint/inner"],
        )?;

        assert_eq!(
            manifest.globs_left_matching_nothing(Path::new("tasks/lint"))?,
            ["tasks/lint/*"]
        );
        Ok(())
    }

    #[test]
    fn an_invalid_glob_is_a_failure_naming_it() -> TestOutcome {
        let (_scratch, manifest) =
            workspace_with("globs-invalid", "members = [\"tasks/[\"]", &["tasks"])?;

        let failure = manifest
            .globs_left_matching_nothing(Path::new("tasks/lint"))
            .err()
            .ok_or("an invalid glob was meant to fail")?;
        assert!(failure.to_string().contains("`tasks/[`"), "{failure}");
        Ok(())
    }

    // `entries_pointing_under`.

    #[test]
    fn patches_replacements_and_workspace_dependencies_under_it_are_named() -> TestOutcome {
        let (_scratch, manifest) = read_manifest(
            "entries-pointing-under",
            "[workspace.dependencies]\nlint = { path = \"tasks/lint\" }\n\
             helper = { path = \"tasks/./lint/helper\" }\nother = { path = \"tasks/lint-extra\" }\n\
             registry = \"1\"\n\n\
             [patch.crates-io]\nserde = { path = \"tasks/lint/vendor/serde\" }\nok = \"1\"\n\n\
             [patch.'https://example.invalid/x']\nx = { path = \"x/../tasks/lint\" }\n\n\
             [replace]\n\"foo:0.1.0\" = { path = \"tasks/lint/foo\" }\n\
             \"bar:0.1.0\" = { path = \"elsewhere\" }\n",
        )?;

        assert_eq!(
            manifest.entries_pointing_under(Path::new("tasks/lint"), Some("lint")),
            [
                "[workspace.dependencies] helper",
                "[patch.crates-io] serde",
                "[patch.https://example.invalid/x] x",
                "[replace] \"foo:0.1.0\"",
            ]
        );
        assert_eq!(
            manifest.entries_pointing_under(Path::new("tasks/lint"), None)[0],
            "[workspace.dependencies] lint",
            "the entry is named when the caller does not drop it"
        );
        Ok(())
    }

    #[test]
    fn a_manifest_with_nothing_pointing_under_the_directory_names_nothing() -> TestOutcome {
        let (_scratch, manifest) = read_manifest(
            "entries-pointing-nowhere",
            "[package]\nname = \"cli\"\n\n[patch.crates-io]\nserde = { git = \"https://x\" }\n",
        )?;

        assert!(
            manifest
                .entries_pointing_under(Path::new("tasks/lint"), None)
                .is_empty()
        );
        Ok(())
    }
}
