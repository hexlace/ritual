//! Rituals kept in subdirectories of `.rituals/`, at any depth.
//!
//! A project may group its rituals however it likes beneath `.rituals/`, for
//! instance `.rituals/private/lint` for one it never publishes. `add` only
//! scaffolds `.rituals/<name>`, so a story that needs a ritual further down
//! writes it here, the way a person moving a directory by hand would.

use std::path::PathBuf;

use super::{Outcome, Project, TestOutcome, crates, manifest};

/// Writes a leaf ritual crate named `crate_name` at `directory`, a path from
/// the project root such as `.rituals/private/lint`, and makes it a
/// workspace member. Nothing mounts it on the command line yet.
pub(crate) fn write_ritual(
    project: &Project,
    directory: &str,
    crate_name: &str,
) -> Outcome<PathBuf> {
    let crate_dir = project.root().join(directory);
    crates::write_crate(
        &crate_dir,
        &crates::leaf_manifest(crate_name),
        &crates::leaf_lib(crate_name),
    )?;
    manifest::edit(&project.workspace_manifest_path(), |document| {
        manifest::push_member_on_its_own_line(document, directory)
    })?;
    Ok(crate_dir)
}

/// Gives the command line crate a dependency on the ritual at `directory`
/// and lists it among its tasks, under its crate name, without regenerating.
/// The command line crate sits one level below the root, as `new` makes it.
pub(crate) fn list_as_a_task(project: &Project, directory: &str, crate_name: &str) -> TestOutcome {
    manifest::edit(&project.cli_manifest_path(), |document| {
        manifest::add_path_dependency(
            document,
            &["dependencies"],
            crate_name,
            &format!("../{directory}"),
        )?;
        manifest::push_task(document, crate_name)
    })
}

/// [`write_ritual`] and [`list_as_a_task`], then `regenerate`, so the
/// command line carries the ritual as a command.
pub(crate) fn mounted_ritual(project: &Project, directory: &str, crate_name: &str) -> TestOutcome {
    write_ritual(project, directory, crate_name)?;
    list_as_a_task(project, directory, crate_name)?;
    project.run_cli(&["regenerate"])?.expect_success(&format!(
        "`regenerate` after mounting the ritual in {directory}"
    ));
    Ok(())
}
