//! Finding this repository's checkout, and skipping out loud when there is
//! none.
//!
//! A story scaffolds its project with `ritual new --path <checkout>`, so the
//! project builds against ritual's crates exactly as they are on disk —
//! `crates/rituals`, `crates/rituals-core` and the task crates under
//! `tasks/`. That needs the workspace this crate is a member of. A copy of
//! this crate on its own (the package `cargo package` produces, unpacked
//! somewhere) has none, and a story run there has nothing to import from.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use super::process::cargo_query;
use super::{Outcome, TestOutcome, path_to_str};

/// This repository's checkout: the root of the workspace this crate is a
/// member of.
pub(crate) struct Checkout {
    root: PathBuf,
}

/// The file that marks a workspace root as a ritual checkout: the same file
/// `new` and `create` check a `--path` against. The story support cannot
/// call that check itself, since this crate deliberately depends on nothing
/// from the composition library.
const RITUAL_CHECKOUT_MARKER: &str = "crates/rituals/Cargo.toml";

/// Why there is no checkout to run a story against.
pub(crate) struct NoCheckout(String);

impl Checkout {
    /// Asks Cargo for the root of the workspace this crate belongs to —
    /// `cargo locate-project --workspace`, the same question ritual itself
    /// asks — and returns it when it is a workspace this crate is a member
    /// of rather than this crate on its own.
    ///
    /// Cargo decides membership, so however the root's `members` names this
    /// crate — a path, a glob — the answer is the same. A copy of this crate
    /// on its own is its own workspace root, and one sitting inside some
    /// other workspace that does not list it is refused by Cargo. A copy
    /// that some other workspace does list is a member of that workspace,
    /// not of ritual's, so the root must also carry
    /// [`RITUAL_CHECKOUT_MARKER`]. Each is the absence of a checkout, and the
    /// reason says which.
    pub(crate) fn find() -> Outcome<Result<Self, NoCheckout>> {
        let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let located = cargo_query(
            crate_dir,
            &["locate-project", "--workspace", "--message-format", "plain"],
        )?;
        if located.exit_code != Some(0) {
            return Ok(Err(NoCheckout(format!(
                "cargo found no workspace this crate is a member of: {}",
                located.stderr.trim()
            ))));
        }

        let workspace_manifest = PathBuf::from(located.stdout.trim());
        let Some(root) = workspace_manifest.parent() else {
            return Ok(Err(NoCheckout(format!(
                "cargo named {} as the workspace manifest, which has no directory",
                workspace_manifest.display()
            ))));
        };
        // Compared canonically: either path may reach the directory through
        // a symlink the other resolved.
        let same_directory = |left: &Path, right: &Path| {
            left.canonicalize()
                .ok()
                .is_some_and(|left| right.canonicalize().ok() == Some(left))
        };
        if same_directory(root, crate_dir) {
            return Ok(Err(NoCheckout(
                "this crate is its own workspace, not a member of ritual's".to_string(),
            )));
        }
        if !root.join(RITUAL_CHECKOUT_MARKER).is_file() {
            return Ok(Err(NoCheckout(format!(
                "the workspace at {} is not a ritual checkout: it has no {RITUAL_CHECKOUT_MARKER}",
                root.display()
            ))));
        }
        Ok(Ok(Self {
            root: root.to_path_buf(),
        }))
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The checkout's root as a `--path` argument.
    pub(crate) fn path_argument(&self) -> Outcome<&str> {
        path_to_str(&self.root)
    }
}

/// Runs `story` against this repository's checkout, or, when there is no
/// checkout to run it against, skips it and says so.
///
/// The skip is written straight to the test process's standard error rather
/// than through `eprintln!`, which the test harness captures and discards
/// for a passing test: a skip has to be visible in an ordinary `cargo test`
/// run, or it reads exactly like a pass.
pub(crate) fn in_checkout(story: impl FnOnce(&Checkout) -> TestOutcome) -> TestOutcome {
    match Checkout::find()? {
        Ok(checkout) => story(&checkout),
        Err(NoCheckout(reason)) => {
            report_skip(&format!(
                "it builds against ritual's own crates, and {reason}"
            ));
            Ok(())
        }
    }
}

/// Writes one line saying the current test was skipped, and why.
///
/// The test harness names each test's thread after the test, so the line
/// carries the test's own name wherever the harness runs it on one.
pub(crate) fn report_skip(reason: &str) {
    let current = std::thread::current();
    let test = current.name().unwrap_or("a story test");
    // Best effort: a skip notice that cannot be written changes nothing about
    // the result, and a failed write to stderr has nowhere better to go.
    drop(writeln!(std::io::stderr(), "SKIPPED {test}: {reason}"));
}
