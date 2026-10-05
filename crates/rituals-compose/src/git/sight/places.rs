//! Where each directory that moves is and will be, spelled from git's top
//! level, and where each file under one goes.

use std::path::{Path, PathBuf};

use crate::git::{Unanswered, canonical, from_the_top_level};
use crate::relocation::Relocation;

/// The directories of a [`Relocation`], spelled the way git is asked about
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Places {
    from_directory: PathBuf,
    to_directory: PathBuf,
    moved: Vec<(String, String)>,
}

impl Places {
    /// Spells the places of `relocation` from `top_level`, which is the top
    /// level of the repository the project is in, resolved through symbolic
    /// links.
    ///
    /// The directories everything moves out of and into are resolved through
    /// symbolic links, from their parents for the second, which does not
    /// exist yet. A moved directory is not: it stays the link it is, so git
    /// is asked about the link, which is what it keeps.
    pub(super) fn of(relocation: &Relocation, top_level: &Path) -> Result<Self, Unanswered> {
        let from_directory = from_the_top_level(&canonical(relocation.moved_out_of())?, top_level)?;

        let into = relocation.moved_into();
        let (Some(parent), Some(name)) = (into.parent(), into.file_name()) else {
            return Err(Unanswered::Failed(format!(
                "{} has no directory holding it to ask git from",
                into.display()
            )));
        };
        let to_directory = from_the_top_level(&canonical(parent)?.join(name), top_level)?;

        let moved = relocation
            .moved()
            .iter()
            .map(|directory| {
                let Ok(rest) = directory.strip_prefix(relocation.moved_out_of()) else {
                    unreachable!(
                        "{} was given to the relocation as under {}",
                        directory.display(),
                        relocation.moved_out_of().display()
                    )
                };
                (
                    spelled(&from_directory.join(rest)),
                    spelled(&to_directory.join(rest)),
                )
            })
            .collect();
        Ok(Self {
            from_directory,
            to_directory,
            moved,
        })
    }

    /// Each moved directory as `(where it is, where it will be)`.
    pub(super) fn moved(&self) -> &[(String, String)] {
        &self.moved
    }

    /// Where `path`, which is at or under a moved directory, will be: the
    /// same place under the directory everything moves into, with a trailing
    /// slash kept.
    pub(super) fn destination(&self, path: &str) -> String {
        let Ok(below) = Path::new(path.trim_end_matches('/')).strip_prefix(&self.from_directory)
        else {
            unreachable!(
                "{path} is under a moved directory, and every moved directory is under {}",
                self.from_directory.display()
            )
        };
        let mut destination = spelled(&self.to_directory.join(below));
        if path.ends_with('/') {
            destination.push('/');
        }
        destination
    }

    /// Where `path` is now when it is where a moved file will be, which is
    /// where a rule file's copy stands in the work tree git is asked about;
    /// and `path` itself when it is anywhere else.
    pub(super) fn at_its_current_place(&self, path: &Path) -> PathBuf {
        self.moved
            .iter()
            .find_map(|(before, after)| {
                path.strip_prefix(after)
                    .ok()
                    .map(|rest| Path::new(before).join(rest))
            })
            .unwrap_or_else(|| path.to_path_buf())
    }
}

/// `path` as the text git is asked about.
fn spelled(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::Places;
    use crate::relocation::Relocation;
    use crate::test_support::{ScratchDir, TestOutcome};

    /// `tasks/greet` and `tasks/group/deep` move into `.rituals/`, in a
    /// project whose root is `project/` below the top level.
    fn places(scratch: &ScratchDir) -> Result<Places, Box<dyn std::error::Error>> {
        let root = scratch.path().join("project");
        std::fs::create_dir_all(root.join("tasks"))?;
        let relocation = Relocation::new(
            &root.join("tasks"),
            &root.join(".rituals"),
            [root.join("tasks/greet"), root.join("tasks/group/deep")],
        );
        let top_level = std::fs::canonicalize(scratch.path())?;
        Ok(Places::of(&relocation, &top_level).map_err(|error| error.to_string())?)
    }

    #[test]
    fn moved_directories_are_spelled_from_the_top_level() -> TestOutcome {
        let scratch = ScratchDir::new("sight-places-spelled")?;

        let places = places(&scratch)?;

        assert_eq!(
            places.moved(),
            [
                (
                    "project/tasks/greet".to_string(),
                    "project/.rituals/greet".to_string()
                ),
                (
                    "project/tasks/group/deep".to_string(),
                    "project/.rituals/group/deep".to_string()
                ),
            ]
        );
        Ok(())
    }

    #[test]
    fn a_file_goes_to_the_same_place_under_the_new_directory_and_keeps_a_trailing_slash()
    -> TestOutcome {
        let scratch = ScratchDir::new("sight-places-destination")?;
        let places = places(&scratch)?;

        assert_eq!(
            places.destination("project/tasks/greet/src/lib.rs"),
            "project/.rituals/greet/src/lib.rs"
        );
        assert_eq!(
            places.destination("project/tasks/greet/vendor/up/"),
            "project/.rituals/greet/vendor/up/"
        );
        assert_eq!(
            places.destination("project/tasks/greet"),
            "project/.rituals/greet"
        );
        Ok(())
    }

    #[test]
    fn a_path_at_a_new_place_is_given_its_current_one_and_any_other_path_is_not() -> TestOutcome {
        let scratch = ScratchDir::new("sight-places-current")?;
        let places = places(&scratch)?;

        assert_eq!(
            places.at_its_current_place(Path::new("project/.rituals/greet/.gitignore")),
            PathBuf::from("project/tasks/greet/.gitignore")
        );
        assert_eq!(
            places.at_its_current_place(Path::new("project/.rituals/.gitignore")),
            PathBuf::from("project/.rituals/.gitignore")
        );
        assert_eq!(
            places.at_its_current_place(Path::new("/home/me/.config/git/ignore")),
            PathBuf::from("/home/me/.config/git/ignore")
        );
        Ok(())
    }
}
