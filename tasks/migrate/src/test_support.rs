//! A scratch directory helper for this crate's own tests.
//!
//! `rituals-compose` already has one, but a different crate is a genuine
//! boundary that module cannot cross — `tasks/new`, `tasks/create` and `tasks/remove` each
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
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rituals_compose::git::fixture;

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
    /// Creates a fresh, empty directory named
    /// `ritual-migrate-<tag>-<pid>-<counter>` under the system temp root.
    pub(crate) fn new(tag: &str) -> Result<Self, Box<dyn Error>> {
        let unique = SCRATCH_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ritual-migrate-{tag}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path)?;
        // Resolved through symbolic links, as the working directory a task
        // runs in is: Cargo reports every path that way, and a root spelled
        // through a link would not compare equal to the members under it.
        Ok(Self(std::fs::canonicalize(path)?))
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

/// Makes `directory` a repository and commits everything in it.
pub(crate) fn init_and_commit(directory: &Path) -> TestOutcome {
    fixture::git(directory, &["init", "--quiet"])?;
    fixture::commit_everything(directory)?;
    Ok(())
}

/// Writes each `(path, contents)` under `root`, creating the directories
/// above it.
pub(crate) fn write_files(root: &Path, files: &[(&str, &str)]) -> TestOutcome {
    for (path, contents) in files {
        let path = root.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
    }
    Ok(())
}

/// A package manifest for the crate `name`, which is a task when `is_task`.
pub(crate) fn package(name: &str, is_task: bool) -> String {
    let task = if is_task {
        "\n[package.metadata.ritual]\ntask = true\n"
    } else {
        ""
    };
    format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n{task}")
}

/// A workspace at `root` whose members are the directories in `members`,
/// each a library crate named for its last component, and a task unless it
/// is in `plain`. `members` entries may be globs: the directories they
/// match are the ones in `directories`.
///
/// The workspace also holds the command line crate `ritual` at `cli/`, which
/// depends on every directory that is a task and lists each in its `tasks`,
/// as the command line of a project that imported them does.
pub(crate) fn workspace(
    root: &Path,
    members: &[&str],
    directories: &[&str],
    plain: &[&str],
) -> TestOutcome {
    let listed: Vec<String> = members
        .iter()
        .chain(&["cli"])
        .map(|member| format!("\"{member}\""))
        .collect();
    write_files(
        root,
        &[(
            "Cargo.toml",
            &format!(
                "[workspace]\nmembers = [{}]\nresolver = \"3\"\n",
                listed.join(", ")
            ),
        )],
    )?;
    let mut imports = Vec::new();
    for directory in directories {
        let name = directory.rsplit('/').next().unwrap_or(directory);
        let is_task = !plain.contains(directory);
        if is_task {
            imports.push((name, *directory));
        }
        let manifest = package(name, is_task);
        write_files(
            root,
            &[
                (&format!("{directory}/Cargo.toml"), &manifest),
                (&format!("{directory}/src/lib.rs"), "//! A fixture.\n"),
            ],
        )?;
    }
    command_line(root, &imports)
}

/// Writes the command line crate `ritual` at `cli/`, depending on and
/// listing each `(name, directory)` in `imports`.
fn command_line(root: &Path, imports: &[(&str, &str)]) -> TestOutcome {
    let mut dependencies = String::new();
    for (name, directory) in imports {
        let _ = writeln!(dependencies, "{name} = {{ path = \"../{directory}\" }}");
    }
    let names: Vec<String> = imports
        .iter()
        .map(|(name, _directory)| format!("\"{name}\""))
        .collect();
    write_files(
        root,
        &[
            (
                "cli/Cargo.toml",
                &format!(
                    "[package]\nname = \"ritual\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
                     [[bin]]\nname = \"ritual\"\npath = \"src/main.rs\"\n\n\
                     [dependencies]\n{dependencies}\n\
                     [package.metadata.ritual]\ntasks = [{}]\n",
                    names.join(", ")
                ),
            ),
            ("cli/src/main.rs", "fn main() {}\n"),
        ],
    )
}
