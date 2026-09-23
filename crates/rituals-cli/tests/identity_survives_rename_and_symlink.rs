//! A composed CLI's shown identity — what `--version` and the `Usage:` line
//! report — is the bin name it was compiled as, not the name the process
//! happened to be invoked under.
//!
//! `--version` on the global binary prints `ritual <version>` and never the
//! package name; on a freshly scaffolded project's own CLI it prints that
//! project's bin name the same way. Both `--version` and `Usage:` report
//! that name through a symlink and through a hard link — a second name for
//! the same file — with no rebuild.
//!
//! The global-binary cases build nothing: they hard-link or symlink the one
//! `ritual` binary Cargo built to run this suite. No rebuild happens between
//! the plainly invoked run and the renamed or symlinked one, so nothing here
//! can pass because a rebuild picked something up.

mod support;

use std::path::{Path, PathBuf};

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

/// Gives `binary`, a binary Cargo already built, a second name, `new_name`,
/// in a fresh directory beside it, and returns that directory, which removes
/// the link on drop, with the new path.
///
/// A hard link, not a copy. A copy writes a new file through a writable file
/// descriptor in this process, and every test here spawns children from its
/// own thread: on Linux a child forked while that descriptor is open holds
/// it until the child execs, and until then executing the copy fails with
/// `ETXTBSY` ("Text file busy"). A hard link writes no file, only a name for
/// the one Cargo's linker wrote and closed in another process, so no writable
/// descriptor exists for a child to inherit. The directory sits beside the
/// binary rather than under the system temp root because a hard link cannot
/// cross filesystems, and nothing makes the temp root and the target
/// directory share one.
fn link_under_new_name(binary: &Path, new_name: &str) -> support::Outcome<(TempDir, PathBuf)> {
    let directory = TempDir::new_in(parent_of(binary)?, "renamed")?;
    let renamed_path = directory.path().join(new_name);
    assert_ne!(
        renamed_path.file_name(),
        binary.file_name(),
        "the second name must differ from the name Cargo built the binary under"
    );
    std::fs::hard_link(binary, &renamed_path)
        .context("hard-linking the built binary under a new name failed")?;

    // The same file, not a lookalike: had anything written a new file here,
    // this would be a copy again, with the race above.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let original = std::fs::metadata(binary).context("reading the built binary failed")?;
        let renamed = std::fs::metadata(&renamed_path).context("reading the new name failed")?;
        assert_eq!(
            (renamed.dev(), renamed.ino()),
            (original.dev(), original.ino()),
            "expected {} to be a second name for {}",
            renamed_path.display(),
            binary.display()
        );
    }

    Ok((directory, renamed_path))
}

/// The version line the plainly built, plainly named binary reports for
/// `--version` — the ground truth every renamed or symlinked invocation in
/// this file is compared against. Reading it this way, rather than parsing
/// the workspace manifest's own version field, keeps this file honest about
/// what it actually checks: not "does some string equal some other string
/// I computed", but "does this exact binary, run under a different name,
/// report what it reports under its own".
fn plain_invocation_version_line() -> support::Outcome<String> {
    let working_dir = TempDir::new("plain-version")?;
    let result = run_ritual(working_dir.path(), &["--version"])?;
    result.expect_success("`--version` on the plainly built, plainly invoked binary");
    Ok(result.stdout)
}

/// The binary run under a second name must still report `ritual` for
/// `--version` — the compiled identity, not the name it happens to be
/// called on disk — and must never fall back to the crate's package name
/// (`rituals-cli`).
#[test]
fn version_reports_the_compiled_bin_name_through_a_hard_link() -> TestOutcome {
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

    let (_link_dir, renamed_path) = link_under_new_name(ritual_binary(), "totally-not-ritual")?;

    let renamed_result = run_binary_directly(&renamed_path, &["--version"])?;
    renamed_result.expect_success("`--version` on a hard link to the built binary");

    assert_eq!(
        renamed_result.stdout, ground_truth,
        "expected --version through a hard link to report exactly what the \
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
/// hard link too, and must agree with what the plainly invoked binary
/// prints.
#[test]
fn usage_line_reports_the_compiled_bin_name_through_a_hard_link() -> TestOutcome {
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

    let (_link_dir, renamed_path) = link_under_new_name(ritual_binary(), "totally-not-ritual")?;

    let renamed_result = run_binary_directly(&renamed_path, &[])?;
    assert_eq!(
        renamed_result.exit_code,
        Some(2),
        "expected a hard link with no subcommand to exit with clap's argument-error \
         status; stderr was:\n{}",
        renamed_result.stderr
    );

    assert_eq!(
        renamed_result.stderr, plain_result.stderr,
        "expected the Usage: line through a hard link to report exactly what the \
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

/// The same two properties again, through a symlink rather than a hard link —
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
/// for `--version`, the same way, through a hard link to its own built
/// binary — not merely when run plainly.
#[test]
fn version_on_a_freshly_scaffolded_projects_own_cli_survives_a_hard_link() -> TestOutcome {
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

        let (_link_dir, renamed_path) =
            link_under_new_name(&built_binary, "not-the-projects-name")?;

        let renamed_result = run_binary_directly(&renamed_path, &["--version"])?;
        renamed_result
            .expect_success("`--version` on a hard link to the scaffolded project's binary");
        assert_eq!(
            renamed_result.stdout, plain_result.stdout,
            "expected a hard link to the scaffolded project's binary to report exactly \
             what the plainly named build reports"
        );
        Ok(())
    })
}
