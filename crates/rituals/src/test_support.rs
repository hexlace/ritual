//! A test double shared by every test module in this crate that needs a
//! task with no arguments and a handler that always succeeds.
//!
//! One definition of "a task that takes nothing and never fails", used by
//! every test module in this crate that needs one: the same pair copied
//! into several modules can drift apart without anything noticing.
//
// `redundant_pub_crate` (clippy nursery) wants `pub` here because this
// module is private, but `pub(crate)` is the visibility that is actually
// true — it stays correct if this module is ever re-exported at a different
// level — so the nursery lint gives way.
#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) reflects this module's actual visibility even though the enclosing \
              module is private; see the note above"
)]

use crate::outcome::Outcome;

/// A task's argument struct with no fields to parse.
#[derive(clap::Args)]
pub(crate) struct NoArguments {}

/// This test double's handler contract is `Fn(A) -> Outcome`, so it always
/// returns `Outcome` even though this one never fails — unlike a real
/// task, whose body can fail.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the handler contract is `Fn(A) -> Outcome`; this test double never fails on \
              purpose, but the signature still has to match what a real handler returns"
)]
pub(crate) fn run_ok(_arguments: NoArguments) -> Outcome {
    Ok(())
}
