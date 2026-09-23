//! Editing a manifest in place with `toml_edit`, so a scaffolding task can
//! append to a manifest a human wrote without disturbing its comments or
//! formatting.

use std::path::{Path, PathBuf};

use rituals::{Failure, Name};
use toml_edit::{Array, DocumentMut, InlineTable, Item, RawString, Value};

/// A manifest a scaffolding task is about to edit.
///
/// Holds the bytes it had when it was read so that a failed run can put
/// exactly those bytes back — rather than relying on `toml_edit`
/// round-tripping an untouched document, which is a claim this crate would
/// then depend on without checking.
#[derive(Debug)]
pub struct Manifest {
    path: PathBuf,
    original: String,
    document: DocumentMut,
}

impl Manifest {
    /// Reads and parses the manifest at `path`, keeping its original bytes
    /// for [`Manifest::restore_if_changed`].
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming `path` if it cannot be read or does not
    /// parse as TOML.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::manifest::Manifest;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-manifest-read-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(&manifest_path, "[package]\nname = \"demo\"\n")?;
    /// let manifest = Manifest::read(&manifest_path)?;
    /// assert_eq!(manifest.path(), manifest_path);
    ///
    /// let missing = directory.join("missing/Cargo.toml");
    /// assert!(Manifest::read(&missing).is_err());
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn read(path: &Path) -> Result<Self, Failure> {
        let original = std::fs::read_to_string(path).map_err(|error| {
            Failure::new(format!("reading {} failed", path.display())).caused_by(error)
        })?;
        let document = original.parse::<DocumentMut>().map_err(|error| {
            Failure::new(format!("parsing {} failed", path.display())).caused_by(error)
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            original,
            document,
        })
    }

    /// The path this manifest was read from and is written back to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes the current document back to [`Manifest::path`].
    ///
    /// This writes the whole file from the in-memory document, with no
    /// check that the file on disk still matches what [`Manifest::read`]
    /// saw: two scaffolding tasks running at once in the same checkout, or
    /// a hand edit landing between the read and this write, can be
    /// overwritten by it. This is deliberate: it carries the same
    /// property `cargo add` itself has — one person runs this by hand, in
    /// one checkout, and every write it makes is visible in `git diff`
    /// before it is committed. See [`Manifest::restore_if_changed`] for the
    /// same threat model applied to a failed run's undo.
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming the path if writing fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::manifest::Manifest;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-manifest-write-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(&manifest_path, "[workspace]\nmembers = [\n    \"ritual\",\n]\n")?;
    /// let mut manifest = Manifest::read(&manifest_path)?;
    /// manifest.append_workspace_member("tasks/lint")?;
    ///
    /// manifest.write()?;
    ///
    /// let on_disk = std::fs::read_to_string(&manifest_path)?;
    /// assert!(on_disk.contains("tasks/lint"));
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn write(&self) -> Result<(), Failure> {
        std::fs::write(&self.path, self.document.to_string()).map_err(|error| {
            Failure::new(format!("writing {} failed", self.path.display())).caused_by(error)
        })
    }

    /// Writes `original` back to [`Manifest::path`], but only if the file on
    /// disk no longer matches it — so a write that failed before it opened
    /// the file (the ordinary case: nothing was touched) is not reported as
    /// an undo failure for a file that was never written to.
    ///
    /// The same threat model documented on [`Manifest::write`] applies to
    /// this restore: it overwrites whatever is on disk with the bytes
    /// `Manifest::read` captured at the start of the run, with no check
    /// that those bytes are still what a concurrent scaffolding task or a
    /// hand edit landing during this run would want kept. Deliberate, for
    /// the same reason as `write`.
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming the path if reading it back or writing
    /// the original bytes fails.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::manifest::Manifest;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-manifest-restore-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// let original = "[package]\nname = \"demo\"\n";
    /// # std::fs::write(&manifest_path, original)?;
    /// let manifest = Manifest::read(&manifest_path)?;
    ///
    /// // A later step in the same run failed partway through, after
    /// // writing something else to the file.
    /// std::fs::write(&manifest_path, "[package]\nname = \"half-written\"\n")?;
    ///
    /// manifest.restore_if_changed()?;
    /// assert_eq!(std::fs::read_to_string(&manifest_path)?, original);
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn restore_if_changed(&self) -> Result<(), Failure> {
        let current = std::fs::read_to_string(&self.path).map_err(|error| {
            Failure::new(format!("reading {} failed", self.path.display())).caused_by(error)
        })?;
        if current == self.original {
            return Ok(());
        }
        std::fs::write(&self.path, &self.original).map_err(|error| {
            Failure::new(format!("writing {} failed", self.path.display())).caused_by(error)
        })
    }

    /// Reports whether this manifest's `[workspace.dependencies]` table
    /// declares `crate_name`.
    #[must_use]
    pub fn declares_workspace_dependency(&self, crate_name: &str) -> bool {
        self.document
            .get("workspace")
            .and_then(Item::as_table)
            .and_then(|workspace| workspace.get("dependencies"))
            .and_then(Item::as_table)
            .is_some_and(|dependencies| dependencies.contains_key(crate_name))
    }

    /// Reports whether this manifest has a `[workspace] members` array to
    /// append to — call this before [`Manifest::append_workspace_member`] to
    /// refuse before anything is written, rather than learning it from that
    /// method's own refusal after other files have already changed.
    #[must_use]
    pub fn has_workspace_members_list(&self) -> bool {
        self.document
            .get("workspace")
            .and_then(Item::as_table)
            .and_then(|workspace| workspace.get("members"))
            .and_then(Item::as_array)
            .is_some()
    }

    /// Appends `member` to this manifest's `[workspace] members` array —
    /// copying only the positioning whitespace of the array's current last
    /// entry, which is what makes this a one-line diff rather than a
    /// reflow.
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming [`Manifest::path`] when there is no
    /// `[workspace] members` array — call
    /// [`Manifest::has_workspace_members_list`] first to refuse before
    /// anything is written.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::manifest::Manifest;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-manifest-append-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(&manifest_path, "[workspace]\nmembers = [\n    \"ritual\",\n]\n")?;
    /// let mut manifest = Manifest::read(&manifest_path)?;
    ///
    /// assert!(manifest.has_workspace_members_list());
    /// manifest.append_workspace_member("tasks/lint")?;
    ///
    /// let without_members_list = directory.join("no-members.toml");
    /// # std::fs::write(&without_members_list, "[workspace]\n")?;
    /// let mut bare = Manifest::read(&without_members_list)?;
    /// assert!(bare.append_workspace_member("tasks/lint").is_err());
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn append_workspace_member(&mut self, member: &str) -> Result<(), Failure> {
        let members = self
            .document
            .get_mut("workspace")
            .and_then(Item::as_table_mut)
            .and_then(|workspace| workspace.get_mut("members"))
            .and_then(Item::as_array_mut)
            .ok_or_else(|| no_members_list(&self.path))?;

        push_matching_style(members, member);
        Ok(())
    }

    /// Adds `name = { path = "<dependency_path>" }` to this manifest's
    /// `[dependencies]` table, always a one-line inline table, and appends
    /// `"name"` to `[package.metadata.ritual] tasks`.
    ///
    /// The two writes happen together because they are one operation: a
    /// dependency with no matching `tasks` entry, or a `tasks` entry with
    /// no matching dependency, is a state `add` itself refuses to leave a
    /// project in. This method holds that itself rather than leaning on
    /// its one caller: both destinations are checked before either is
    /// written, so nothing changes in this document unless both are
    /// present.
    ///
    /// # Errors
    ///
    /// Returns a [`Failure`] naming [`Manifest::path`] when `[dependencies]`
    /// or `[package.metadata.ritual] tasks` is missing or the wrong shape.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals::Name;
    /// use rituals_compose::manifest::Manifest;
    ///
    /// # let directory = std::env::temp_dir()
    /// #     .join(format!("rituals-compose-doctest-manifest-import-{}", std::process::id()));
    /// # std::fs::create_dir_all(&directory)?;
    /// let manifest_path = directory.join("Cargo.toml");
    /// # std::fs::write(
    /// #     &manifest_path,
    /// #     "[dependencies]\n\n[package.metadata.ritual]\ntasks = []\n",
    /// # )?;
    /// let mut manifest = Manifest::read(&manifest_path)?;
    /// let name = Name::new("lint")?;
    ///
    /// manifest.import_task(&name, "../tasks/lint")?;
    /// manifest.write()?;
    ///
    /// let on_disk = std::fs::read_to_string(&manifest_path)?;
    /// assert!(on_disk.contains("lint = { path = \"../tasks/lint\" }"));
    /// assert!(on_disk.contains("\"lint\""));
    ///
    /// let bare_path = directory.join("bare.toml");
    /// # std::fs::write(&bare_path, "[package]\nname = \"demo\"\n")?;
    /// let mut bare = Manifest::read(&bare_path)?;
    /// assert!(bare.import_task(&name, "../tasks/lint").is_err());
    /// # std::fs::remove_dir_all(&directory)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn import_task(&mut self, name: &Name, dependency_path: &str) -> Result<(), Failure> {
        // Both destinations are resolved, read-only, before either is
        // written — the refusal below must be reachable with the document
        // still exactly as `Manifest::read` left it, not after the
        // dependency insert has already happened.
        self.document
            .get("dependencies")
            .and_then(Item::as_table)
            .ok_or_else(|| {
                Failure::new(format!(
                    "{} has no [dependencies] table to add `{name}` to",
                    self.path.display()
                ))
            })?;
        self.document
            .get("package")
            .and_then(Item::as_table)
            .and_then(|package| package.get("metadata"))
            .and_then(Item::as_table)
            .and_then(|metadata| metadata.get("ritual"))
            .and_then(Item::as_table)
            .and_then(|ritual| ritual.get("tasks"))
            .and_then(Item::as_array)
            .ok_or_else(|| {
                Failure::new(format!(
                    "{} has no [package.metadata.ritual] tasks list to add `{name}` to",
                    self.path.display()
                ))
            })?;

        // Both destinations exist, so both re-fetches below are infallible:
        // nothing between the checks above and here can have changed the
        // document's shape.
        let dependencies = self
            .document
            .get_mut("dependencies")
            .and_then(Item::as_table_mut)
            .unwrap_or_else(|| unreachable!("[dependencies] checked present above"));

        let mut inline = InlineTable::new();
        inline.insert("path", Value::from(dependency_path));
        dependencies.insert(name.as_str(), Item::Value(Value::from(inline)));

        let tasks = self
            .document
            .get_mut("package")
            .and_then(Item::as_table_mut)
            .and_then(|package| package.get_mut("metadata"))
            .and_then(Item::as_table_mut)
            .and_then(|metadata| metadata.get_mut("ritual"))
            .and_then(Item::as_table_mut)
            .and_then(|ritual| ritual.get_mut("tasks"))
            .and_then(Item::as_array_mut)
            .unwrap_or_else(|| {
                unreachable!("[package.metadata.ritual] tasks checked present above")
            });
        push_matching_style(tasks, name.as_str());

        Ok(())
    }
}

fn no_members_list(manifest_path: &Path) -> Failure {
    Failure::new(format!(
        "{} has no [workspace] members list to append to; add `members = []`",
        manifest_path.display()
    ))
}

/// Appends `value` to `array`, copying only the *positioning* whitespace of
/// its current last entry — the newline and indent that put that entry on
/// its own line, or a single space when the array is on one line — never the
/// text a human wrote.
///
/// A comment beside the last entry is carried forward rather than copied,
/// from whichever of two places `toml_edit` stores it, depending on whether
/// a trailing comma already exists — see [`carry_from_trailing`] and
/// [`carry_from_suffix`] for the two rules and why they differ. Either way,
/// only an actual comment ever moves: text with no `#` in it is incidental
/// whitespace, not something a human wrote to say anything, and each helper
/// decides on its own what happens to it instead of guessing it is a
/// comment.
///
/// An empty array has no element to copy positioning from, and inventing an
/// indent would be a guess, so the new entry follows `[` directly.
fn push_matching_style(array: &mut Array, value: &str) {
    let mut new_value: Value = value.into();

    let Some(last_index) = array.len().checked_sub(1) else {
        array.push_formatted(new_value);
        return;
    };

    let last_prefix = raw_text(
        array
            .get(last_index)
            .and_then(|value| value.decor().prefix()),
    );
    // The positioning whitespace: from the last element's prefix's own last
    // newline onward, or one space when that prefix never broke a line.
    let positioning_prefix = last_prefix.rfind('\n').map_or_else(
        || " ".to_string(),
        |position| last_prefix[position..].to_string(),
    );

    let (comment_head, suffix_tail) = if array.trailing_comma() {
        carry_from_trailing(array)
    } else {
        carry_from_suffix(array, last_index)
    };

    new_value
        .decor_mut()
        .set_prefix(format!("{comment_head}{positioning_prefix}"));
    new_value.decor_mut().set_suffix(suffix_tail);

    array.push_formatted(new_value);
}

/// The trailing-comma half of [`push_matching_style`]'s comment carry: the
/// comment, if any, lives in [`Array::trailing`], a property of the array
/// itself, not of any element — the existing trailing comma is about to
/// become an ordinary mid-array separator the moment a new entry follows
/// it, so the last entry's own suffix already renders in the right place
/// and is never touched here.
///
/// Only the text up to and including the comment's own last newline moves,
/// onto the new entry's prefix; what follows (ordinarily just that
/// newline, closing out the line the array's `]` sits on) stays behind as
/// the array's new `trailing`. When `trailing` holds no comment at all —
/// bare whitespace, most often a single space — none of it is a human's
/// comment to relocate: it already renders after whichever element ends up
/// last, comma or not, so it is left exactly as it is and nothing is
/// carried onto the new entry.
fn carry_from_trailing(array: &mut Array) -> (String, String) {
    let trailing_text = raw_text(Some(array.trailing()));
    if !trailing_text.contains('#') {
        return (String::new(), String::new());
    }
    let (head, tail) = split_at_last_newline(&trailing_text);
    array.set_trailing(tail);
    (head, String::new())
}

/// The no-trailing-comma half of [`push_matching_style`]'s comment carry:
/// the comment, if any, lives in the last entry's own suffix decor, sitting
/// directly before `]`. `toml_edit` writes the separating comma immediately
/// after that suffix, so leaving a comment in place would put the comma
/// inside it and the document would no longer parse — the comment moves
/// onto the new entry instead, ahead of its positioning whitespace, so a
/// single-element `x # comment` becomes `x, # comment` rather than gaining
/// a comma the comment has swallowed. Only the text up to and including the
/// comment's own last newline moves; what follows becomes the new entry's
/// own suffix.
///
/// When the suffix holds no comment, it is not a comment to move — but,
/// unlike `trailing`, there is nowhere for bare whitespace to keep
/// rendering in the right place once the last entry stops being last, so it
/// moves wholesale onto the new entry's own suffix instead of being
/// discarded: `[ "ritual" ]` keeps its space before `]` as
/// `[ "ritual", "tasks/lint" ]`, not `[ "ritual", "tasks/lint"]`.
fn carry_from_suffix(array: &mut Array, last_index: usize) -> (String, String) {
    let last_suffix = raw_text(
        array
            .get(last_index)
            .and_then(|value| value.decor().suffix()),
    );
    if last_suffix.is_empty() {
        return (String::new(), String::new());
    }

    let (head, tail) = if last_suffix.contains('#') {
        split_at_last_newline(&last_suffix)
    } else {
        (String::new(), last_suffix)
    };

    if let Some(last) = array.get_mut(last_index) {
        last.decor_mut().set_suffix("");
    }
    (head, tail)
}

/// Splits `text` at its own last newline, that newline included in the
/// second half — the shared rule for carrying a comment forward in
/// [`carry_from_trailing`] and [`carry_from_suffix`]. When `text` never
/// breaks a line, the whole of it is the first half and the second is
/// empty.
fn split_at_last_newline(text: &str) -> (String, String) {
    text.rfind('\n').map_or_else(
        || (text.to_string(), String::new()),
        |position| (text[..position].to_string(), text[position..].to_string()),
    )
}

/// Reads a [`RawString`]'s text, or `""` when it is absent — the decor
/// accessors return `None` for a decor that was never set, which is the
/// ordinary case for every entry that carries no comment.
fn raw_text(raw: Option<&RawString>) -> String {
    raw.and_then(RawString::as_str).unwrap_or("").to_string()
}

/// Reports whether the manifest at `path` parses and declares
/// `[package.metadata.ritual] task = true`.
///
/// Used to tell a leftover task crate `add` can finish importing from an
/// unrelated collision. Any failure to read or parse answers `false`: an
/// unreadable directory is exactly the unrelated-collision case. Twin of
/// [`declares_a_workspace`], which shares this contract: both parse
/// read-only with [`toml_edit::ImDocument::parse`] and answer one
/// structural question.
///
/// # Examples
///
/// ```
/// use rituals_compose::manifest::declares_a_task_crate;
///
/// # let directory = std::env::temp_dir()
/// #     .join(format!("rituals-compose-doctest-declares-a-task-crate-{}", std::process::id()));
/// # std::fs::create_dir_all(&directory)?;
/// let task_manifest = directory.join("Cargo.toml");
/// # std::fs::write(
/// #     &task_manifest,
/// #     "[package]\nname = \"lint\"\n\n[package.metadata.ritual]\ntask = true\n",
/// # )?;
/// assert!(declares_a_task_crate(&task_manifest));
///
/// let missing = directory.join("missing.toml");
/// assert!(!declares_a_task_crate(&missing));
/// # std::fs::remove_dir_all(&directory)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn declares_a_task_crate(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(document) = toml_edit::ImDocument::parse(text) else {
        return false;
    };
    document
        .get("package")
        .and_then(Item::as_table)
        .and_then(|package| package.get("metadata"))
        .and_then(Item::as_table)
        .and_then(|metadata| metadata.get("ritual"))
        .and_then(Item::as_table)
        .and_then(|ritual| ritual.get("task"))
        .and_then(Item::as_value)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Reports whether the manifest at `path` parses and declares a top-level
/// `workspace` key.
///
/// That is a real, declared workspace root, as opposed to an ordinary
/// package that Cargo treats as its own implicit one-package workspace. Any
/// failure to read or parse answers `false`. Twin of
/// [`declares_a_task_crate`], which shares this contract: both parse
/// read-only with [`toml_edit::ImDocument::parse`] and answer one
/// structural question.
///
/// # Examples
///
/// ```
/// use rituals_compose::manifest::declares_a_workspace;
///
/// # let directory = std::env::temp_dir()
/// #     .join(format!("rituals-compose-doctest-declares-a-workspace-{}", std::process::id()));
/// # std::fs::create_dir_all(&directory)?;
/// let workspace_manifest = directory.join("Cargo.toml");
/// # std::fs::write(&workspace_manifest, "[workspace]\nmembers = []\n")?;
/// assert!(declares_a_workspace(&workspace_manifest));
///
/// let package_manifest = directory.join("package.toml");
/// # std::fs::write(&package_manifest, "[package]\nname = \"demo\"\n")?;
/// assert!(!declares_a_workspace(&package_manifest));
/// # std::fs::remove_dir_all(&directory)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn declares_a_workspace(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(document) = toml_edit::ImDocument::parse(text) else {
        return false;
    };
    document.get("workspace").is_some()
}

/// Computes the forward-slash `path = "…"` value a dependency at
/// `crate_dir` wants in the manifest at `manifest_path`.
///
/// Both arguments are absolute, and the result is the same on every
/// platform because its separator is always `/`, never
/// [`std::path::MAIN_SEPARATOR`].
///
/// # Examples
///
/// ```
/// use rituals_compose::manifest::dependency_path;
///
/// // Any absolute directory will do; the temporary directory is one on
/// // every platform, where a literal `/workspace` is not absolute on Windows.
/// let workspace = std::env::temp_dir().join("workspace");
/// let manifest_path = workspace.join("ritual/Cargo.toml");
/// let crate_dir = workspace.join("tasks/lint");
///
/// assert_eq!(dependency_path(&manifest_path, &crate_dir), "../tasks/lint");
/// ```
///
/// # Panics
///
/// Panics if `manifest_path` or `crate_dir` is not absolute, since a
/// relative path between two relative paths depends on a working directory
/// neither carries. Panics too if `manifest_path` has no parent directory —
/// a filesystem root such as `/` — since it then names no manifest file at
/// all.
#[must_use]
pub fn dependency_path(manifest_path: &Path, crate_dir: &Path) -> String {
    assert!(
        manifest_path.is_absolute(),
        "manifest_path must be absolute, got {}",
        manifest_path.display()
    );
    assert!(
        crate_dir.is_absolute(),
        "crate_dir must be absolute, got {}",
        crate_dir.display()
    );

    #[expect(
        clippy::panic,
        reason = "a documented precondition, the same as the two assertions above"
    )]
    let Some(manifest_directory) = manifest_path.parent() else {
        panic!(
            "manifest_path must name a file, not a filesystem root, got {}",
            manifest_path.display()
        );
    };

    let from_components: Vec<_> = manifest_directory.components().collect();
    let to_components: Vec<_> = crate_dir.components().collect();

    let shared = from_components
        .iter()
        .zip(to_components.iter())
        .take_while(|(from, to)| from == to)
        .count();

    let ascents = from_components.len() - shared;
    let mut segments: Vec<String> = std::iter::repeat_n("..".to_string(), ascents).collect();
    segments.extend(
        to_components[shared..]
            .iter()
            .map(|component| component.as_os_str().to_string_lossy().into_owned()),
    );

    segments.join("/")
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::{Path, PathBuf};

    use rituals::Name;

    use super::{
        Manifest, declares_a_task_crate, declares_a_workspace, dependency_path, push_matching_style,
    };
    use crate::test_support::{ScratchDir, TestOutcome};

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[test]
    fn manifest_is_send_and_sync() {
        assert_send::<Manifest>();
        assert_sync::<Manifest>();
    }

    /// Parses `source` as `members = <array>`, appends `"tasks/lint"` with
    /// [`push_matching_style`], and asserts both that the rendered text is
    /// exactly `expected` and that re-parsing it yields the member sequence
    /// `expected_members` — the re-parse is the oracle that catches a comma
    /// landing inside a comment, which renders as plausible-looking text but
    /// does not parse back to the same members.
    fn assert_append_renders(
        source: &str,
        expected: &str,
        expected_members: &[&str],
    ) -> TestOutcome {
        let full_source = format!("members = {source}\n");
        let mut document: toml_edit::DocumentMut = full_source.parse()?;

        let array = document
            .get_mut("members")
            .and_then(toml_edit::Item::as_array_mut)
            .expect("each fixture's members key is an array");
        push_matching_style(array, "tasks/lint");

        let rendered = document.to_string();
        let expected_full = format!("members = {expected}\n");
        assert_eq!(rendered, expected_full, "rendered text did not match");

        let reparsed: toml_edit::DocumentMut = rendered.parse()?;
        let members: Vec<String> = reparsed
            .get("members")
            .and_then(toml_edit::Item::as_array)
            .map(|array| {
                array
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(
            members, expected_members,
            "re-parsed member sequence did not match"
        );
        Ok(())
    }

    /// Writes `content` to a scratch manifest and reads it back through
    /// [`Manifest::read`] — the setup every method test below shares.
    fn manifest(tag: &str, content: &str) -> Result<(ScratchDir, Manifest), Box<dyn Error>> {
        let scratch = ScratchDir::new(tag)?;
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(&path, content)?;
        let manifest = Manifest::read(&path)?;
        Ok((scratch, manifest))
    }

    #[test]
    fn appending_a_member_preserves_multi_line_style_and_comments() -> TestOutcome {
        let (_scratch, mut manifest) = manifest(
            "append-preserves-style",
            "[workspace]\n# a comment\nmembers = [\n    \"ritual\",\n]\nresolver = \"3\"\n",
        )?;
        let result = manifest.append_workspace_member("tasks/lint");
        assert!(result.is_ok(), "expected the append to succeed: {result:?}");
        let rendered = manifest.document.to_string();
        assert_eq!(
            rendered,
            "[workspace]\n# a comment\nmembers = [\n    \"ritual\",\n    \"tasks/lint\",\n]\nresolver = \"3\"\n"
        );
        Ok(())
    }

    #[test]
    fn appending_to_a_missing_members_list_is_refused() -> TestOutcome {
        let (_scratch, mut manifest) =
            manifest("append-missing-list", "[workspace]\nresolver = \"3\"\n")?;
        assert!(!manifest.has_workspace_members_list());
        let result = manifest.append_workspace_member("tasks/lint");
        assert!(result.is_err(), "expected the append to be refused");
        if let Err(error) = result {
            assert!(error.to_string().contains("Cargo.toml"));
            assert!(error.to_string().contains("members = []"));
        }
        Ok(())
    }

    #[test]
    fn push_matching_style_on_an_empty_inline_array() -> TestOutcome {
        assert_append_renders("[]", "[\"tasks/lint\"]", &["tasks/lint"])
    }

    #[test]
    fn push_matching_style_on_a_single_element_inline_array() -> TestOutcome {
        // A single-element array's last entry has no earlier element to
        // copy a separating space from, only a positioning prefix derived
        // from its own (single-line) prefix — the case that guards against
        // a missing space after the comma (`["ritual","tasks/lint"]`).
        assert_append_renders(
            "[\"ritual\"]",
            "[\"ritual\", \"tasks/lint\"]",
            &["ritual", "tasks/lint"],
        )
    }

    #[test]
    fn push_matching_style_on_a_two_element_inline_array() -> TestOutcome {
        assert_append_renders(
            "[\"ritual\", \"tasks/foo\"]",
            "[\"ritual\", \"tasks/foo\", \"tasks/lint\"]",
            &["ritual", "tasks/foo", "tasks/lint"],
        )
    }

    #[test]
    fn push_matching_style_on_a_single_element_inline_array_with_a_trailing_comma() -> TestOutcome {
        assert_append_renders(
            "[\"ritual\",]",
            "[\"ritual\", \"tasks/lint\",]",
            &["ritual", "tasks/lint"],
        )
    }

    #[test]
    fn push_matching_style_on_a_trailing_comma_with_bare_whitespace_after_it() -> TestOutcome {
        // `Array::trailing()` here is a single incidental space, not a
        // comment — nothing for `#` to mark. Carrying it as though it were
        // a comment doubles the space ahead of the new entry and drops the
        // space that belonged before `]`; the fix leaves `trailing()`
        // exactly as it is, since it already renders after whichever
        // element ends up last, and carries nothing.
        assert_append_renders(
            "[ \"ritual\", ]",
            "[ \"ritual\", \"tasks/lint\", ]",
            &["ritual", "tasks/lint"],
        )
    }

    #[test]
    fn push_matching_style_on_bare_whitespace_with_no_trailing_comma() -> TestOutcome {
        // The no-comma sibling of the case above: the last entry's own
        // suffix is a single incidental space with no comment in it. That
        // space was positioning `"ritual"` before `]`, which is about to
        // stop being last, so it moves wholesale onto the new entry's own
        // suffix rather than being discarded or mistaken for a comment.
        assert_append_renders(
            "[ \"ritual\" ]",
            "[ \"ritual\", \"tasks/lint\" ]",
            &["ritual", "tasks/lint"],
        )
    }

    #[test]
    fn push_matching_style_on_an_empty_multi_line_array() -> TestOutcome {
        assert_append_renders("[\n]", "[\"tasks/lint\"\n]", &["tasks/lint"])
    }

    #[test]
    fn push_matching_style_on_a_single_element_multi_line_array() -> TestOutcome {
        assert_append_renders(
            "[\n    \"ritual\",\n]",
            "[\n    \"ritual\",\n    \"tasks/lint\",\n]",
            &["ritual", "tasks/lint"],
        )
    }

    #[test]
    fn push_matching_style_on_a_two_element_multi_line_array() -> TestOutcome {
        assert_append_renders(
            "[\n    \"ritual\",\n    \"tasks/foo\",\n]",
            "[\n    \"ritual\",\n    \"tasks/foo\",\n    \"tasks/lint\",\n]",
            &["ritual", "tasks/foo", "tasks/lint"],
        )
    }

    #[test]
    fn push_matching_style_on_a_multi_line_array_with_no_trailing_comma() -> TestOutcome {
        assert_append_renders(
            "[\n    \"ritual\"\n]",
            "[\n    \"ritual\",\n    \"tasks/lint\"\n]",
            &["ritual", "tasks/lint"],
        )
    }

    #[test]
    fn push_matching_style_carries_a_comment_that_already_precedes_the_last_element() -> TestOutcome
    {
        // The comment is already in the second element's own prefix (it
        // follows the first element's comma), so appending after it must
        // leave the comment exactly where it was — once, on `"ritual"`'s
        // line — not duplicate it.
        assert_append_renders(
            "[\n    \"ritual\", # the cli\n    \"tasks/foo\",\n]",
            "[\n    \"ritual\", # the cli\n    \"tasks/foo\",\n    \"tasks/lint\",\n]",
            &["ritual", "tasks/foo", "tasks/lint"],
        )
    }

    #[test]
    fn push_matching_style_carries_a_comment_on_the_last_elements_own_line() -> TestOutcome {
        // The comment is on the last (and only) element's own line, with no
        // comma yet — it must move ahead of the new element's positioning
        // whitespace, and the comma must land before it, not inside it.
        assert_append_renders(
            "[\n    \"ritual\" # the cli\n]",
            "[\n    \"ritual\", # the cli\n    \"tasks/lint\"\n]",
            &["ritual", "tasks/lint"],
        )
    }

    #[test]
    fn push_matching_style_leaves_a_comment_before_the_last_element_untouched() -> TestOutcome {
        assert_append_renders(
            "[\n    # the cli\n    \"ritual\",\n]",
            "[\n    # the cli\n    \"ritual\",\n    \"tasks/lint\",\n]",
            &["ritual", "tasks/lint"],
        )
    }

    #[test]
    fn push_matching_style_reproduces_a_tab_indent() -> TestOutcome {
        assert_append_renders(
            "[\n\t\"ritual\",\n]",
            "[\n\t\"ritual\",\n\t\"tasks/lint\",\n]",
            &["ritual", "tasks/lint"],
        )
    }

    #[test]
    fn push_matching_style_carries_a_comment_after_an_existing_trailing_comma() -> TestOutcome {
        // toml_edit stores a comment that follows an already-present
        // trailing comma in the array's own `trailing()`, not in the last
        // element's suffix decor — a property of the array, not of any
        // element. Appending must read it by name and carry it forward the
        // same way a suffix comment is carried, or it silently renders
        // after whichever element ends up last instead of the one it was
        // written about, unmoved in position but describing the wrong
        // member once a new one follows it.
        assert_append_renders(
            "[\n    \"ritual\", # the cli\n]",
            "[\n    \"ritual\", # the cli\n    \"tasks/lint\",\n]",
            &["ritual", "tasks/lint"],
        )
    }

    #[test]
    fn push_matching_style_carries_both_a_suffix_comment_and_a_trailing_comment() -> TestOutcome {
        // An unusual but valid shape: the comma sits on its own line, so
        // "ritual" carries a comment in its own suffix decor (before the
        // comma) *and* the array's trailing() carries a second comment
        // (after the comma). Once a trailing comma already exists, that
        // comma becomes an ordinary mid-array separator the moment a new
        // element follows it — "ritual"'s suffix already renders in the
        // right place and needs no carrying, only trailing() does. Both
        // comments must survive, neither lost nor merged onto one line
        // where a reader could no longer tell them apart.
        assert_append_renders(
            "[\n    \"ritual\" # suffix comment\n    , # trailing comment\n]",
            "[\n    \"ritual\" # suffix comment\n    , # trailing comment\n    \"tasks/lint\",\n]",
            &["ritual", "tasks/lint"],
        )
    }

    #[test]
    fn declares_workspace_dependency_reads_the_table() -> TestOutcome {
        let (_scratch, with) = manifest(
            "declares-workspace-dependency-with",
            "[workspace.dependencies]\nrituals = { path = \"crates/rituals\" }\n",
        )?;
        assert!(with.declares_workspace_dependency("rituals"));

        let (_scratch, without) = manifest(
            "declares-workspace-dependency-without",
            "[workspace]\nresolver = \"3\"\n",
        )?;
        assert!(!without.declares_workspace_dependency("rituals"));
        Ok(())
    }

    #[test]
    fn importing_a_task_writes_a_one_line_inline_table() -> TestOutcome {
        let (_scratch, mut manifest) = manifest(
            "import-task",
            "[dependencies]\nrituals.workspace = true\n\n\
             [package.metadata.ritual]\ntasks = [\"new\"]\n",
        )?;
        let name = Name::new("lint")?;
        let result = manifest.import_task(&name, "../tasks/lint");
        assert!(result.is_ok(), "expected the edit to succeed: {result:?}");
        let rendered = manifest.document.to_string();
        assert!(rendered.contains("lint = { path = \"../tasks/lint\" }"));
        assert!(rendered.contains("tasks = [\"new\", \"lint\"]"));
        Ok(())
    }

    /// The positive control for the pair below: a manifest with
    /// `[dependencies]` but no `[package.metadata.ritual] tasks` array
    /// reaches the dependency write before the refusal, so unless
    /// `import_task` resolves both destinations before mutating either,
    /// the document changes even though the call returns `Err`.
    #[test]
    fn importing_a_task_without_a_tasks_list_leaves_the_document_unchanged() -> TestOutcome {
        let (_scratch, mut manifest) = manifest(
            "import-task-no-tasks-list",
            "[dependencies]\nrituals.workspace = true\n",
        )?;
        let name = Name::new("lint")?;
        let before = manifest.document.to_string();

        let result = manifest.import_task(&name, "../tasks/lint");

        assert!(result.is_err(), "expected the edit to be refused");
        assert_eq!(
            manifest.document.to_string(),
            before,
            "the document must be unchanged when import_task is refused"
        );
        Ok(())
    }

    /// The mirror of the test above: a `tasks` array with no
    /// `[dependencies]` table refuses before either destination is
    /// touched, since the dependency lookup runs first and fails
    /// immediately.
    #[test]
    fn importing_a_task_without_a_dependencies_table_leaves_the_document_unchanged() -> TestOutcome
    {
        let (_scratch, mut manifest) = manifest(
            "import-task-no-dependencies-table",
            "[package.metadata.ritual]\ntasks = []\n",
        )?;
        let name = Name::new("lint")?;
        let before = manifest.document.to_string();

        let result = manifest.import_task(&name, "../tasks/lint");

        assert!(result.is_err(), "expected the edit to be refused");
        assert_eq!(
            manifest.document.to_string(),
            before,
            "the document must be unchanged when import_task is refused"
        );
        Ok(())
    }

    /// Removes `..` and `.` components lexically, the way joining a
    /// relative path onto a base directory needs before comparing it
    /// against a target — `Path` never does this on its own.
    fn normalize(path: &Path) -> PathBuf {
        let mut result = PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::ParentDir => {
                    result.pop();
                }
                std::path::Component::CurDir => {}
                other => result.push(other.as_os_str()),
            }
        }
        result
    }

    #[test]
    fn dependency_path_rejoins_to_the_target_when_nested_one_level() {
        let manifest_directory = Path::new("/workspace/ritual");
        let manifest_path = manifest_directory.join("Cargo.toml");
        let crate_dir = Path::new("/workspace/tasks/lint");
        let relative = dependency_path(&manifest_path, crate_dir);
        assert_eq!(relative, "../tasks/lint");
        assert_eq!(normalize(&manifest_directory.join(relative)), crate_dir);
    }

    #[test]
    fn dependency_path_rejoins_to_the_target_when_nested_two_levels() {
        let manifest_directory = Path::new("/workspace/crates/rituals-cli");
        let manifest_path = manifest_directory.join("Cargo.toml");
        let crate_dir = Path::new("/workspace/tasks/new");
        let relative = dependency_path(&manifest_path, crate_dir);
        assert_eq!(relative, "../../tasks/new");
        assert_eq!(normalize(&manifest_directory.join(relative)), crate_dir);
    }

    #[test]
    fn dependency_path_when_the_cli_crate_is_at_the_workspace_root() {
        let manifest_directory = Path::new("/workspace");
        let manifest_path = manifest_directory.join("Cargo.toml");
        let crate_dir = Path::new("/workspace/tasks/lint");
        let relative = dependency_path(&manifest_path, crate_dir);
        assert_eq!(relative, "tasks/lint");
        assert_eq!(normalize(&manifest_directory.join(relative)), crate_dir);
    }

    #[test]
    #[should_panic(expected = "manifest_path must name a file, not a filesystem root")]
    fn dependency_path_refuses_a_filesystem_root_as_the_manifest_path() {
        let temp_dir = std::env::temp_dir();
        let filesystem_root = temp_dir.ancestors().last().unwrap_or(&temp_dir);
        assert!(filesystem_root.is_absolute());
        assert_eq!(filesystem_root.parent(), None);

        let _ = dependency_path(filesystem_root, &temp_dir.join("tasks/lint"));
    }

    #[test]
    fn restore_if_changed_puts_back_the_original_bytes() -> TestOutcome {
        let scratch = ScratchDir::new("restore-changed")?;
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(&path, "[workspace]\nmembers = [\"ritual\"]\n")?;

        let manifest = Manifest::read(&path)?;
        std::fs::write(
            &path,
            "[workspace]\nmembers = [\"ritual\", \"tasks/lint\"]\n",
        )?;

        let restored = manifest.restore_if_changed();
        assert!(
            restored.is_ok(),
            "expected the restore to succeed: {restored:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&path)?,
            "[workspace]\nmembers = [\"ritual\"]\n"
        );
        Ok(())
    }

    #[test]
    fn restore_if_changed_does_not_write_when_the_file_is_unchanged() -> TestOutcome {
        let scratch = ScratchDir::new("restore-unchanged")?;
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(&path, "[workspace]\nmembers = [\"ritual\"]\n")?;

        let manifest = Manifest::read(&path)?;

        // Read-only: if `restore_if_changed` tried to write despite the file
        // already matching, this would turn that attempt into a failure
        // instead of silently succeeding either way.
        let mut permissions = std::fs::metadata(&path)?.permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions)?;

        // A process that ignores the read-only bit (root, on Unix) can open
        // the file for writing anyway, which would make the assertion below
        // pass whether or not `restore_if_changed` actually attempted a
        // write. This probe opens for write without writing anything, so it
        // tells the two cases apart without disturbing the file's contents.
        let permission_is_enforced = std::fs::OpenOptions::new().write(true).open(&path).is_err();

        let restored = if permission_is_enforced {
            manifest.restore_if_changed()
        } else {
            Ok(())
        };

        // Cleanup, on a scratch file this test alone created and is about to
        // delete — not a security boundary `set_readonly(false)`'s
        // world-writable warning is guarding here.
        let mut permissions = std::fs::metadata(&path)?.permissions();
        #[expect(
            clippy::permissions_set_readonly_false,
            reason = "restoring a scratch file's own permissions before removing it, not \
                      granting access to anything"
        )]
        {
            permissions.set_readonly(false);
        }
        std::fs::set_permissions(&path, permissions)?;

        if !permission_is_enforced {
            crate::test_support::report_skip(
                "restore_if_changed_does_not_write_when_the_file_is_unchanged \
                     could not demonstrate a blocked write because this process does not \
                     honour the read-only permission bit",
            );
            return Ok(());
        }

        assert!(
            restored.is_ok(),
            "expected no write attempt against an unchanged, read-only file: {restored:?}"
        );
        Ok(())
    }

    #[test]
    fn declares_a_task_crate_is_true_for_a_real_scaffolded_task_crate() -> TestOutcome {
        let scratch = ScratchDir::new("declares-a-task-crate-true")?;
        let path = scratch.path().join("Cargo.toml");
        let name = Name::new("lint")?;
        // Rendered by the same function `add`/`create` write, so this test
        // cannot drift from what a real scaffolded manifest looks like.
        let manifest_text = crate::task_crate::manifest(&name, &crate::source::Source::Inherited);
        std::fs::write(&path, manifest_text)?;

        assert!(declares_a_task_crate(&path));
        Ok(())
    }

    #[test]
    fn declares_a_task_crate_is_false_for_a_plain_package_manifest() -> TestOutcome {
        let scratch = ScratchDir::new("declares-a-task-crate-plain")?;
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(
            &path,
            "[package]\nname = \"unrelated\"\nversion = \"0.1.0\"\n",
        )?;

        assert!(!declares_a_task_crate(&path));
        Ok(())
    }

    #[test]
    fn declares_a_task_crate_is_false_for_a_missing_manifest() {
        let path = Path::new("/does/not/exist/Cargo.toml");
        assert!(!declares_a_task_crate(path));
    }

    #[test]
    fn declares_a_task_crate_is_false_for_a_manifest_that_does_not_parse() -> TestOutcome {
        let scratch = ScratchDir::new("declares-a-task-crate-unparseable")?;
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(&path, "this is not valid TOML {{{")?;

        assert!(!declares_a_task_crate(&path));
        Ok(())
    }

    #[test]
    fn declares_a_workspace_is_true_for_a_declared_virtual_workspace_root() -> TestOutcome {
        let scratch = ScratchDir::new("declares-a-workspace-virtual")?;
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(&path, "[workspace]\nmembers = []\nresolver = \"3\"\n")?;

        assert!(declares_a_workspace(&path));
        Ok(())
    }

    #[test]
    fn declares_a_workspace_is_false_for_a_plain_package_manifest() -> TestOutcome {
        let scratch = ScratchDir::new("declares-a-workspace-plain")?;
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(
            &path,
            "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;

        assert!(!declares_a_workspace(&path));
        Ok(())
    }

    #[test]
    fn declares_a_workspace_is_false_for_a_missing_manifest() {
        let path = Path::new("/does/not/exist/Cargo.toml");
        assert!(!declares_a_workspace(path));
    }

    #[test]
    fn declares_a_workspace_is_false_for_a_manifest_that_does_not_parse() -> TestOutcome {
        let scratch = ScratchDir::new("declares-a-workspace-unparseable")?;
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(&path, "this is not valid TOML {{{")?;

        assert!(!declares_a_workspace(&path));
        Ok(())
    }
}
