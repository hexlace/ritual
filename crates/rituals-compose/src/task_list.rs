//! Resolving a composed CLI's `[package.metadata.ritual] tasks` list into
//! the entries its generated file lists.
//!
//! Reached through [`crate::metadata::Metadata::resolve_task_list`], and
//! [`crate::metadata::Metadata::resolve_task_list_excluding`] for the list
//! with one key left out — this module holds the behaviour, those methods
//! are the entry points.
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

use rituals::{Failure, Name, Outcome};

use crate::generated_file::{self, Entry};
use crate::metadata::{DepKind, Metadata, Node, Package};
use crate::project;
use crate::rust_name::extern_identifier;
use crate::sentence::join_with_and;

/// Reads `package_name`'s `[package.metadata.ritual] tasks` list from
/// `metadata` and resolves every name in it to an [`Entry`], in the order
/// the manifest names them.
///
/// This is a pure read: nothing is written, and each check runs only once the
/// one before it has succeeded — `package_name` found as a workspace member
/// with one binary target, `tasks` present and a list of strings, each name
/// valid and one the generated file compiles with, no duplicates, a matching
/// normal dependency present on every target, that dependency resolved, the
/// resolved crate marked `task = true`, and that crate built on the composed
/// CLI's own `rituals`. A name has to be valid before it can be matched to a
/// dependency, and a dependency has to resolve before what it declares about
/// itself can be read.
///
/// # Errors
///
/// Returns a [`Failure`] naming the specific problem: a project that cannot
/// be located, an absent or wrongly-shaped `tasks` list, an invalid or
/// duplicated name, a name that would hide `std` or `core`, a name with no
/// matching dependency, a dependency declared only under a `cfg(...)`
/// target, a dependency that resolves to a crate whose `task` value is
/// missing, `false`, or not a boolean, or one built on another `rituals`
/// than the composed CLI's, or on none.
pub(crate) fn resolve(metadata: &Metadata, package_name: &str) -> Result<Vec<Entry>, Failure> {
    resolve_leaving_out(metadata, package_name, None)
}

/// [`resolve`] with every occurrence of `excluded` left out of the list
/// before any name is checked, so what remains can be verified while
/// `excluded` is about to be removed, whatever is wrong with it.
///
/// An `excluded` that is not in the list changes nothing.
///
/// # Errors
///
/// Returns the same refusals as [`resolve`], for the names that remain.
pub(crate) fn resolve_excluding(
    metadata: &Metadata,
    package_name: &str,
    excluded: &str,
) -> Result<Vec<Entry>, Failure> {
    resolve_leaving_out(metadata, package_name, Some(excluded))
}

/// The one resolution both entry points above run, so that leaving a key
/// out cannot change what is checked for the rest.
fn resolve_leaving_out(
    metadata: &Metadata,
    package_name: &str,
    excluded: Option<&str>,
) -> Result<Vec<Entry>, Failure> {
    let project = project::locate(metadata, package_name)?;
    let cli_package = project.package;

    let task_name_strings: Vec<&str> = declared_keys(&project, package_name)?
        .into_iter()
        .filter(|key| Some(*key) != excluded)
        .collect();
    let task_names = validate_task_names(&task_name_strings)?;
    assert_no_duplicates(&task_names)?;

    let node = resolve_node_of(metadata, cli_package, package_name);

    task_names
        .into_iter()
        .map(|name| resolve_one(&name, package_name, node, metadata))
        .collect()
}

/// Refuses unless `package_name`'s dependency under `key` is a task: found
/// the way [`resolve_one`] finds the one a listed name stands for, and judged
/// by the same [`declaration`], so what this accepts the resolver accepts
/// once `key` is in the list.
///
/// Reached through [`crate::metadata::Metadata::ensure_dependency_is_a_task`].
/// Its refusals are for a key that is not in the list yet, so none of them
/// says it is named there.
pub(crate) fn ensure_dependency_is_a_task(
    metadata: &Metadata,
    package_name: &str,
    key: &Name,
) -> Outcome {
    let project = project::locate(metadata, package_name)?;
    generated_file::ensure_key_hides_no_crate(key)?;
    let node = resolve_node_of(metadata, project.package, package_name);

    match find_dependency(key, node, &metadata.packages) {
        DependencyFound::Absent => Err(no_such_dependency(key, package_name)),
        DependencyFound::DevelopmentOrBuildOnly => {
            Err(only_a_dev_or_build_dependency(key, package_name))
        }
        DependencyFound::OnlyUnderTargets(predicates) => {
            Err(conditional_dependency(key, package_name, &predicates))
        }
        DependencyFound::Normal { package, .. } => match declaration(package) {
            TaskDeclaration::Task => {
                ensure_same_rituals(metadata, node, package).map_err(|disagreement| {
                    rituals_refusal(&disagreement, key, package_name, Asking::Import)
                })
            }
            TaskDeclaration::NotATask => Err(not_a_task_crate(key, &package.name)),
            TaskDeclaration::NotABoolean => Err(task_value_not_a_boolean(&package.name)),
        },
    }
}

/// Reads `project`'s `[package.metadata.ritual] tasks` list as the strings
/// it holds, in manifest order, without checking that any of them is a
/// usable name or names a dependency.
///
/// # Errors
///
/// Returns a [`Failure`] naming the manifest when the list is absent or is
/// not a list of strings.
pub(crate) fn declared_keys<'a>(
    project: &project::Project<'a>,
    package_name: &str,
) -> Result<Vec<&'a str>, Failure> {
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

    read_task_name_strings(tasks_value, relative_manifest_path)
}

/// Reads `value` as a JSON array of strings, or refuses naming
/// `manifest_path`.
fn read_task_name_strings<'a>(
    value: &'a serde_json::Value,
    manifest_path: &Path,
) -> Result<Vec<&'a str>, Failure> {
    let Some(array) = value.as_array() else {
        return Err(wrong_shape(manifest_path));
    };

    array
        .iter()
        .map(|item| item.as_str().ok_or_else(|| wrong_shape(manifest_path)))
        .collect()
}

/// Validates every raw string in `names` as a [`Name`] the generated file
/// can compile with, refusing on the first that is not one — this runs
/// before duplicates are checked, so a name that is both invalid and
/// repeated is refused for its spelling, not its repetition.
fn validate_task_names(names: &[&str]) -> Result<Vec<Name>, Failure> {
    names
        .iter()
        .map(|name_string| {
            let name = Name::new(name_string)?;
            generated_file::ensure_key_hides_no_crate(&name)?;
            Ok(name)
        })
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

/// `package`'s own resolve node.
fn resolve_node_of<'a>(metadata: &'a Metadata, package: &Package, package_name: &str) -> &'a Node {
    let node = metadata
        .resolve
        .nodes
        .iter()
        .find(|node| node.id == package.id);
    let Some(node) = node else {
        // Every workspace member is its own resolve node — a cargo metadata
        // invariant, not a condition this framework's caller can act on.
        unreachable!("workspace member `{package_name}` has no resolve node");
    };
    node
}

/// What the resolve node of a composed CLI says about the dependency a key
/// names.
enum DependencyFound<'a> {
    /// No dependency answers to the key.
    Absent,
    /// One does, but only as a dev- or build-dependency.
    DevelopmentOrBuildOnly,
    /// A normal dependency, but only under these `cfg(...)` predicates,
    /// never unconditionally: the composed CLI would fail to compile on
    /// every other target.
    OnlyUnderTargets(Vec<String>),
    /// A normal dependency on every target, resolved to `package`, which
    /// rustc knows by `extern_identifier`.
    Normal {
        package: &'a Package,
        extern_identifier: &'a str,
    },
}

/// Finds the dependency of `node` that `key` names: matched to a resolve
/// edge, that edge present as a normal dependency, on every target and not
/// only under `cfg(...)` predicates, and that edge resolved to a package —
/// each check running only once the one before it has succeeded.
///
/// One rule for [`resolve_one`] and [`ensure_dependency_is_a_task`], which
/// differ only in what they say about what they find.
fn find_dependency<'a>(key: &Name, node: &'a Node, packages: &'a [Package]) -> DependencyFound<'a> {
    // `resolve.nodes[].deps[].name` is the extern-crate identifier rustc is
    // given, which is the dependency key with any hyphen already turned
    // into an underscore by Cargo (a dependency key `ritual-task` is
    // reported here as `ritual_task`) — never the literal key itself.
    // Matched by name alone, regardless of dep_kinds, so a dependency that
    // exists only under a predicate or only as a dev-dependency is found
    // here and refused by the checks below rather than read as absent.
    let key_identifier = extern_identifier(key.as_str());
    let node_dependency = node
        .deps
        .iter()
        .find(|dependency| dependency.name == key_identifier);
    let Some(node_dependency) = node_dependency else {
        return DependencyFound::Absent;
    };

    let normal_kinds: Vec<&DepKind> = node_dependency
        .dep_kinds
        .iter()
        .filter(|kind| kind.kind.is_none())
        .collect();
    if normal_kinds.is_empty() {
        return DependencyFound::DevelopmentOrBuildOnly;
    }
    if normal_kinds.iter().all(|kind| kind.target.is_some()) {
        let predicates = normal_kinds
            .iter()
            .filter_map(|kind| kind.target.clone())
            .collect();
        return DependencyFound::OnlyUnderTargets(predicates);
    }

    let resolved_package = packages
        .iter()
        .find(|package| package.id == node_dependency.pkg);
    let Some(package) = resolved_package else {
        // Every id a resolve node names as a dependency is guaranteed present
        // in packages[] by cargo metadata's own schema.
        unreachable!(
            "resolve node references package id `{}` absent from packages[]",
            node_dependency.pkg
        );
    };

    DependencyFound::Normal {
        package,
        extern_identifier: &node_dependency.name,
    }
}

/// What a crate's own `[package.metadata.ritual] task` says about it.
enum TaskDeclaration {
    /// `task = true`.
    Task,
    /// `task = false`, or no such key.
    NotATask,
    /// `task` is there, but not a boolean.
    NotABoolean,
}

/// Reads what `package` declares about itself. The one rule for what makes a
/// crate a task, whichever way it is being asked.
fn declaration(package: &Package) -> TaskDeclaration {
    match package
        .metadata
        .get("ritual")
        .and_then(|ritual| ritual.get("task"))
    {
        Some(serde_json::Value::Bool(true)) => TaskDeclaration::Task,
        Some(serde_json::Value::Bool(false)) | None => TaskDeclaration::NotATask,
        Some(_wrong_shape) => TaskDeclaration::NotABoolean,
    }
}

/// The crate every task is built on and every generated file names.
const RITUALS: &str = "rituals";

/// How a task fails to be built on the same `rituals` as the composed CLI
/// that would mount it.
///
/// A task hands the command line a `rituals::Task`, and the generated file
/// passes it to the CLI's own `rituals::run`. Two packages called `rituals`
/// are two different crates to Rust, whatever their versions say, so the
/// generated file compiles only when both resolve to the very same package.
enum RitualsDisagreement<'a> {
    /// The composed CLI has no normal dependency on `rituals`, so no task
    /// can be mounted in it.
    CliHasNone,
    /// The task's crate has no normal dependency on `rituals`, so it has no
    /// `rituals::Task` to hand over.
    TaskHasNone { task: &'a Package },
    /// Each resolves to a different package.
    Differs {
        cli: &'a Package,
        task_rituals: &'a Package,
    },
}

/// Refuses unless `task` resolves `rituals` to the same package, by id, as
/// the composed CLI whose resolve node is `cli_node`.
fn ensure_same_rituals<'a>(
    metadata: &'a Metadata,
    cli_node: &Node,
    task: &'a Package,
) -> Result<(), RitualsDisagreement<'a>> {
    let Some(cli) = rituals_of(cli_node, &metadata.packages) else {
        return Err(RitualsDisagreement::CliHasNone);
    };
    let task_node = resolve_node_of(metadata, task, &task.name);
    let Some(task_rituals) = rituals_of(task_node, &metadata.packages) else {
        return Err(RitualsDisagreement::TaskHasNone { task });
    };
    if task_rituals.id == cli.id {
        Ok(())
    } else {
        Err(RitualsDisagreement::Differs { cli, task_rituals })
    }
}

/// The package called `rituals` that `node` depends on as a normal
/// dependency, under whatever key.
fn rituals_of<'a>(node: &Node, packages: &'a [Package]) -> Option<&'a Package> {
    node.deps
        .iter()
        .filter(|dependency| dependency.dep_kinds.iter().any(|kind| kind.kind.is_none()))
        .filter_map(|dependency| packages.iter().find(|package| package.id == dependency.pkg))
        .find(|package| package.name == RITUALS)
}

/// Which question a refusal answers, which decides how it names the key and
/// what it says to do.
#[derive(Clone, Copy)]
enum Asking {
    /// Whether a dependency just declared under the key can be imported.
    Import,
    /// Whether a key already in `[package.metadata.ritual] tasks` resolves.
    Listed,
}

/// The refusal for a task that is not built on the composed CLI's own
/// `rituals`. Names both by version, and by where each comes from when the
/// versions alone would not say why they differ.
fn rituals_refusal(
    disagreement: &RitualsDisagreement<'_>,
    key: &Name,
    cli_package_name: &str,
    asking: Asking,
) -> Failure {
    // Where the key is already in the list, the refusal says so first and
    // offers taking it out as well as putting it right.
    let (listed, or_drop) = match asking {
        Asking::Import => (String::new(), String::new()),
        Asking::Listed => (
            format!("`{key}` is named in [package.metadata.ritual] tasks, but "),
            format!(", or drop `{key}` from the list"),
        ),
    };
    match disagreement {
        RitualsDisagreement::CliHasNone => Failure::new(format!(
            "{listed}`{cli_package_name}` does not depend on rituals, which its generated file \
             is built on; add `rituals` under its [dependencies]{or_drop}"
        )),
        RitualsDisagreement::TaskHasNone { task } => {
            let crate_name = &task.name;
            match asking {
                Asking::Import => {
                    let subject = if key.as_str() == crate_name {
                        format!("`{key}`")
                    } else {
                        format!("`{key}` resolves to `{crate_name}`, which")
                    };
                    Failure::new(format!(
                        "{subject} declares `task = true` but does not depend on rituals, so it \
                         has no task to hand a command line; choose a task crate built on rituals"
                    ))
                }
                Asking::Listed => Failure::new(format!(
                    "{listed}`{crate_name}` does not depend on rituals, so it has no task to hand \
                     a command line; drop `{key}` from the list"
                )),
            }
        }
        RitualsDisagreement::Differs { cli, task_rituals } => {
            let subject = match asking {
                Asking::Import => format!("`{key}`"),
                Asking::Listed => format!("{listed}it"),
            };
            let cli_series = release_series(&cli.version);
            if release_series(&task_rituals.version) == cli_series {
                Failure::new(format!(
                    "{subject} is built for rituals {} from {} and this project uses rituals {} \
                     from {}, which Rust reads as two different crates; build both on the same \
                     rituals{or_drop}",
                    task_rituals.version,
                    source_of(task_rituals),
                    cli.version,
                    source_of(cli),
                ))
            } else {
                let remedy = match asking {
                    Asking::Import => "import",
                    Asking::Listed => "depend on",
                };
                Failure::new(format!(
                    "{subject} is built for rituals {} and this project uses {}; {remedy} a \
                     release of it made for {cli_series}{or_drop}",
                    task_rituals.version, cli.version,
                ))
            }
        }
    }
}

/// The releases of `version` that can share one `rituals` with it, as a
/// person names them: `0.2` for any `0.2.x`, `1` for any `1.x.y`.
fn release_series(version: &str) -> String {
    match version.split_once('.') {
        Some(("0", rest)) => {
            let minor = rest.split_once('.').map_or(rest, |(minor, _patch)| minor);
            format!("0.{minor}")
        }
        Some((major, _rest)) => major.to_string(),
        None => version.to_string(),
    }
}

/// Where `package` comes from, as a person would name it: crates.io,
/// another registry, a git repository, or a directory.
fn source_of(package: &Package) -> String {
    const CRATES_IO: [&str; 2] = [
        "registry+https://github.com/rust-lang/crates.io-index",
        "sparse+https://index.crates.io/",
    ];
    match package.source.as_deref() {
        None => package
            .manifest_path
            .parent()
            .unwrap_or(&package.manifest_path)
            .display()
            .to_string(),
        Some(source) if CRATES_IO.contains(&source) => "crates.io".to_string(),
        Some(source) => match source.split_once('+') {
            Some(("registry" | "sparse", url)) => format!("the registry at {url}"),
            Some(("git", url)) => format!("git at {url}"),
            _ => source.to_string(),
        },
    }
}

/// Resolves one already-validated task name to an [`Entry`]: matched to a
/// real dependency, that dependency present on every target (not only under
/// a `cfg(...)` predicate), that dependency resolved to a package, that
/// package marked `task = true`, and built on the composed CLI's own
/// `rituals` — each check running only once the one before it has
/// succeeded.
fn resolve_one(
    name: &Name,
    cli_package_name: &str,
    node: &Node,
    metadata: &Metadata,
) -> Result<Entry, Failure> {
    match find_dependency(name, node, &metadata.packages) {
        // A dev- or build-dependency is not a normal dependency at all, the
        // same refusal as no entry matching by name.
        DependencyFound::Absent | DependencyFound::DevelopmentOrBuildOnly => {
            Err(no_dependency_called(name, cli_package_name))
        }
        DependencyFound::OnlyUnderTargets(predicates) => {
            Err(conditional_dependency(name, cli_package_name, &predicates))
        }
        DependencyFound::Normal {
            package,
            extern_identifier,
        } => match declaration(package) {
            TaskDeclaration::Task => ensure_same_rituals(metadata, node, package)
                .map(|()| Entry::new(name.as_str(), extern_identifier))
                .map_err(|disagreement| {
                    rituals_refusal(&disagreement, name, cli_package_name, Asking::Listed)
                }),
            TaskDeclaration::NotATask => Err(not_marked(name.as_str(), &package.name)),
            TaskDeclaration::NotABoolean => Err(Failure::new(format!(
                "`{}` declares [package.metadata.ritual] task, but its value is not a boolean; \
                 it reads `task = true`",
                package.name
            ))),
        },
    }
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

/// The refusal for a key no dependency of the composed CLI answers to.
fn no_such_dependency(key: &Name, cli_package_name: &str) -> Failure {
    Failure::new(format!(
        "`{cli_package_name}` has no dependency called `{key}`"
    ))
}

/// The refusal for a key whose dependency is only a dev- or build-dependency:
/// the composed CLI's own build never has it, so a command could not run.
fn only_a_dev_or_build_dependency(key: &Name, cli_package_name: &str) -> Failure {
    Failure::new(format!(
        "`{key}` is only a dev- or build-dependency of `{cli_package_name}`; a task has to be \
         a dependency under [dependencies]"
    ))
}

/// The refusal for a dependency whose crate does not declare `task = true`,
/// for a key not in the list yet. It names the key as well when the key is
/// not the crate's own name, since that is the name a person typed.
fn not_a_task_crate(key: &Name, crate_name: &str) -> Failure {
    let subject = if key.as_str() == crate_name {
        format!("`{crate_name}` is not a task crate")
    } else {
        format!("`{key}` resolves to `{crate_name}`, which is not a task crate")
    };
    Failure::new(format!(
        "{subject}: it does not declare `task = true` in its own [package.metadata.ritual], and \
         only a task crate can be a command; choose a task crate, or, if `{crate_name}` is \
         yours, declare `task = true` there first"
    ))
}

/// The refusal for a dependency whose crate declares `task` as something
/// other than a boolean, for a key not in the list yet.
fn task_value_not_a_boolean(crate_name: &str) -> Failure {
    Failure::new(format!(
        "`{crate_name}` declares [package.metadata.ritual] task, but its value is not a \
         boolean; choose a task crate, or, if `{crate_name}` is yours, make it read \
         `task = true`"
    ))
}

#[cfg(test)]
mod tests {
    use rituals::Name;

    use super::{resolve, resolve_excluding};
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

    /// The refusal `ensure_dependency_is_a_task` gives for `key` against the
    /// fixture as edited by `edit`, or `None` when it accepts it.
    fn refusal_for(
        key: &str,
        edit: impl FnOnce(&mut Metadata),
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        edit(&mut metadata);
        let key = Name::new(key)?;
        Ok(metadata
            .ensure_dependency_is_a_task("demo-ritual", &key)
            .err()
            .map(|failure| failure.to_string()))
    }

    /// Sets the `[package.metadata]` of the fixture's package `package_name`.
    fn set_package_metadata(
        metadata: &mut Metadata,
        package_name: &str,
        metadata_value: serde_json::Value,
    ) {
        let package = metadata
            .packages
            .iter_mut()
            .find(|package| package.name == package_name);
        assert!(
            package.is_some(),
            "expected {package_name} among the fixture's packages"
        );
        if let Some(package) = package {
            package.metadata = metadata_value;
        }
    }

    #[test]
    fn a_dependency_on_a_task_crate_is_accepted_under_its_own_name() -> TestOutcome {
        assert_eq!(refusal_for("task-true", |_| {})?, None);
        Ok(())
    }

    #[test]
    fn a_dependency_reached_through_a_rename_is_accepted_under_the_rename() -> TestOutcome {
        assert_eq!(refusal_for("renamed", |_| {})?, None);
        Ok(())
    }

    #[test]
    fn a_dependency_whose_key_is_a_keyword_is_accepted() -> TestOutcome {
        assert_eq!(refusal_for("move", |_| {})?, None);
        Ok(())
    }

    /// The tasks list is what an import is about to extend, so the check
    /// must not read it: here the list is empty and the dependency is
    /// accepted all the same.
    #[test]
    fn the_tasks_list_is_not_consulted() -> TestOutcome {
        let outcome = refusal_for("task-true", |metadata| {
            set_demo_ritual_metadata(metadata, serde_json::json!({ "ritual": { "tasks": [] } }));
        })?;
        assert_eq!(outcome, None);
        Ok(())
    }

    #[test]
    fn a_dependency_on_a_crate_with_no_ritual_table_is_refused_as_not_a_task() -> TestOutcome {
        assert_eq!(
            refusal_for("task-null", |_| {})?.as_deref(),
            Some(
                "`task-null` is not a task crate: it does not declare `task = true` in its own \
                 [package.metadata.ritual], and only a task crate can be a command; choose a \
                 task crate, or, if `task-null` is yours, declare `task = true` there first"
            )
        );
        Ok(())
    }

    #[test]
    fn a_dependency_declaring_task_false_is_refused_as_not_a_task() -> TestOutcome {
        let outcome = refusal_for("task-true", |metadata| {
            set_package_metadata(
                metadata,
                "task-true",
                serde_json::json!({ "ritual": { "task": false } }),
            );
        })?;
        assert!(
            outcome
                .as_deref()
                .is_some_and(|message| message.starts_with("`task-true` is not a task crate:")),
            "expected the not-a-task refusal; got {outcome:?}"
        );
        Ok(())
    }

    #[test]
    fn a_renamed_dependency_that_is_not_a_task_names_both_the_key_and_the_crate() -> TestOutcome {
        let outcome = refusal_for("renamed", |metadata| {
            set_package_metadata(metadata, "task-renamed-source", serde_json::Value::Null);
        })?;
        assert_eq!(
            outcome.as_deref(),
            Some(
                "`renamed` resolves to `task-renamed-source`, which is not a task crate: it does \
                 not declare `task = true` in its own [package.metadata.ritual], and only a task \
                 crate can be a command; choose a task crate, or, if `task-renamed-source` is \
                 yours, declare `task = true` there first"
            )
        );
        Ok(())
    }

    #[test]
    fn a_task_value_that_is_not_a_boolean_is_refused_saying_what_it_should_read() -> TestOutcome {
        assert_eq!(
            refusal_for("task-malformed", |_| {})?.as_deref(),
            Some(
                "`task-malformed` declares [package.metadata.ritual] task, but its value is not a \
                 boolean; choose a task crate, or, if `task-malformed` is yours, make it read \
                 `task = true`"
            )
        );
        Ok(())
    }

    #[test]
    fn a_key_with_no_dependency_is_refused_naming_the_package_and_the_key() -> TestOutcome {
        assert_eq!(
            refusal_for("no-such-key", |_| {})?.as_deref(),
            Some("`demo-ritual` has no dependency called `no-such-key`")
        );
        Ok(())
    }

    #[test]
    fn a_dev_only_dependency_is_refused_saying_a_task_has_to_be_a_normal_one() -> TestOutcome {
        assert_eq!(
            refusal_for("task-dev-only", |_| {})?.as_deref(),
            Some(
                "`task-dev-only` is only a dev- or build-dependency of `demo-ritual`; a task has \
                 to be a dependency under [dependencies]"
            )
        );
        Ok(())
    }

    #[test]
    fn a_dependency_present_only_under_a_target_predicate_is_refused_naming_it() -> TestOutcome {
        let outcome = refusal_for("task-true", |metadata| {
            let node = demo_ritual_node(metadata);
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
        })?;
        assert_eq!(
            outcome.as_deref(),
            Some(
                "`task-true` is a dependency of `demo-ritual` only under `cfg(windows)`, but a \
                 task has to be present on every target the command line builds for; declare it \
                 under [dependencies]"
            )
        );
        Ok(())
    }

    #[test]
    fn a_package_that_is_not_a_workspace_member_is_refused_before_any_dependency_is_read()
    -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        let outcome = metadata.ensure_dependency_is_a_task("nonexistent", &Name::new("task-true")?);

        assert!(
            outcome.is_err(),
            "expected an unknown package to be refused"
        );
        if let Err(failure) = outcome {
            assert!(
                failure
                    .to_string()
                    .contains("`nonexistent` is not a member of the workspace"),
                "unexpected refusal: {failure}"
            );
        }
        Ok(())
    }

    /// The two checks share one rule: for every dependency key the fixture
    /// offers, in every state it offers one, a key the import check accepts
    /// is a key the resolver accepts once that key is in the list, and a key
    /// it refuses is one the resolver refuses.
    #[test]
    fn the_import_check_and_the_resolver_accept_and_refuse_the_same_keys() -> TestOutcome {
        let keys = [
            "task-true",
            "renamed",
            "move",
            "task-null",
            "task-malformed",
            "task-dev-only",
            "no-such-key",
            "std",
            "core",
        ];
        for key in keys {
            let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
            set_demo_ritual_metadata(
                &mut metadata,
                serde_json::json!({ "ritual": { "tasks": [key] } }),
            );

            let import_check =
                metadata.ensure_dependency_is_a_task("demo-ritual", &Name::new(key)?);
            let resolver = resolve(&metadata, "demo-ritual");

            assert_eq!(
                import_check.is_ok(),
                resolver.is_ok(),
                "`{key}`: import check said {import_check:?}, resolver said {resolver:?}"
            );
        }
        Ok(())
    }

    /// The fixture's resolve node for the package `package_name`.
    fn node_of<'a>(
        metadata: &'a mut Metadata,
        package_name: &str,
    ) -> Result<&'a mut Node, Box<dyn std::error::Error>> {
        let id = metadata
            .packages
            .iter()
            .find(|package| package.name == package_name)
            .map(|package| package.id.clone())
            .ok_or_else(|| format!("expected {package_name} among the fixture's packages"))?;
        Ok(metadata
            .resolve
            .nodes
            .iter_mut()
            .find(|node| node.id == id)
            .ok_or_else(|| format!("expected a resolve node for {package_name}"))?)
    }

    /// Builds `task-true` on a second package called `rituals`, at `version`
    /// and from `source`, beside the one the fixture's CLI uses: a copy of
    /// that package under another id, with `task-true`'s `rituals` edge
    /// pointed at it, as `cargo metadata` reports two copies of one crate.
    fn build_task_true_on_another_rituals(
        metadata: &mut Metadata,
        version: &str,
        source: Option<&str>,
    ) -> TestOutcome {
        let mut other = parse(DEMO_WORKSPACE.as_bytes())?
            .packages
            .into_iter()
            .find(|package| package.name == "rituals")
            .ok_or("expected rituals among the fixture's packages")?;
        other.id = format!("other-rituals#{version}");
        other.version = version.to_string();
        other.source = source.map(str::to_string);
        let other_id = other.id.clone();
        metadata.packages.push(other);

        let edge = node_of(metadata, "task-true")?
            .deps
            .iter_mut()
            .find(|dependency| dependency.name == "rituals")
            .ok_or("expected task-true to depend on rituals in the fixture")?;
        edge.pkg = other_id;
        Ok(())
    }

    /// What the import check and the resolver each say about `task-true`
    /// once `edit` has been made to the fixture, with `task-true` the one
    /// key in the list.
    fn both_refusals(
        edit: impl Fn(&mut Metadata) -> TestOutcome,
    ) -> Result<(Option<String>, Option<String>), Box<dyn std::error::Error>> {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        set_demo_ritual_metadata(
            &mut metadata,
            serde_json::json!({ "ritual": { "tasks": ["task-true"] } }),
        );
        edit(&mut metadata)?;
        let import_check = metadata
            .ensure_dependency_is_a_task("demo-ritual", &Name::new("task-true")?)
            .err()
            .map(|failure| failure.to_string());
        let resolver = resolve(&metadata, "demo-ritual")
            .err()
            .map(|failure| failure.to_string());
        Ok((import_check, resolver))
    }

    #[test]
    fn every_fixture_task_shares_the_clis_rituals_and_is_accepted() -> TestOutcome {
        let (import_check, resolver) = both_refusals(|_| Ok(()))?;
        assert_eq!((import_check, resolver), (None, None));
        Ok(())
    }

    #[test]
    fn a_task_built_for_another_release_of_rituals_is_refused_naming_both_versions() -> TestOutcome
    {
        let (import_check, resolver) = both_refusals(|metadata| {
            build_task_true_on_another_rituals(
                metadata,
                "0.2.0",
                Some("registry+https://github.com/rust-lang/crates.io-index"),
            )
        })?;

        assert_eq!(
            import_check.as_deref(),
            Some(
                "`task-true` is built for rituals 0.2.0 and this project uses 0.1.2; import a \
                 release of it made for 0.1"
            )
        );
        assert_eq!(
            resolver.as_deref(),
            Some(
                "`task-true` is named in [package.metadata.ritual] tasks, but it is built for \
                 rituals 0.2.0 and this project uses 0.1.2; depend on a release of it made for \
                 0.1, or drop `task-true` from the list"
            )
        );
        Ok(())
    }

    /// The same version from two places is still two crates, so the versions
    /// alone cannot say why: the refusal names where each comes from.
    #[test]
    fn a_task_built_on_the_same_version_from_elsewhere_is_refused_naming_both_sources()
    -> TestOutcome {
        let (import_check, resolver) = both_refusals(|metadata| {
            build_task_true_on_another_rituals(
                metadata,
                "0.1.2",
                Some("registry+https://github.com/rust-lang/crates.io-index"),
            )
        })?;

        assert_eq!(
            import_check.as_deref(),
            Some(
                "`task-true` is built for rituals 0.1.2 from crates.io and this project uses \
                 rituals 0.1.2 from /scrubbed/checkout/rituals, which Rust reads as two different \
                 crates; build both on the same rituals"
            )
        );
        assert!(
            resolver
                .as_deref()
                .is_some_and(|message| message.starts_with(
                    "`task-true` is named in [package.metadata.ritual] tasks, but it is built for \
                 rituals 0.1.2 from crates.io"
                ) && message
                    .ends_with(", or drop `task-true` from the list")),
            "expected the resolver to refuse the same task; got {resolver:?}"
        );
        Ok(())
    }

    #[test]
    fn a_task_crate_that_does_not_depend_on_rituals_is_refused() -> TestOutcome {
        let (import_check, resolver) = both_refusals(|metadata| {
            node_of(metadata, "task-true")?
                .deps
                .retain(|dependency| dependency.name != "rituals");
            Ok(())
        })?;

        assert_eq!(
            import_check.as_deref(),
            Some(
                "`task-true` declares `task = true` but does not depend on rituals, so it has no \
                 task to hand a command line; choose a task crate built on rituals"
            )
        );
        assert_eq!(
            resolver.as_deref(),
            Some(
                "`task-true` is named in [package.metadata.ritual] tasks, but `task-true` does \
                 not depend on rituals, so it has no task to hand a command line; drop \
                 `task-true` from the list"
            )
        );
        Ok(())
    }

    #[test]
    fn a_command_line_that_does_not_depend_on_rituals_mounts_no_task() -> TestOutcome {
        let (import_check, resolver) = both_refusals(|metadata| {
            node_of(metadata, "demo-ritual")?
                .deps
                .retain(|dependency| dependency.name != "rituals");
            Ok(())
        })?;

        assert_eq!(
            import_check.as_deref(),
            Some(
                "`demo-ritual` does not depend on rituals, which its generated file is built on; \
                 add `rituals` under its [dependencies]"
            )
        );
        assert!(
            resolver
                .as_deref()
                .is_some_and(|message| message.contains("`demo-ritual` does not depend on rituals")),
            "expected the resolver to refuse the same list; got {resolver:?}"
        );
        Ok(())
    }

    #[test]
    fn a_listed_key_that_would_hide_std_or_core_is_refused() -> TestOutcome {
        for key in ["std", "core"] {
            let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
            set_demo_ritual_metadata(
                &mut metadata,
                serde_json::json!({ "ritual": { "tasks": ["task-true", key] } }),
            );

            let failure = resolve(&metadata, "demo-ritual")
                .err()
                .map(|failure| failure.to_string());

            assert_eq!(
                failure,
                Some(format!(
                    "`{key}` would hide Rust's own `{key}` crate, which the generated command \
                     line is built on, and it would no longer compile; give this task another key"
                )),
                "`{key}` must be refused before it is matched to a dependency"
            );
        }
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

    #[test]
    fn excluding_a_broken_key_lets_the_rest_resolve() -> TestOutcome {
        // `task-null` is a dependency that does not declare `task = true`,
        // so a list holding it is refused; leaving that key out is what
        // lets the others be checked while it is being removed.
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        set_demo_ritual_metadata(
            &mut metadata,
            serde_json::json!({ "ritual": { "tasks": ["task-true", "task-null", "move"] } }),
        );
        assert!(
            resolve(&metadata, "demo-ritual").is_err(),
            "the list was meant to be broken by task-null"
        );

        let entries = resolve_excluding(&metadata, "demo-ritual", "task-null")?;

        assert_eq!(
            entries,
            vec![
                Entry::new("task-true", "task_true"),
                Entry::new("move", "move"),
            ]
        );
        Ok(())
    }

    #[test]
    fn excluding_a_key_still_refuses_another_broken_one() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        set_demo_ritual_metadata(
            &mut metadata,
            serde_json::json!({
                "ritual": { "tasks": ["task-true", "task-null", "task-malformed"] }
            }),
        );

        let result = resolve_excluding(&metadata, "demo-ritual", "task-null");

        let failure = result
            .err()
            .ok_or("task-malformed was meant to be refused")?;
        assert!(
            failure.to_string().contains("task-malformed"),
            "failure was: {failure}"
        );
        Ok(())
    }

    #[test]
    fn excluding_a_key_that_is_not_listed_resolves_the_whole_list() -> TestOutcome {
        let metadata = parse(DEMO_WORKSPACE.as_bytes())?;

        assert_eq!(
            resolve_excluding(&metadata, "demo-ritual", "absent")?,
            resolve(&metadata, "demo-ritual")?
        );
        Ok(())
    }

    #[test]
    fn excluding_a_key_that_is_listed_twice_leaves_out_both() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        set_demo_ritual_metadata(
            &mut metadata,
            serde_json::json!({ "ritual": { "tasks": ["task-true", "move", "task-true"] } }),
        );

        let entries = resolve_excluding(&metadata, "demo-ritual", "task-true")?;

        assert_eq!(entries, vec![Entry::new("move", "move")]);
        Ok(())
    }

    #[test]
    fn excluding_a_key_still_refuses_a_list_of_the_wrong_shape() -> TestOutcome {
        let mut metadata = parse(DEMO_WORKSPACE.as_bytes())?;
        set_demo_ritual_metadata(
            &mut metadata,
            serde_json::json!({ "ritual": { "tasks": "task-true" } }),
        );

        let failure = resolve_excluding(&metadata, "demo-ritual", "task-true")
            .err()
            .ok_or("a list of the wrong shape was meant to be refused")?;

        assert!(failure.to_string().contains("is not a list of strings"));
        Ok(())
    }
}
