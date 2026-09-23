//! How a composed command line's generated file and manifests are
//! maintained.
//!
//! This is what a scaffolding task links against — `add`, `create`, `new` and
//! `regenerate`, the task crates ritual's own bundle groups — for the renderers
//! that produce a task crate's own files and a composed command line's
//! generated file, and for the metadata and manifest operations `add` and
//! `regenerate` need. Nothing here runs on a composed command line's own path:
//! assembling a command line from a project's imports and dispatching to one of
//! them live in `rituals`, which a composed CLI crate depends on directly.
//!
//! Nothing from `rituals` is re-exported here. Every crate that needs
//! [`rituals::Task`], [`rituals::CommandLine`] or [`rituals::run`] already
//! names `rituals` itself, so a second path to them would be a second way
//! to do one thing. An ordinary, freshly scaffolded task crate depends on
//! `rituals` alone and never on this crate at all.
//!
//! This crate reads the resolved dependency graph via `cargo metadata` and
//! writes manifests in place with `toml_edit`, so that a scaffolding task
//! can append to a manifest a human wrote without disturbing its comments
//! or formatting.
//!
//! # Examples
//!
//! Rendering the two files a new task crate starts as, the way `add` and
//! `create` do before writing them:
//!
//! ```
//! use rituals::Name;
//! use rituals_compose::source::Source;
//! use rituals_compose::task_crate;
//!
//! let name = Name::new("lint")?;
//! let source = Source::Git("https://example.com/ritual".to_string());
//! let manifest = task_crate::manifest(&name, &source);
//! let lib = task_crate::lib(&name);
//!
//! assert!(manifest.contains("name = \"lint\""));
//! assert!(manifest.contains("rituals = { git = \"https://example.com/ritual\" }"));
//! assert!(lib.contains("pub fn task() -> Task"));
//! # Ok::<(), rituals::InvalidName>(())
//! ```

pub mod generated_file;
pub mod manifest;
pub mod metadata;
mod project;
pub mod sentence;
pub mod source;
pub mod task_crate;
mod task_list;
#[cfg(test)]
mod test_support;
pub mod top_level;
pub mod workspace;
