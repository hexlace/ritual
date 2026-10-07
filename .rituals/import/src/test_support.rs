//! What this crate's tests share: the outcome a test returns, a scratch
//! project and `import`'s arguments as a person types them, and the scratch
//! directory and tree snapshot every crate's unit tests take from
//! `rituals_compose::test_util`.
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

use rituals::clap::{self, CommandFactory, Parser};

use crate::arguments::ImportArguments;

pub(crate) use rituals_compose::test_util::{ScratchDir, Snapshot, snapshot};

/// What a test in this crate returns — the error path carries only a setup
/// failure (a filesystem operation, a TOML fixture that would not parse),
/// never the property under test, which is always carried by an
/// `assert!`/`assert_eq!` instead.
pub(crate) type TestOutcome = Result<(), Box<dyn Error>>;

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

/// How the CLI crate and every crate written beside it depend on the
/// project's `rituals`, from one directory below the workspace root.
pub(crate) const RITUALS_DEPENDENCY: &str = "rituals = { path = \"../rituals\" }";

/// A scratch project shaped like one `new` scaffolds, small enough that
/// Cargo resolves it with nothing to download: a workspace whose one member
/// is a composed CLI crate called `demo-ritual` with an empty task list,
/// depending on a crate called `rituals` beside it. Task crates to import
/// are written beside it too, and need nothing beyond the manifest that
/// marks them and the same `rituals`, which a task and the command line
/// mounting it must share.
pub(crate) struct ScratchProject {
    root: ScratchDir,
}

impl ScratchProject {
    pub(crate) fn new(tag: &str) -> Result<Self, Box<dyn Error>> {
        let root = ScratchDir::new(tag)?;
        std::fs::create_dir_all(root.path().join("cli/src"))?;
        std::fs::create_dir_all(root.path().join("rituals/src"))?;
        std::fs::write(
            root.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"cli\"]\nresolver = \"3\"\n",
        )?;
        std::fs::write(
            root.path().join("cli/Cargo.toml"),
            format!(
                "[package]\nname = \"demo-ritual\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
                 [dependencies]\n{RITUALS_DEPENDENCY}\n\n[package.metadata.ritual]\ntasks = []\n"
            ),
        )?;
        std::fs::write(
            root.path().join("rituals/Cargo.toml"),
            "[package]\nname = \"rituals\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        std::fs::write(root.path().join("rituals/src/lib.rs"), "")?;
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
    /// the CLI, depending on the project's `rituals`, marked as a task when
    /// `is_a_task`, and returns its directory.
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
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n\
                 [dependencies]\n{RITUALS_DEPENDENCY}\n{mark}"
            ),
        )?;
        std::fs::write(directory.join("src/lib.rs"), "")?;
        Ok(directory)
    }

    /// Has Cargo write the lockfile a built project has, so a test starts
    /// from the state of a project that has been built, where a lockfile is
    /// there to be restored rather than created.
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

    /// The project's whole tree, symbolic links as links, for a before and
    /// after to compare a run against.
    pub(crate) fn snapshot(&self) -> Result<Snapshot, Box<dyn Error>> {
        Ok(snapshot(self.root())?)
    }
}
