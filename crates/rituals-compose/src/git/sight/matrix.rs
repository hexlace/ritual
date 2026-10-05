//! The repositories the move check is judged against, and the oracle that
//! says whether its judgement can be trusted.
//!
//! Each [`Case`] builds a repository whose `tasks/greet` is about to move to
//! `.rituals/greet`, and says how the check should judge it. Two tests read
//! the one table: one asks the check and compares what it says with what the
//! case expects, and the other, the oracle, asks the check, then really
//! renames the directory and asks git itself about the new places. The check
//! answers from a work tree of copies, which is only worth trusting if git
//! says the same of the real tree once the move has happened.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::prediction::{Prediction, predict, relative_to};
use super::{attributes, ensure_with, ignore, sparse};
use crate::git::fixture::{commit_everything, git};
use crate::git::test_support::contained_in;
use crate::git::{Flag, SeenDifferently, run_git, top_level_of};
use crate::relocation::Relocation;
use crate::test_support::{ScratchDir, TestOutcome};

/// What building a case's repository hands the test: the global git
/// configuration file the check must be run with, for a case about what a
/// developer's own configuration says.
type Built = Result<Option<PathBuf>, Box<dyn Error>>;

/// A repository, and how the check should judge moving `tasks/greet` out of
/// it.
struct Case {
    name: &'static str,
    /// Builds the repository at the path, which does not exist yet.
    build: fn(&Path) -> Built,
    /// The kind of difference the check should find, then one line for each
    /// file, in the order git lists them; nothing at all for a move that
    /// changes nothing. A path inside the scratch directory is `<scratch>`.
    expected: &'static [&'static str],
}

/// Every case in the table.
fn cases() -> Vec<Case> {
    let mut cases = ignore_cases();
    cases.extend(rules_outside_the_project_cases());
    cases.extend(attribute_cases());
    cases.extend(index_and_checkout_cases());
    cases
}

/// The cases about which files the project's own ignore files ignore.
fn ignore_cases() -> Vec<Case> {
    vec![
        Case {
            name: "a rule for dot directories with the usual negations",
            build: dot_directories_with_negations,
            expected: &[
                "ignored afterwards",
                ".rituals/greet/Cargo.toml (`.*` in .gitignore:2)",
                ".rituals/greet/src/lib.rs (`.*` in .gitignore:2)",
            ],
        },
        Case {
            name: "a force-added file a rule matches at both places",
            build: a_force_added_file_a_rule_matches_at_both_places,
            expected: &[
                "ignored afterwards",
                ".rituals/greet/secret.txt (`secret.txt` in .gitignore:2)",
            ],
        },
        Case {
            name: "an ignored file the new place would not ignore",
            build: an_ignored_file_the_new_place_would_not_ignore,
            expected: &[
                "no longer ignored",
                "tasks/greet/.env (`tasks/greet/.env` in .gitignore:2)",
            ],
        },
        Case {
            name: "a rule in the directory the task moves out of",
            build: a_rule_in_the_directory_the_task_moves_out_of,
            expected: &[
                "no longer ignored",
                "tasks/greet/a.env (`*.env` in tasks/.gitignore:1)",
            ],
        },
        Case {
            name: "an ignore file inside the task moves with it",
            build: an_ignore_file_inside_the_task,
            expected: &[],
        },
        Case {
            name: "a pattern that matches at both places",
            build: a_pattern_that_matches_at_both_places,
            expected: &[],
        },
        Case {
            name: "a negation that re-includes the new directory",
            build: a_negation_that_re_includes_the_new_directory,
            expected: &[],
        },
        Case {
            name: "an ignored nested repository the new place would not ignore",
            build: an_ignored_nested_repository_the_new_place_would_not_ignore,
            expected: &[
                "no longer ignored",
                "tasks/greet/vendor/up (`tasks/greet/vendor/` in .gitignore:2)",
            ],
        },
        Case {
            name: "an untracked file the new place would ignore",
            build: an_untracked_file_the_new_place_would_ignore,
            expected: &[
                "ignored afterwards",
                ".rituals/greet/notes.txt (`.rituals/greet/notes.txt` in .gitignore:2)",
            ],
        },
        Case {
            name: "file names with a space and with non-ASCII letters",
            build: file_names_with_a_space_and_non_ascii_letters,
            expected: &[],
        },
    ]
}

/// The cases about ignore rules that are not in a file of the project: the repository's own `info/exclude`, and a developer's global file.
fn rules_outside_the_project_cases() -> Vec<Case> {
    vec![
        Case {
            name: "a rule in info/exclude",
            build: a_rule_in_info_exclude,
            expected: &[
                "ignored afterwards",
                ".rituals/greet/Cargo.toml (`/.rituals` in .git/info/exclude:1)",
                ".rituals/greet/src/lib.rs (`/.rituals` in .git/info/exclude:1)",
            ],
        },
        Case {
            name: "a rule in info/exclude of a linked work tree",
            build: a_rule_in_info_exclude_of_a_linked_work_tree,
            expected: &[
                "ignored afterwards",
                ".rituals/greet/Cargo.toml (`/.rituals` in <scratch>/main/.git/info/exclude:1)",
                ".rituals/greet/src/lib.rs (`/.rituals` in <scratch>/main/.git/info/exclude:1)",
            ],
        },
        Case {
            name: "a rule in the global excludes file",
            build: a_rule_in_the_global_excludes_file,
            expected: &[
                "ignored afterwards",
                ".rituals/greet/Cargo.toml (`/.rituals` in <scratch>/global-ignore:1)",
                ".rituals/greet/src/lib.rs (`/.rituals` in <scratch>/global-ignore:1)",
            ],
        },
    ]
}

/// The cases about which attributes git gives.
fn attribute_cases() -> Vec<Case> {
    vec![
        Case {
            name: "an attributes rule keyed on tasks",
            build: an_attributes_rule_keyed_on_tasks,
            expected: &[
                "attributes",
                "tasks/greet/assets/logo.bin: diff=lfs, filter=lfs, merge=lfs, -text => none",
            ],
        },
        Case {
            name: "a rule in info/attributes",
            build: a_rule_in_info_attributes,
            expected: &[
                "attributes",
                "tasks/greet/Cargo.toml: export-ignore => none",
                "tasks/greet/src/lib.rs: export-ignore => none",
            ],
        },
        Case {
            name: "a rule in the global attributes file",
            build: a_rule_in_the_global_attributes_file,
            expected: &[
                "attributes",
                "tasks/greet/Cargo.toml: export-ignore => none",
                "tasks/greet/src/lib.rs: export-ignore => none",
            ],
        },
        Case {
            name: "an attributes file inside the task with the binary macro",
            build: an_attributes_file_inside_the_task,
            expected: &[],
        },
    ]
}

/// The cases about what the index and the checkout say.
fn index_and_checkout_cases() -> Vec<Case> {
    vec![
        Case {
            name: "a cone sparse checkout without the new directory",
            build: a_cone_sparse_checkout_without_the_new_directory,
            expected: &[
                "outside sparse checkout",
                ".rituals/greet/Cargo.toml",
                ".rituals/greet/src/lib.rs",
            ],
        },
        Case {
            name: "a sparse checkout turned off with its patterns still there",
            build: a_sparse_checkout_turned_off,
            expected: &[],
        },
        Case {
            name: "an assume-unchanged file",
            build: an_assume_unchanged_file,
            expected: &["unwatched", "tasks/greet/src/lib.rs (assume-unchanged)"],
        },
        Case {
            name: "a skip-worktree file that is on disk",
            build: a_skip_worktree_file_on_disk,
            expected: &["unwatched", "tasks/greet/src/lib.rs (skip-worktree)"],
        },
        Case {
            name: "a skip-worktree file that is not on disk",
            build: a_skip_worktree_file_not_on_disk,
            expected: &[],
        },
        Case {
            name: "a symbolic link for a task directory a rule would ignore",
            build: a_symbolic_link_a_rule_would_ignore,
            expected: &[
                "ignored afterwards",
                ".rituals/greet (`.rituals/` in .gitignore:2)",
            ],
        },
        Case {
            name: "a symbolic link for a task directory nothing differs for",
            build: a_symbolic_link_nothing_differs_for,
            expected: &[],
        },
    ]
}

fn write(root: &Path, file: &str, contents: &str) -> TestOutcome {
    let path = root.join(file);
    std::fs::create_dir_all(path.parent().ok_or("a fixture file has a directory")?)?;
    std::fs::write(path, contents)?;
    Ok(())
}

/// The task, and a project ignore file that keeps the build directory out,
/// with `files` written over them, in a repository that is initialised and
/// has not committed anything.
fn repository(root: &Path, files: &[(&str, &str)]) -> TestOutcome {
    write(
        root,
        "tasks/greet/Cargo.toml",
        "[package]\nname = \"greet\"\n",
    )?;
    write(root, "tasks/greet/src/lib.rs", "//! The greet task.\n")?;
    write(root, ".gitignore", "/target\n")?;
    for (file, contents) in files {
        write(root, file, contents)?;
    }
    git(root, &["init", "--quiet"])?;
    Ok(())
}

/// [`repository`], with everything committed.
fn committed(root: &Path, files: &[(&str, &str)]) -> TestOutcome {
    repository(root, files)?;
    commit_everything(root)?;
    Ok(())
}

/// A rule that ignores every dot directory, but for the ones a project needs
/// tracked. `.rituals` is a dot directory.
fn dot_directories_with_negations(root: &Path) -> Built {
    committed(
        root,
        &[(
            ".gitignore",
            "/target\n.*\n!.gitignore\n!.github\n!.cargo\n",
        )],
    )?;
    Ok(None)
}

/// A file a rule ignores at both places is still tracked while it is
/// force-added, and a commit after the move would drop it: the new place is
/// ignored, so `git add --all` would not add it.
fn a_force_added_file_a_rule_matches_at_both_places(root: &Path) -> Built {
    repository(
        root,
        &[
            (".gitignore", "/target\nsecret.txt\n"),
            ("tasks/greet/secret.txt", "a secret\n"),
        ],
    )?;
    git(root, &["add", "--all"])?;
    git(root, &["add", "--force", "tasks/greet/secret.txt"])?;
    git(root, &["commit", "--message", "fixture"])?;
    Ok(None)
}

fn an_ignored_file_the_new_place_would_not_ignore(root: &Path) -> Built {
    committed(
        root,
        &[
            (".gitignore", "/target\ntasks/greet/.env\n"),
            ("tasks/greet/.env", "SECRET=1\n"),
        ],
    )?;
    Ok(None)
}

fn a_rule_in_the_directory_the_task_moves_out_of(root: &Path) -> Built {
    committed(
        root,
        &[
            ("tasks/.gitignore", "*.env\n"),
            ("tasks/greet/a.env", "SECRET=1\n"),
        ],
    )?;
    Ok(None)
}

/// The task's own ignore file moves with it, so the file it ignores is
/// ignored at both places: before the move, nothing can see the file at the
/// new place, which is why the check asks a work tree that has it there.
fn an_ignore_file_inside_the_task(root: &Path) -> Built {
    committed(
        root,
        &[
            ("tasks/greet/.gitignore", ".env\n"),
            ("tasks/greet/.env", "SECRET=1\n"),
        ],
    )?;
    Ok(None)
}

fn a_pattern_that_matches_at_both_places(root: &Path) -> Built {
    committed(
        root,
        &[
            (".gitignore", "/target\n*.log\n"),
            ("tasks/greet/build.log", "output\n"),
        ],
    )?;
    Ok(None)
}

/// `-v` reports a negation as the rule that decided, and a path it names is
/// not ignored.
fn a_negation_that_re_includes_the_new_directory(root: &Path) -> Built {
    committed(
        root,
        &[(".gitignore", "/target\n.*\n!.rituals\n!.gitignore\n")],
    )?;
    Ok(None)
}

fn a_rule_in_info_exclude(root: &Path) -> Built {
    committed(root, &[])?;
    write(root, ".git/info/exclude", "/.rituals\n")?;
    Ok(None)
}

/// The repository under test is a linked work tree of one beside it, whose
/// `info/exclude` is the main repository's, outside the work tree.
fn a_rule_in_info_exclude_of_a_linked_work_tree(root: &Path) -> Built {
    let scratch = root.parent().ok_or("a fixture root has a parent")?;
    let main = scratch.join("main");
    committed(&main, &[])?;
    write(&main, ".git/info/exclude", "/.rituals\n")?;
    let linked = root.to_str().ok_or("a scratch path that is not UTF-8")?;
    git(
        &main,
        &["worktree", "add", "--quiet", linked, "-b", "linked"],
    )?;
    Ok(None)
}

/// The rule lives in a file a developer's own configuration names, which
/// the repository knows nothing about.
fn a_rule_in_the_global_excludes_file(root: &Path) -> Built {
    committed(root, &[])?;
    let scratch = std::fs::canonicalize(root.parent().ok_or("a fixture root has a parent")?)?;
    write(&scratch, "global-ignore", "/.rituals\n")?;
    let ignore = scratch.join("global-ignore");
    let configuration = scratch.join("global-gitconfig");
    write(
        &scratch,
        "global-gitconfig",
        &format!("[core]\n\texcludesFile = {}\n", ignore.display()),
    )?;
    Ok(Some(configuration))
}

/// Git LFS stores a file it filters as a pointer and the file's bytes
/// elsewhere. No driver is installed here, so the commit stores the file as
/// it is, which is all the check needs: the attributes decide by path.
fn an_attributes_rule_keyed_on_tasks(root: &Path) -> Built {
    repository(
        root,
        &[
            (
                ".gitattributes",
                "tasks/**/*.bin filter=lfs diff=lfs merge=lfs -text\n",
            ),
            ("tasks/greet/assets/logo.bin", "bin\n"),
        ],
    )?;
    commit_everything(root)?;
    Ok(None)
}

fn a_rule_in_info_attributes(root: &Path) -> Built {
    committed(root, &[])?;
    write(root, ".git/info/attributes", "tasks/** export-ignore\n")?;
    Ok(None)
}

/// The rule lives in a file a developer's own configuration names.
fn a_rule_in_the_global_attributes_file(root: &Path) -> Built {
    committed(root, &[])?;
    let scratch = std::fs::canonicalize(root.parent().ok_or("a fixture root has a parent")?)?;
    write(&scratch, "global-attributes", "tasks/** export-ignore\n")?;
    let attributes = scratch.join("global-attributes");
    let configuration = scratch.join("global-gitconfig");
    write(
        &scratch,
        "global-gitconfig",
        &format!("[core]\n\tattributesFile = {}\n", attributes.display()),
    )?;
    Ok(Some(configuration))
}

/// An attributes file inside the task moves with it, and the macro it uses
/// gives the same attributes at the new place.
fn an_attributes_file_inside_the_task(root: &Path) -> Built {
    committed(
        root,
        &[
            ("tasks/greet/.gitattributes", "*.bin binary\n"),
            ("tasks/greet/logo.bin", "bin\n"),
        ],
    )?;
    Ok(None)
}

fn a_cone_sparse_checkout_without_the_new_directory(root: &Path) -> Built {
    committed(root, &[])?;
    git(root, &["sparse-checkout", "set", "--cone", "tasks"])?;
    Ok(None)
}

/// `git sparse-checkout disable` turns the setting off and leaves the
/// patterns file, which `check-rules` would still read.
fn a_sparse_checkout_turned_off(root: &Path) -> Built {
    committed(root, &[])?;
    git(root, &["sparse-checkout", "set", "--cone", "tasks"])?;
    git(root, &["sparse-checkout", "disable"])?;
    Ok(None)
}

/// A repository of its own inside the task, which git lists as one entry and
/// a rule ignores now only because of where it is.
fn an_ignored_nested_repository_the_new_place_would_not_ignore(root: &Path) -> Built {
    repository(root, &[(".gitignore", "/target\ntasks/greet/vendor/\n")])?;
    let nested = root.join("tasks/greet/vendor/up");
    write(&nested, "up.txt", "a repository of its own\n")?;
    git(&nested, &["init", "--quiet"])?;
    commit_everything(&nested)?;
    commit_everything(root)?;
    Ok(None)
}

/// A file a commit would carry, not yet committed: git sees it now, so a
/// rule at the new place that ignores it changes what a commit carries.
fn an_untracked_file_the_new_place_would_ignore(root: &Path) -> Built {
    committed(
        root,
        &[(".gitignore", "/target\n.rituals/greet/notes.txt\n")],
    )?;
    write(root, "tasks/greet/notes.txt", "not committed\n")?;
    Ok(None)
}

/// Git is asked in NUL-separated form, so a name is never quoted or split.
fn file_names_with_a_space_and_non_ascii_letters(root: &Path) -> Built {
    committed(
        root,
        &[
            (".gitignore", "/target\n*.log\n"),
            ("tasks/greet/a b.log", "output\n"),
            ("tasks/greet/caf\u{e9}.rs", "//! A name with an accent.\n"),
        ],
    )?;
    Ok(None)
}

fn an_assume_unchanged_file(root: &Path) -> Built {
    committed(root, &[])?;
    git(
        root,
        &[
            "update-index",
            "--assume-unchanged",
            "tasks/greet/src/lib.rs",
        ],
    )?;
    Ok(None)
}

fn a_skip_worktree_file_on_disk(root: &Path) -> Built {
    committed(root, &[])?;
    git(
        root,
        &["update-index", "--skip-worktree", "tasks/greet/src/lib.rs"],
    )?;
    Ok(None)
}

/// A rename leaves behind a file that is not on disk, so there is nothing
/// to say about where it would be.
fn a_skip_worktree_file_not_on_disk(root: &Path) -> Built {
    committed(root, &[])?;
    git(
        root,
        &["update-index", "--skip-worktree", "tasks/greet/src/lib.rs"],
    )?;
    std::fs::remove_file(root.join("tasks/greet/src/lib.rs"))?;
    Ok(None)
}

/// The task directory is a link to a directory beside `tasks/`. Git keeps a
/// link as one entry, so the check asks about the link and not about the
/// files it leads to.
fn a_linked_task(root: &Path, ignore: &str) -> TestOutcome {
    write(
        root,
        "elsewhere/greet/Cargo.toml",
        "[package]\nname = \"greet\"\n",
    )?;
    write(root, ".gitignore", ignore)?;
    std::fs::create_dir_all(root.join("tasks"))?;
    std::os::unix::fs::symlink("../elsewhere/greet", root.join("tasks/greet"))?;
    git(root, &["init", "--quiet"])?;
    commit_everything(root)?;
    Ok(())
}

fn a_symbolic_link_a_rule_would_ignore(root: &Path) -> Built {
    a_linked_task(root, "/target\n.rituals/\n")?;
    Ok(None)
}

fn a_symbolic_link_nothing_differs_for(root: &Path) -> Built {
    a_linked_task(root, "/target\n")?;
    Ok(None)
}

/// A built repository: where it is, which global configuration to ask with,
/// and the scratch directory that holds it, which goes when this does.
struct Fixture {
    scratch: ScratchDir,
    root: PathBuf,
    global_configuration: Option<PathBuf>,
}

impl Fixture {
    fn new(case: &Case) -> Result<Self, Box<dyn Error>> {
        let scratch = ScratchDir::new("sight-matrix")?;
        let root = scratch.path().join("project");
        let global_configuration = (case.build)(&root)
            .map_err(|error| format!("building {:?} failed: {error}", case.name))?;
        Ok(Self {
            scratch,
            root,
            global_configuration,
        })
    }

    /// `git`, kept to the scratch directory and reading the case's global
    /// configuration, when it has one.
    fn new_git(&self) -> impl Fn() -> Command + use<'_> {
        let contained = contained_in(&self.root);
        let global_configuration = self.global_configuration.clone();
        move || {
            let mut command = contained();
            if let Some(configuration) = &global_configuration {
                command.env("GIT_CONFIG_GLOBAL", configuration);
            }
            command
        }
    }

    fn relocation(&self) -> Relocation {
        Relocation::new(
            &self.root.join("tasks"),
            &self.root.join(".rituals"),
            [self.root.join("tasks/greet")],
        )
    }

    /// The scratch directory as git spells it, resolved through symbolic
    /// links.
    fn scratch_as_git_spells_it(&self) -> Result<String, Box<dyn Error>> {
        Ok(std::fs::canonicalize(self.scratch.path())?
            .to_string_lossy()
            .into_owned())
    }
}

/// What the check said, as the case's `expected` lines say it.
fn described(result: &Result<(), SeenDifferently>, scratch: &str) -> Vec<String> {
    let lines: Vec<String> = match result {
        Ok(()) => return Vec::new(),
        Err(SeenDifferently::Unanswered(unanswered)) => {
            vec!["unanswered".to_string(), unanswered.to_string()]
        }
        Err(SeenDifferently::WouldBeIgnored(files)) => {
            std::iter::once("ignored afterwards".to_string())
                .chain(files.iter().map(|ignored| {
                    format!("{} ({})", ignored.file().to().display(), ignored.rule())
                }))
                .collect()
        }
        Err(SeenDifferently::WouldNoLongerBeIgnored(files)) => {
            std::iter::once("no longer ignored".to_string())
                .chain(files.iter().map(|ignored| {
                    format!("{} ({})", ignored.file().from().display(), ignored.rule())
                }))
                .collect()
        }
        Err(SeenDifferently::AttributesWouldChange(changes)) => {
            let say = |attributes: &[crate::git::Attribute]| {
                if attributes.is_empty() {
                    "none".to_string()
                } else {
                    attributes
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            };
            std::iter::once("attributes".to_string())
                .chain(changes.iter().map(|change| {
                    format!(
                        "{}: {} => {}",
                        change.file().from().display(),
                        say(change.before()),
                        say(change.after())
                    )
                }))
                .collect()
        }
        Err(SeenDifferently::OutsideSparseCheckout(files)) => {
            std::iter::once("outside sparse checkout".to_string())
                .chain(files.iter().map(|file| file.to().display().to_string()))
                .collect()
        }
        Err(SeenDifferently::Unwatched(files)) => std::iter::once("unwatched".to_string())
            .chain(files.iter().map(|file| {
                let flag = match file.flag() {
                    Flag::AssumeUnchanged => "assume-unchanged",
                    Flag::SkipWorktree => "skip-worktree",
                };
                format!("{} ({flag})", file.path().display())
            }))
            .collect(),
    };
    lines
        .into_iter()
        .map(|line| line.replace(scratch, "<scratch>"))
        .collect()
}

/// Every case in the table is judged as it says it should be.
///
/// Builds each repository, asks the check about moving `tasks/greet` into
/// `.rituals/`, and compares what it said, as lines, with what the case
/// expects. Every case is run and every disagreement reported together, so
/// one wrong case does not hide the next.
#[test]
fn every_case_is_judged_as_it_should_be() -> TestOutcome {
    let mut disagreements = Vec::new();
    for case in cases() {
        let built = Fixture::new(&case)?;
        let result = ensure_with(built.new_git(), &built.relocation(), &built.root);

        let said = described(&result, &built.scratch_as_git_spells_it()?);
        if said != case.expected {
            disagreements.push(format!(
                "{}:\n  expected {:?}\n  said     {said:?}",
                case.name, case.expected
            ));
        }
    }

    assert!(
        disagreements.is_empty(),
        "the check judged these cases otherwise:\n{}",
        disagreements.join("\n")
    );
    Ok(())
}

/// What git says about the new places once the directory has really moved
/// agrees with what the check predicted before it did.
///
/// For every case, asks the check, then renames `tasks/greet` to
/// `.rituals/greet` in the same repository and asks git itself, in the real
/// tree, the three things the check predicts from its work tree of copies:
/// the rule that ignores each new path, the attributes of each, and whether
/// the sparse-checkout patterns include it. Then stages everything, as a
/// commit after the move would, and checks the outcome the prediction is for:
/// a file is in the index at its new place exactly when no rule ignores it
/// there. A disagreement anywhere is a case the check cannot be trusted for.
#[test]
fn the_prediction_agrees_with_git_after_a_real_move() -> TestOutcome {
    let mut disagreements = Vec::new();
    for case in cases() {
        let built = Fixture::new(&case)?;
        let new_git = built.new_git();
        let prediction = predict(&new_git, &built.relocation(), &built.root)
            .map_err(|error| format!("predicting {:?} failed: {error}", case.name))?;
        assert!(
            !prediction.files.is_empty(),
            "{:?} must have files that move, or it shows nothing",
            case.name
        );

        std::fs::create_dir_all(built.root.join(".rituals"))?;
        std::fs::rename(
            built.root.join("tasks/greet"),
            built.root.join(".rituals/greet"),
        )?;

        let top_level = top_level_of(&new_git, &built.root)
            .map_err(|error| format!("{}: {error}", case.name))?;
        for disagreement in disagreements_with_git(&new_git, &top_level, &prediction)
            .map_err(|error| format!("asking git about {:?} failed: {error}", case.name))?
        {
            disagreements.push(format!("{}: {disagreement}", case.name));
        }
    }

    assert!(
        disagreements.is_empty(),
        "git and the prediction disagree:\n{}",
        disagreements.join("\n")
    );
    Ok(())
}

/// Where git, asked in the real tree after the move, says something other
/// than `prediction` did.
fn disagreements_with_git(
    new_git: &impl Fn() -> Command,
    top_level: &Path,
    prediction: &Prediction,
) -> Result<Vec<String>, crate::git::Unanswered> {
    let mut disagreements = Vec::new();
    let files = &prediction.files;

    let to_paths: Vec<String> = files.iter().map(|file| file.to.clone()).collect();
    let ignored = ignore::ask(new_git, top_level, &[], &to_paths)?;
    for (file, rule) in files.iter().zip(ignored) {
        let rule = rule.map(|rule| relative_to(rule, top_level));
        if rule != file.ignored_afterwards {
            disagreements.push(format!(
                "{}: the check predicted the rule {:?} and git said {rule:?}",
                file.to, file.ignored_afterwards
            ));
        }
    }

    let with_attributes: Vec<&crate::git::sight::prediction::Predicted> = files
        .iter()
        .filter(|file| file.attributes.is_some())
        .collect();
    let paths: Vec<String> = with_attributes.iter().map(|file| file.to.clone()).collect();
    let mut said = attributes::ask(new_git, top_level, &[], &paths)?;
    for file in with_attributes {
        let predicted = file
            .attributes
            .as_ref()
            .map(|(_before, after)| after.clone());
        let attributes = said.remove(&file.to);
        if attributes != predicted {
            disagreements.push(format!(
                "{}: the check predicted the attributes {predicted:?} and git said {attributes:?}",
                file.to
            ));
        }
    }

    let in_sparse: Vec<&crate::git::sight::prediction::Predicted> = files
        .iter()
        .filter(|file| file.in_sparse_checkout.is_some())
        .collect();
    let paths: Vec<String> = in_sparse.iter().map(|file| file.to.clone()).collect();
    let included = sparse::included(new_git, top_level, &paths)?;
    for file in &in_sparse {
        if file.in_sparse_checkout != Some(included.contains(&file.to)) {
            disagreements.push(format!(
                "{}: the check predicted the sparse checkout includes it: {:?}, and git said {}",
                file.to,
                file.in_sparse_checkout,
                included.contains(&file.to)
            ));
        }
    }

    if in_sparse
        .iter()
        .all(|file| file.in_sparse_checkout == Some(true))
    {
        disagreements.extend(disagreements_after_staging(new_git, top_level, prediction)?);
    }
    Ok(disagreements)
}

/// Stages everything and reads the index: a file that moved is in it at its
/// new place when nothing ignores it there, and is not when something does.
/// A repository of its own is not asked, since staging one adds a gitlink and
/// warns.
fn disagreements_after_staging(
    new_git: &impl Fn() -> Command,
    top_level: &Path,
    prediction: &Prediction,
) -> Result<Vec<String>, crate::git::Unanswered> {
    run_git(new_git, top_level, &["add", "--all"])?;
    let staged = run_git(new_git, top_level, &["ls-files", "-z", "--cached"])?;
    let staged: Vec<String> = String::from_utf8_lossy(&staged)
        .split('\0')
        .map(str::to_string)
        .collect();
    Ok(prediction
        .files
        .iter()
        .filter(|file| !file.entry.is_a_directory())
        .filter_map(|file| {
            let predicted = file.ignored_afterwards.is_none();
            let found = staged.contains(&file.to);
            (predicted != found).then(|| {
                format!(
                    "{}: the check predicted git would stage it: {predicted}, and git did: {found}",
                    file.to
                )
            })
        })
        .collect())
}
