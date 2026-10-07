//! The two files every task crate `create` scaffolds is made of.
//!
//! Inside a project and on its own, a new task crate is the same two files, so
//! this is the one place they are written and the one place their error
//! wording lives; each caller reports what was written in its own way.
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

use std::path::Path;

use rituals::{Failure, Name};
use rituals_compose::source::Source;
use rituals_compose::task_crate::{self, Audience};

/// The crate's manifest, relative to the crate's directory.
const MANIFEST_FILE: &str = "Cargo.toml";

/// The crate's library, relative to the crate's directory.
const LIBRARY_FILE: &str = "src/lib.rs";

/// Writes `Cargo.toml` and `src/lib.rs` into `directory` for the task crate
/// `name`, taking `rituals` from `source` and saying who the ritual is for
/// through `audience`, and returns the two files in the order written, each
/// relative to `directory`.
///
/// `directory` may or may not exist yet; `src/` is created inside it. Stops
/// at the first failure and leaves what was written, because the caller's
/// rollback removes the whole directory rather than this function undoing
/// file by file.
///
/// # Errors
///
/// Returns a [`Failure`] naming the path that could not be created or
/// written.
pub(crate) fn write(
    directory: &Path,
    name: &Name,
    source: &Source,
    audience: Audience,
) -> Result<[&'static str; 2], Failure> {
    let source_directory = directory.join("src");
    std::fs::create_dir_all(&source_directory).map_err(|error| {
        Failure::new(format!("creating {} failed", source_directory.display())).caused_by(error)
    })?;

    let manifest_path = directory.join(MANIFEST_FILE);
    std::fs::write(&manifest_path, task_crate::manifest(name, source, audience)).map_err(
        |error| {
            Failure::new(format!("writing {} failed", manifest_path.display())).caused_by(error)
        },
    )?;

    let library_path = directory.join(LIBRARY_FILE);
    std::fs::write(&library_path, task_crate::lib(name)).map_err(|error| {
        Failure::new(format!("writing {} failed", library_path.display())).caused_by(error)
    })?;

    Ok([MANIFEST_FILE, LIBRARY_FILE])
}

#[cfg(test)]
mod tests {
    use rituals::Name;
    use rituals_compose::source::Source;
    use rituals_compose::task_crate::Audience;

    use super::write;
    use crate::test_support::{ScratchDir, TestOutcome};

    fn demo_name() -> Name {
        Name::new("demo").expect("demo is a valid name")
    }

    /// Writing into a directory that does not exist yet makes it, with `src/`
    /// inside, and reports the two files relative to it, manifest first.
    #[test]
    fn the_two_files_are_written_and_named_relative_to_the_directory() -> TestOutcome {
        let scratch = ScratchDir::new("crate-files")?;
        let directory = scratch.path().join("demo");

        let written = write(
            &directory,
            &demo_name(),
            &Source::Inherited,
            Audience::Private,
        )?;

        assert_eq!(written, ["Cargo.toml", "src/lib.rs"]);
        for file in written {
            assert!(directory.join(file).is_file(), "{file} must exist");
        }
        let manifest = std::fs::read_to_string(directory.join("Cargo.toml"))?;
        assert!(manifest.contains("name = \"demo\""), "{manifest}");
        Ok(())
    }

    /// The manifest says who the ritual is for: a private one cannot be
    /// published, a public one can, and where `rituals` comes from follows
    /// `source`.
    #[test]
    fn the_manifest_says_who_the_ritual_is_for_and_where_rituals_comes_from() -> TestOutcome {
        for (audience, source, private, dependency) in [
            (
                Audience::Private,
                Source::Inherited,
                true,
                "rituals.workspace = true",
            ),
            (Audience::Public, Source::Registry, false, "rituals = "),
        ] {
            let scratch = ScratchDir::new("crate-files-audience")?;
            let directory = scratch.path().join("demo");

            write(&directory, &demo_name(), &source, audience)?;

            let manifest = std::fs::read_to_string(directory.join("Cargo.toml"))?;
            assert_eq!(
                manifest.contains("publish = false\n"),
                private,
                "{manifest}"
            );
            assert!(manifest.contains(dependency), "{manifest}");
        }
        Ok(())
    }

    /// A directory sitting where `Cargo.toml` should go fails the first file
    /// write with `EISDIR`; the failure names that path and nothing after it
    /// is written.
    #[test]
    fn a_failed_write_names_the_path_and_writes_nothing_after_it() -> TestOutcome {
        let scratch = ScratchDir::new("crate-files-failure")?;
        let directory = scratch.path().join("demo");
        std::fs::create_dir_all(directory.join("Cargo.toml"))?;

        let failure = write(
            &directory,
            &demo_name(),
            &Source::Inherited,
            Audience::Private,
        )
        .err()
        .ok_or("expected the poisoned manifest path to fail the write")?;

        assert!(
            failure
                .with_causes()
                .to_string()
                .contains(&directory.join("Cargo.toml").display().to_string()),
            "{failure}"
        );
        assert!(!directory.join("src/lib.rs").exists());
        Ok(())
    }
}
