//! One path value a manifest edit changed, as a person reads it.

use std::fmt;

use crate::sentence::join_with_and;

/// One path value that [`Manifest::repoint`](super::Manifest::repoint)
/// changed, read only through [`Display`](fmt::Display).
///
/// It renders the clause a report puts after the file's name, spelled the
/// way every ritual message spells a manifest location: the table and key as
/// `[table] key`, and each value in backticks.
///
/// # Examples
///
/// ```
/// use rituals_compose::manifest::Manifest;
/// use rituals_compose::relocation::Relocation;
///
/// # let directory = std::env::temp_dir()
/// #     .join(format!("rituals-compose-doctest-path-change-{}", std::process::id()));
/// # std::fs::create_dir_all(&directory)?;
/// let manifest_path = directory.join("Cargo.toml");
/// # std::fs::write(
/// #     &manifest_path,
/// #     "[dependencies]\ngreet = { path = \"tasks/greet\" }\n",
/// # )?;
/// let mut manifest = Manifest::read(&manifest_path)?;
/// let relocation = Relocation::new(
///     &directory.join("tasks"),
///     &directory.join(".rituals"),
///     [directory.join("tasks/greet")],
/// );
///
/// let changes = manifest.repoint(&relocation)?;
///
/// assert_eq!(
///     changes[0].to_string(),
///     "[dependencies] greet path `tasks/greet` is now `.rituals/greet`"
/// );
/// # std::fs::remove_dir_all(&directory)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathChange(Change);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Change {
    /// A value that was respelled.
    Repointed {
        place: String,
        old: String,
        new: String,
    },
    /// A glob that still matches something that did not move, so it stays
    /// and a glob for the new place is added beside it.
    GlobKept {
        place: String,
        old: String,
        still_matching: Vec<String>,
        gained: String,
    },
}

impl PathChange {
    /// `place`, which reads as `[table] key`, held `old` and now holds `new`.
    pub(super) fn repointed(place: &str, old: &str, new: &str) -> Self {
        Self(Change::Repointed {
            place: place.to_string(),
            old: old.to_string(),
            new: new.to_string(),
        })
    }

    /// The list at `place` keeps the glob `old`, which still matches
    /// `still_matching`, and gains the glob `gained`.
    pub(super) fn glob_kept(
        place: &str,
        old: &str,
        still_matching: Vec<String>,
        gained: &str,
    ) -> Self {
        assert!(
            !still_matching.is_empty(),
            "a glob is kept only because something it matches stays"
        );
        Self(Change::GlobKept {
            place: place.to_string(),
            old: old.to_string(),
            still_matching,
            gained: gained.to_string(),
        })
    }
}

impl fmt::Display for PathChange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Change::Repointed { place, old, new } => {
                write!(formatter, "{place} `{old}` is now `{new}`")
            }
            Change::GlobKept {
                place,
                old,
                still_matching,
                gained,
            } => {
                let still_matching: Vec<String> = still_matching
                    .iter()
                    .map(|directory| format!("`{directory}`"))
                    .collect();
                write!(
                    formatter,
                    "{place} keeps `{old}`, which still matches {}, and gains `{gained}`",
                    join_with_and(&still_matching)
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PathChange;

    #[test]
    fn a_respelled_value_reads_as_the_place_then_old_and_new() {
        assert_eq!(
            PathChange::repointed("[workspace] members", "tasks/*", ".rituals/*").to_string(),
            "[workspace] members `tasks/*` is now `.rituals/*`"
        );
    }

    #[test]
    fn a_kept_glob_names_what_it_still_matches_and_what_it_gains() {
        assert_eq!(
            PathChange::glob_kept(
                "[workspace] members",
                "tasks/*",
                vec!["tasks/helper".to_string()],
                ".rituals/*"
            )
            .to_string(),
            "[workspace] members keeps `tasks/*`, which still matches `tasks/helper`, and gains \
             `.rituals/*`"
        );
        assert_eq!(
            PathChange::glob_kept(
                "[workspace] default-members",
                "tasks/*",
                vec!["tasks/helper".to_string(), "tasks/notes".to_string()],
                ".rituals/*"
            )
            .to_string(),
            "[workspace] default-members keeps `tasks/*`, which still matches `tasks/helper` and \
             `tasks/notes`, and gains `.rituals/*`"
        );
    }

    #[test]
    #[should_panic(expected = "a glob is kept only because something it matches stays")]
    fn a_kept_glob_with_nothing_left_to_match_is_a_bug() {
        let _ = PathChange::glob_kept("[workspace] members", "tasks/*", Vec::new(), ".rituals/*");
    }
}
