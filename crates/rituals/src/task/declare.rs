//! Building the `clap::Command` tree for a task, leaf or bundle, without
//! recursion, so nesting depth never becomes stack depth.

use super::{Body, Task};

/// One `clap::Command` still being built, paired with an iterator over the
/// children it has not yet attached — the state [`declare_tree`]'s explicit
/// stack carries per level of nesting.
struct Frame<'a> {
    command: clap::Command,
    remaining: std::slice::Iter<'a, (&'static str, Task)>,
}

/// Builds the `clap::Command` `task` mounts under `name` — recursively in
/// shape, iteratively in implementation: a leaf's command is built and
/// returned directly, and a bundle's tree is built with an explicit stack
/// of [`Frame`]s, post-order, rather than a recursive call per level.
///
/// The loop has no fixed iteration limit because it does not need one: the
/// tree it walks is finite by construction. A [`Task`]'s
/// children are owned, not shared — `Task` is neither `Clone` nor reachable
/// behind an `Rc` — so no task can contain itself, and each iteration either
/// attaches one already-built leaf command or consumes one child from a
/// finite, already-built `Vec` (`Task::group` collects `children` into a
/// `Vec` before this function ever sees it). There is no path through this
/// loop that does not shrink the work left to do.
pub(super) fn declare_tree(task: &Task, name: &'static str) -> clap::Command {
    let children = match &task.body {
        Body::Handler { augment, .. } => return declare_leaf(*augment, task.about, name),
        Body::Children(children) => children,
    };

    let root = clap::Command::new(name)
        .about(task.about)
        .subcommand_required(true)
        .arg_required_else_help(true);
    let mut stack = vec![Frame {
        command: root,
        remaining: children.iter(),
    }];

    loop {
        let Some(frame) = stack.last_mut() else {
            // The loop only ever returns, below, before the stack is
            // popped down to empty.
            unreachable!("declare_tree's stack is never left empty without returning first");
        };

        let Some((child_name, child_task)) = frame.remaining.next() else {
            // This frame's children are all attached: pop it and attach its
            // finished command to its parent frame, or return it directly
            // when it was the root (the stack is now empty).
            let finished = stack
                .pop()
                .unwrap_or_else(|| unreachable!("just matched Some(frame) on this same stack"));
            let Some(parent) = stack.last_mut() else {
                return finished.command;
            };
            parent.command = std::mem::take(&mut parent.command).subcommand(finished.command);
            continue;
        };

        match &child_task.body {
            Body::Handler { augment, .. } => {
                let child_command = declare_leaf(*augment, child_task.about, child_name);
                frame.command = std::mem::take(&mut frame.command).subcommand(child_command);
            }
            Body::Children(grandchildren) => {
                let child_command = clap::Command::new(*child_name)
                    .about(child_task.about)
                    .subcommand_required(true)
                    .arg_required_else_help(true);
                stack.push(Frame {
                    command: child_command,
                    remaining: grandchildren.iter(),
                });
            }
        }
    }
}

/// Builds a leaf task's own `clap::Command`: its declared arguments, named
/// `name`, with `about` applied after `augment` — see [`super::Task::declare`]'s
/// doc comment for why the order matters.
fn declare_leaf(
    augment: fn(clap::Command) -> clap::Command,
    about: &'static str,
    name: &'static str,
) -> clap::Command {
    augment(clap::Command::new(name)).about(about)
}

#[cfg(test)]
mod tests {
    use super::declare_tree;
    use crate::task::Task;
    use crate::test_support::run_ok;

    #[test]
    fn declare_tree_on_a_leaf_is_unchanged_from_a_plain_command() {
        let task = Task::new("say hello", run_ok);
        let command = declare_tree(&task, "greet");
        assert_eq!(command.get_name(), "greet");
        assert_eq!(
            command.get_about().map(ToString::to_string),
            Some("say hello".to_string())
        );
        assert_eq!(command.get_subcommands().count(), 0);
    }

    /// A group's `clap::Command` lists its children as subcommands, in the
    /// order they were given to `Task::group`, each carrying its own about
    /// text — not the bundle's.
    #[test]
    fn declare_tree_on_a_group_lists_its_children_in_order_with_their_own_about() {
        let bundle = Task::group(
            "a bundle",
            [
                ("first", Task::new("the first child", run_ok)),
                ("second", Task::new("the second child", run_ok)),
            ],
        );
        let command = declare_tree(&bundle, "bundle");

        let names: Vec<&str> = command
            .get_subcommands()
            .map(clap::Command::get_name)
            .collect();
        assert_eq!(names, ["first", "second"]);

        let abouts: Vec<Option<String>> = command
            .get_subcommands()
            .map(|subcommand| subcommand.get_about().map(ToString::to_string))
            .collect();
        assert_eq!(
            abouts,
            [
                Some("the first child".to_string()),
                Some("the second child".to_string()),
            ]
        );
    }

    /// Nesting is not limited to one level: a grandchild's command is
    /// attached under its own parent, not under the outer bundle directly.
    #[test]
    fn declare_tree_on_a_two_level_group_puts_the_grandchild_under_the_child() {
        let inner = Task::group("inner bundle", [("leaf", Task::new("a leaf", run_ok))]);
        let outer = Task::group("outer bundle", [("inner", inner)]);
        let command = declare_tree(&outer, "outer");

        let top_names: Vec<&str> = command
            .get_subcommands()
            .map(clap::Command::get_name)
            .collect();
        assert_eq!(top_names, ["inner"]);

        let inner_command = command
            .get_subcommands()
            .find(|subcommand| subcommand.get_name() == "inner");
        assert!(inner_command.is_some(), "expected an `inner` subcommand");
        if let Some(inner_command) = inner_command {
            let grandchild_names: Vec<&str> = inner_command
                .get_subcommands()
                .map(clap::Command::get_name)
                .collect();
            assert_eq!(grandchild_names, ["leaf"]);
        }
    }
}
