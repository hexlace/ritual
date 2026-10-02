//! `cargo ritual remove <name>` refuses, before it writes anything, a name
//! that does not pick out exactly one task: one that is neither a key in
//! `tasks` nor a crate any key imports, and a crate that more than one key
//! imports, where the refusal names the keys so the person can choose.
//!
//! A refused run leaves the project byte-identical — every manifest, the
//! generated file, every task directory, and `Cargo.lock`, which reading
//! the project with `cargo metadata` creates or rewrites when it is missing
//! or behind.

mod support;

use support::removal::{
    assert_remove_is_refused_and_leaves_the_lockfile, assert_remove_is_refused_and_writes_nothing,
    project_with_a_committed_task,
};
use support::{TempDir, TestOutcome, git, in_checkout, manifest, run_ritual};

#[test]
fn a_name_that_is_neither_a_key_nor_an_imported_crate_is_refused() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-unknown-name")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;

        assert_remove_is_refused_and_writes_nothing(&project, "nosuch", &["nosuch"])?;

        // A workspace member that no key imports is not a task of this
        // project's command line, so it is not removable either, and its
        // directory is not deleted.
        project.write_leaf("orphan")?;
        git::commit_everything(project.root())?;
        assert_remove_is_refused_and_writes_nothing(&project, "orphan", &["orphan"])
    })
}

#[test]
fn a_crate_imported_under_more_than_one_key_is_refused_naming_the_keys() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refuses-shared-crate")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;

        // Cargo will not take one package under two names, so the two keys
        // import two releases of a crate with the same name, from two
        // directories outside the project: `shared-task` 0.1.0 and 0.2.0.
        for (key, version) in [("alpha", "0.1.0"), ("beta", "0.2.0")] {
            let parent = working_dir.path().join(format!("release-{version}"));
            std::fs::create_dir(&parent)?;
            run_ritual(
                &parent,
                &["create", "shared-task", "--path", checkout.path_argument()?],
            )?
            .expect_success("`ritual create shared-task --path <checkout>`");
            let crate_dir = parent.join("shared-task");
            manifest::edit(&crate_dir.join("Cargo.toml"), |document| {
                document["package"]["version"] = toml_edit::value(version);
                Ok(())
            })?;
            project.mount(&crate_dir, key, "shared-task")?;
        }
        project
            .alias(&["regenerate"])?
            .expect_success("`cargo ritual regenerate` after mounting two shared-task releases");
        git::commit_everything(project.root())?;

        assert_remove_is_refused_and_writes_nothing(&project, "shared-task", &["alpha", "beta"])
    })
}

/// A lockfile that has fallen behind the manifests — here, missing the
/// `greet` package the project depends on by path — is one `cargo metadata`
/// rewrites. The refusal puts it back as it was, stale.
#[test]
fn a_refusal_leaves_a_stale_lockfile_stale() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refusal-stale-lockfile")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;

        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "nosuch",
            &["nosuch"],
            |lockfile| {
                let current = std::fs::read_to_string(lockfile)?;
                let greet = "[[package]]\nname = \"greet\"\n";
                let start = current
                    .find(greet)
                    .ok_or("the built project's Cargo.lock has no `greet` package")?;
                let end = current[start..]
                    .find("\n\n")
                    .map_or(current.len(), |offset| start + offset + 2);
                std::fs::write(
                    lockfile,
                    format!("{}{}", &current[..start], &current[end..]),
                )?;
                Ok(())
            },
        )
    })
}

/// With no lockfile at all, `cargo metadata` writes one. The refusal takes
/// it away again.
#[test]
fn a_refusal_leaves_no_lockfile_where_there_was_none() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-refusal-no-lockfile")?;
        let project = project_with_a_committed_task(checkout, &working_dir, "greet")?;

        assert_remove_is_refused_and_leaves_the_lockfile(
            &project,
            "nosuch",
            &["nosuch"],
            |lockfile| {
                std::fs::remove_file(lockfile)?;
                Ok(())
            },
        )
    })
}
