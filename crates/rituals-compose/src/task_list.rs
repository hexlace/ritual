//! Resolving a composed CLI's `[package.metadata.ritual] tasks` list into
//! the entries its generated file lists.
//!
//! Reached through [`crate::metadata::Metadata::resolve_task_list`] — this
//! module holds the behaviour, that method is the entry point.
//
// `redundant_pub_crate` (clippy nursery) wants `pub` here because this
// module is private, but `pub(crate)` is the visibility that is actually
// true — it stays correct if this module is ever re-exported at a different
// level — so the nursery lint gives way.
#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) reflects this module's actual visibility even though the enclosing \
              module is private; see the note above"
)]

use std::path::Path;

use rituals::{Failure, Name};

use crate::generated_file::Entry;
use crate::metadata::{DepKind, Metadata, Node, Package};
use crate::project;
use crate::sentence::join_with_and;

/// Reads `package_name`'s `[package.metadata.ritual] tasks` list from
/// `metadata` and resolves every name in it to an [`Entry`], in the order
/// the manifest names them.
///
/// This is a pure read: nothing is written, and each check runs only once the
/// one before it has succeeded — `package_name` found as a workspace member
/// with one binary target, `tasks` present and a list of strings, each name
/// valid, no duplicates, a matching normal dependency present on every target,
/// that dependency resolved, and the resolved crate marked `task = true`. A
/// name has to be valid before it can be matched to a dependency, and a
/// dependency has to resolve before what it declares about itself can be read.
///
/// # Errors
///
/// Returns a [`Failure`] naming the specific problem: a project that cannot
/// be located, an absent or wrongly-shaped `tasks` list, an invalid or
/// duplicated name, a name with no matching dependency, a dependency
/// declared only under a `cfg(...)` target, or a dependency that resolves
/// to a crate whose `task` value is missing, `false`, or not a boolean.
pub(crate) fn resolve(metadata: &Metadata, package_name: &str) -> Result<Vec<Entry>, Failure> {
    let project = project::locate(metadata, package_name)?;
    let cli_package = project.package;

    let relative_manifest_path = cli_package
        .manifest_path
        .strip_prefix(project.workspace_root)
        .unwrap_or(cli_package.manifest_path.as_path());

    let Some(tasks_value) = cli_package
        .metadata
        .get("ritual")
        .and_then(|ritual| ritual.get("tasks"))
    else {
        return Err(tasks_absent(package_name, relative_manifest_path));
    };

    let task_name_strings = read_task_name_strings(tasks_value, relative_manifest_path)?;
    let task_names = validate_task_names(&task_name_strings)?;
    assert_no_duplicates(&task_names)?;

    let node = metadata
        .resolve
        .nodes
        .iter()
        .find(|node| node.id == cli_package.id);
    let Some(node) = node else {
        // Every workspace member is its own resolve node — a cargo metadata
        // invariant, not a condition this framework's caller can act on.
        unreachable!("workspace member `{package_name}` has no resolve node");
    };

    task_names
        .into_iter()
        .map(|name| resolve_one(&name, package_name, node, &metadata.packages))
        .collect()
}

/// Reads `value` as a JSON array of strings, or refuses naming
/// `manifest_path`.
fn read_task_name_strings(
    value: &serde_json::Value,
    manifest_path: &Path,
) -> Result<Vec<String>, Failure> {
    let Some(array) = value.as_array() else {
        return Err(wrong_shape(manifest_path));
    };

    array
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| wrong_shape(manifest_path))
        })
        .collect()
}

/// Validates every raw string in `names` as a [`Name`], refusing on the
/// first invalid one — this runs before duplicates are checked, so a name
/// that is both invalid and repeated is refused for its spelling, not its
/// repetition.
fn validate_task_names(names: &[String]) -> Result<Vec<Name>, Failure> {
    names
        .iter()
        .map(|name_string| Name::new(name_string).map_err(Failure::from))
        .collect()
}

/// Refuses when `names` contains the same name twice.
fn assert_no_duplicates(names: &[Name]) -> Result<(), Failure> {
    for (index, name) in names.iter().enumerate() {
        if names[..index].contains(name) {
            return Err(Failure::new(format!(
                "`{name}` is named twice in [package.metadata.ritual] tasks; a command answers \
                 to one name"
            )));
        }
    }
    Ok(())
}

/// Resolves one already-validated task name to an [`Entry`]: matched to a
/// real dependency, that dependency present on every target (not only under
/// a `cfg(...)` predicate), that dependency resolved to a package, and that
/// package marked `task = true` — each check running only once the one
/// before it has succeeded.
fn resolve_one(
    name: &Name,
    cli_package_name: &str,
    node: &Node,
    packages: &[Package],
) -> Result<Entry, Failure> {
    // `resolve.nodes[].deps[].name` is the extern-crate identifier rustc is
    // given, which is the dependency key with any hyphen already turned
    // into an underscore by Cargo (a dependency key `ritual-task` is
    // reported here as `ritual_task`) — never the literal key itself.
    // Matched by name alone, regardless of dep_kinds, so a dependency that
    // exists only under a predicate or only as a dev-dependency is found
    // here and refused by the checks below rather than read as absent.
    let extern_identifier = name.as_str().replace('-', "_");
    let node_dependency = node
        .deps
        .iter()
        .find(|dependency| dependency.name == extern_identifier);
    let Some(node_dependency) = node_dependency else {
        return Err(no_dependency_called(name, cli_package_name));
    };

    let normal_kinds: Vec<&DepKind> = node_dependency
        .dep_kinds
        .iter()
        .filter(|kind| kind.kind.is_none())
        .collect();
    if normal_kinds.is_empty() {
        // Every entry is a dev- or build-dependency: `name` is not a normal
        // dependency at all, the same refusal as no entry matching by name.
        return Err(no_dependency_called(name, cli_package_name));
    }
    if normal_kinds.iter().all(|kind| kind.target.is_some()) {
        // Present as a normal dependency, but only under one or more
        // `cfg(...)` predicates, never unconditionally: the composed CLI
        // would fail to compile on every other target.
        let predicates: Vec<String> = normal_kinds
            .iter()
            .filter_map(|kind| kind.target.clone())
            .collect();
        return Err(conditional_dependency(name, cli_package_name, &predicates));
    }

    let resolved_package = packages
        .iter()
        .find(|package| package.id == node_dependency.pkg);
    let Some(resolved_package) = resolved_package else {
        // Every id a resolve node names as a dependency is guaranteed present
        // in packages[] by cargo metadata's own schema.
        unreachable!(
            "resolve node references package id `{}` absent from packages[]",
            node_dependency.pkg
        );
    };

    match resolved_package
        .metadata
        .get("ritual")
        .and_then(|ritual| ritual.get("task"))
    {
        Some(serde_json::Value::Bool(true)) => {}
        Some(serde_json::Value::Bool(false)) | None => {
            return Err(not_marked(name.as_str(), &resolved_package.name));
        }
        Some(_wrong_shape) => {
            return Err(Failure::new(format!(
                "`{}` declares [package.metadata.ritual] task, but its value is not a boolean; \
                 it reads `task = true`",
                resolved_package.name
            )));
        }
    }

    Ok(Entry::new(name.as_str(), node_dependency.name.clone()))
}

fn tasks_absent(package_name: &str, manifest_path: &Path) -> Failure {
    Failure::new(format!(
        "`{package_name}` has no [package.metadata.ritual] tasks list in {}; add one, even if \
         it is empty: `tasks = []`",
        manifest_path.display()
    ))
}

fn wrong_shape(manifest_path: &Path) -> Failure {
    Failure::new(format!(
        "[package.metadata.ritual] tasks in {} is not a list of strings; it reads `tasks = \
         [\"lint\"]`",
        manifest_path.display()
    ))
}

/// The refusal for a task name with no matching normal dependency at all —
/// no dependency by that name, or one that exists only as a dev- or
/// build-dependency.
fn no_dependency_called(name: &Name, cli_package_name: &str) -> Failure {
    Failure::new(format!(
        "`{name}` is named in [package.metadata.ritual] tasks, but `{cli_package_name}` has no \
         dependency called `{name}`; add the dependency, or drop `{name}` from the list"
    ))
}

/// The refusal for a task name whose dependency exists only under one or
/// more `cfg(...)` predicates, never unconditionally — a task has to be
/// present on every target the composed CLI builds for, or the CLI fails to
/// compile on every target the predicate excludes, including the one
/// `regenerate` itself needs to run on to put things right.
fn conditional_dependency(name: &Name, cli_package_name: &str, predicates: &[String]) -> Failure {
    let backticked: Vec<String> = predicates
        .iter()
        .map(|target| format!("`{target}`"))
        .collect();
    Failure::new(format!(
        "`{name}` is a dependency of `{cli_package_name}` only under {}, but a task has to be \
         present on every target the command line builds for; declare it under [dependencies]",
        join_with_and(&backticked)
    ))
}

fn not_marked(key: &str, resolved_name: &str) -> Failure {
    if key == resolved_name {
        Failure::new(format!(
            "`{key}` is named in [package.metadata.ritual] tasks, but `{key}` does not declare \
             `task = true` in its own [package.metadata.ritual]; add it there, or drop `{key}` \
             from the list"
        ))
    } else {
        Failure::new(format!(
            "`{key}` is named in [package.metadata.ritual] tasks, but the crate it resolves \
             to, `{resolved_name}`, does not declare `task = true` in its own \
             [package.metadata.ritual]; add it there, or drop `{key}` from the list"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::resolve;
    use crate::generated_file::Entry;
    use crate::metadata::{DepKind, Metadata, Node, parse};
    use crate::test_support::TestOutcome;

    const DEMO_WORKSPACE: &str = include_str!("metadata/fixtures/demo-workspace.json");

    /// The fixture's own resolve node for `demo-ritual` — every test that
    /// edits a resolve edge starts from here, rather than repeating the
    /// `id.contains(..)` lookup at each call site.
    fn demo_ritual_node(metadata: &mut Metadata) -> &mut Node {
        metadata
            .resolve
            .nodes
            .iter_mut()
            .find(|node| node.id.contains("cli#demo-ritual"))
            .expect("the fixture has a demo-ritual resolve node")
    }

    /// Sets `demo-ritual`'s `[package.metadata.ritual]` table in a parsed
    /// copy of the fixture to `metadata_value`, for tests that exercise one
    /// particular shape of it without needing a second fixture file.
    ///
    /// Asserts the package is actually found first: silently leaving the
    /// fixture unedited when it is not would let a test exercise the
    /// fixture's unmodified, already-valid tasks list instead of the shape
    /// it means to test, and some of those shapes are ones `resolve` also
    /// accepts.
    fn set_demo_ritual_metadata(metadata: &mut Metadata, metadata_value: serde_json::Value) {
        let cli = metadata
            .packages
            .iter_mut()
            .find(|package| package.name == "demo-ritual");
        assert!(
            cli.is_some(),
            "expected demo-ritual among the fixture's packages"
        );
        if let Some(cli) = cli {
            cli.metadata = metadata_value;
        }
    }

    #[test]
    fn resolves_the_declared_tasks_in_manifest_order() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let entries = resolve(&metadata, "demo-ritual");
        assert!(
            entries.is_ok(),
            "expected resolution to succeed: {:?}",
            entries.err()
        );
        if let Ok(entries) = entries {
            // tasks = ["task-true", "renamed", "move"] in the fixture, and
            // cargo metadata reports the keyword key `move`'s extern-crate
            // name as `move` too (no `r#`; that prefix is a rendering-time
            // decision, not something Cargo reports).
            assert_eq!(
                entries,
                vec![
                    Entry::new("task-true", "task_true"),
                    Entry::new("renamed", "renamed"),
                    Entry::new("move", "move"),
                ]
            );
        }
        Ok(())
    }

    #[test]
    fn a_dependency_not_marked_task_true_is_refused_naming_the_resolved_crate() -> TestOutcome {
        // `task-null` is a dependency of demo-ritual but is not listed in
        // its `tasks`; hand-editing the fixture's tasks list to add it
        // exercises the "not marked" refusal without a second fixture.
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        set_demo_ritual_metadata(
            &mut metadata,
            serde_json::json!({ "ritual": { "tasks": ["task-null"] } }),
        );

        let entries = resolve(&metadata, "demo-ritual");
        assert!(
            entries.is_err(),
            "expected task-null to be refused: not marked task = true"
        );
        if let Err(error) = entries {
            let message = error.to_string();
            assert!(message.contains("task-null"));
            assert!(message.contains("task = true"));
        }
        Ok(())
    }

    #[test]
    fn a_malformed_task_value_is_refused_naming_the_wrong_shape() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        set_demo_ritual_metadata(
            &mut metadata,
            serde_json::json!({ "ritual": { "tasks": ["task-malformed"] } }),
        );

        let entries = resolve(&metadata, "demo-ritual");
        assert!(entries.is_err(), "expected task-malformed to be refused");
        if let Err(error) = entries {
            let message = error.to_string();
            assert!(message.contains("task-malformed"));
            assert!(message.contains("is not a boolean"));
        }
        Ok(())
    }

    #[test]
    fn a_dev_only_dependency_is_refused_as_not_a_dependency() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        set_demo_ritual_metadata(
            &mut metadata,
            serde_json::json!({ "ritual": { "tasks": ["task-dev-only"] } }),
        );

        let entries = resolve(&metadata, "demo-ritual");
        assert!(
            entries.is_err(),
            "expected the dev-only dependency to be refused"
        );
        if let Err(error) = entries {
            let message = error.to_string();
            assert!(message.contains("task-dev-only"));
            assert!(message.contains("no dependency called"));
        }
        Ok(())
    }

    /// For a crate that depends on `deep` only under
    /// `[target.'cfg(windows)'.dependencies]`, `cargo metadata` reports one
    /// `dep_kinds` entry, `{"kind": null, "target": "cfg(windows)"}`. This
    /// changes only that field on the fixture's `task-true` edge — present
    /// unconditionally there — from `null` to that string, leaving
    /// every other fixture entry's `"target": null` untouched.
    #[test]
    fn a_dependency_declared_only_under_a_target_predicate_is_refused_naming_it() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        let node = demo_ritual_node(&mut metadata);
        let dependency = node.deps.iter_mut().find(|dep| dep.name == "task_true");
        assert!(
            dependency.is_some(),
            "expected a task_true dependency edge in the fixture"
        );
        if let Some(dependency) = dependency {
            for kind in &mut dependency.dep_kinds {
                kind.target = Some("cfg(windows)".to_string());
            }
        }

        let entries = resolve(&metadata, "demo-ritual");
        assert!(
            entries.is_err(),
            "expected a predicate-only dependency to be refused"
        );
        if let Err(error) = entries {
            let message = error.to_string();
            assert!(message.contains("task-true"));
            assert!(message.contains("`cfg(windows)`"));
            assert!(message.contains("declare it under [dependencies]"));
        }
        Ok(())
    }

    /// For a crate that depends on `deep` both unconditionally and under
    /// `[target.'cfg(windows)'.dependencies]`, `cargo metadata` reports two
    /// `dep_kinds` entries on the same edge, `{"kind": null, "target": null}`
    /// and `{"kind": null, "target": "cfg(windows)"}`. This appends that
    /// second entry to the fixture's `task-true` edge, keeping its existing
    /// unconditional one.
    #[test]
    fn a_dependency_declared_both_unconditionally_and_under_a_predicate_is_accepted() -> TestOutcome
    {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        let node = demo_ritual_node(&mut metadata);
        let dependency = node.deps.iter_mut().find(|dep| dep.name == "task_true");
        // `find` returning `None` here has to be a hard failure, not a
        // silent no-op: with the push below skipped, the fixture's
        // unconditional `task-true` edge is unchanged and still resolves
        // on its own, so the assertion below would pass without ever
        // exercising "unconditional alongside a predicate" at all.
        assert!(
            dependency.is_some(),
            "expected a task_true dependency edge in the fixture"
        );
        if let Some(dependency) = dependency {
            dependency.dep_kinds.push(DepKind {
                kind: None,
                target: Some("cfg(windows)".to_string()),
            });
        }

        let entries = resolve(&metadata, "demo-ritual");
        assert!(
            entries.is_ok(),
            "expected a dependency present unconditionally, alongside a predicate, to be \
             accepted: {:?}",
            entries.err()
        );
        Ok(())
    }

    #[test]
    fn a_name_listed_twice_is_refused() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        set_demo_ritual_metadata(
            &mut metadata,
            serde_json::json!({ "ritual": { "tasks": ["task-true", "task-true"] } }),
        );

        let entries = resolve(&metadata, "demo-ritual");
        assert!(entries.is_err(), "expected a duplicated name to be refused");
        if let Err(error) = entries {
            assert!(error.to_string().contains("named twice"));
        }
        Ok(())
    }

    /// An invalid name listed twice must be refused for its spelling, not
    /// its repetition: `resolve`'s doc comment promises validity is
    /// checked before duplicates, and `!!!` fails both checks, so this is
    /// the one case that tells the two orderings apart.
    #[test]
    fn an_invalid_name_listed_twice_is_refused_for_spelling_not_repetition() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        set_demo_ritual_metadata(
            &mut metadata,
            serde_json::json!({ "ritual": { "tasks": ["!!!", "!!!"] } }),
        );

        let entries = resolve(&metadata, "demo-ritual");
        assert!(entries.is_err(), "expected the invalid name to be refused");
        if let Err(error) = entries {
            let message = error.to_string();
            assert!(
                message.contains("is not a usable name"),
                "expected the spelling refusal, got: {message}"
            );
            assert!(
                !message.contains("named twice"),
                "expected the spelling refusal, not the duplicate refusal: {message}"
            );
        }
        Ok(())
    }

    #[test]
    fn an_absent_tasks_list_is_refused_naming_the_manifest() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        set_demo_ritual_metadata(&mut metadata, serde_json::Value::Null);

        let entries = resolve(&metadata, "demo-ritual");
        assert!(
            entries.is_err(),
            "expected an absent tasks list to be refused"
        );
        if let Err(error) = entries {
            let message = error.to_string();
            assert!(message.contains("demo-ritual"));
            assert!(message.contains("tasks = []"));
        }
        Ok(())
    }

    #[test]
    fn a_non_list_tasks_value_is_refused() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        set_demo_ritual_metadata(
            &mut metadata,
            serde_json::json!({ "ritual": { "tasks": "task-true" } }),
        );

        let entries = resolve(&metadata, "demo-ritual");
        assert!(
            entries.is_err(),
            "expected a non-list tasks value to be refused"
        );
        if let Err(error) = entries {
            assert!(error.to_string().contains("is not a list of strings"));
        }
        Ok(())
    }
}
