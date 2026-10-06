//! A project whose command line crate is the workspace root: one
//! `Cargo.toml` holding both the `[package]` of the command line and the
//! `[workspace]`.

use super::{OptionContext, Outcome, Project, ResultContext, manifest, write_text};

/// Rebuilds `project`, which `new` scaffolded with its command line crate in
/// `ritual/`, so that crate is the workspace root: the command line's
/// manifest gains the workspace tables and an empty member list, its
/// `src/main.rs` moves up, and `ritual/` goes. Returns the project as it now
/// is, opened afresh, because where its command line crate lives changed.
pub(crate) fn fold_the_command_line_into_the_root(project: &Project) -> Outcome<Project> {
    let root = project.root().to_path_buf();
    let workspace = project.workspace_manifest()?;
    let mut folded = project.cli_manifest()?;
    folded["workspace"] = workspace
        .get("workspace")
        .context("the scaffolded workspace manifest has a [workspace] table")?
        .clone();
    manifest::set_strings(&mut folded, &["workspace", "members"], &[])?;
    let folded_text = folded.to_string();

    let old_directory = project.composed_cli_dir().to_path_buf();
    std::fs::create_dir(root.join("src")).context("creating the root's src/")?;
    std::fs::rename(old_directory.join("src/main.rs"), root.join("src/main.rs"))
        .context("moving the generated command line up to the root")?;
    std::fs::remove_dir_all(&old_directory).context("removing the old command line crate")?;
    write_text(&root.join("Cargo.toml"), &folded_text)?;
    Project::open(&root)
}
