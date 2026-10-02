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

use std::path::Path;

use rituals::Failure;

use crate::metadata::{Dependency, Metadata, Package};
use crate::{project, task_list};

/// One key in a composed CLI's `[package.metadata.ritual] tasks` list, and
/// what it imports.
///
/// Every question about the dependency behind the key is answered from the
/// resolved graph `cargo metadata` reports: which package the key reaches,
/// where that package lives, whether it is a member of the workspace, and
/// what else in the workspace relies on it. A key with no normal dependency
/// behind it answers every one of them with nothing.
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
    package_name: Option<&'a str>,
    directory: Option<&'a Path>,
    is_workspace_member: bool,
    other_dependents: Vec<&'a str>,
    members_inside: Vec<&'a str>,
}

impl<'a> TaskImport<'a> {
    /// A key whose dependency, if it has one, resolves to nothing: no
    /// package, no directory, not a member, nothing depending on it.
    const fn unresolved(key: &'a str, package_name: Option<&'a str>) -> Self {
        Self {
            key,
            package_name,
            directory: None,
            is_workspace_member: false,
            other_dependents: Vec::new(),
            members_inside: Vec::new(),
        }
    }

    /// The key as `[package.metadata.ritual] tasks` spells it, which is also
    /// the dependency key in the manifest.
    #[must_use]
    pub const fn key(&self) -> &'a str {
        self.key
    }

    /// The name of the package the key imports, or `None` when no normal
    /// dependency of the composed CLI has this key.
    ///
    /// A dev- or build-dependency under the key does not count: a task is
    /// mounted from a normal dependency.
    #[must_use]
    pub const fn package_name(&self) -> Option<&'a str> {
        self.package_name
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

    /// The workspace packages, by name, that depend on the imported package
    /// in some way this key's dependency line is not.
    ///
    /// That is every other workspace package with an edge to it, and the
    /// composed CLI itself when it also depends on the package as a dev- or
    /// build-dependency, which taking this key's dependency out does not
    /// remove. Empty when nothing else relies on it.
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
    let package_name = Some(dependency.name.as_str());

    let Some(edge) = resolved_edge(metadata, cli_package, key) else {
        return Ok(TaskImport::unresolved(key, package_name));
    };
    let Some(resolved) = metadata
        .packages
        .iter()
        .find(|package| package.id == edge.pkg)
    else {
        return Ok(TaskImport::unresolved(key, package_name));
    };

    let directory = dependency.path.as_deref();
    if let Some(directory) = directory {
        ensure_manifest_is_in(key, resolved, directory)?;
    }

    Ok(TaskImport {
        key,
        package_name,
        directory,
        is_workspace_member: metadata.workspace_members.contains(&resolved.id),
        other_dependents: other_dependents(metadata, cli_package, key, resolved),
        members_inside: directory
            .map(|directory| members_inside(metadata, resolved, directory))
            .unwrap_or_default(),
    })
}

/// The composed CLI's normal dependency that `key` names, matched by its
/// `package = "…"` rename or, without one, its own name. One declared on
/// every target is preferred over one under a `cfg(...)` predicate.
fn declared_dependency<'a>(cli_package: &'a Package, key: &str) -> Option<&'a Dependency> {
    cli_package
        .dependencies
        .iter()
        .filter(|dependency| {
            dependency.kind.is_none()
                && dependency.rename.as_deref().unwrap_or(&dependency.name) == key
        })
        .min_by_key(|dependency| dependency.target.is_some())
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
    let extern_name = key.replace('-', "_");
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

/// Every workspace package, by name, that depends on `resolved` in a way
/// that is not `key`'s normal dependency line in the composed CLI, each
/// named once.
fn other_dependents<'a>(
    metadata: &'a Metadata,
    cli_package: &Package,
    key: &str,
    resolved: &Package,
) -> Vec<&'a str> {
    let extern_name = key.replace('-', "_");
    let mut dependents: Vec<&str> = Vec::new();
    for node in &metadata.resolve.nodes {
        if !metadata.workspace_members.contains(&node.id) {
            continue;
        }
        let depends_otherwise =
            node.deps
                .iter()
                .filter(|edge| edge.pkg == resolved.id)
                .any(|edge| {
                    let is_keys_edge = node.id == cli_package.id && edge.name == extern_name;
                    if is_keys_edge {
                        // The same edge also carries a dev- or build-dependency
                        // when one is declared under the same name; that is not
                        // removed with the normal one.
                        edge.dep_kinds.iter().any(|kind| kind.kind.is_some())
                    } else {
                        true
                    }
                });
        if !depends_otherwise {
            continue;
        }
        let Some(dependent) = metadata
            .packages
            .iter()
            .find(|package| package.id == node.id)
        else {
            continue;
        };
        if !dependents.contains(&dependent.name.as_str()) {
            dependents.push(dependent.name.as_str());
        }
    }
    dependents
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
                && package.manifest_path.starts_with(directory)
        })
        .map(|package| package.name.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{TaskImport, read};
    use crate::metadata::{DepKind, Metadata, NodeDependency, parse};
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

    /// The fixture's own resolve edge from `demo-ritual` to `task-true`.
    fn the_cli_edge_to_task_true(metadata: &mut Metadata) -> &mut NodeDependency {
        metadata
            .resolve
            .nodes
            .iter_mut()
            .find(|node| node.id.contains("cli#demo-ritual"))
            .expect("the fixture has a demo-ritual resolve node")
            .deps
            .iter_mut()
            .find(|edge| edge.name == "task_true")
            .expect("the fixture has a task_true edge")
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

    #[test]
    fn another_packages_edge_to_the_same_crate_is_an_other_dependent() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        let node = metadata
            .resolve
            .nodes
            .iter_mut()
            .find(|node| node.id == TASK_NULL_ID)
            .expect("the fixture has a task-null resolve node");
        node.deps.push(NodeDependency {
            name: "task_true".to_string(),
            pkg: TASK_TRUE_ID.to_string(),
            dep_kinds: vec![DepKind {
                kind: None,
                target: None,
            }],
        });

        let imports = read(&metadata, "demo-ritual")?;

        assert_eq!(imports[0].other_dependents(), ["task-null"]);
        assert!(
            imports[1].other_dependents().is_empty(),
            "an edge to task-true says nothing about the other keys"
        );
        Ok(())
    }

    #[test]
    fn an_edge_from_a_package_outside_the_workspace_is_not_an_other_dependent() -> TestOutcome {
        // A dependency pulled in from outside has no say in what a
        // workspace member's directory may lose.
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        metadata.workspace_members.retain(|id| id != TASK_NULL_ID);
        let node = metadata
            .resolve
            .nodes
            .iter_mut()
            .find(|node| node.id == TASK_NULL_ID)
            .expect("the fixture has a task-null resolve node");
        node.deps.push(NodeDependency {
            name: "task_true".to_string(),
            pkg: TASK_TRUE_ID.to_string(),
            dep_kinds: vec![DepKind {
                kind: None,
                target: None,
            }],
        });

        let imports = read(&metadata, "demo-ritual")?;

        assert!(imports[0].other_dependents().is_empty());
        Ok(())
    }

    #[test]
    fn a_dev_dependency_on_the_same_crate_makes_the_cli_an_other_dependent() -> TestOutcome {
        // Taking out the normal dependency line leaves a dev-dependency
        // under the same key, which deleting the directory would break.
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        the_cli_edge_to_task_true(&mut metadata)
            .dep_kinds
            .push(DepKind {
                kind: Some("dev".to_string()),
                target: None,
            });

        let imports = read(&metadata, "demo-ritual")?;

        assert_eq!(imports[0].other_dependents(), ["demo-ritual"]);
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
