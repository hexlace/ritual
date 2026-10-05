//! Repointing a `[workspace]`'s lists of members.

use std::path::Path;

use rituals::Failure;
use toml_edit::{Array, Item, TableLike};

use super::{Repointer, replace_string};
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

    /// Entries of `exclude` that are at or under a moved directory follow it.
    /// A glob there is left as it is: Cargo reads `exclude` as paths.
    fn excluded(&mut self, entries: &mut Array) -> Result<(), Failure> {
        for position in 0..entries.len() {
            let Some(value) = entries.get_mut(position) else {
                continue;
            };
            let is_a_path = value.as_str().is_some_and(|entry| !is_a_glob(entry));
            if is_a_path {
                self.repoint_value(value, "[workspace] exclude")?;
            }
        }
        Ok(())
    }

    /// Repoints the glob `entry`, which is `value`, when it matches a
    /// directory that moves, and returns the glob to add beside it when it
    /// has to stay.
    ///
    /// A glob that matches no directory that moves is left alone. When
    /// nothing else it matches stays, it is replaced in place by the same
    /// glob under the new place. When something does, such as a crate that is
    /// not a task, it is kept, since replacing it would drop that crate from
    /// the workspace, and the new place's glob is added beside it, unless
    /// `present` has it already.
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
        if !matches
            .iter()
            .any(|matched| self.relocation.destination(matched).is_some())
        {
            return Ok(None);
        }

        let mut surviving: Vec<String> = matches
            .iter()
            .filter(|matched| self.relocation.destination(matched).is_none() && matched.is_dir())
            .map(|matched| {
                matched
                    .strip_prefix(&self.base_before)
                    .unwrap_or(matched)
                    .display()
                    .to_string()
            })
            .collect();
        surviving.sort();

        let new = self.glob_after_the_move(&pattern, Path::new(entry).is_absolute());
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

    /// The glob `pattern`, normalised and under the directory the moved ones
    /// come out of, as it reads under the directory they move into: absolute
    /// when it was written absolute, and relative to where the manifest is
    /// afterwards otherwise.
    fn glob_after_the_move(&self, pattern: &Path, was_absolute: bool) -> String {
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

#[cfg(test)]
mod tests {
    use super::super::tests::{project_with_manifest, relocation, reported};
    use crate::manifest::Manifest;
    use crate::test_support::TestOutcome;

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
        std::fs::remove_dir(scratch.path().join("tasks/helper"))?;
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
