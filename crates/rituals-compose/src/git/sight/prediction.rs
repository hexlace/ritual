//! What git would say about every file that moves, once it has moved, asked
//! before anything has.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::listing::{self, Entry, Standing};
use super::places::Places;
use super::scratch::{Scratch, rule_file_copies};
use super::{Attribute, IgnoreRule, MovedFile};
use super::{attributes, ignore, sparse};
use crate::git::{Flag, Unanswered, run_git, top_level_of};
use crate::relocation::Relocation;

/// One file that moves, with everything git says about it at both places.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Predicted {
    /// The file where it is now, and how the index stands towards it.
    pub(super) entry: Entry,
    /// The file where it will be, spelled as the entry's path is.
    pub(super) to: String,
    /// The rule that ignores it now. Only asked of a file the index lacks,
    /// since a tracked file is one git sees whatever a rule says.
    pub(super) ignored_now: Option<IgnoreRule>,
    /// The rule that would ignore it at its new place.
    pub(super) ignored_afterwards: Option<IgnoreRule>,
    /// Its attributes now and at its new place. Only asked of a file git sees
    /// at both places: one it ignores is neither stored nor given attributes
    /// that decide anything.
    pub(super) attributes: Option<(Vec<Attribute>, Vec<Attribute>)>,
    /// Whether the sparse-checkout patterns include its new place, when the
    /// checkout is a sparse one and git sees the file.
    pub(super) in_sparse_checkout: Option<bool>,
}

impl Predicted {
    /// Whether git sees the file where it is now: it is tracked, or no rule
    /// ignores it.
    pub(super) const fn is_seen_now(&self) -> bool {
        match self.entry.standing {
            Standing::Tracked | Standing::Flagged(_) => true,
            Standing::Untracked => self.ignored_now.is_none(),
        }
    }

    /// Whether git sees the file at both places, and stores it at each, so
    /// that what it says about its attributes and its place in the checkout
    /// decides what a commit carries.
    fn is_stored_at_both_places(&self) -> bool {
        self.is_seen_now() && !self.entry.is_a_directory() && self.ignored_afterwards.is_none()
    }

    /// The file, where it is and where it will be.
    pub(super) fn moved_file(&self) -> MovedFile {
        MovedFile::new(
            PathBuf::from(self.entry.path.trim_end_matches('/')),
            PathBuf::from(self.to.trim_end_matches('/')),
        )
    }
}

/// Every file that moves, and where the directories it is under go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Prediction {
    pub(super) places: Places,
    pub(super) files: Vec<Predicted>,
}

/// Asks git, with `new_git`, what it says about every file under the
/// directories `relocation` moves, at its place now and at its place
/// afterwards, in the project at `workspace_root`.
///
/// Reads the project and the repository and writes nothing to either. What
/// git would say at the new places is asked of a work tree of copies in the
/// system's temporary directory, which is gone again when this returns.
pub(super) fn predict(
    new_git: &impl Fn() -> Command,
    relocation: &Relocation,
    workspace_root: &Path,
) -> Result<Prediction, Unanswered> {
    let top_level = top_level_of(new_git, workspace_root)?;
    let places = Places::of(relocation, &top_level)?;
    let directories: Vec<String> = places
        .moved()
        .iter()
        .map(|(before, _after)| before.clone())
        .collect();
    let mut files: Vec<Predicted> = moving_entries(new_git, &top_level, &directories)?
        .into_iter()
        .map(|entry| Predicted {
            to: places.destination(&entry.path),
            entry,
            ignored_now: None,
            ignored_afterwards: None,
            attributes: None,
            in_sparse_checkout: None,
        })
        .collect();
    if files.is_empty() {
        return Ok(Prediction { places, files });
    }

    let now = ignored_now(new_git, &top_level, &files)?;
    for (file, rule) in files.iter_mut().zip(now) {
        file.ignored_now = rule;
    }
    let scratch = Scratch::create()?;
    let scope = stand_in_for_the_new_places(new_git, &top_level, &places, &files, &scratch)?;
    let afterwards = ignore::ask(new_git, scratch.path(), &scope, &new_paths(&files))?;
    for (file, rule) in files.iter_mut().zip(afterwards) {
        file.ignored_afterwards = rule;
    }
    attributes_at_both_places(new_git, &top_level, &scope, &scratch, &mut files)?;
    sparse_checkout_at_new_places(new_git, &top_level, &mut files)?;
    Ok(Prediction { places, files })
}

/// The entries that move: every file under the directories, less the ones
/// that are not on disk to be moved. A tracked file marked skip-worktree and
/// absent is left behind by a rename, so nothing about where it would be is a
/// question.
fn moving_entries(
    new_git: &impl Fn() -> Command,
    top_level: &Path,
    directories: &[String],
) -> Result<Vec<Entry>, Unanswered> {
    Ok(listing::list(new_git, top_level, directories)?
        .into_iter()
        .filter(|entry| {
            entry.standing != Standing::Flagged(Flag::SkipWorktree)
                || top_level.join(&entry.path).symlink_metadata().is_ok()
        })
        .collect())
}

/// The rule that ignores each file now, in the real tree, for the files the
/// index lacks, and none for the rest.
fn ignored_now(
    new_git: &impl Fn() -> Command,
    top_level: &Path,
    files: &[Predicted],
) -> Result<Vec<Option<IgnoreRule>>, Unanswered> {
    let untracked: Vec<String> = files
        .iter()
        .filter(|file| file.entry.standing == Standing::Untracked)
        .map(|file| file.entry.path.clone())
        .collect();
    let mut answers = ignore::ask(new_git, top_level, &[], &untracked)?.into_iter();
    Ok(files
        .iter()
        .map(|file| match file.entry.standing {
            Standing::Untracked => answers.next().flatten(),
            Standing::Tracked | Standing::Flagged(_) => None,
        })
        .collect())
}

/// The leading options that make git ask about the stand-in work tree, after
/// filling it with the rule files as they will stand: the repository's own
/// git directory, and the scratch directory as its work tree.
fn stand_in_for_the_new_places(
    new_git: &impl Fn() -> Command,
    top_level: &Path,
    places: &Places,
    files: &[Predicted],
    scratch: &Scratch,
) -> Result<Vec<String>, Unanswered> {
    let entries: Vec<&str> = files.iter().map(|file| file.entry.path.as_str()).collect();
    let copies = rule_file_copies(places.moved(), &entries, |entry| places.destination(entry));
    scratch.fill(top_level, &copies)?;
    let git_directory = run_git(new_git, top_level, &["rev-parse", "--absolute-git-dir"])?;
    Ok(vec![
        format!(
            "--git-dir={}",
            String::from_utf8_lossy(&git_directory).trim_end()
        ),
        format!("--work-tree={}", scratch.path().display()),
    ])
}

fn new_paths(files: &[Predicted]) -> Vec<String> {
    files.iter().map(|file| file.to.clone()).collect()
}

/// Fills in each file's attributes now and at its new place, for the files
/// git stores at both.
fn attributes_at_both_places(
    new_git: &impl Fn() -> Command,
    top_level: &Path,
    scope: &[String],
    scratch: &Scratch,
    files: &mut [Predicted],
) -> Result<(), Unanswered> {
    let stored: Vec<usize> = (0..files.len())
        .filter(|index| files[*index].is_stored_at_both_places())
        .collect();
    let now: Vec<String> = stored
        .iter()
        .map(|i| files[*i].entry.path.clone())
        .collect();
    let afterwards: Vec<String> = stored.iter().map(|i| files[*i].to.clone()).collect();
    let mut now = attributes::ask(new_git, top_level, &[], &now)?;
    let mut afterwards = attributes::ask(new_git, scratch.path(), scope, &afterwards)?;
    for index in stored {
        let file = &mut files[index];
        let (Some(before), Some(after)) =
            (now.remove(&file.entry.path), afterwards.remove(&file.to))
        else {
            unreachable!("git was asked about {} and its new place", file.entry.path)
        };
        file.attributes = Some((before, after));
    }
    Ok(())
}

/// Fills in whether the sparse-checkout patterns include each file's new
/// place, when the checkout is a sparse one.
fn sparse_checkout_at_new_places(
    new_git: &impl Fn() -> Command,
    top_level: &Path,
    files: &mut [Predicted],
) -> Result<(), Unanswered> {
    if !sparse::is_on(new_git, top_level)? {
        return Ok(());
    }
    let stored: Vec<String> = files
        .iter()
        .filter(|file| file.is_stored_at_both_places())
        .map(|file| file.to.clone())
        .collect();
    let included = sparse::included(new_git, top_level, &stored)?;
    for file in files
        .iter_mut()
        .filter(|file| file.is_stored_at_both_places())
    {
        file.in_sparse_checkout = Some(included.contains(&file.to));
    }
    Ok(())
}
