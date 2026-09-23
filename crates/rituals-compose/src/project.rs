//! Locating a composed CLI crate inside a parsed `cargo metadata` document.
//!
//! Reached through [`crate::metadata::Metadata::locate_project`] — this
//! module holds the behaviour, that method is the entry point.
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

use rituals::{Failure, Name};

use crate::metadata::{Metadata, Package};

/// Everything the framework needs to know about the composed CLI crate it
/// is running as part of: where the workspace is, which package it is, and
/// where its one generated file lives.
///
/// Declared here and re-exported at [`crate::metadata::Project`], where a
/// caller names it — reached only through
/// [`crate::metadata::Metadata::locate_project`].
#[derive(Debug)]
pub struct Project<'a> {
    pub(crate) workspace_root: &'a Path,
    pub(crate) package: &'a Package,
    pub(crate) binary_src_path: &'a Path,
}

impl Project<'_> {
    /// The root of the workspace this project's composed CLI crate is a
    /// member of.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::metadata;
    ///
    /// // `Project` is only ever built by `Metadata::locate_project`, which
    /// // needs a real `cargo metadata` call, so this example stays
    /// // `no_run`.
    /// let document = metadata::fetch(Path::new("."))?;
    /// let project = document.locate_project("demo-ritual")?;
    /// let task_dir = project.workspace_root().join("tasks/lint");
    /// println!("a new task would go in {}", task_dir.display());
    /// # Ok::<(), rituals::Failure>(())
    /// ```
    #[must_use]
    pub const fn workspace_root(&self) -> &Path {
        self.workspace_root
    }

    /// The composed CLI crate's own `Cargo.toml` — not the workspace's.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals_compose::manifest::Manifest;
    /// use rituals_compose::metadata;
    ///
    /// // `Project` is only ever built by `Metadata::locate_project`, which
    /// // needs a real `cargo metadata` call, so this example stays
    /// // `no_run`.
    /// let document = metadata::fetch(Path::new("."))?;
    /// let project = document.locate_project("demo-ritual")?;
    /// let manifest = Manifest::read(project.manifest_path())?;
    /// println!("editing {}", manifest.path().display());
    /// # Ok::<(), rituals::Failure>(())
    /// ```
    #[must_use]
    pub fn manifest_path(&self) -> &Path {
        &self.package.manifest_path
    }

    /// Reports whether this project's manifest already declares a
    /// dependency under `key` — matched by name or, when the dependency is
    /// renamed, by its `package = "…"` rename, since either one occupies
    /// the key a new import would need.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals::Name;
    /// use rituals_compose::metadata;
    ///
    /// // `Project` is only ever built by `Metadata::locate_project`, which
    /// // needs a real `cargo metadata` call, so this example stays
    /// // `no_run`.
    /// let document = metadata::fetch(Path::new("."))?;
    /// let project = document.locate_project("demo-ritual")?;
    /// let key = Name::new("lint")?;
    /// if project.declares_dependency_key(&key) {
    ///     println!("`lint` already names a dependency");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn declares_dependency_key(&self, key: &Name) -> bool {
        self.package.dependencies.iter().any(|dependency| {
            dependency.rename.as_deref().unwrap_or(&dependency.name) == key.as_str()
        })
    }

    /// Reports whether this project's `[package.metadata.ritual] tasks`
    /// list already names `name`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    ///
    /// use rituals::Name;
    /// use rituals_compose::metadata;
    ///
    /// // `Project` is only ever built by `Metadata::locate_project`, which
    /// // needs a real `cargo metadata` call, so this example stays
    /// // `no_run`.
    /// let document = metadata::fetch(Path::new("."))?;
    /// let project = document.locate_project("demo-ritual")?;
    /// let name = Name::new("lint")?;
    /// if project.lists_task(&name) {
    ///     println!("`lint` is already in [package.metadata.ritual] tasks");
    /// }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn lists_task(&self, name: &Name) -> bool {
        self.package
            .metadata
            .get("ritual")
            .and_then(|ritual| ritual.get("tasks"))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|tasks| {
                tasks
                    .iter()
                    .any(|task| task.as_str().is_some_and(|task| task == name.as_str()))
            })
    }
}

/// Finds the workspace member named `package_name` in `metadata`, and its
/// single binary target.
///
/// `package_name` normally arrives through the identity a task asked to
/// receive — `Identity::package_name()` — rather than being read directly:
/// `cargo metadata` walks up from the current directory to find the
/// workspace root, and this function then identifies the caller among the
/// workspace's members by name, so `add` and `regenerate` work from any
/// subdirectory of a project.
///
/// # Errors
///
/// Returns a [`Failure`] naming `package_name` and the workspace root when
/// no workspace member has that name, and one naming `package_name` when
/// its manifest declares no `[[bin]]` target or more than one.
pub(crate) fn locate<'a>(
    metadata: &'a Metadata,
    package_name: &str,
) -> Result<Project<'a>, Failure> {
    let package = metadata.packages.iter().find(|package| {
        package.name == package_name && metadata.workspace_members.contains(&package.id)
    });

    let Some(package) = package else {
        return Err(Failure::new(format!(
            "`{package_name}` is not a member of the workspace at {}",
            metadata.workspace_root.display()
        )));
    };

    let binary_src_path = single_binary_target(package_name, package)?;

    Ok(Project {
        workspace_root: &metadata.workspace_root,
        package,
        binary_src_path,
    })
}

/// Returns the `src_path` of `package`'s one `[[bin]]` target, or a
/// [`Failure`] naming `package_name` when there is none or more than one.
fn single_binary_target<'a>(package_name: &str, package: &'a Package) -> Result<&'a Path, Failure> {
    let binaries: Vec<&Path> = package
        .targets
        .iter()
        .filter(|target| target.kind.iter().any(|kind| kind == "bin"))
        .map(|target| target.src_path.as_path())
        .collect();

    match binaries.as_slice() {
        [] => Err(Failure::new(format!(
            "`{package_name}` has no [[bin]] target to write"
        ))),
        [only] => Ok(only),
        _ => Err(Failure::new(format!(
            "`{package_name}` has more than one [[bin]] target; ritual writes the one the \
             command line is generated into"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rituals::Name;

    use super::{Project, locate};
    use crate::metadata::{Dependency, Package, parse};
    use crate::test_support::TestOutcome;

    const DEMO_WORKSPACE: &str = include_str!("metadata/fixtures/demo-workspace.json");

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[test]
    fn project_is_send_and_sync() {
        assert_send::<Project<'static>>();
        assert_sync::<Project<'static>>();
    }

    /// A `Name` from a literal already known to be valid.
    fn valid_name(value: &str) -> Name {
        Name::new(value).expect("a test passes only names it knows are valid")
    }

    /// Builds a [`Package`] with the given dependencies and
    /// `[package.metadata.ritual] tasks` list — enough of the schema for
    /// [`Project::declares_dependency_key`] and [`Project::lists_task`],
    /// neither of which reads anything else on the type.
    fn package_with(dependency_names: &[&str], tasks: &[&str]) -> Package {
        Package {
            id: "demo-ritual 0.1.0".to_string(),
            name: "demo-ritual".to_string(),
            manifest_path: PathBuf::from("/workspace/ritual/Cargo.toml"),
            targets: Vec::new(),
            dependencies: dependency_names
                .iter()
                .map(|name| Dependency {
                    name: (*name).to_string(),
                    kind: None,
                    rename: None,
                })
                .collect(),
            metadata: serde_json::json!({ "ritual": { "tasks": tasks } }),
        }
    }

    /// A [`Project`] over a hand-built [`Package`], for the accessor tests
    /// below that need a metadata shape `locate` cannot produce on its own
    /// (no fixture member is both a composed CLI and shaped exactly one way).
    fn project_over(package: &Package) -> Project<'_> {
        Project {
            workspace_root: std::path::Path::new("/workspace"),
            package,
            binary_src_path: std::path::Path::new("/workspace/ritual/src/main.rs"),
        }
    }

    #[test]
    fn the_composed_cli_package_is_found_by_name() {
        let metadata = parse(DEMO_WORKSPACE.as_bytes());
        assert!(
            metadata.is_ok(),
            "expected the fixture to parse: {metadata:?}"
        );
        if let Ok(metadata) = metadata {
            let project = locate(&metadata, "demo-ritual");
            assert!(
                project.is_ok(),
                "expected demo-ritual to be found: {:?}",
                project.err()
            );
            if let Ok(project) = project {
                assert_eq!(project.package.name, "demo-ritual");
                assert!(project.binary_src_path.ends_with("main.rs"));
            }
        }
    }

    #[test]
    fn a_name_not_in_the_workspace_is_refused_naming_the_package() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let project = locate(&metadata, "nonexistent-package");
        assert!(
            project.is_err(),
            "expected an unknown package name to be refused"
        );
        if let Err(error) = project {
            let message = error.to_string();
            assert!(message.contains("nonexistent-package"));
            assert!(message.contains("is not a member of the workspace at"));
        }
        Ok(())
    }

    #[test]
    fn a_member_with_no_binary_target_is_refused_naming_the_missing_bin() -> TestOutcome {
        // `task-true` is a workspace member in the fixture, but it is a
        // library — a task crate, not a composed CLI — so it has no
        // `[[bin]]` target for `locate` to find.
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let project = locate(&metadata, "task-true");
        assert!(
            project.is_err(),
            "expected a binary-less member to be refused"
        );
        if let Err(error) = project {
            assert!(error.to_string().contains("task-true"));
            assert!(error.to_string().contains("[[bin]]"));
        }
        Ok(())
    }

    #[test]
    fn declares_dependency_key_finds_a_plain_dependency() {
        let package = package_with(&["lint"], &[]);
        let project = project_over(&package);
        assert!(project.declares_dependency_key(&valid_name("lint")));
        assert!(!project.declares_dependency_key(&valid_name("second")));
    }

    /// The fixture's `demo-ritual` package depends on `task-renamed-source`
    /// under the Cargo rename `renamed` — nothing else in this module
    /// covers this match.
    #[test]
    fn declares_dependency_key_finds_a_dependency_reached_through_a_rename() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let project = locate(&metadata, "demo-ritual");
        assert!(
            project.is_ok(),
            "expected demo-ritual to be found: {:?}",
            project.err()
        );
        if let Ok(project) = project {
            assert!(project.declares_dependency_key(&valid_name("renamed")));
            assert!(!project.declares_dependency_key(&valid_name("task-renamed-source")));
        }
        Ok(())
    }

    #[test]
    fn lists_task_finds_a_name_in_the_tasks_list() {
        let package = package_with(&[], &["new", "create"]);
        let project = project_over(&package);
        assert!(project.lists_task(&valid_name("new")));
        assert!(!project.lists_task(&valid_name("lint")));
    }

    #[test]
    fn lists_task_is_false_when_the_ritual_table_is_absent() {
        let package = Package {
            metadata: serde_json::Value::Null,
            ..package_with(&[], &[])
        };
        let project = project_over(&package);
        assert!(!project.lists_task(&valid_name("new")));
    }
}
