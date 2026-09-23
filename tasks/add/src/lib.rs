//! The `add` task: scaffold a task crate in this project, import it, and
//! regenerate the task list.

mod import;
#[cfg(test)]
mod test_support;

use std::path::Path;

use import::Import;
use rituals::{CommandLine, Failure, Name, Outcome, Task, clap};
use rituals_compose::manifest::{self, Manifest};
use rituals_compose::{metadata, top_level};

/// `add`'s one argument: the name the new task will answer to.
#[derive(clap::Args)]
struct AddArguments {
    /// the name the new task will answer to
    name: String,
}

/// This task, for a command line to mount under whatever name imports it.
///
/// Reads the composed CLI's command line it is invoked with, the same
/// opt-in any task can use, to find itself among the workspace's members
/// and, once it has finished writing, to regenerate with.
//
// No `# Examples` section: a `task()` function has exactly one call-site
// shape — a mount line in a generated file — and an example here could only
// restate `let task = add::task();`, which shows nothing a reader needs.
#[must_use]
pub fn task() -> Task {
    Task::receiving_command_line(
        "scaffold a task crate in this project, import it, and regenerate",
        |command_line: &CommandLine, arguments: AddArguments| run(command_line, &arguments),
    )
}

fn run(command_line: &CommandLine, arguments: &AddArguments) -> Outcome {
    let import = prepare(command_line, arguments)?;
    import::finish(command_line, import)
}

/// Validates `name` first, since it is the one input here a person typed
/// directly rather than something already trusted by construction, then weighs
/// it against the bin name the running command line was built as, then against
/// that *running* command line's own top level — each cheaper and more specific
/// than the next, none of them needing a subprocess, and each a fact about the
/// name itself rather than about the project's wider state. The bin-name check
/// has to come before `ensure_command_is_free`: that check deliberately lets
/// the bin's own name through, because `regenerate` shares it and a
/// bin-name-mounted bundle's own manifest entry has exactly that key, so
/// refusing there would refuse `regenerate` forever. It also has to come before
/// `already_imported_refusal` below: in a default-scaffolded project the bundle
/// is already imported under that key, and that refusal would otherwise answer
/// a different question — "already a task" rather than "that slot has to be a
/// bundle" — with the wrong remedy. Every check after these three runs against
/// the project itself, once `cargo metadata` has been fetched, in this order:
/// `name` is not already imported, `tasks/<name>` is not a leftover directory,
/// no workspace member is already called `name`, the project's *existing* task
/// list resolves, the workspace manifest can take the import, then the composed
/// CLI's manifest is read. Every refusal about `name` itself comes before the
/// one about the project's existing list, so a leftover directory actually in
/// the way of the name just typed is never masked by an unrelated entry
/// elsewhere in the list — both are pure pre-write reads, so nothing is lost by
/// checking the more specific one first. Returns what `add` needs to write —
/// nothing is written until every refusal here has passed.
fn prepare(command_line: &CommandLine, arguments: &AddArguments) -> Result<Import, Failure> {
    let name = Name::new(&arguments.name)?;
    ensure_the_name_is_not_the_bin_name(&name, command_line.identity().binary_name())?;
    top_level::ensure_command_is_free(command_line, name.as_str())?;
    let package = command_line.identity().package_name();

    let current_dir = std::env::current_dir()
        .map_err(|error| Failure::new("reading the current directory failed").caused_by(error))?;
    let document = metadata::fetch(&current_dir)?;
    document.ensure_runs_in_its_own_project(package, "add", name.as_str())?;
    let project = document.locate_project(package)?;

    // Runs before the leftover check below: whether `name` is already a
    // dependency or already listed is a fact about the manifest, and it is
    // what tells `tasks/<name>` existing apart from `tasks/<name>` existing
    // *and being a leftover* — the only state `leftover_refusal` is true in.
    let regenerate = top_level::management_command(command_line, "regenerate");
    already_imported_refusal(
        package,
        &name,
        project.declares_dependency_key(&name),
        project.lists_task(&name),
        &regenerate,
    )?;

    let tasks_directory = project.workspace_root().join("tasks");
    let task_crate_dir = tasks_directory.join(name.as_str());
    assert!(
        task_crate_dir.starts_with(&tasks_directory),
        "joining a validated Name under tasks/ must stay under tasks/"
    );

    let cli_manifest_path = project.manifest_path().to_path_buf();
    let dependency_path = manifest::dependency_path(&cli_manifest_path, &task_crate_dir);

    if task_crate_dir.exists() {
        return Err(leftover_refusal(
            package,
            &name,
            &task_crate_dir,
            &dependency_path,
            &regenerate,
        ));
    }

    if document.has_workspace_member(&name) {
        return Err(package_name_taken_refusal(&name, project.workspace_root()));
    }

    // The same resolver `regenerate` runs, against the metadata already
    // fetched above — no second `cargo metadata` call. Its refusal is
    // returned as-is: it already says what is wrong and what to do, and
    // nothing about `name` has been written yet for it to undo.
    document.resolve_task_list(package)?;

    let workspace_manifest_path = project.workspace_root().join("Cargo.toml");
    let workspace_manifest = Manifest::read(&workspace_manifest_path)?;
    ensure_workspace_can_take_the_import(&workspace_manifest)?;

    let cli_manifest = Manifest::read(&cli_manifest_path)?;
    let tasks_directory_was_created = !tasks_directory.exists();

    Ok(Import {
        name,
        task_crate_dir,
        tasks_directory_was_created,
        workspace_manifest,
        cli_manifest,
        dependency_path,
        workspace_root: project.workspace_root().to_path_buf(),
    })
}

/// Refuses when `name` equals the bin name the running command line was
/// built as — the slot [`top_level::ensure_command_is_free`] deliberately
/// leaves free for a bin-name-mounted bundle's own manifest entry, and for
/// the child that bundle promotes when the two happen to share a key.
/// `add` only ever scaffolds a plain task, and a plain task sitting there
/// makes the command line refuse to start at its next build — so this
/// refuses on `add`'s own behalf, earlier and with different wording than
/// the shared check.
//
// No assertions beyond the one comparison that is the whole body,
// `name.as_str() == binary_name`, returning early on a match — a second
// check here could only restate that comparison a different way. The other
// half of the contract — a different name passes, a name that only contains
// the bin name passes, a bin name that looks like a package name is still
// refused — is held by the unit tests beside this function.
fn ensure_the_name_is_not_the_bin_name(name: &Name, binary_name: &str) -> Outcome {
    if name.as_str() == binary_name {
        return Err(bin_name_refusal(binary_name));
    }
    Ok(())
}

/// The refusal for a name equal to the bin's own name.
//
// Whatever is mounted under the bin's name becomes the command line's own
// top level and has to be a bundle, and `add` only scaffolds plain tasks.
// That rule is in the design document for anyone mounting a bundle by hand;
// the person who typed `add` needs only the remedy.
fn bin_name_refusal(binary_name: &str) -> Failure {
    Failure::new(format!(
        "`{binary_name}` is reserved for this command line's own commands; give this task \
         another name"
    ))
}

/// Refuses when `name` is already spoken for in the composed CLI's own
/// manifest — a dependency (whether or not it is also listed), or listed
/// with no matching dependency. Runs before the `tasks/<name>` directory is
/// even looked at: all three of these states are about the manifest, true
/// or false regardless of what is on disk, and a leftover directory's own
/// refusal ([`leftover_refusal`]) is only accurate once none of them apply.
///
/// The two flags decide four genuinely different outcomes, so they are
/// matched as a pair rather than read through a sequential if-chain: a
/// reader sees all four states at once, and the compiler checks that none
/// of them was left out.
///
/// `add` and `regenerate` are not reserved names; this refusal is about a
/// key already taken in this composed CLI's own manifest and nothing else.
/// Whether `add` is free at the top level depends on where ritual's own
/// bundle is mounted. In a project whose binary is called `ritual`, the
/// bundle is flattened into the top level and already provides `add`, so
/// [`top_level::ensure_command_is_free`] refuses the name before this
/// runs. In a project that named its own command line — `acme`, say — the
/// bundle stays under its own key, `add` is free, and the project may
/// scaffold a task of its own called `add`. That is the design, not an
/// oversight: a composed command line's top level is its own namespace, so
/// `acme add` can mean the project's own scaffolder while `acme ritual add`
/// stays ritual's.
///
/// `regenerate` is how a person types this command line's `regenerate`, as
/// [`top_level::management_command`] spells it, so every remedy here can be
/// copied as written.
fn already_imported_refusal(
    package: &str,
    name: &Name,
    already_a_dependency: bool,
    already_listed: bool,
    regenerate: &str,
) -> Outcome {
    match (already_a_dependency, already_listed) {
        (true, true) => Err(Failure::new(format!(
            "`{name}` is already a task of `{package}`; if its command is missing from the \
             command line, run `{regenerate}`"
        ))),
        (true, false) => Err(Failure::new(format!(
            "`{package}` already has a dependency called `{name}` that is not in \
             [package.metadata.ritual] tasks; add `\"{name}\"` to that list and run \
             `{regenerate}`"
        ))),
        (false, true) => Err(Failure::new(format!(
            "`{name}` is named in [package.metadata.ritual] tasks but `{package}` has no \
             dependency called `{name}`; drop it from the list and run add again, or add the \
             dependency by hand and run `{regenerate}`"
        ))),
        (false, false) => Ok(()),
    }
}

/// The refusal for a workspace that already has a package called `name` —
/// a task's crate is named after the command, so this would collide with
/// it.
fn package_name_taken_refusal(name: &Name, workspace_root: &Path) -> Failure {
    Failure::new(format!(
        "the workspace at {} already has a package called `{name}`; a task's crate is named \
         after the command",
        workspace_root.display()
    ))
}

/// Refuses when the workspace manifest cannot take the import: no
/// inheritable `rituals` source, or no `[workspace] members` list to
/// append to.
fn ensure_workspace_can_take_the_import(workspace_manifest: &Manifest) -> Outcome {
    let path = workspace_manifest.path();
    if !workspace_manifest.declares_workspace_dependency("rituals") {
        return Err(Failure::new(format!(
            "add needs `rituals` in [workspace.dependencies] of {}, so that a task crate's \
             dependency on it does not change when the crate moves; add it there, as a \
             version such as `rituals = \"{}\"` or with a path or git source",
            path.display(),
            rituals::VERSION
        )));
    }
    if !workspace_manifest.has_workspace_members_list() {
        return Err(Failure::new(format!(
            "{} has no [workspace] members list to append to; add `members = []`",
            path.display()
        )));
    }
    Ok(())
}

/// The refusal for a `tasks/<name>` directory that already exists: either it
/// holds a task crate this project does not import (a leftover from a run
/// that did not finish), or it is an unrelated collision.
fn leftover_refusal(
    package: &str,
    name: &Name,
    task_crate_dir: &Path,
    dependency_path: &str,
    regenerate: &str,
) -> Failure {
    let manifest_path = task_crate_dir.join("Cargo.toml");
    if !manifest::declares_a_task_crate(&manifest_path) {
        return Failure::new(format!(
            "refusing to create tasks/{name}: it already exists"
        ));
    }

    Failure::new(format!(
        "refusing to create tasks/{name}: it already exists and is a task crate `{package}` \
         does not import; add `{name} = {{ path = \"{dependency_path}\" }}` to `{package}`'s \
         [dependencies] and `\"{name}\"` to its [package.metadata.ritual] tasks, then run \
         `{regenerate}`"
    ))
}

#[cfg(test)]
mod tests {
    use rituals::Name;

    use super::{already_imported_refusal, ensure_the_name_is_not_the_bin_name};

    /// How a default project's `regenerate` is typed, handed to the
    /// refusals the way `prepare` hands them the real spelling.
    const REGENERATE: &str = "cargo ritual regenerate";

    /// A `Name` from a literal already known to be valid — mirrors
    /// `tasks/create`'s own `demo_name` helper: unwrapping a `Result` this
    /// test module already controls, without `.unwrap()` itself.
    fn valid_name(value: &str) -> Name {
        Name::new(value).expect("a test passes only names it knows are valid")
    }

    /// This is also the message a retry hits after a post-commit
    /// regenerate failure: `add`'s manifest writes already succeeded, so a
    /// second `add <name>` (instead of the printed `regenerate`
    /// remedy) lands here rather than on `leftover_refusal` — the remedy
    /// text has to be true in both cases, not only the accidental-repeat
    /// one.
    #[test]
    fn already_imported_refusal_refuses_a_dependency_that_is_also_listed() {
        let result =
            already_imported_refusal("demo-ritual", &valid_name("lint"), true, true, REGENERATE);
        assert!(result.is_err(), "expected the import to be refused");
        if let Err(error) = result {
            let message = error.to_string();
            assert!(message.contains("`lint` is already a task of `demo-ritual`"));
            assert!(message.contains("if its command is missing from the command line"));
            assert!(message.contains("run `cargo ritual regenerate`"));
        }
    }

    #[test]
    fn already_imported_refusal_refuses_a_dependency_that_is_not_listed() {
        let result =
            already_imported_refusal("demo-ritual", &valid_name("lint"), true, false, REGENERATE);
        assert!(result.is_err(), "expected the import to be refused");
        if let Err(error) = result {
            assert!(error.to_string().contains("not in"));
        }
    }

    /// `tasks = ["zzz"]` names a task with no matching dependency, which is
    /// not a leftover directory and not a name already fully imported —
    /// `add zzz` must refuse before it ever appends `zzz` to the list a
    /// second time.
    #[test]
    fn already_imported_refusal_refuses_a_name_listed_without_a_dependency() {
        let result =
            already_imported_refusal("demo-ritual", &valid_name("zzz"), false, true, REGENERATE);
        assert!(result.is_err(), "expected the import to be refused");
        if let Err(error) = result {
            let message = error.to_string();
            assert!(message.contains("named in [package.metadata.ritual] tasks"));
            assert!(message.contains("no dependency called `zzz`"));
            assert!(message.contains("run add again"));
            assert!(message.contains("run `cargo ritual regenerate`"));
        }
    }

    #[test]
    fn already_imported_refusal_allows_a_name_that_is_neither() {
        let result = already_imported_refusal(
            "demo-ritual",
            &valid_name("second"),
            false,
            false,
            REGENERATE,
        );
        assert!(
            result.is_ok(),
            "expected a genuinely new name to be allowed: {result:?}"
        );
    }

    #[test]
    fn the_bin_name_is_refused_as_reserved_and_asks_for_another_name() {
        let result = ensure_the_name_is_not_the_bin_name(&valid_name("ritual"), "ritual");
        assert!(result.is_err(), "expected the bin's own name to be refused");
        if let Err(error) = result {
            assert_eq!(
                error.to_string(),
                "`ritual` is reserved for this command line's own commands; give this task \
                 another name"
            );
        }
    }

    #[test]
    fn a_different_name_passes() {
        let result = ensure_the_name_is_not_the_bin_name(&valid_name("lint"), "ritual");
        assert!(
            result.is_ok(),
            "expected a different name to pass: {result:?}"
        );
    }

    /// The exact-match half of the check, alongside the refusal test
    /// above: a name that merely contains the bin name is not the bin
    /// name.
    #[test]
    fn a_name_that_only_contains_the_bin_name_passes() {
        let result = ensure_the_name_is_not_the_bin_name(&valid_name("ritual-helper"), "ritual");
        assert!(
            result.is_ok(),
            "expected a name that only contains the bin name to pass: {result:?}"
        );
    }

    /// Pins that the check keys on the *bin* name, never on the package
    /// name: a bin that happens to be spelled like a package name is still
    /// refused when a task would be named the same.
    #[test]
    fn a_bin_name_that_looks_like_a_package_name_is_still_refused() {
        let result = ensure_the_name_is_not_the_bin_name(&valid_name("demo-ritual"), "demo-ritual");
        assert!(
            result.is_err(),
            "expected the bin name to be refused even when it looks like a package name"
        );
    }
}
