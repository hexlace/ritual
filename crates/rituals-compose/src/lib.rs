//! How a composed command line's generated file and manifests are
//! maintained.
//!
//! This is what a scaffolding task links against — such as `add`, `create`,
//! `import`, `migrate`, `new`, `regenerate` and `remove`, the task crates ritual's
//! own bundle groups — for the renderers that produce a task crate's own files and
//! a composed command line's generated file, for the metadata and manifest
//! operations those tasks need, for the [`cargo`] a task that runs Cargo uses
//! and the [`shell`] that renders a command for a person to copy, for the
//! [`layout`] that says where a scaffolder puts a ritual, for the [`git`]
//! checks that say whether git can give back what a task is about to delete
//! or move, for the [`relocation`] that says where paths go when directories
//! move, for the [`paths`] that compares and spells them, and for the
//! [`rollback`] that puts a project back when a task's run does not finish.
//! Nothing here runs on a composed command line's own path: assembling a
//! command line from a project's imports and dispatching to one of them live
//! in `rituals`, which a composed CLI crate depends on directly.
//!
//! Nothing from `rituals` is re-exported here. Every crate that needs
//! [`rituals::Task`], [`rituals::CommandLine`] or [`rituals::run`] already
//! names `rituals` itself, so a second path to them would be a second way
//! to do one thing. An ordinary, freshly scaffolded task crate depends on
//! `rituals` alone and never on this crate at all.
//!
//! This crate reads the resolved dependency graph via `cargo metadata` and
//! writes manifests in place with `toml_edit`, so that a scaffolding task
//! can append to or remove from a manifest a human wrote without disturbing
//! its comments or formatting. It also reads Cargo's own configuration
//! files, through [`cargo_config`], for the paths a build reads that no
//! manifest names.
//!
//! # Feature flags
//!
//! - `test-util` — adds `git::fixture`, a `git` that reads nothing from the
//!   machine it runs on and the commands that build a fixture repository, for
//!   a crate whose tests run against `git`; and `test_util`, the scratch
//!   directory and tree snapshot every crate's unit tests share. It is test
//!   support for the crates of ritual's own repository, nothing needs it at
//!   run time, and it sits outside this crate's stability promise: either
//!   module can change or go in any release.
//!
//! # Examples
//!
//! Rendering the two files a new task crate starts as, the way `create` does
//! before writing them:
//!
//! ```
//! use rituals::Name;
//! use rituals_compose::source::Source;
//! use rituals_compose::task_crate::{self, Audience};
//!
//! let name = Name::new("lint")?;
//! let source = Source::Git("https://example.com/ritual".to_string());
//! let manifest = task_crate::manifest(&name, &source, Audience::Public);
//! let lib = task_crate::lib(&name);
//!
//! assert!(manifest.contains("name = \"lint\""));
//! assert!(manifest.contains("rituals = { git = \"https://example.com/ritual\" }"));
//! assert!(lib.contains("pub fn task() -> Task"));
//! # Ok::<(), rituals::InvalidName>(())
//! ```

pub mod cargo;
pub mod cargo_config;
pub mod generated_file;
pub mod git;
pub mod layout;
pub mod manifest;
pub mod metadata;
pub mod paths;
mod project;
pub mod relocation;
pub mod rollback;
pub mod rust_name;
pub mod sentence;
pub mod shell;
pub mod source;
pub mod task_crate;
mod task_imports;
mod task_list;
#[cfg(test)]
mod test_support;
#[cfg(any(test, feature = "test-util"))]
pub mod test_util;
pub mod top_level;
pub mod workspace;
