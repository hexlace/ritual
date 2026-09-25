//! `cargo xtask publish-plan <tag>`: work out which crates a publish of
//! `<tag>` uploads, dry-run that publish, and print the plan.
//!
//! The upload itself is not done here, and nothing here ever holds a
//! credential. A dry run compiles every crate it would upload, and each build
//! script and proc macro compiled along the way runs with the environment it
//! was started in. So `.github/workflows/publish.yml` runs this in a job that
//! has no token and cannot mint one, and the job that can then runs
//! `cargo publish --no-verify` with the plan's exclusions, which compiles
//! nothing.
//!
//! Leaving out what crates.io already has is what makes a failed publish safe
//! to run again. `cargo publish --workspace` publishes the members in
//! dependency order and refuses a version crates.io already has, so after a
//! run that published some crates and then failed, a plain re-run would stop
//! at the first of them. This asks crates.io about each crate first and plans
//! to leave out the ones it has.

use std::error::Error;
use std::fmt;
use std::path::Path;

use serde_json::Value;

use crate::process::{self, CommandError, Program};
use crate::verify::{self, VerifyError};
use crate::version::ReleaseTag;

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

    /// The plan as the publish job reads it: a `publish=` line naming the
    /// members to upload and an `exclude=` line naming the members to pass
    /// to `cargo publish --workspace` as `--exclude`, each a space-separated
    /// list. Both lines are always present, and a list may be empty.
    ///
    /// The lines are in the `name=value` form a workflow step appends to
    /// `$GITHUB_OUTPUT`. The job that reads them checks every name against
    /// the crate-name grammar itself rather than trusting this, because the
    /// job that writes them has run third-party build scripts by then.
    pub(crate) fn outputs(&self) -> String {
        format!(
            "publish={}\nexclude={}\n",
            spaced(self.to_publish.iter()),
            spaced(self.excluded()),
        )
    }
}

/// The members' names, separated by single spaces.
fn spaced<'a>(members: impl Iterator<Item = &'a Member>) -> String {
    members
        .map(|member| member.name.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Plans the publish of every member the workspace under `root` releases at
/// `tag`, and dry-runs it.
///
/// The tag is checked against the workspace version first, so a release
/// whose tag and manifests disagree plans nothing. The dry run packages and
/// compiles every member the plan uploads, exactly as `cargo publish` would
/// verify them, and is skipped when there is nothing to upload. The plan is
/// returned whether or not anything was left to publish.
pub(crate) fn run(root: &Path, tag: &str) -> Result<Plan, PublishError> {
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
        // The dry run below does not refuse a duplicate: `cargo publish
        // --dry-run` only warns that the version exists. So it checks that
        // every crate packages and builds, not what crates.io already has.
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
    let mut arguments = vec!["publish", "--workspace", "--locked", "--dry-run"];
    for member in plan.excluded() {
        arguments.extend(["--exclude", member.name.as_str()]);
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

    /// Reads `outputs` back the way the publish job does: two lines, each
    /// `key=` then names separated by single spaces.
    fn read_outputs(outputs: &str) -> [(String, Vec<String>); 2] {
        let lines: Vec<&str> = outputs.split_terminator('\n').collect();
        assert_eq!(lines.len(), 2, "exactly two lines: {outputs:?}");
        [lines[0], lines[1]].map(|line| {
            let (key, value) = line.split_once('=').expect("each line is key=value");
            let mut names: Vec<String> = value.split(' ').map(str::to_string).collect();
            names.retain(|name| !name.is_empty());
            names.sort_unstable();
            (key.to_string(), names)
        })
    }

    #[test]
    fn the_outputs_name_what_to_upload_and_what_to_exclude() {
        let members = members(&this_workspace()).expect("the metadata has the expected shape");
        let plan = plan(members, |member| {
            Ok::<_, Infallible>(member.name != "rituals-cli" && member.name != "rituals-core-new")
        })
        .expect("infallible");
        let outputs = plan.outputs();
        assert!(
            !outputs.contains("  "),
            "one space between names: {outputs:?}"
        );
        let [(publish, to_publish), (exclude, excluded)] = read_outputs(&outputs);
        assert_eq!((publish.as_str(), exclude.as_str()), ("publish", "exclude"));
        assert_eq!(to_publish, ["rituals-cli", "rituals-core-new"]);
        assert_eq!(
            excluded,
            [
                "rituals",
                "rituals-compose",
                "rituals-core",
                "rituals-core-add",
                "rituals-core-create",
                "rituals-core-regenerate",
                "xtask",
            ]
        );
    }

    #[test]
    fn nothing_to_upload_still_writes_both_lines() {
        let members = members(&this_workspace()).expect("the metadata has the expected shape");
        let plan = plan(members, |_| Ok::<_, Infallible>(true)).expect("infallible");
        let outputs = plan.outputs();
        assert!(outputs.starts_with("publish=\n"), "{outputs:?}");
        let [_, (_, excluded)] = read_outputs(&outputs);
        assert_eq!(
            excluded.len(),
            RELEASED.len() + 1,
            "every member, xtask included"
        );
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
