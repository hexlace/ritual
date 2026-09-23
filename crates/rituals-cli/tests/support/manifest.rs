//! Reading and editing Cargo manifests as TOML documents.
//!
//! Every manifest a story reads or edits goes through `toml_edit`, the same
//! parser `add` itself edits manifests with, rather than through substring
//! searches: a substring cannot tell a table header from the same text in a
//! comment, the `[package]` name from the `[[bin]]` name, or one array entry
//! from a longer one that contains it. Lookups return what the document
//! says; edits refuse, rather than silently doing nothing, when the thing
//! they were asked to change is not there.

use std::path::Path;

use toml_edit::{Array, DocumentMut, Item, Value};

use super::{OptionContext, Outcome, ResultContext, TestOutcome, failure, read_text, write_text};

/// Parses the manifest at `path`.
pub(crate) fn read(path: &Path) -> Outcome<DocumentMut> {
    read_text(path)?
        .parse()
        .context(&format!("parsing {} as TOML failed", path.display()))
}

/// Reads the manifest at `path`, applies `edit` to it, and writes it back.
///
/// Everything `edit` does not touch — other tables, comments, formatting —
/// is written back as it was.
pub(crate) fn edit(path: &Path, edit: impl FnOnce(&mut DocumentMut) -> TestOutcome) -> TestOutcome {
    let mut document = read(path)?;
    edit(&mut document)?;
    write_text(path, &document.to_string())
}

/// The item at `keys`, walking tables and inline tables alike, or `None` if
/// any step along the way is missing.
pub(crate) fn lookup<'document>(
    document: &'document DocumentMut,
    keys: &[&str],
) -> Option<&'document Item> {
    keys.iter()
        .try_fold(document.as_item(), |item, key| item.get(key))
}

/// The item at `keys`, for editing, or `None` if any step is missing.
fn lookup_mut<'document>(
    document: &'document mut DocumentMut,
    keys: &[&str],
) -> Option<&'document mut Item> {
    keys.iter()
        .try_fold(document.as_item_mut(), |item, key| item.get_mut(key))
}

/// The string at `keys`, if there is one.
pub(crate) fn string_at<'document>(
    document: &'document DocumentMut,
    keys: &[&str],
) -> Option<&'document str> {
    lookup(document, keys).and_then(Item::as_str)
}

/// The strings in the array at `keys`, or `None` if there is no array
/// there. A non-string entry is an error: none of the arrays this suite
/// reads may hold one.
fn strings_at(document: &DocumentMut, keys: &[&str]) -> Outcome<Option<Vec<String>>> {
    let Some(array) = lookup(document, keys).and_then(Item::as_array) else {
        return Ok(None);
    };
    array
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .context(&format!("a non-string entry in `{}`", keys.join(".")))
        })
        .collect::<Outcome<Vec<_>>>()
        .map(Some)
}

/// The keys of the table at `keys`, in the order the document lists them —
/// empty when there is no such table.
pub(crate) fn keys_of(document: &DocumentMut, keys: &[&str]) -> Vec<String> {
    lookup(document, keys)
        .and_then(Item::as_table_like)
        .map(|table| table.iter().map(|(key, _)| key.to_string()).collect())
        .unwrap_or_default()
}

/// The package the dependency `key` in `[dependencies]` resolves to: its
/// `package` field, or `key` itself when it has none — following
/// `key.workspace = true` into `workspace_manifest`'s
/// `[workspace.dependencies]`, where an inherited dependency is declared.
/// `None` when there is no such dependency.
pub(crate) fn dependency_package(
    manifest: &DocumentMut,
    workspace_manifest: &DocumentMut,
    key: &str,
) -> Option<String> {
    let dependency = lookup(manifest, &["dependencies", key])?;
    let inherited = dependency.get("workspace").and_then(Item::as_bool) == Some(true);
    let declaration = if inherited {
        lookup(workspace_manifest, &["workspace", "dependencies", key])?
    } else {
        dependency
    };
    Some(
        declaration
            .get("package")
            .and_then(Item::as_str)
            .unwrap_or(key)
            .to_string(),
    )
}

/// `[package] name`.
pub(crate) fn package_name(document: &DocumentMut) -> Outcome<String> {
    string_at(document, &["package", "name"])
        .map(str::to_string)
        .context("no `[package] name` in the manifest")
}

/// The name of every `[[bin]]` target the manifest declares, in order.
pub(crate) fn bin_names(document: &DocumentMut) -> Vec<String> {
    document
        .get("bin")
        .and_then(Item::as_array_of_tables)
        .map(|bins| {
            bins.iter()
                .filter_map(|bin| bin.get("name").and_then(Item::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Every `[[bin]]` target the manifest declares, in order, as its name and
/// its `path` — `None` for a target that leaves the path to Cargo.
pub(crate) fn bin_targets(document: &DocumentMut) -> Vec<(String, Option<String>)> {
    document
        .get("bin")
        .and_then(Item::as_array_of_tables)
        .map(|bins| {
            bins.iter()
                .filter_map(|bin| {
                    let name = bin.get("name").and_then(Item::as_str)?;
                    let path = bin.get("path").and_then(Item::as_str);
                    Some((name.to_string(), path.map(str::to_string)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Asserts the manifest declares exactly one `[[bin]]` target, named
/// `name`, at `src/main.rs` — the file the generated command line is
/// written to.
#[track_caller]
pub(crate) fn assert_sole_bin_at_main_rs(document: &DocumentMut, name: &str) {
    assert_eq!(
        bin_targets(document),
        [(name.to_string(), Some("src/main.rs".to_string()))],
        "expected one [[bin]] named `{name}` at src/main.rs; manifest was:\n{document}"
    );
}

/// The one `[[bin]]` name the manifest declares; an error unless there is
/// exactly one.
pub(crate) fn sole_bin_name(document: &DocumentMut) -> Outcome<String> {
    match bin_names(document).as_slice() {
        [only] => Ok(only.clone()),
        other => failure(format!(
            "expected exactly one [[bin]] target in the manifest, found {other:?}"
        )),
    }
}

/// Renames the manifest's one `[[bin]]` target to `new_name`, leaving
/// `[package] name` — a different field, however often it starts out
/// holding the same string — untouched.
pub(crate) fn rename_sole_bin(document: &mut DocumentMut, new_name: &str) -> TestOutcome {
    let bins = document
        .get_mut("bin")
        .and_then(Item::as_array_of_tables_mut)
        .context("no [[bin]] table in the manifest")?;
    if bins.len() != 1 {
        return failure(format!(
            "expected exactly one [[bin]] target to rename, found {}",
            bins.len()
        ));
    }
    let bin = bins
        .get_mut(0)
        .context("a one-entry [[bin]] array has an entry at 0")?;
    bin.insert("name", toml_edit::value(new_name));
    Ok(())
}

/// `[workspace] members`, or `None` if the manifest has no `[workspace]`
/// table with a member list.
pub(crate) fn workspace_members(document: &DocumentMut) -> Option<Vec<String>> {
    strings_at(document, &["workspace", "members"])
        .ok()
        .flatten()
}

/// `[package.metadata.ritual] tasks`.
pub(crate) fn tasks(document: &DocumentMut) -> Outcome<Vec<String>> {
    strings_at(document, &["package", "metadata", "ritual", "tasks"])?
        .context("no `[package.metadata.ritual] tasks` array in the manifest")
}

/// Whether the manifest declares `[package.metadata.ritual] task = true`.
pub(crate) fn declares_itself_a_task(document: &DocumentMut) -> bool {
    lookup(document, &["package", "metadata", "ritual", "task"]).and_then(Item::as_bool)
        == Some(true)
}

/// The array at `keys`, for editing.
fn array_mut<'document>(
    document: &'document mut DocumentMut,
    keys: &[&str],
) -> Outcome<&'document mut Array> {
    lookup_mut(document, keys)
        .and_then(Item::as_array_mut)
        .context(&format!("no `{}` array in the manifest", keys.join(".")))
}

/// Appends `entry` to the array at `keys`.
fn push_string(document: &mut DocumentMut, keys: &[&str], entry: &str) -> TestOutcome {
    array_mut(document, keys)?.push(entry);
    Ok(())
}

/// The index of `entry` in `array`, or an error naming the array.
fn position_of(array: &Array, entry: &str, keys: &[&str]) -> Outcome<usize> {
    array
        .iter()
        .position(|value| value.as_str() == Some(entry))
        .context(&format!("no \"{entry}\" in `{}`", keys.join(".")))
}

/// Removes `entry` from the array at `keys`; an error if it is not there.
fn remove_string(document: &mut DocumentMut, keys: &[&str], entry: &str) -> TestOutcome {
    let array = array_mut(document, keys)?;
    let index = position_of(array, entry, keys)?;
    array.remove(index);
    Ok(())
}

const TASKS: [&str; 4] = ["package", "metadata", "ritual", "tasks"];
const MEMBERS: [&str; 2] = ["workspace", "members"];

/// Appends `key` to `[package.metadata.ritual] tasks`.
pub(crate) fn push_task(document: &mut DocumentMut, key: &str) -> TestOutcome {
    push_string(document, &TASKS, key)
}

/// Removes `key` from `[package.metadata.ritual] tasks`; an error if it is
/// not listed.
pub(crate) fn remove_task(document: &mut DocumentMut, key: &str) -> TestOutcome {
    remove_string(document, &TASKS, key)
}

/// Appends `member` to `[workspace] members`.
pub(crate) fn push_member(document: &mut DocumentMut, member: &str) -> TestOutcome {
    push_string(document, &MEMBERS, member)
}

/// Replaces `old` with `new` in `[workspace] members`; an error if `old` is
/// not a member.
pub(crate) fn replace_member(document: &mut DocumentMut, old: &str, new: &str) -> TestOutcome {
    let array = array_mut(document, &MEMBERS)?;
    let index = position_of(array, old, &MEMBERS)?;
    array.replace(index, new);
    Ok(())
}

/// Removes the dependency `key` from `[dependencies]`; an error if there is
/// no such dependency.
pub(crate) fn remove_dependency(document: &mut DocumentMut, key: &str) -> TestOutcome {
    lookup_mut(document, &["dependencies"])
        .and_then(Item::as_table_like_mut)
        .context("no [dependencies] table in the manifest")?
        .remove(key)
        .map(drop)
        .context(&format!("no dependency `{key}` in [dependencies]"))
}

/// Points the dependency `key`'s `path` at `new_path`; an error if the
/// dependency is missing or is not a path dependency.
pub(crate) fn set_dependency_path(
    document: &mut DocumentMut,
    key: &str,
    new_path: &str,
) -> TestOutcome {
    let dependency = lookup_mut(document, &["dependencies", key])
        .and_then(Item::as_table_like_mut)
        .context(&format!(
            "no dependency table for `{key}` in [dependencies]"
        ))?;
    let path = dependency
        .get_mut("path")
        .context(&format!("dependency `{key}` has no `path`"))?;
    *path = Item::Value(Value::from(new_path));
    Ok(())
}
