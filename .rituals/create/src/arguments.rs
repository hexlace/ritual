//! What a person types after `create`: the ritual's name or path, and who it
//! is for.
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

use std::path::Path;

use rituals::clap;
use rituals_compose::shell;
use rituals_compose::task_crate::Audience;

/// The arguments of a task that scaffolds a ritual: what to call it, and who
/// it is for.
///
/// `create` flattens these in beside where an outside-the-project crate takes
/// `rituals` from. The deprecated `add` takes these alone, because inside a
/// project every ritual inherits the workspace's `rituals`.
#[derive(clap::Args, Clone, Debug, PartialEq, Eq)]
pub struct ScaffoldArguments {
    /// the ritual's name, or a path below .rituals/ whose last component is
    /// its name
    #[arg(value_name = "NAME|PATH")]
    name_or_path: String,

    /// make a ritual that can be published, with no `publish = false`
    #[arg(long)]
    public: bool,
}

/// The positional argument, read as what it says: a bare name, or a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NameOrPath<'a> {
    /// No `/` in it: a name, placed directly in `.rituals/`.
    Name(&'a str),
    /// A `/` in it: a path, read from the directory it was typed in.
    Path(&'a Path),
}

impl ScaffoldArguments {
    /// Who the ritual is for: private unless `--public` was given.
    pub(crate) const fn audience(&self) -> Audience {
        if self.public {
            Audience::Public
        } else {
            Audience::Private
        }
    }

    /// The positional argument as a name when it holds no `/`, and as a path
    /// when it does.
    pub(crate) fn name_or_path(&self) -> NameOrPath<'_> {
        if self.name_or_path.contains('/') {
            NameOrPath::Path(Path::new(&self.name_or_path))
        } else {
            NameOrPath::Name(&self.name_or_path)
        }
    }

    /// The words that run this again, as a person types them after
    /// `create`: the argument as it was typed, then `--public` when it was
    /// given. A remedy and a retry name them, so they paste as written.
    ///
    /// A path stays as typed: it is read against whichever project the
    /// person is in when they run it again.
    pub(crate) fn to_run_again(&self) -> String {
        let mut words = vec![self.name_or_path.as_str()];
        if self.public {
            words.push("--public");
        }
        shell::join(words)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rituals::clap::{self, Parser};

    use super::{NameOrPath, ScaffoldArguments};
    use rituals_compose::task_crate::Audience;

    #[derive(clap::Parser)]
    struct Command {
        #[command(flatten)]
        scaffold: ScaffoldArguments,
    }

    fn parsed(arguments: &[&str]) -> ScaffoldArguments {
        let mut words = vec!["create"];
        words.extend_from_slice(arguments);
        match Command::try_parse_from(words) {
            Ok(command) => command.scaffold,
            Err(error) => unreachable!("a test passes only arguments that parse: {error}"),
        }
    }

    #[test]
    fn a_value_without_a_slash_is_a_name() {
        assert_eq!(parsed(&["lint"]).name_or_path(), NameOrPath::Name("lint"));
        assert_eq!(
            parsed(&["code-lint"]).name_or_path(),
            NameOrPath::Name("code-lint")
        );
    }

    #[test]
    fn a_value_with_a_slash_is_a_path() {
        for typed in [
            ".rituals/private/lint",
            "private/lint",
            "./lint",
            "../lint",
            ".rituals/",
        ] {
            assert_eq!(
                parsed(&[typed]).name_or_path(),
                NameOrPath::Path(Path::new(typed)),
                "{typed}"
            );
        }
    }

    #[test]
    fn a_ritual_is_private_unless_public_is_given() {
        assert_eq!(parsed(&["lint"]).audience(), Audience::Private);
        assert_eq!(parsed(&["lint", "--public"]).audience(), Audience::Public);
        assert_eq!(parsed(&["--public", "lint"]).audience(), Audience::Public);
    }

    #[test]
    fn the_words_to_run_again_are_the_argument_as_typed_then_the_flag() {
        assert_eq!(parsed(&["lint"]).to_run_again(), "lint");
        assert_eq!(
            parsed(&["lint", "--public"]).to_run_again(),
            "lint --public"
        );
        assert_eq!(
            parsed(&["--public", "lint"]).to_run_again(),
            "lint --public"
        );
        assert_eq!(
            parsed(&[".rituals/private/lint"]).to_run_again(),
            ".rituals/private/lint"
        );
    }

    /// The path is typed into a shell, so one with a space in it is quoted
    /// the way the shell will read it back.
    #[test]
    fn a_path_with_a_space_is_quoted_so_the_remedy_pastes_as_written() {
        assert_eq!(parsed(&["my tasks/lint"]).to_run_again(), "'my tasks/lint'");
    }
}
