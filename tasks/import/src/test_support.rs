//! A scratch directory helper for this crate's own tests.
//!
//! `rituals-compose` already has one, but a different crate is a genuine
//! boundary that module cannot cross — `tasks/new` and `tasks/create` each
//! keep their own copy for the same reason, and this is this crate's.
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

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rituals::clap::{self, CommandFactory, Parser};

use crate::arguments::ImportArguments;

/// What a test in this crate returns — the error path carries only a setup
/// failure (a filesystem operation, a TOML fixture that would not parse),
/// never the property under test, which is always carried by an
/// `assert!`/`assert_eq!` instead.
pub(crate) type TestOutcome = Result<(), Box<dyn Error>>;

/// A counter for [`ScratchDir::new`], so two scratch directories created in
/// the same test process never collide.
///
/// This is test-fixture uniqueness, not a production seed: nothing here
/// needs to be unpredictable, only distinct within one test run.
static SCRATCH_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory under the system temp root that removes itself on drop.
pub(crate) struct ScratchDir(PathBuf);

impl ScratchDir {
    /// Creates a fresh, empty directory named `ritual-import-<tag>-<pid>-<counter>`
    /// under the system temp root.
    pub(crate) fn new(tag: &str) -> Result<Self, Box<dyn Error>> {
        let unique = SCRATCH_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ritual-import-{tag}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a command line parses `import`'s arguments from, so a test can give
/// them as a person types them.
#[derive(clap::Parser, Debug)]
struct Typed {
    #[command(flatten)]
    arguments: ImportArguments,
}

/// `import`'s arguments, parsed from `words` as clap parses a real run's.
pub(crate) fn typed_arguments(words: &[&str]) -> ImportArguments {
    parse_typed(words)
        .expect("the arguments a test gives parse")
        .arguments
}

/// The clap command `import`'s arguments make, named as a command line names
/// it, for a test to read its usage and help from.
pub(crate) fn import_command() -> clap::Command {
    Typed::command().name("import")
}

/// Whether clap refuses `words` as `import`'s arguments.
pub(crate) fn clap_refuses(words: &[&str]) -> bool {
    parse_typed(words).is_err()
}

fn parse_typed(words: &[&str]) -> Result<Typed, clap::Error> {
    let mut full = vec!["import"];
    full.extend_from_slice(words);
    Typed::try_parse_from(full)
}

/// Every file under a directory tree, as a path paired with its bytes,
/// sorted by path: a before and after to compare a run against.
pub(crate) type Snapshot = Vec<(PathBuf, Vec<u8>)>;

/// A scratch project shaped like one `new` scaffolds, small enough that
/// Cargo resolves it with nothing to download: a workspace whose one member
/// is a composed CLI crate called `demo-ritual` with an empty task list.
/// Task crates to import are written beside it, and need nothing beyond the
/// manifest that marks them.
pub(crate) struct ScratchProject {
    root: ScratchDir,
}

impl ScratchProject {
    pub(crate) fn new(tag: &str) -> Result<Self, Box<dyn Error>> {
        let root = ScratchDir::new(tag)?;
        std::fs::create_dir_all(root.path().join("cli/src"))?;
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"cli\"]\nresolver = \"3\"\n",
        )?;
        std::fs::write(
            root.path().join("cli/Cargo.toml"),
            "[package]\nname = \"demo-ritual\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
             [dependencies]\n\n[package.metadata.ritual]\ntasks = []\n",
        )?;
        std::fs::write(root.path().join("cli/src/main.rs"), "fn main() {}\n")?;
        Ok(Self { root })
    }

    /// The workspace root.
    pub(crate) fn root(&self) -> &Path {
        self.root.path()
    }

    /// The composed CLI crate's directory, where a person would type `import`.
    pub(crate) fn cli_dir(&self) -> PathBuf {
        self.root().join("cli")
    }

    pub(crate) fn cli_manifest_path(&self) -> PathBuf {
        self.cli_dir().join("Cargo.toml")
    }

    pub(crate) fn lockfile_path(&self) -> PathBuf {
        self.root().join("Cargo.lock")
    }

    /// Writes a library crate called `name` in a directory of that name beside
    /// the CLI, marked as a task when `is_a_task`, and returns its directory.
    pub(crate) fn write_crate(
        &self,
        name: &str,
        is_a_task: bool,
    ) -> Result<PathBuf, Box<dyn Error>> {
        let directory = self.root().join(name);
        std::fs::create_dir_all(directory.join("src"))?;
        let mark = if is_a_task {
            "\n[package.metadata.ritual]\ntask = true\n"
        } else {
            ""
        };
        std::fs::write(
            directory.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{mark}"
            ),
        )?;
        std::fs::write(directory.join("src/lib.rs"), "")?;
        Ok(directory)
    }

    /// Has Cargo write the lockfile a built project has, which `cargo
    /// metadata` would otherwise create the first time anything asks it a
    /// question, so a test of what a run leaves alone starts from the state
    /// of a project that has been built.
    pub(crate) fn with_a_lockfile(self) -> Result<Self, Box<dyn Error>> {
        let generated = rituals_compose::cargo::command()
            .arg("generate-lockfile")
            .current_dir(self.cli_dir())
            .output()?;
        assert!(
            generated.status.success(),
            "cargo made a lockfile: {}",
            String::from_utf8_lossy(&generated.stderr)
        );
        Ok(self)
    }

    pub(crate) fn snapshot(&self) -> Result<Snapshot, Box<dyn Error>> {
        let mut files = Vec::new();
        let mut pending = vec![self.root().to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(&directory)? {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_dir() {
                    pending.push(path);
                } else {
                    let bytes = std::fs::read(&path)?;
                    files.push((path, bytes));
                }
            }
        }
        files.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(files)
    }
}
