//! What `import` is asked, and the words that ask it again.
//
// `redundant_pub_crate` (clippy nursery) wants `pub` here because this
// module is private, but `pub(crate)` is the visibility that is actually
// true — it stays correct if this module is ever re-exported at a different
// level — so the nursery lint gives way.
#![expect(
    clippy::redundant_pub_crate,
    reason = "pub(crate) reflects this module's actual visibility even though the enclosing \
              module is private; see the note above"
)]

use std::path::{Path, PathBuf};

use rituals::{Failure, Name, clap};
use rituals_compose::shell;

use crate::refusals;

/// `import`'s arguments: the crate to import, the key it answers to, and
/// where it comes from.
///
/// The grammar is Cargo's own, for the half that names the crate: the crate
/// is `<crate>[@<version>]`, and `--git`, `--branch`, `--tag`, `--rev` and
/// `--path` mean what they mean to `cargo add`. What clap checks is only how
/// the flags relate (a path is not a git repository, a branch needs one, and
/// a repository is taken at one reference or none), which is fixed, is shown
/// in `--help`, and is refused before any of `import` runs.
//
// The field for the crate is `crate_spec` because `crate` is a keyword.
#[derive(clap::Args, Debug)]
pub(crate) struct ImportArguments {
    /// the task crate to import, as Cargo names it; add `@<version>` for a release other than
    /// the newest
    #[arg(value_name = "CRATE")]
    crate_spec: String,

    /// the name the imported task will answer to; the crate's name when not given
    #[arg(value_name = "KEY")]
    key: Option<String>,

    /// take the crate from a git repository instead of the registry
    #[arg(long, value_name = "URL")]
    git: Option<String>,

    /// the branch of the `--git` repository to take
    #[arg(long, value_name = "BRANCH", requires = "git", group = "git_reference")]
    branch: Option<String>,

    /// the tag of the `--git` repository to take
    #[arg(long, value_name = "TAG", requires = "git", group = "git_reference")]
    tag: Option<String>,

    /// the commit of the `--git` repository to take
    #[arg(long, value_name = "REV", requires = "git", group = "git_reference")]
    rev: Option<String>,

    /// take the crate from a directory instead of the registry
    #[arg(long, value_name = "DIR", conflicts_with = "git")]
    path: Option<PathBuf>,
}

/// Where the crate comes from, as the flags name it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CrateSource<'a> {
    /// The registry Cargo is configured with, which is crates.io by default.
    Registry,
    /// A git repository, at one reference or at its default branch.
    Git {
        url: &'a str,
        reference: Option<GitReference<'a>>,
    },
    /// A directory.
    Path(&'a Path),
}

/// The one reference of a git repository `--git` was given with.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GitReference<'a> {
    Branch(&'a str),
    Tag(&'a str),
    Revision(&'a str),
}

impl ImportArguments {
    /// The crate as it was typed: its name, and its version when `@` gave one.
    pub(crate) fn crate_spec(&self) -> &str {
        &self.crate_spec
    }

    /// The crate's name: everything before the first `@`.
    pub(crate) fn crate_name(&self) -> &str {
        crate_name_of(&self.crate_spec)
    }

    /// Where the crate comes from.
    pub(crate) fn source(&self) -> CrateSource<'_> {
        match (&self.git, &self.path) {
            (None, None) => {
                assert!(
                    self.git_reference().is_none(),
                    "clap refuses a branch, tag or commit without --git (`requires`)"
                );
                CrateSource::Registry
            }
            (Some(url), None) => CrateSource::Git {
                url,
                reference: self.git_reference(),
            },
            (None, Some(path)) => {
                assert!(
                    self.git_reference().is_none(),
                    "clap refuses a branch, tag or commit without --git (`requires`)"
                );
                CrateSource::Path(path)
            }
            (Some(_), Some(_)) => {
                unreachable!("clap refuses --path with --git while parsing (`conflicts_with`)")
            }
        }
    }

    /// The branch, tag or commit given, of which clap allows at most one.
    fn git_reference(&self) -> Option<GitReference<'_>> {
        match (&self.branch, &self.tag, &self.rev) {
            (None, None, None) => None,
            (Some(branch), None, None) => Some(GitReference::Branch(branch)),
            (None, Some(tag), None) => Some(GitReference::Tag(tag)),
            (None, None, Some(revision)) => Some(GitReference::Revision(revision)),
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => {
                unreachable!("clap refuses two of --branch, --tag and --rev (the one group)")
            }
        }
    }

    /// The key the imported task answers to: the one given, or the crate's
    /// name when none was.
    ///
    /// A crate name is not always a name a command can have (`my_crate` has an
    /// underscore), and the person gave no other, so that refusal says to give
    /// one and hands back the whole command to run with a suggestion in it.
    /// `import_command` is how this command line spells `import`, and
    /// `current_dir` is where the person typed it.
    pub(crate) fn key(&self, import_command: &str, current_dir: &Path) -> Result<Name, Failure> {
        if let Some(key) = &self.key {
            return Ok(Name::new(key)?);
        }
        Name::new(self.crate_name()).map_err(|unusable| {
            // A suggestion is offered only when it is itself a name a
            // command can have; otherwise the placeholder stands where the
            // key goes, written as `<name>` is in the refusal for being
            // outside the project, and so left unquoted.
            let key_text = suggested_key(self.crate_name()).map_or_else(
                || "<key>".to_string(),
                |suggestion| shell::join([suggestion.as_str()]),
            );
            refusals::unusable_default_key(
                &unusable,
                &format!(
                    "{import_command} {}",
                    self.command_words(current_dir, Some(key_text))
                ),
            )
        })
    }

    /// The words that run this import again, as a person copies them: the
    /// crate, the key if one was given, then where it comes from, with a
    /// relative `--path` made absolute against `current_dir`.
    ///
    /// The refusal for being outside the project tells the person to run the
    /// command from another directory, where a relative path would point
    /// somewhere else, so the path is the one it is here.
    pub(crate) fn to_run_again(&self, current_dir: &Path) -> String {
        let key_text = self.key.as_deref().map(|key| shell::join([key]));
        self.command_words(current_dir, key_text)
    }

    /// The crate, then `key_text` (already shell text, so a placeholder can
    /// stand in for a key), then where the crate comes from.
    fn command_words(&self, current_dir: &Path, key_text: Option<String>) -> String {
        let mut parts = vec![shell::join([self.crate_spec.as_str()])];
        parts.extend(key_text);
        let flags = self.source().flags(Some(current_dir));
        if !flags.is_empty() {
            parts.push(shell::join(flags));
        }
        parts.join(" ")
    }
}

impl CrateSource<'_> {
    /// The flags that say where the crate comes from, as words, in the order
    /// the grammar gives them: `--git`, its reference, then `--path`.
    ///
    /// `current_dir` of `None` leaves a `--path` as it was typed, which is
    /// what `cargo add` run in the person's own directory wants. `Some` joins
    /// it onto that directory, which is what a command to be run from
    /// somewhere else wants. A path that is not UTF-8 is not supported, and
    /// is written with replacement characters rather than refused.
    pub(crate) fn flags(&self, current_dir: Option<&Path>) -> Vec<String> {
        match self {
            CrateSource::Registry => Vec::new(),
            CrateSource::Git { url, reference } => {
                let mut flags = vec!["--git".to_string(), (*url).to_string()];
                if let Some(reference) = reference {
                    let (flag, value) = match reference {
                        GitReference::Branch(branch) => ("--branch", branch),
                        GitReference::Tag(tag) => ("--tag", tag),
                        GitReference::Revision(revision) => ("--rev", revision),
                    };
                    flags.extend([flag.to_string(), (*value).to_string()]);
                }
                flags
            }
            CrateSource::Path(path) => {
                let path = current_dir.map_or_else(|| path.to_path_buf(), |dir| dir.join(path));
                vec!["--path".to_string(), path.to_string_lossy().into_owned()]
            }
        }
    }
}

/// The text of `crate_spec` before its first `@`.
fn crate_name_of(crate_spec: &str) -> &str {
    crate_spec
        .split_once('@')
        .map_or(crate_spec, |(name, _version)| name)
}

/// The crate's name made into one a command can have, if that can be done:
/// lowercased, with `_` as `-`.
fn suggested_key(crate_name: &str) -> Option<Name> {
    Name::new(&crate_name.to_ascii_lowercase().replace('_', "-")).ok()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{CrateSource, GitReference, ImportArguments, crate_name_of};
    use crate::test_support::{TestOutcome, clap_refuses, import_command, typed_arguments};

    fn parsed(words: &[&str]) -> ImportArguments {
        typed_arguments(words)
    }

    fn refused(words: &[&str]) -> bool {
        clap_refuses(words)
    }

    const IMPORT: &str = "cargo ritual import";

    #[test]
    fn the_crate_name_is_what_comes_before_the_first_at_sign() {
        // Cargo's own grammar is `<crate>[@<version>]`; whatever follows the
        // first `@` is the version's, even if it holds another.
        let table = [
            ("a", "a"),
            ("a@1", "a"),
            ("a@1.2.3", "a"),
            ("a@1@2", "a"),
            ("@1", ""),
            ("", ""),
        ];
        for (spec, name) in table {
            assert_eq!(crate_name_of(spec), name, "for {spec:?}");
        }
    }

    #[test]
    fn a_crate_alone_is_taken_from_the_registry() {
        let arguments = parsed(&["greeter@0.1.0"]);

        assert_eq!(arguments.crate_spec(), "greeter@0.1.0");
        assert_eq!(arguments.crate_name(), "greeter");
        assert_eq!(arguments.source(), CrateSource::Registry);
    }

    #[test]
    fn each_source_flag_names_its_source() {
        assert_eq!(
            parsed(&["g", "--git", "https://x/g.git"]).source(),
            CrateSource::Git {
                url: "https://x/g.git",
                reference: None
            }
        );
        assert_eq!(
            parsed(&["g", "--git", "u", "--branch", "next"]).source(),
            CrateSource::Git {
                url: "u",
                reference: Some(GitReference::Branch("next"))
            }
        );
        assert_eq!(
            parsed(&["g", "--git", "u", "--tag", "v1"]).source(),
            CrateSource::Git {
                url: "u",
                reference: Some(GitReference::Tag("v1"))
            }
        );
        assert_eq!(
            parsed(&["g", "--git", "u", "--rev", "abc123"]).source(),
            CrateSource::Git {
                url: "u",
                reference: Some(GitReference::Revision("abc123"))
            }
        );
        assert_eq!(
            parsed(&["g", "--path", "../g"]).source(),
            CrateSource::Path(Path::new("../g"))
        );
    }

    #[test]
    fn the_flags_that_cannot_go_together_are_refused_while_parsing() {
        assert!(
            refused(&["g", "--path", "p", "--git", "u"]),
            "a path and a repository"
        );
        assert!(
            refused(&["g", "--branch", "b"]),
            "a branch with no repository"
        );
        assert!(refused(&["g", "--tag", "t"]), "a tag with no repository");
        assert!(refused(&["g", "--rev", "r"]), "a commit with no repository");
        assert!(
            refused(&["g", "--git", "u", "--branch", "b", "--tag", "t"]),
            "a branch and a tag"
        );
        assert!(
            refused(&["g", "--git", "u", "--tag", "t", "--rev", "r"]),
            "a tag and a commit"
        );
        assert!(refused(&["--path", "p"]), "no crate at all");
        assert!(refused(&["g", "k", "extra"]), "a third positional");
    }

    #[test]
    fn the_usage_line_and_help_say_what_each_argument_is() {
        let mut command = import_command();
        let usage = command.render_usage().to_string();
        let help = command.render_long_help().to_string();

        assert!(
            usage.contains("import [OPTIONS] <CRATE> [KEY]"),
            "usage was: {usage}"
        );
        for line in [
            "the task crate to import, as Cargo names it; add `@<version>` for a release other \
             than the newest",
            "the name the imported task will answer to; the crate's name when not given",
            "take the crate from a git repository instead of the registry",
            "the branch of the `--git` repository to take",
            "the tag of the `--git` repository to take",
            "the commit of the `--git` repository to take",
            "take the crate from a directory instead of the registry",
        ] {
            assert!(help.contains(line), "help lacks {line:?}; it was:\n{help}");
        }
        for value_name in [
            "--git <URL>",
            "--branch <BRANCH>",
            "--tag <TAG>",
            "--rev <REV>",
            "--path <DIR>",
        ] {
            assert!(
                help.contains(value_name),
                "help lacks {value_name}; it was:\n{help}"
            );
        }
    }

    #[test]
    fn the_key_is_the_one_given() -> TestOutcome {
        let key = parsed(&["greeter", "hail"]).key(IMPORT, Path::new("/work"))?;

        assert_eq!(key.as_str(), "hail");
        Ok(())
    }

    #[test]
    fn the_key_is_the_crates_name_when_none_is_given() -> TestOutcome {
        let key = parsed(&["greeter@0.1.0"]).key(IMPORT, Path::new("/work"))?;

        assert_eq!(key.as_str(), "greeter");
        Ok(())
    }

    #[test]
    fn a_key_that_is_not_a_usable_name_is_refused_as_the_name_is() {
        let result = parsed(&["greeter", "Hail"]).key(IMPORT, Path::new("/work"));

        assert!(result.is_err(), "expected `Hail` to be refused");
        if let Err(failure) = result {
            assert_eq!(
                failure.to_string(),
                rituals::Name::new("Hail")
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_default()
            );
        }
    }

    /// The suggestion in the refusal, if there is one, is the crate's name
    /// made into one a command can have, and the command shows where the
    /// key goes, even when the person typed the flags first.
    #[test]
    fn a_crate_name_that_is_not_a_usable_key_is_refused_with_the_command_to_run() {
        let table = [
            (
                parsed(&["my_crate"]),
                "`my_crate` is not a usable name; a name starts with a lowercase letter, \
                 continues with lowercase letters, digits and hyphens, and does not end with a \
                 hyphen; the key defaults to the crate's name, so give one: \
                 `cargo ritual import my_crate my-crate`",
            ),
            (
                parsed(&["Foo@1.0.0"]),
                "`Foo` is not a usable name; a name starts with a lowercase letter, continues \
                 with lowercase letters, digits and hyphens, and does not end with a hyphen; the \
                 key defaults to the crate's name, so give one: \
                 `cargo ritual import Foo@1.0.0 foo`",
            ),
            (
                parsed(&["9lives"]),
                "`9lives` is not a usable name; a name starts with a lowercase letter, continues \
                 with lowercase letters, digits and hyphens, and does not end with a hyphen; the \
                 key defaults to the crate's name, so give one: \
                 `cargo ritual import 9lives <key>`",
            ),
            (
                parsed(&["crate"]),
                "`crate` is not a usable name; `crate`, `self` and `super` cannot be written as \
                 a Rust identifier, and a command's name has to be one; the key defaults to the \
                 crate's name, so give one: `cargo ritual import crate <key>`",
            ),
            (
                parsed(&["--path", "tasks/my_crate", "my_crate"]),
                "`my_crate` is not a usable name; a name starts with a lowercase letter, \
                 continues with lowercase letters, digits and hyphens, and does not end with a \
                 hyphen; the key defaults to the crate's name, so give one: \
                 `cargo ritual import my_crate my-crate --path /work/tasks/my_crate`",
            ),
        ];
        for (arguments, expected) in table {
            let result = arguments.key(IMPORT, Path::new("/work"));

            assert!(result.is_err(), "expected {arguments:?} to be refused");
            if let Err(failure) = result {
                assert_eq!(failure.to_string(), expected);
            }
        }
    }

    #[test]
    fn a_hyphenated_crate_name_is_a_usable_key_as_it_is() -> TestOutcome {
        let key = parsed(&["hexlace-rituals"]).key(IMPORT, Path::new("/work"))?;

        assert_eq!(key.as_str(), "hexlace-rituals");
        Ok(())
    }

    #[test]
    fn the_words_to_run_again_follow_the_grammar_whatever_order_they_were_typed_in() {
        let arguments = parsed(&[
            "--tag",
            "v1",
            "--git",
            "https://example.com/x.git",
            "greeter@0.1.0",
            "hail",
        ]);

        assert_eq!(
            arguments.to_run_again(Path::new("/work")),
            "greeter@0.1.0 hail --git https://example.com/x.git --tag v1"
        );
        assert_eq!(
            parsed(&["greeter"]).to_run_again(Path::new("/work")),
            "greeter"
        );
    }

    #[test]
    fn a_relative_path_is_made_absolute_against_where_it_was_typed() {
        let arguments = parsed(&["greeter", "--path", "../greeter"]);

        assert_eq!(
            arguments.to_run_again(Path::new("/work/demo")),
            "greeter --path /work/demo/../greeter"
        );
        assert_eq!(
            parsed(&["greeter", "--path", "/elsewhere/greeter"]).to_run_again(Path::new("/work")),
            "greeter --path /elsewhere/greeter"
        );
    }

    #[test]
    fn a_path_with_a_space_is_quoted_so_it_pastes_as_one_argument() {
        let arguments = parsed(&["greeter", "--path", "my tasks/greeter"]);

        assert_eq!(
            arguments.to_run_again(Path::new("/work")),
            "greeter --path '/work/my tasks/greeter'"
        );
    }

    #[test]
    fn the_flags_leave_a_path_as_typed_or_join_it_to_a_directory() {
        let arguments = parsed(&["g", "--path", "../g"]);

        assert_eq!(arguments.source().flags(None), ["--path", "../g"]);
        assert_eq!(
            arguments.source().flags(Some(Path::new("/work"))),
            ["--path", "/work/../g"]
        );
        assert_eq!(parsed(&["g"]).source().flags(None), Vec::<String>::new());
        assert_eq!(
            parsed(&["g", "--git", "u", "--rev", "abc"])
                .source()
                .flags(None),
            ["--git", "u", "--rev", "abc"]
        );
    }
}
