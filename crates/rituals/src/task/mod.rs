//! What a task is: its one-line description, and either the clap arguments
//! it folds in and the function that runs it, or a group of named children.

mod declare;

use std::fmt;

use crate::command_line::CommandLine;
use crate::outcome::{Failure, Outcome};

/// One task: a one-line description, and either the command-line arguments
/// it declares the ordinary clap way and the function that runs them, or a
/// group of named children.
///
/// Built with [`Task::new`], with [`Task::receiving_command_line`] to also
/// be handed the command line it runs in, or with [`Task::group`] to build a
/// bundle: a task whose command is a group of named children.
///
/// A task is never named here. The name it answers to is the dependency key
/// of whichever project imports it, which is why the same crate can be
/// mounted twice under two different names. `Task::new("greet", run)` reads
/// as though the first argument names the task; it does not, it is the
/// one-line description shown in `--help`.
pub struct Task {
    about: &'static str,
    body: Body,
}

/// What a task actually does: run something, or group named children.
///
/// Private, so [`Task`] stays an opaque struct with private fields — a third
/// body could be added later without a
/// breaking change, and nothing outside this module needs to match on
/// which one a given task has. [`Task::declare`], [`Task::invoke`] and
/// [`Task::children`] are the only crate-internal doors onto it.
enum Body {
    /// A leaf: `augment` declares this task's own arguments on a fresh
    /// `clap::Command`, and `handler` runs it once clap has parsed them.
    Handler {
        augment: fn(clap::Command) -> clap::Command,
        handler: Box<Handler>,
    },
    /// A bundle: named children, in the order given to [`Task::group`],
    /// which is also where the invariants this variant relies on — at
    /// least one child, distinct names, none of them `help` — are
    /// enforced, once, at construction.
    Children(Vec<(&'static str, Task)>),
}

/// The shape both of [`Task`]'s leaf constructors box: the command line this
/// task is mounted in, and its own already-parsed arguments. Named so the
/// field above reads as a boxed value rather than tripping
/// `clippy::type_complexity` on the trait object it boxes.
type Handler = dyn Fn(&CommandLine, &clap::ArgMatches) -> Outcome + Send + Sync;

// `Task` is moved and stored by value throughout this framework (a bundle's
// own `Vec<(&'static str, Task)>` included), so its size is asserted here
// rather than merely hoped to stay small — a regression would silently
// inflate the cost of every move.
const _: () = assert!(std::mem::size_of::<Task>() <= 128);

/// Parses `matches` into `A`, the shape both of [`Task`]'s leaf constructors
/// share — the only difference between them is whether the command line
/// reaches the handler, not how arguments are parsed.
fn parse_arguments<A: clap::Args>(matches: &clap::ArgMatches) -> Result<A, Failure> {
    A::from_arg_matches(matches).map_err(|error| Failure::new(error.to_string()))
}

impl Task {
    /// Builds a task from its one-line description and the function that
    /// runs it.
    ///
    /// `about` is shown in `--help` and is not the task's name — see the
    /// type-level documentation. `A` is the argument struct a task author
    /// declares with `#[derive(rituals::clap::Args)]`; `run` receives it
    /// already parsed. A task built this way never sees the command line it
    /// is mounted in — use [`Task::receiving_command_line`] to opt in to
    /// that.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals::{Outcome, Task, clap, report};
    ///
    /// #[derive(clap::Args)]
    /// struct Arguments {
    ///     who: String,
    /// }
    ///
    /// fn run(arguments: Arguments) -> Outcome {
    ///     report(format!("hello, {}", arguments.who));
    ///     Ok(())
    /// }
    ///
    /// let task = Task::new("say hello to somebody", run);
    /// ```
    #[must_use]
    pub fn new<A>(about: &'static str, run: impl Fn(A) -> Outcome + Send + Sync + 'static) -> Self
    where
        A: clap::Args + 'static,
    {
        Self {
            about,
            body: Body::Handler {
                augment: <A as clap::Args>::augment_args,
                handler: Box::new(
                    move |_command_line: &CommandLine, matches: &clap::ArgMatches| {
                        let arguments = parse_arguments::<A>(matches)?;
                        run(arguments)
                    },
                ),
            },
        }
    }

    /// Builds a task whose handler also receives the command line it is
    /// mounted in, alongside its own parsed arguments.
    ///
    /// Use this when a task needs to name the command line it runs in, find
    /// that command line's own crate among a workspace's members, or see the
    /// commands at its top level. The [`CommandLine`] arrives each time the
    /// task runs, from whichever command line mounts it.
    ///
    /// To unit-test such a task, keep the closure handed to this constructor
    /// thin, taking from the command line only what the task needs, and
    /// test the function it calls.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals::{CommandLine, Outcome, Task, clap, report};
    ///
    /// #[derive(clap::Args)]
    /// struct Arguments {
    ///     who: String,
    /// }
    ///
    /// fn run(command_line: &CommandLine, arguments: Arguments) -> Outcome {
    ///     let identity = command_line.identity();
    ///     report(format!(
    ///         "hello, {} — from {} {}",
    ///         arguments.who,
    ///         identity.binary_name(),
    ///         identity.version()
    ///     ));
    ///     Ok(())
    /// }
    ///
    /// let task = Task::receiving_command_line("say hello to somebody", run);
    /// ```
    #[must_use]
    pub fn receiving_command_line<A>(
        about: &'static str,
        run: impl Fn(&CommandLine, A) -> Outcome + Send + Sync + 'static,
    ) -> Self
    where
        A: clap::Args + 'static,
    {
        Self {
            about,
            body: Body::Handler {
                augment: <A as clap::Args>::augment_args,
                handler: Box::new(
                    move |command_line: &CommandLine, matches: &clap::ArgMatches| {
                        let arguments = parse_arguments::<A>(matches)?;
                        run(command_line, arguments)
                    },
                ),
            },
        }
    }

    /// Builds a bundle: a task whose command is a group of named children.
    ///
    /// A bundle is imported and mounted exactly like any other task — the
    /// key it answers to comes from whoever imports it, the same as for a
    /// leaf — and bundles nest, because a child may itself be a bundle.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals::{Outcome, Task, clap, report};
    ///
    /// #[derive(clap::Args)]
    /// struct NoArguments {}
    ///
    /// fn run(_arguments: NoArguments) -> Outcome {
    ///     report("ran");
    ///     Ok(())
    /// }
    ///
    /// // A real bundle groups other crates' own `task()` functions, the
    /// // same way a generated file's own mount list does — written here
    /// // with `Task::new` inline instead, since a doctest cannot depend on
    /// // another crate.
    /// let bundle = Task::group(
    ///     "maintain this project",
    ///     [
    ///         ("add", Task::new("scaffold a task", run)),
    ///         ("regenerate", Task::new("rewrite the generated task list", run)),
    ///     ],
    /// );
    /// ```
    ///
    /// # Panics
    ///
    /// Panics when `children` is empty, when it names one command twice, or
    /// when it names a command `help`: clap adds a `help` command under
    /// every group, so a child called that is two commands with one name.
    #[must_use]
    pub fn group(
        about: &'static str,
        children: impl IntoIterator<Item = (&'static str, Self)>,
    ) -> Self {
        let children: Vec<(&'static str, Self)> = children.into_iter().collect();

        // A bundle with no children can never run anything — the same
        // mistake as a task with no way to be invoked at all.
        assert!(
            !children.is_empty(),
            "a bundle must have at least one child; `Task::group` was given none"
        );

        for (index, (name, _)) in children.iter().enumerate() {
            // Two children of one bundle with one name is a bug in the
            // bundle crate's own compiled source, with no user input
            // involved, so it panics — caught here, at every depth and in
            // both build profiles, rather than left to clap's own
            // duplicate-subcommand check, which is a debug assertion: a
            // release build silently keeps the first of the two.
            assert!(
                !children[..index].iter().any(|(earlier, _)| earlier == name),
                "a bundle's children must have distinct names; `{name}` is named twice"
            );
        }

        for (name, _) in &children {
            // clap adds its own `help` command under every group that has
            // subcommands, so a child called that is two commands with one
            // name — the same collision a top-level `help` is refused for
            // at startup.
            assert!(
                *name != "help",
                "a bundle's children cannot be named `help`: clap adds its own `help` command \
                 under every group"
            );
        }

        Self {
            about,
            body: Body::Children(children),
        }
    }

    /// Builds the `clap::Command` this task mounts under `name` — a leaf's
    /// own command when this task was built with [`Task::new`] or
    /// [`Task::receiving_command_line`], or the whole subtree of a bundle's
    /// children when it was built with [`Task::group`].
    ///
    /// `about` is applied *after* `augment`, not before: `#[derive(clap::Args)]`
    /// generates an `augment_args` that applies the argument struct's own doc
    /// comment as the command's about text, unconditionally, as the very last
    /// thing it does — so calling `.about()` before augmenting has no effect
    /// once the struct carries a doc comment, which every scaffolded task's
    /// `Arguments` does. Applying it after is what makes `Task::new`'s
    /// `about` argument the one that actually reaches `--help`.
    ///
    /// Crate-internal: only the dispatcher needs to turn a task into a
    /// `clap::Command`; it is not part of what a task author reads.
    #[must_use]
    pub(crate) fn declare(&self, name: &'static str) -> clap::Command {
        declare::declare_tree(self, name)
    }

    /// Parses `matches` into this task's arguments and runs it, handing
    /// `command_line` to the handler only when the task was built with
    /// [`Task::receiving_command_line`] — a task built with [`Task::new`]
    /// ignores it.
    ///
    /// Crate-internal: only the dispatcher's descend loop reaches this, and
    /// only ever with a leaf — [`Task::children`] is what a bundle is walked
    /// with instead.
    ///
    /// # Errors
    ///
    /// Returns the handler's own [`Failure`], or one built from a clap parse
    /// error on `matches`. `invoke` only ever receives the matches produced
    /// by parsing against the `clap::Command` [`Task::declare`] built for the
    /// same task, so a derived argument struct always parses; a hand-written
    /// `clap::Args` impl whose `from_arg_matches` disagrees with its own
    /// `augment_args` can still fail here, and that is the task's defect, so
    /// it is reported as the task's refusal rather than stopping the
    /// dispatcher.
    ///
    /// # Panics
    ///
    /// Panics if called on a task built with [`Task::group`] — the
    /// dispatcher's descend loop never does this; see [`Task::children`].
    pub(crate) fn invoke(&self, command_line: &CommandLine, matches: &clap::ArgMatches) -> Outcome {
        match &self.body {
            Body::Handler { handler, .. } => handler(command_line, matches),
            Body::Children(_) => {
                unreachable!("the dispatcher's descend loop never invokes a bundle directly")
            }
        }
    }

    /// This task's children, when it was built with [`Task::group`] — `None`
    /// for a task built with [`Task::new`] or [`Task::receiving_command_line`].
    ///
    /// Crate-internal: only the dispatcher's descend loop needs to walk a
    /// bundle's children by name; [`Task`] stays opaque to everyone else.
    pub(crate) fn children(&self) -> Option<&[(&'static str, Self)]> {
        match &self.body {
            Body::Children(children) => Some(children),
            Body::Handler { .. } => None,
        }
    }

    /// Consumes this task and returns its children, when it was built with
    /// [`Task::group`] — or gives this same task back, unconsumed, when it
    /// was not.
    ///
    /// Crate-internal: only the dispatcher's flatten step needs to take a
    /// bundle's children by value, to splice them into a composed command
    /// line's top level in the mount's own place.
    pub(crate) fn into_children(self) -> Result<Vec<(&'static str, Self)>, Self> {
        match self.body {
            Body::Children(children) => Ok(children),
            Body::Handler { .. } => Err(self),
        }
    }
}

impl fmt::Debug for Task {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug_struct = formatter.debug_struct("Task");
        debug_struct.field("about", &self.about);
        if let Body::Children(children) = &self.body {
            let child_names: Vec<&'static str> = children.iter().map(|(name, _)| *name).collect();
            debug_struct.field("children", &child_names);
        }
        debug_struct.finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::Task;
    use crate::command_line::CommandLine;
    use crate::identity::Identity;
    use crate::outcome::Outcome;
    use crate::test_support::run_ok;

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[test]
    fn task_is_send_and_sync() {
        assert_send::<Task>();
        assert_sync::<Task>();
    }

    /// An arbitrary command line for tests that need one to invoke a task
    /// with — not any real composed CLI's own command line, and its
    /// identity's three values are deliberately distinct so a test that
    /// reads the wrong field back would notice.
    fn a_command_line() -> CommandLine {
        CommandLine::from_dispatch(
            Identity::from_macro_expansion("test-package", "test-binary", "0.0.0-test"),
            [],
        )
    }

    #[test]
    fn declare_carries_the_about_text_onto_the_command() {
        let task = Task::new("say hello", run_ok);
        let command = task.declare("greet");
        assert_eq!(command.get_name(), "greet");
        assert_eq!(
            command.get_about().map(ToString::to_string),
            Some("say hello".to_string())
        );
    }

    /// Every scaffolded task's `Arguments` struct carries a doc comment
    /// (`/// What this task accepts on the command line.`), which
    /// `#[derive(clap::Args)]` turns into an `about` call of its own,
    /// applied last inside `augment_args` — so a test double with no doc
    /// comment at all, like `NoArguments`, cannot catch `declare` getting
    /// the ordering backwards and silently losing `Task::new`'s own
    /// `about` text to it.
    ///
    /// a doc comment that must never win over `Task::new`'s about text
    #[derive(clap::Args)]
    struct DocCommentedArguments {}

    #[expect(
        clippy::unnecessary_wraps,
        reason = "the handler contract is `Fn(A) -> Outcome`; this test double never fails on \
                  purpose, but the signature still has to match what a real handler returns"
    )]
    fn run_doc_commented(_arguments: DocCommentedArguments) -> Outcome {
        Ok(())
    }

    #[test]
    fn declare_prefers_task_news_about_over_the_argument_structs_doc_comment() {
        let task = Task::new("say hello", run_doc_commented);
        let command = task.declare("greet");
        assert_eq!(
            command.get_about().map(ToString::to_string),
            Some("say hello".to_string())
        );
    }

    #[test]
    fn invoke_runs_the_handler_against_parsed_arguments() {
        let task = Task::new("say hello", run_ok);
        let command = task.declare("greet");
        let parsed = command.try_get_matches_from(["greet"]);
        assert!(
            parsed.is_ok(),
            "parsing an empty argument list should succeed: {parsed:?}"
        );

        if let Ok(matches) = parsed {
            assert!(task.invoke(&a_command_line(), &matches).is_ok());
        }
    }

    /// A task built with [`Task::new`] receives only its parsed arguments.
    /// `run_ok` never reads a command line at all, so the property under
    /// test is that `invoke` accepts one and the plain-built task still
    /// runs regardless of which command line it is handed.
    #[test]
    fn a_task_built_the_plain_way_ignores_the_command_line_it_is_invoked_with() {
        let task = Task::new("say hello", run_ok);
        let command = task.declare("greet");
        let parsed = command.try_get_matches_from(["greet"]);
        assert!(
            parsed.is_ok(),
            "parsing an empty argument list should succeed: {parsed:?}"
        );

        if let Ok(matches) = parsed {
            let other_command_line = CommandLine::from_dispatch(
                Identity::from_macro_expansion("other-package", "other-binary", "9.9.9"),
                ["build"],
            );
            let first = task.invoke(&a_command_line(), &matches);
            let second = task.invoke(&other_command_line, &matches);
            assert!(first.is_ok());
            assert!(second.is_ok());
        }
    }

    #[derive(clap::Args)]
    struct GreetingArguments {
        who: String,
    }

    /// A task built with [`Task::receiving_command_line`] is handed the
    /// exact command line `invoke` was called with — asserted on the
    /// identity's three accessors *and* on the flattened list, so a
    /// transposition between fields would be caught here.
    #[test]
    fn receiving_command_line_hands_the_handler_the_invoking_command_line() {
        let task = Task::receiving_command_line(
            "say hello, naming the CLI",
            |command_line: &CommandLine, arguments: GreetingArguments| {
                let identity = command_line.identity();
                assert_eq!(identity.package_name(), "test-package");
                assert_eq!(identity.binary_name(), "test-binary");
                assert_eq!(identity.version(), "0.0.0-test");
                assert_eq!(command_line.flattened_commands(), ["daily"]);
                assert_eq!(arguments.who, "world");
                Ok(())
            },
        );
        let command = task.declare("greet");
        let parsed = command.try_get_matches_from(["greet", "world"]);
        assert!(
            parsed.is_ok(),
            "parsing a single positional argument should succeed: {parsed:?}"
        );

        if let Ok(matches) = parsed {
            let command_line = CommandLine::from_dispatch(
                Identity::from_macro_expansion("test-package", "test-binary", "0.0.0-test"),
                ["daily"],
            );
            assert!(task.invoke(&command_line, &matches).is_ok());
        }
    }

    #[test]
    #[should_panic(expected = "a bundle must have at least one child")]
    fn group_panics_on_no_children() {
        let _ = Task::group("empty", std::iter::empty::<(&'static str, Task)>());
    }

    #[test]
    #[should_panic(expected = "`add` is named twice")]
    fn group_panics_on_a_repeated_child_name() {
        let _ = Task::group(
            "a bundle with a duplicate",
            [
                ("add", Task::new("first", run_ok)),
                ("add", Task::new("second", run_ok)),
            ],
        );
    }

    #[test]
    #[should_panic(expected = "cannot be named `help`")]
    fn group_panics_on_a_child_named_help() {
        let _ = Task::group("a bundle with help", [("help", Task::new("nope", run_ok))]);
    }

    /// A bundle with no duplicate and no `help` child builds without
    /// panicking — the positive-space companion to the three panic tests
    /// above, so the checks are shown to accept good input, not only
    /// reject bad input.
    #[test]
    fn group_builds_a_well_formed_bundle_without_panicking() {
        let bundle = Task::group(
            "a well-formed bundle",
            [
                ("add", Task::new("first", run_ok)),
                ("regenerate", Task::new("second", run_ok)),
            ],
        );
        assert_eq!(bundle.children().map(<[_]>::len), Some(2));
    }
}
