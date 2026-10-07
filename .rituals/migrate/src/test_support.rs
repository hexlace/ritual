//! What this crate's tests share: the outcome a test returns, the fixture
//! workspaces they build, and the scratch directory every crate's unit tests
//! take from `rituals_compose::test_util`.
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
use std::path::Path;

use rituals::{CommandLine, Identity};
use rituals_compose::git::fixture;

// Every test in this crate asks for `ScratchDir::resolved`. `migrate`
// decides what to do by comparing the directories Cargo reports, which are
// resolved, with paths built from the root. Under a root spelled through a
// link nothing matches, so a test asserting that something is not moved or
// does not apply would pass whatever the code under test decided.
pub(crate) use rituals_compose::test_util::ScratchDir;

/// What a test in this crate returns — the error path carries only a setup
/// failure (a filesystem operation, a TOML fixture that would not parse),
/// never the property under test, which is always carried by an
/// `assert!`/`assert_eq!` instead.
pub(crate) type TestOutcome = Result<(), Box<dyn Error>>;

/// A command line built as `bin`, reached through `path`, as dispatch would
/// hand it to `migrate`.
pub(crate) fn command_line(
    bin: &'static str,
    path: impl IntoIterator<Item = &'static str>,
) -> CommandLine {
    CommandLine::from_dispatch(
        Identity::from_macro_expansion("demo-ritual", bin, "0.1.0"),
        ["add", "regenerate"],
        path,
    )
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
pub(crate) fn workspace(
    root: &Path,
    members: &[&str],
    directories: &[&str],
    plain: &[&str],
) -> TestOutcome {
    let listed: Vec<String> = members
        .iter()
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
    for directory in directories {
        let name = directory.rsplit('/').next().unwrap_or(directory);
        let manifest = package(name, !plain.contains(directory));
        write_files(
            root,
            &[
                (&format!("{directory}/Cargo.toml"), &manifest),
                (&format!("{directory}/src/lib.rs"), "//! A fixture.\n"),
            ],
        )?;
    }
    Ok(())
}
