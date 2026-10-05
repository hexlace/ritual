//! What a prediction says about the move: the first kind of difference that
//! holds for any file, with every file it holds for.

use std::path::PathBuf;

use super::listing::Standing;
use super::prediction::{Predicted, Prediction};
use super::{AttributeChange, IgnoredFile, SeenDifferently};
use crate::git::Unwatched;

/// Judges `prediction`: nothing differs, or the first kind that does, in the
/// order they are checked.
///
/// An ignore rule that sits in a file that moves is named where the file is
/// now, not at its new place, where it is not yet.
pub(super) fn judge(prediction: &Prediction) -> Result<(), SeenDifferently> {
    let files = &prediction.files;

    let ignored: Vec<IgnoredFile> = files
        .iter()
        .filter(|file| file.is_seen_now())
        .filter_map(|file| {
            let rule = file.ignored_afterwards.as_ref()?;
            let source = prediction.places.at_its_current_place(rule.source());
            Some(IgnoredFile::new(
                file.moved_file(),
                super::IgnoreRule::new(source, rule.line(), rule.pattern().to_string()),
            ))
        })
        .collect();
    if !ignored.is_empty() {
        return Err(SeenDifferently::WouldBeIgnored(ignored));
    }

    let unignored: Vec<IgnoredFile> = files
        .iter()
        .filter(|file| !file.is_seen_now() && file.ignored_afterwards.is_none())
        .filter_map(|file| {
            Some(IgnoredFile::new(
                file.moved_file(),
                file.ignored_now.clone()?,
            ))
        })
        .collect();
    if !unignored.is_empty() {
        return Err(SeenDifferently::WouldNoLongerBeIgnored(unignored));
    }

    let changed: Vec<AttributeChange> = files
        .iter()
        .filter_map(|file| {
            let (before, after) = file.attributes.as_ref()?;
            (before != after)
                .then(|| AttributeChange::new(file.moved_file(), before.clone(), after.clone()))
        })
        .collect();
    if !changed.is_empty() {
        return Err(SeenDifferently::AttributesWouldChange(changed));
    }

    let outside: Vec<_> = files
        .iter()
        .filter(|file| file.in_sparse_checkout == Some(false))
        .map(Predicted::moved_file)
        .collect();
    if !outside.is_empty() {
        return Err(SeenDifferently::OutsideSparseCheckout(outside));
    }

    let unwatched: Vec<Unwatched> = files
        .iter()
        .filter_map(|file| match file.entry.standing {
            Standing::Flagged(flag) => Some(Unwatched::new(PathBuf::from(&file.entry.path), flag)),
            Standing::Tracked | Standing::Untracked => None,
        })
        .collect();
    if !unwatched.is_empty() {
        return Err(SeenDifferently::Unwatched(unwatched));
    }
    Ok(())
}
