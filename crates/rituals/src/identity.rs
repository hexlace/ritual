//! A composed CLI's own identity: the package name, bin name and version
//! Cargo fixed when it was built.
//
// Neither `Identity` nor this module asserts anything at runtime. The only
// production caller is `identity!`'s expansion, which hands
// `from_macro_expansion` three `env!` results — Cargo guarantees each one is a
// non-empty string before the crate compiles at all, so a runtime check here
// would have nothing left to guard.

/// The identity of the composed command line a task is running inside — not
/// the task's own identity, and not a person's.
///
/// Carries the three facts Cargo fixes at compile time for a binary target:
/// its package name, its bin name, and its version. It is built with
/// [`macro@crate::identity`], which compiles only inside a binary target's
/// own source, because it reads `CARGO_BIN_NAME`, and Cargo defines that
/// variable nowhere else. That is deliberate: an `Identity` is only ever
/// meaningful as *the* identity of the command line currently running, and a
/// library crate — a task crate included — has no command line of its own to
/// be the identity of.
///
/// A task can opt in to being handed one of these at invocation time, with
/// [`crate::Task::receiving_command_line`].
///
/// # Examples
///
/// ```
/// use rituals::{CommandLine, Outcome, report};
///
/// fn run(command_line: &CommandLine) -> Outcome {
///     let identity = command_line.identity();
///     report(format!(
///         "{} {} (package {})",
///         identity.binary_name(),
///         identity.version(),
///         identity.package_name()
///     ));
///     Ok(())
/// }
/// # let identity = rituals::Identity::from_macro_expansion("acme-cli", "acme", "0.1.0");
/// # assert_eq!(identity.package_name(), "acme-cli");
/// # assert_eq!(identity.binary_name(), "acme");
/// # assert_eq!(identity.version(), "0.1.0");
/// # run(&rituals::CommandLine::from_dispatch(identity, []))?;
/// # Ok::<(), rituals::Failure>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Identity {
    package_name: &'static str,
    binary_name: &'static str,
    version: &'static str,
}

impl Identity {
    /// The name of the Cargo package the running command line was built
    /// from — `rituals-cli` for the global tool.
    ///
    /// A task only needs this to find its own crate among a workspace's
    /// members (see `rituals_compose::metadata::Metadata::locate_project`);
    /// nothing the framework prints to a person ever uses it.
    #[must_use]
    pub const fn package_name(self) -> &'static str {
        self.package_name
    }

    /// The name of the compiled binary the running command line was built
    /// as — `ritual` for the global tool.
    ///
    /// This name is fixed when the binary target is built; a rename or a
    /// symlink on disk afterward does not change it, which is what lets
    /// `--version`, `Usage:` and a refusal line all say one name however the
    /// file was reached. It is the name a person types when the tool is
    /// installed under the name it was built as.
    #[must_use]
    pub const fn binary_name(self) -> &'static str {
        self.binary_name
    }

    /// The version of the running command line, from its own `Cargo.toml`.
    #[must_use]
    pub const fn version(self) -> &'static str {
        self.version
    }

    /// Builds an [`Identity`] from the three strings [`macro@crate::identity`]
    /// read with `env!`.
    ///
    /// Hidden and `pub` rather than private: a macro's expansion needs a
    /// callable path to reach across the crate boundary, and `$crate::` paths
    /// cannot name a private item. The name states what a call site is
    /// claiming — that these three strings came out of an `identity!`
    /// expansion — so a hand-written call is visibly a lie. Call
    /// [`macro@crate::identity`] instead.
    ///
    /// Nothing checks the three strings, because there is nobody to check
    /// them against: a hand-written call can only fabricate an identity for
    /// the caller's own command line, and the caller is then the one who
    /// reads a wrong name in `--version`, a wrong prefix on a refusal, or
    /// watches `add` look for a package that is not theirs. Hidden, and named
    /// so the call reads as the claim it makes, is the strictest shape a
    /// constructor a macro expansion has to reach can take.
    #[doc(hidden)]
    #[must_use]
    pub const fn from_macro_expansion(
        package_name: &'static str,
        binary_name: &'static str,
        version: &'static str,
    ) -> Self {
        Self {
            package_name,
            binary_name,
            version,
        }
    }
}

/// Builds the [`Identity`] of the command line this macro is invoked from.
///
/// Expands to `env!` on the three Cargo variables that name a binary
/// target — `CARGO_PKG_NAME`, `CARGO_BIN_NAME`, `CARGO_PKG_VERSION` — read in
/// whichever crate calls this macro, not the crate it is defined in. Cargo
/// only defines `CARGO_BIN_NAME` while compiling a binary target, so this
/// macro is a compile error anywhere else: inside a library, inside an
/// integration test, inside a doctest. That is not a limitation to work
/// around: the compiler checks that this macro is only ever used in a
/// binary, rather than leaving it to a convention someone has to remember.
/// It does not check the hidden constructor the expansion calls; see
/// `Identity::from_macro_expansion` for why that is left unchecked.
///
/// The one real call site is the generated file every composed CLI checks
/// in, where it appears once, as `rituals::identity!()`.
///
/// # Examples
///
/// ```ignore
/// // Not a compiled doctest: a doctest compiles as neither a binary
/// // target nor an integration test, so `CARGO_BIN_NAME` is undefined for
/// // it — the same reason this macro cannot appear inside a library or a
/// // test. Real coverage comes from `crates/rituals-cli` compiling at all,
/// // plus its integration tests that check `--version` and `Usage:`.
/// let identity = rituals::identity!();
/// assert_eq!(identity.binary_name(), "ritual");
/// ```
#[macro_export]
macro_rules! identity {
    () => {
        $crate::Identity::from_macro_expansion(
            ::core::env!("CARGO_PKG_NAME"),
            ::core::env!("CARGO_BIN_NAME"),
            ::core::env!("CARGO_PKG_VERSION"),
        )
    };
}

#[cfg(test)]
mod tests {
    use super::Identity;

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    const fn assert_copy<T: Copy>() {}

    #[test]
    fn identity_is_send_and_sync_and_copy() {
        assert_send::<Identity>();
        assert_sync::<Identity>();
        assert_copy::<Identity>();
    }

    /// Three same-typed `&'static str` values, assigned to three fields in
    /// exactly one place, `Identity::from_macro_expansion` — a shape where
    /// two of the three could be transposed and every type check would
    /// still pass. Passing three distinct values and reading all three
    /// back through their own accessors is what catches that.
    #[test]
    fn three_distinct_values_reach_their_own_accessors() {
        let identity = Identity::from_macro_expansion("a-package", "a-binary", "1.2.3");
        assert_eq!(identity.package_name(), "a-package");
        assert_eq!(identity.binary_name(), "a-binary");
        assert_eq!(identity.version(), "1.2.3");
    }
}
