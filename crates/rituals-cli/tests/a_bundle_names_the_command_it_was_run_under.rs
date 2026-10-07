//! A bundle is mounted under whatever key the project chose, so the command
//! that reaches one of its tasks is the project's to spell, not the
//! bundle's. A task that tells a person to run it again asks the command
//! line it was handed for that command, and what it prints is what the
//! person types: `cargo ritual tools db sync` for a bundle mounted under
//! `tools`, and `cargo acme tools db sync` in a project made with
//! `--cli acme`.
//!
//! The bundle here is `acme-tools`, mounted under `tools` rather than its own
//! name, with `sync` one bundle further down, under `db`. The remedy it
//! prints is then run, word for word, through `cargo` in the project, and
//! reaches the same task: that is what makes it one a person can paste.
//!
//! Ritual's own bundle is a bundle like any other, so a project may mount it
//! under `tools` too, and then its hints go through `tools`:
//! `cargo ritual tools regenerate`, or `cargo acme tools regenerate` under
//! `--cli acme`. That hint is run the same way.
//!
//! Each of ritual's tasks is published as a crate of its own, so a project
//! can also mount one alone, outside ritual's bundle. `import` mounted under
//! `imp` in a project made with `--cli acme` names its own re-run as
//! `cargo acme imp …`, the words that reached it, and that command is run
//! word for word too.

mod support;

use std::fmt::Write as _;

use support::{
    Checkout, Outcome, Project, RunOutput, TempDir, TestOutcome, crates, created, failure,
    generated, in_checkout, manifest, path_to_str, read_text, write_text,
};
use toml_edit::Item;

/// The bundle's crate name, which a project could mount it under as it is.
const CRATE_NAME: &str = "acme-tools";

/// The key this story mounts the bundle under instead.
const MOUNT_KEY: &str = "tools";

/// What `sync` refuses with, ahead of the command it names.
const REFUSAL: &str = "the database is not set up yet; set it up, then run";

/// Renders the bundle's `src/lib.rs`: `db`, a bundle holding `sync`, whose
/// handler refuses with a remedy spelled by `CommandLine::cargo_command`.
fn bundle_lib() -> String {
    let mut output = String::new();
    output.push_str("//! A hand-written bundle crate, built for this story.\n\n");
    output.push_str("use rituals::{CommandLine, Failure, Outcome, Task, clap};\n\n");
    output.push_str("/// What `sync` accepts on the command line — nothing.\n");
    output.push_str("#[derive(clap::Args)]\n");
    output.push_str("struct Arguments {}\n\n");
    output.push_str("/// This bundle, for a command line to mount under whatever key it likes.\n");
    output.push_str("#[must_use]\n");
    output.push_str("pub fn task() -> Task {\n");
    output.push_str("    let sync = Task::receiving_command_line(\"sync the database\", sync);\n");
    output.push_str("    let db = Task::group(\"the project's database\", [(\"sync\", sync)]);\n");
    output.push_str("    Task::group(\"tools for a project\", [(\"db\", db)])\n");
    output.push_str("}\n\n");
    output.push_str("fn sync(command_line: &CommandLine, _arguments: Arguments) -> Outcome {\n");
    output.push_str("    Err(Failure::new(format!(\n");
    let _ = writeln!(output, "        \"{REFUSAL} `{{}}` again\",");
    output.push_str("        command_line.cargo_command()\n");
    output.push_str("    )))\n");
    output.push_str("}\n");
    output
}

/// Writes the bundle under `.rituals/acme-tools`, makes it a workspace
/// member, mounts it under [`MOUNT_KEY`], and regenerates through
/// `regenerate`, typed as `regenerate_path` says it is in this project.
fn write_and_mount_the_bundle(project: &Project, regenerate_path: &[&str]) -> TestOutcome {
    let member = format!(".rituals/{CRATE_NAME}");
    let crate_dir = project.root().join(&member);
    crates::write_crate(
        &crate_dir,
        &crates::bundle_manifest(CRATE_NAME, &[]),
        &bundle_lib(),
    )?;
    manifest::edit(&project.workspace_manifest_path(), |document| {
        manifest::push_member(document, &member)
    })?;
    project.mount(&crate_dir, MOUNT_KEY, CRATE_NAME)?;
    project
        .run_cli(regenerate_path)?
        .expect_success("`regenerate` after mounting the bundle under `tools`");
    Ok(())
}

/// The command a refusal line names between its last pair of backticks.
fn named_command(refusal: &str) -> Outcome<&str> {
    match refusal.rsplit('`').nth(1) {
        Some(command) if !command.is_empty() => Ok(command),
        _ => failure(format!(
            "expected a command in backticks; the refusal was: {refusal}"
        )),
    }
}

/// Runs `cargo tools db sync` through the built binary, asserts the remedy
/// names `expected`, then runs that remedy through `cargo` in the project
/// and asserts it reaches `sync` again.
fn assert_the_remedy_is_the_command_typed(
    project: &Project,
    bin_name: &str,
    expected: &str,
) -> TestOutcome {
    let output = project.run_cli(&[MOUNT_KEY, "db", "sync"])?;
    output.expect_failure("`tools db sync`, which always refuses");
    let refusal = output.sole_line_prefixed_with(bin_name);
    assert_eq!(refusal, format!("{REFUSAL} `{expected}` again"));

    let command = named_command(refusal)?;
    let Some(arguments) = command.strip_prefix("cargo ") else {
        return failure(format!("expected a cargo command; it was `{command}`"));
    };
    let words: Vec<&str> = arguments.split(' ').collect();
    let pasted: RunOutput = project.cargo(&words)?;
    pasted.expect_failure(&format!("`{command}`, pasted from the refusal"));
    assert!(
        pasted.stderr.contains(&format!("{bin_name}: {refusal}")),
        "expected `{command}` to reach `sync` again; stderr was:\n{}",
        pasted.stderr
    );
    Ok(())
}

/// The key `new` mounts ritual's own bundle under.
const RITUAL_KEY: &str = "ritual";

/// The task the remounted bundle's stories create, then take out of the
/// task list so `create` refuses it.
const TASK: &str = "lint";

/// Mounts ritual's own bundle under [`MOUNT_KEY`] instead of [`RITUAL_KEY`],
/// and regenerates through the new key.
///
/// Cargo refuses one package under two keys, so the command line cannot be
/// built with the bundle under both while it regenerates. The manifest's key
/// and task entry change together, the generated file's one mount line is
/// changed by hand to match so the command line builds, and `regenerate`,
/// reached through the new key, then writes the file from the manifest.
fn remount_rituals_bundle(project: &Project) -> TestOutcome {
    manifest::edit(&project.cli_manifest_path(), |document| {
        let Some(dependencies) = document
            .get_mut("dependencies")
            .and_then(Item::as_table_like_mut)
        else {
            return failure("no [dependencies] table in the command line's manifest");
        };
        let Some(declaration) = dependencies.remove(RITUAL_KEY) else {
            return failure(format!("no dependency `{RITUAL_KEY}` in [dependencies]"));
        };
        dependencies.insert(MOUNT_KEY, declaration);
        manifest::remove_task(document, RITUAL_KEY)?;
        manifest::push_task(document, MOUNT_KEY)
    })?;

    let path = project.generated_file_path();
    let generated_file = read_text(&path)?;
    let mounted = format!("(\"{RITUAL_KEY}\", {RITUAL_KEY}::task())");
    if generated_file.matches(&mounted).count() != 1 {
        return failure(format!(
            "expected the generated file to mount `{mounted}` once; it was:\n{generated_file}"
        ));
    }
    let remounted = format!("(\"{MOUNT_KEY}\", {MOUNT_KEY}::task())");
    write_text(&path, &generated_file.replace(&mounted, &remounted))?;

    project
        .run_cli(&[MOUNT_KEY, "regenerate"])?
        .expect_success("`tools regenerate` after remounting ritual's bundle under `tools`");
    Ok(())
}

/// Creates [`TASK`] through the remounted bundle, then takes it out of the
/// task list, leaving its dependency: the state `create` refuses with a
/// `regenerate` hint.
fn create_a_task_and_take_it_out_of_the_list(project: &Project) -> TestOutcome {
    project
        .run_cli(&[MOUNT_KEY, "create", TASK])?
        .expect_success("`tools create lint` through the remounted bundle");
    manifest::edit(&project.cli_manifest_path(), |document| {
        manifest::remove_task(document, TASK)
    })
}

/// Runs `tools create lint` again, asserts its refusal ends with `expected`,
/// then runs that hint word for word through `cargo` in the project and
/// asserts it reached ritual's `regenerate`: the file it writes mounts the
/// bundle and no longer `lint`.
fn assert_rituals_hint_is_the_command_typed(
    project: &Project,
    bin_name: &str,
    expected: &str,
) -> TestOutcome {
    let package = manifest::package_name(&project.cli_manifest()?)?;
    let output = project.run_cli(&[MOUNT_KEY, "create", TASK])?;
    let refusal = created::the_refusal(&output, bin_name);
    assert_eq!(
        refusal,
        format!(
            "`{package}` already has a dependency called `{TASK}` that is not in \
             [package.metadata.ritual] tasks; add `\"{TASK}\"` to that list and run `{expected}`"
        )
    );

    let command = named_command(refusal)?;
    let Some(arguments) = command.strip_prefix("cargo ") else {
        return failure(format!("expected a cargo command; it was `{command}`"));
    };
    let words: Vec<&str> = arguments.split(' ').collect();
    project
        .cargo(&words)?
        .expect_success(&format!("`{command}`, pasted from the refusal"));
    assert_eq!(
        generated::mounted_entries(&project.generated_file()?),
        [(MOUNT_KEY.to_string(), MOUNT_KEY.to_string())],
        "expected `{command}` to regenerate without `{TASK}`"
    );
    Ok(())
}

#[test]
fn a_bundle_under_a_key_of_the_project_s_choosing_names_that_key_in_its_remedy() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("bundle-names-its-command")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;

        write_and_mount_the_bundle(&project, &["regenerate"])?;
        assert_the_remedy_is_the_command_typed(&project, "ritual", "cargo ritual tools db sync")
    })
}

#[test]
fn under_a_named_cli_the_remedy_names_that_cli() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("bundle-names-its-command-named-cli")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &["--cli", "acme"])?;

        write_and_mount_the_bundle(&project, &["ritual", "regenerate"])?;
        assert_the_remedy_is_the_command_typed(&project, "acme", "cargo acme tools db sync")
    })
}

#[test]
fn rituals_own_bundle_under_a_key_of_the_project_s_choosing_names_that_key_in_its_hints()
-> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("rituals-bundle-names-its-command")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &[])?;

        remount_rituals_bundle(&project)?;
        create_a_task_and_take_it_out_of_the_list(&project)?;
        assert_rituals_hint_is_the_command_typed(
            &project,
            "ritual",
            "cargo ritual tools regenerate",
        )
    })
}

#[test]
fn under_a_named_cli_rituals_own_hints_name_that_cli() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("rituals-bundle-names-its-command-named-cli")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &["--cli", "acme"])?;

        remount_rituals_bundle(&project)?;
        create_a_task_and_take_it_out_of_the_list(&project)?;
        assert_rituals_hint_is_the_command_typed(&project, "acme", "cargo acme tools regenerate")
    })
}

/// The key the story mounts `rituals-core-import` under, on its own.
const IMPORT_KEY: &str = "imp";

/// A crate whose name cannot be a key, so `import` refuses it without one
/// and hands back the command to run with a key in it.
const UNUSABLE_CRATE: &str = "my_crate";

/// The key that refusal suggests for [`UNUSABLE_CRATE`].
const SUGGESTED_KEY: &str = "my-crate";

/// Mounts this checkout's `rituals-core-import` under [`IMPORT_KEY`],
/// beside ritual's bundle, and regenerates through the bundle's own
/// `regenerate`, which a project made with `--cli acme` reaches under
/// `ritual`.
fn mount_import_on_its_own(project: &Project, checkout: &Checkout) -> TestOutcome {
    let crate_dir = checkout.root().join(".rituals/import");
    project.mount(&crate_dir, IMPORT_KEY, "rituals-core-import")?;
    project
        .run_cli(&[RITUAL_KEY, "regenerate"])?
        .expect_success("`ritual regenerate` after mounting `import` under `imp`");
    Ok(())
}

/// Runs `imp my_crate --path …` through the built binary, asserts the
/// refusal names `expected` word for word, then runs that command through
/// `cargo` in the project and asserts it imported the crate under the key
/// it suggested.
fn assert_imports_own_rerun_is_the_command_typed(
    project: &Project,
    crate_dir: &str,
    expected: &str,
) -> TestOutcome {
    let output = project.run_cli(&[IMPORT_KEY, UNUSABLE_CRATE, "--path", crate_dir])?;
    output.expect_failure("`imp my_crate`, whose name cannot be a key");
    assert_eq!(
        output.sole_line_prefixed_with("acme"),
        format!(
            "`{UNUSABLE_CRATE}` is not a usable name; a name starts with a lowercase letter, \
             continues with lowercase letters, digits and hyphens, and does not end with a \
             hyphen; the key defaults to the crate's name, so give one: `{expected}`"
        )
    );

    let Some(arguments) = expected.strip_prefix("cargo ") else {
        return failure(format!("expected a cargo command; it was `{expected}`"));
    };
    let words: Vec<&str> = arguments.split(' ').collect();
    project
        .cargo(&words)?
        .expect_success(&format!("`{expected}`, pasted from the refusal"));
    let tasks = manifest::tasks(&project.cli_manifest()?)?;
    assert!(
        tasks.iter().any(|task| task == SUGGESTED_KEY),
        "expected `{expected}` to import `{UNUSABLE_CRATE}` under `{SUGGESTED_KEY}`; the task \
         list was {tasks:?}"
    );
    Ok(())
}

#[test]
fn import_mounted_on_its_own_names_the_key_it_was_run_under() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("import-names-its-own-key")?;
        let project = Project::scaffold(checkout, working_dir.path(), "demo", &["--cli", "acme"])?;

        mount_import_on_its_own(&project, checkout)?;
        let crate_dir = project.write_leaf(UNUSABLE_CRATE)?.canonicalize()?;
        let crate_dir = path_to_str(&crate_dir)?;
        assert_imports_own_rerun_is_the_command_typed(
            &project,
            crate_dir,
            &format!("cargo acme imp {UNUSABLE_CRATE} {SUGGESTED_KEY} --path {crate_dir}"),
        )
    })
}
