//! Shared fixtures for the story tests in this directory.
//!
//! Every story test builds its own project inside a temporary directory,
//! drives it through the real `ritual` binary and plain `cargo`, and reads
//! back only what a caller of those tools could see: exit codes, the bytes on
//! stdout and stderr, and the files left on disk. The one thing a story reads
//! from this repository's own tree is its location, handed to `ritual` as the
//! `--path` source a fixture project imports ritual's crates from — see
//! [`in_checkout`].
//!
//! # What the suite needs from the machine
//!
//! A scaffolded project carries no lockfile, so every build resolves its
//! registry dependencies (clap and what it pulls in) against crates.io: the
//! story tests need the network, or a Cargo registry cache warm enough to
//! answer offline. A new release of one of those dependencies reaches these
//! tests the same day it reaches a newly scaffolded project, which is the
//! point of scaffolding from scratch rather than from a pinned lockfile.
//!
//! # Running two suites at once
//!
//! Nothing here writes to a fixed path, and no two stories share a build:
//! each project builds into a target directory inside its own temporary
//! directory (see [`process`]), so no story can replace a binary another is
//! about to run. Two checkouts, two target directories over one checkout,
//! or a runner that starts every test at once, all run side by side without
//! touching each other. The price is that each project compiles its
//! registry dependencies for itself.
//!
//! This module is included, whole, by every test file in this directory via
//! `mod support;`, and each test file calls only the part it needs, so
//! `dead_code` fires per binary on whatever that binary leaves unused. The
//! suite as a whole uses all of it; `support_helpers_read_what_they_claim.rs`
//! is where this module's own behaviour is checked, once.
#![allow(
    dead_code,
    reason = "each test binary uses a different part of this shared module"
)]
// `unreachable_pub` wants everything here `pub(crate)` because the module is
// private, and clippy's `redundant_pub_crate` then reports that same
// `pub(crate)` as redundant. The first is right about this module's actual
// visibility, so the second gives way.
#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) is required by unreachable_pub; see the note above"
)]

pub(crate) mod checkout;
pub(crate) mod crates;
pub(crate) mod generated;
pub(crate) mod help;
pub(crate) mod manifest;
pub(crate) mod process;
pub(crate) mod project;
pub(crate) mod tree;

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

// Re-exported here so a story names what it uses in one `use support::{…}`;
// each test binary uses a different subset of them.
#[allow(
    unused_imports,
    reason = "each test binary uses a different part of this shared module"
)]
pub(crate) use self::{
    checkout::{Checkout, in_checkout},
    crates::Child,
    process::{RunOutput, cargo, run_binary, run_ritual},
    project::Project,
    tree::{assert_trees_identical, snapshot_tree},
};

/// What a test in this suite returns.
///
/// The error path carries a setup failure (a file that would not read, a
/// process that would not spawn), never the property under test, which is
/// always carried by an `assert!`/`assert_eq!`. The workspace's lint table
/// denies `unwrap` and `panic`, and allows `expect` only inside a `#[test]`
/// function, so a fallible setup step in a helper propagates with `?`.
pub(crate) type TestOutcome = Result<(), Box<dyn Error>>;

/// The general form of [`TestOutcome`] for a helper that produces a value
/// rather than just succeeding or failing.
pub(crate) type Outcome<T> = Result<T, Box<dyn Error>>;

/// A plain string turned into the boxed error [`Outcome`] carries.
#[derive(Debug)]
struct Failure(String);

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for Failure {}

/// Turns a message into an [`Outcome`] error, for the spots where nothing
/// upstream failed but a precondition this suite relies on did not hold —
/// "no `[[bin]]` table in this manifest", not "the file would not read".
pub(crate) fn failure<T>(message: impl Into<String>) -> Outcome<T> {
    Err(Box::new(Failure(message.into())))
}

/// Adds context to a [`Result`], turning any error into a boxed [`Outcome`]
/// error prefixed with `message`. Stands in for `.expect(message)`, which
/// the workspace's lints allow inside a `#[test]` function but not in a
/// helper like the ones in this module.
pub(crate) trait ResultContext<T> {
    fn context(self, message: &str) -> Outcome<T>;
}

impl<T, E: fmt::Display> ResultContext<T> for Result<T, E> {
    fn context(self, message: &str) -> Outcome<T> {
        self.map_err(|error| Box::new(Failure(format!("{message}: {error}"))) as Box<dyn Error>)
    }
}

/// Adds context to an [`Option`], turning `None` into a boxed [`Outcome`]
/// error carrying `message`. Stands in for `.expect(message)` on an
/// `Option`, which the workspace's lints allow inside a `#[test]` function
/// but not in a helper like the ones in this module.
pub(crate) trait OptionContext<T> {
    fn context(self, message: &str) -> Outcome<T>;
}

impl<T> OptionContext<T> for Option<T> {
    fn context(self, message: &str) -> Outcome<T> {
        self.ok_or_else(|| Box::new(Failure(message.to_string())) as Box<dyn Error>)
    }
}

/// Renders `path` as UTF-8 text, the shape every `ritual`/`cargo` argument in
/// this suite needs.
pub(crate) fn path_to_str(path: &Path) -> Outcome<&str> {
    path.to_str().context("path is not valid UTF-8")
}

/// Reads `path` as UTF-8 text, naming the file when it will not read.
pub(crate) fn read_text(path: &Path) -> Outcome<String> {
    fs::read_to_string(path).context(&format!("reading {} failed", path.display()))
}

/// Writes `contents` to `path`, naming the file when it will not write.
pub(crate) fn write_text(path: &Path, contents: &str) -> TestOutcome {
    fs::write(path, contents).context(&format!("writing {} failed", path.display()))
}

/// A counter for [`TempDir::new`], so two temporary directories created in
/// the same process never collide.
///
/// Fixture uniqueness, not a seed: nothing here needs to be unpredictable,
/// only distinct within one process, and the process id in the name makes it
/// distinct across processes.
static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory under the system temp root that removes itself on drop.
///
/// Every fixture a test needs — a scaffolded project, a hand-written crate
/// standing in for an unmarked dependency, a working directory for an
/// adversarial-name attempt — lives inside one of these, never inside this
/// repository's own tree.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a fresh, empty directory named
    /// `ritual-tests-<prefix>-<pid>-<counter>` under the system temp root.
    pub(crate) fn new(prefix: &str) -> Outcome<Self> {
        let unique = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ritual-tests-{prefix}-{}-{unique}",
            std::process::id()
        ));

        fs::create_dir_all(&path).context("creating a temp dir failed")?;

        // Canonicalized once, here: on macOS the system temp root is reached
        // through a symlink (`/var` -> `/private/var`). A story that reaches
        // one directory both through an absolute path (`cargo add --path`,
        // which Cargo canonicalizes) and through a relative one (a workspace
        // member, a `path = "../sibling"` dependency) would otherwise have
        // Cargo see two different strings for one directory and refuse with a
        // lockfile collision.
        let path = path
            .canonicalize()
            .context("canonicalizing a freshly created temp dir failed")?;

        let is_empty = fs::read_dir(&path)
            .context("reading a freshly created temp dir failed")?
            .next()
            .is_none();
        assert!(is_empty, "a freshly created temp dir must start empty");

        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Best effort: a leftover temp directory costs disk space, not
        // correctness, and a test that already failed should not fail a
        // second time over cleanup.
        drop(fs::remove_dir_all(&self.path));
    }
}
