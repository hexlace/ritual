//! Running processes — `ritual`, `cargo`, and binaries a story built — and
//! reading back what they reported.
//!
//! Every Cargo build a story causes, directly or through `ritual`, lands in
//! a target directory that belongs to that story alone —
//! [`Project::target_dir`](super::Project::target_dir), inside the story's
//! own temporary directory — whatever the outer test run's environment
//! says. Nothing is shared between stories, not even intermediate build
//! artifacts: Cargo names a workspace member's artifacts after its package
//! name, version and path *relative to its workspace root*, so two stories
//! that each scaffold a project called `demo`, or a task crate at
//! `tasks/wake`, would build to the same artifact name in a shared
//! directory, and one could run a binary compiled from the other's
//! sources. Each project therefore compiles its registry dependencies once
//! for itself.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::{OptionContext, Outcome, ResultContext};

/// The `cargo` this suite was itself run by.
///
/// Cargo sets `CARGO` for every test process it runs, so a project a story
/// builds uses the same `cargo` as the suite rather than whatever comes
/// first on `PATH`. Which `rustc` that `cargo` runs is the environment's
/// choice: under rustup, the toolchain it selected for the suite is
/// exported to every process the suite starts, and applies there too.
fn cargo_program() -> Outcome<OsString> {
    std::env::var_os("CARGO").context("CARGO is not set; run this suite through `cargo test`")
}

/// What running a process reported back — the whole of what a caller of that
/// process can observe.
#[derive(Debug)]
pub(crate) struct RunOutput {
    /// The process's exit status, or `None` if it was killed by a signal
    /// rather than exiting on its own.
    pub(crate) exit_code: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl RunOutput {
    /// Asserts that the process exited successfully.
    ///
    /// Used at setup steps of a story — "scaffold a project" — where the
    /// story has nothing to say if the setup itself failed, so the failure
    /// should read as a clear precondition violation rather than a
    /// confusing downstream assertion.
    #[track_caller]
    pub(crate) fn expect_success(&self, what: &str) -> &Self {
        assert_eq!(
            self.exit_code,
            Some(0),
            "expected {what} to succeed\nstdout:\n{}\nstderr:\n{}",
            self.stdout,
            self.stderr
        );
        self
    }

    /// Asserts that the process exited unsuccessfully, on its own rather
    /// than by a signal.
    #[track_caller]
    pub(crate) fn expect_failure(&self, what: &str) -> &Self {
        assert!(
            self.exit_code.is_some_and(|code| code != 0),
            "expected {what} to be refused, but it exited {:?}\nstdout:\n{}\nstderr:\n{}",
            self.exit_code,
            self.stdout,
            self.stderr
        );
        self
    }

    /// Asserts that stderr is exactly one line, prefixed with `<bin_name>: `,
    /// and returns what follows the prefix.
    ///
    /// This is the shape every refusal and warning the framework writes
    /// takes. Only meaningful on the output of a binary run directly — see
    /// [`run_binary`] — since `cargo run` writes its own progress lines to
    /// the same stream.
    #[track_caller]
    pub(crate) fn sole_line_prefixed_with(&self, bin_name: &str) -> &str {
        let prefix = format!("{bin_name}: ");
        assert_eq!(
            self.stderr.lines().count(),
            1,
            "expected exactly one line on stderr; stderr was:\n{}",
            self.stderr
        );
        let message = self.stderr.trim_end().strip_prefix(&prefix);
        assert!(
            message.is_some(),
            "expected the line to be prefixed with `{prefix}`; stderr was:\n{}",
            self.stderr
        );
        message.unwrap_or_default()
    }
}

/// Runs `command` to completion, with nothing on its stdin, and collects
/// what it reported.
fn run_to_completion(command: &mut Command, description: &str) -> Outcome<RunOutput> {
    let output = command
        .stdin(Stdio::null())
        .output()
        .context(&format!("spawning {description} failed"))?;
    Ok(RunOutput {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// Points every Cargo build `command` causes at `target_dir`, and clears a
/// separate build directory the outer environment may have set, so nothing
/// the build writes lands anywhere shared.
fn in_own_target_dir<'command>(
    command: &'command mut Command,
    target_dir: &Path,
) -> &'command mut Command {
    command
        .env("CARGO_TARGET_DIR", target_dir)
        .env_remove("CARGO_BUILD_BUILD_DIR")
}

/// Runs `cargo` with `arguments` in `current_dir`, building into
/// `target_dir`.
pub(crate) fn cargo(
    current_dir: &Path,
    target_dir: &Path,
    arguments: &[&str],
) -> Outcome<RunOutput> {
    let mut command = Command::new(cargo_program()?);
    command.args(arguments).current_dir(current_dir);
    run_to_completion(
        in_own_target_dir(&mut command, target_dir),
        &format!("`cargo {}`", arguments.join(" ")),
    )
}

/// Runs a `cargo` command that reads a workspace and builds nothing, such as
/// `locate-project`, with `arguments` in `current_dir`.
pub(crate) fn cargo_query(current_dir: &Path, arguments: &[&str]) -> Outcome<RunOutput> {
    run_to_completion(
        Command::new(cargo_program()?)
            .args(arguments)
            .current_dir(current_dir),
        &format!("`cargo {}`", arguments.join(" ")),
    )
}

/// Runs `cargo` with `arguments` in `current_dir` with neither Cargo
/// location set, whatever this process's own environment carries — so the
/// build lands where Cargo puts it by default, `<workspace>/target`, and
/// `cargo clean` removes exactly that.
///
/// For the one story whose subject is Cargo's default layout. `current_dir`
/// must be inside that story's own temporary directory, so the default
/// location is the story's own.
pub(crate) fn cargo_with_default_locations(
    current_dir: &Path,
    arguments: &[&str],
) -> Outcome<RunOutput> {
    run_to_completion(
        Command::new(cargo_program()?)
            .args(arguments)
            .current_dir(current_dir)
            .env_remove("CARGO_TARGET_DIR")
            .env_remove("CARGO_BUILD_BUILD_DIR"),
        &format!("`cargo {}`", arguments.join(" ")),
    )
}

/// Runs `binary` directly with `arguments` in `current_dir` — never through
/// `cargo run`, whose own `Compiling …`/`Running …` lines share the child's
/// stderr and would answer any `stderr.contains(…)` about a name that
/// appears in a crate name, a path, or the command line itself.
///
/// Anything `binary` builds lands in `current_dir`'s own `target/`.
pub(crate) fn run_binary(
    binary: &Path,
    current_dir: &Path,
    arguments: &[&str],
) -> Outcome<RunOutput> {
    let mut command = Command::new(binary);
    command.args(arguments).current_dir(current_dir);
    run_to_completion(
        in_own_target_dir(&mut command, &current_dir.join("target")),
        &binary.display().to_string(),
    )
}

/// Runs the `ritual` binary this suite was built with —
/// `CARGO_BIN_EXE_ritual`, the binary a caller would install — with
/// `arguments` in `current_dir`.
pub(crate) fn run_ritual(current_dir: &Path, arguments: &[&str]) -> Outcome<RunOutput> {
    run_binary(ritual_binary(), current_dir, arguments)
}

/// The `ritual` binary this suite was built with.
pub(crate) fn ritual_binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_ritual"))
}

/// The path Cargo gives a built binary named `bin_name` under `target_dir`.
pub(crate) fn built_binary_path(target_dir: &Path, bin_name: &str) -> PathBuf {
    target_dir
        .join("debug")
        .join(format!("{bin_name}{}", std::env::consts::EXE_SUFFIX))
}

/// The parent directory of `path`, for the few helpers that need one and
/// hold a path that always has one.
pub(crate) fn parent_of(path: &Path) -> Outcome<&Path> {
    path.parent()
        .context(&format!("{} has no parent directory", path.display()))
}
