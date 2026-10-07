//! What every test module in this crate shares: the outcome a test returns,
//! and the scratch directory and skip notice from [`crate::test_util`],
//! named here so a test module finds all of it in one place.
//!
//! Declared behind `#[cfg(test)]` at the `mod test_support;` site in
//! `lib.rs`, not inside this file — the whole point of a shared module is
//! one thing to read, and a second `#![cfg(test)]` here would say the same
//! thing twice.
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

use std::error::Error;

pub(crate) use crate::test_util::{ScratchDir, report_skip};

/// What a test in this crate returns — the error path carries only a setup
/// failure (a filesystem operation, a TOML fixture that would not parse),
/// never the property under test, which is always carried by an
/// `assert!`/`assert_eq!` instead.
pub(crate) type TestOutcome = Result<(), Box<dyn Error>>;
