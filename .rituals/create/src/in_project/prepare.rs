//! Deciding everything `create` needs before anything is written, inside a
//! project.
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

use rituals::{CommandLine, Failure};
use rituals_compose::generated_file::TaskKey;
use rituals_compose::layout::{self, TaskPlace, TypedPath};
use rituals_compose::manifest::{self, ManifestPaths, Manifests};
use rituals_compose::metadata::{self, Metadata, Project};
use rituals_compose::rollback::Changes;
use rituals_compose::task_crate::Audience;
use rituals_compose::top_level;

use super::refusals::{self, RemedyCommands};
use super::scaffolding::Scaffolding;

/// What a person asked `create` to make, as far as it can be known without
/// the project.
///
/// A bare name is validated before the run begins, since it needs nothing
/// from the project. A path is validated inside the run, by
/// [`layout::place_at`], because which component is last depends on folding
/// it against the directory it was typed in, and placing it needs the
/// workspace root.
#[derive(Debug)]
pub(crate) enum Requested<'a> {
    /// A bare name, already a valid key.
    Name(TaskKey),
    /// A path as typed.
    Path(&'a Path),
}

/// Everything [`prepare`] reads that a person typed or the process was
/// given, none of it from the project.
pub(crate) struct Request<'a> {
    pub(crate) command_line: &'a CommandLine,
    pub(crate) current_dir: &'a Path,
    pub(crate) requested: Requested<'a>,
    pub(crate) audience: Audience,
    /// The words after `create` that run this again, for a remedy.
    pub(crate) run_again: &'a str,
}

/// Validates everything about the project and the name, refusing at the
/// first thing that is wrong, and returns what the writes need.
///
/// This runs inside the caller's [`rituals_compose::rollback::attempt`], from
/// the first subprocess on, because `cargo metadata` creates the lockfile of a
/// project that has none and rewrites a stale one:
/// [`metadata::fetch_in_its_own_project`] records it through `changes` before
/// asking, and a refusal anywhere after puts it back. The checks run in this
/// order, each only once the one before it has passed:
///
/// 1. That this is the project the running command line belongs to, before
///    any check on the name against the command line: the running binary's
///    own top level says something about the project only once the binary
///    is known to be the project's.
/// 2. Where the task goes, which is also what its name is.
/// 3. That the name is not the bin's own name, which the check after it
///    deliberately lets through, and then that it is not already a top-level
///    command. The bin-name check has to come first: that check lets the
///    bin's own name through, because `regenerate` shares it, and a
///    bin-name-mounted bundle's own manifest entry has exactly that key.
///    It also has to come before the already-imported check: in a
///    default-scaffolded project the bundle is already imported under that
///    key, and that refusal would otherwise answer a different question
///    with the wrong remedy.
/// 4. That the name is not already spoken for in the command line's
///    manifest, that its directory is not a leftover, and that no workspace
///    member is already called by it. A refusal about the name itself comes
///    before the one about the project's existing task list, so a leftover
///    directory actually in the way is never masked by an unrelated entry
///    elsewhere in the list; both are pure reads.
/// 5. That the project's existing task list resolves, since `create` ends
///    by regenerating, and that the workspace manifest can take the new
///    member.
pub(crate) fn prepare(changes: &mut Changes, request: Request<'_>) -> Result<Scaffolding, Failure> {
    let package = request.command_line.identity().package_name();
    let document = metadata::fetch_in_its_own_project(
        changes,
        request.current_dir,
        package,
        "create",
        request.run_again,
    )?;
    let project = document.locate_project(package)?;
    let (name, place) =
        place_the_task(&document, &project, request.current_dir, request.requested)?;

    let regenerate = top_level::management_command(request.command_line, "regenerate");
    ensure_the_name_is_free(request.command_line, &project, &name, &regenerate)?;
    let dependency_path = manifest::dependency_path(project.manifest_path(), place.directory());
    let leftover = Leftover {
        package,
        dependency_path: &dependency_path,
        regenerate: &regenerate,
    };
    ensure_nothing_is_in_the_way(&document, &project, &name, &place, &leftover)?;

    // The same resolver `regenerate` runs, against the metadata already
    // fetched above — no second `cargo metadata` call. Its refusal is
    // returned as-is: it already says what is wrong and what to do.
    document.resolve_task_list(package)?;

    let manifests = Manifests::read(&ManifestPaths::of(&project))?;
    refusals::ensure_workspace_can_take_the_import(manifests.workspace())?;

    Ok(Scaffolding {
        name,
        place,
        audience: request.audience,
        manifests,
        dependency_path,
        workspace_root: project.workspace_root().to_path_buf(),
    })
}

/// Where the task goes, and the key it is listed under: a bare name is placed
/// directly in the tasks directory, whatever directory it was typed in, and a
/// path is read from the directory it was typed in and refused when it leads
/// into a ritual that is already there.
fn place_the_task(
    document: &Metadata,
    project: &Project<'_>,
    current_dir: &Path,
    requested: Requested<'_>,
) -> Result<(TaskKey, TaskPlace), Failure> {
    match requested {
        Requested::Name(key) => {
            let place = layout::place_for(project.workspace_root(), key.as_name());
            Ok((key, place))
        }
        Requested::Path(path) => {
            let place =
                layout::place_at(project.workspace_root(), TypedPath { current_dir, path })?;
            layout::ensure_in_no_member(
                project.workspace_root(),
                path,
                &place,
                document
                    .workspace_members()
                    .into_iter()
                    .map(|member| (member.package_name(), member.directory())),
            )?;
            let key = TaskKey::new(place.name().clone())?;
            Ok((key, place))
        }
    }
}

/// Refuses a name the running command line cannot take: the bin's own name,
/// a name that is already a top-level command, or one the manifest has
/// already spoken for.
fn ensure_the_name_is_free(
    command_line: &CommandLine,
    project: &Project<'_>,
    name: &TaskKey,
    regenerate: &str,
) -> Result<(), Failure> {
    refusals::ensure_the_name_is_not_the_bin_name(
        name.as_name(),
        command_line.identity().binary_name(),
    )?;
    top_level::ensure_command_is_free(command_line, name.as_str())?;
    refusals::already_imported_refusal(
        command_line.identity().package_name(),
        name.as_name(),
        project.declares_dependency_key(name.as_name()),
        project.lists_task(name.as_name()),
        &RemedyCommands {
            regenerate,
            import: &top_level::management_command(command_line, "import"),
        },
    )
}

/// What a refusal of a leftover task directory says to do about it.
struct Leftover<'a> {
    /// The composed command line's package.
    package: &'a str,
    /// The `path` its dependency on the leftover crate would have.
    dependency_path: &'a str,
    /// How a person types this command line's `regenerate`.
    regenerate: &'a str,
}

/// Refuses a task directory that already exists, and a package name a
/// workspace member already has.
fn ensure_nothing_is_in_the_way(
    document: &Metadata,
    project: &Project<'_>,
    name: &TaskKey,
    place: &TaskPlace,
    leftover: &Leftover<'_>,
) -> Result<(), Failure> {
    if place.directory().exists() {
        return Err(refusals::leftover_refusal(
            leftover.package,
            name.as_name(),
            place,
            leftover.dependency_path,
            leftover.regenerate,
        ));
    }
    if document.has_workspace_member(name.as_name()) {
        return Err(refusals::package_name_taken_refusal(
            name.as_name(),
            project.workspace_root(),
        ));
    }
    Ok(())
}
