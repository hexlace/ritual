//! `cargo ritual remove` never removes ritual's own bundle, `rituals-core`,
//! whichever key imports it: removing it would take away `add`,
//! `regenerate` and the rest, the commands that put things back.
//!
//! The guard is on the package, not on the key `ritual` it is usually
//! imported under, so importing the bundle under another name does not get
//! around it.

mod support;

use std::path::Path;
use support::removal::{
    assert_invocation_is_refused_and_writes_nothing, assert_remove_is_refused_and_writes_nothing,
    project_with_a_committed_task,
};

use support::{
    Project, TempDir, TestOutcome, git, in_checkout, manifest, path_to_str, read_text, write_text,
};

#[test]
fn the_bundle_under_its_usual_key_is_refused() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-bundle-usual-key")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;

        assert_remove_is_refused_and_writes_nothing(&project, "ritual", &["rituals-core"])
    })
}

/// Renames the bundle's key, its dependency and its mount in the generated
/// file from `ritual` to `new_key`, each exactly once.
fn rename_the_bundle_key(project: &Project, new_key: &str) -> TestOutcome {
    let replace_once = |path: &Path, from: &str, to: &str| -> TestOutcome {
        let text = read_text(path)?;
        assert_eq!(
            text.matches(from).count(),
            1,
            "fixture precondition: expected `{from}` exactly once in {}",
            path.display()
        );
        write_text(path, &text.replace(from, to))
    };
    let cli_manifest = project.cli_manifest_path();
    replace_once(
        &cli_manifest,
        "ritual = { package = \"rituals-core\"",
        &format!("{new_key} = {{ package = \"rituals-core\""),
    )?;
    manifest::edit(&cli_manifest, |document| {
        manifest::remove_task(document, "ritual")?;
        manifest::push_task(document, new_key)
    })?;
    replace_once(
        &project.generated_file_path(),
        "(\"ritual\", ritual::task())",
        &format!("(\"{new_key}\", {new_key}::task())"),
    )
}

#[test]
fn the_bundle_under_another_key_is_refused_too() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-bundle-other-key")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;

        // Move the bundle from `ritual` to `tools` by hand, in one step:
        // Cargo will not take one package under two names, so the key, the
        // dependency and the generated mount are renamed together. `tools`
        // is then the only key whose package is `rituals-core`, so the
        // refusal can only be the guard on the package. Mounted under a key
        // other than the bin name, the bundle nests: `remove` is reached as
        // `tools remove`.
        let bundle_dir = checkout.root().join(".rituals/ritual");
        rename_the_bundle_key(&project, "tools")?;
        git::commit_everything(project.root())?;

        let cli_manifest = project.cli_manifest()?;
        assert_eq!(
            manifest::string_at(&cli_manifest, &["dependencies", "tools", "package"]),
            Some("rituals-core"),
            "fixture precondition: `tools` should import rituals-core; manifest \
             was:\n{cli_manifest}"
        );
        assert_eq!(
            manifest::string_at(&cli_manifest, &["dependencies", "tools", "path"]),
            Some(path_to_str(&bundle_dir)?),
            "fixture precondition: `tools` should come from this checkout; manifest \
             was:\n{cli_manifest}"
        );

        assert_invocation_is_refused_and_writes_nothing(
            &project,
            &["tools", "remove", "tools"],
            &["rituals-core"],
        )
    })
}
