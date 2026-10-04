//! What ritual prints is the part of its documentation people read most, so
//! it is read here exactly as the program writes it: `new`, `add` and
//! `create` each end by naming the next step as a command a person can
//! copy, `add` and `regenerate` name the tasks a generated file mounts
//! rather than counting them, and a refusal names its remedy in the same
//! copyable form.
//!
//! Every line is read from a binary run directly, never through `cargo
//! run`, whose own progress lines share stderr.

mod support;

use support::{Project, TempDir, TestOutcome, in_checkout, path_to_str, read_text, run_ritual};

/// The lines a successful run wrote to stdout.
fn lines(stdout: &str) -> Vec<&str> {
    stdout.lines().collect()
}

#[test]
fn a_default_project_reports_each_step_and_names_the_next_one() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("output-default")?;
        let scaffolded = run_ritual(
            working_dir.path(),
            &["new", "demo", "--path", checkout.path_argument()?],
        )?;
        scaffolded.expect_success("`ritual new demo`");
        assert_eq!(
            lines(&scaffolded.stdout),
            [
                "created demo/Cargo.toml",
                "created demo/.gitignore",
                "created demo/.cargo/config.toml",
                "created demo/ritual/Cargo.toml",
                "created demo/ritual/src/main.rs",
                "next: cd demo && cargo ritual --help",
            ]
        );
        let project_root = working_dir.path().join("demo");
        assert_eq!(read_text(&project_root.join(".gitignore"))?, "/target\n");

        let project = Project::open(&project_root)?;
        let binary = project.build()?;

        let added = support::run_binary(&binary, project.root(), &["add", "hello"])?;
        added.expect_success("`ritual add hello`");
        assert_eq!(
            lines(&added.stdout),
            [
                "created .rituals/hello/Cargo.toml",
                "created .rituals/hello/src/lib.rs",
                "updated Cargo.toml",
                "updated ritual/Cargo.toml",
                "updated ritual/src/main.rs (tasks: ritual, hello)",
                "next: edit .rituals/hello/src/lib.rs, then run cargo ritual hello",
            ]
        );

        let regenerated = support::run_binary(&binary, project.root(), &["regenerate"])?;
        regenerated.expect_success("`ritual regenerate` with nothing changed");
        assert_eq!(
            lines(&regenerated.stdout),
            ["ritual/src/main.rs is already up to date (tasks: ritual, hello)"]
        );

        let repeated = support::run_binary(&binary, project.root(), &["add", "hello"])?;
        repeated.expect_failure("a second `ritual add hello`");
        assert_eq!(
            repeated.sole_line_prefixed_with("ritual"),
            "`hello` is already a task of `demo-ritual`; if its command is missing from the \
             command line, run `cargo ritual regenerate`"
        );

        let reserved = support::run_binary(&binary, project.root(), &["add", "ritual"])?;
        reserved.expect_failure("`ritual add ritual`");
        assert_eq!(
            reserved.sole_line_prefixed_with("ritual"),
            "`ritual` is reserved for this command line's own commands; give this task another \
             name"
        );

        let nested_new = support::run_binary(
            &binary,
            project.root(),
            &["new", "inner", "--path", checkout.path_argument()?],
        )?;
        nested_new.expect_failure("`ritual new inner` at the project root");
        let message = nested_new.sole_line_prefixed_with("ritual");
        assert!(
            message.starts_with(&format!(
                "refusing `new inner`: {} is a Cargo workspace, ",
                path_to_str(project.root())?
            )),
            "expected the refusal to name the root once; message was:\n{message}"
        );
        assert_eq!(
            message.matches(path_to_str(project.root())?).count(),
            1,
            "expected the root to be named exactly once; message was:\n{message}"
        );
        Ok(())
    })
}

#[test]
fn a_named_command_line_spells_its_hints_with_its_own_name() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("output-named")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &["--cli", "acme"])?;
        let binary = project.build()?;

        let added = support::run_binary(&binary, project.root(), &["ritual", "add", "lint"])?;
        added.expect_success("`acme ritual add lint`");
        assert_eq!(
            lines(&added.stdout).last().copied(),
            Some("next: edit .rituals/lint/src/lib.rs, then run cargo acme lint")
        );

        let repeated = support::run_binary(&binary, project.root(), &["ritual", "add", "lint"])?;
        repeated.expect_failure("a second `acme ritual add lint`");
        assert_eq!(
            repeated.sole_line_prefixed_with("acme"),
            "`lint` is already a task of `demo-ritual`; if its command is missing from the \
             command line, run `cargo acme ritual regenerate`"
        );
        Ok(())
    })
}

#[test]
fn create_ends_with_the_import_command_that_brings_the_new_crate_in() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("output-create")?;
        let created = run_ritual(
            working_dir.path(),
            &["create", "lint", "--path", checkout.path_argument()?],
        )?;
        created.expect_success("`ritual create lint`");
        let crate_dir = working_dir.path().join("lint");
        // The scratch directory's path has nothing in it a shell would quote,
        // so the command is the plain words; the quoting itself is tested
        // where it is rendered.
        let typed = format!("import lint --path {}", path_to_str(&crate_dir)?);
        let next = format!(
            "next: in the project that will use it, run cargo ritual {typed} (or cargo <name> \
             ritual {typed} if it was made with --cli <name>)"
        );
        assert_eq!(
            lines(&created.stdout),
            [
                "created lint/Cargo.toml",
                "created lint/src/lib.rs",
                next.as_str(),
            ]
        );
        Ok(())
    })
}

#[test]
fn a_command_line_run_outside_its_project_hands_back_the_command_to_run() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("output-outside-project")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;

        let global = run_ritual(project.root(), &["add", "lint"])?;
        global.expect_failure("the global `ritual add lint` inside a project");
        assert_eq!(
            global.sole_line_prefixed_with("ritual"),
            "`add` works inside the project this command line belongs to; in your project, run \
             `cargo ritual add lint` (or `cargo <name> ritual add lint` if it was made with \
             `--cli <name>`)"
        );

        // The same holds for a project's own command line run from inside
        // another project, not only for the global binary.
        let other = Project::scaffold(checkout, working_dir.path(), "other", &[])?;
        let binary = project.build()?;
        let foreign = support::run_binary(&binary, other.root(), &["regenerate"])?;
        foreign.expect_failure("`demo`'s `ritual regenerate` inside `other`");
        assert_eq!(
            foreign.sole_line_prefixed_with("ritual"),
            "`regenerate` works inside the project this command line belongs to; in your \
             project, run `cargo ritual regenerate` (or `cargo <name> ritual regenerate` if it \
             was made with `--cli <name>`)"
        );
        Ok(())
    })
}
