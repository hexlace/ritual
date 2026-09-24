//! Ritual's release automation, run as `cargo xtask <command>`.
//!
//! ```text
//! cargo xtask bump <tag>                 move every version site and Cargo.lock to <tag>
//! cargo xtask verify-tag <tag>           refuse unless the workspace is at <tag>
//! cargo xtask publish <tag> [--dry-run]  publish every crate <tag> releases
//! cargo xtask release-notes <owner/repo> <tag> <target>
//!                                        print the draft release body for <tag>
//! ```
//!
//! A tag is `v` followed by a full `MAJOR.MINOR.PATCH`, such as `v0.1.1`. The
//! release workflows in `.github/workflows/` run these commands, and each one
//! runs the same way from a checkout, so what a workflow will do can be tried
//! locally first.
//
// `redundant_pub_crate` (clippy nursery) wants `pub` on every item below,
// because a binary's modules are all private. `pub(crate)` is the visibility
// that is actually true, and plain `pub` would trip the workspace's
// `unreachable_pub` instead, so for this whole crate the nursery lint gives way.
#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) is this binary's real visibility; plain pub trips unreachable_pub"
)]

mod bump;
mod manifest;
mod notes;
mod process;
mod publish;
mod verify;
mod version;
mod workspace;

use std::error::Error;
use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "usage:
  cargo xtask bump <tag>
  cargo xtask verify-tag <tag>
  cargo xtask publish <tag> [--dry-run]
  cargo xtask release-notes <owner/repo> <tag> <target>

A tag is `v` then MAJOR.MINOR.PATCH, such as v0.1.1.";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let root = workspace::root();

    let outcome: Result<String, Box<dyn Error>> = match arguments.as_slice() {
        ["bump", tag] => bump::run(&root, tag)
            .map(|bumped| {
                format!(
                    "bumped {} -> {}: [workspace.package], {} internal requirements, Cargo.lock",
                    bumped.from, bumped.to, bumped.internal_requirements
                )
            })
            .map_err(Box::from),
        ["verify-tag", tag] => verify::run(&root, tag)
            .map(|tag| format!("the workspace is at {tag}"))
            .map_err(Box::from),
        ["release-notes", repository, tag, target] => {
            return match notes::run(&root, repository, tag, target) {
                Ok(body) => match std::io::stdout().write_all(body.as_bytes()) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(error) => {
                        report(&format!("error: writing the release body failed: {error}"));
                        ExitCode::FAILURE
                    }
                },
                Err(error) => {
                    report(&format!("error: {error}"));
                    ExitCode::FAILURE
                }
            };
        }
        ["publish", tag] => publish_report(&root, tag, publish::Mode::Publish),
        ["publish", tag, "--dry-run"] => publish_report(&root, tag, publish::Mode::DryRun),
        _ => {
            report(USAGE);
            return ExitCode::FAILURE;
        }
    };
    match outcome {
        Ok(summary) => {
            report(&summary);
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(&format!("error: {error}"));
            ExitCode::FAILURE
        }
    }
}

fn publish_report(
    root: &std::path::Path,
    tag: &str,
    mode: publish::Mode,
) -> Result<String, Box<dyn Error>> {
    let plan = publish::run(root, tag, mode)?;
    let names = |members: &[publish::Member]| {
        members
            .iter()
            .map(publish::Member::name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    if plan.to_publish.is_empty() {
        return Ok(format!(
            "nothing to publish: already on crates.io: [{}]; never published: [{}]",
            names(&plan.already_published),
            names(&plan.never_published),
        ));
    }
    let verb = match mode {
        publish::Mode::Publish => "published",
        publish::Mode::DryRun => "dry-run published",
    };
    Ok(format!(
        "{verb}: [{}]; already on crates.io, skipped: [{}]; never published: [{}]",
        names(&plan.to_publish),
        names(&plan.already_published),
        names(&plan.never_published),
    ))
}

/// Writes one line to stderr, where Cargo writes its own progress. Written
/// straight to the stream rather than through `eprintln!`, which panics if
/// stderr is closed; a report that cannot be written is not worth a panic.
fn report(line: &str) {
    drop(writeln!(std::io::stderr(), "{line}"));
}
