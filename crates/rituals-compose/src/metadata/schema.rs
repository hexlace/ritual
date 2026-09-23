//! The subset of `cargo metadata --format-version 1`'s JSON this framework
//! reads.
//!
//! Every field below is named for what `cargo metadata` actually calls it,
//! not for a friendlier synonym, so a reader can hold this file next to
//! Cargo's own documentation and see the same words.
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

use std::path::PathBuf;

use serde::Deserialize;

/// The whole of `cargo metadata --format-version 1`'s output that this
/// framework reads.
///
/// Get one from [`super::fetch`], which runs `cargo metadata` and parses
/// what it prints. Its fields are not public: a caller asks it questions
/// through its methods — [`Metadata::locate_project`],
/// [`Metadata::resolve_task_list`] and [`Metadata::has_workspace_member`] —
/// so the subset of Cargo's schema it reads can change without breaking
/// anyone.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::metadata;
///
/// // Runs a real `cargo metadata` against a workspace on disk, so this
/// // example is `no_run`.
/// let document = metadata::fetch(Path::new("."))?;
/// let entries = document.resolve_task_list("demo-ritual")?;
/// for entry in &entries {
///     println!("{}", entry.command());
/// }
/// # Ok::<(), rituals::Failure>(())
/// ```
#[derive(Debug, Deserialize)]
pub struct Metadata {
    /// The schema version. Checked to be `1` when the output is parsed,
    /// which refuses any other; every other field's meaning is pinned to
    /// that version.
    pub(crate) version: u64,
    pub(crate) workspace_root: PathBuf,
    /// The package ids of every workspace member — as opposed to a
    /// dependency pulled in from outside the workspace.
    pub(crate) workspace_members: Vec<String>,
    pub(crate) packages: Vec<Package>,
    pub(crate) resolve: Resolve,
}

/// One package in the resolved graph — a workspace member or a dependency,
/// at any depth.
#[derive(Debug, Deserialize)]
pub(crate) struct Package {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) manifest_path: PathBuf,
    pub(crate) targets: Vec<Target>,
    /// Read by `add`, to distinguish "not a dependency at all" from "a
    /// dependency, but not listed in `tasks`".
    pub(crate) dependencies: Vec<Dependency>,
    /// `[package.metadata]`, as a raw JSON value rather than a typed struct —
    /// deliberately: one malformed `[package.metadata.ritual]` anywhere in
    /// a project's graph should refuse naming that package, not fail the
    /// whole parse. Absent in the source JSON when a package
    /// declares no `[package.metadata]` table at all.
    #[serde(default)]
    pub(crate) metadata: serde_json::Value,
}

/// One build target of a package — a library, a binary, or anything else
/// Cargo builds from it.
#[derive(Debug, Deserialize)]
pub(crate) struct Target {
    pub(crate) kind: Vec<String>,
    pub(crate) src_path: PathBuf,
}

/// One entry in a package's own `[dependencies]` (or `[dev-dependencies]`,
/// `[build-dependencies]`) table, as that package's manifest declares it —
/// not yet resolved to the package it points at.
///
/// [`crate::metadata::Project::declares_dependency_key`]'s "already a
/// dependency" check matches by `name`-or-`rename` regardless of `kind` — a
/// key already spoken for under any kind cannot be reused — so `kind`
/// itself is read only by this module's own tests, that being a genuine
/// part of what a parsing test over this schema should cover even though no
/// production code branches on it.
#[derive(Debug, Deserialize)]
pub(crate) struct Dependency {
    pub(crate) name: String,
    /// `null` for a normal dependency, `"dev"` or `"build"` otherwise.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "read by this module's own parsing tests; no production code branches on \
                      a manifest-level dependency's kind, since the presence check matches \
                      regardless of it"
        )
    )]
    pub(crate) kind: Option<String>,
    /// The `package = "…"` rename, when the manifest gives one.
    pub(crate) rename: Option<String>,
}

/// The resolved dependency graph.
#[derive(Debug, Deserialize)]
pub(crate) struct Resolve {
    pub(crate) nodes: Vec<Node>,
}

/// One package's position in the resolved graph: which packages it depends
/// on, and what extern-crate name rustc gives each one.
#[derive(Debug, Deserialize)]
pub(crate) struct Node {
    pub(crate) id: String,
    pub(crate) deps: Vec<NodeDependency>,
}

/// One resolved dependency of a [`Node`].
#[derive(Debug, Deserialize)]
pub(crate) struct NodeDependency {
    /// The extern-crate identifier rustc is given for this dependency — the
    /// dependency key or its rename, with any hyphen already turned into an
    /// underscore by Cargo. Never `r#`-prefixed here even when it is a Rust
    /// keyword; that prefix is a rendering decision, not something Cargo
    /// reports (`cargo metadata` reports `move`, not `r#move`).
    pub(crate) name: String,
    /// The package id this dependency resolves to.
    pub(crate) pkg: String,
    pub(crate) dep_kinds: Vec<DepKind>,
}

/// One of the kinds under which a [`NodeDependency`] is depended upon — a
/// package can be both a normal and a dev-dependency at once, hence a list.
#[derive(Debug, Deserialize)]
pub(crate) struct DepKind {
    /// `null` for a normal dependency, `"dev"` or `"build"` otherwise —
    /// mirrors [`Dependency::kind`], but reported per resolved edge rather
    /// than per manifest entry.
    pub(crate) kind: Option<String>,
    /// `null` when this edge holds on every target, or the `cfg(...)`
    /// predicate string when it was declared under
    /// `[target.'cfg(...)'.dependencies]`. A dependency declared only under
    /// `[target.'cfg(windows)'.dependencies]` reports `{"kind": null,
    /// "target": "cfg(windows)"}` here; one declared both unconditionally
    /// and under that predicate reports two entries in the enclosing
    /// [`NodeDependency::dep_kinds`], `{"kind": null, "target": null}` and
    /// `{"kind": null, "target": "cfg(windows)"}`.
    pub(crate) target: Option<String>,
}
