//! Every crate in this workspace is released at one version, together.
//!
//! `new` writes `rituals::VERSION` as the requirement for both `rituals` and
//! `rituals-core`, so a project created by one release needs both crates at
//! that release. That holds only while every member takes its version from
//! the workspace and every internal requirement names that same version;
//! this checks both, from the manifests, for every member Cargo lists.

mod support;

use std::path::{Path, PathBuf};

use support::process::cargo_query;
use support::{
    Checkout, OptionContext, Outcome, ResultContext, TestOutcome, in_checkout, manifest,
};

/// The workspace version, and every `[workspace.dependencies]` entry with a
/// `path` — ritual's own crates — whose requirement is not exactly it.
fn internal_requirements_off_the_workspace_version(
    root_manifest: &toml_edit::DocumentMut,
) -> Outcome<(String, Vec<String>, usize)> {
    let version = manifest::lookup(root_manifest, &["workspace", "package", "version"])
        .and_then(toml_edit::Item::as_str)
        .context("the root manifest declares no [workspace.package] version")?
        .to_string();
    let dependencies = manifest::lookup(root_manifest, &["workspace", "dependencies"])
        .and_then(toml_edit::Item::as_table_like)
        .context("the root manifest declares no [workspace.dependencies]")?;

    let mut internal = 0;
    let mut off = Vec::new();
    for (key, item) in dependencies.iter() {
        if item.get("path").is_none() {
            continue;
        }
        internal += 1;
        let requirement = item.get("version").and_then(toml_edit::Item::as_str);
        if requirement != Some(version.as_str()) {
            off.push(format!(
                "[workspace.dependencies] {key} requires {requirement:?}, not {version:?}"
            ));
        }
    }
    Ok((version, off, internal))
}

/// Every workspace member's manifest, as Cargo lists the members.
fn member_manifests(checkout: &Checkout) -> Outcome<Vec<PathBuf>> {
    let metadata = cargo_query(
        checkout.root(),
        &["metadata", "--no-deps", "--format-version", "1"],
    )?;
    metadata.expect_success("`cargo metadata --no-deps`");
    let document: serde_json::Value =
        serde_json::from_str(&metadata.stdout).context("cargo metadata printed invalid JSON")?;
    document["packages"]
        .as_array()
        .context("cargo metadata listed no packages")?
        .iter()
        .map(|package| {
            package["manifest_path"]
                .as_str()
                .map(PathBuf::from)
                .context("a package in cargo metadata has no manifest_path")
        })
        .collect()
}

/// What in one member's manifest lets its version, or the version of a
/// ritual crate it depends on, drift from the workspace's: a version of its
/// own, or a path dependency declared outside `[workspace.dependencies]`.
fn member_drift(manifest_path: &Path) -> Outcome<Vec<String>> {
    let document = manifest::read(manifest_path)?;
    let name = manifest::package_name(&document)?;
    let mut drift = Vec::new();

    let inherits = manifest::lookup(&document, &["package", "version", "workspace"])
        .and_then(toml_edit::Item::as_bool);
    if inherits != Some(true) {
        drift.push(format!("{name} does not declare version.workspace = true"));
    }
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        for key in manifest::keys_of(&document, &[table]) {
            if manifest::lookup(&document, &[table, &key, "path"]).is_some() {
                drift.push(format!(
                    "{name} declares [{table}] {key} by path itself, not through \
                     [workspace.dependencies]"
                ));
            }
        }
    }
    Ok(drift)
}

#[test]
fn every_member_and_every_internal_requirement_is_at_the_workspace_version() -> TestOutcome {
    in_checkout(|checkout| {
        let root_manifest = manifest::read(&checkout.root().join("Cargo.toml"))?;
        let (version, mut drift, internal) =
            internal_requirements_off_the_workspace_version(&root_manifest)?;
        assert!(
            internal > 0,
            "expected [workspace.dependencies] to declare ritual's own crates by path"
        );

        let members = member_manifests(checkout)?;
        assert!(
            !members.is_empty(),
            "expected cargo metadata to list members"
        );
        for manifest_path in &members {
            drift.extend(member_drift(manifest_path)?);
        }

        assert!(
            drift.is_empty(),
            "every crate must release at the workspace version {version}:\n{}",
            drift.join("\n")
        );
        Ok(())
    })
}
