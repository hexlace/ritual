//! Repointing the paths a manifest writes when directories move.
//!
//! Cargo reads a path out of a manifest in a good many places: a dependency's
//! `path`, a `[patch]` or `[replace]` entry, a target's source file, a
//! package's `build`, `readme` and `license-file`, and the `[workspace]`
//! lists of members. When a directory moves, every one of them that reaches
//! it, or that is read from a manifest that moved, has to be rewritten, and
//! every one that does not has to be left exactly as a person wrote it.

mod members;

use std::path::{Path, PathBuf};

use rituals::Failure;
use toml_edit::{DocumentMut, Item, TableLike, Value};

use super::{Manifest, PathChange};
use crate::paths::{lies_under, normalize};
use crate::relocation::Relocation;

/// The tables of dependencies a manifest, or one of its `[target.<t>]`
/// tables, can hold, in both spellings Cargo accepts for the dashed ones.
const DEPENDENCY_TABLES: [&str; 5] = [
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
    "dev_dependencies",
    "build_dependencies",
];

/// The arrays of tables that name a source file of a build target.
const TARGET_ARRAYS: [&str; 4] = ["bin", "example", "test", "bench"];

/// The `[package]` keys whose value is a path.
const PACKAGE_PATHS: [&str; 4] = ["build", "readme", "license-file", "workspace"];

/// The `[workspace.package]` keys whose value is a path.
const WORKSPACE_PACKAGE_PATHS: [&str; 2] = ["readme", "license-file"];

impl Manifest {
    /// Repoints every path this manifest writes that a move changes, in the
    /// document in memory only, and returns what it changed.
    ///
    /// Nothing is written: the caller writes the manifest with
    /// [`Manifest::write`], inside the run that does the move. The manifest
    /// is read as it is before the move, in its own directory, and is taken
    /// to be in the directory the relocation sends that to afterwards, or
    /// the same one if it does not move. A path that still reaches what it
    /// reached is left as the person wrote it.
    ///
    /// Every string path in these places goes through the relocation:
    ///
    /// - a dependency's `path`, in `dependencies`, `dev-dependencies` and
    ///   `build-dependencies` and their underscore spellings, at the top
    ///   level and in every `[target.<t>]` table;
    /// - `[workspace.dependencies]`, `[patch.<source>]` and `[replace]`;
    /// - `[package]`'s `build`, `workspace`, `readme` and `license-file`, and
    ///   `[workspace.package]`'s `readme` and `license-file`;
    /// - `[lib]`'s `path`, and the `path` of every `[[bin]]`, `[[example]]`,
    ///   `[[test]]` and `[[bench]]`;
    /// - the entries of `[workspace]` `members`, `default-members` and
    ///   `exclude`. A glob that matches a directory that moves is replaced
    ///   by the same glob under the new place when nothing else it matches
    ///   stays, and otherwise is kept, with the new place's glob added beside
    ///   it. A glob that matches nothing that moves, or that lies outside the
    ///   moved directories' parent, is left alone.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::manifest::Manifest;
    /// use rituals_compose::relocation::Relocation;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-manifest-repoint-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(
    /// #     &manifest_path,
    /// #     "[dependencies]\n# the greeting\ngreet = { path = \"tasks/greet\" }\n",
    /// # )?;
    /// let mut manifest = Manifest::read(&manifest_path)?;
    /// let relocation = Relocation::new(
    ///     &directory.join("tasks"),
    ///     &directory.join(".rituals"),
    ///     [directory.join("tasks/greet")],
    /// );
    ///
    /// let changes = manifest.repoint(&relocation)?;
    ///
    /// assert_eq!(changes.len(), 1);
    /// assert_eq!(
    ///     changes[0].to_string(),
    ///     "[dependencies] greet path `tasks/greet` is now `.rituals/greet`"
    /// );
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] before changing anything when a path reaches a
    /// moved directory by what is on disk but not as written, such as
    /// through a symbolic link or a differently spelled directory, since
    /// then there is no telling how to repoint it. It also returns the
    /// failure of expanding a `members` glob that cannot be expanded. The
    /// document is unchanged when it fails.
    ///
    /// # Panics
    ///
    /// Panics if [`Manifest::path`] is not absolute.
    pub fn repoint(&mut self, relocation: &Relocation) -> Result<Vec<PathChange>, Failure> {
        let base_before = self.directory();
        assert!(
            base_before.is_absolute(),
            "a manifest to repoint must be at an absolute path, got {}",
            self.path.display()
        );
        let base_after = relocation
            .destination(&base_before)
            .unwrap_or_else(|| base_before.clone());

        // Edited as a copy, so a refusal partway through leaves this
        // document exactly as it was.
        let mut document = self.document.clone();
        let mut repointer = Repointer {
            manifest_path: &self.path,
            relocation,
            base_before,
            base_after,
            changes: Vec::new(),
        };
        repointer.repoint(&mut document)?;

        let changes = repointer.changes;
        self.document = document;
        Ok(changes)
    }
}

/// One run of [`Manifest::repoint`]: what it reads, and what it has changed
/// so far.
struct Repointer<'a> {
    manifest_path: &'a Path,
    relocation: &'a Relocation,
    /// The directory the manifest is in now, which every relative path in it
    /// is read from.
    base_before: PathBuf,
    /// The directory it is in once the move is done.
    base_after: PathBuf,
    changes: Vec<PathChange>,
}

impl Repointer<'_> {
    fn repoint(&mut self, document: &mut DocumentMut) -> Result<(), Failure> {
        let root = document.as_table_mut();
        self.package(root)?;
        self.targets(root)?;
        self.dependency_tables(root, "")?;
        if let Some(targets) = root.get_mut("target").and_then(Item::as_table_like_mut) {
            for (target, tables) in targets.iter_mut() {
                if let Some(tables) = tables.as_table_like_mut() {
                    let prefix = format!("target.{}.", target.get());
                    self.dependency_tables(tables, &prefix)?;
                }
            }
        }
        if let Some(workspace) = root.get_mut("workspace").and_then(Item::as_table_like_mut) {
            self.workspace(workspace)?;
        }
        self.patches(root)?;
        self.replacements(root)
    }

    /// `[package]`'s `build`, `workspace`, `readme` and `license-file`.
    fn package(&mut self, root: &mut dyn TableLike) -> Result<(), Failure> {
        let Some(package) = root.get_mut("package").and_then(Item::as_table_like_mut) else {
            return Ok(());
        };
        for field in PACKAGE_PATHS {
            if let Some(item) = package.get_mut(field) {
                self.repoint_item(item, &format!("[package] {field}"))?;
            }
        }
        Ok(())
    }

    /// `[lib]`'s `path` and the `path` of every `[[bin]]`, `[[example]]`,
    /// `[[test]]` and `[[bench]]`.
    fn targets(&mut self, root: &mut dyn TableLike) -> Result<(), Failure> {
        if let Some(library) = root.get_mut("lib").and_then(Item::as_table_like_mut) {
            if let Some(item) = library.get_mut("path") {
                self.repoint_item(item, "[lib] path")?;
            }
        }
        for kind in TARGET_ARRAYS {
            let Some(item) = root.get_mut(kind) else {
                continue;
            };
            for target in tables_of(item) {
                let place = target.get("name").and_then(Item::as_str).map_or_else(
                    || format!("[[{kind}]] path"),
                    |name| format!("[[{kind}]] {name} path"),
                );
                if let Some(item) = target.get_mut("path") {
                    self.repoint_item(item, &place)?;
                }
            }
        }
        Ok(())
    }

    /// Every dependency table in `holder`, which is the manifest's root or a
    /// `[target.<t>]` table, named `[<prefix><table>]` in a report.
    fn dependency_tables(
        &mut self,
        holder: &mut dyn TableLike,
        prefix: &str,
    ) -> Result<(), Failure> {
        for table in DEPENDENCY_TABLES {
            if let Some(declarations) = holder.get_mut(table).and_then(Item::as_table_like_mut) {
                self.declarations(declarations, &format!("[{prefix}{table}]"))?;
            }
        }
        Ok(())
    }

    /// The `path` of every dependency declared in `declarations`, which is
    /// named `label` in a report.
    fn declarations(
        &mut self,
        declarations: &mut dyn TableLike,
        label: &str,
    ) -> Result<(), Failure> {
        for (key, declaration) in declarations.iter_mut() {
            let Some(declaration) = declaration.as_table_like_mut() else {
                continue;
            };
            if let Some(item) = declaration.get_mut("path") {
                self.repoint_item(item, &format!("{label} {} path", key.get()))?;
            }
        }
        Ok(())
    }

    /// `[workspace.dependencies]`, `[workspace.package]` and the lists of
    /// members.
    fn workspace(&mut self, workspace: &mut dyn TableLike) -> Result<(), Failure> {
        self.member_lists(workspace)?;
        if let Some(package) = workspace
            .get_mut("package")
            .and_then(Item::as_table_like_mut)
        {
            for field in WORKSPACE_PACKAGE_PATHS {
                if let Some(item) = package.get_mut(field) {
                    self.repoint_item(item, &format!("[workspace.package] {field}"))?;
                }
            }
        }
        if let Some(declarations) = workspace
            .get_mut("dependencies")
            .and_then(Item::as_table_like_mut)
        {
            self.declarations(declarations, "[workspace.dependencies]")?;
        }
        Ok(())
    }

    /// Every `[patch.<source>]` entry's `path`.
    fn patches(&mut self, root: &mut dyn TableLike) -> Result<(), Failure> {
        let Some(sources) = root.get_mut("patch").and_then(Item::as_table_like_mut) else {
            return Ok(());
        };
        for (source, declarations) in sources.iter_mut() {
            if let Some(declarations) = declarations.as_table_like_mut() {
                self.declarations(declarations, &format!("[patch.{}]", source.get()))?;
            }
        }
        Ok(())
    }

    /// Every `[replace]` entry's `path`, each named by its specification.
    fn replacements(&mut self, root: &mut dyn TableLike) -> Result<(), Failure> {
        let Some(replacements) = root.get_mut("replace").and_then(Item::as_table_like_mut) else {
            return Ok(());
        };
        for (specification, declaration) in replacements.iter_mut() {
            let Some(declaration) = declaration.as_table_like_mut() else {
                continue;
            };
            if let Some(item) = declaration.get_mut("path") {
                self.repoint_item(item, &format!("[replace] `{}` path", specification.get()))?;
            }
        }
        Ok(())
    }

    /// Repoints `item` when it is a string, and leaves it alone when it is
    /// anything else: `readme = true` and `readme.workspace = true` are not
    /// paths.
    fn repoint_item(&mut self, item: &mut Item, place: &str) -> Result<(), Failure> {
        item.as_value_mut()
            .map_or(Ok(()), |value| self.repoint_value(value, place))
    }

    /// Repoints the string `value`, if it is one, replacing it in place so
    /// the spacing and comments around it stay.
    fn repoint_value(&mut self, value: &mut Value, place: &str) -> Result<(), Failure> {
        let Some(written) = value.as_str().map(str::to_string) else {
            return Ok(());
        };
        let Some(new) = self.respelled(&written, place)? else {
            return Ok(());
        };
        replace_string(value, &new);
        self.changes
            .push(PathChange::repointed(place, &written, &new));
        Ok(())
    }

    /// What `written` should say after the move, or `None` to keep it.
    ///
    /// Refuses a path that reaches a moved directory by what is on disk but
    /// not as written: it would be left pointing at a directory that is no
    /// longer there, and writing it differently would be a guess.
    fn respelled(&self, written: &str, place: &str) -> Result<Option<String>, Failure> {
        let joined = self.base_before.join(written);
        let reached = self
            .relocation
            .moved()
            .iter()
            .find(|moved| lies_under(&joined, moved));
        let moves_as_written = self.relocation.destination(&normalize(&joined)).is_some();
        if let (Some(moved), false) = (reached, moves_as_written) {
            return Err(Failure::new(format!(
                "{}: {place} `{written}` reaches {} through a symbolic link or a differently \
                 spelled directory, so ritual cannot tell how to repoint it; spell it through \
                 {}, or move {} by hand",
                self.manifest_path.display(),
                moved.display(),
                self.relocation.moved_out_of().display(),
                moved.display()
            )));
        }
        Ok(self
            .relocation
            .repointed(written, &self.base_before, &self.base_after))
    }
}

/// Every table `item` holds, whether it is written as `[[name]]` tables or
/// as an inline array of inline tables.
fn tables_of(item: &mut Item) -> Vec<&mut dyn TableLike> {
    match item {
        Item::ArrayOfTables(tables) => tables
            .iter_mut()
            .map(|table| table as &mut dyn TableLike)
            .collect(),
        Item::Value(Value::Array(values)) => values
            .iter_mut()
            .filter_map(Value::as_inline_table_mut)
            .map(|table| table as &mut dyn TableLike)
            .collect(),
        Item::None | Item::Value(_) | Item::Table(_) => Vec::new(),
    }
}

/// Puts the string `new` where `value` is, with the spacing and comment
/// around it as they were.
fn replace_string(value: &mut Value, new: &str) {
    let decor = value.decor().clone();
    *value = Value::from(new);
    *value.decor_mut() = decor;
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use toml_edit::DocumentMut;

    use super::Manifest;
    use crate::relocation::Relocation;
    use crate::test_support::{ScratchDir, TestOutcome};

    /// A scratch project with `tasks/greet`, `tasks/shout` and `tasks/helper`
    /// directories, and the manifest `at` holding `content`.
    pub(super) fn project_with_manifest(
        tag: &str,
        at: &str,
        content: &str,
    ) -> Result<(ScratchDir, Manifest), Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new(tag)?;
        for directory in ["tasks/greet", "tasks/shout", "tasks/helper", "ritual"] {
            std::fs::create_dir_all(scratch.path().join(directory))?;
        }
        let path = scratch.path().join(at);
        std::fs::create_dir_all(path.parent().ok_or("a manifest has a directory")?)?;
        std::fs::write(&path, content)?;
        let manifest = Manifest::read(&path)?;
        Ok((scratch, manifest))
    }

    /// `tasks/greet` and `tasks/shout` moving to `.rituals/`, as the first
    /// step of `migrate` moves them; `tasks/helper` stays.
    pub(super) fn relocation(root: &Path) -> Relocation {
        Relocation::new(
            &root.join("tasks"),
            &root.join(".rituals"),
            [root.join("tasks/greet"), root.join("tasks/shout")],
        )
    }

    /// What `repoint` reported, as a person reads it.
    pub(super) fn reported(
        manifest: &mut Manifest,
        relocation: &Relocation,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        Ok(manifest
            .repoint(relocation)?
            .iter()
            .map(ToString::to_string)
            .collect())
    }

    /// Every table a dependency's `path` can be written in, at the top level
    /// and under a target, in each shape a dependency is written: an inline
    /// table, a dotted key and a table of its own. Each is rewritten where it
    /// is, with the comment and spacing around it as the person wrote them.
    #[test]
    fn a_dependency_path_is_rewritten_in_every_table_and_shape_it_can_be_written_in() -> TestOutcome
    {
        let (scratch, mut manifest) = project_with_manifest(
            "repoint-dependencies",
            "ritual/Cargo.toml",
            "[dependencies]\n\
             greet = { path = \"../tasks/greet\" } # the greeting\n\
             shout.path = \"../tasks/shout\"\n\
             serde = \"1\"\n\
             \n\
             [dev-dependencies]\n\
             greet = { version = \"0.1\", path = \"../tasks/greet\" }\n\
             \n\
             [build_dependencies]\n\
             greet = { path = \"../tasks/greet/\" }\n\
             \n\
             [target.'cfg(unix)'.dependencies]\n\
             shout = { path = \"../tasks/shout\" }\n\
             \n\
             [dependencies.extra]\n\
             path = \"../tasks/greet/extra\"\n",
        )?;

        let changes = reported(&mut manifest, &relocation(scratch.path()))?;

        assert_eq!(
            changes,
            [
                "[dependencies] greet path `../tasks/greet` is now `../.rituals/greet`",
                "[dependencies] shout path `../tasks/shout` is now `../.rituals/shout`",
                "[dependencies] extra path `../tasks/greet/extra` is now `../.rituals/greet/extra`",
                "[dev-dependencies] greet path `../tasks/greet` is now `../.rituals/greet`",
                "[build_dependencies] greet path `../tasks/greet/` is now `../.rituals/greet`",
                "[target.cfg(unix).dependencies] shout path `../tasks/shout` is now `../.rituals/shout`",
            ]
        );
        assert_eq!(
            manifest.document.to_string(),
            "[dependencies]\n\
             greet = { path = \"../.rituals/greet\" } # the greeting\n\
             shout.path = \"../.rituals/shout\"\n\
             serde = \"1\"\n\
             \n\
             [dev-dependencies]\n\
             greet = { version = \"0.1\", path = \"../.rituals/greet\" }\n\
             \n\
             [build_dependencies]\n\
             greet = { path = \"../.rituals/greet\" }\n\
             \n\
             [target.'cfg(unix)'.dependencies]\n\
             shout = { path = \"../.rituals/shout\" }\n\
             \n\
             [dependencies.extra]\n\
             path = \"../.rituals/greet/extra\"\n"
        );
        Ok(())
    }

    /// A manifest with nothing a move touches comes back byte for byte, with
    /// nothing reported: a registry dependency, a path outside every moved
    /// directory, a sibling that merely shares a prefix, and values that are
    /// not strings.
    #[test]
    fn a_manifest_no_move_touches_is_unchanged_and_reports_nothing() -> TestOutcome {
        let content = "[package]\n\
                       name = \"ritual\"\n\
                       readme = true\n\
                       license-file = { workspace = true }\n\
                       \n\
                       [dependencies]\n\
                       serde = \"1\"\n\
                       vendored = { path = \"../vendor/x\" }\n\
                       helper = { path = \"../tasks/helper\" }\n\
                       lookalike = { path = \"../tasks/greet-extra\" }\n\
                       inherited.workspace = true\n\
                       numbered = { path = 5 }\n";
        let (scratch, mut manifest) =
            project_with_manifest("repoint-untouched", "ritual/Cargo.toml", content)?;

        let changes = reported(&mut manifest, &relocation(scratch.path()))?;

        assert_eq!(changes, Vec::<String>::new());
        assert_eq!(manifest.document.to_string(), content);
        Ok(())
    }

    /// Nothing at all is as valid a manifest as any other.
    #[test]
    fn an_empty_manifest_has_nothing_to_repoint() -> TestOutcome {
        let (scratch, mut manifest) =
            project_with_manifest("repoint-empty", "ritual/Cargo.toml", "")?;

        let changes = reported(&mut manifest, &relocation(scratch.path()))?;

        assert_eq!(changes, Vec::<String>::new());
        assert_eq!(manifest.document.to_string(), "");
        Ok(())
    }

    /// A task that moves keeps every path that still leads where it did: the
    /// other task, which moves with it, and anything outside `tasks/` at the
    /// same depth. Only a path to something that stays beside it is
    /// respelled, because it is one directory deeper now.
    #[test]
    fn a_manifest_that_moves_keeps_what_still_works_and_respells_what_does_not() -> TestOutcome {
        let (scratch, mut manifest) = project_with_manifest(
            "repoint-moved-manifest",
            "tasks/shout/Cargo.toml",
            "[package]\n\
             name = \"shout\"\n\
             build = \"build.rs\"\n\
             readme = \"../../readme.md\"\n\
             \n\
             [lib]\n\
             path = \"src/lib.rs\"\n\
             \n\
             [dependencies]\n\
             greet = { path = \"../greet\" }\n\
             vendored = { path = \"../../vendor/x\" }\n\
             helper = { path = \"../helper\" }\n\
             \n\
             [dev-dependencies]\n\
             beside = { path = \"../../tasks/helper\" }\n",
        )?;

        let changes = reported(&mut manifest, &relocation(scratch.path()))?;

        assert_eq!(
            changes,
            ["[dependencies] helper path `../helper` is now `../../tasks/helper`"]
        );
        assert_eq!(
            manifest.document.to_string(),
            "[package]\n\
             name = \"shout\"\n\
             build = \"build.rs\"\n\
             readme = \"../../readme.md\"\n\
             \n\
             [lib]\n\
             path = \"src/lib.rs\"\n\
             \n\
             [dependencies]\n\
             greet = { path = \"../greet\" }\n\
             vendored = { path = \"../../vendor/x\" }\n\
             helper = { path = \"../../tasks/helper\" }\n\
             \n\
             [dev-dependencies]\n\
             beside = { path = \"../../tasks/helper\" }\n"
        );
        Ok(())
    }

    /// Every other key Cargo reads a path from.
    #[test]
    fn the_package_target_and_workspace_paths_are_rewritten_too() -> TestOutcome {
        let (scratch, mut manifest) = project_with_manifest(
            "repoint-other-keys",
            "Cargo.toml",
            "bench = [{ name = \"speed\", path = \"tasks/shout/benches/speed.rs\" }]\n\
             \n\
             [package]\n\
             name = \"ritual\"\n\
             build = \"tasks/greet/build.rs\"\n\
             readme = \"tasks/greet/readme.md\"\n\
             license-file = \"./tasks/shout/LICENSE\"\n\
             workspace = \"tasks/greet\"\n\
             \n\
             [lib]\n\
             path = \"tasks/greet/src/lib.rs\"\n\
             \n\
             [[bin]]\n\
             name = \"tool\"\n\
             path = \"tasks/shout/src/main.rs\"\n\
             \n\
             [[example]]\n\
             path = \"tasks/greet/examples/hello.rs\"\n\
             \n\
             [[test]]\n\
             name = \"flow\"\n\
             path = \"tasks/greet/tests/flow.rs\"\n\
             \n\
             [workspace.package]\n\
             readme = \"tasks/greet/readme.md\"\n\
             license-file = \"tasks/shout/LICENSE\"\n\
             \n\
             [workspace.dependencies]\n\
             greet = { path = \"tasks/greet\" }\n\
             \n\
             [patch.crates-io]\n\
             shout = { path = \"tasks/shout\" }\n\
             \n\
             [patch.\"https://example.com/index\"]\n\
             greet = { path = \"tasks/greet\" }\n\
             \n\
             [replace]\n\
             \"greet:0.1.0\" = { path = \"tasks/greet\" }\n",
        )?;

        let changes = reported(&mut manifest, &relocation(scratch.path()))?;

        assert_eq!(
            changes,
            [
                "[package] build `tasks/greet/build.rs` is now `.rituals/greet/build.rs`",
                "[package] readme `tasks/greet/readme.md` is now `.rituals/greet/readme.md`",
                "[package] license-file `./tasks/shout/LICENSE` is now `.rituals/shout/LICENSE`",
                "[package] workspace `tasks/greet` is now `.rituals/greet`",
                "[lib] path `tasks/greet/src/lib.rs` is now `.rituals/greet/src/lib.rs`",
                "[[bin]] tool path `tasks/shout/src/main.rs` is now `.rituals/shout/src/main.rs`",
                "[[example]] path `tasks/greet/examples/hello.rs` is now `.rituals/greet/examples/hello.rs`",
                "[[test]] flow path `tasks/greet/tests/flow.rs` is now `.rituals/greet/tests/flow.rs`",
                "[[bench]] speed path `tasks/shout/benches/speed.rs` is now `.rituals/shout/benches/speed.rs`",
                "[workspace.package] readme `tasks/greet/readme.md` is now `.rituals/greet/readme.md`",
                "[workspace.package] license-file `tasks/shout/LICENSE` is now `.rituals/shout/LICENSE`",
                "[workspace.dependencies] greet path `tasks/greet` is now `.rituals/greet`",
                "[patch.crates-io] shout path `tasks/shout` is now `.rituals/shout`",
                "[patch.https://example.com/index] greet path `tasks/greet` is now `.rituals/greet`",
                "[replace] `greet:0.1.0` path `tasks/greet` is now `.rituals/greet`",
            ]
        );
        let document = manifest.document.to_string();
        assert!(
            !document.contains("\"tasks/"),
            "every path into `tasks/` must have been rewritten:\n{document}"
        );
        Ok(())
    }

    /// An absolute path stays absolute, and is rewritten only when it leads
    /// to a task that moves.
    #[test]
    fn an_absolute_path_is_rewritten_to_an_absolute_path() -> TestOutcome {
        let (scratch, manifest) =
            project_with_manifest("repoint-absolute", "ritual/Cargo.toml", "[dependencies]\n")?;
        let root = scratch.path();
        let content = format!(
            "[dependencies]\n\
             greet = {{ path = \"{root}/tasks/greet\" }}\n\
             helper = {{ path = \"{root}/tasks/helper\" }}\n",
            root = root.display()
        );
        std::fs::write(manifest.path(), &content)?;
        let mut manifest = Manifest::read(manifest.path())?;

        let changes = reported(&mut manifest, &relocation(root))?;

        assert_eq!(
            changes,
            [format!(
                "[dependencies] greet path `{root}/tasks/greet` is now `{root}/.rituals/greet`",
                root = root.display()
            )]
        );
        assert_eq!(
            manifest.document.to_string(),
            format!(
                "[dependencies]\n\
                 greet = {{ path = \"{root}/.rituals/greet\" }}\n\
                 helper = {{ path = \"{root}/tasks/helper\" }}\n",
                root = root.display()
            )
        );
        Ok(())
    }

    /// A path that gets to a moved directory through a link cannot be
    /// repointed by reading it, and writing it differently would be a guess,
    /// so the manifest is refused and nothing it edited before is kept.
    #[test]
    fn a_path_through_a_symbolic_link_to_a_moved_directory_is_refused_and_changes_nothing()
    -> TestOutcome {
        let content = "[dependencies]\n\
                       greet = { path = \"../tasks/greet\" }\n\
                       linked = { path = \"../alias/greet\" }\n";
        let (scratch, mut manifest) =
            project_with_manifest("repoint-link", "ritual/Cargo.toml", content)?;
        std::os::unix::fs::symlink("tasks", scratch.path().join("alias"))?;

        let outcome = manifest.repoint(&relocation(scratch.path()));

        let Err(failure) = outcome else {
            return Err("a path through a link to a moved directory must be refused".into());
        };
        assert_eq!(
            failure.to_string(),
            format!(
                "{manifest}: [dependencies] linked path `../alias/greet` reaches {moved} through \
                 a symbolic link or a differently spelled directory, so ritual cannot tell how \
                 to repoint it; spell it through {from}, or move {moved} by hand",
                manifest = manifest.path().display(),
                moved = scratch.path().join("tasks/greet").display(),
                from = scratch.path().join("tasks").display(),
            )
        );
        assert_eq!(
            manifest.document.to_string(),
            content,
            "a refused manifest keeps every edit it would have made out of the document"
        );
        Ok(())
    }

    /// The same refusal for a member entry and for an absolute path, each
    /// named as a person finds it in the manifest.
    #[test]
    fn a_member_or_an_absolute_path_through_a_link_is_refused_by_its_place() -> TestOutcome {
        let (scratch, mut manifest) = project_with_manifest(
            "repoint-link-member",
            "Cargo.toml",
            "[workspace]\nmembers = [\"alias/greet\"]\n",
        )?;
        std::os::unix::fs::symlink("tasks", scratch.path().join("alias"))?;
        let outcome = manifest.repoint(&relocation(scratch.path()));
        let failure = outcome
            .err()
            .ok_or("a member through a link must be refused")?;
        assert!(
            failure
                .to_string()
                .contains("[workspace] members `alias/greet` reaches "),
            "{failure}"
        );

        let absolute = format!(
            "[dependencies]\ngreet = {{ path = \"{}/alias/greet\" }}\n",
            scratch.path().display()
        );
        std::fs::write(manifest.path(), absolute)?;
        let mut manifest = Manifest::read(manifest.path())?;
        let outcome = manifest.repoint(&relocation(scratch.path()));
        let failure = outcome
            .err()
            .ok_or("an absolute path through a link must be refused")?;
        assert!(
            failure.to_string().contains("[dependencies] greet path `"),
            "{failure}"
        );
        Ok(())
    }

    /// A moved directory reached through a link is refused wherever the link
    /// is; the same link to something that does not move is not a reason to
    /// refuse.
    #[test]
    fn a_path_through_a_link_to_something_that_stays_is_not_refused() -> TestOutcome {
        let content = "[dependencies]\nhelper = { path = \"../alias/helper\" }\n";
        let (scratch, mut manifest) =
            project_with_manifest("repoint-link-stays", "ritual/Cargo.toml", content)?;
        std::os::unix::fs::symlink("tasks", scratch.path().join("alias"))?;

        let changes = reported(&mut manifest, &relocation(scratch.path()))?;

        assert_eq!(changes, Vec::<String>::new());
        assert_eq!(manifest.document.to_string(), content);
        Ok(())
    }

    #[test]
    #[should_panic(expected = "a manifest to repoint must be at an absolute path")]
    fn a_manifest_at_a_relative_path_is_a_bug() {
        let mut manifest = Manifest {
            path: PathBuf::from("Cargo.toml"),
            document: DocumentMut::new(),
        };
        let _ = manifest.repoint(&relocation(Path::new("/w")));
    }
}
