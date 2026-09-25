//! `cargo xtask verify-tag <tag>`: does the workspace release what the tag
//! names?

use std::error::Error;
use std::fmt;
use std::path::Path;

use crate::manifest::{self, ManifestError};
use crate::version::{ParseTagError, ReleaseTag, Version};
use crate::workspace::{self, ManifestFileError};

/// Checks that `tag` is a release tag and that the workspace under `root` is
/// at exactly its version, and returns that tag.
pub(crate) fn run(root: &Path, tag: &str) -> Result<ReleaseTag, VerifyError> {
    let tag = ReleaseTag::parse(tag).map_err(VerifyError::Tag)?;
    let document = workspace::read_manifest(root).map_err(VerifyError::ManifestFile)?;
    let workspace = manifest::workspace_version(&document).map_err(VerifyError::Manifest)?;
    if workspace == tag.version() {
        Ok(tag)
    } else {
        Err(VerifyError::Mismatch { tag, workspace })
    }
}

/// Why a tag does not match the workspace.
#[derive(Debug)]
pub(crate) enum VerifyError {
    Tag(ParseTagError),
    ManifestFile(ManifestFileError),
    Manifest(ManifestError),
    Mismatch { tag: ReleaseTag, workspace: Version },
}

impl fmt::Display for VerifyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(error) => write!(formatter, "{error}"),
            Self::ManifestFile(error) => write!(formatter, "{error}"),
            Self::Manifest(error) => write!(formatter, "{error}"),
            Self::Mismatch { tag, workspace } => write!(
                formatter,
                "the tag is {tag} but the workspace version is {workspace}"
            ),
        }
    }
}

impl Error for VerifyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Tag(error) => Some(error),
            Self::ManifestFile(error) => Some(error),
            Self::Manifest(error) => Some(error),
            Self::Mismatch { .. } => None,
        }
    }
}
