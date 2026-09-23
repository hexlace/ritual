//! Reading the command list out of a composed CLI's `--help`.
//!
//! Every composed CLI renders `--help` through clap, which lists one
//! command per row under a `Commands:` header: the name indented by two
//! spaces, then its description. Reading that block — rather than asking
//! whether the name appears anywhere in the output — is what tells a
//! command apart from the bin name on the `Usage:` line, from a word in
//! some other command's description, and from a longer command name that
//! contains it. The order the rows come in is part of what several stories
//! check, and it exists nowhere but in this rendering.

use super::RunOutput;

/// How far clap indents a command's name in the `Commands:` block. A row
/// whose text starts deeper than this is the continuation of a description
/// that wrapped, not a command.
const COMMAND_INDENT: &str = "  ";

/// The command names listed in `help_stdout`'s `Commands:` block, in the
/// order clap lists them.
///
/// The block runs from the `Commands:` header to the first line that is
/// blank or not indented. Within it, a row whose name starts at exactly
/// [`COMMAND_INDENT`] contributes its first word; deeper rows are wrapped
/// description text and contribute nothing.
pub(crate) fn command_names(help_stdout: &str) -> Vec<String> {
    help_stdout
        .lines()
        .skip_while(|line| line.trim_end() != "Commands:")
        .skip(1)
        .take_while(|line| line.starts_with(COMMAND_INDENT))
        .filter_map(|line| line.strip_prefix(COMMAND_INDENT))
        .filter(|row| row.starts_with(|character: char| !character.is_whitespace()))
        .filter_map(|row| row.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

/// Whether `name` is listed as a whole command name in `help_stdout`'s
/// `Commands:` block — see [`command_names`].
pub(crate) fn lists_command(help_stdout: &str, name: &str) -> bool {
    command_names(help_stdout)
        .iter()
        .any(|listed| listed == name)
}

/// Asserts that `output` failed because clap did not recognise `name` as a
/// subcommand — not merely that it failed, which a bare non-zero exit
/// cannot tell apart from any other failure.
#[track_caller]
pub(crate) fn assert_refuses_unrecognized_subcommand(output: &RunOutput, name: &str) {
    output.expect_failure(&format!("running the unrecognised subcommand `{name}`"));
    let quoted_name = format!("'{name}'");
    assert!(
        output.stderr.contains("unrecognized subcommand") && output.stderr.contains(&quoted_name),
        "expected clap to name `{name}` as the unrecognised subcommand, not some unrelated \
         failure; stderr was:\n{}",
        output.stderr
    );
}
