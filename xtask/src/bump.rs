//! `cargo xtask bump <tag>`: move the whole workspace to a new release.

use std::error::Error;
use std::fmt;
use std::path::Path;

use crate::manifest::{self, ManifestError};
use crate::process::{self, CommandError, Program};
use crate::version::{ParseTagError, ReleaseTag, Version};
use crate::workspace::{self, ManifestFileError, Snapshot};

/// What a bump changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Bumped {
    pub(crate) from: Version,
    pub(crate) to: Version,
    pub(crate) internal_requirements: usize,
}

/// Moves the workspace under `root` to the version `tag` names: the
/// `[workspace.package]` version, every internal requirement, and
/// `Cargo.lock`.
///
/// Refuses a tag that is not a release, or whose version is not strictly
/// greater than the one the workspace is at, before writing anything. If the
/// lockfile cannot be brought into line, the manifest and the lockfile are
/// both put back as they were, so the same tag can simply be tried again.
pub(crate) fn run(root: &Path, tag: &str) -> Result<Bumped, BumpError> {
    apply(root, tag, |root| {
        // `--workspace` moves only the members' own entries in the lockfile;
        // every registry dependency stays at the version it is locked to.
        process::run(Program::Cargo, root, &["update", "--workspace"])?;
        // Proves the lockfile now agrees with the manifests, the way the
        // `package` and `msrv` jobs will insist on it.
        process::query(
            Program::Cargo,
            root,
            &["metadata", "--locked", "--format-version", "1"],
        )
        .map(drop)
    })
}

/// [`run`], with the lockfile update passed in so a test can make it fail.
fn apply(
    root: &Path,
    tag: &str,
    update_lockfile: impl FnOnce(&Path) -> Result<(), CommandError>,
) -> Result<Bumped, BumpError> {
    let tag = ReleaseTag::parse(tag).map_err(BumpError::Tag)?;
    let mut document = workspace::read_manifest(root).map_err(BumpError::ManifestFile)?;
    let from = manifest::workspace_version(&document).map_err(BumpError::Manifest)?;
    let to = tag.version();
    if to <= from {
        return Err(BumpError::NotNewer { from, to });
    }

    let internal = manifest::set_release_version(&mut document, to).map_err(BumpError::Manifest)?;
    let before = Snapshot::take(root).map_err(BumpError::Snapshot)?;
    let written = workspace::write_manifest(root, &document).map_err(BumpError::ManifestFile);
    if let Err(error) = written.and_then(|()| update_lockfile(root).map_err(BumpError::Cargo)) {
        return Err(match before.restore(root) {
            Ok(()) => error,
            Err(restore) => BumpError::NotRestored {
                cause: Box::new(error),
                restore,
            },
        });
    }

    Ok(Bumped {
        from,
        to,
        internal_requirements: internal.len(),
    })
}

/// Why a bump did not happen, or did not finish.
#[derive(Debug)]
pub(crate) enum BumpError {
    Tag(ParseTagError),
    ManifestFile(ManifestFileError),
    Manifest(ManifestError),
    NotNewer {
        from: Version,
        to: Version,
    },
    Snapshot(std::io::Error),
    Cargo(CommandError),
    NotRestored {
        cause: Box<Self>,
        restore: std::io::Error,
    },
}

impl fmt::Display for BumpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(error) => write!(formatter, "{error}"),
            Self::ManifestFile(error) => write!(formatter, "{error}"),
            Self::Manifest(error) => write!(formatter, "{error}"),
            Self::NotNewer { from, to } => write!(
                formatter,
                "v{to} is not newer than the workspace version {from}; \
                 a release must be strictly greater than the one before it"
            ),
            Self::Snapshot(error) => write!(
                formatter,
                "reading Cargo.toml and Cargo.lock before changing them failed: {error}"
            ),
            Self::Cargo(error) => write!(
                formatter,
                "updating the lockfile failed, so Cargo.toml and Cargo.lock are back as \
                 they were: {error}"
            ),
            Self::NotRestored { cause, restore } => write!(
                formatter,
                "{cause}; and putting Cargo.toml and Cargo.lock back failed too ({restore}), \
                 so check them with `git status` before trying again"
            ),
        }
    }
}

impl Error for BumpError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Tag(error) => Some(error),
            Self::ManifestFile(error) => Some(error),
            Self::Manifest(error) => Some(error),
            Self::NotNewer { .. } => None,
            Self::Snapshot(error) => Some(error),
            Self::Cargo(error) => Some(error),
            Self::NotRestored { cause, .. } => Some(cause.as_ref()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// This repository's own root manifest and lockfile.
    const ROOT_MANIFEST: &str = include_str!("../../Cargo.toml");
    const LOCKFILE: &str = include_str!("../../Cargo.lock");

    /// A fresh directory holding copies of the root manifest and lockfile,
    /// named for the test that uses it and this process, so no two tests or
    /// runs share one. Removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(test: &str) -> Self {
            let path = std::env::temp_dir().join(scratch_name(test, std::process::id()));
            drop(std::fs::remove_dir_all(&path));
            std::fs::create_dir_all(&path).expect("the scratch directory is created");
            std::fs::write(path.join("Cargo.toml"), ROOT_MANIFEST).expect("the manifest is copied");
            std::fs::write(path.join("Cargo.lock"), LOCKFILE).expect("the lockfile is copied");
            Self(path)
        }

        fn read(&self, file: &str) -> String {
            std::fs::read_to_string(self.0.join(file)).expect("the file reads")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            drop(std::fs::remove_dir_all(&self.0));
        }
    }

    fn scratch_name(test: &str, process: u32) -> String {
        format!("ritual-xtask-bump-{test}-{process}")
    }

    #[test]
    fn scratch_names_differ_by_test_and_by_process() {
        assert_ne!(scratch_name("a", 1), scratch_name("b", 1));
        assert_ne!(scratch_name("a", 1), scratch_name("a", 2));
    }

    fn a_failed_update(_: &Path) -> Result<(), CommandError> {
        process::run(Program::Cargo, Path::new("."), &["--no-such-flag-exists"])
    }

    #[test]
    fn a_failed_lockfile_update_puts_both_files_back() {
        let scratch = Scratch::new("restores");
        let error = apply(&scratch.0, "v98.0.0", a_failed_update).expect_err("the update fails");
        assert!(matches!(error, BumpError::Cargo(_)), "{error}");
        assert!(error.to_string().contains("back as they were"), "{error}");
        assert_eq!(scratch.read("Cargo.toml"), ROOT_MANIFEST);
        assert_eq!(scratch.read("Cargo.lock"), LOCKFILE);
        // So the same tag goes through once the cause is fixed.
        apply(&scratch.0, "v98.0.0", |_| Ok(())).expect("the retry applies");
    }

    #[test]
    fn a_successful_bump_leaves_the_new_manifest_in_place() {
        let scratch = Scratch::new("applies");
        let bumped = apply(&scratch.0, "v98.0.0", |_| Ok(())).expect("the bump applies");
        assert_eq!(bumped.to.to_string(), "98.0.0");
        assert_ne!(scratch.read("Cargo.toml"), ROOT_MANIFEST);
        assert!(scratch.read("Cargo.toml").contains("version = \"98.0.0\""));
    }

    #[test]
    fn a_refused_tag_writes_nothing() {
        let scratch = Scratch::new("refuses");
        for tag in ["v0.0.0", "0.9.9", "v1.0.0-rc.1"] {
            assert!(apply(&scratch.0, tag, |_| Ok(())).is_err(), "{tag}");
        }
        assert_eq!(scratch.read("Cargo.toml"), ROOT_MANIFEST);
        assert_eq!(scratch.read("Cargo.lock"), LOCKFILE);
    }
}
