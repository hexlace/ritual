//! What Cargo's configuration files point at: the paths a build reads
//! because a `.cargo/config.toml` says so, not because a manifest does.
//!
//! Two settings there name a directory Cargo then reads on every build,
//! whether or not anything uses it: a `paths` override, and a `path` under
//! `[patch.<source>]`. `cargo metadata` reports neither, and a `--config`
//! flag cannot be read back on stable Cargo, so the files are read here, the
//! ones Cargo itself reads: `.cargo/config.toml` and its older name
//! `.cargo/config` in a directory and in each directory above it, then the
//! same two in `$CARGO_HOME`, which is `~/.cargo` unless set.
//!
//! # Examples
//!
//! ```no_run
//! use std::path::Path;
//!
//! use rituals_compose::cargo_config;
//!
//! // Reads the configuration files above a real directory, and in
//! // `$CARGO_HOME`, so this example is `no_run`.
//! let start = std::env::current_dir()?;
//! let directory = Path::new("/w/tasks/lint");
//! for entry in cargo_config::entries_pointing_under(&[start.as_path()], directory)? {
//!     println!("still read: {entry}");
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::path::{Path, PathBuf};

use rituals::Failure;
use toml_edit::{DocumentMut, Item, TableLike};

use crate::paths::lies_under;

/// The names Cargo reads a configuration file under, in a `.cargo`
/// directory or in `$CARGO_HOME`: the current one, then the older one.
const FILE_NAMES: [&str; 2] = ["config.toml", "config"];

/// Returns every configuration setting Cargo reads that points into
/// `directory`.
///
/// Each is named with the file it is in, such as `[patch.crates-io] lint in
/// /w/.cargo/config.toml`. `starts` are the directories a build may run from, such as the current
/// directory and the workspace root; the files in each and in every
/// directory above are read, and so are the ones in `$CARGO_HOME`. A
/// relative path in a file is read from the directory that holds its
/// `.cargo` directory, as Cargo reads it, and `directory` should be
/// absolute.
///
/// # Errors
///
/// Returns a [`Failure`] naming a configuration file that exists but cannot
/// be read or is not valid TOML, which Cargo would refuse too.
pub fn entries_pointing_under(starts: &[&Path], directory: &Path) -> Result<Vec<String>, Failure> {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")));
    entries_pointing_under_with(starts, cargo_home.as_deref(), directory)
}

/// [`entries_pointing_under`], with `$CARGO_HOME` given rather than read
/// from the environment, so a test can point it somewhere of its own.
fn entries_pointing_under_with(
    starts: &[&Path],
    cargo_home: Option<&Path>,
    directory: &Path,
) -> Result<Vec<String>, Failure> {
    let mut files: Vec<PathBuf> = Vec::new();
    let in_a_dot_cargo = starts
        .iter()
        .flat_map(|start| start.ancestors())
        .map(|ancestor| ancestor.join(".cargo"));
    for holder in in_a_dot_cargo.chain(cargo_home.map(Path::to_path_buf)) {
        for name in FILE_NAMES {
            let file = holder.join(name);
            if !files.contains(&file) {
                files.push(file);
            }
        }
    }

    let mut entries = Vec::new();
    for file in files {
        let Some(document) = read(&file)? else {
            continue;
        };
        let base = file
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let points_under = |path: &str| lies_under(&base.join(path), directory);

        let overrides = document.get("paths").and_then(Item::as_array);
        for path in overrides
            .into_iter()
            .flatten()
            .filter_map(|path| path.as_str())
        {
            if points_under(path) {
                entries.push(format!("`paths` entry `{path}` in {}", file.display()));
            }
        }
        for (source, patches) in entries_of(document.get("patch")) {
            for (name, declaration) in entries_of(Some(patches)) {
                let path = declaration
                    .as_table_like()
                    .and_then(|declaration| declaration.get("path"))
                    .and_then(Item::as_str);
                if path.is_some_and(points_under) {
                    entries.push(format!("[patch.{source}] {name} in {}", file.display()));
                }
            }
        }
    }
    Ok(entries)
}

/// The configuration file at `path`, parsed, or `None` when there is none.
fn read(path: &Path) -> Result<Option<DocumentMut>, Failure> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(Failure::new(format!("reading {} failed", path.display())).caused_by(error));
        }
    };
    text.parse::<DocumentMut>().map(Some).map_err(|error| {
        Failure::new(format!("{} is not valid TOML", path.display())).caused_by(error)
    })
}

/// Every key and value of `item`, when it is a table of any kind, and
/// nothing otherwise.
fn entries_of(item: Option<&Item>) -> impl Iterator<Item = (&str, &Item)> {
    item.and_then(Item::as_table_like)
        .into_iter()
        .flat_map(TableLike::iter)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::entries_pointing_under_with;
    use crate::test_support::{ScratchDir, TestOutcome};

    #[test]
    fn a_paths_override_and_a_patch_under_the_directory_are_named_with_their_file() -> TestOutcome {
        let scratch = ScratchDir::new("cargo-config-under")?;
        let project = scratch.path().join("project");
        std::fs::create_dir_all(project.join(".cargo"))?;
        std::fs::create_dir_all(project.join("ritual"))?;
        std::fs::write(
            project.join(".cargo/config.toml"),
            "paths = [\"tasks/./lint\", \"vendor/other\"]\n\n\
             [alias]\nritual = \"run -p demo-ritual --\"\n\n\
             [patch.crates-io]\nhelper = { path = \"tasks/lint/helper\" }\n\
             fine = { path = \"vendor/fine\" }\n",
        )?;
        let directory = project.join("tasks/lint");

        let entries = entries_pointing_under_with(&[&project.join("ritual")], None, &directory)?;

        let file = project.join(".cargo/config.toml").display().to_string();
        assert_eq!(
            entries,
            [
                format!("`paths` entry `tasks/./lint` in {file}"),
                format!("[patch.crates-io] helper in {file}"),
            ]
        );
        Ok(())
    }

    /// A file above the project and one in `$CARGO_HOME` are read too, each
    /// relative to the directory holding its `.cargo` — for `$CARGO_HOME`,
    /// the directory that holds it.
    #[test]
    fn files_above_the_start_and_in_cargo_home_are_read_from_their_own_base() -> TestOutcome {
        let scratch = ScratchDir::new("cargo-config-above")?;
        let project = scratch.path().join("outer/project");
        std::fs::create_dir_all(scratch.path().join("outer/.cargo"))?;
        std::fs::create_dir_all(&project)?;
        std::fs::write(
            scratch.path().join("outer/.cargo/config"),
            "paths = [\"project/tasks/lint\"]\n",
        )?;
        let cargo_home = scratch.path().join("home/.cargo");
        std::fs::create_dir_all(&cargo_home)?;
        std::fs::write(
            cargo_home.join("config.toml"),
            "[patch.crates-io]\nlint = { path = \"../outer/project/tasks/lint\" }\n",
        )?;
        let directory = project.join("tasks/lint");

        let entries = entries_pointing_under_with(&[&project], Some(&cargo_home), &directory)?;

        assert_eq!(entries.len(), 2, "{entries:?}");
        assert!(entries[0].starts_with("`paths` entry `project/tasks/lint` in "));
        assert!(entries[1].starts_with("[patch.crates-io] lint in "));
        Ok(())
    }

    #[test]
    fn no_configuration_or_none_pointing_under_names_nothing() -> TestOutcome {
        let scratch = ScratchDir::new("cargo-config-nothing")?;
        std::fs::create_dir_all(scratch.path().join(".cargo"))?;
        std::fs::write(
            scratch.path().join(".cargo/config.toml"),
            "paths = [\"tasks/lint-extra\"]\n",
        )?;

        let entries = entries_pointing_under_with(
            &[scratch.path()],
            Some(&scratch.path().join("no-such-home")),
            &scratch.path().join("tasks/lint"),
        )?;

        assert!(entries.is_empty(), "{entries:?}");
        Ok(())
    }

    #[test]
    fn a_file_that_is_not_toml_is_a_failure_naming_it() -> TestOutcome {
        let scratch = ScratchDir::new("cargo-config-invalid")?;
        std::fs::create_dir_all(scratch.path().join(".cargo"))?;
        std::fs::write(scratch.path().join(".cargo/config.toml"), "paths = [\n")?;

        let failure = entries_pointing_under_with(&[scratch.path()], None, Path::new("/x"))
            .err()
            .ok_or("an invalid file was meant to fail")?;
        assert!(
            failure
                .to_string()
                .contains("config.toml is not valid TOML")
        );
        Ok(())
    }
}
