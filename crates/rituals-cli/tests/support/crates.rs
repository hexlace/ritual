//! The source of hand-written task crates: leaves, and bundles over them.
//!
//! Rendered as plain text rather than through `rituals_compose`: no story in
//! this suite depends on ritual's crates as libraries; every one drives
//! `ritual` the way an outside caller would. These are the only place a
//! story spells `rituals::Task::new` or `rituals::Task::group`, so a change
//! to either is a change here, once.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use super::{ResultContext, TestOutcome};

/// One child of a hand-written bundle.
#[derive(Clone, Copy)]
pub(crate) enum Child<'name> {
    /// A separate leaf crate, `crate_name`, mounted under `key` — a path
    /// dependency of the bundle.
    Crate {
        key: &'name str,
        crate_name: &'name str,
    },
    /// A leaf declared inside the bundle's own source, mounted under `key`,
    /// that reports `<key> ran` — for a story that needs a name in the
    /// bundle and nothing else about the child.
    Inline { key: &'name str },
}

/// The line a hand-written leaf mounted under `name` prints when it runs —
/// a leaf crate by its crate name, an inline leaf by its key.
pub(crate) fn ran_line(name: &str) -> String {
    format!("{name} ran")
}

/// Renders a hand-written leaf task crate's `Cargo.toml`: the shape `add`
/// scaffolds — a workspace-inherited dependency on `rituals` and nothing
/// else — and `[package.metadata.ritual] task = true`.
pub(crate) fn leaf_manifest(crate_name: &str) -> String {
    let mut output = String::new();
    output.push_str("[package]\n");
    let _ = writeln!(output, "name = \"{crate_name}\"");
    output.push_str("version = \"0.1.0\"\n");
    output.push_str("edition = \"2024\"\n");
    output.push('\n');
    output.push_str("[dependencies]\n");
    output.push_str("rituals.workspace = true\n");
    output.push('\n');
    output.push_str("[package.metadata.ritual]\n");
    output.push_str("task = true\n");
    output
}

/// Renders a hand-written leaf task crate's `src/lib.rs`: a
/// `rituals::Task::new` whose handler prints [`ran_line`] for `crate_name`,
/// so a story dispatching through several levels of nesting can tell which
/// leaf ran from stdout, not merely that dispatch exited zero.
pub(crate) fn leaf_lib(crate_name: &str) -> String {
    let mut output = String::new();
    output.push_str("//! A hand-written leaf task crate, built for this test suite.\n\n");
    output.push_str("use rituals::{Outcome, Task, clap, report};\n\n");
    output.push_str("/// What this task accepts on the command line — nothing.\n");
    output.push_str("#[derive(clap::Args)]\n");
    output.push_str("struct Arguments {}\n\n");
    output.push_str("/// This task, for a command line to mount under whatever name imports it.\n");
    output.push_str("#[must_use]\n");
    output.push_str("pub fn task() -> Task {\n");
    output.push_str("    Task::new(\"a leaf task built for this test suite\", run)\n");
    output.push_str("}\n\n");
    output.push_str("fn run(_arguments: Arguments) -> Outcome {\n");
    let _ = writeln!(output, "    report({:?});", ran_line(crate_name));
    output.push_str("    Ok(())\n");
    output.push_str("}\n");
    output
}

/// Renders a hand-written bundle crate's `Cargo.toml`: the same single
/// framework dependency every leaf has, `rituals`, plus one path dependency
/// per [`Child::Crate`] — keyed by the child's crate name, whatever key the
/// bundle mounts it under.
pub(crate) fn bundle_manifest(crate_name: &str, children: &[Child<'_>]) -> String {
    let mut output = String::new();
    output.push_str("[package]\n");
    let _ = writeln!(output, "name = \"{crate_name}\"");
    output.push_str("version = \"0.1.0\"\n");
    output.push_str("edition = \"2024\"\n");
    output.push('\n');
    output.push_str("[dependencies]\n");
    output.push_str("rituals.workspace = true\n");
    for child in children {
        if let Child::Crate { crate_name, .. } = child {
            let _ = writeln!(output, "{crate_name} = {{ path = \"../{crate_name}\" }}");
        }
    }
    output.push('\n');
    output.push_str("[package.metadata.ritual]\n");
    output.push_str("task = true\n");
    output
}

/// Renders a hand-written bundle crate's `src/lib.rs`: a task built with
/// `rituals::Task::group`, described by `about`, grouping `children` in the
/// order given.
pub(crate) fn bundle_lib(about: &str, children: &[Child<'_>]) -> String {
    let inline_keys: Vec<&str> = children
        .iter()
        .filter_map(|child| match child {
            Child::Inline { key } => Some(*key),
            Child::Crate { .. } => None,
        })
        .collect();

    let mut output = String::new();
    output.push_str("//! A hand-written bundle crate, built for this test suite.\n\n");
    if inline_keys.is_empty() {
        output.push_str("use rituals::Task;\n\n");
    } else {
        output.push_str("use rituals::{Outcome, Task, clap, report};\n\n");
        output.push_str("/// What an inline leaf accepts on the command line — nothing.\n");
        output.push_str("#[derive(clap::Args)]\n");
        output.push_str("struct Arguments {}\n\n");
    }
    output.push_str("/// This task, for a command line to mount under whatever name imports it.\n");
    output.push_str("#[must_use]\n");
    output.push_str("pub fn task() -> Task {\n");
    output.push_str("    Task::group(\n");
    let _ = writeln!(output, "        {about:?},");
    output.push_str("        [\n");
    for child in children {
        let _ = writeln!(output, "            {},", group_entry(child));
    }
    output.push_str("        ],\n");
    output.push_str("    )\n");
    output.push_str("}\n");
    for key in inline_keys {
        output.push('\n');
        output.push_str(&inline_handler(key));
    }
    output
}

/// The name of the handler function an inline leaf under `key` runs.
fn inline_handler_name(key: &str) -> String {
    format!("run_{}", key.replace('-', "_"))
}

/// One `(key, task)` entry of a bundle's `Task::group` list.
fn group_entry(child: &Child<'_>) -> String {
    match child {
        Child::Crate { key, crate_name } => {
            format!("({key:?}, {}::task())", crate_name.replace('-', "_"))
        }
        Child::Inline { key } => format!(
            "({key:?}, Task::new(\"an inline leaf built for this test suite\", {}))",
            inline_handler_name(key)
        ),
    }
}

/// The handler an inline leaf under `key` runs: it prints [`ran_line`] for
/// `key`.
fn inline_handler(key: &str) -> String {
    format!(
        "fn {}(_arguments: Arguments) -> Outcome {{\n    report({:?});\n    Ok(())\n}}\n",
        inline_handler_name(key),
        ran_line(key)
    )
}

/// Writes `crate_dir/Cargo.toml` and `crate_dir/src/lib.rs`, creating
/// `crate_dir/src` first — the two files every hand-written crate in this
/// suite is made of, the same pair `add` writes for a plain task crate.
pub(crate) fn write_crate(crate_dir: &Path, manifest: &str, lib: &str) -> TestOutcome {
    let source_directory = crate_dir.join("src");
    fs::create_dir_all(&source_directory)
        .context(&format!("creating {} failed", source_directory.display()))?;
    fs::write(crate_dir.join("Cargo.toml"), manifest)
        .context("writing the crate's Cargo.toml failed")?;
    fs::write(source_directory.join("lib.rs"), lib)
        .context("writing the crate's src/lib.rs failed")?;
    Ok(())
}
