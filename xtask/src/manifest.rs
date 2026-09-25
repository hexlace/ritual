//! The release version as the root manifest records it.
//!
//! It is written in two kinds of place: `[workspace.package] version`, which
//! every member inherits, and the `version` requirement on every
//! `[workspace.dependencies]` entry that has a `path` — ritual's own crates,
//! which a published manifest can only find by version.
//! `every_crate_releases_at_the_workspace_version.rs` checks that the two
//! agree; this module is what keeps them agreeing when the version moves.

use std::error::Error;
use std::fmt;

use toml_edit::{DocumentMut, Item, TableLike, Value};

use crate::version::{ParseVersionError, Version};

/// The version in `[workspace.package]`.
pub(crate) fn workspace_version(document: &DocumentMut) -> Result<Version, ManifestError> {
    let text = document
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("version"))
        .ok_or(ManifestError::MissingWorkspaceVersion)?
        .as_str()
        .ok_or(ManifestError::WorkspaceVersionNotAString)?;
    Version::parse(text).map_err(ManifestError::WorkspaceVersionNotARelease)
}

/// Writes `version` to `[workspace.package]` and to every internal requirement
/// in `[workspace.dependencies]`, and returns the names of the internal
/// dependencies it rewrote.
///
/// Only the version strings change: comments, key order and spacing around
/// each value stay as they were. Nothing is written unless every site can be:
/// an internal dependency without a `version` is refused rather than given
/// one, since its absence means the manifest has stopped looking the way this
/// code assumes.
pub(crate) fn set_release_version(
    document: &mut DocumentMut,
    version: Version,
) -> Result<Vec<String>, ManifestError> {
    let internal = internal_dependency_names(document)?;
    if internal.is_empty() {
        return Err(ManifestError::NoInternalDependencies);
    }
    let text = version.to_string();

    let package_version = document
        .get_mut("workspace")
        .and_then(|workspace| workspace.get_mut("package"))
        .and_then(|package| package.get_mut("version"))
        .ok_or(ManifestError::MissingWorkspaceVersion)?;
    replace_string(package_version, &text).ok_or(ManifestError::WorkspaceVersionNotAString)?;

    let dependencies = workspace_dependencies_mut(document)?;
    for name in &internal {
        let requirement = dependencies
            .get_mut(name)
            .and_then(|entry| entry.as_table_like_mut())
            .and_then(|entry| entry.get_mut("version"))
            .ok_or_else(|| ManifestError::InternalWithoutVersion(name.clone()))?;
        replace_string(requirement, &text)
            .ok_or_else(|| ManifestError::InternalWithoutVersion(name.clone()))?;
    }
    Ok(internal)
}

/// The keys of every `[workspace.dependencies]` entry with a `path`, each
/// checked to carry a string `version` before anything is edited.
fn internal_dependency_names(document: &DocumentMut) -> Result<Vec<String>, ManifestError> {
    let dependencies = document
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(Item::as_table_like)
        .ok_or(ManifestError::MissingWorkspaceDependencies)?;
    let mut names = Vec::new();
    for (name, entry) in dependencies.iter() {
        let Some(entry) = entry.as_table_like() else {
            continue;
        };
        if entry.get("path").is_none() {
            continue;
        }
        if entry.get("version").and_then(Item::as_str).is_none() {
            return Err(ManifestError::InternalWithoutVersion(name.to_string()));
        }
        names.push(name.to_string());
    }
    Ok(names)
}

fn workspace_dependencies_mut(
    document: &mut DocumentMut,
) -> Result<&mut dyn TableLike, ManifestError> {
    document
        .get_mut("workspace")
        .and_then(|workspace| workspace.get_mut("dependencies"))
        .and_then(Item::as_table_like_mut)
        .ok_or(ManifestError::MissingWorkspaceDependencies)
}

/// Replaces the string at `item` with `text`, keeping the whitespace and
/// comments around it. `None` if `item` is not a string.
fn replace_string(item: &mut Item, text: &str) -> Option<()> {
    let value = item.as_value_mut()?;
    value.as_str()?;
    let decor = value.decor().clone();
    *value = Value::from(text);
    *value.decor_mut() = decor;
    Some(())
}

/// The root manifest does not have the shape a release version needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ManifestError {
    MissingWorkspaceVersion,
    WorkspaceVersionNotAString,
    WorkspaceVersionNotARelease(ParseVersionError),
    MissingWorkspaceDependencies,
    NoInternalDependencies,
    InternalWithoutVersion(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingWorkspaceVersion => {
                formatter.write_str("the root manifest declares no [workspace.package] version")
            }
            Self::WorkspaceVersionNotAString => {
                formatter.write_str("[workspace.package] version is not a string")
            }
            Self::WorkspaceVersionNotARelease(error) => {
                write!(formatter, "[workspace.package] version: {error}")
            }
            Self::MissingWorkspaceDependencies => {
                formatter.write_str("the root manifest declares no [workspace.dependencies]")
            }
            Self::NoInternalDependencies => formatter
                .write_str("[workspace.dependencies] declares none of ritual's own crates by path"),
            Self::InternalWithoutVersion(name) => write!(
                formatter,
                "[workspace.dependencies] {name} has a path but no version string"
            ),
        }
    }
}

impl Error for ManifestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::WorkspaceVersionNotARelease(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This repository's own root manifest, so the tests edit the file the
    /// release workflow edits rather than a sketch of it.
    const ROOT_MANIFEST: &str = include_str!("../../Cargo.toml");

    fn parse(text: &str) -> DocumentMut {
        text.parse().expect("the fixture is valid TOML")
    }

    fn release(text: &str) -> Version {
        Version::parse(text).expect("the fixture version is a release")
    }

    #[test]
    fn bumping_the_root_manifest_moves_every_site_and_only_those_lines() {
        let mut document = parse(ROOT_MANIFEST);
        let before = workspace_version(&document).expect("the root manifest has a version");
        let after = release("98.76.54");
        assert_ne!(before, after);

        let rewritten = set_release_version(&mut document, after).expect("the bump applies");
        assert!(
            !rewritten.is_empty(),
            "the root manifest names its own crates"
        );
        assert_eq!(workspace_version(&document), Ok(after));

        let edited = document.to_string();
        let old = format!("\"{before}\"");
        let new = format!("\"{after}\"");
        let original_lines: Vec<&str> = ROOT_MANIFEST.lines().collect();
        let edited_lines: Vec<&str> = edited.lines().collect();
        assert_eq!(original_lines.len(), edited_lines.len());
        let mut changed = 0;
        for (original, edited) in original_lines.iter().zip(&edited_lines) {
            if original != edited {
                changed += 1;
                assert_eq!(
                    original.replacen(&old, &new, 1),
                    *edited,
                    "a changed line changed anything but its version"
                );
            }
        }
        // The package version plus one line per internal dependency.
        assert_eq!(changed, 1 + rewritten.len());
    }

    #[test]
    fn a_bumped_manifest_reads_back_and_bumps_again() {
        let mut document = parse(ROOT_MANIFEST);
        set_release_version(&mut document, release("0.2.0")).expect("the first bump applies");
        let mut reparsed = parse(&document.to_string());
        assert_eq!(workspace_version(&reparsed), Ok(release("0.2.0")));
        set_release_version(&mut reparsed, release("0.3.0")).expect("the second bump applies");
        assert_eq!(workspace_version(&reparsed), Ok(release("0.3.0")));
    }

    #[test]
    fn a_dependency_table_written_as_a_table_is_rewritten_too() {
        let mut document = parse(
            "[workspace.package]\nversion = \"0.1.0\"\n\n\
             [workspace.dependencies.rituals]\npath = \"crates/rituals\"\nversion = \"0.1.0\" # kept\n",
        );
        set_release_version(&mut document, release("0.1.1")).expect("the bump applies");
        assert_eq!(
            document.to_string(),
            "[workspace.package]\nversion = \"0.1.1\"\n\n\
             [workspace.dependencies.rituals]\npath = \"crates/rituals\"\nversion = \"0.1.1\" # kept\n",
        );
    }

    #[test]
    fn an_internal_dependency_without_a_version_is_refused_and_nothing_changes() {
        let text = "[workspace.package]\nversion = \"0.1.0\"\n\n[workspace.dependencies]\n\
                    rituals = { path = \"crates/rituals\", version = \"0.1.0\" }\n\
                    ritual = { path = \"crates/rituals-core\" }\n";
        let mut document = parse(text);
        assert_eq!(
            set_release_version(&mut document, release("0.1.1")),
            Err(ManifestError::InternalWithoutVersion("ritual".to_string()))
        );
        assert_eq!(document.to_string(), text);
    }

    #[test]
    fn registry_dependencies_are_left_alone() {
        let text = "[workspace.package]\nversion = \"0.1.0\"\n\n[workspace.dependencies]\n\
                    rituals = { path = \"crates/rituals\", version = \"0.1.0\" }\n\
                    serde_json = { version = \"0.1.0\" }\n\
                    plain = \"0.1.0\"\n";
        let mut document = parse(text);
        let rewritten =
            set_release_version(&mut document, release("0.1.1")).expect("the bump applies");
        assert_eq!(rewritten, ["rituals"]);
        assert!(
            document
                .to_string()
                .contains("serde_json = { version = \"0.1.0\" }\nplain = \"0.1.0\"\n")
        );
    }

    #[test]
    fn a_manifest_without_the_expected_shape_is_refused() {
        let cases = [
            ("", ManifestError::MissingWorkspaceVersion),
            (
                "[workspace.package]\nversion = 1\n",
                ManifestError::WorkspaceVersionNotAString,
            ),
            (
                "[workspace.package]\nversion = \"0.1.0\"\n",
                ManifestError::MissingWorkspaceDependencies,
            ),
            (
                "[workspace.package]\nversion = \"0.1.0\"\n[workspace.dependencies]\nx = \"1\"\n",
                ManifestError::NoInternalDependencies,
            ),
        ];
        for (text, error) in cases {
            let mut document = parse(text);
            let outcome = workspace_version(&document)
                .and_then(|_| set_release_version(&mut document, release("0.1.1")));
            assert_eq!(outcome.map(|_| ()), Err(error), "for {text:?}");
        }
    }

    #[test]
    fn a_workspace_version_that_is_not_a_release_is_refused() {
        let document = parse("[workspace.package]\nversion = \"0.1.0-rc.1\"\n");
        assert!(matches!(
            workspace_version(&document),
            Err(ManifestError::WorkspaceVersionNotARelease(_))
        ));
    }
}
