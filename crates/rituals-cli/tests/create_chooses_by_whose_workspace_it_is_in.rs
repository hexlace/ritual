//! `create` works in its project when the directory it runs in belongs to
//! the workspace of the command line running it. Anywhere else it asks Cargo
//! whether a crate made there would build on its own: under an ordinary
//! package with no `[workspace]` it would, so `create` makes a crate of its
//! own there, `--path` and all, as 0.1 did. Inside a declared workspace that
//! is not its own, or under a manifest Cargo cannot place in a workspace, it
//! would not, and `create` refuses before writing anything, with a remedy
//! that is true for that place: it never sends a person to a `cargo ritual`
//! a plain Cargo workspace does not have, and never to drop a `--path` that
//! would only lead to the next refusal.

mod support;

use std::path::Path;

use support::created::files_in;
use support::{
    TempDir, TestOutcome, assert_trees_identical, cargo, in_checkout, path_to_str, run_ritual,
    snapshot_tree, write_text,
};

/// A plain package, with no `[workspace]` table, at `root`.
fn a_plain_package(root: &Path) -> TestOutcome {
    std::fs::create_dir_all(root.join("src"))?;
    write_text(
        &root.join("Cargo.toml"),
        "[package]\nname = \"plain\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )?;
    write_text(&root.join("src/lib.rs"), "")
}

#[test]
fn under_a_plain_package_create_makes_a_crate_of_its_own_that_builds() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("create-under-a-plain-package")?;
        a_plain_package(working_dir.path())?;
        let work = working_dir.path().join("work");
        std::fs::create_dir(&work)?;
        let package_before = support::read_text(&working_dir.path().join("Cargo.toml"))?;

        let created = run_ritual(
            &work,
            &["create", "lint", "--path", checkout.path_argument()?],
        )?;
        created.expect_success("`ritual create lint --path <checkout>` under a plain package");
        assert_eq!(
            support::created::stdout_lines(&created)[..2],
            ["created lint/Cargo.toml", "created lint/src/lib.rs"]
        );
        let crate_dir = work.join("lint");
        assert_eq!(files_in(&crate_dir)?, ["Cargo.toml", "src/lib.rs"]);
        let manifest_path = crate_dir.join("Cargo.toml");
        cargo(
            &crate_dir,
            &crate_dir.join("target"),
            &["build", "--manifest-path", path_to_str(&manifest_path)?],
        )?
        .expect_success("building the crate `create` made under a plain package");

        // Without `--path` it is made the same way, taking `rituals` from
        // the registry, which this story does not reach for.
        run_ritual(&work, &["create", "other"])?
            .expect_success("`ritual create other` under a plain package");
        assert_eq!(files_in(&work.join("other"))?, ["Cargo.toml", "src/lib.rs"]);

        assert_eq!(
            package_before,
            support::read_text(&working_dir.path().join("Cargo.toml"))?,
            "the package above is not a project to `create`, so it is never written"
        );
        Ok(())
    })
}

/// Runs `create lint` in `directory`, with and without `--path`, asserts
/// both are refused with the same message, that `says` holds of it, and
/// that nothing under `root` changed.
fn assert_refused_alike_with_and_without_path(
    checkout: &support::Checkout,
    root: &Path,
    directory: &Path,
    says: impl Fn(&str) -> bool,
) -> TestOutcome {
    let before = snapshot_tree(root)?;
    let bare = run_ritual(directory, &["create", "lint"])?;
    bare.expect_failure("`ritual create lint`");
    let with_path = run_ritual(
        directory,
        &["create", "lint", "--path", checkout.path_argument()?],
    )?;
    with_path.expect_failure("`ritual create lint --path <checkout>`");

    assert_eq!(
        bare.stderr, with_path.stderr,
        "`--path` changes nothing about where a crate may be made, so the refusal is the same"
    );
    let message = bare.stderr.trim_end();
    assert!(
        message.starts_with("ritual: refusing `create lint`"),
        "expected ritual's own refusal; stderr was:\n{message}"
    );
    assert!(says(message), "the refusal said:\n{message}");
    assert!(
        !message.contains("cargo ritual") && !message.contains("--path"),
        "nothing here has a `cargo ritual`, and `--path` is not the problem; the refusal \
         said:\n{message}"
    );
    assert_trees_identical(
        "a refused `create` must write nothing",
        &before,
        &snapshot_tree(root)?,
    );
    Ok(())
}

/// Cargo's own words follow the refusal; they name the manifest it could
/// not read, however a given Cargo phrases the rest.
#[test]
fn under_an_empty_or_malformed_manifest_create_is_refused_in_cargos_words() -> TestOutcome {
    in_checkout(|checkout| {
        for (prefix, manifest) in [
            ("create-under-an-empty-manifest", ""),
            ("create-under-a-malformed-manifest", "[package\n"),
        ] {
            let working_dir = TempDir::new(prefix)?;
            write_text(&working_dir.path().join("Cargo.toml"), manifest)?;
            let work = working_dir.path().join("work");
            std::fs::create_dir(&work)?;

            assert_refused_alike_with_and_without_path(
                checkout,
                working_dir.path(),
                &work,
                |message| {
                    message.contains("cargo cannot resolve a workspace for that directory")
                        && message.contains("run create outside any Cargo workspace")
                        && message.contains("Cargo.toml")
                },
            )?;
        }
        Ok(())
    })
}

#[test]
fn in_a_plain_cargo_workspace_create_is_refused_naming_the_way_out_it_has() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("create-in-a-plain-workspace")?;
        let root = working_dir.path();
        write_text(
            &root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"member\"]\nresolver = \"3\"\n",
        )?;
        a_plain_package(&root.join("member"))?;
        let work = root.join("work");
        std::fs::create_dir(&work)?;

        let expected = format!(
            "ritual: refusing `create lint`: {} is inside the Cargo workspace rooted at {}, and \
             a crate made there would not build on its own; run create outside any Cargo \
             workspace",
            work.canonicalize()?.display(),
            root.canonicalize()?.display()
        );
        assert_refused_alike_with_and_without_path(checkout, root, &work, |message| {
            message == expected
        })
    })
}
