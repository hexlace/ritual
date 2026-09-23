//! Running `regenerate` against this repository's own composed CLI
//! reproduces the file already checked in, byte for byte, and changes
//! nothing else. That composed CLI depends on `rituals`, mounts ritual's
//! management bundle under the key `ritual` as its one task, and its
//! generated file calls the framework's dispatcher and identity macro
//! directly.
//!
//! The test never touches this repository's own working tree. It copies the
//! tree as it would be committed — `git ls-files -z --cached --others
//! --exclude-standard`, piped through `tar` — into a temporary directory and
//! runs there. Copying the working tree rather than archiving `HEAD` means
//! uncommitted changes are what gets tested, so the test answers for the
//! code on disk, committed or not. `target/` and everything else gitignored
//! stay out of the copy by construction.
//!
//! Needs `git` and `tar` on `PATH`, and a git checkout to list files from;
//! it skips, and says so, without one.

mod support;

use std::path::Path;
use std::process::{Command, Stdio};

use support::process::built_binary_path;
use support::{
    Checkout, OptionContext, ResultContext, TempDir, TestOutcome, assert_trees_identical, cargo,
    checkout, in_checkout, read_text, run_binary, snapshot_tree,
};
use support::{generated, manifest};

/// Copies the working tree as it would be committed — tracked files plus
/// untracked files that are not gitignored — into `destination`, which must
/// already exist and be empty.
///
/// Chains three processes the way the equivalent shell pipeline would:
/// `git ls-files -z --cached --others --exclude-standard | tar --null -T -
/// -c -f - | tar -x -C destination`. Each process's stdout is handed to the
/// next before the next is spawned, so nothing waits on a pipe nobody is
/// reading.
fn copy_working_tree_as_it_would_be_committed(
    checkout: &Checkout,
    destination: &Path,
) -> TestOutcome {
    let mut list_files = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(checkout.root())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .context("spawning `git ls-files` failed")?;
    let list_files_stdout = list_files
        .stdout
        .take()
        .context("git ls-files's stdout was not piped")?;

    let mut build_archive = Command::new("tar")
        .args(["--null", "-T", "-", "-c", "-f", "-"])
        .current_dir(checkout.root())
        .stdin(list_files_stdout)
        .stdout(Stdio::piped())
        .spawn()
        .context("spawning `tar -c` failed")?;
    let build_archive_stdout = build_archive
        .stdout
        .take()
        .context("tar -c's stdout was not piped")?;

    let extract_status = Command::new("tar")
        .args(["-x", "-C"])
        .arg(destination)
        .stdin(build_archive_stdout)
        .status()
        .context("running `tar -x` failed")?;
    let build_archive_status = build_archive
        .wait()
        .context("waiting for `tar -c` failed")?;
    let list_files_status = list_files
        .wait()
        .context("waiting for `git ls-files` failed")?;

    assert!(
        list_files_status.success(),
        "`git ls-files` failed with {list_files_status}"
    );
    assert!(
        build_archive_status.success(),
        "`tar -c` failed with {build_archive_status}"
    );
    assert!(
        extract_status.success(),
        "`tar -x` failed with {extract_status}"
    );
    assert!(
        destination.join("Cargo.toml").is_file(),
        "expected the copied tree to contain the workspace manifest at its root"
    );

    Ok(())
}

/// Builds the copy's `ritual` binary into the copy's own `target/` and runs
/// `regenerate` with it.
fn regenerate_the_copy(copy_root: &Path) -> TestOutcome {
    let target_dir = copy_root.join("target");
    cargo(copy_root, &target_dir, &["build", "--bin", "ritual"])?
        .expect_success("building the copy's `ritual` binary");
    run_binary(
        &built_binary_path(&target_dir, "ritual"),
        copy_root,
        &["regenerate"],
    )?
    .expect_success("`ritual regenerate` against a copy of the working tree");
    Ok(())
}

/// The copy's composed CLI manifest: a `rituals` dependency inherited from
/// the workspace, no `rituals-compose` in any dependency table, ritual's
/// bundle imported under `ritual`, and `ritual` as its one task.
fn assert_the_manifest_shape(copy_root: &Path) -> TestOutcome {
    let cli_manifest = manifest::read(&copy_root.join("crates/rituals-cli/Cargo.toml"))?;

    assert_eq!(
        manifest::lookup(&cli_manifest, &["dependencies", "rituals", "workspace"])
            .and_then(toml_edit::Item::as_bool),
        Some(true),
        "expected `rituals-cli` to depend on `rituals` from the workspace; manifest \
         was:\n{cli_manifest}"
    );
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        assert!(
            !manifest::keys_of(&cli_manifest, &[table]).contains(&"rituals-compose".to_string()),
            "expected `rituals-cli` to depend on nothing from the composition library; \
             [{table}] names it"
        );
    }
    let workspace_manifest = manifest::read(&copy_root.join("Cargo.toml"))?;
    assert_eq!(
        manifest::dependency_package(&cli_manifest, &workspace_manifest, "ritual").as_deref(),
        Some("rituals-core"),
        "expected ritual's management bundle imported under the key `ritual`; manifest \
         was:\n{cli_manifest}"
    );
    assert_eq!(manifest::tasks(&cli_manifest)?, ["ritual"]);

    Ok(())
}

/// The copy's generated file calls `rituals::run` with `rituals::identity!()`
/// directly, reaches nothing through `rituals_compose`, and mounts exactly
/// one entry, the bundle. Compared with whitespace removed, so the check is
/// about the tokens the file carries rather than how they are laid out.
fn assert_the_generated_file_shape(copy_root: &Path) -> TestOutcome {
    let generated_file = read_text(&copy_root.join("crates/rituals-cli/src/main.rs"))?;
    let tokens = generated::tokens(&generated_file);

    assert!(
        tokens.contains("rituals::run(rituals::identity!(),["),
        "expected the generated file to call the framework's dispatcher and identity macro \
         directly; file was:\n{generated_file}"
    );
    assert!(
        !tokens.contains("rituals_compose::"),
        "expected the generated file to reach nothing through the composition library; \
         file was:\n{generated_file}"
    );
    assert_eq!(
        generated::mounted_entries(&generated_file),
        [("ritual".to_string(), "ritual".to_string())],
        "expected exactly one mounted entry, ritual's bundle under `ritual`; file \
         was:\n{generated_file}"
    );

    Ok(())
}

#[test]
fn regenerating_this_repositorys_own_composed_cli_changes_nothing() -> TestOutcome {
    in_checkout(|checkout| {
        if !checkout.root().join(".git").exists() {
            checkout::report_skip("it lists the files to copy with `git ls-files`");
            return Ok(());
        }

        let copy = TempDir::new("own-repo-regenerate")?;
        copy_working_tree_as_it_would_be_committed(checkout, copy.path())?;

        let before = snapshot_tree(copy.path())?;
        regenerate_the_copy(copy.path())?;
        let after = snapshot_tree(copy.path())?;
        assert_trees_identical(
            "regenerating a clean copy of this repository's own working tree",
            &before,
            &after,
        );

        assert_the_manifest_shape(copy.path())?;
        assert_the_generated_file_shape(copy.path())?;

        Ok(())
    })
}
