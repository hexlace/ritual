//! Which paths the checkout's sparse-checkout patterns include.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use crate::git::{Unanswered, failure_of, nul_terminated, run_git_for_output, run_git_with_input};

/// Whether the checkout the work tree at `top_level` belongs to is a sparse
/// one.
///
/// The configuration decides, not whether a patterns file exists: `git
/// sparse-checkout disable` turns the setting off and leaves the file, and
/// `check-rules` either fails when sparse checkout was never on or, after
/// `disable`, prints nothing, which would read as every path being outside.
pub(super) fn is_on(new_git: &impl Fn() -> Command, top_level: &Path) -> Result<bool, Unanswered> {
    let output = run_git_for_output(
        new_git,
        top_level,
        &["config", "--bool", "core.sparseCheckout"],
        &[],
    )?;
    // Exit 1 says the setting is not set, which is the same as off.
    match output.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&output.stdout).trim_end() == "true"),
        Some(1) => Ok(false),
        Some(_) | None => Err(failure_of(&output)),
    }
}

/// The ones among `paths`, each spelled from `top_level`, that the sparse
/// checkout's patterns include.
pub(super) fn included(
    new_git: &impl Fn() -> Command,
    top_level: &Path,
    paths: &[String],
) -> Result<BTreeSet<String>, Unanswered> {
    if paths.is_empty() {
        return Ok(BTreeSet::new());
    }
    let input = nul_terminated(paths.iter().map(String::as_bytes));
    let output = run_git_with_input(
        new_git,
        top_level,
        &["sparse-checkout", "check-rules", "-z"],
        &input,
    )?;
    Ok(String::from_utf8_lossy(&output)
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect())
}
