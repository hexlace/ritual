//! `cargo xtask publish <tag> [--dry-run]`: publish every crate the tag
//! releases, skipping any already on crates.io.
//!
//! Skipping is what makes a failed run safe to run again. `cargo publish
//! --workspace` publishes the members in dependency order and refuses a
//! version crates.io already has, so after a run that published some crates
//! and then failed, a plain re-run would stop at the first of them. This asks
//! crates.io about each crate first and leaves out the ones it has.

use std::error::Error;
use std::fmt;
use std::path::Path;

use serde_json::Value;

use crate::process::{self, CommandError, Program};
use crate::verify::{self, VerifyError};
use crate::version::ReleaseTag;

/// Whether to publish for real or only package and verify.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Publish,
    DryRun,
}

/// One workspace member, by the name and version it publishes under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Member {
    name: String,
    version: String,
}

impl Member {
    /// The package name the member publishes under.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// The package ID spec for exactly this release, as `cargo info` takes it.
    fn spec(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }
}

/// Which members this run publishes and which it leaves out, and why.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) to_publish: Vec<Member>,
    pub(crate) already_published: Vec<Member>,
    pub(crate) never_published: Vec<Member>,
}

impl Plan {
    /// Every member this run leaves out, for `--exclude`.
    fn excluded(&self) -> impl Iterator<Item = &Member> {
        self.already_published.iter().chain(&self.never_published)
    }
}

/// Publishes every member the workspace under `root` releases at `tag`.
///
/// The tag is checked against the workspace version first, so a release
/// whose tag and manifests disagree publishes nothing. What happened is
/// returned whether or not anything was left to publish.
pub(crate) fn run(root: &Path, tag: &str, mode: Mode) -> Result<Plan, PublishError> {
    let tag: ReleaseTag = verify::run(root, tag).map_err(PublishError::Verify)?;
    let metadata = process::query(
        Program::Cargo,
        root,
        &["metadata", "--no-deps", "--locked", "--format-version", "1"],
    )
    .map_err(PublishError::Cargo)?;
    let metadata: Value =
        serde_json::from_str(&metadata).map_err(|_| PublishError::Metadata("invalid JSON"))?;
    let members = members(&metadata)?;
    if let Some(member) = members
        .iter()
        .map(|(member, _)| member)
        .find(|member| member.version != tag.version().to_string())
    {
        return Err(PublishError::MemberOffTag {
            member: member.clone(),
            tag,
        });
    }
    let plan = plan(members, |member| {
        // `name@version` names exactly that release, and `cargo info` fails
        // when the registry has no such version. It also fails when crates.io
        // cannot be reached, and for a version that was published and then
        // yanked. Both count as "not published", so the worst this can do is
        // include a crate that `cargo publish` then refuses as a duplicate.
        // The reverse — skipping one that is not there — cannot happen,
        // since success means crates.io answered with it.
        //
        // A dry run does not refuse a duplicate: `cargo publish --dry-run`
        // only warns that the version exists. So a dry run checks that every
        // crate packages and builds, not what crates.io already has.
        process::succeeds(
            Program::Cargo,
            root,
            &["info", "--quiet", "--registry", "crates-io", &member.spec()],
        )
    })
    .map_err(PublishError::Cargo)?;

    if plan.to_publish.is_empty() {
        return Ok(plan);
    }
    let mut arguments = vec!["publish", "--workspace", "--locked"];
    for member in plan.excluded() {
        arguments.extend(["--exclude", member.name.as_str()]);
    }
    if mode == Mode::DryRun {
        arguments.push("--dry-run");
    }
    process::run(Program::Cargo, root, &arguments).map_err(PublishError::Cargo)?;
    Ok(plan)
}

/// Whether a member may be published to crates.io at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Publishable {
    Yes,
    No,
}

/// Every workspace member `cargo metadata` lists, and whether it may be
/// published.
///
/// `publish` is `null` for a crate that may go to any registry and `[]` for
/// `publish = false`. A list of named registries is refused: nothing here
/// declares one, and publishing such a crate to crates.io would be wrong.
pub(crate) fn members(metadata: &Value) -> Result<Vec<(Member, Publishable)>, PublishError> {
    let packages = metadata["packages"]
        .as_array()
        .ok_or(PublishError::Metadata("no packages array"))?;
    let workspace_members = metadata["workspace_members"]
        .as_array()
        .ok_or(PublishError::Metadata("no workspace_members array"))?;
    packages
        .iter()
        .filter(|package| workspace_members.contains(&package["id"]))
        .map(|package| {
            let name = package["name"]
                .as_str()
                .ok_or(PublishError::Metadata("a package has no name"))?;
            let version = package["version"]
                .as_str()
                .ok_or(PublishError::Metadata("a package has no version"))?;
            let publishable = match &package["publish"] {
                Value::Null => Publishable::Yes,
                Value::Array(registries) if registries.is_empty() => Publishable::No,
                _ => return Err(PublishError::NamedRegistries(name.to_string())),
            };
            let member = Member {
                name: name.to_string(),
                version: version.to_string(),
            };
            Ok((member, publishable))
        })
        .collect()
}

/// Sorts `members` into what to publish and what to leave out, asking
/// `is_published` only about the members that may be published.
pub(crate) fn plan<E>(
    members: Vec<(Member, Publishable)>,
    mut is_published: impl FnMut(&Member) -> Result<bool, E>,
) -> Result<Plan, E> {
    let mut plan = Plan::default();
    for (member, publishable) in members {
        match publishable {
            Publishable::No => plan.never_published.push(member),
            Publishable::Yes if is_published(&member)? => plan.already_published.push(member),
            Publishable::Yes => plan.to_publish.push(member),
        }
    }
    Ok(plan)
}

/// Why publishing stopped.
#[derive(Debug)]
pub(crate) enum PublishError {
    Verify(VerifyError),
    Cargo(CommandError),
    Metadata(&'static str),
    NamedRegistries(String),
    MemberOffTag { member: Member, tag: ReleaseTag },
}

impl fmt::Display for PublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Verify(error) => write!(formatter, "{error}"),
            Self::Cargo(error) => write!(formatter, "{error}"),
            Self::Metadata(reason) => write!(formatter, "cargo metadata: {reason}"),
            Self::NamedRegistries(name) => write!(
                formatter,
                "{name} restricts `publish` to named registries, which this does not handle"
            ),
            Self::MemberOffTag { member, tag } => write!(
                formatter,
                "{} is at {}, not the tag's version {}",
                member.name,
                member.version,
                tag.version()
            ),
        }
    }
}

impl Error for PublishError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Verify(error) => Some(error),
            Self::Cargo(error) => Some(error),
            Self::Metadata(_) | Self::NamedRegistries(_) | Self::MemberOffTag { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;

    /// This workspace's real `cargo metadata`, so the tests read what Cargo
    /// prints rather than a sketch of it.
    fn this_workspace() -> Value {
        let root = crate::workspace::root();
        let output = process::query(
            Program::Cargo,
            &root,
            &["metadata", "--no-deps", "--format-version", "1"],
        )
        .expect("cargo metadata runs on this workspace");
        serde_json::from_str(&output).expect("cargo metadata prints JSON")
    }

    fn names(members: &[Member]) -> Vec<&str> {
        let mut names: Vec<&str> = members.iter().map(|member| member.name.as_str()).collect();
        names.sort_unstable();
        names
    }

    const RELEASED: [&str; 8] = [
        "rituals",
        "rituals-cli",
        "rituals-compose",
        "rituals-core",
        "rituals-core-add",
        "rituals-core-create",
        "rituals-core-new",
        "rituals-core-regenerate",
    ];

    #[test]
    fn this_workspace_releases_eight_crates_and_never_publishes_xtask() {
        let members = members(&this_workspace()).expect("the metadata has the expected shape");
        let plan = plan(members, |_| Ok::<_, Infallible>(false)).expect("infallible");
        assert_eq!(names(&plan.to_publish), RELEASED);
        assert_eq!(names(&plan.never_published), ["xtask"]);
        assert!(plan.already_published.is_empty());
    }

    #[test]
    fn crates_already_on_the_registry_are_left_out_and_nothing_else_is() {
        let members = members(&this_workspace()).expect("the metadata has the expected shape");
        let mut asked = Vec::new();
        let plan = plan(members, |member| {
            asked.push(member.name.clone());
            Ok::<_, Infallible>(member.name == "rituals" || member.name == "rituals-core-new")
        })
        .expect("infallible");
        assert_eq!(
            names(&plan.already_published),
            ["rituals", "rituals-core-new"]
        );
        assert_eq!(plan.to_publish.len(), RELEASED.len() - 2);
        assert!(
            !asked.iter().any(|name| name == "xtask"),
            "an unpublishable member is never looked up"
        );
        let excluded: Vec<Member> = plan.excluded().cloned().collect();
        assert_eq!(
            names(&excluded),
            ["rituals", "rituals-core-new", "xtask"],
            "every member left out is excluded from `cargo publish --workspace`"
        );
    }

    #[test]
    fn every_crate_already_published_leaves_nothing_to_publish() {
        let members = members(&this_workspace()).expect("the metadata has the expected shape");
        let plan = plan(members, |_| Ok::<_, Infallible>(true)).expect("infallible");
        assert!(plan.to_publish.is_empty());
        assert_eq!(names(&plan.already_published), RELEASED);
    }

    #[test]
    fn a_failed_lookup_stops_the_plan() {
        let members = members(&this_workspace()).expect("the metadata has the expected shape");
        assert_eq!(plan(members, |_| Err("offline")), Err("offline"));
    }

    #[test]
    fn a_crate_restricted_to_named_registries_is_refused() {
        let mut metadata = this_workspace();
        metadata["packages"][0]["publish"] = serde_json::json!(["somewhere-else"]);
        assert!(matches!(
            members(&metadata),
            Err(PublishError::NamedRegistries(_))
        ));
    }

    #[test]
    fn packages_outside_the_workspace_are_not_members() {
        let mut metadata = this_workspace();
        metadata["workspace_members"] = serde_json::json!([]);
        assert_eq!(members(&metadata).expect("the shape is valid").len(), 0);
    }
}
