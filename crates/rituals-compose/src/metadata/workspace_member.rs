//! A workspace member as a task asks about it.

use std::path::Path;

use super::Package;
use crate::task_list::{TaskDeclaration, declaration};

/// A package that is a member of the workspace a [`Metadata`] describes.
///
/// Declared here and re-exported at [`crate::metadata::WorkspaceMember`],
/// where a caller names it: only ever built by
/// [`Metadata::workspace_members`].
///
/// [`Metadata`]: crate::metadata::Metadata
/// [`Metadata::workspace_members`]: crate::metadata::Metadata::workspace_members
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
/// let tasks: Vec<_> = document
///     .workspace_members()
///     .into_iter()
///     .filter(|member| member.declares_a_task_crate())
///     .map(|member| member.directory())
///     .collect();
/// println!("tasks live in {tasks:?}");
/// # Ok::<(), rituals::Failure>(())
/// ```
#[derive(Debug, Clone, Copy)]
pub struct WorkspaceMember<'a> {
    pub(super) package: &'a Package,
}

impl<'a> WorkspaceMember<'a> {
    /// The package's name, as its manifest gives it.
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
    /// for member in document.workspace_members() {
    ///     println!("a member called {}", member.package_name());
    /// }
    /// # Ok::<(), rituals::Failure>(())
    /// ```
    #[must_use]
    pub fn package_name(&self) -> &'a str {
        &self.package.name
    }

    /// The package's own `Cargo.toml`, absolute.
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
    /// for member in document.workspace_members() {
    ///     println!("{} is read from {}", member.package_name(), member.manifest_path().display());
    /// }
    /// # Ok::<(), rituals::Failure>(())
    /// ```
    #[must_use]
    pub fn manifest_path(&self) -> &'a Path {
        &self.package.manifest_path
    }

    /// The directory holding the package's manifest, absolute.
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
    /// for member in document.workspace_members() {
    ///     println!("{} lives in {}", member.package_name(), member.directory().display());
    /// }
    /// # Ok::<(), rituals::Failure>(())
    /// ```
    #[must_use]
    pub fn directory(&self) -> &'a Path {
        self.package.manifest_path.parent().unwrap_or_else(|| {
            unreachable!(
                "cargo metadata reports {} as the path of a manifest file, which has a \
                     directory",
                self.package.manifest_path.display()
            )
        })
    }

    /// Whether the package declares `[package.metadata.ritual] task = true`,
    /// which is what makes a crate a task.
    ///
    /// The one rule for that is the one the task list is resolved with, so a
    /// crate is a task here exactly when a task list naming it would accept
    /// it: a missing table, a missing key, `task = false` and a key that is
    /// not a boolean are all `false`.
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
    /// let tasks = document
    ///     .workspace_members()
    ///     .into_iter()
    ///     .filter(|member| member.declares_a_task_crate())
    ///     .count();
    /// println!("{tasks} members are task crates");
    /// # Ok::<(), rituals::Failure>(())
    /// ```
    #[must_use]
    pub fn declares_a_task_crate(&self) -> bool {
        match declaration(self.package) {
            TaskDeclaration::Task => true,
            TaskDeclaration::NotATask | TaskDeclaration::NotABoolean => false,
        }
    }
}
