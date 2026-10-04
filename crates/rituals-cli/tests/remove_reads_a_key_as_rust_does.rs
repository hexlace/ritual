//! A key and its dependency line are matched the way Rust reads them, with
//! `-` and `_` as one name, for `remove` as for `regenerate`. A project that
//! lists `chore-job` in `tasks` over a dependency written `chore_job`
//! builds, because Cargo gives both spellings one extern crate name, so
//! `remove chore-job` takes that dependency line out with the key.

mod support;

use support::removal::{assert_help_lists, exists};
use support::{TempDir, TestOutcome, generated, in_checkout, manifest, run_ritual};

#[test]
fn a_key_spelled_with_a_hyphen_takes_its_underscored_dependency_line_with_it() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("remove-reads-a-key-as-rust-does")?;
        let work = working_dir.path().join("work");
        std::fs::create_dir(&work)?;
        let project = support::Project::scaffold(checkout, &work, "demo", &[])?;

        let external = working_dir.path().join("external");
        std::fs::create_dir(&external)?;
        run_ritual(
            &external,
            &["create", "chore", "--path", checkout.path_argument()?],
        )?
        .expect_success("`ritual create chore --path <checkout>`");
        let crate_dir = external.join("chore");

        project.mount(&crate_dir, "chore_job", "chore")?;
        manifest::edit(&project.cli_manifest_path(), |document| {
            manifest::remove_task(document, "chore_job")?;
            manifest::push_task(document, "chore-job")
        })?;
        project
            .alias(&["regenerate"])?
            .expect_success("`cargo ritual regenerate` with `chore-job` over `chore_job`");
        assert_help_lists(
            &project,
            "importing `chore-job`",
            &[
                "add",
                "regenerate",
                "new",
                "create",
                "import",
                "remove",
                "migrate",
                "chore-job",
                "help",
            ],
        )?;

        project
            .run_cli(&["remove", "chore-job"])?
            .expect_success("`cargo ritual remove chore-job` over a `chore_job` dependency");

        let cli_manifest = project.cli_manifest()?;
        assert_eq!(
            manifest::tasks(&cli_manifest)?,
            ["ritual"],
            "expected `chore-job` to leave `tasks`; manifest was:\n{cli_manifest}"
        );
        assert!(
            manifest::lookup(&cli_manifest, &["dependencies", "chore_job"]).is_none(),
            "expected the `chore_job` dependency line to go with `chore-job`; manifest \
             was:\n{cli_manifest}"
        );
        assert_eq!(
            generated::mounted_entries(&project.generated_file()?),
            [("ritual".to_string(), "ritual".to_string())],
            "expected the regenerated file to mount the bundle alone"
        );
        assert!(
            exists(&crate_dir),
            "expected the crate outside the workspace to stay"
        );
        assert_help_lists(
            &project,
            "`remove chore-job`",
            &[
                "add",
                "regenerate",
                "new",
                "create",
                "import",
                "remove",
                "migrate",
                "help",
            ],
        )
    })
}
