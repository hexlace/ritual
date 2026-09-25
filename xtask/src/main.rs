//! Ritual's release automation, run as `cargo xtask <command>`.
//!
//! ```text
//! cargo xtask bump <tag>           move every version site and Cargo.lock to <tag>
//! cargo xtask verify-tag <tag>     refuse unless the workspace is at <tag>
//! cargo xtask publish-plan <tag>   dry-run publishing <tag>, then print what
//!                                  to upload and what to leave out
//! cargo xtask release-contributors <owner/repo> <tag> <target> <dir>
//!                                  write the previous release and the
//!                                  Contributors section for <tag> into <dir>
//! ```
//!
//! A tag is `v` followed by a full `MAJOR.MINOR.PATCH`, such as `v0.1.1`. The
//! release workflows in `.github/workflows/` run these commands, and each one
//! runs the same way from a checkout, so what a workflow will do can be tried
//! locally first. None of them uploads anything: the publish workflow calls
//! `cargo publish` itself, in a job that compiles nothing.
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
mod contributors;
mod manifest;
mod process;
mod publish;
mod verify;
mod version;
mod workspace;

use std::error::Error;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage:
  cargo xtask bump <tag>
  cargo xtask verify-tag <tag>
  cargo xtask publish-plan <tag>
  cargo xtask release-contributors <owner/repo> <tag> <target> <dir>

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
        ["release-contributors", repository, tag, target, directory] => {
            contributors::run(&root, repository, tag, target)
                .map_err(Box::from)
                .and_then(|credits| {
                    write_credits(Path::new(directory), &credits)?;
                    Ok(format!(
                        "wrote {PREVIOUS_RELEASE_FILE} ({}) and {CONTRIBUTORS_FILE} to {directory}",
                        credits
                            .previous
                            .map_or_else(|| "none".to_string(), |previous| previous.to_string())
                    ))
                })
        }
        ["publish-plan", tag] => {
            return match publish::run(&root, tag) {
                Ok(plan) => {
                    report(&summary(&plan));
                    emit(&plan.outputs(), "the plan")
                }
                Err(error) => {
                    report(&format!("error: {error}"));
                    ExitCode::FAILURE
                }
            };
        }
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

/// What a publish plan uploads and leaves out, for a person reading the log.
fn summary(plan: &publish::Plan) -> String {
    let names = |members: &[publish::Member]| {
        members
            .iter()
            .map(publish::Member::name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let uploads = if plan.to_publish.is_empty() {
        "nothing to publish".to_string()
    } else {
        format!("dry-run published: [{}]", names(&plan.to_publish))
    };
    format!(
        "{uploads}; already on crates.io, skipped: [{}]; never published: [{}]",
        names(&plan.already_published),
        names(&plan.never_published),
    )
}

/// The file `release-contributors` writes the previous release's tag to:
/// the tag and a newline, or nothing at all for a first release.
const PREVIOUS_RELEASE_FILE: &str = "previous-release";

/// The file `release-contributors` writes the Contributors section to.
const CONTRIBUTORS_FILE: &str = "contributors.md";

/// Writes what the read-only half of a draft release hands on into
/// `directory`, which must already exist.
fn write_credits(directory: &Path, credits: &contributors::Credits) -> Result<(), String> {
    let previous = credits
        .previous
        .map_or_else(String::new, |previous| format!("{previous}\n"));
    for (name, contents) in [
        (PREVIOUS_RELEASE_FILE, previous.as_str()),
        (CONTRIBUTORS_FILE, credits.section.as_str()),
    ] {
        let path = directory.join(name);
        std::fs::write(&path, contents)
            .map_err(|error| format!("writing {} failed: {error}", path.display()))?;
    }
    Ok(())
}

/// Writes `body` to stdout, which carries only what a command produces for
/// another program to read; progress and errors go to stderr.
fn emit(body: &str, what: &str) -> ExitCode {
    match std::io::stdout().write_all(body.as_bytes()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            report(&format!("error: writing {what} failed: {error}"));
            ExitCode::FAILURE
        }
    }
}

/// Writes one line to stderr, where Cargo writes its own progress. Written
/// straight to the stream rather than through `eprintln!`, which panics if
/// stderr is closed; a report that cannot be written is not worth a panic.
fn report(line: &str) {
    drop(writeln!(std::io::stderr(), "{line}"));
}
