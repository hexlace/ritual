#![doc = include_str!("../readme.md")]
//!
//! ## What this crate provides
//!
//! - For writing a task: [`Task`], [`Outcome`], [`Failure`], [`report()`], and
//!   [`clap`].
//! - For a task that needs the command line it runs in:
//!   [`Task::receiving_command_line`], [`CommandLine`] and [`Identity`].
//! - For generated code and scaffolding tools rather than tasks: [`run`],
//!   [`identity!`], [`Name`], [`InvalidName`] and [`VERSION`].

pub use clap;

mod command_line;
mod dispatch;
mod identity;
mod name;
mod outcome;
mod report;
mod task;
#[cfg(test)]
mod test_support;

pub use command_line::CommandLine;
pub use dispatch::run;
pub use identity::Identity;
pub use name::{InvalidName, Name};
pub use outcome::{Failure, Outcome};
pub use report::report;
pub use task::Task;

/// The version of this crate, as the build that compiled it saw it.
///
/// This is the version of the framework, not of any command line built on
/// it; that one is [`Identity::version`]. Ritual's scaffolding tasks write
/// it into a new project's manifests, so the project asks crates.io for the
/// `rituals` and `rituals-core` the scaffolding binary was built with.
///
/// # Examples
///
/// ```
/// let requirement = format!("rituals = \"{}\"", rituals::VERSION);
/// assert!(requirement.starts_with("rituals = \""));
/// ```
//
// One constant can name every crate ritual publishes because they are all
// released at the same version together. A project imports `rituals`
// directly and `rituals-core` for its management tasks, and the generated
// `main.rs` passes a task from one to a function in the other: requested at
// different versions, the two could resolve to two copies of `rituals`
// whose `Task` types are not the same type.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
