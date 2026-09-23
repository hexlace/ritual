//! This repository's own composed CLI — the `ritual` binary — lists exactly
//! its four commands, `add`, `regenerate`, `new` and `create`, in that
//! order, and reports its own bin name for `--version`, never its package
//! name, `rituals-cli`.
//!
//! The four commands are the children of ritual's management bundle,
//! mounted under the key `ritual` and flattened onto the top level because
//! the bin is named `ritual` too; the key itself is never a command. Runs
//! the `ritual` binary this suite was built with, so it needs no build of
//! its own.

mod support;

use support::{TempDir, TestOutcome, help, run_ritual};

#[test]
fn this_repositorys_own_cli_lists_its_four_commands_in_order() -> TestOutcome {
    let working_dir = TempDir::new("own-cli-help")?;
    let help = run_ritual(working_dir.path(), &["--help"])?;
    help.expect_success("`ritual --help`");

    assert_eq!(
        help::command_names(&help.stdout),
        ["add", "regenerate", "new", "create", "help"],
        "stdout was:\n{}",
        help.stdout
    );
    Ok(())
}

#[test]
fn this_repositorys_own_cli_reports_its_bin_name_for_version() -> TestOutcome {
    let working_dir = TempDir::new("own-cli-version")?;
    let version = run_ritual(working_dir.path(), &["--version"])?;
    version.expect_success("`ritual --version`");

    assert_eq!(
        version.stdout,
        format!("ritual {}\n", env!("CARGO_PKG_VERSION")),
        "expected --version to report the bin name and this crate's version"
    );
    Ok(())
}
