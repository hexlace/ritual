//! Running the programs the release tasks drive — Cargo and the GitHub
//! CLI — as child processes.

use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::process::{Command, ExitStatus, Output, Stdio};

/// A program this crate runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Program {
    /// Cargo: the one running this task, or plain `cargo` when run some
    /// other way. Cargo sets `CARGO` for every program it runs, so the child
    /// uses the same toolchain as the parent.
    Cargo,
    /// The GitHub CLI, which authenticates from `GH_TOKEN` in a workflow and
    /// from its own login locally.
    Gh,
}

impl Program {
    fn path(self) -> OsString {
        match self {
            Self::Cargo => std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")),
            Self::Gh => OsString::from("gh"),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Gh => "gh",
        }
    }
}

/// Runs `program` with `arguments` in `root`, all of its output passed
/// through to this process's stderr, and refuses unless it succeeds.
///
/// Stdout too goes to stderr, so that this process's stdout carries only
/// what a command prints for another program to read, such as the plan
/// `publish-plan` hands to the publish workflow.
pub(crate) fn run(program: Program, root: &Path, arguments: &[&str]) -> Result<(), CommandError> {
    let status = Command::new(program.path())
        .args(arguments)
        .current_dir(root)
        .stdout(std::io::stderr())
        .status()
        .map_err(|error| CommandError::spawn(program, arguments, error))?;
    if status.success() {
        Ok(())
    } else {
        Err(CommandError::failed(program, arguments, status))
    }
}

/// Runs `program` with `arguments` in `root` and returns what it printed to
/// stdout, refusing unless it succeeds. Its stderr passes through.
pub(crate) fn query(
    program: Program,
    root: &Path,
    arguments: &[&str],
) -> Result<String, CommandError> {
    let Output { status, stdout, .. } = Command::new(program.path())
        .args(arguments)
        .current_dir(root)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| CommandError::spawn(program, arguments, error))?;
    if !status.success() {
        return Err(CommandError::failed(program, arguments, status));
    }
    String::from_utf8(stdout).map_err(|_| CommandError {
        command: command_line(program, arguments),
        fault: CommandFault::NotUtf8,
    })
}

/// Runs `program` with `arguments` in `root`, with nothing printed, and says
/// only whether it succeeded.
pub(crate) fn succeeds(
    program: Program,
    root: &Path,
    arguments: &[&str],
) -> Result<bool, CommandError> {
    Command::new(program.path())
        .args(arguments)
        .current_dir(root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .map_err(|error| CommandError::spawn(program, arguments, error))
}

fn command_line(program: Program, arguments: &[&str]) -> String {
    format!("{} {}", program.name(), arguments.join(" "))
}

/// A child process that could not be started or did not succeed.
#[derive(Debug)]
pub(crate) struct CommandError {
    command: String,
    fault: CommandFault,
}

#[derive(Debug)]
enum CommandFault {
    Spawn(std::io::Error),
    Failed(ExitStatus),
    NotUtf8,
}

impl CommandError {
    fn spawn(program: Program, arguments: &[&str], error: std::io::Error) -> Self {
        Self {
            command: command_line(program, arguments),
            fault: CommandFault::Spawn(error),
        }
    }

    fn failed(program: Program, arguments: &[&str], status: ExitStatus) -> Self {
        Self {
            command: command_line(program, arguments),
            fault: CommandFault::Failed(status),
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.fault {
            CommandFault::Spawn(error) => {
                write!(formatter, "`{}` could not start: {error}", self.command)
            }
            CommandFault::Failed(status) => {
                write!(formatter, "`{}` failed: {status}", self.command)
            }
            CommandFault::NotUtf8 => write!(formatter, "`{}` printed invalid UTF-8", self.command),
        }
    }
}

impl Error for CommandError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.fault {
            CommandFault::Spawn(error) => Some(error),
            CommandFault::Failed(_) | CommandFault::NotUtf8 => None,
        }
    }
}
