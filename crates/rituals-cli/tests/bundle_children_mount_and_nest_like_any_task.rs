//! A task can be a bundle — a named group of child commands — and a
//! composed CLI mounts a bundle under a key exactly the way it mounts any
//! other task. Bundles nest: a bundle may contain another bundle, to any
//! depth, and a command several levels deep runs when its full path is
//! typed. A bundle reached through its own key lists its children as
//! subcommands under that key, the same shape the top level has.
//!
//! Everything here is mounted under an ordinary key, `daily`, never the
//! binary's own name; flattening is `bin_name_flattening_and_rename_toggles_it.rs`.
//!
//! Every crate this file writes depends on `rituals` and nothing else from
//! this framework (see `support::crates`), so the project building at all
//! shows a bundle needs nothing beyond `rituals` to be declared and
//! dispatched.

mod support;

use support::{Child, Project, TempDir, TestOutcome, help, in_checkout};

/// Writes three leaves (`wake`, `shop`, `pickup`), a bundle (`chores`)
/// grouping the first two, and a bundle (`errands`) grouping `chores` and
/// `pickup` — two levels of nesting — then mounts `errands` at the top
/// level under `daily` and regenerates.
fn write_and_mount_the_nested_bundle(project: &Project) -> TestOutcome {
    for leaf in ["wake", "shop", "pickup"] {
        project.write_leaf(leaf)?;
    }
    project.write_bundle(
        "chores",
        "housework",
        &[
            Child::Crate {
                key: "wake",
                crate_name: "wake",
            },
            Child::Crate {
                key: "shop",
                crate_name: "shop",
            },
        ],
    )?;
    let errands_dir = project.write_bundle(
        "errands",
        "the day's plan",
        &[
            Child::Crate {
                key: "chores",
                crate_name: "chores",
            },
            Child::Crate {
                key: "pickup",
                crate_name: "pickup",
            },
        ],
    )?;
    project.mount(&errands_dir, "daily", "errands")?;

    project
        .run_cli(&["regenerate"])?
        .expect_success("`regenerate` after mounting the nested bundle");
    Ok(())
}

/// Asserts that `--help` for `path` lists exactly `expected`, then clap's
/// own `help`.
fn assert_help_lists(project: &Project, path: &[&str], expected: &[&str]) -> TestOutcome {
    let mut arguments = path.to_vec();
    arguments.push("--help");
    let help = project.run_cli(&arguments)?;
    help.expect_success(&format!("`{}`", arguments.join(" ")));

    let mut expected = expected.to_vec();
    expected.push("help");
    assert_eq!(
        help::command_names(&help.stdout),
        expected,
        "`{}` listed the wrong commands; stdout was:\n{}",
        arguments.join(" "),
        help.stdout
    );
    Ok(())
}

/// Asserts that `path` runs the leaf `leaf`.
fn assert_runs_leaf(project: &Project, path: &[&str], leaf: &str) -> TestOutcome {
    let result = project.run_cli(path)?;
    result.expect_success(&format!("`{}`", path.join(" ")));
    assert!(
        result.stdout.contains(&format!("{leaf} ran")),
        "expected `{}` to run `{leaf}`; stdout was:\n{}",
        path.join(" "),
        result.stdout
    );
    Ok(())
}

#[test]
fn a_nested_bundle_mounts_like_any_task_and_dispatches_through_every_level() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("bundle-nesting")?;
        let project = Project::scaffold(checkout, working_dir.path(), "nesting-demo", &[])?;

        write_and_mount_the_nested_bundle(&project)?;

        // The top level shows `daily` beside ritual's own flattened tasks,
        // and none of `daily`'s descendants.
        assert_help_lists(
            &project,
            &[],
            &["add", "regenerate", "new", "create", "daily"],
        )?;
        // Each bundle lists its immediate children, and not theirs.
        assert_help_lists(&project, &["daily"], &["chores", "pickup"])?;
        assert_help_lists(&project, &["daily", "chores"], &["wake", "shop"])?;
        // Three levels deep and two levels deep both dispatch to their leaf.
        assert_runs_leaf(&project, &["daily", "chores", "wake"], "wake")?;
        assert_runs_leaf(&project, &["daily", "pickup"], "pickup")?;

        Ok(())
    })
}
