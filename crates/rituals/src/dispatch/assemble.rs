//! Assembling a composed command line's mounts, dropping — rather than
//! panicking on — an import that cannot be mounted; and flattening the one
//! bundle, if any, mounted under the compiled binary's own name.

use crate::name::Name;
use crate::outcome::Failure;
use crate::task::Task;

use super::{refusal_line, report_to_stderr};

/// The message written when an imported command can't be mounted — its name
/// collides with a name already assembled, or is not a usable [`Name`] —
/// with no prefix of its own: [`refusal_line`] is the one place that is
/// added.
fn mount_refusal(name: &str) -> String {
    format!(
        "`{name}` collides with an earlier import, or is not a usable name; dropped from this \
         command line — run `regenerate` to fix its generated file"
    )
}

/// Assembles `imported` into one command line, in order, dropping — rather
/// than panicking on — an import whose name is invalid or collides with a
/// name already assembled.
///
/// This is the second site of the check whose first site is the task-list
/// resolver: the resolver refuses to write a generated file with such a
/// name, and this is what stops the dispatcher mounting one anyway if the
/// file was hand-edited. It never panics and never bricks the command
/// line — `regenerate` stays reachable, so the next run clears it.
pub(super) fn assemble(
    binary_name: &str,
    imported: impl IntoIterator<Item = (&'static str, Task)>,
) -> Vec<(&'static str, Task)> {
    let mut tasks = Vec::new();

    for (name, task) in imported {
        let already_present = tasks.iter().any(|(existing, _)| *existing == name);
        let usable = Name::new(name).is_ok();

        if already_present || !usable {
            report_to_stderr(refusal_line(binary_name, mount_refusal(name)));
            continue;
        }

        tasks.push((name, task));
    }

    tasks
}

/// What [`flatten`] produces: the final top-level mount list, and the names
/// (if any) it promoted from a bundle mounted under the binary's own name —
/// the datum [`crate::CommandLine::flattened_commands`] reports. A named
/// struct rather than a tuple, because a bare two-`Vec` return is two
/// anonymous positions of one outer shape with nothing to tell them apart
/// at the call site.
#[derive(Debug)]
pub(super) struct TopLevel {
    pub(super) mounts: Vec<(&'static str, Task)>,
    pub(super) flattened: Vec<&'static str>,
}

/// Splices the children of the one mount whose key equals `binary_name`
/// into its position at the top level, leaving every other mount where it
/// was — the rule that makes whatever is mounted under a composed CLI's own
/// compiled name that CLI's top level. Identity, with an empty
/// [`TopLevel::flattened`], when no mount's key equals `binary_name`.
///
/// Runs after [`assemble`], so it always sees a deduplicated, validated
/// mount list; a name that collides only because of what flattening
/// produces is therefore always a genuinely new collision, never one
/// `assemble` already resolved. Flatten is top-level only and runs once: it
/// never descends into a mount's own children looking for a nested bundle
/// keyed on `binary_name`, and it never re-scans its own output for a
/// second flattening pass — a spliced-in child that happens to share the
/// bin's name is an ordinary top-level command from here on.
///
/// The mount list this reads comes from the generated file, written by
/// `regenerate` from the project's own manifest, and from the crates that
/// manifest names — all the project's own, none of it supplied by anyone
/// else. Only the shape is checked (is the one mount at `binary_name` a
/// bundle, do its children collide with anything), because the remedy is
/// always the project's own manifest and source.
///
/// # Errors
///
/// Refuses when the mount at `binary_name` is not a bundle, when
/// flattening it would give the top level two commands with one name, or
/// when the top level — before or after flattening — would answer to
/// `help`, which clap already gives every command that has subcommands.
pub(super) fn flatten(
    binary_name: &str,
    mounts: Vec<(&'static str, Task)>,
) -> Result<TopLevel, Failure> {
    let bin_name_index = mounts.iter().position(|(key, _)| *key == binary_name);

    let (mounts, flattened) = match bin_name_index {
        None => (mounts, Vec::new()),
        Some(index) => {
            let mut mounts = mounts;
            let (_, mount) = mounts.remove(index);
            let children = mount
                .into_children()
                .map_err(|_leaf| leaf_under_bin_name_refusal(binary_name))?;
            ensure_no_flatten_collision(binary_name, &mounts, &children)?;
            let flattened: Vec<&'static str> = children.iter().map(|(name, _)| *name).collect();
            mounts.splice(index..index, children);
            (mounts, flattened)
        }
    };

    ensure_no_top_level_help(&mounts)?;

    Ok(TopLevel { mounts, flattened })
}

/// The refusal for a plain (non-bundle) task mounted under the key equal to
/// the compiled binary's own name.
fn leaf_under_bin_name_refusal(bin_name: &str) -> Failure {
    // `bin_name` fills both roles in the sentence below — the colliding
    // mount key, and the name this command line was built as — because
    // they are the same string by construction: this refusal only ever
    // fires for the one mount whose key equals the compiled binary's own
    // name.
    Failure::new(format!(
        "`{bin_name}` is mounted under `{bin_name}`, the name this command line was built as, \
         so it is this command line's top level — but it is not a bundle. Make it one with \
         `Task::group`, or mount it under another name."
    ))
}

/// Refuses when any of `children`'s names already names one of
/// `surrounding_mounts` — checked over the whole of `surrounding_mounts`,
/// which holds every other mount regardless of whether it sat before or
/// after the bin-name mount in the original list, so a collision is caught
/// from either side.
fn ensure_no_flatten_collision(
    bin_name: &str,
    surrounding_mounts: &[(&'static str, Task)],
    children: &[(&'static str, Task)],
) -> Result<(), Failure> {
    for (child_name, _) in children {
        if surrounding_mounts
            .iter()
            .any(|(mount_name, _)| mount_name == child_name)
        {
            return Err(flatten_collision_refusal(bin_name, child_name));
        }
    }
    Ok(())
}

/// The refusal for a bundle's child whose name already matches another
/// top-level mount key.
///
/// This refuses rather than dropping one of the two with a warning, unlike
/// a literally duplicated line in the generated file: `regenerate` cannot
/// fix a name that appears nowhere in the generated file, since a
/// flattened child's name is compiled into a bundle crate rather than
/// written out in that file. And clap's duplicate-subcommand check is a
/// debug assertion: a debug build panics on two subcommands sharing one
/// name, while a release build silently keeps the first of the two and
/// drops the second — so silently keeping one of two here would make
/// one command mean different things in the two build profiles, rather
/// than refusing and deciding nothing.
fn flatten_collision_refusal(bin_name: &str, name: &str) -> Failure {
    Failure::new(format!(
        "`{name}` would be a top-level command of this command line twice: once from the \
         bundle mounted under `{bin_name}`, the name this command line was built as, and once \
         mounted under `{name}` itself. Rename the bundle's child, or mount that task under \
         another name."
    ))
}

/// Refuses when any of `mounts`' own keys is `help`.
///
/// `help` is not a name ritual reserves for itself — nothing here treats it
/// specially the way a genuinely reserved name would be. It collides
/// because clap gives every command with subcommands a `help` of its own,
/// for free, and a project is exactly as free to mount a task under that
/// key as under any other; ritual embraces that rather than fighting it,
/// and refuses the collision the same way any other top-level collision is
/// refused. Checked once, here, over the final top level — after
/// [`assemble`] and after any flattening above — so it catches a mount key
/// literally named `help`. A flattened child named `help` cannot reach
/// it: [`Task::group`] already refuses to build a bundle with one.
fn ensure_no_top_level_help(mounts: &[(&'static str, Task)]) -> Result<(), Failure> {
    if mounts.iter().any(|(key, _)| *key == "help") {
        return Err(help_collision_refusal());
    }
    Ok(())
}

/// The refusal for a top-level command named `help`.
fn help_collision_refusal() -> Failure {
    Failure::new(
        "`help` would be a top-level command of this command line twice: once as this mount, \
         and once as the `help` command clap gives every command that has subcommands. Mount \
         it under another name.",
    )
}

#[cfg(test)]
mod tests {
    use super::{assemble, flatten, mount_refusal};
    use crate::dispatch::refusal_line;
    use crate::task::Task;
    use crate::test_support::run_ok;

    fn task() -> Task {
        Task::new("a test task", run_ok)
    }

    #[test]
    fn imports_are_assembled_in_order() {
        let imported = vec![("regenerate", task()), ("greet", task())];
        let assembled = assemble("demo", imported);
        let names: Vec<&str> = assembled.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["regenerate", "greet"]);
    }

    #[test]
    fn an_import_colliding_with_an_earlier_import_is_dropped() {
        let imported = vec![("greet", task()), ("greet", task())];
        let assembled = assemble("demo", imported);
        let names: Vec<&str> = assembled.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["greet"]);
    }

    #[test]
    fn an_import_with_an_invalid_name_is_dropped() {
        let imported = vec![("regenerate", task()), ("../evil", task())];
        let assembled = assemble("demo", imported);
        let names: Vec<&str> = assembled.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["regenerate"]);
    }

    /// `add` is not reserved: nothing at the assembly step treats it
    /// specially, so an import named `add` mounts exactly like any other
    /// name.
    #[test]
    fn a_task_named_add_mounts_like_any_other_import() {
        let imported = vec![("add", task())];
        let assembled = assemble("demo", imported);
        let names: Vec<&str> = assembled.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["add"]);
    }

    /// The mount-refusal message, from `mount_refusal` alone with no prefix,
    /// still leads with the bin name once `refusal_line` wraps it —
    /// assertable directly against the two pure functions rather than only
    /// by capturing a subprocess's stderr.
    #[test]
    fn a_mount_refusal_leads_with_the_bin_name() {
        let line = refusal_line("ritual", mount_refusal("new"));
        assert!(line.starts_with("ritual: "));
        assert!(line.contains("`new`"));
        assert!(line.contains("collides"));
    }

    fn names_of(mounts: &[(&'static str, Task)]) -> Vec<&'static str> {
        mounts.iter().map(|(name, _)| *name).collect()
    }

    #[test]
    fn flatten_with_no_bin_name_mount_is_the_identity() {
        let mounts = vec![("add", task()), ("regenerate", task())];
        let result = flatten("acme", mounts);
        assert!(result.is_ok(), "expected no refusal: {result:?}");
        if let Ok(top_level) = result {
            assert_eq!(names_of(&top_level.mounts), ["add", "regenerate"]);
            assert!(
                top_level.flattened.is_empty(),
                "expected nothing to have been promoted: {:?}",
                top_level.flattened
            );
        }
    }

    #[test]
    fn flatten_splices_in_place_and_preserves_surrounding_order() {
        let bundle = Task::group("acme's own commands", [("add", task()), ("build", task())]);
        let mounts = vec![("lint", task()), ("acme", bundle), ("format", task())];
        let result = flatten("acme", mounts);
        assert!(result.is_ok(), "expected no refusal: {result:?}");
        if let Ok(top_level) = result {
            assert_eq!(
                names_of(&top_level.mounts),
                ["lint", "add", "build", "format"],
                "the bundle's children replace it in place, in their own order, and every \
                 surrounding mount stays exactly where it was"
            );
        }
    }

    /// The names `flatten` reports as promoted are exactly the bundle's own
    /// children, in their own order — the datum
    /// `CommandLine::flattened_commands` reports back to a task.
    #[test]
    fn flatten_reports_the_names_it_promoted() {
        let bundle = Task::group("acme's own commands", [("add", task()), ("build", task())]);
        let mounts = vec![("acme", bundle)];
        let result = flatten("acme", mounts);
        assert!(result.is_ok(), "expected no refusal: {result:?}");
        if let Ok(top_level) = result {
            assert_eq!(top_level.flattened, ["add", "build"]);
        }
    }

    /// A leaf mounted under the bin's own name — not a bundle — is refused,
    /// naming both the colliding key and the bin name (here, the same
    /// string by construction).
    #[test]
    fn a_leaf_under_the_bin_name_is_refused() {
        let mounts = vec![("acme", task())];
        let result = flatten("acme", mounts);
        assert!(result.is_err(), "expected a refusal: {result:?}");
        if let Err(failure) = result {
            let message = failure.to_string();
            assert!(message.contains("`acme`"), "message was: {message}");
            assert!(message.contains("Task::group"), "message was: {message}");
        }
    }

    /// A bundle's child colliding with a mount that sits *after* the
    /// bundle's own position in the original list is refused, naming the
    /// command and both sources.
    #[test]
    fn a_child_colliding_with_a_later_mount_key_is_refused() {
        let bundle = Task::group("acme's own commands", [("lint", task())]);
        let mounts = vec![("acme", bundle), ("lint", task())];
        let result = flatten("acme", mounts);
        assert!(result.is_err(), "expected a refusal: {result:?}");
        if let Err(failure) = result {
            let message = failure.to_string();
            assert!(message.contains("`lint`"), "message was: {message}");
            assert!(message.contains("`acme`"), "message was: {message}");
            assert!(message.contains("another name"), "message was: {message}");
        }
    }

    /// The same collision, checked from the other direction: a mount that
    /// sits *before* the bundle's own position in the original list. A
    /// one-directional scan (checking only what followed the bundle) would
    /// miss this.
    #[test]
    fn a_child_colliding_with_an_earlier_mount_key_is_refused() {
        let bundle = Task::group("acme's own commands", [("lint", task())]);
        let mounts = vec![("lint", task()), ("acme", bundle)];
        let result = flatten("acme", mounts);
        assert!(result.is_err(), "expected a refusal: {result:?}");
        if let Err(failure) = result {
            let message = failure.to_string();
            assert!(message.contains("`lint`"), "message was: {message}");
        }
    }

    /// A child that happens to share the bin's own name is promoted to the
    /// top level like any other child — and, once there, is an ordinary
    /// top-level command, not flattened a second time.
    #[test]
    fn a_child_named_the_same_as_the_bin_is_mounted_once_not_flattened_again() {
        let bundle = Task::group("acme's own commands", [("acme", task())]);
        let mounts = vec![("acme", bundle)];
        let result = flatten("acme", mounts);
        assert!(result.is_ok(), "expected no refusal: {result:?}");
        if let Ok(top_level) = result {
            assert_eq!(names_of(&top_level.mounts), ["acme"]);
        }
    }

    /// A bundle mounted under a key equal to the bin name, but nested one
    /// level down inside another bundle, is untouched: flatten only ever
    /// looks at the top level.
    #[test]
    fn a_nested_bundle_under_a_key_equal_to_the_bin_name_is_untouched() {
        let inner = Task::group("nested, keyed the same as the bin", [("lint", task())]);
        let outer = Task::group("an ordinary bundle", [("acme", inner)]);
        let mounts = vec![("outer", outer)];
        let result = flatten("acme", mounts);
        assert!(result.is_ok(), "expected no refusal: {result:?}");
        if let Ok(top_level) = result {
            assert_eq!(names_of(&top_level.mounts), ["outer"]);
        }
    }

    /// A mount key literally named `help` is refused, naming clap.
    #[test]
    fn a_mount_key_named_help_is_refused() {
        let mounts = vec![("help", task())];
        let result = flatten("acme", mounts);
        assert!(result.is_err(), "expected a refusal: {result:?}");
        if let Err(failure) = result {
            let message = failure.to_string();
            assert!(message.contains("`help`"), "message was: {message}");
            assert!(message.contains("clap"), "message was: {message}");
        }
    }

    /// The same `help` check, exercised after a real flatten has happened
    /// alongside it: `ensure_no_top_level_help` scans the *final* top
    /// level, so a `help` mount sitting beside a flattened bundle is
    /// refused exactly the same way. (A `help`-named child produced *by*
    /// flattening cannot be constructed at all — `Task::group` panics on
    /// one before this function ever sees it — so this is the closest
    /// exercise of that path this suite can build through the public API.)
    #[test]
    fn a_help_mount_alongside_a_flattened_bundle_is_refused() {
        let bundle = Task::group("acme's own commands", [("lint", task())]);
        let mounts = vec![("acme", bundle), ("help", task())];
        let result = flatten("acme", mounts);
        assert!(result.is_err(), "expected a refusal: {result:?}");
        if let Err(failure) = result {
            let message = failure.to_string();
            assert!(message.contains("`help`"), "message was: {message}");
            assert!(message.contains("clap"), "message was: {message}");
        }
    }
}
