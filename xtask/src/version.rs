//! A release version, and the tag that names one.
//!
//! Only a full `MAJOR.MINOR.PATCH` release is accepted: no pre-release and no
//! build metadata. `new` writes `rituals::VERSION` into every project it
//! scaffolds as the lowest release that project will accept, and
//! `registry_is_the_default_source.rs` holds that version to a full release,
//! so a pre-release here would ship a binary whose own tests refuse it.

use std::error::Error;
use std::fmt;

/// A release version: three numbers and nothing else.
///
/// The derived ordering compares `major`, then `minor`, then `patch`, which is
/// exactly `SemVer` precedence for versions without a pre-release.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    /// Parses `MAJOR.MINOR.PATCH`, each part a decimal number with no leading
    /// zero, as `SemVer` requires.
    pub(crate) fn parse(text: &str) -> Result<Self, ParseVersionError> {
        let refuse = |reason| ParseVersionError {
            input: text.to_string(),
            reason,
        };
        if text.contains(['-', '+']) {
            return Err(refuse(VersionFault::PreReleaseOrBuild));
        }
        let parts: Vec<&str> = text.split('.').collect();
        let [major, minor, patch] = parts.as_slice() else {
            return Err(refuse(VersionFault::NotThreeParts));
        };
        Ok(Self {
            major: parse_part(major).map_err(refuse)?,
            minor: parse_part(minor).map_err(refuse)?,
            patch: parse_part(patch).map_err(refuse)?,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// One numeric part of a version.
fn parse_part(part: &str) -> Result<u64, VersionFault> {
    if part.is_empty() {
        return Err(VersionFault::EmptyPart);
    }
    if !part.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(VersionFault::NotDigits);
    }
    if part.len() > 1 && part.starts_with('0') {
        return Err(VersionFault::LeadingZero);
    }
    // Every byte is a digit, so the only way left to fail is overflow.
    part.parse().map_err(|_| VersionFault::TooLarge)
}

/// Why a string is not a release version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VersionFault {
    PreReleaseOrBuild,
    NotThreeParts,
    EmptyPart,
    NotDigits,
    LeadingZero,
    TooLarge,
}

impl fmt::Display for VersionFault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PreReleaseOrBuild => {
                "pre-release and build metadata are not released: `new` writes this \
                 version into every project as the lowest release it accepts, so it \
                 must be a full MAJOR.MINOR.PATCH"
            }
            Self::NotThreeParts => "it must have exactly three parts, MAJOR.MINOR.PATCH",
            Self::EmptyPart => "a part is empty",
            Self::NotDigits => "a part is not a decimal number",
            Self::LeadingZero => "a part has a leading zero, which SemVer forbids",
            Self::TooLarge => "a part is too large",
        })
    }
}

/// A string that is not a release version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParseVersionError {
    input: String,
    reason: VersionFault,
}

impl fmt::Display for ParseVersionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "`{}` is not a release version: {}",
            self.input, self.reason
        )
    }
}

impl Error for ParseVersionError {}

/// A release tag: `v` followed by a [`Version`], such as `v0.1.1`.
///
/// The tag names the GitHub release, the release branch and the commit; the
/// version inside it is what every manifest carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ReleaseTag {
    version: Version,
}

impl ReleaseTag {
    /// Parses `v` followed by a release version.
    pub(crate) fn parse(text: &str) -> Result<Self, ParseTagError> {
        let refuse = |reason| ParseTagError {
            input: text.to_string(),
            reason,
        };
        let Some(version) = text.strip_prefix('v') else {
            return Err(refuse(TagFault::MissingPrefix));
        };
        Version::parse(version)
            .map(|version| Self { version })
            .map_err(|error| refuse(TagFault::Version(error.reason)))
    }

    /// The version this tag releases.
    pub(crate) const fn version(self) -> Version {
        self.version
    }
}

impl fmt::Display for ReleaseTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "v{}", self.version)
    }
}

/// Why a string is not a release tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TagFault {
    MissingPrefix,
    Version(VersionFault),
}

/// A string that is not a release tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParseTagError {
    input: String,
    reason: TagFault,
}

impl fmt::Display for ParseTagError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "`{}` is not a release tag: ", self.input)?;
        match self.reason {
            TagFault::MissingPrefix => formatter.write_str("it must start with `v`")?,
            TagFault::Version(fault) => write!(formatter, "{fault}")?,
        }
        formatter.write_str("; a release tag is `v` then MAJOR.MINOR.PATCH, such as v0.1.1")
    }
}

impl Error for ParseTagError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn a_full_release_tag_parses_to_its_version() {
        let tag = ReleaseTag::parse("v0.1.1").expect("v0.1.1 is a release tag");
        assert_eq!(tag.version(), version(0, 1, 1));
        assert_eq!(tag.to_string(), "v0.1.1");
    }

    #[test]
    fn zero_parts_and_large_parts_parse() {
        assert_eq!(Version::parse("0.0.0"), Ok(version(0, 0, 0)));
        assert_eq!(Version::parse("10.20.30"), Ok(version(10, 20, 30)));
        assert_eq!(
            Version::parse("18446744073709551615.0.0"),
            Ok(version(u64::MAX, 0, 0))
        );
    }

    fn fault_of_tag(text: &str) -> TagFault {
        ReleaseTag::parse(text)
            .expect_err(&format!("{text:?} must be refused"))
            .reason
    }

    #[test]
    fn every_malformed_tag_is_refused_for_its_own_reason() {
        let cases = [
            ("0.1.1", TagFault::MissingPrefix),
            ("V0.1.1", TagFault::MissingPrefix),
            ("", TagFault::MissingPrefix),
            ("v", TagFault::Version(VersionFault::NotThreeParts)),
            ("v0.1", TagFault::Version(VersionFault::NotThreeParts)),
            ("v0.1.1.0", TagFault::Version(VersionFault::NotThreeParts)),
            ("v0..1", TagFault::Version(VersionFault::EmptyPart)),
            ("v0.1.", TagFault::Version(VersionFault::EmptyPart)),
            (
                "v0.1.1-garbage",
                TagFault::Version(VersionFault::PreReleaseOrBuild),
            ),
            (
                "v0.1.1-rc.1",
                TagFault::Version(VersionFault::PreReleaseOrBuild),
            ),
            (
                "v0.1.1+build",
                TagFault::Version(VersionFault::PreReleaseOrBuild),
            ),
            ("v0.1.x", TagFault::Version(VersionFault::NotDigits)),
            ("v0.1. 1", TagFault::Version(VersionFault::NotDigits)),
            ("v0.1.1 ", TagFault::Version(VersionFault::NotDigits)),
            ("vv0.1.1", TagFault::Version(VersionFault::NotDigits)),
            ("v0.01.1", TagFault::Version(VersionFault::LeadingZero)),
            ("v00.1.1", TagFault::Version(VersionFault::LeadingZero)),
            (
                "v18446744073709551616.0.0",
                TagFault::Version(VersionFault::TooLarge),
            ),
        ];
        for (text, fault) in cases {
            assert_eq!(fault_of_tag(text), fault, "for {text:?}");
        }
    }

    #[test]
    fn non_ascii_digits_are_not_digits() {
        // `char::is_numeric` would accept these; SemVer does not.
        assert_eq!(
            fault_of_tag("v0.1.\u{0661}"),
            TagFault::Version(VersionFault::NotDigits)
        );
    }

    #[test]
    fn the_refusal_names_the_input_and_the_expected_shape() {
        let message = ReleaseTag::parse("0.1.1")
            .expect_err("a tag without `v` is refused")
            .to_string();
        assert_eq!(
            message,
            "`0.1.1` is not a release tag: it must start with `v`; \
             a release tag is `v` then MAJOR.MINOR.PATCH, such as v0.1.1"
        );
    }

    #[test]
    fn ordering_is_semver_precedence() {
        let ascending = [
            version(0, 0, 9),
            version(0, 1, 0),
            version(0, 1, 1),
            version(0, 1, 10),
            version(0, 2, 0),
            version(1, 0, 0),
        ];
        for pair in ascending.windows(2) {
            assert!(pair[0] < pair[1], "{} < {}", pair[0], pair[1]);
        }
    }
}
