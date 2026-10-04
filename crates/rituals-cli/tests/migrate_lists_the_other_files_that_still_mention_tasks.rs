//! After it has moved the tasks and edited the manifests, `migrate` lists
//! every other file git tracks that still mentions `tasks/` — a CI workflow,
//! a script, the readme — and edits none of them: whether a path in a
//! workflow means the task directory is the person's to judge.
//!
//! The listing names files; how the sentence around each is worded is not
//! pinned. A tracked file that does not mention `tasks/`, and an ignored one
//! that does (git does not track it, so it is not part of the project's
//! history), are not listed.

mod support;

use support::migration::{assert_names, everything_written};
use support::{TempDir, TestOutcome, git, in_checkout, legacy, read_text, write_text};

/// Files that mention `tasks/`, tracked: where they are and what they say.
const MENTIONING: [(&str, &str); 3] = [
    (
        ".github/workflows/ci.yml",
        "steps:\n  - run: cargo test --manifest-path tasks/greet/Cargo.toml\n",
    ),
    (
        "readme.md",
        "# demo\n\nThe project's tooling is in tasks/greet.\n",
    ),
    ("scripts/check.sh", "#!/bin/sh\nls tasks/\n"),
];

#[test]
fn every_other_tracked_file_that_mentions_tasks_is_listed_and_left_alone() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("migrate-lists-mentions")?;
        let project = legacy::project_with_tasks(checkout, &working_dir, &["greet"])?;
        for (file, text) in MENTIONING {
            let path = project.root().join(file);
            std::fs::create_dir_all(support::process::parent_of(&path)?)?;
            write_text(&path, text)?;
        }
        write_text(
            &project.root().join("notes.txt"),
            "says nothing about the layout\n",
        )?;
        // Ignored, so never in the repository, though it mentions the path.
        let gitignore = project.root().join(".gitignore");
        write_text(&gitignore, &format!("{}*.log\n", read_text(&gitignore)?))?;
        write_text(&project.root().join("build.log"), "built tasks/greet\n")?;
        project.build()?;
        git::init_and_commit_everything(project.root())?;

        let migrated = project.run_cli(&["migrate"])?;
        migrated.expect_success("`migrate` on a project whose other files mention tasks/");

        // Named, and not edited.
        assert_names(
            &migrated,
            "`migrate`",
            &[".github/workflows/ci.yml", "readme.md", "scripts/check.sh"],
        );
        for (file, text) in MENTIONING {
            assert_eq!(
                read_text(&project.root().join(file))?,
                text,
                "expected `migrate` to leave {file} for the person to judge"
            );
        }

        // Not named: a tracked file with nothing to say about it, and an
        // ignored file git does not know.
        let written = everything_written(&migrated);
        for file in ["notes.txt", "build.log"] {
            assert!(
                !written.contains(file),
                "expected `migrate` not to list {file}; it wrote:\n{written}"
            );
        }
        Ok(())
    })
}
