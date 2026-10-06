//! Repointing a `[workspace]`'s lists of members.

use std::path::{Path, PathBuf};

use rituals::Failure;
use toml_edit::{Array, Item, TableLike};

use super::{Repointer, replace_string};
use crate::manifest::entry_removal::remove_matching_style;
use crate::manifest::member_globs::{expand, is_a_glob};
use crate::manifest::{PathChange, push_matching_style};
use crate::paths::{normalize, relative};

/// The two `[workspace]` lists whose entries name members, so a glob in
/// either is read as the directories it matches.
const MEMBER_LISTS: [&str; 2] = ["members", "default-members"];

impl Repointer<'_> {
    /// `members`, `default-members` and `exclude`.
    pub(super) fn member_lists(&mut self, workspace: &mut dyn TableLike) -> Result<(), Failure> {
        for list in MEMBER_LISTS {
            if let Some(entries) = workspace.get_mut(list).and_then(Item::as_array_mut) {
                self.members(entries, &format!("[workspace] {list}"))?;
            }
        }
        if let Some(entries) = workspace.get_mut("exclude").and_then(Item::as_array_mut) {
            self.excluded(entries)?;
        }
        Ok(())
    }

    /// Every entry of a list of members, which is named `place` in a report.
    ///
    /// A path is repointed like any other. A glob under the directory the
    /// moved ones come out of is expanded, since only what it matches says
    /// whether it still matches anything once they have moved.
    fn members(&mut self, entries: &mut Array, place: &str) -> Result<(), Failure> {
        let mut present: Vec<String> = entries
            .iter()
            .filter_map(|entry| entry.as_str().map(str::to_string))
            .collect();
        let mut gained: Vec<String> = Vec::new();
        for position in 0..entries.len() {
            let Some(value) = entries.get_mut(position) else {
                continue;
            };
            let Some(entry) = value.as_str().map(str::to_string) else {
                continue;
            };
            if !is_a_glob(&entry) {
                self.repoint_value(value, place)?;
                continue;
            }
            if let Some(new) = self.moved_glob(&entry, &present, place, value)? {
                present.push(new.clone());
                gained.push(new);
            }
        }
        for glob in gained {
            push_matching_style(entries, &glob);
        }
        Ok(())
    }

    /// Entries of `exclude` follow the directories they keep out of the
    /// workspace. One at or under a moved directory is repointed with it. One
    /// that holds moved directories, such as a directory that groups tasks
    /// and that a glob would otherwise match, is carried to the same place
    /// under the new directory, where those directories now sit; it is kept
    /// at the old place as well only while something is left there. A glob
    /// is left as it is: Cargo reads `exclude` as paths.
    fn excluded(&mut self, entries: &mut Array) -> Result<(), Failure> {
        const PLACE: &str = "[workspace] exclude";
        let mut present: Vec<String> = entries
            .iter()
            .filter_map(|entry| entry.as_str().map(str::to_string))
            .collect();
        let mut gained: Vec<String> = Vec::new();
        let mut dropped: Vec<usize> = Vec::new();
        for position in 0..entries.len() {
            let Some(value) = entries.get_mut(position) else {
                continue;
            };
            let Some(entry) = value.as_str().map(str::to_string) else {
                continue;
            };
            if is_a_glob(&entry) {
                continue;
            }
            let Some(holder) = self.holder_of_moved_directories(&entry) else {
                self.repoint_value(value, PLACE)?;
                continue;
            };
            let new = self.spelled_after_the_move(&holder, Path::new(&entry).is_absolute());
            let left = self.what_is_left_in(&holder)?;
            if !left.is_empty() {
                if !present.contains(&new) {
                    self.changes
                        .push(PathChange::exclude_kept(PLACE, &entry, left, &new));
                    present.push(new.clone());
                    gained.push(new);
                }
            } else if present.contains(&new) {
                self.changes.push(PathChange::dropped(PLACE, &entry, &new));
                dropped.push(position);
            } else {
                replace_string(value, &new);
                self.changes
                    .push(PathChange::repointed(PLACE, &entry, &new));
                present.push(new);
            }
        }
        for position in dropped.into_iter().rev() {
            remove_matching_style(entries, position);
        }
        for path in gained {
            push_matching_style(entries, &path);
        }
        Ok(())
    }

    /// The directory `entry` names, normalised, when it is at or under the
    /// directory the moved ones come out of and holds at least one of them
    /// below it; `None` for anything else, a moved directory itself included.
    fn holder_of_moved_directories(&self, entry: &str) -> Option<PathBuf> {
        let holder = normalize(&self.base_before.join(entry));
        let holds_one = self
            .relocation
            .moved()
            .iter()
            .any(|moved| moved != &holder && moved.starts_with(&holder));
        (holds_one && holder.starts_with(self.relocation.moved_out_of())).then_some(holder)
    }

    /// What will still be in `directory` once the moved directories have gone
    /// and the directories they emptied are removed, by name, sorted: every
    /// entry that does not move and is not a directory emptied by the moves.
    /// Empty when nothing will be, so the directory itself goes.
    fn what_is_left_in(&self, directory: &Path) -> Result<Vec<String>, Failure> {
        let mut left = Vec::new();
        for child in children_of(directory)? {
            if self.remains(&child)? {
                left.push(self.spelled_from_the_manifest(&child));
            }
        }
        left.sort();
        Ok(left)
    }

    /// Whether `path` will still be there once the moved directories have
    /// gone and the directories they emptied are removed: it does not move,
    /// and either no moved directory is below it, or something else in it
    /// remains too.
    fn remains(&self, path: &Path) -> Result<bool, Failure> {
        if self.relocation.destination(path).is_some() {
            return Ok(false);
        }
        let holds_a_moved_directory = self
            .relocation
            .moved()
            .iter()
            .any(|moved| moved != path && moved.starts_with(path));
        if !holds_a_moved_directory {
            return Ok(true);
        }
        for child in children_of(path)? {
            if self.remains(&child)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether anything at `path` stays where it is once the moved
    /// directories have gone: a file that does not move, or a directory with
    /// something in it, at any depth, that stays. A directory whose contents
    /// all move, or that holds nothing, keeps nothing where it is.
    fn stays(&self, path: &Path) -> Result<bool, Failure> {
        if self.relocation.destination(path).is_some() {
            return Ok(false);
        }
        if !is_a_directory(path)? {
            return Ok(true);
        }
        for child in children_of(path)? {
            if self.stays(&child)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `path` as the manifest's directory spells it.
    fn spelled_from_the_manifest(&self, path: &Path) -> String {
        path.strip_prefix(&self.base_before)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    /// Repoints the glob `entry`, which is `value`, when it matches a
    /// directory that moves, and returns the glob to add beside it when it
    /// has to stay.
    ///
    /// A glob that matches no directory that moves, or holds one, is left
    /// alone. When nothing else it matches stays, it is replaced in place by
    /// the same glob under the new place. A matched directory stays only if
    /// something in it does, at any depth, so a directory that grouped tasks
    /// and empties when they move does not keep the glob, which would then
    /// match nothing and read to Cargo as a literal path. When something
    /// does stay, such as a crate that is not a task, the glob is kept, since
    /// replacing it would drop that crate from the workspace, and the new
    /// place's glob is added beside it, unless `present` has it already.
    fn moved_glob(
        &mut self,
        entry: &str,
        present: &[String],
        place: &str,
        value: &mut toml_edit::Value,
    ) -> Result<Option<String>, Failure> {
        let pattern = normalize(&self.base_before.join(entry));
        if !pattern.starts_with(self.relocation.moved_out_of()) {
            return Ok(None);
        }
        let matches = expand(&self.base_before, entry)?;
        // A match moves itself, sits in a directory that moves, or holds one.
        let touches_a_move = matches.iter().any(|matched| {
            self.relocation.destination(matched).is_some()
                || self
                    .relocation
                    .moved()
                    .iter()
                    .any(|moved| moved.starts_with(matched))
        });
        if !touches_a_move {
            return Ok(None);
        }

        let mut surviving: Vec<String> = Vec::new();
        for matched in &matches {
            // Cargo skips a matched file, so only a directory can keep the
            // glob alive.
            if is_a_directory(matched)? && self.stays(matched)? {
                surviving.push(self.spelled_from_the_manifest(matched));
            }
        }
        surviving.sort();

        let new = self.spelled_after_the_move(&pattern, Path::new(entry).is_absolute());
        if surviving.is_empty() {
            replace_string(value, &new);
            self.changes.push(PathChange::repointed(place, entry, &new));
            return Ok(None);
        }
        if present.contains(&new) {
            return Ok(None);
        }
        self.changes
            .push(PathChange::glob_kept(place, entry, surviving, &new));
        Ok(Some(new))
    }

    /// The glob or path `pattern`, normalised and at or under the directory
    /// the moved ones come out of, as it reads under the directory they move
    /// into: absolute when it was written absolute, and relative to where the
    /// manifest is afterwards otherwise.
    fn spelled_after_the_move(&self, pattern: &Path, was_absolute: bool) -> String {
        let below = pattern
            .strip_prefix(self.relocation.moved_out_of())
            .unwrap_or_else(|_| {
                unreachable!(
                    "{} was checked to be under {}",
                    pattern.display(),
                    self.relocation.moved_out_of().display()
                )
            });
        let moved = self.relocation.moved_into().join(below);
        if was_absolute {
            moved.display().to_string()
        } else {
            relative(&normalize(&self.base_after), &moved)
        }
    }
}

/// Whether `path` is a directory, not following a symbolic link: a link is
/// an entry of its own, which stays where it is whatever it leads to.
fn is_a_directory(path: &Path) -> Result<bool, Failure> {
    match path.symlink_metadata() {
        Ok(metadata) => Ok(metadata.is_dir()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(Failure::new(format!("reading {} failed", path.display())).caused_by(error))
        }
    }
}

/// Every entry in the directory `directory`, or none when it is not one.
fn children_of(directory: &Path) -> Result<Vec<PathBuf>, Failure> {
    if !is_a_directory(directory)? {
        return Ok(Vec::new());
    }
    let listing_failed = |error: std::io::Error| {
        Failure::new(format!("listing {} failed", directory.display())).caused_by(error)
    };
    let mut children = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(listing_failed)? {
        children.push(entry.map_err(listing_failed)?.path());
    }
    Ok(children)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::tests::{project_with_manifest, relocation, reported};
    use crate::manifest::Manifest;
    use crate::relocation::Relocation;
    use crate::test_support::{ScratchDir, TestOutcome};

    /// The members of a 0.1 project that grouped `lint` in `tasks/group/`,
    /// kept the group out of its glob, and named `lint` itself.
    const GROUPED: &str = "[workspace]\n\
                           members = [\"ritual\", \"tasks/*\", \"tasks/group/lint\"]\n";

    /// A grouped project with `exclude` holding `excluded`, its manifest
    /// read, and `extra` paths created under it: a directory for each one
    /// ending in `/`, and an empty file otherwise.
    fn grouped_project(
        excluded: &str,
        extra: &[&str],
    ) -> Result<(ScratchDir, Manifest), Box<dyn std::error::Error>> {
        let (scratch, manifest) = project_with_manifest(
            "members-grouped",
            "Cargo.toml",
            &format!("{GROUPED}exclude = {excluded}\n"),
        )?;
        std::fs::remove_dir_all(scratch.path().join("tasks/helper"))?;
        std::fs::create_dir_all(scratch.path().join("tasks/group/lint"))?;
        for path in extra {
            let full = scratch.path().join(path);
            if path.ends_with('/') {
                std::fs::create_dir_all(full)?;
            } else {
                std::fs::write(full, "")?;
            }
        }
        Ok((scratch, manifest))
    }

    /// `greet`, `shout` and the grouped `lint` moving to `.rituals/`.
    fn grouped_relocation(root: &Path) -> Relocation {
        Relocation::new(
            &root.join("tasks"),
            &root.join(".rituals"),
            [
                root.join("tasks/greet"),
                root.join("tasks/shout"),
                root.join("tasks/group/lint"),
            ],
        )
    }

    /// `tasks/group` matches the glob but everything in it moves, so it does
    /// not keep the glob: once it is removed `tasks/*` would match nothing.
    /// The `exclude` that kept it out of the glob follows it to
    /// `.rituals/group`, where `.rituals/*` would otherwise match it.
    #[test]
    fn a_group_whose_tasks_all_move_neither_keeps_the_glob_nor_its_old_exclude() -> TestOutcome {
        let (scratch, mut manifest) = grouped_project("[\"tasks/group\"]", &[])?;

        let changes = reported(&mut manifest, &grouped_relocation(scratch.path()))?;

        assert_eq!(
            changes,
            [
                "[workspace] members `tasks/*` is now `.rituals/*`",
                "[workspace] members `tasks/group/lint` is now `.rituals/group/lint`",
                "[workspace] exclude `tasks/group` is now `.rituals/group`",
            ]
        );
        assert_eq!(
            manifest.document.to_string(),
            "[workspace]\n\
             members = [\"ritual\", \".rituals/*\", \".rituals/group/lint\"]\n\
             exclude = [\".rituals/group\"]\n"
        );
        Ok(())
    }

    /// An `exclude` that already names the new place loses the old entry,
    /// whose directory is gone, rather than naming it twice.
    #[test]
    fn an_exclude_already_listing_the_new_place_drops_the_old_one() -> TestOutcome {
        let (scratch, mut manifest) =
            grouped_project("[\"tasks/group\", \".rituals/group\"]", &[])?;

        let changes = reported(&mut manifest, &grouped_relocation(scratch.path()))?;

        assert_eq!(
            changes,
            [
                "[workspace] members `tasks/*` is now `.rituals/*`",
                "[workspace] members `tasks/group/lint` is now `.rituals/group/lint`",
                "[workspace] exclude drops `tasks/group`, which is gone once its contents move, \
                 since it already lists `.rituals/group`",
            ]
        );
        assert_eq!(
            manifest.document.to_string(),
            "[workspace]\n\
             members = [\"ritual\", \".rituals/*\", \".rituals/group/lint\"]\n\
             exclude = [\".rituals/group\"]\n"
        );
        Ok(())
    }

    /// A file left in the group keeps it where it is: the glob still matches
    /// it, so the glob stays and gains the new one, and its `exclude` stays
    /// and gains the new group, which the new glob would otherwise match.
    #[test]
    fn a_group_that_still_holds_a_file_keeps_the_glob_and_its_exclude() -> TestOutcome {
        let (scratch, mut manifest) =
            grouped_project("[\"tasks/group\"]", &["tasks/group/readme.md"])?;

        let changes = reported(&mut manifest, &grouped_relocation(scratch.path()))?;

        assert_eq!(
            changes,
            [
                "[workspace] members keeps `tasks/*`, which still matches `tasks/group`, and \
                 gains `.rituals/*`",
                "[workspace] members `tasks/group/lint` is now `.rituals/group/lint`",
                "[workspace] exclude keeps `tasks/group`, which still holds \
                 `tasks/group/readme.md`, and gains `.rituals/group`",
            ]
        );
        assert_eq!(
            manifest.document.to_string(),
            "[workspace]\n\
             members = [\"ritual\", \"tasks/*\", \".rituals/group/lint\", \".rituals/*\"]\n\
             exclude = [\"tasks/group\", \".rituals/group\"]\n"
        );
        Ok(())
    }

    /// An empty directory in the group stays where it is, so the group does,
    /// and its `exclude` with it; but nothing in it stays that the glob could
    /// read, so the glob is replaced.
    #[test]
    fn an_empty_directory_left_in_a_group_keeps_its_exclude_but_not_the_glob() -> TestOutcome {
        let (scratch, mut manifest) =
            grouped_project("[\"tasks/group\"]", &["tasks/group/scratch/"])?;

        let changes = reported(&mut manifest, &grouped_relocation(scratch.path()))?;

        assert_eq!(
            changes,
            [
                "[workspace] members `tasks/*` is now `.rituals/*`",
                "[workspace] members `tasks/group/lint` is now `.rituals/group/lint`",
                "[workspace] exclude keeps `tasks/group`, which still holds \
                 `tasks/group/scratch`, and gains `.rituals/group`",
            ]
        );
        Ok(())
    }

    /// An `exclude` entry that holds no moved directory, and one that is a
    /// glob, are left as written.
    #[test]
    fn an_exclude_holding_nothing_that_moves_is_left_alone() -> TestOutcome {
        let (scratch, mut manifest) = grouped_project(
            "[\"tasks/other\", \"tasks/g*\", \"vendor\"]",
            &["tasks/other/"],
        )?;

        let changes = reported(&mut manifest, &grouped_relocation(scratch.path()))?;

        assert!(
            changes.iter().all(|change| !change.contains("exclude")),
            "{changes:?}"
        );
        assert!(
            manifest
                .document
                .to_string()
                .ends_with("exclude = [\"tasks/other\", \"tasks/g*\", \"vendor\"]\n")
        );
        Ok(())
    }

    /// A path entry names a directory the way Cargo reads it, so it is
    /// matched however it is spelled and written back in one canonical form,
    /// in every list that holds paths. Anything that does not lead into a
    /// moved directory stays as written, `exclude`'s globs included.
    #[test]
    fn entries_naming_a_moved_directory_are_rewritten_in_every_list() -> TestOutcome {
        let (scratch, mut manifest) = project_with_manifest(
            "members-paths",
            "Cargo.toml",
            "[workspace]\n\
             members = [\n\
             \x20   \"ritual\",\n\
             \x20   \"tasks/./greet\", # the greeting\n\
             \x20   \"./tasks/shout/\",\n\
             \x20   \"tasks/helper\",\n\
             ]\n\
             default-members = [\"tasks//greet\"]\n\
             exclude = [\"tasks/greet/scratch\", \"vendor\", \"tasks/*\"]\n",
        )?;

        let changes = reported(&mut manifest, &relocation(scratch.path()))?;

        assert_eq!(
            changes,
            [
                "[workspace] members `tasks/./greet` is now `.rituals/greet`",
                "[workspace] members `./tasks/shout/` is now `.rituals/shout`",
                "[workspace] default-members `tasks//greet` is now `.rituals/greet`",
                "[workspace] exclude `tasks/greet/scratch` is now `.rituals/greet/scratch`",
            ]
        );
        assert_eq!(
            manifest.document.to_string(),
            "[workspace]\n\
             members = [\n\
             \x20   \"ritual\",\n\
             \x20   \".rituals/greet\", # the greeting\n\
             \x20   \".rituals/shout\",\n\
             \x20   \"tasks/helper\",\n\
             ]\n\
             default-members = [\".rituals/greet\"]\n\
             exclude = [\".rituals/greet/scratch\", \"vendor\", \"tasks/*\"]\n"
        );
        Ok(())
    }

    #[test]
    fn an_absolute_member_stays_absolute() -> TestOutcome {
        let (scratch, manifest) =
            project_with_manifest("members-absolute", "Cargo.toml", "[workspace]\n")?;
        let root = scratch.path();
        std::fs::write(
            manifest.path(),
            format!(
                "[workspace]\nmembers = [\"{root}/tasks/greet\", \"{root}/tasks/helper\"]\n",
                root = root.display()
            ),
        )?;
        let mut manifest = Manifest::read(manifest.path())?;

        let changes = reported(&mut manifest, &relocation(root))?;

        assert_eq!(
            changes,
            [format!(
                "[workspace] members `{root}/tasks/greet` is now `{root}/.rituals/greet`",
                root = root.display()
            )]
        );
        assert_eq!(
            manifest.document.to_string(),
            format!(
                "[workspace]\nmembers = [\"{root}/.rituals/greet\", \"{root}/tasks/helper\"]\n",
                root = root.display()
            )
        );
        Ok(())
    }

    /// Nothing but tasks is under `tasks/`, so the glob is the same glob in
    /// the new place and keeps matching every task.
    #[test]
    fn a_glob_whose_every_match_moves_is_replaced_in_place() -> TestOutcome {
        let (scratch, mut manifest) = project_with_manifest(
            "members-glob-replaced",
            "Cargo.toml",
            "[workspace]\nmembers = [\"ritual\", \"./tasks/*\"] # tasks\n",
        )?;
        std::fs::remove_dir_all(scratch.path().join("tasks/helper"))?;
        // Cargo skips a matched file, so it is not something that stays.
        std::fs::write(scratch.path().join("tasks/notes.md"), "not a crate\n")?;

        let changes = reported(&mut manifest, &relocation(scratch.path()))?;

        assert_eq!(
            changes,
            ["[workspace] members `./tasks/*` is now `.rituals/*`"]
        );
        assert_eq!(
            manifest.document.to_string(),
            "[workspace]\nmembers = [\"ritual\", \".rituals/*\"] # tasks\n"
        );
        Ok(())
    }

    /// `tasks/helper` is not a task and stays, so the glob keeps matching it
    /// and a glob for the new place is added: replacing it would drop the
    /// helper from the workspace.
    #[test]
    fn a_glob_that_still_matches_something_is_kept_and_the_new_one_added() -> TestOutcome {
        let (scratch, mut manifest) = project_with_manifest(
            "members-glob-kept",
            "Cargo.toml",
            "[workspace]\n\
             members = [\n\
             \x20   \"ritual\",\n\
             \x20   \"tasks/*\", # every task\n\
             ]\n\
             default-members = [\"tasks/*\"]\n",
        )?;

        let changes = reported(&mut manifest, &relocation(scratch.path()))?;

        assert_eq!(
            changes,
            [
                "[workspace] members keeps `tasks/*`, which still matches `tasks/helper`, and \
                 gains `.rituals/*`",
                "[workspace] default-members keeps `tasks/*`, which still matches \
                 `tasks/helper`, and gains `.rituals/*`",
            ]
        );
        assert_eq!(
            manifest.document.to_string(),
            "[workspace]\n\
             members = [\n\
             \x20   \"ritual\",\n\
             \x20   \"tasks/*\", # every task\n\
             \x20   \".rituals/*\",\n\
             ]\n\
             default-members = [\"tasks/*\", \".rituals/*\"]\n"
        );
        Ok(())
    }

    /// Asked twice, or written by hand already, the new glob is not added a
    /// second time, and nothing is reported because nothing changed.
    #[test]
    fn a_kept_glob_whose_new_place_is_already_listed_changes_nothing() -> TestOutcome {
        let content = "[workspace]\nmembers = [\"tasks/*\", \".rituals/*\"]\n";
        let (scratch, mut manifest) =
            project_with_manifest("members-glob-present", "Cargo.toml", content)?;

        let changes = reported(&mut manifest, &relocation(scratch.path()))?;

        assert_eq!(changes, Vec::<String>::new());
        assert_eq!(manifest.document.to_string(), content);
        Ok(())
    }

    /// A glob that matches nothing that moves, one that is nowhere near
    /// `tasks/`, and a workspace with no list of members, are left alone: a
    /// glob that matches nothing is something Cargo reads as a literal path,
    /// and nothing about the move touches it.
    #[test]
    fn globs_that_no_move_touches_are_left_as_written() -> TestOutcome {
        for content in [
            "[workspace]\nmembers = [\"ritual\", \"crates/*\", \"tasks/nothing-*\", \"*/x\"]\n",
            "[workspace]\nresolver = \"3\"\n",
            "[workspace]\nmembers = []\n",
        ] {
            let (scratch, mut manifest) =
                project_with_manifest("members-glob-untouched", "Cargo.toml", content)?;

            let changes = reported(&mut manifest, &relocation(scratch.path()))?;

            assert_eq!(changes, Vec::<String>::new(), "{content}");
            assert_eq!(manifest.document.to_string(), content);
        }
        Ok(())
    }

    /// A glob Cargo cannot expand either is its failure, named, and the
    /// document is as it was.
    #[test]
    fn a_glob_that_cannot_be_expanded_is_refused_and_changes_nothing() -> TestOutcome {
        let content = "[workspace]\nmembers = [\"tasks/greet\", \"tasks/[a\"]\n";
        let (scratch, mut manifest) =
            project_with_manifest("members-glob-invalid", "Cargo.toml", content)?;

        let outcome = manifest.repoint(&relocation(scratch.path()));

        let Err(failure) = outcome else {
            return Err("a glob that cannot be expanded must be refused".into());
        };
        assert!(
            failure
                .to_string()
                .starts_with("the [workspace] members glob `tasks/[a` could not be expanded: "),
            "{failure}"
        );
        assert_eq!(manifest.document.to_string(), content);
        Ok(())
    }
}
