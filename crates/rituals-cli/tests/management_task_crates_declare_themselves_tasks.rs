//! Ritual's management bundle and each of the four tasks in it are crates
//! that declare themselves tasks, under the names they are published as.
//!
//! `rituals-core` imports its four children in code, not through
//! `[package.metadata.ritual] tasks`, so nothing that builds this
//! repository reads their `task = true`. A project that imports one of them
//! directly — `rituals-core-add` on its own, under a key of its own choosing
//! — does, and `regenerate` refuses a crate that does not declare it. This
//! is what holds that declaration in place.

mod support;

use support::manifest;
use support::{Checkout, TestOutcome, in_checkout};

/// Asserts that the crate in `directory` of the checkout is published as
/// `package_name` and declares `[package.metadata.ritual] task = true`.
fn assert_declares_itself_a_task(
    checkout: &Checkout,
    directory: &str,
    package_name: &str,
) -> TestOutcome {
    let manifest_path = checkout.root().join(directory).join("Cargo.toml");
    let document = manifest::read(&manifest_path)?;

    assert_eq!(
        manifest::package_name(&document)?,
        package_name,
        "{}",
        manifest_path.display()
    );
    assert!(
        manifest::declares_itself_a_task(&document),
        "expected {} to declare `[package.metadata.ritual] task = true`",
        manifest_path.display()
    );

    Ok(())
}

#[test]
fn the_management_bundle_and_its_four_tasks_declare_themselves_tasks() -> TestOutcome {
    in_checkout(|checkout| {
        for (directory, package_name) in [
            ("crates/rituals-core", "rituals-core"),
            ("tasks/add", "rituals-core-add"),
            ("tasks/regenerate", "rituals-core-regenerate"),
            ("tasks/new", "rituals-core-new"),
            ("tasks/create", "rituals-core-create"),
        ] {
            assert_declares_itself_a_task(checkout, directory, package_name)?;
        }
        Ok(())
    })
}
