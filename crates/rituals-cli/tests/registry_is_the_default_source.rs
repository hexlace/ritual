//! `new` and `create` take ritual's own crates from crates.io unless told
//! otherwise, at exactly the version of `rituals` the running binary was
//! built against. `--path` and `--git` replace that source; naming both is
//! an argument error clap refuses while parsing, before anything is written.
//!
//! Every story here reads the manifests a scaffolding run wrote, parsed with
//! `toml_edit` the way Cargo parses them, and builds nothing: a registry
//! source is only buildable once the crates are published, and this suite
//! does not depend on crates.io being reachable.

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use support::{
    Outcome, ResultContext, TempDir, TestOutcome, in_checkout, path_to_str, run_ritual,
    snapshot_tree,
};

/// A git URL for the `--git` stories. Nothing fetches it: `new` and `create`
/// write it into a manifest and stop.
const GIT_URL: &str = "https://example.invalid/ritual.git";

/// Reads and parses the manifest at `path`.
fn read_manifest(path: &Path) -> Outcome<toml_edit::DocumentMut> {
    let text = fs::read_to_string(path).context(&format!("reading {} failed", path.display()))?;
    text.parse::<toml_edit::DocumentMut>()
        .context(&format!("{} is not valid TOML", path.display()))
}

/// Every field of the dependency declaration `item`, as Cargo reads it: a
/// bare string is shorthand for `{ version = "…" }`, and an inline table
/// contributes each of its keys. Anything else — a missing key, a nested
/// table, a non-string field — is returned as a `kind` entry naming it, so
/// the comparison against an expected declaration fails and says why.
fn dependency_fields(item: &toml_edit::Item) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    if let Some(version) = item.as_str() {
        fields.insert("version".to_string(), version.to_string());
    } else if let Some(table) = item.as_inline_table() {
        for (key, value) in table {
            let value = value
                .as_str()
                .map_or_else(|| format!("<not a string: {value}>"), str::to_string);
            fields.insert(key.to_string(), value);
        }
    } else {
        fields.insert("kind".to_string(), format!("<{}>", item.type_name()));
    }
    fields
}

/// Builds an expected declaration from literal pairs.
fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

/// What a scaffolded project declares for ritual's own crates: the root
/// manifest's `[workspace.dependencies] rituals`, and the command line
/// crate's `[dependencies] ritual`.
struct ProjectDependencies {
    workspace_rituals: BTreeMap<String, String>,
    cli_ritual: BTreeMap<String, String>,
}

/// Runs `ritual new demo <source_arguments…>` in a fresh directory and reads
/// back the two dependency declarations it wrote.
fn new_project(source_arguments: &[&str]) -> Outcome<ProjectDependencies> {
    let working_dir = TempDir::new("registry-source-new")?;
    let mut arguments = vec!["new", "demo"];
    arguments.extend_from_slice(source_arguments);
    run_ritual(working_dir.path(), &arguments)?.expect_success(&format!("`ritual {arguments:?}`"));

    let project = working_dir.path().join("demo");
    let workspace = read_manifest(&project.join("Cargo.toml"))?;
    let cli = read_manifest(&project.join("ritual").join("Cargo.toml"))?;

    Ok(ProjectDependencies {
        workspace_rituals: dependency_fields(&workspace["workspace"]["dependencies"]["rituals"]),
        cli_ritual: dependency_fields(&cli["dependencies"]["ritual"]),
    })
}

/// Runs `ritual create chore <source_arguments…>` in a fresh directory and
/// reads back the `rituals` dependency it wrote.
fn create_task(source_arguments: &[&str]) -> Outcome<BTreeMap<String, String>> {
    let working_dir = TempDir::new("registry-source-create")?;
    let mut arguments = vec!["create", "chore"];
    arguments.extend_from_slice(source_arguments);
    run_ritual(working_dir.path(), &arguments)?.expect_success(&format!("`ritual {arguments:?}`"));

    let manifest = read_manifest(&working_dir.path().join("chore").join("Cargo.toml"))?;
    Ok(dependency_fields(&manifest["dependencies"]["rituals"]))
}

/// The version a registry source must request: the `rituals` this binary
/// was built against, written in full so it is also the lowest release a
/// project will accept.
fn expected_version() -> &'static str {
    let version = rituals::VERSION;
    let parts: Vec<&str> = version.split('.').collect();
    assert!(
        parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())),
        "rituals::VERSION must be a full MAJOR.MINOR.PATCH release, was {version:?}"
    );
    version
}

#[test]
fn new_without_a_source_flag_requests_this_version_from_crates_io() -> TestOutcome {
    let version = expected_version();
    let dependencies = new_project(&[])?;

    assert_eq!(
        dependencies.workspace_rituals,
        fields(&[("version", version)]),
        "the workspace must declare `rituals = \"{version}\"` and nothing else"
    );
    assert_eq!(
        dependencies.cli_ritual,
        fields(&[("package", "rituals-core"), ("version", version)]),
        "the command line crate must declare \
         `ritual = {{ package = \"rituals-core\", version = \"{version}\" }}`"
    );
    Ok(())
}

#[test]
fn create_without_a_source_flag_requests_this_version_from_crates_io() -> TestOutcome {
    let version = expected_version();
    assert_eq!(
        create_task(&[])?,
        fields(&[("version", version)]),
        "the task crate must declare `rituals = \"{version}\"` and nothing else"
    );
    Ok(())
}

#[test]
fn new_with_a_path_takes_both_crates_from_that_checkout() -> TestOutcome {
    in_checkout(|checkout| {
        let dependencies = new_project(&["--path", checkout.path_argument()?])?;

        let rituals = checkout.root().join("crates").join("rituals");
        let core = checkout.root().join("crates").join("rituals-core");
        assert_eq!(
            dependencies.workspace_rituals,
            fields(&[("path", path_to_str(&rituals)?)])
        );
        assert_eq!(
            dependencies.cli_ritual,
            fields(&[("package", "rituals-core"), ("path", path_to_str(&core)?)])
        );
        Ok(())
    })
}

#[test]
fn new_with_a_git_url_takes_both_crates_from_that_repository() -> TestOutcome {
    let dependencies = new_project(&["--git", GIT_URL])?;

    assert_eq!(dependencies.workspace_rituals, fields(&[("git", GIT_URL)]));
    assert_eq!(
        dependencies.cli_ritual,
        fields(&[("package", "rituals-core"), ("git", GIT_URL)])
    );
    Ok(())
}

#[test]
fn create_with_a_path_takes_rituals_from_that_checkout() -> TestOutcome {
    in_checkout(|checkout| {
        let rituals = checkout.root().join("crates").join("rituals");
        assert_eq!(
            create_task(&["--path", checkout.path_argument()?])?,
            fields(&[("path", path_to_str(&rituals)?)])
        );
        Ok(())
    })
}

#[test]
fn create_with_a_git_url_takes_rituals_from_that_repository() -> TestOutcome {
    assert_eq!(
        create_task(&["--git", GIT_URL])?,
        fields(&[("git", GIT_URL)])
    );
    Ok(())
}

/// Runs `ritual <arguments…> --path <dir> --git <url>` in an empty
/// directory and checks clap refuses it as an argument error — exit status
/// 2, naming both flags — leaving the directory untouched.
///
/// The path is never read: clap refuses the pair before any task code
/// runs, so it need not be a checkout.
fn assert_both_sources_are_refused(arguments: &[&str]) -> TestOutcome {
    let working_dir = TempDir::new("registry-source-both")?;
    let before = snapshot_tree(working_dir.path())?;

    let mut arguments = arguments.to_vec();
    arguments.extend_from_slice(&["--path", "checkout", "--git", GIT_URL]);
    let output = run_ritual(working_dir.path(), &arguments)?;

    assert_eq!(
        output.exit_code,
        Some(2),
        "expected `ritual {arguments:?}` to be refused as an argument error; stderr was:\n{}",
        output.stderr
    );
    assert!(
        output.stderr.contains("--path") && output.stderr.contains("--git"),
        "expected the refusal to name both flags; stderr was:\n{}",
        output.stderr
    );
    assert_eq!(
        before,
        snapshot_tree(working_dir.path())?,
        "a refused run must not write anything"
    );
    Ok(())
}

#[test]
fn new_with_both_a_path_and_a_git_url_is_refused() -> TestOutcome {
    assert_both_sources_are_refused(&["new", "demo"])
}

#[test]
fn create_with_both_a_path_and_a_git_url_is_refused() -> TestOutcome {
    assert_both_sources_are_refused(&["create", "chore"])
}
