//! Where the workspace is, and reading and writing its root manifest.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

/// The root of the workspace this task was built from: the directory above
/// its own manifest.
pub(crate) fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// The root manifest's path under `root`.
fn manifest_path(root: &Path) -> PathBuf {
    root.join("Cargo.toml")
}

/// Reads and parses the root manifest.
pub(crate) fn read_manifest(root: &Path) -> Result<DocumentMut, ManifestFileError> {
    let path = manifest_path(root);
    let text = std::fs::read_to_string(&path).map_err(|error| ManifestFileError {
        path: path.clone(),
        fault: ManifestFileFault::Read(error),
    })?;
    text.parse().map_err(|error| ManifestFileError {
        path,
        fault: ManifestFileFault::Parse(error),
    })
}

/// Writes `document` back as the root manifest.
pub(crate) fn write_manifest(root: &Path, document: &DocumentMut) -> Result<(), ManifestFileError> {
    let path = manifest_path(root);
    std::fs::write(&path, document.to_string()).map_err(|error| ManifestFileError {
        path,
        fault: ManifestFileFault::Write(error),
    })
}

/// The root manifest and the lockfile, byte for byte, as they were when
/// taken, so an edit that fails partway can put both back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    manifest: Vec<u8>,
    lockfile: Vec<u8>,
}

impl Snapshot {
    /// Reads both files under `root`.
    pub(crate) fn take(root: &Path) -> Result<Self, std::io::Error> {
        Ok(Self {
            manifest: std::fs::read(manifest_path(root))?,
            lockfile: std::fs::read(lockfile_path(root))?,
        })
    }

    /// Writes both files under `root` back as they were.
    pub(crate) fn restore(&self, root: &Path) -> Result<(), std::io::Error> {
        std::fs::write(manifest_path(root), &self.manifest)?;
        std::fs::write(lockfile_path(root), &self.lockfile)
    }
}

/// The lockfile's path under `root`.
fn lockfile_path(root: &Path) -> PathBuf {
    root.join("Cargo.lock")
}

/// The root manifest could not be read, parsed or written.
#[derive(Debug)]
pub(crate) struct ManifestFileError {
    path: PathBuf,
    fault: ManifestFileFault,
}

#[derive(Debug)]
enum ManifestFileFault {
    Read(std::io::Error),
    Parse(toml_edit::TomlError),
    Write(std::io::Error),
}

impl fmt::Display for ManifestFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path.display();
        match &self.fault {
            ManifestFileFault::Read(error) => write!(formatter, "reading {path} failed: {error}"),
            ManifestFileFault::Parse(error) => {
                write!(formatter, "parsing {path} as TOML failed: {error}")
            }
            ManifestFileFault::Write(error) => write!(formatter, "writing {path} failed: {error}"),
        }
    }
}

impl Error for ManifestFileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.fault {
            ManifestFileFault::Read(error) | ManifestFileFault::Write(error) => Some(error),
            ManifestFileFault::Parse(error) => Some(error),
        }
    }
}
