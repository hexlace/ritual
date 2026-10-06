//! Whose workspace a directory is in, asked of Cargo without resolving
//! anything, so asking writes nothing.

use std::path::Path;

use rituals::Failure;
use serde::Deserialize;

use super::SUPPORTED_FORMAT_VERSION;
use crate::cargo;
use crate::manifest;
use crate::workspace::{Located, locate_project};

/// Which command line, if any, owns the Cargo workspace a directory is in.
///
/// What [`whose_workspace`] answers, for a task that works in its own
/// project and does something else everywhere else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Owner {
    /// The asking command line's package is a member: this is its own
    /// project.
    TheCommandLine,
    /// The workspace is declared with a `[workspace]` table, and a member
    /// other than the asking command line declares a `tasks` list: this is
    /// another composed command line's project.
    AnotherCommandLine,
    /// No command line owns it: Cargo finds no workspace here, or one with
    /// no member that declares a `tasks` list, or a single package that
    /// declares no `[workspace]` table at all.
    NoCommandLine,
}

/// The part of `cargo metadata --no-deps`'s output this question reads.
#[derive(Deserialize)]
struct Membership {
    version: u64,
    workspace_members: Vec<String>,
    packages: Vec<MemberPackage>,
}

/// One package of [`Membership`].
#[derive(Deserialize)]
struct MemberPackage {
    id: String,
    name: String,
    #[serde(default)]
    metadata: serde_json::Value,
}

/// Asks Cargo which command line, if any, owns the workspace `current_dir`
/// is in, where the asking command line's package is `package_name`.
///
/// Its own project is the same test [`super::fetch_in_its_own_project`]
/// makes: a workspace member is called `package_name`. It is asked with
/// `cargo locate-project` and then `cargo metadata --no-deps`, neither of
/// which resolves dependencies, so neither writes `Cargo.lock` or reaches a
/// registry, and a task asks before its run begins.
///
/// # Errors
///
/// Returns a [`Failure`] saying so when `cargo` cannot be run, and one with
/// Cargo's own words when it places `current_dir` in a workspace but cannot
/// read that workspace's members.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// use rituals_compose::metadata::{self, Owner};
///
/// // Runs `cargo` against the directory on disk, so this example is
/// // `no_run`.
/// match metadata::whose_workspace(Path::new("."), "demo-ritual")? {
///     Owner::TheCommandLine => println!("scaffold into this project"),
///     Owner::AnotherCommandLine => println!("refuse: run that project's own"),
///     Owner::NoCommandLine => println!("ask whether a crate stands alone here"),
/// }
/// # Ok::<(), rituals::Failure>(())
/// ```
pub fn whose_workspace(current_dir: &Path, package_name: &str) -> Result<Owner, Failure> {
    let root_manifest = match locate_project(current_dir, true)? {
        Located::Found(root_manifest) => root_manifest,
        Located::NotFound(_no_workspace) => return Ok(Owner::NoCommandLine),
    };
    let members = fetch_members(current_dir)?;
    Ok(owner(
        &members,
        package_name,
        manifest::declares_a_workspace(&root_manifest),
    ))
}

/// Who owns a workspace whose members are `members`, for the command line
/// whose package is `package_name`; `declared` is whether its root manifest
/// has a `[workspace]` table.
fn owner(members: &Membership, package_name: &str, declared: bool) -> Owner {
    let mut member_packages = members
        .packages
        .iter()
        .filter(|package| members.workspace_members.contains(&package.id));
    if member_packages
        .clone()
        .any(|package| package.name == package_name)
    {
        return Owner::TheCommandLine;
    }
    if declared && member_packages.any(declares_a_task_list) {
        return Owner::AnotherCommandLine;
    }
    Owner::NoCommandLine
}

/// Whether `package` declares `[package.metadata.ritual] tasks`, which only
/// a composed command line does.
fn declares_a_task_list(package: &MemberPackage) -> bool {
    package
        .metadata
        .get("ritual")
        .and_then(|ritual| ritual.get("tasks"))
        .is_some()
}

/// Runs `cargo metadata --no-deps --format-version 1` in `current_dir` and
/// reads its members.
fn fetch_members(current_dir: &Path) -> Result<Membership, Failure> {
    let output = cargo::command()
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(current_dir)
        .output()
        .map_err(|error| Failure::new("running `cargo metadata` failed").caused_by(error))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Failure::new(format!(
            "cargo metadata failed: {}",
            stderr.trim_end()
        )));
    }
    parse_members(&output.stdout)
}

/// Parses `cargo metadata --no-deps --format-version 1`'s output, as far as
/// [`Membership`] reads it.
fn parse_members(document: &[u8]) -> Result<Membership, Failure> {
    let members: Membership = serde_json::from_slice(document)
        .map_err(|error| Failure::new("parsing cargo metadata output failed").caused_by(error))?;
    if members.version != SUPPORTED_FORMAT_VERSION {
        return Err(Failure::new(format!(
            "cargo metadata returned format version {}, but this framework understands only \
             version {SUPPORTED_FORMAT_VERSION}",
            members.version
        )));
    }
    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::{Owner, owner, parse_members, whose_workspace};
    use crate::test_support::{ScratchDir, TestOutcome};

    /// A `--no-deps` document with `cli` (declaring `tasks`), `task` and
    /// `outside`, of which `cli` and `task` are members.
    const DOCUMENT: &str = r#"{
        "version": 1,
        "workspace_members": ["cli-id", "task-id"],
        "resolve": null,
        "packages": [
            {"id": "cli-id", "name": "demo-ritual", "metadata": {"ritual": {"tasks": []}}},
            {"id": "task-id", "name": "lint", "metadata": {"ritual": {"task": true}}},
            {"id": "outside-id", "name": "other-ritual", "metadata": {"ritual": {"tasks": []}}}
        ]
    }"#;

    #[test]
    fn a_member_named_after_the_command_line_makes_it_its_own() -> TestOutcome {
        let members = parse_members(DOCUMENT.as_bytes()).map_err(|failure| failure.to_string())?;
        assert_eq!(owner(&members, "demo-ritual", true), Owner::TheCommandLine);
        assert_eq!(owner(&members, "demo-ritual", false), Owner::TheCommandLine);
        Ok(())
    }

    /// Only a member counts: `other-ritual` declares `tasks` but is not one.
    #[test]
    fn a_declared_workspace_with_another_task_list_is_another_command_lines() -> TestOutcome {
        let members = parse_members(DOCUMENT.as_bytes()).map_err(|failure| failure.to_string())?;
        assert_eq!(
            owner(&members, "other-ritual", true),
            Owner::AnotherCommandLine
        );
        assert_eq!(
            owner(&members, "lint-ritual", true),
            Owner::AnotherCommandLine
        );
        Ok(())
    }

    #[test]
    fn with_no_workspace_table_or_no_task_list_no_command_line_owns_it() -> TestOutcome {
        let members = parse_members(DOCUMENT.as_bytes()).map_err(|failure| failure.to_string())?;
        assert_eq!(owner(&members, "other-ritual", false), Owner::NoCommandLine);

        let without_a_task_list = DOCUMENT.replace(r#"{"ritual": {"tasks": []}}"#, "null");
        let members =
            parse_members(without_a_task_list.as_bytes()).map_err(|failure| failure.to_string())?;
        assert_eq!(owner(&members, "other-ritual", true), Owner::NoCommandLine);
        Ok(())
    }

    #[test]
    fn another_format_version_is_refused() {
        let failure = parse_members(
            DOCUMENT
                .replace(r#""version": 1"#, r#""version": 2"#)
                .as_bytes(),
        )
        .err()
        .map(|failure| failure.to_string());
        assert_eq!(
            failure.as_deref(),
            Some(
                "cargo metadata returned format version 2, but this framework understands only \
                 version 1"
            )
        );
    }

    #[test]
    fn a_directory_in_no_workspace_is_owned_by_no_command_line() -> TestOutcome {
        let scratch = ScratchDir::new("owner-no-workspace")?;
        assert_eq!(
            whose_workspace(scratch.path(), "demo-ritual").map_err(|failure| failure.to_string())?,
            Owner::NoCommandLine
        );
        Ok(())
    }

    /// Asking reads members without resolving them, so a workspace that has
    /// never been resolved is still without a lockfile afterwards.
    #[test]
    fn asking_writes_no_lockfile() -> TestOutcome {
        let scratch = ScratchDir::new("owner-writes-nothing")?;
        std::fs::create_dir_all(scratch.path().join("member/src"))?;
        std::fs::write(
            scratch.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"member\"]\nresolver = \"3\"\n",
        )?;
        std::fs::write(
            scratch.path().join("member/Cargo.toml"),
            "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        std::fs::write(scratch.path().join("member/src/lib.rs"), "")?;

        assert_eq!(
            whose_workspace(&scratch.path().join("member"), "member")
                .map_err(|failure| failure.to_string())?,
            Owner::TheCommandLine
        );
        assert!(
            !scratch.path().join("Cargo.lock").exists(),
            "asking whose workspace this is must not resolve it"
        );
        Ok(())
    }
}
