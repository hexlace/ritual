//! The cargo this process was launched by.
//!
//! Under `cargo run` and `cargo test`, Cargo sets `$CARGO` to the binary that
//! started the process. A task that runs cargo itself uses that one, so a
//! nested call is never a different cargo than the one in charge, which
//! whatever `cargo` resolves to on `$PATH` can be.

use std::ffi::OsString;
use std::process::Command;

/// Returns a [`Command`] for the cargo this process was launched by.
///
/// That is the binary `$CARGO` names, or `cargo` on `$PATH` when it is not
/// set, which is how a program started outside Cargo finds it. The caller adds
/// its own arguments and working directory.
///
/// # Examples
///
/// Asking the cargo that is running this very example for its version:
///
/// ```
/// use rituals_compose::cargo;
///
/// let output = cargo::command().arg("--version").output()?;
///
/// assert!(output.status.success());
/// assert!(String::from_utf8_lossy(&output.stdout).starts_with("cargo "));
/// # Ok::<(), std::io::Error>(())
/// ```
#[must_use]
pub fn command() -> Command {
    Command::new(program(std::env::var_os("CARGO")))
}

/// The program to run for cargo, given what `$CARGO` held.
///
/// Split from [`command`] so a test can say what each case answers without
/// setting an environment variable, which needs `unsafe` in this edition.
fn program(from_environment: Option<OsString>) -> OsString {
    from_environment.unwrap_or_else(|| OsString::from("cargo"))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{command, program};

    #[test]
    fn the_program_is_what_cargo_set_when_it_set_one() {
        // Cargo sets `$CARGO` to the binary that started the process, which
        // may live anywhere, so the answer has to be that path as given.
        let launching_cargo = OsString::from("/opt/toolchain/bin/cargo");

        assert_eq!(program(Some(launching_cargo.clone())), launching_cargo);
    }

    #[test]
    fn the_program_is_cargo_on_the_path_when_cargo_set_nothing() {
        assert_eq!(program(None), OsString::from("cargo"));
    }

    #[test]
    fn the_command_runs_the_program_the_environment_names() {
        // Pairs the pure function with the one that reads the environment,
        // so neither can change without the other.
        assert_eq!(
            command().get_program(),
            program(std::env::var_os("CARGO")).as_os_str()
        );
    }
}
