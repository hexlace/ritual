//! What a composed CLI's `[package.metadata.ritual] tasks` keys import, and
//! what else in the workspace depends on it: the facts a task that takes
//! one out has to know before it writes anything.
//!
//! Reached through [`crate::metadata::Metadata::task_imports`] — this
//! module holds the behaviour and the [`TaskImport`] it returns, that method
//! is the entry point.
//
// `redundant_pub_crate` (clippy nursery) wants `pub` here because this
// module is private, but `pub(crate)` is the visibility that is actually
// true — it stays correct if this module is ever re-exported at a different
// level — so the nursery lint gives way.
#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) reflects this module's actual visibility even though the enclosing \
              module is private; see the note above"
)]

use std::path::{Path, PathBuf};

use rituals::Failure;

use crate::metadata::{self, Dependency, Metadata, Package};
use crate::paths::lies_under;
use crate::rust_name::extern_identifier;
use crate::{project, task_list};

/// One key in a composed CLI's `[package.metadata.ritual] tasks` list, and
/// what it imports.
///
/// Every question about the dependency behind the key is answered from what
/// `cargo metadata` reports: which package the key reaches and where that
/// package lives from the resolved graph, whether it is a member of the
/// workspace from Cargo's own list, and what else relies on it from what
/// each package declares. A key with no normal dependency behind it answers
/// every one of them with nothing.
///
/// Got from [`crate::metadata::Metadata::task_imports`].
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::metadata;
///
/// // Reads a document `fetch` already produced from a real `cargo
/// // metadata` call, so this example stays `no_run`.
/// let document = metadata::fetch(Path::new("."))?;
/// for task in document.task_imports("demo-ritual")? {
///     match task.directory() {
///         Some(directory) if task.is_workspace_member() => {
///             println!("`{}` lives in {}", task.key(), directory.display());
///         }
///         _ => println!("`{}` is not a member of this workspace", task.key()),
///     }
/// }
/// # Ok::<(), rituals::Failure>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskImport<'a> {
    key: &'a str,
    dependency: Option<DependencyLine<'a>>,
    directory: Option<&'a Path>,
    is_workspace_member: bool,
    other_dependents: Vec<&'a str>,
    members_inside: Vec<&'a str>,
}

/// The composed CLI's dependency line behind a key: the key it is written
/// under, which Rust reads as the same name as the `tasks` key, and the
/// package it names. One value, so a key cannot have one without the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DependencyLine<'a> {
    key: &'a str,
    package_name: &'a str,
}

impl<'a> TaskImport<'a> {
    /// A key whose dependency, if it has one, resolves to nothing: no
    /// package, no directory, not a member, nothing depending on it.
    const fn unresolved(key: &'a str, dependency: Option<DependencyLine<'a>>) -> Self {
        Self {
            key,
            dependency,
            directory: None,
            is_workspace_member: false,
            other_dependents: Vec::new(),
            members_inside: Vec::new(),
        }
    }

    /// The key as `[package.metadata.ritual] tasks` spells it.
    #[must_use]
    pub const fn key(&self) -> &'a str {
        self.key
    }

    /// The key the dependency is written under in the composed CLI's
    /// manifest, or `None` when no normal dependency of the composed CLI has
    /// this key.
    ///
    /// Rust reads `-` and `_` in a dependency key as one name, so this is
    /// [`TaskImport::key`] or its other spelling: a `tasks` entry `a-b` is
    /// mounted from a dependency written `a_b`, the same way
    /// [`Metadata::resolve_task_list`](crate::metadata::Metadata::resolve_task_list)
    /// finds it. A task that edits the dependency line edits it under this
    /// key.
    #[must_use]
    pub fn dependency_key(&self) -> Option<&'a str> {
        self.dependency.map(|dependency| dependency.key)
    }

    /// The name of the package the key imports, or `None` when no normal
    /// dependency of the composed CLI has this key.
    ///
    /// A dev- or build-dependency under the key does not count: a task is
    /// mounted from a normal dependency.
    #[must_use]
    pub fn package_name(&self) -> Option<&'a str> {
        self.dependency.map(|dependency| dependency.package_name)
    }

    /// The directory the dependency's `path` points at, or `None` for a
    /// dependency from a registry or git, which has none in this project.
    #[must_use]
    pub const fn directory(&self) -> Option<&'a Path> {
        self.directory
    }

    /// Whether the imported package is a member of this workspace, as
    /// opposed to a path dependency that lives outside it.
    ///
    /// Answered by Cargo, not by comparing paths with the workspace's
    /// `members` list: Cargo also counts a path dependency inside the
    /// workspace root as a member, and expands globs.
    #[must_use]
    pub const fn is_workspace_member(&self) -> bool {
        self.is_workspace_member
    }

    /// The packages, by name, that rely on the imported package in some way
    /// this key's dependency line is not.
    ///
    /// Read from what each package declares, never from the resolved graph,
    /// which holds only the edges the active features reach: a dependency of
    /// any kind, on any target, optional or not, from a workspace member or
    /// from a crate outside the workspace, counts. With a
    /// [`TaskImport::directory`], that is every declared `path` under it and
    /// every build target whose source file lies under it; without one, every
    /// declared dependency on a package of the same name. The composed CLI
    /// counts too when it declares the package again, as a dev- or
    /// build-dependency, which taking this key's dependency out does not
    /// remove. A package that lies under the directory itself is not
    /// counted, since it goes with it. Empty when nothing else relies on it.
    ///
    /// Only the packages `cargo metadata` loaded are read here; see
    /// [`Metadata::dependents_outside_the_graph`] for the rest.
    #[must_use]
    pub fn other_dependents(&self) -> &[&'a str] {
        &self.other_dependents
    }

    /// The other workspace members, by name, whose manifests lie under
    /// [`TaskImport::directory`]. Empty when there is no directory.
    #[must_use]
    pub fn members_inside(&self) -> &[&'a str] {
        &self.members_inside
    }
}

/// Reads every key in `package_name`'s `[package.metadata.ritual] tasks`
/// list, in manifest order, and answers what each one imports.
///
/// A pure read, like resolving the list, but it checks less: a key need not
/// be a usable name, nor name a task crate, because a person taking a
/// broken key out is exactly who needs to see it.
///
/// # Errors
///
/// Returns a [`Failure`] naming the manifest when the project cannot be
/// located or the list is absent or not a list of strings, and one naming
/// the key when `cargo metadata`'s report of where its package lives
/// disagrees with where its dependency says it is.
pub(crate) fn read<'a>(
    metadata: &'a Metadata,
    package_name: &str,
) -> Result<Vec<TaskImport<'a>>, Failure> {
    let project = project::locate(metadata, package_name)?;
    let keys = task_list::declared_keys(&project, package_name)?;

    keys.into_iter()
        .map(|key| import_under(metadata, project.package, key))
        .collect()
}

/// Answers what `key` imports from `cli_package`.
fn import_under<'a>(
    metadata: &'a Metadata,
    cli_package: &'a Package,
    key: &'a str,
) -> Result<TaskImport<'a>, Failure> {
    let Some(dependency) = declared_dependency(cli_package, key) else {
        return Ok(TaskImport::unresolved(key, None));
    };
    let line = Some(DependencyLine {
        key: dependency_key(dependency),
        package_name: &dependency.name,
    });

    let Some(edge) = resolved_edge(metadata, cli_package, key) else {
        return Ok(TaskImport::unresolved(key, line));
    };
    let Some(resolved) = metadata
        .packages
        .iter()
        .find(|package| package.id == edge.pkg)
    else {
        return Ok(TaskImport::unresolved(key, line));
    };

    let directory = dependency.path.as_deref();
    if let Some(directory) = directory {
        ensure_manifest_is_in(key, resolved, directory)?;
    }

    Ok(TaskImport {
        key,
        dependency: line,
        directory,
        is_workspace_member: metadata.workspace_members.contains(&resolved.id),
        other_dependents: other_dependents(metadata, cli_package, key, resolved, directory),
        members_inside: directory
            .map(|directory| members_inside(metadata, resolved, directory))
            .unwrap_or_default(),
    })
}

/// The composed CLI's normal dependency that `key` names, matched by the
/// key it is written under, read the way Rust reads it. One declared on
/// every target is preferred over one under a `cfg(...)` predicate.
fn declared_dependency<'a>(cli_package: &'a Package, key: &str) -> Option<&'a Dependency> {
    cli_package
        .dependencies
        .iter()
        .filter(|dependency| dependency.kind.is_none() && is_written_under(dependency, key))
        .min_by_key(|dependency| dependency.target.is_some())
}

/// The key `dependency` is written under: its `package = "…"` rename or,
/// without one, its own name.
fn dependency_key(dependency: &Dependency) -> &str {
    dependency.rename.as_deref().unwrap_or(&dependency.name)
}

/// Whether `dependency` is written under `key`, or under the other spelling
/// of it, which Rust reads as the same name.
fn is_written_under(dependency: &Dependency, key: &str) -> bool {
    extern_identifier(dependency_key(dependency)) == extern_identifier(key)
}

/// The resolved edge from the composed CLI to the package `key` imports.
///
/// Cargo reports the extern-crate name rustc is given, which is the key
/// with any hyphen turned into an underscore. The edge has to be a normal
/// one: a dev-dependency under the same name is not what the key imports.
fn resolved_edge<'a>(
    metadata: &'a Metadata,
    cli_package: &Package,
    key: &str,
) -> Option<&'a crate::metadata::NodeDependency> {
    let extern_name = extern_identifier(key);
    metadata
        .resolve
        .nodes
        .iter()
        .find(|node| node.id == cli_package.id)?
        .deps
        .iter()
        .find(|edge| {
            edge.name == extern_name && edge.dep_kinds.iter().any(|kind| kind.kind.is_none())
        })
}

/// Refuses when the resolved package's manifest is not `directory`'s
/// `Cargo.toml`.
///
/// Cargo derives the one from the other, so they agree; a disagreement
/// means this is not the directory the key's package lives in, and a
/// caller that deleted it would delete the wrong one.
fn ensure_manifest_is_in(key: &str, resolved: &Package, directory: &Path) -> Result<(), Failure> {
    if resolved.manifest_path == directory.join("Cargo.toml") {
        return Ok(());
    }
    Err(Failure::new(format!(
        "cargo metadata puts `{key}`'s package at {}, but its dependency points at {}; ritual \
         cannot tell which directory `{key}` is",
        resolved.manifest_path.display(),
        directory.display()
    )))
}

/// Every package, by name, that relies on `resolved` in a way that is not
/// `key`'s normal dependency line in the composed CLI, each named once.
///
/// With a `directory`, relying on it is declaring a `path` under it or
/// building a target from a file under it, and a package that lies under it
/// is skipped. Without one, it is declaring a dependency on a package named
/// as `resolved` is.
fn other_dependents<'a>(
    metadata: &'a Metadata,
    cli_package: &Package,
    key: &str,
    resolved: &Package,
    directory: Option<&Path>,
) -> Vec<&'a str> {
    let mut dependents: Vec<&str> = Vec::new();
    for package in &metadata.packages {
        if directory.is_some_and(|directory| lies_under(&package.manifest_path, directory)) {
            continue;
        }
        let declares_it = package.dependencies.iter().any(|dependency| {
            let is_keys_line = package.id == cli_package.id
                && dependency.kind.is_none()
                && is_written_under(dependency, key);
            !is_keys_line
                && directory.map_or_else(
                    || dependency.name == resolved.name,
                    |directory| {
                        dependency
                            .path
                            .as_deref()
                            .is_some_and(|path| lies_under(path, directory))
                    },
                )
        });
        let builds_from_it = directory.is_some_and(|directory| {
            package
                .targets
                .iter()
                .any(|target| lies_under(&target.src_path, directory))
        });
        if (declares_it || builds_from_it) && !dependents.contains(&package.name.as_str()) {
            dependents.push(package.name.as_str());
        }
    }
    dependents
}

/// Every package `metadata` did not load that declares a path dependency
/// under `directory`, by name: what [`Metadata::dependents_outside_the_graph`]
/// answers.
pub(crate) fn dependents_outside_the_graph(
    metadata: &Metadata,
    directory: &Path,
) -> Result<Vec<String>, Failure> {
    let mut asked: Vec<PathBuf> = metadata
        .packages
        .iter()
        .map(|package| crate::paths::normalize(&package.manifest_path))
        .collect();
    let mut pending: Vec<PathBuf> = Vec::new();
    for package in &metadata.packages {
        if !lies_under(&package.manifest_path, directory) {
            pending.extend(unasked_path_crates(package, directory, &asked));
        }
    }

    let mut dependents: Vec<String> = Vec::new();
    while let Some(manifest_path) = pending.pop() {
        if asked.contains(&manifest_path) {
            continue;
        }
        asked.push(manifest_path.clone());
        let package = metadata::fetch_declared(&manifest_path)?;
        let declares_it = package.dependencies.iter().any(|dependency| {
            dependency
                .path
                .as_deref()
                .is_some_and(|path| lies_under(path, directory))
        });
        if declares_it && !dependents.contains(&package.name) {
            dependents.push(package.name.clone());
        }
        pending.extend(unasked_path_crates(&package, directory, &asked));
    }
    Ok(dependents)
}

/// The manifests of `package`'s declared path dependencies that are neither
/// under `directory` nor among `asked`, normalised.
fn unasked_path_crates(package: &Package, directory: &Path, asked: &[PathBuf]) -> Vec<PathBuf> {
    package
        .dependencies
        .iter()
        .filter_map(|dependency| dependency.path.as_deref())
        .filter(|path| !lies_under(path, directory))
        .map(|path| crate::paths::normalize(&path.join("Cargo.toml")))
        .filter(|manifest_path| !asked.contains(manifest_path))
        .collect()
}

/// The workspace members other than `resolved`, by name, whose manifests lie
/// under `directory`.
fn members_inside<'a>(
    metadata: &'a Metadata,
    resolved: &Package,
    directory: &Path,
) -> Vec<&'a str> {
    metadata
        .packages
        .iter()
        .filter(|package| {
            package.id != resolved.id
                && metadata.workspace_members.contains(&package.id)
                && lies_under(&package.manifest_path, directory)
        })
        .map(|package| package.name.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{TaskImport, read};
    use crate::metadata::{Dependency, Metadata, parse};
    use crate::test_support::TestOutcome;

    const DEMO_WORKSPACE: &str = include_str!("metadata/fixtures/demo-workspace.json");

    const TASK_TRUE_ID: &str = "path+file:///scrubbed/checkout/task-true#0.1.0";
    const TASK_NULL_ID: &str = "path+file:///scrubbed/checkout/task-null#0.1.0";

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[test]
    fn a_task_import_is_send_and_sync() {
        assert_send::<TaskImport<'static>>();
        assert_sync::<TaskImport<'static>>();
    }

    /// Replaces the fixture's `tasks` list for `demo-ritual`.
    fn list_tasks(metadata: &mut Metadata, keys: &[&str]) {
        let cli = metadata
            .packages
            .iter_mut()
            .find(|package| package.name == "demo-ritual")
            .expect("the fixture has a demo-ritual package");
        cli.metadata = serde_json::json!({ "ritual": { "tasks": keys } });
    }

    #[test]
    fn task_imports_lists_every_key_in_manifest_order() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let imports = read(&metadata, "demo-ritual")?;

        let keys: Vec<&str> = imports.iter().map(TaskImport::key).collect();
        assert_eq!(keys, ["task-true", "renamed", "move"]);
        let task_true = &imports[0];
        assert_eq!(task_true.package_name(), Some("task-true"));
        assert_eq!(
            task_true.directory(),
            Some(Path::new("/scrubbed/checkout/task-true"))
        );
        assert!(task_true.is_workspace_member());
        assert!(task_true.other_dependents().is_empty());
        assert!(task_true.members_inside().is_empty());
        Ok(())
    }

    #[test]
    fn a_renamed_key_reports_the_package_it_imports() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let imports = read(&metadata, "demo-ritual")?;

        // `renamed` is `task-renamed-source` under a Cargo rename, and
        // `move` is `task-keyword` under a key that is a Rust keyword.
        assert_eq!(imports[1].package_name(), Some("task-renamed-source"));
        assert_eq!(
            imports[1].directory(),
            Some(Path::new("/scrubbed/checkout/task-renamed-source"))
        );
        assert_eq!(imports[2].package_name(), Some("task-keyword"));
        Ok(())
    }

    #[test]
    fn a_path_dependency_outside_the_members_is_not_a_member() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        metadata.workspace_members.retain(|id| id != TASK_TRUE_ID);

        let imports = read(&metadata, "demo-ritual")?;

        assert!(!imports[0].is_workspace_member());
        assert_eq!(
            imports[0].directory(),
            Some(Path::new("/scrubbed/checkout/task-true")),
            "a path dependency still has a directory, member or not"
        );
        Ok(())
    }

    #[test]
    fn a_key_with_no_dependency_has_no_package() -> TestOutcome {
        // `nothing` has no dependency at all; `task-dev-only` has one, but
        // only as a dev-dependency, which a task is never mounted from.
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        list_tasks(&mut metadata, &["nothing", "task-dev-only"]);

        let imports = read(&metadata, "demo-ritual")?;

        for import in &imports {
            assert_eq!(import.package_name(), None, "key `{}`", import.key());
            assert_eq!(import.directory(), None, "key `{}`", import.key());
            assert!(!import.is_workspace_member(), "key `{}`", import.key());
            assert!(
                import.other_dependents().is_empty(),
                "key `{}`",
                import.key()
            );
            assert!(import.members_inside().is_empty(), "key `{}`", import.key());
        }
        Ok(())
    }

    #[test]
    fn a_git_dependency_has_no_directory() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        metadata.workspace_members.retain(|id| id != TASK_TRUE_ID);
        let cli = metadata
            .packages
            .iter_mut()
            .find(|package| package.name == "demo-ritual")
            .expect("the fixture has a demo-ritual package");
        for dependency in &mut cli.dependencies {
            if dependency.name == "task-true" {
                dependency.path = None;
            }
        }

        let imports = read(&metadata, "demo-ritual")?;

        assert_eq!(imports[0].package_name(), Some("task-true"));
        assert_eq!(imports[0].directory(), None);
        assert!(!imports[0].is_workspace_member());
        assert!(imports[0].members_inside().is_empty());
        Ok(())
    }

    /// Adds `dependency` to what the package called `package_name` declares.
    fn declare(metadata: &mut Metadata, package_name: &str, dependency: Dependency) {
        metadata
            .packages
            .iter_mut()
            .find(|package| package.name == package_name)
            .expect("the fixture has the package")
            .dependencies
            .push(dependency);
    }

    /// A normal path dependency on `task-true`'s directory, under `name`.
    fn on_task_true(name: &str) -> Dependency {
        Dependency {
            name: name.to_string(),
            kind: None,
            rename: None,
            path: Some(PathBuf::from("/scrubbed/checkout/task-true")),
            target: None,
        }
    }

    #[test]
    fn another_package_declaring_the_directory_is_an_other_dependent() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        declare(&mut metadata, "task-null", on_task_true("task-true"));

        let imports = read(&metadata, "demo-ritual")?;

        assert_eq!(imports[0].other_dependents(), ["task-null"]);
        assert!(
            imports[1].other_dependents().is_empty(),
            "a dependency on task-true says nothing about the other keys"
        );
        Ok(())
    }

    /// An optional dependency that no feature turns on has no edge in the
    /// resolved graph, so only what the package declares shows it. The
    /// fixture's graph has no edge from `task-null` to `task-true` at all.
    #[test]
    fn a_declared_dependency_with_no_resolved_edge_is_an_other_dependent() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        assert!(
            metadata
                .resolve
                .nodes
                .iter()
                .filter(|node| node.id == TASK_NULL_ID)
                .all(|node| node.deps.iter().all(|edge| edge.pkg != TASK_TRUE_ID)),
            "fixture precondition: no resolved edge from task-null to task-true"
        );
        declare(
            &mut metadata,
            "task-null",
            Dependency {
                kind: Some("build".to_string()),
                target: Some("cfg(unix)".to_string()),
                ..on_task_true("task-true")
            },
        );

        let imports = read(&metadata, "demo-ritual")?;

        assert_eq!(imports[0].other_dependents(), ["task-null"]);
        Ok(())
    }

    /// Cargo reads a path dependency's manifest whoever declares it, so a
    /// package outside the workspace counts as much as a member.
    #[test]
    fn a_package_outside_the_workspace_declaring_it_is_an_other_dependent() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        metadata.workspace_members.retain(|id| id != TASK_NULL_ID);
        declare(&mut metadata, "task-null", on_task_true("anything"));

        let imports = read(&metadata, "demo-ritual")?;

        assert_eq!(imports[0].other_dependents(), ["task-null"]);
        Ok(())
    }

    /// A dependency on a crate nested inside the directory is a dependency on
    /// the directory: deleting it takes the nested crate too.
    #[test]
    fn a_dependency_on_a_crate_inside_the_directory_is_an_other_dependent() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        declare(
            &mut metadata,
            "task-null",
            Dependency {
                path: Some(PathBuf::from("/scrubbed/checkout/task-true/./inner")),
                ..on_task_true("inner")
            },
        );

        let imports = read(&metadata, "demo-ritual")?;

        assert_eq!(imports[0].other_dependents(), ["task-null"]);
        Ok(())
    }

    #[test]
    fn a_package_building_a_target_from_a_file_in_the_directory_is_an_other_dependent()
    -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        metadata
            .packages
            .iter_mut()
            .find(|package| package.id == TASK_NULL_ID)
            .expect("the fixture has a task-null package")
            .targets[0]
            .src_path = PathBuf::from("/scrubbed/checkout/task-true/src/lib.rs");

        let imports = read(&metadata, "demo-ritual")?;

        assert_eq!(imports[0].other_dependents(), ["task-null"]);
        Ok(())
    }

    /// A package inside the directory goes with it, so its dependency on the
    /// directory breaks nothing.
    #[test]
    fn a_package_inside_the_directory_is_not_an_other_dependent() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        let inner = metadata
            .packages
            .iter_mut()
            .find(|package| package.id == TASK_NULL_ID)
            .expect("the fixture has a task-null package");
        inner.manifest_path = PathBuf::from("/scrubbed/checkout/task-true/inner/Cargo.toml");
        inner.dependencies.push(on_task_true("task-true"));

        let imports = read(&metadata, "demo-ritual")?;

        assert!(imports[0].other_dependents().is_empty());
        Ok(())
    }

    #[test]
    fn a_dev_dependency_on_the_same_crate_makes_the_cli_an_other_dependent() -> TestOutcome {
        // Taking out the normal dependency line leaves a dev-dependency
        // under the same key, which deleting the directory would break.
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        declare(
            &mut metadata,
            "demo-ritual",
            Dependency {
                kind: Some("dev".to_string()),
                ..on_task_true("task-true")
            },
        );

        let imports = read(&metadata, "demo-ritual")?;

        assert_eq!(imports[0].other_dependents(), ["demo-ritual"]);
        Ok(())
    }

    /// The composed CLI's own normal line under the key, on every target it
    /// is declared on, is what `remove` takes out, so it is not counted.
    #[test]
    fn the_keys_own_lines_on_every_target_are_not_other_dependents() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        declare(
            &mut metadata,
            "demo-ritual",
            Dependency {
                target: Some("cfg(unix)".to_string()),
                ..on_task_true("task-true")
            },
        );

        let imports = read(&metadata, "demo-ritual")?;

        assert!(imports[0].other_dependents().is_empty());
        Ok(())
    }

    #[test]
    fn a_member_nested_inside_the_directory_is_reported() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        metadata
            .packages
            .iter_mut()
            .find(|package| package.id == TASK_NULL_ID)
            .expect("the fixture has a task-null package")
            .manifest_path = PathBuf::from("/scrubbed/checkout/task-true/inner/Cargo.toml");

        let imports = read(&metadata, "demo-ritual")?;

        assert_eq!(imports[0].members_inside(), ["task-null"]);
        Ok(())
    }

    #[test]
    fn a_member_in_a_sibling_directory_with_the_same_prefix_is_not_inside() -> TestOutcome {
        // `task-true-sibling` starts with `task-true` as text, but is not
        // under it as a path.
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        metadata
            .packages
            .iter_mut()
            .find(|package| package.id == TASK_NULL_ID)
            .expect("the fixture has a task-null package")
            .manifest_path = PathBuf::from("/scrubbed/checkout/task-true-sibling/Cargo.toml");

        let imports = read(&metadata, "demo-ritual")?;

        assert!(imports[0].members_inside().is_empty());
        Ok(())
    }

    #[test]
    fn a_manifest_that_is_not_where_the_dependency_points_is_refused() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        metadata
            .packages
            .iter_mut()
            .find(|package| package.id == TASK_TRUE_ID)
            .expect("the fixture has a task-true package")
            .manifest_path = PathBuf::from("/scrubbed/checkout/elsewhere/Cargo.toml");

        let failure = read(&metadata, "demo-ritual")
            .err()
            .ok_or("a disagreement about where the package lives was meant to be refused")?;

        assert_eq!(
            failure.to_string(),
            "cargo metadata puts `task-true`'s package at \
             /scrubbed/checkout/elsewhere/Cargo.toml, but its dependency points at \
             /scrubbed/checkout/task-true; ritual cannot tell which directory `task-true` is"
        );
        Ok(())
    }

    #[test]
    fn a_dependency_on_every_target_is_preferred_over_one_under_a_predicate() -> TestOutcome {
        // The same key declared twice: once under a `cfg(...)` predicate
        // pointing somewhere else, listed first, and once on every target.
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        let cli = metadata
            .packages
            .iter_mut()
            .find(|package| package.name == "demo-ritual")
            .expect("the fixture has a demo-ritual package");
        let conditional = cli
            .dependencies
            .iter()
            .find(|dependency| dependency.name == "task-true")
            .map(|dependency| crate::metadata::Dependency {
                name: dependency.name.clone(),
                kind: None,
                rename: None,
                path: Some(PathBuf::from("/scrubbed/checkout/not-this-one")),
                target: Some("cfg(unix)".to_string()),
            })
            .expect("the fixture declares task-true");
        cli.dependencies.insert(0, conditional);

        let imports = read(&metadata, "demo-ritual")?;

        assert_eq!(
            imports[0].directory(),
            Some(Path::new("/scrubbed/checkout/task-true"))
        );
        Ok(())
    }

    /// The fixture's dependency is written `task-true`; listed as
    /// `task_true`, the key reaches it the way the resolver does, and says
    /// the spelling the dependency line is written under.
    #[test]
    fn a_key_reaches_a_dependency_written_with_its_other_spelling() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        list_tasks(&mut metadata, &["task_true"]);

        let imports = read(&metadata, "demo-ritual")?;

        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].key(), "task_true");
        assert_eq!(imports[0].dependency_key(), Some("task-true"));
        assert_eq!(imports[0].package_name(), Some("task-true"));
        assert_eq!(
            imports[0].directory(),
            Some(Path::new("/scrubbed/checkout/task-true"))
        );
        Ok(())
    }

    #[test]
    fn a_tasks_list_that_is_absent_or_of_the_wrong_shape_is_refused() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        let cli = metadata
            .packages
            .iter_mut()
            .find(|package| package.name == "demo-ritual")
            .expect("the fixture has a demo-ritual package");

        cli.metadata = serde_json::Value::Null;
        let absent = read(&metadata, "demo-ritual")
            .err()
            .ok_or("an absent list was meant to be refused")?;
        assert!(absent.to_string().contains("tasks = []"), "was: {absent}");

        let cli = metadata
            .packages
            .iter_mut()
            .find(|package| package.name == "demo-ritual")
            .expect("the fixture has a demo-ritual package");
        cli.metadata = serde_json::json!({ "ritual": { "tasks": [1] } });
        let wrong = read(&metadata, "demo-ritual")
            .err()
            .ok_or("a list of the wrong shape was meant to be refused")?;
        assert!(
            wrong.to_string().contains("is not a list of strings"),
            "was: {wrong}"
        );
        Ok(())
    }

    #[test]
    fn a_package_that_is_not_in_the_workspace_is_refused() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let failure = read(&metadata, "nonexistent-package")
            .err()
            .ok_or("an unknown package was meant to be refused")?;

        assert!(failure.to_string().contains("nonexistent-package"));
        Ok(())
    }
}
