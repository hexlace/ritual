//! A composed CLI's shown identity — what `--version` and the `Usage:` line
//! report — is the bin name it was compiled as, not the name the process
//! happened to be invoked under.
//!
//! `--version` on the global binary prints `ritual <version>` and never the
//! package name; on a freshly scaffolded project's own CLI it prints that
//! project's bin name the same way. Both `--version` and `Usage:` report
//! that name through a symlink and through a renamed copy of the binary,
//! with no rebuild.
//!
//! The global-binary cases build nothing: they copy or symlink the one
//! `ritual` binary Cargo built to run this suite. No rebuild happens between
//! the plainly invoked run and the renamed or symlinked one, so nothing here
//! can pass because a rebuild picked something up.

mod support;

use std::path::Path;

use support::process::{parent_of, ritual_binary};
use support::{
    Project, ResultContext, RunOutput, TempDir, TestOutcome, in_checkout, run_binary, run_ritual,
};

/// Runs `binary_path` directly, from its own directory — not through
/// [`support::run_ritual`], which is pinned to `CARGO_BIN_EXE_ritual` and so
/// cannot express "the same build, under a different name".
fn run_binary_directly(binary_path: &Path, arguments: &[&str]) -> support::Outcome<RunOutput> {
    run_binary(binary_path, parent_of(binary_path)?, arguments)
}

/// Copies the one binary Cargo already built for this test run to
/// `destination`, preserving the file's executable bit.
fn copy_binary(source: &Path, destination: &Path) -> TestOutcome {
    std::fs::copy(source, destination).context("copying the built binary failed")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(destination, permissions)
            .context("marking the copied binary executable failed")?;
    }

    Ok(())
}

/// The version line the plainly built, plainly named binary reports for
/// `--version` — the ground truth every renamed or symlinked invocation in
/// this file is compared against. Reading it this way, rather than parsing
/// the workspace manifest's own version field, keeps this file honest about
/// what it actually checks: not "does some string equal some other string
/// I computed", but "does a copy of this exact binary, run under a
/// different name, report what the original build reports".
fn plain_invocation_version_line() -> support::Outcome<String> {
    let working_dir = TempDir::new("plain-version")?;
    let result = run_ritual(working_dir.path(), &["--version"])?;
    result.expect_success("`--version` on the plainly built, plainly invoked binary");
    Ok(result.stdout)
}

/// A renamed copy of the binary must still report `ritual` for
/// `--version` — the compiled identity, not the name it happens to be
/// called on disk — and must never fall back to the crate's package name
/// (`rituals-cli`).
#[test]
fn version_reports_the_compiled_bin_name_through_a_renamed_copy() -> TestOutcome {
    let ground_truth = plain_invocation_version_line()?;
    assert!(
        ground_truth.starts_with("ritual "),
        "expected the plainly invoked binary's own --version to start with \
         `ritual `; got:\n{ground_truth}"
    );
    assert!(
        !ground_truth.contains("rituals-cli"),
        "expected --version to never contain the package name `rituals-cli`; got:\n{ground_truth}"
    );

    let working_dir = TempDir::new("version-renamed-copy")?;
    let renamed_path = working_dir.path().join("totally-not-ritual");
    copy_binary(ritual_binary(), &renamed_path)?;

    let renamed_result = run_binary_directly(&renamed_path, &["--version"])?;
    renamed_result.expect_success("`--version` on a renamed copy of the built binary");

    assert_eq!(
        renamed_result.stdout, ground_truth,
        "expected a renamed copy's --version to report exactly what the \
         plainly named build reports, not the name it was invoked under"
    );
    assert!(
        !renamed_result.stdout.contains("totally-not-ritual"),
        "expected --version to never contain the name the binary was renamed \
         to; got:\n{}",
        renamed_result.stdout
    );

    Ok(())
}

/// The `Usage:` line — printed on the missing-subcommand path, to stderr,
/// with exit code 2 — must name `ritual`, the compiled identity, through a
/// renamed copy too, and must agree with what the plainly invoked binary
/// prints.
#[test]
fn usage_line_reports_the_compiled_bin_name_through_a_renamed_copy() -> TestOutcome {
    let plain_working_dir = TempDir::new("plain-usage")?;
    let plain_result = run_ritual(plain_working_dir.path(), &[])?;
    assert_eq!(
        plain_result.exit_code,
        Some(2),
        "expected the plainly invoked binary with no subcommand to exit with clap's \
         argument-error status; stderr was:\n{}",
        plain_result.stderr
    );
    assert!(
        plain_result.stderr.starts_with("Usage: ritual "),
        "expected the plainly invoked binary's own Usage: line to start with \
         `Usage: ritual `; stderr was:\n{}",
        plain_result.stderr
    );

    let working_dir = TempDir::new("usage-renamed-copy")?;
    let renamed_path = working_dir.path().join("totally-not-ritual");
    copy_binary(ritual_binary(), &renamed_path)?;

    let renamed_result = run_binary_directly(&renamed_path, &[])?;
    assert_eq!(
        renamed_result.exit_code,
        Some(2),
        "expected a renamed copy with no subcommand to exit with clap's argument-error \
         status; stderr was:\n{}",
        renamed_result.stderr
    );

    assert_eq!(
        renamed_result.stderr, plain_result.stderr,
        "expected a renamed copy's Usage: line to report exactly what the \
         plainly named build reports, not the name it was invoked under"
    );
    assert!(
        !renamed_result.stderr.contains("totally-not-ritual"),
        "expected the Usage: line to never contain the name the binary was \
         renamed to; stderr was:\n{}",
        renamed_result.stderr
    );

    Ok(())
}

/// The same two properties again, through a symlink rather than a copy —
/// the other invocation shape. Unix-only, since creating a symlink is a
/// platform-specific operation.
#[cfg(unix)]
#[test]
fn version_and_usage_report_the_compiled_bin_name_through_a_symlink() -> TestOutcome {
    let version_ground_truth = plain_invocation_version_line()?;

    let plain_working_dir = TempDir::new("plain-usage")?;
    let plain_usage = run_ritual(plain_working_dir.path(), &[])?;
    plain_usage.expect_failure("the plainly built, plainly invoked binary with no subcommand");

    let working_dir = TempDir::new("symlink")?;
    let symlink_path = working_dir.path().join("ritual-via-symlink");
    std::os::unix::fs::symlink(ritual_binary(), &symlink_path)
        .context("creating the symlink failed")?;

    let symlinked_version = run_binary_directly(&symlink_path, &["--version"])?;
    symlinked_version.expect_success("`--version` through a symlink to the built binary");
    assert_eq!(
        symlinked_version.stdout, version_ground_truth,
        "expected --version through a symlink to report exactly what the \
         plainly named build reports, not the symlink's own name"
    );
    assert!(
        !symlinked_version.stdout.contains("ritual-via-symlink"),
        "expected --version to never contain the symlink's own name; got:\n{}",
        symlinked_version.stdout
    );

    let symlinked_usage = run_binary_directly(&symlink_path, &[])?;
    symlinked_usage.expect_failure("a symlink to the built binary, with no subcommand");
    assert_eq!(
        symlinked_usage.stderr, plain_usage.stderr,
        "expected the Usage: line through a symlink to report exactly what \
         the plainly named build reports, not the symlink's own name"
    );
    assert!(
        !symlinked_usage.stderr.contains("ritual-via-symlink"),
        "expected the Usage: line to never contain the symlink's own name; \
         stderr was:\n{}",
        symlinked_usage.stderr
    );

    Ok(())
}

/// A freshly scaffolded project's own composed CLI reports its own bin name
/// for `--version`, the same way, through a renamed copy of its own built
/// binary — not merely when run plainly.
#[test]
fn version_on_a_freshly_scaffolded_projects_own_cli_survives_a_renamed_copy() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("scaffold-version-rename")?;
        let project = Project::scaffold(checkout, working_dir.path(), "identity-project", &[])?;
        let bin_name = project.bin_name()?;

        let built_binary = project.build()?;
        let plain_result = run_binary_directly(&built_binary, &["--version"])?;
        plain_result.expect_success("`--version` on the scaffolded project's plainly named binary");
        assert!(
            plain_result.stdout.starts_with(&format!("{bin_name} ")),
            "expected the scaffolded project's own --version to start with its own bin name \
             `{bin_name}`; got:\n{}",
            plain_result.stdout
        );

        let renamed_working_dir = TempDir::new("scaffold-version-rename-copy")?;
        let renamed_path = renamed_working_dir.path().join("not-the-projects-name");
        copy_binary(&built_binary, &renamed_path)?;

        let renamed_result = run_binary_directly(&renamed_path, &["--version"])?;
        renamed_result
            .expect_success("`--version` on a renamed copy of the scaffolded project's binary");
        assert_eq!(
            renamed_result.stdout, plain_result.stdout,
            "expected a renamed copy of the scaffolded project's binary to report exactly \
             what the plainly named build reports"
        );
        Ok(())
    })
}
