//! The attributes git gives each of a list of paths.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use super::Attribute;
use super::scoped;
use crate::git::index::parse_attributes;
use crate::git::{Unanswered, nul_terminated, run_git_with_input};

/// What `git check-attr -a -z --stdin` is asked, after the leading options
/// that choose which tree it asks about. `-a` lists every attribute the path
/// has, a macro such as `binary` expanded into the ones it sets.
const CHECK_ATTR: [&str; 4] = ["check-attr", "-a", "-z", "--stdin"];

/// Asks which attributes git gives each of `paths`, each spelled from the top
/// level of the work tree `directory` stands in.
///
/// `scope` is any leading options that choose the tree, as for the ignore
/// question. A path git gives no attribute is in the answer with an empty
/// list, and each list is sorted by the attributes' names, so two answers
/// compare as the sets they are.
pub(super) fn ask(
    new_git: &impl Fn() -> Command,
    directory: &Path,
    scope: &[String],
    paths: &[String],
) -> Result<BTreeMap<String, Vec<Attribute>>, Unanswered> {
    if paths.is_empty() {
        return Ok(BTreeMap::new());
    }
    let arguments = scoped(scope, &CHECK_ATTR);
    let input = nul_terminated(paths.iter().map(String::as_bytes));
    let output = run_git_with_input(new_git, directory, &arguments, &input)?;
    let mut answers: BTreeMap<String, Vec<Attribute>> = paths
        .iter()
        .map(|path| (path.clone(), Vec::new()))
        .collect();
    for (path, name, value) in parse_attributes(&output)? {
        let path = path.to_string_lossy().into_owned();
        let Some(attributes) = answers.get_mut(&path) else {
            return Err(Unanswered::Failed(format!(
                "git check-attr answered about {path:?}, which was not asked about"
            )));
        };
        attributes.extend(Attribute::from_check_attr(&name, &value));
    }
    for attributes in answers.values_mut() {
        attributes.sort_by(|first, second| first.name().cmp(second.name()));
    }
    Ok(answers)
}
