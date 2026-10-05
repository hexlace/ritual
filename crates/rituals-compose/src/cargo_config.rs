//! What Cargo's configuration files point at: the paths a build reads
//! because a configuration file says so, not because a manifest does.
//!
//! Two settings there name a directory Cargo then reads on every build,
//! whether or not anything uses it: a `paths` override, and a `path` under
//! `[patch.<source>]`. A third names a file Cargo must find before it can
//! build at all: an `include` that is not `optional`. `cargo metadata`
//! reports none of them, and a `--config` flag cannot be read back on stable
//! Cargo, so the files are read here, the ones Cargo itself reads.
//!
//! Those are, for each directory a build may start from and every directory
//! above it, the file in its `.cargo` directory, then the one in
//! `$CARGO_HOME`, which is `~/.cargo` unless set. In each place that file is
//! `config` when it exists and `config.toml` otherwise: Cargo reads the
//! older name in preference and ignores the other. Then every file those
//! include, and every file those include in turn.
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
//! let directory = Path::new("/w/.rituals/lint");
//! for entry in cargo_config::entries_pointing_under(&[start.as_path()], directory)? {
//!     println!("still read: {entry}");
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::path::{Path, PathBuf};

use rituals::Failure;
use toml_edit::{DocumentMut, Item, TableLike, Value};

use crate::paths::{lies_under, opens_through};

/// The names Cargo reads a configuration file under, in a `.cargo`
/// directory or in `$CARGO_HOME`, in the order it prefers them: it reads the
/// first that exists, and only that one.
const FILE_NAMES: [&str; 2] = ["config", "config.toml"];

/// The extension Cargo requires of a file named by `include`; it refuses
/// any other.
const INCLUDE_EXTENSION: &str = "toml";

/// Returns every configuration setting Cargo reads that points into
/// `directory`.
///
/// Each is named with the file it is in, such as `[patch.crates-io] lint in
/// /w/.cargo/config.toml`. `starts` are the directories a build may run
/// from, such as the current directory, the workspace root and each
/// member's directory; the files in each and in every directory above are
/// read, and so are the ones in `$CARGO_HOME`, and every file any of those
/// includes. A relative path in a file is read from the parent of the
/// directory holding the file, as Cargo reads it, and `directory` should be
/// absolute.
///
/// A file inside `directory` is not read, since it goes with it, and an
/// `optional` include of one is not either; an include of one that is not
/// `optional` is an entry, because Cargo refuses to build without the file.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::cargo_config;
///
/// // A task about to delete `.rituals/lint` asks whether a build would still
/// // read it because of a configuration file; this reads the files above a
/// // real directory and in `$CARGO_HOME`, so it is `no_run`.
/// let workspace_root = Path::new("/w");
/// let directory = workspace_root.join(".rituals/lint");
/// let entries = cargo_config::entries_pointing_under(&[workspace_root], &directory)?;
/// if !entries.is_empty() {
///     println!("still read through {}", entries.join(", "));
/// }
/// # Ok::<(), rituals::Failure>(())
/// ```
///
/// # Errors
///
/// Returns a [`Failure`] naming a configuration file that Cargo refuses
/// too: one that exists but cannot be read or is not valid TOML; and,
/// wherever Cargo reads `include`, an `include` that is not a list of paths
/// and `{ path, optional }` tables or names a file that is not `.toml`, an
/// include that is not `optional` of a file that does not exist, and a file
/// included twice from one starting file, which Cargo calls a cycle. A
/// Cargo too old to read `include` ignores some of these; they are refused
/// there too, because any Cargo that reads `include` refuses the project.
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
    let in_a_dot_cargo = starts
        .iter()
        .flat_map(|start| start.ancestors())
        .map(|ancestor| ancestor.join(".cargo"));
    let mut holders: Vec<PathBuf> = Vec::new();
    for holder in in_a_dot_cargo.chain(cargo_home.map(Path::to_path_buf)) {
        // A file in the directory is deleted with it, and nothing builds
        // from inside a directory that is gone.
        if !lies_under(&holder, directory) && !holders.contains(&holder) {
            holders.push(holder);
        }
    }

    let mut entries = Vec::new();
    for holder in holders {
        let Some(file) = FILE_NAMES
            .iter()
            .map(|name| holder.join(name))
            .find(|file| file.exists())
        else {
            continue;
        };
        entries.extend(entries_through(file, directory)?);
    }
    Ok(entries)
}

/// One file to read, and whether its absence is allowed: a starting file
/// that is not there is simply not read, and so is an `optional` include.
struct Pending {
    file: PathBuf,
    /// The file that includes this one, for an include that is not
    /// `optional`.
    required_by: Option<PathBuf>,
}

/// Every entry pointing into `directory` in `file` and in every file it
/// includes, at any depth.
///
/// Files are tracked by the path they are reached by, as written: Cargo
/// refuses a file reached twice from one starting file, a diamond as much as
/// a loop, and compares those paths without resolving `..`. So every step
/// either reads a file not read before from this start or fails, and a chain
/// whose spelling grows on each step, such as `sub/../config.toml` including
/// itself, ends when the path grows past what the filesystem will open.
fn entries_through(file: PathBuf, directory: &Path) -> Result<Vec<String>, Failure> {
    let mut entries = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut pending = vec![Pending {
        file,
        required_by: None,
    }];
    while let Some(Pending { file, required_by }) = pending.pop() {
        if seen.contains(&file) {
            return Err(Failure::new(format!(
                "{} is included twice from one configuration file, which Cargo refuses as a \
                 cycle wherever it reads `include`",
                file.display()
            )));
        }
        seen.push(file.clone());
        let Some(document) = read(&file)? else {
            match required_by {
                Some(includer) => {
                    return Err(Failure::new(format!(
                        "{} includes {}, which does not exist, and wherever Cargo reads \
                         `include` it refuses one that is not `optional` when its file is \
                         missing",
                        includer.display(),
                        file.display()
                    )));
                }
                None => continue,
            }
        };

        // Cargo joins an include onto the including file's directory, and
        // reads a relative path in any file from that directory's parent,
        // both as written: `.cargo/../extra.toml` reads its paths from
        // `.cargo`, the parent of `.cargo/..`.
        let holder = file.parent().map(Path::to_path_buf).unwrap_or_default();
        let base = holder.parent().map(Path::to_path_buf).unwrap_or_default();
        entries.extend(entries_in(&document, &file, &base, directory));

        let mut included = Vec::new();
        for Include { path, optional } in includes(&document, &file)? {
            let target = holder.join(&path);
            // Opened as written, so a `..` after a link goes up from where
            // the link leads.
            if opens_through(&target, directory) {
                // Gone with the directory: Cargo skips an `optional` file it
                // cannot find, and refuses to build without any other.
                if !optional {
                    entries.push(format!("`include` of `{path}` in {}", file.display()));
                }
                continue;
            }
            included.push(Pending {
                file: target,
                required_by: (!optional).then(|| file.clone()),
            });
        }
        // Last pushed is first read, so the includes are read in the order
        // they are written.
        pending.extend(included.into_iter().rev());
    }
    Ok(entries)
}

/// The `paths` and `[patch]` entries in `document`, the file at `file`,
/// that point into `directory` once read from `base`.
fn entries_in(document: &DocumentMut, file: &Path, base: &Path, directory: &Path) -> Vec<String> {
    let points_under = |path: &str| lies_under(&base.join(path), directory);
    let mut entries = Vec::new();

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
    entries
}

/// One file an `include` names, as written, and whether Cargo builds
/// without it when it is missing.
struct Include {
    path: String,
    optional: bool,
}

/// Every file `document`'s `include` names, in order.
///
/// Cargo takes a list whose items are each a path or a `{ path, optional }`
/// table, each naming a `.toml` file, and refuses anything else, a single
/// path not in a list included; so this refuses the same, naming `file`.
fn includes(document: &DocumentMut, file: &Path) -> Result<Vec<Include>, Failure> {
    let unreadable = || {
        Failure::new(format!(
            "`include` in {} is not a list of paths and `{{ path, optional }}` tables, which \
             Cargo refuses wherever it reads `include`",
            file.display()
        ))
    };
    let includes: Vec<Include> = match document.get("include") {
        None => Vec::new(),
        Some(Item::ArrayOfTables(tables)) => tables
            .iter()
            .map(|table| include_from(table, &unreadable))
            .collect::<Result<_, _>>()?,
        Some(Item::Value(Value::Array(items))) => items
            .iter()
            .map(|item| match item {
                Value::String(path) => Ok(Include {
                    path: path.value().clone(),
                    optional: false,
                }),
                Value::InlineTable(table) => include_from(table, &unreadable),
                _ => Err(unreadable()),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(unreadable()),
    };
    if let Some(include) = includes.iter().find(|include| {
        Path::new(&include.path)
            .extension()
            .is_none_or(|extension| extension != INCLUDE_EXTENSION)
    }) {
        return Err(Failure::new(format!(
            "`include` in {} names `{}`, which does not end in `.{INCLUDE_EXTENSION}`, and \
             Cargo refuses it",
            file.display(),
            include.path
        )));
    }
    Ok(includes)
}

/// The include a `{ path, optional }` table describes; `optional` is
/// `false` when it is left out.
fn include_from(
    table: &dyn TableLike,
    unreadable: &impl Fn() -> Failure,
) -> Result<Include, Failure> {
    let path = table
        .get("path")
        .and_then(Item::as_str)
        .ok_or_else(unreadable)?;
    let optional = match table.get("optional") {
        None => false,
        Some(optional) => optional.as_bool().ok_or_else(unreadable)?,
    };
    Ok(Include {
        path: path.to_string(),
        optional,
    })
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

    /// Writes each `(path, contents)` under `root`, making its directories.
    fn write_files(root: &Path, files: &[(&str, &str)]) -> TestOutcome {
        for (path, contents) in files {
            let path = root.join(path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, contents)?;
        }
        Ok(())
    }

    /// The entries pointing into `project/tasks/lint`, starting from
    /// `project` with no `$CARGO_HOME`.
    fn entries_for_lint(project: &Path) -> Result<Vec<String>, rituals::Failure> {
        entries_pointing_under_with(&[project], None, &project.join("tasks/lint"))
    }

    /// Cargo takes a list of paths, a list of `{ path, optional }` tables,
    /// or `[[include]]` tables, and follows an include in an included file
    /// too; each is followed here.
    #[test]
    fn every_form_of_include_is_followed_at_any_depth() -> TestOutcome {
        let scratch = ScratchDir::new("cargo-config-include-forms")?;
        let project = scratch.path();
        write_files(
            project,
            &[
                (
                    ".cargo/config.toml",
                    "include = [\"strings.toml\", { path = \"table.toml\" }]\n",
                ),
                (".cargo/strings.toml", "paths = [\"tasks/lint\"]\n"),
                (
                    ".cargo/table.toml",
                    "[[include]]\npath = \"nested.toml\"\noptional = false\n",
                ),
                (
                    ".cargo/nested.toml",
                    "[patch.crates-io]\nlint = { path = \"tasks/lint\" }\n",
                ),
            ],
        )?;

        let entries = entries_for_lint(project)?;

        let in_cargo = |name: &str| project.join(".cargo").join(name).display().to_string();
        assert_eq!(
            entries,
            [
                format!("`paths` entry `tasks/lint` in {}", in_cargo("strings.toml")),
                format!("[patch.crates-io] lint in {}", in_cargo("nested.toml")),
            ]
        );
        Ok(())
    }

    /// An include is joined onto the including file's directory, and its
    /// own paths are read from the parent of the directory it is in, both
    /// as written, as measured on Cargo 1.95: `conf/extra.toml` reads from
    /// the project, `a/b/c/extra.toml` from `a/b`, and `.cargo/../extra.toml`
    /// from `.cargo`, the parent of `.cargo/..`.
    #[test]
    fn an_included_file_reads_its_paths_from_the_parent_of_its_own_directory() -> TestOutcome {
        let scratch = ScratchDir::new("cargo-config-include-base")?;
        let project = scratch.path();
        write_files(
            project,
            &[
                (
                    ".cargo/config.toml",
                    "include = [\"../conf/extra.toml\", \"../a/b/c/extra.toml\", \
                     \"../top.toml\"]\n",
                ),
                ("conf/extra.toml", "paths = [\"tasks/lint\"]\n"),
                ("a/b/c/extra.toml", "paths = [\"../../tasks/lint\"]\n"),
                ("top.toml", "paths = [\"../tasks/lint\"]\n"),
            ],
        )?;

        let entries = entries_for_lint(project)?;

        assert_eq!(entries.len(), 3, "{entries:?}");
        assert!(entries[0].starts_with("`paths` entry `tasks/lint` in "));
        assert!(entries[1].starts_with("`paths` entry `../../tasks/lint` in "));
        assert!(entries[2].starts_with("`paths` entry `../tasks/lint` in "));

        // Read from the project, as a file in `.cargo` would be, the last
        // one points beside it rather than into it.
        write_files(project, &[("top.toml", "paths = [\"tasks/lint\"]\n")])?;
        assert_eq!(entries_for_lint(project)?.len(), 2);
        Ok(())
    }

    #[test]
    fn a_missing_optional_include_is_skipped_and_a_missing_required_one_fails() -> TestOutcome {
        let scratch = ScratchDir::new("cargo-config-include-missing")?;
        let project = scratch.path();
        write_files(
            project,
            &[(
                ".cargo/config.toml",
                "include = [{ path = \"absent.toml\", optional = true }]\n",
            )],
        )?;
        assert!(entries_for_lint(project)?.is_empty());

        write_files(
            project,
            &[(".cargo/config.toml", "include = [\"absent.toml\"]\n")],
        )?;
        let failure = entries_for_lint(project)
            .err()
            .ok_or("a missing required include was meant to fail")?;
        assert!(
            failure
                .to_string()
                .contains("absent.toml, which does not exist"),
            "{failure}"
        );
        Ok(())
    }

    /// A file inside the directory goes with it: Cargo then refuses an
    /// include of it that is not `optional`, and skips one that is.
    #[test]
    fn a_required_include_inside_the_directory_is_an_entry_and_an_optional_one_is_not()
    -> TestOutcome {
        let scratch = ScratchDir::new("cargo-config-include-inside")?;
        let project = scratch.path();
        write_files(
            project,
            &[
                (
                    ".cargo/config.toml",
                    "include = [\"../tasks/lint/required.toml\", \
                     { path = \"../tasks/lint/optional.toml\", optional = true }]\n",
                ),
                ("tasks/lint/required.toml", "[alias]\n"),
                ("tasks/lint/optional.toml", "paths = [\"tasks/lint\"]\n"),
            ],
        )?;

        let entries = entries_for_lint(project)?;

        assert_eq!(
            entries,
            [format!(
                "`include` of `../tasks/lint/required.toml` in {}",
                project.join(".cargo/config.toml").display()
            )]
        );
        Ok(())
    }

    /// Cargo refuses a file reached twice from one starting file, a diamond
    /// as much as a loop.
    #[test]
    fn a_file_included_twice_from_one_start_fails_as_a_cycle() -> TestOutcome {
        for (name, files) in [
            (
                "loop",
                [
                    (".cargo/config.toml", "include = [\"a.toml\"]\n"),
                    (".cargo/a.toml", "include = [\"config.toml\"]\n"),
                    (".cargo/b.toml", "\n"),
                ],
            ),
            (
                "diamond",
                [
                    (".cargo/config.toml", "include = [\"a.toml\", \"b.toml\"]\n"),
                    (".cargo/a.toml", "include = [\"c.toml\"]\n"),
                    (".cargo/b.toml", "include = [\"c.toml\"]\n"),
                ],
            ),
        ] {
            let scratch = ScratchDir::new(&format!("cargo-config-include-{name}"))?;
            write_files(scratch.path(), &files)?;
            std::fs::write(scratch.path().join(".cargo/c.toml"), "\n")?;

            let failure = entries_for_lint(scratch.path())
                .err()
                .ok_or_else(|| format!("a {name} was meant to fail"))?;
            assert!(failure.to_string().contains("cycle"), "{name}: {failure}");
        }
        Ok(())
    }

    #[test]
    fn an_include_cargo_refuses_is_a_failure_naming_its_file() -> TestOutcome {
        for (name, include) in [
            ("string", "include = \"extra.toml\"\n"),
            ("table", "include = { path = \"extra.toml\" }\n"),
            ("number", "include = [3]\n"),
            (
                "optional",
                "include = [{ path = \"extra.toml\", optional = \"yes\" }]\n",
            ),
            ("extension", "include = [\"extra\"]\n"),
        ] {
            let scratch = ScratchDir::new(&format!("cargo-config-include-refused-{name}"))?;
            write_files(
                scratch.path(),
                &[(".cargo/config.toml", include), (".cargo/extra.toml", "\n")],
            )?;

            let failure = entries_for_lint(scratch.path())
                .err()
                .ok_or_else(|| format!("`{include}` was meant to fail"))?;
            assert!(
                failure.to_string().contains("config.toml")
                    && failure.to_string().contains("Cargo refuses"),
                "{name}: {failure}"
            );
        }
        Ok(())
    }

    /// With both names in one place, Cargo 1.95 reads `config` and ignores
    /// `config.toml`, so a setting only in the ignored one is not an entry.
    #[test]
    fn config_is_read_in_preference_to_config_toml_and_alone() -> TestOutcome {
        let scratch = ScratchDir::new("cargo-config-both-names")?;
        let project = scratch.path();
        write_files(
            project,
            &[
                (".cargo/config", "[alias]\n"),
                (".cargo/config.toml", "paths = [\"tasks/lint\"]\n"),
            ],
        )?;
        assert!(entries_for_lint(project)?.is_empty());

        write_files(
            project,
            &[(".cargo/config", "paths = [\"tasks/lint/x\"]\n")],
        )?;
        assert_eq!(entries_for_lint(project)?.len(), 1);
        Ok(())
    }

    /// A starting directory inside the directory, such as the task's own
    /// member directory, has its file deleted with it, so it is not read.
    #[test]
    fn a_file_inside_the_directory_is_not_read() -> TestOutcome {
        let scratch = ScratchDir::new("cargo-config-inside")?;
        let project = scratch.path();
        write_files(
            project,
            &[
                (".cargo/config.toml", "\n"),
                ("tasks/lint/.cargo/config.toml", "paths = [\".\"]\n"),
            ],
        )?;

        let entries = entries_pointing_under_with(
            &[&project.join("tasks/lint")],
            None,
            &project.join("tasks/lint"),
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
