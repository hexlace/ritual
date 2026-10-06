//! The refusals `create` makes inside a project, each decided before
//! anything is written.

use std::path::Path;

use rituals::{Failure, Name, Outcome};
use rituals_compose::layout::TaskPlace;
use rituals_compose::manifest::{self, Manifest};
use rituals_compose::rust_name::other_spelling_clause;

/// Refuses when `name` equals the bin name the running command line was
/// built as — the slot [`rituals_compose::top_level::ensure_command_is_free`] deliberately
/// leaves free for a bin-name-mounted bundle's own manifest entry, and for
/// the child that bundle promotes when the two happen to share a key.
/// `create` only ever scaffolds a plain task, and a plain task sitting there
/// makes the command line refuse to start at its next build — so this
/// refuses on `create`'s own behalf, earlier and with different wording
/// than the shared check.
//
// No assertions beyond the one comparison that is the whole body,
// `name.as_str() == binary_name`, returning early on a match — a second
// check here could only restate that comparison a different way. The other
// half of the contract — a different name passes, a name that only contains
// the bin name passes, a bin name that looks like a package name is still
// refused — is held by the unit tests beside this function.
pub(super) fn ensure_the_name_is_not_the_bin_name(name: &Name, binary_name: &str) -> Outcome {
    if name.as_str() == binary_name {
        return Err(bin_name_refusal(binary_name));
    }
    Ok(())
}

/// The refusal for a name equal to the bin's own name.
//
// Whatever is mounted under the bin's name becomes the command line's own
// top level and has to be a bundle, and `create` only scaffolds plain tasks.
// That rule is in the design document for anyone mounting a bundle by hand;
// the person who typed `create` needs only the remedy.
fn bin_name_refusal(binary_name: &str) -> Failure {
    Failure::new(format!(
        "`{binary_name}` is reserved for this command line's own commands; give this task \
         another name"
    ))
}

/// The two commands an already-imported refusal can send a person to, each
/// spelled as [`rituals_compose::top_level::management_command`] spells it for this command
/// line.
///
/// Both are plain command strings, so as two positional arguments they could
/// be swapped without the compiler noticing; named fields cannot be.
pub(super) struct RemedyCommands<'a> {
    /// How a person types this command line's `regenerate`.
    pub(super) regenerate: &'a str,
    /// How a person types this command line's `import`.
    pub(super) import: &'a str,
}

/// Refuses when `name` is already spoken for in the composed CLI's own
/// manifest — a dependency (whether or not it is also listed), or listed
/// with no matching dependency. Runs before the task's directory is even
/// looked at: all three of these states are about the manifest, true
/// or false regardless of what is on disk, and a leftover directory's own
/// refusal ([`leftover_refusal`]) is only accurate once none of them apply.
///
/// The two flags decide four genuinely different outcomes, so they are
/// matched as a pair rather than read through a sequential if-chain: a
/// reader sees all four states at once, and the compiler checks that none
/// of them was left out.
///
/// `add`, `create` and `regenerate` are not reserved names; this refusal is
/// about a key already taken in this composed CLI's own manifest and nothing
/// else. Whether `add` is free at the top level depends on where ritual's own
/// bundle is mounted. In a project whose binary is called `ritual`, the
/// bundle is flattened into the top level and already provides `add`, so
/// [`rituals_compose::top_level::ensure_command_is_free`] refuses the name before this
/// runs. In a project that named its own command line — `acme`, say — the
/// bundle stays under its own key, `add` is free, and the project may
/// scaffold a task of its own called `add`. That is the design, not an
/// oversight: a composed command line's top level is its own namespace, so
/// `acme add` can mean the project's own scaffolder while `acme ritual add`
/// stays ritual's.
///
/// A dependency counts as already declared when its key reads as `name` to
/// rustc, with `-` as `_` (see [`rituals_compose::metadata::Project::declares_dependency_key`]),
/// so the second arm names the underscore spelling too when `name` has a
/// hyphen: the manifest line it is about may be spelled either way.
///
/// `commands` holds how a person types this command line's `regenerate` and
/// `import`, as [`rituals_compose::top_level::management_command`] spells them, so every remedy
/// here can be copied as written.
pub(super) fn already_imported_refusal(
    package: &str,
    name: &Name,
    already_a_dependency: bool,
    already_listed: bool,
    commands: &RemedyCommands,
) -> Outcome {
    let RemedyCommands { regenerate, import } = commands;
    match (already_a_dependency, already_listed) {
        (true, true) => Err(Failure::new(format!(
            "`{name}` is already a task of `{package}`; if its command is missing from the \
             command line, run `{regenerate}`"
        ))),
        (true, false) => Err(Failure::new(format!(
            "`{package}` already has a dependency called `{name}`{other_spelling} that is not in \
             [package.metadata.ritual] tasks; add `\"{name}\"` to that list and run \
             `{regenerate}`",
            other_spelling = other_spelling_clause(name),
        ))),
        (false, true) => Err(Failure::new(format!(
            "`{name}` is named in [package.metadata.ritual] tasks but `{package}` has no \
             dependency called `{name}`; drop it from the list, then run create again or \
             `{import} <crate> {name}`"
        ))),
        (false, false) => Ok(()),
    }
}

/// The refusal for a workspace that already has a package called `name` —
/// a task's crate is named after the command, so this would collide with
/// it.
pub(super) fn package_name_taken_refusal(name: &Name, workspace_root: &Path) -> Failure {
    Failure::new(format!(
        "the workspace at {} already has a package called `{name}`; a task's crate is named \
         after the command",
        workspace_root.display()
    ))
}

/// Refuses when the workspace manifest cannot take the import: no
/// inheritable `rituals` source, or no `[workspace] members` list to
/// append to.
pub(super) fn ensure_workspace_can_take_the_import(workspace_manifest: &Manifest) -> Outcome {
    let path = workspace_manifest.path();
    if !workspace_manifest.declares_workspace_dependency("rituals") {
        return Err(Failure::new(format!(
            "create needs `rituals` in [workspace.dependencies] of {}, so that a task crate's \
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

/// The refusal for a task directory that already exists: either it holds a
/// task crate this project does not import (a leftover from a run that did
/// not finish), or it is an unrelated collision.
pub(super) fn leftover_refusal(
    package: &str,
    name: &Name,
    place: &TaskPlace,
    dependency_path: &str,
    regenerate: &str,
) -> Failure {
    let manifest_path = place.directory().join("Cargo.toml");
    let directory = place.from_the_root();
    if !manifest::declares_a_task_crate(&manifest_path) {
        return Failure::new(format!("refusing to create {directory}: it already exists"));
    }

    Failure::new(format!(
        "refusing to create {directory}: it already exists and is a task crate `{package}` \
         does not import; add `{name} = {{ path = \"{dependency_path}\" }}` to `{package}`'s \
         [dependencies] and `\"{name}\"` to its [package.metadata.ritual] tasks, then run \
         `{regenerate}`"
    ))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rituals::Name;
    use rituals_compose::layout;
    use rituals_compose::manifest::Manifest;
    use rituals_compose::source::Source;
    use rituals_compose::task_crate::{self, Audience};

    use super::{
        RemedyCommands, already_imported_refusal, ensure_the_name_is_not_the_bin_name,
        ensure_workspace_can_take_the_import, leftover_refusal, package_name_taken_refusal,
    };
    use crate::test_support::{ScratchDir, TestOutcome};

    /// How a default project's `regenerate` is typed, handed to the
    /// refusals the way `prepare` hands them the real spelling.
    const REGENERATE: &str = "cargo ritual regenerate";

    /// How a default project's `import` is typed, likewise.
    const IMPORT: &str = "cargo ritual import";

    const COMMANDS: RemedyCommands<'_> = RemedyCommands {
        regenerate: REGENERATE,
        import: IMPORT,
    };

    /// A `Name` from a literal already known to be valid: unwrapping a
    /// `Result` this test module already controls, without `.unwrap()`
    /// itself.
    fn valid_name(value: &str) -> Name {
        Name::new(value).expect("a test passes only names it knows are valid")
    }

    /// A task made twice lands here, since the first run made it a
    /// dependency and listed it. Its command can still be missing from the
    /// command line, when the generated file has been edited or reverted
    /// since, so the remedy is the one that puts it back.
    #[test]
    fn already_imported_refusal_refuses_a_dependency_that_is_also_listed() {
        let result =
            already_imported_refusal("demo-ritual", &valid_name("lint"), true, true, &COMMANDS);
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
            already_imported_refusal("demo-ritual", &valid_name("lint"), true, false, &COMMANDS);
        assert!(result.is_err(), "expected the import to be refused");
        if let Err(error) = result {
            assert!(error.to_string().contains("not in"));
        }
    }

    /// Rust reads a hyphen and an underscore in a key as one name, so a key
    /// the person typed with a hyphen can be taken by a dependency declared
    /// with an underscore they cannot find by searching for what they typed.
    /// The refusal names both spellings, so the manifest line it is about is
    /// the one they find.
    #[test]
    fn already_imported_refusal_names_the_underscore_spelling_of_a_hyphenated_key() {
        let result =
            already_imported_refusal("demo-ritual", &valid_name("a-b"), true, false, &COMMANDS);
        assert!(result.is_err(), "expected the import to be refused");
        if let Err(error) = result {
            assert_eq!(
                error.to_string(),
                "`demo-ritual` already has a dependency called `a-b` (or `a_b`, which Rust reads \
                 as the same name) that is not in [package.metadata.ritual] tasks; add `\"a-b\"` \
                 to that list and run `cargo ritual regenerate`"
            );
        }
    }

    #[test]
    fn already_imported_refusal_offers_no_second_spelling_for_a_key_with_no_hyphen() {
        let result =
            already_imported_refusal("demo-ritual", &valid_name("lint"), true, false, &COMMANDS);
        assert!(result.is_err(), "expected the import to be refused");
        if let Err(error) = result {
            assert_eq!(
                error.to_string(),
                "`demo-ritual` already has a dependency called `lint` that is not in \
                 [package.metadata.ritual] tasks; add `\"lint\"` to that list and run \
                 `cargo ritual regenerate`"
            );
        }
    }

    /// `tasks = ["zzz"]` names a task with no matching dependency, which is
    /// not a leftover directory and not a name already fully imported —
    /// `create zzz` must refuse before it ever appends `zzz` to the list a
    /// second time.
    #[test]
    fn already_imported_refusal_refuses_a_name_listed_without_a_dependency() {
        let result =
            already_imported_refusal("demo-ritual", &valid_name("zzz"), false, true, &COMMANDS);
        assert!(result.is_err(), "expected the import to be refused");
        if let Err(error) = result {
            let message = error.to_string();
            assert!(message.contains("named in [package.metadata.ritual] tasks"));
            assert!(message.contains("no dependency called `zzz`"));
            assert!(
                message.contains(
                    "drop it from the list, then run create again or `cargo ritual import <crate> zzz`"
                ),
                "expected the drop first, then either add or the import command; message was: \
                 {message}"
            );
            assert!(
                !message.contains("by hand"),
                "a person is never sent to edit the manifest for the dependency; message was: \
                 {message}"
            );
        }
    }

    #[test]
    fn already_imported_refusal_allows_a_name_that_is_neither() {
        let result = already_imported_refusal(
            "demo-ritual",
            &valid_name("second"),
            false,
            false,
            &COMMANDS,
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

    #[test]
    fn a_package_name_a_member_already_has_is_refused_naming_the_workspace() {
        let refusal = package_name_taken_refusal(&valid_name("lint"), Path::new("/work/demo"));
        assert_eq!(
            refusal.to_string(),
            "the workspace at /work/demo already has a package called `lint`; a task's crate \
             is named after the command"
        );
    }

    /// A workspace manifest written to a scratch file and read back as the
    /// run reads it.
    fn workspace_manifest(
        tag: &str,
        text: &str,
    ) -> Result<(ScratchDir, Manifest), Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new(tag)?;
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(&path, text)?;
        let manifest = Manifest::read(&path)?;
        Ok((scratch, manifest))
    }

    #[test]
    fn a_workspace_that_declares_rituals_and_has_a_members_list_can_take_the_import() -> TestOutcome
    {
        let (_scratch, manifest) = workspace_manifest(
            "can-take",
            "[workspace]\nmembers = []\n\n[workspace.dependencies]\nrituals = \"1\"\n",
        )?;
        assert!(ensure_workspace_can_take_the_import(&manifest).is_ok());
        Ok(())
    }

    #[test]
    fn a_workspace_without_a_rituals_dependency_is_refused_saying_how_to_declare_it() -> TestOutcome
    {
        let (_scratch, manifest) = workspace_manifest("no-rituals", "[workspace]\nmembers = []\n")?;
        let refused = ensure_workspace_can_take_the_import(&manifest)
            .err()
            .ok_or("expected a workspace with no `rituals` to be refused")?
            .to_string();
        assert!(
            refused.starts_with("create needs `rituals` in [workspace.dependencies] of "),
            "{refused}"
        );
        assert!(
            refused.contains(&format!("`rituals = \"{}\"`", rituals::VERSION)),
            "{refused}"
        );
        Ok(())
    }

    #[test]
    fn a_workspace_without_a_members_list_is_refused_saying_to_add_one() -> TestOutcome {
        let (_scratch, manifest) = workspace_manifest(
            "no-members",
            "[workspace]\n\n[workspace.dependencies]\nrituals = \"1\"\n",
        )?;
        let refused = ensure_workspace_can_take_the_import(&manifest)
            .err()
            .ok_or("expected a workspace with no members list to be refused")?
            .to_string();
        assert!(
            refused.ends_with("has no [workspace] members list to append to; add `members = []`"),
            "{refused}"
        );
        Ok(())
    }

    /// A directory in the way that is not a task crate is an unrelated
    /// collision, and one that is a task crate is a leftover the project
    /// does not import, with the lines that would import it.
    #[test]
    fn a_task_directory_in_the_way_is_refused_as_a_collision_or_as_a_leftover() -> TestOutcome {
        let scratch = ScratchDir::new("leftover")?;
        let name = valid_name("lint");
        let place = layout::place_for(scratch.path(), &name);
        std::fs::create_dir_all(place.directory())?;

        let collision = leftover_refusal(
            "demo-ritual",
            &name,
            &place,
            "../.rituals/lint",
            "cargo ritual regenerate",
        );
        assert_eq!(
            collision.to_string(),
            "refusing to create .rituals/lint: it already exists"
        );

        std::fs::write(
            place.directory().join("Cargo.toml"),
            task_crate::manifest(&name, &Source::Inherited, Audience::Private),
        )?;
        let leftover = leftover_refusal(
            "demo-ritual",
            &name,
            &place,
            "../.rituals/lint",
            "cargo ritual regenerate",
        );
        assert_eq!(
            leftover.to_string(),
            "refusing to create .rituals/lint: it already exists and is a task crate \
             `demo-ritual` does not import; add `lint = { path = \"../.rituals/lint\" }` to \
             `demo-ritual`'s [dependencies] and `\"lint\"` to its [package.metadata.ritual] \
             tasks, then run `cargo ritual regenerate`"
        );
        Ok(())
    }
}
