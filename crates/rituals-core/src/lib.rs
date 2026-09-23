//! The management tasks ritual ships, grouped as one bundle.
//!
//! [`task()`] is the whole of this crate's public surface — exactly as it
//! is for a leaf, because a bundle is a task. Nothing distinguishes the
//! two: the manifest carries the ordinary
//! `[package.metadata.ritual] task = true`, the function is the ordinary
//! `task()`, and a command line imports this crate the ordinary way. That
//! sameness is the point, not an economy — it is what lets a project write
//! a bundle of its own and mount it beside this one.
//!
//! A pure group of the leaf task crates and nothing else. It adds no
//! capability they did not already have, and nothing of its own.
//!
//! The children are the packages `rituals-core-add`,
//! `rituals-core-regenerate`, `rituals-core-new` and `rituals-core-create`,
//! imported here under the names they answer to — `add`, `regenerate`,
//! `new` and `create` — the same way a composed command line imports any
//! task.

use rituals::Task;

/// This bundle, for a command line to mount under whatever name imports it
/// — always `ritual` on every composed command line ritual itself writes.
///
/// The order of the children is load-bearing. Mounted under a key equal to
/// the compiled binary's own name, this bundle is flattened at startup and
/// its children take its place at the top level *in this order*, which is
/// what makes `ritual --help` list `add`, `regenerate`, `new` and `create`
/// in that order. Reordering them changes that output.
//
// No `# Examples` section: a `task()` function has exactly one call-site
// shape — a mount line in a generated file — and an example here could only
// restate `let task = rituals_core::task();`, which shows nothing a reader
// needs. The real example is the generated file this repository checks in,
// `crates/rituals-cli/src/main.rs`.
//
// No assertions of its own either. The whole body is one `Task::group`
// call, which asserts three things at construction — at least one child,
// distinct names, no child called `help`. A second check here could only
// re-test `Task::group`'s own contract from outside it. The one property
// this function adds, the order of the children, is held by the unit test
// below and by the integration test that pins `ritual --help`.
#[must_use]
pub fn task() -> Task {
    Task::group(
        "maintain this project with ritual's own tasks",
        [
            ("add", add::task()),
            ("regenerate", regenerate::task()),
            ("new", new::task()),
            ("create", create::task()),
        ],
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_bundle_names_its_four_children_in_the_order_help_depends_on() {
        // Task::children is crate-internal to `rituals` by design, so
        // Task's own Debug impl — which exists to print
        // `children: [names]` — is the only door onto the order from
        // outside that crate. What breaks if this fails is `ritual --help`:
        // a bin-name-mounted bundle flattens its children into the top
        // level in exactly this order.
        let rendered = format!("{:?}", super::task());
        assert!(
            rendered.contains(r#"children: ["add", "regenerate", "new", "create"]"#),
            "expected the four children in flatten order; Debug was: {rendered}"
        );
    }
}
