//! A ritual `create` makes is private by default, and Cargo is what holds it
//! to that: `cargo publish --dry-run` refuses it, naming the `publish` key,
//! before it needs a registry. With `--public` the manifest has no `publish`
//! key, so that refusal is not Cargo's reason for stopping.
//!
//! Nothing here reaches a registry. Cargo is run `--offline` with an empty
//! `CARGO_HOME`, so no registry index or cached crate can answer for it. The
//! publish check comes first, so the private ritual is refused with Cargo's
//! own words; a public one gets past that check and then stops at the
//! registry it is not allowed to read, so a full public dry-run cannot be
//! shown here. What this shows of a public ritual is that Cargo does not
//! stop it for `publish`; the private ritual's refusal, from the same
//! command in the same place, is the control that the check can speak.

mod support;

use std::path::Path;

use support::process::cargo_without_a_registry;
use support::{Project, RunOutput, TempDir, TestOutcome, in_checkout, run_binary, run_ritual};

/// `cargo publish --dry-run` in `crate_dir`, off the network.
fn dry_run(crate_dir: &Path, working_dir: &TempDir) -> support::Outcome<RunOutput> {
    let cargo_home = TempDir::new_in(working_dir.path(), "empty-cargo-home")?;
    cargo_without_a_registry(
        crate_dir,
        &crate_dir.join("target"),
        cargo_home.path(),
        &["publish", "--dry-run"],
    )
}

/// The dry run's refusal is Cargo's `publish` one: it names the crate as one
/// that cannot be published and the key that says so, and it says nothing
/// about a registry, which is how this shows none was asked.
#[track_caller]
fn assert_refused_for_publish(output: &RunOutput, name: &str) {
    output.expect_failure(&format!("`cargo publish --dry-run` of private `{name}`"));
    assert!(
        output
            .stderr
            .contains(&format!("`{name}` cannot be published"))
            && output.stderr.contains("package.publish"),
        "expected Cargo's refusal to publish `{name}`, naming `package.publish`; stderr \
         was:\n{}",
        output.stderr
    );
    assert!(
        !output.stderr.to_lowercase().contains("registry"),
        "expected the refusal to come before Cargo needed a registry; stderr was:\n{}",
        output.stderr
    );
}

/// A public ritual is not stopped by the `publish` key: whatever else Cargo
/// says offline, it does not say that.
#[track_caller]
fn assert_not_refused_for_publish(output: &RunOutput) {
    assert!(
        !output.stderr.contains("cannot be published")
            && !output.stderr.contains("package.publish"),
        "expected Cargo not to refuse over the `publish` key; stderr was:\n{}",
        output.stderr
    );
}

#[test]
fn a_standalone_private_ritual_is_refused_by_cargo_publish() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("publish-standalone-private")?;
        run_ritual(
            working_dir.path(),
            &["create", "lint", "--path", checkout.path_argument()?],
        )?
        .expect_success("`ritual create lint`");

        assert_refused_for_publish(
            &dry_run(&working_dir.path().join("lint"), &working_dir)?,
            "lint",
        );
        Ok(())
    })
}

#[test]
fn a_standalone_public_ritual_is_not_refused_for_its_publish_key() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("publish-standalone-public")?;
        run_ritual(
            working_dir.path(),
            &[
                "create",
                "lint",
                "--public",
                "--path",
                checkout.path_argument()?,
            ],
        )?
        .expect_success("`ritual create lint --public`");

        assert_not_refused_for_publish(&dry_run(&working_dir.path().join("lint"), &working_dir)?);
        Ok(())
    })
}

#[test]
fn a_private_ritual_made_inside_a_project_is_refused_by_cargo_publish() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("publish-in-project-private")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        run_binary(&project.build()?, project.root(), &["create", "lint"])?
            .expect_success("`ritual create lint` in a project");

        assert_refused_for_publish(
            &dry_run(&project.root().join(".rituals/lint"), &working_dir)?,
            "lint",
        );
        Ok(())
    })
}

#[test]
fn a_public_ritual_made_inside_a_project_is_not_refused_for_its_publish_key() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("publish-in-project-public")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        run_binary(
            &project.build()?,
            project.root(),
            &["create", "lint", "--public"],
        )?
        .expect_success("`ritual create lint --public` in a project");

        assert_not_refused_for_publish(&dry_run(
            &project.root().join(".rituals/lint"),
            &working_dir,
        )?);
        Ok(())
    })
}
