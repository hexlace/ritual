//! `add` is `create`'s old name: inside a project it makes exactly what
//! `create` makes, says on standard error that it is going away, and
//! succeeds. Its `--help` line, and the bundle's list of commands, say it is
//! deprecated, so a person reading either learns it before running it.

mod support;

use support::created::{ADD_IS_NOW_CREATE, Audience, stdout_lines};
use support::tree::{assert_trees_identical, snapshot_tree};
use support::{Project, TempDir, TestOutcome, help, in_checkout, run_binary};

/// Scaffolds two identical projects, runs `add` in one and `create` in the other
/// with the same arguments, and asserts the reports, the trees and the
/// standard error agree as `add`'s deprecation requires.
fn assert_add_makes_what_create_makes(audience: Audience, prefix: &str) -> TestOutcome {
    in_checkout(|checkout| {
        let first_dir = TempDir::new(&format!("{prefix}-add"))?;
        let second_dir = TempDir::new(&format!("{prefix}-create"))?;
        let by_add = Project::scaffold(checkout, first_dir.path(), "demo", &[])?;
        let by_create = Project::scaffold(checkout, second_dir.path(), "demo", &[])?;

        let mut add_arguments = vec!["add", "lint"];
        add_arguments.extend_from_slice(audience.flags());
        let mut create_arguments = vec!["create", "lint"];
        create_arguments.extend_from_slice(audience.flags());

        let added = run_binary(&by_add.build()?, by_add.root(), &add_arguments)?;
        added.expect_success(&format!("`ritual {}`", add_arguments.join(" ")));
        assert_eq!(
            added
                .stderr
                .lines()
                .filter(|line| line.contains(ADD_IS_NOW_CREATE))
                .count(),
            1,
            "expected the deprecation on stderr exactly once; stderr was:\n{}",
            added.stderr
        );
        let created = run_binary(&by_create.build()?, by_create.root(), &create_arguments)?;
        created.expect_success(&format!("`ritual {}`", create_arguments.join(" ")));

        assert_eq!(
            stdout_lines(&added),
            stdout_lines(&created),
            "expected `add` to report what `create` reports"
        );
        assert_trees_identical(
            "`add lint` and `create lint` in identical projects",
            &snapshot_tree(by_create.root())?,
            &snapshot_tree(by_add.root())?,
        );
        assert!(
            !created.stderr.contains("deprecated") && !created.stderr.contains(ADD_IS_NOW_CREATE),
            "`create` is not deprecated; stderr was:\n{}",
            created.stderr
        );
        Ok(())
    })
}

#[test]
fn add_makes_the_tree_create_makes_and_says_it_is_going_away() -> TestOutcome {
    assert_add_makes_what_create_makes(Audience::Private, "add-is-create")
}

#[test]
fn add_public_makes_the_tree_create_public_makes() -> TestOutcome {
    assert_add_makes_what_create_makes(Audience::Public, "add-public-is-create")
}

#[test]
fn add_says_it_is_deprecated_in_its_help_and_in_the_bundles_list() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("add-help-deprecated")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;
        let binary = project.build()?;

        let listing = run_binary(&binary, project.root(), &["--help"])?;
        listing.expect_success("`ritual --help`");
        let add_row = listing
            .stdout
            .lines()
            .skip_while(|line| line.trim_end() != "Commands:")
            .skip(1)
            .find(|line| line.trim_start().starts_with("add "))
            .ok_or("the bundle's list of commands has no `add` row")?;
        assert!(
            add_row.to_lowercase().contains("deprecated"),
            "expected the `add` row of `--help` to say it is deprecated; the row was:\n{add_row}"
        );
        assert!(
            help::lists_command(&listing.stdout, "create"),
            "expected `create` still listed; stdout was:\n{}",
            listing.stdout
        );

        let own_help = run_binary(&binary, project.root(), &["add", "--help"])?;
        own_help.expect_success("`ritual add --help`");
        assert!(
            own_help.stdout.to_lowercase().contains("deprecated"),
            "expected `add --help` to say `add` is deprecated; stdout was:\n{}",
            own_help.stdout
        );
        Ok(())
    })
}
