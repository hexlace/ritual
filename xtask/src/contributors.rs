//! `cargo xtask release-contributors <owner/repo> <tag> <target> <dir>`: the
//! part of a draft release's body that needs only read access.
//!
//! A draft's body has four parts: a marked place at the top for the release's
//! prose, which a person writes before publishing; GitHub's generated list of
//! every pull request merged since the previous release; and every person who
//! authored or co-authored a commit in that range. This builds the last part,
//! and finds the previous release that bounds the range. release-draft.yml
//! assembles the rest, because GitHub's generate-notes endpoint needs a token
//! that can write, and a job holding one compiles nothing.
//!
//! The contributors are built here rather than taken from GitHub's notes,
//! whose "New Contributors" section names only people whose first pull
//! request is in the release, and names nobody who co-authored a commit
//! without opening the pull request. `Commit.authors` in GitHub's GraphQL API
//! lists a commit's author and every `Co-authored-by` trailer, each resolved
//! to an account where GitHub can match the email.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::Path;

use serde_json::Value;

use crate::process::{self, CommandError, Program};
use crate::version::{ParseTagError, ReleaseTag};

/// The most commit IDs GitHub's GraphQL `nodes` field takes in one query.
const GRAPHQL_NODES_PER_QUERY: usize = 100;

/// What the read-only half of a draft release hands on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Credits {
    /// The release the range starts after, or `None` for a first release.
    pub(crate) previous: Option<ReleaseTag>,
    /// The body's Contributors section, heading included.
    pub(crate) section: String,
}

/// Finds the previous release for `tag`, a release of `target`, in
/// `repository` (`owner/name`), and the Contributors section for the commits
/// since it.
///
/// The range starts at the highest published release below `tag`, or at the
/// first commit when there is none. Every call here reads.
pub(crate) fn run(
    root: &Path,
    repository: &str,
    tag: &str,
    target: &str,
) -> Result<Credits, CreditsError> {
    let tag = ReleaseTag::parse(tag).map_err(CreditsError::Tag)?;
    let published = gh(
        root,
        &[
            "api",
            "--paginate",
            &format!("repos/{repository}/releases"),
            "--jq",
            ".[] | select((.draft or .prerelease) | not) | .tag_name",
        ],
    )?;
    let previous = previous_release(published.lines(), tag);

    let range = previous.map_or_else(
        || format!("repos/{repository}/commits?sha={target}&per_page=100"),
        |previous| format!("repos/{repository}/compare/{previous}...{target}?per_page=100"),
    );
    let jq = if previous.is_some() {
        ".commits[].node_id"
    } else {
        ".[].node_id"
    };
    let commit_ids = gh(root, &["api", "--paginate", &range, "--jq", jq])?;
    let commit_ids: Vec<&str> = commit_ids.lines().filter(|id| !id.is_empty()).collect();

    let mut responses = Vec::new();
    for chunk in commit_ids.chunks(GRAPHQL_NODES_PER_QUERY) {
        let mut arguments = vec![
            "api".to_string(),
            "graphql".to_string(),
            "-f".to_string(),
            format!("query={COMMIT_AUTHORS_QUERY}"),
        ];
        for id in chunk {
            arguments.extend(["-f".to_string(), format!("ids[]={id}")]);
        }
        let response = gh(
            root,
            &arguments.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
        responses.push(
            serde_json::from_str::<Value>(&response)
                .map_err(|_| CreditsError::Response("GraphQL printed invalid JSON"))?,
        );
    }
    let contributors = contributors(&responses)?;

    Ok(Credits {
        previous,
        section: section(previous, &contributors),
    })
}

/// Every author of each commit, co-authors included.
const COMMIT_AUTHORS_QUERY: &str = "query($ids: [ID!]!) { nodes(ids: $ids) { \
     ... on Commit { oid authors(first: 100) { nodes { name user { login } } } } } }";

fn gh(root: &Path, arguments: &[&str]) -> Result<String, CreditsError> {
    process::query(Program::Gh, root, arguments).map_err(CreditsError::Gh)
}

/// The highest release in `published` below `tag`. Tags that are not release
/// tags are ignored, and so is anything at or above `tag`, so a patch
/// published for an older line never becomes the starting point.
pub(crate) fn previous_release<'tags>(
    published: impl IntoIterator<Item = &'tags str>,
    tag: ReleaseTag,
) -> Option<ReleaseTag> {
    published
        .into_iter()
        .filter_map(|text| ReleaseTag::parse(text).ok())
        .filter(|published| *published < tag)
        .max()
}

/// Someone credited on a commit.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Contributor {
    /// A GitHub account, by login.
    Account(String),
    /// A name from a commit whose email matches no GitHub account.
    Unlinked(String),
}

impl Contributor {
    /// Whether this is a bot: GitHub ends every bot's name and login with
    /// `[bot]`, and no person's login can contain brackets.
    fn is_bot(&self) -> bool {
        match self {
            Self::Account(name) | Self::Unlinked(name) => name.ends_with("[bot]"),
        }
    }
}

impl fmt::Display for Contributor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Account(login) => write!(formatter, "@{login}"),
            Self::Unlinked(name) => formatter.write_str(name),
        }
    }
}

/// Every contributor in the GraphQL `responses`, once each, sorted by how
/// they read without regard to case.
///
/// Bots are left out. GitHub names every bot `<name>[bot]`, both as a commit
/// author and as the login it resolves to: `github-actions[bot]`, which
/// authors the release pull request's own commit, arrives with that login.
pub(crate) fn contributors(responses: &[Value]) -> Result<Vec<Contributor>, CreditsError> {
    let mut by_sort_key = BTreeMap::new();
    for response in responses {
        if response.get("errors").is_some() {
            return Err(CreditsError::Response("GraphQL answered with errors"));
        }
        let commits = response["data"]["nodes"]
            .as_array()
            .ok_or(CreditsError::Response("GraphQL answered with no nodes"))?;
        for commit in commits {
            let authors = commit["authors"]["nodes"]
                .as_array()
                .ok_or(CreditsError::Response("a commit has no authors"))?;
            for author in authors {
                let contributor = if let Some(login) = author["user"]["login"].as_str() {
                    Contributor::Account(login.to_string())
                } else {
                    let name = author["name"]
                        .as_str()
                        .ok_or(CreditsError::Response("an author has no name"))?;
                    Contributor::Unlinked(name.to_string())
                };
                if contributor.is_bot() {
                    continue;
                }
                by_sort_key.insert(contributor.to_string().to_lowercase(), contributor);
            }
        }
    }
    Ok(by_sort_key.into_values().collect())
}

/// The body's Contributors section: every contributor since `previous`.
pub(crate) fn section(previous: Option<ReleaseTag>, contributors: &[Contributor]) -> String {
    let since = previous.map_or_else(
        || "in this release".to_string(),
        |previous| format!("since {previous}"),
    );
    let credits = if contributors.is_empty() {
        format!("Nobody has authored a commit {since}.\n")
    } else {
        let list = contributors
            .iter()
            .map(|contributor| format!("* {contributor}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("Everyone who authored or co-authored a commit {since}:\n\n{list}\n")
    };
    format!("## Contributors\n\n{credits}")
}

/// Why the Contributors section could not be written.
#[derive(Debug)]
pub(crate) enum CreditsError {
    Tag(ParseTagError),
    Gh(CommandError),
    Response(&'static str),
}

impl fmt::Display for CreditsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(error) => write!(formatter, "{error}"),
            Self::Gh(error) => write!(formatter, "{error}"),
            Self::Response(reason) => write!(formatter, "GitHub's answer: {reason}"),
        }
    }
}

impl Error for CreditsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Tag(error) => Some(error),
            Self::Gh(error) => Some(error),
            Self::Response(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `COMMIT_AUTHORS_QUERY` answer for 9360276, a commit Bee authored
    /// with a `Co-authored-by` trailer for Delphi, captured the same day.
    const COMMIT_AUTHORS: &str = include_str!("../tests/fixtures/commit-authors.json");

    /// The `COMMIT_AUTHORS_QUERY` answer for two public bot commits, captured
    /// on 2026-09-24: one by Dependabot in actions/checkout, one by GitHub
    /// Actions in version-fox/vfox-cmake. Each bot comes back with a login.
    const BOT_COMMIT_AUTHORS: &str = include_str!("../tests/fixtures/bot-commit-authors.json");

    fn tag(text: &str) -> ReleaseTag {
        ReleaseTag::parse(text).expect("the fixture tag is a release tag")
    }

    fn json(text: &str) -> Value {
        serde_json::from_str(text).expect("the fixture is JSON")
    }

    #[test]
    fn a_co_author_is_a_contributor_alongside_the_author() {
        let contributors = contributors(&[json(COMMIT_AUTHORS)]).expect("the answer parses");
        assert_eq!(
            contributors,
            [
                Contributor::Account("beeauvin".to_string()),
                Contributor::Account("delphimoon".to_string()),
            ]
        );
    }

    #[test]
    fn the_captured_release_credits_its_author_and_co_author() {
        let contributors = contributors(&[json(COMMIT_AUTHORS)]).expect("the answer parses");
        assert_eq!(
            section(Some(tag("v0.1.0")), &contributors),
            "## Contributors\n\n\
             Everyone who authored or co-authored a commit since v0.1.0:\n\n\
             * @beeauvin\n* @delphimoon\n"
        );
    }

    fn author(name: &str, login: Option<&str>) -> Value {
        serde_json::json!({ "name": name, "user": login.map(|login| serde_json::json!({ "login": login })) })
    }

    fn response(commits: &[&[Value]]) -> Value {
        let nodes: Vec<Value> = commits
            .iter()
            .map(|authors| serde_json::json!({ "oid": "0", "authors": { "nodes": authors } }))
            .collect();
        serde_json::json!({ "data": { "nodes": nodes } })
    }

    #[test]
    fn bots_are_left_out_when_github_resolves_them_to_a_login() {
        let captured = json(BOT_COMMIT_AUTHORS);
        let logins: Vec<&str> = captured["data"]["nodes"]
            .as_array()
            .expect("the capture has nodes")
            .iter()
            .filter_map(|commit| commit["authors"]["nodes"][0]["user"]["login"].as_str())
            .collect();
        assert_eq!(
            logins,
            ["dependabot[bot]", "github-actions[bot]"],
            "the capture is what this test says it is"
        );
        assert_eq!(
            contributors(&[captured, json(COMMIT_AUTHORS)]).expect("the answers parse"),
            [
                Contributor::Account("beeauvin".to_string()),
                Contributor::Account("delphimoon".to_string()),
            ]
        );
    }

    #[test]
    fn unlinked_bots_are_left_out_and_unlinked_people_are_kept_by_name() {
        let answer = response(&[
            &[author("github-actions[bot]", None)],
            &[
                author("Someone Unlinked", None),
                author("Bee", Some("beeauvin")),
            ],
        ]);
        assert_eq!(
            contributors(&[answer]).expect("the answer parses"),
            [
                Contributor::Account("beeauvin".to_string()),
                Contributor::Unlinked("Someone Unlinked".to_string()),
            ]
        );
    }

    #[test]
    fn contributors_across_commits_and_queries_appear_once_sorted_without_case() {
        let first = response(&[&[author("Bee", Some("beeauvin")), author("zed", Some("Zed"))]]);
        let second = response(&[&[
            author("Bee again", Some("beeauvin")),
            author("amy", Some("amy")),
        ]]);
        let listed: Vec<String> = contributors(&[first, second])
            .expect("the answers parse")
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(listed, ["@amy", "@beeauvin", "@Zed"]);
    }

    #[test]
    fn a_graphql_error_is_refused_rather_than_read_as_no_contributors() {
        let answer =
            serde_json::json!({ "data": { "nodes": [null] }, "errors": [{ "type": "NOT_FOUND" }] });
        assert!(matches!(
            contributors(&[answer]),
            Err(CreditsError::Response(_))
        ));
    }

    #[test]
    fn a_null_node_without_an_error_is_refused_too() {
        let answer = serde_json::json!({ "data": { "nodes": [null] } });
        assert!(contributors(&[answer]).is_err());
    }

    #[test]
    fn the_previous_release_is_the_highest_published_one_below_the_tag() {
        let published = [
            "v0.1.0",
            "v0.2.0",
            "v0.1.5",
            "v0.3.0",
            "not-a-release",
            "v1.0.0-rc.1",
        ];
        assert_eq!(
            previous_release(published, tag("v0.2.1")),
            Some(tag("v0.2.0"))
        );
        assert_eq!(
            previous_release(published, tag("v0.1.6")),
            Some(tag("v0.1.5"))
        );
        assert_eq!(previous_release(published, tag("v0.1.0")), None);
        assert_eq!(previous_release([], tag("v0.1.0")), None);
    }

    #[test]
    fn a_first_release_says_so_instead_of_naming_a_previous_one() {
        assert_eq!(
            section(None, &[]),
            "## Contributors\n\nNobody has authored a commit in this release.\n"
        );
    }
}
