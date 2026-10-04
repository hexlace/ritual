//! Which directories of a project hold Cargo configuration a build reads.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Obstacle, run_git};

/// The names Cargo reads a configuration file under in a `.cargo`
/// directory.
const CARGO_CONFIGURATION_FILE_NAMES: [&str; 2] = ["config", "config.toml"];

/// Returns every directory under `workspace_root` holding a `.cargo/config`
/// or `.cargo/config.toml` that git tracks, or would: tracked, or untracked
/// and not ignored.
///
/// A build started in that directory or below it reads the file, wherever
/// the directory is in the project, so each is a place a build can start
/// from. Git lists them because it already walks the whole tree and knows
/// which files belong to the project; an ignored one is left out, as git
/// leaves it out of what it would commit.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::git;
///
/// // Needs a real repository on disk and runs `git`, so this example is
/// // `no_run`.
/// for directory in git::directories_holding_cargo_configuration(Path::new("."))? {
///     println!("a build in {} reads Cargo configuration", directory.display());
/// }
/// # Ok::<(), rituals_compose::git::Obstacle>(())
/// ```
///
/// # Errors
///
/// Returns [`Obstacle::GitMissing`] when `git` cannot be run,
/// [`Obstacle::NotARepository`] when `workspace_root` is not in a git
/// repository, and [`Obstacle::Failed`] for anything else git reports. It
/// returns no other variant.
pub fn directories_holding_cargo_configuration(
    workspace_root: &Path,
) -> Result<Vec<PathBuf>, Obstacle> {
    directories_holding_cargo_configuration_with(|| Command::new("git"), workspace_root)
}

/// [`directories_holding_cargo_configuration`], with `git` started from
/// `new_git`.
fn directories_holding_cargo_configuration_with(
    new_git: impl Fn() -> Command,
    workspace_root: &Path,
) -> Result<Vec<PathBuf>, Obstacle> {
    let listed = run_git(
        &new_git,
        workspace_root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            ".",
        ],
    )?;
    let text = String::from_utf8_lossy(&listed);
    let mut directories: Vec<PathBuf> = Vec::new();
    for file in text
        .split('\0')
        .filter(|file| !file.is_empty())
        .map(Path::new)
    {
        let is_configuration = file.file_name().is_some_and(|name| {
            CARGO_CONFIGURATION_FILE_NAMES
                .iter()
                .any(|known| name == *known)
        }) && file
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|holder| holder == ".cargo");
        let Some(directory) = file.parent().and_then(Path::parent) else {
            continue;
        };
        // Git names each file from the directory it was asked in, and a file
        // with unmerged stages more than once.
        let directory = if directory.as_os_str().is_empty() {
            workspace_root.to_path_buf()
        } else {
            workspace_root.join(directory)
        };
        if is_configuration && !directories.contains(&directory) {
            directories.push(directory);
        }
    }
    Ok(directories)
}

#[cfg(test)]
mod tests {
    use super::directories_holding_cargo_configuration_with;
    use crate::git::Obstacle;
    use crate::git::test_support::{commit_everything, contained_in, git};
    use crate::test_support::{ScratchDir, TestOutcome};

    /// A `.cargo/config` or `.cargo/config.toml` anywhere under the root,
    /// tracked or untracked, names the directory holding its `.cargo`; an
    /// ignored one, a file of another name in `.cargo`, and a `config.toml`
    /// outside one do not.
    #[test]
    fn every_cargo_configuration_git_would_keep_names_its_directory() -> TestOutcome {
        let scratch = ScratchDir::new("cargo-configuration")?;
        let root = scratch.path();
        for file in [
            ".cargo/config.toml",
            "docs/.cargo/config.toml",
            "deep/er/.cargo/config",
            "untracked/.cargo/config.toml",
            "ignored/.cargo/config.toml",
            "other/.cargo/extra.toml",
            "loose/config.toml",
        ] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().ok_or("a fixture file has a directory")?)?;
            std::fs::write(&path, "\n")?;
        }
        std::fs::write(root.join(".gitignore"), "ignored/\nuntracked/\n")?;
        git(root, &["init"])?;
        commit_everything(root)?;
        std::fs::write(root.join(".gitignore"), "ignored/\n")?;

        let mut directories =
            directories_holding_cargo_configuration_with(contained_in(root), root)
                .map_err(|obstacle| format!("{obstacle:?}"))?;
        directories.sort();

        assert_eq!(
            directories,
            [
                root.to_path_buf(),
                root.join("deep/er"),
                root.join("docs"),
                root.join("untracked"),
            ]
        );
        Ok(())
    }

    #[test]
    fn listing_cargo_configuration_outside_a_repository_is_refused_as_no_repository() -> TestOutcome
    {
        let scratch = ScratchDir::new("cargo-configuration-no-repository")?;

        let listed = directories_holding_cargo_configuration_with(
            contained_in(scratch.path()),
            scratch.path(),
        );

        assert_eq!(listed, Err(Obstacle::NotARepository));
        Ok(())
    }
}
