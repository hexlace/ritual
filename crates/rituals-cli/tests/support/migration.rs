//! Fixtures and checks shared by the stories about `migrate`.
//!
//! `migrate`'s messages are not fixed apart from `nothing to migrate`, so
//! these checks read what a run did from the tree it left, the manifests it
//! wrote and the exit status, and read its output only for names: that a
//! moved task, an edited manifest or a leftover file is mentioned, never for
//! how the sentence around the name goes.

use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use super::removal::assert_a_refusal;
use super::{
    OptionContext, Outcome, Project, ResultContext, RunOutput, TestOutcome, assert_trees_identical,
    git, manifest, snapshot_tree, tree::Snapshot,
};

/// Everything a run wrote, stdout then stderr: the output of a command that
/// leaves its report in either.
pub(crate) fn everything_written(output: &RunOutput) -> String {
    format!("{}\n{}", output.stdout, output.stderr)
}

/// Asserts `output` is a successful run that said, in so many words,
/// `nothing to migrate`.
#[track_caller]
pub(crate) fn assert_nothing_to_migrate(output: &RunOutput, what: &str) {
    output.expect_success(what);
    let written = everything_written(output);
    assert!(
        written.contains("nothing to migrate"),
        "expected {what} to say `nothing to migrate`; it wrote:\n{written}"
    );
}

/// Asserts the output of a run names every one of `names`.
#[track_caller]
pub(crate) fn assert_names(output: &RunOutput, what: &str, names: &[&str]) {
    let written = everything_written(output);
    for name in names {
        assert!(
            written.contains(name),
            "expected the output of {what} to name `{name}`; it wrote:\n{written}"
        );
    }
}

/// `relative`, as a path written in a manifest in `manifest_directory`,
/// resolved to the directory it names: joined, with `.` dropped and `..`
/// applied, without asking the file system (which may not have it any more).
pub(crate) fn resolve(manifest_directory: &Path, relative: &str) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in manifest_directory.join(relative).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other),
        }
    }
    resolved
}

/// Asserts that the path dependency `key` in the table at `table_keys` of the
/// manifest at `manifest_path` leads to `target`, however the path is
/// spelled.
///
/// What a move has to preserve is where a dependency leads, so this compares
/// destinations rather than text; a story that means to pin the text reads
/// the manifest itself.
#[track_caller]
pub(crate) fn assert_dependency_leads_to(
    manifest_path: &Path,
    table_keys: &[&str],
    key: &str,
    target: &Path,
) -> TestOutcome {
    let document = manifest::read(manifest_path)?;
    let written = manifest::dependency_path(&document, table_keys, key).ok_or_else(|| {
        format!(
            "expected {} to declare `{key}` as a path dependency; manifest was:\n{document}",
            manifest_path.display()
        )
    })?;
    let directory = manifest_path
        .parent()
        .context("a manifest has a directory")?;
    assert_eq!(
        resolve(directory, written),
        target,
        "expected `{key}` in {} (written `{written}`) to lead to {}",
        manifest_path.display(),
        target.display()
    );
    Ok(())
}

/// Whether `path` exists as anything at all.
pub(crate) fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// Makes the file at `path` read-only and reports whether that is enforced,
/// by trying to open it for writing. When it is not (the process is root,
/// say), it is made writable again before returning `false`.
pub(crate) fn made_file_read_only(path: &Path) -> Result<bool, std::io::Error> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444))?;
    let enforced = std::fs::OpenOptions::new().write(true).open(path).is_err();
    if !enforced {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))?;
    }
    Ok(enforced)
}

/// Makes the directory at `path` read-only and reports whether that is
/// enforced, by trying to create a file in it. When it is not, it is made
/// writable again before returning `false`.
pub(crate) fn made_directory_read_only(path: &Path) -> Result<bool, std::io::Error> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o555))?;
    let probe = path.join(".write-probe");
    let enforced = std::fs::File::create(&probe).is_err();
    if !enforced {
        std::fs::remove_file(&probe)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(enforced)
}

/// Gives `path` back its owner's write permission: `0o644` for a file,
/// `0o755` for a directory.
pub(crate) fn made_writable(path: &Path) -> TestOutcome {
    let mode = if path.is_dir() { 0o755 } else { 0o644 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .context(&format!("making {} writable failed", path.display()))
}

/// The `git …` commands a message quotes in backticks, in the order it
/// quotes them, with a quoted `a && b` taken as two. Each is returned as the
/// arguments after `git`.
///
/// A recovery command is the one thing a failed `migrate` may print for the
/// person to run, and the form it takes elsewhere in ritual: a command in
/// backticks, starting `git `.
pub(crate) fn printed_git_commands(message: &str) -> Vec<Vec<String>> {
    message
        .split('`')
        .skip(1)
        .step_by(2)
        .flat_map(|quoted| quoted.split("&&"))
        .filter_map(|command| {
            let mut words = command.split_whitespace();
            (words.next() == Some("git")).then(|| words.map(str::to_string).collect())
        })
        .collect()
}

/// What a story that failed `migrate` on purpose found afterwards.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WayBack {
    /// The project was left byte-identical to what it was.
    LeftAsItWas,
    /// The project was left changed, and the output gave a `git` command
    /// that put it back byte-identically.
    GivenBack(Vec<String>),
}

/// Checks a `migrate` that was made to fail kept its promise: it was refused
/// by ritual, naming each of `names`, and the project is either as it was
/// (`before`) or was put back by running the `git` commands the output
/// printed, from the project's root. Says which.
///
/// Whichever the output chose, a project that changed and printed no command
/// fails here, and so does a command that does not restore every byte: the
/// promise is a project that builds, or is as it was, or says exactly how to
/// get it back, and a message without a working way back is none of them.
///
/// Whatever made the run fail has to be undone before this is called, so the
/// printed commands run against a project git can write to; undoing it
/// changes no file's content, so it does not disturb the comparison.
#[track_caller]
pub(crate) fn assert_failure_kept_the_promise(
    project: &Project,
    what: &str,
    failed: &RunOutput,
    names: &[&str],
    before: &Snapshot,
) -> Outcome<WayBack> {
    assert_failure_kept_the_promise_reading(project, what, failed, names, before, || {
        snapshot_tree(project.root())
    })
}

/// [`assert_failure_kept_the_promise`], reading the project's tree with
/// `read_tree` instead of snapshotting all of it, for a story that holds a
/// part of the tree to a different standard and takes it out of both sides
/// of the comparison.
#[track_caller]
pub(crate) fn assert_failure_kept_the_promise_reading(
    project: &Project,
    what: &str,
    failed: &RunOutput,
    names: &[&str],
    before: &Snapshot,
    read_tree: impl Fn() -> Outcome<Snapshot>,
) -> Outcome<WayBack> {
    let bin_name = project.bin_name()?;
    let message = assert_a_refusal(failed, &bin_name, what);
    for name in names {
        assert!(
            message.contains(name),
            "expected the failure of {what} to name `{name}`; stderr was:\n{message}"
        );
    }

    let after = read_tree()?;
    if &after == before {
        return Ok(WayBack::LeftAsItWas);
    }

    let commands = printed_git_commands(&everything_written(failed));
    assert!(
        !commands.is_empty(),
        "{what} left the project changed ({:?}) and printed no `git` command to put it back; \
         it wrote:\n{}",
        super::tree::changed_paths(before, &after),
        everything_written(failed)
    );
    for command in &commands {
        let arguments: Vec<&str> = command.iter().map(String::as_str).collect();
        git::git(project.root(), &arguments)?
            .expect_success(&format!("the printed recovery `git {}`", command.join(" ")));
    }
    assert_trees_identical(
        &format!(
            "the recovery {what} printed ({commands:?}) must give every file back, byte for byte"
        ),
        before,
        &read_tree()?,
    );
    Ok(WayBack::GivenBack(
        commands.iter().map(|command| command.join(" ")).collect(),
    ))
}

/// A 0.1-layout project, committed, with the tasks `greet` and `shout`,
/// where `shout` depends on `greet` by `path = "../greet"` — the way a person
/// who has two tasks in `tasks/` and wants one to use the other writes it.
pub(crate) fn committed_project_where_shout_depends_on_greet(
    checkout: &super::Checkout,
    working_dir: &super::TempDir,
) -> Outcome<Project> {
    let project = super::legacy::project_with_tasks(checkout, working_dir, &["greet", "shout"])?;
    manifest::edit(&project.root().join("tasks/shout/Cargo.toml"), |document| {
        manifest::add_path_dependency(document, &["dependencies"], "greet", "../greet")
    })?;
    project.build()?;
    git::init_and_commit_everything(project.root())?;
    Ok(project)
}

/// The text of every manifest `new` and the 0.1 `add` write, with `@RITUALS@`
/// and `@CORE@` where the checkout's `crates/rituals` and
/// `crates/rituals-core` go, filled in for `checkout`.
pub(crate) fn fill_in_checkout(template: &str, checkout: &super::Checkout) -> Outcome<String> {
    let rituals = super::path_to_str(&checkout.root().join("crates/rituals"))?.to_string();
    let core = super::path_to_str(&checkout.root().join("crates/rituals-core"))?.to_string();
    Ok(template
        .replace("@RITUALS@", &rituals)
        .replace("@CORE@", &core))
}

/// Every path in `snapshot` at or below `directory`, as the paths they have
/// under `new_directory` instead, with their contents.
pub(crate) fn moved(snapshot: &Snapshot, directory: &str, new_directory: &str) -> Snapshot {
    snapshot
        .iter()
        .filter_map(|(path, entry)| {
            path.strip_prefix(directory)
                .ok()
                .map(|rest| (PathBuf::from(new_directory).join(rest), entry.clone()))
        })
        .collect()
}
