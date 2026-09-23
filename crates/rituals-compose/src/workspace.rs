//! Whether a crate can be written into a directory at all, given what Cargo
//! resolves for it.
//!
//! Both `new` and `create` scaffold a crate that has to build on its own,
//! and neither can do that inside another project's Cargo workspace: a
//! member only builds as part of the workspace that claims it. This module
//! answers that one question — can a crate be written here, standing
//! alone — and renders the refusal when the answer is no.
//!
//! The input is the working directory the scaffolding task was run in,
//! plus that task's own three clauses of wording — never anything a third
//! party supplies. What Cargo resolves for that directory is asked with
//! `cargo locate-project`; `rituals` carries nothing about manifests or
//! metadata by design, so this lives here rather than there.

use std::path::{Path, PathBuf};
use std::process::Command;

use rituals::{Failure, Outcome};

use crate::manifest;

/// What `cargo locate-project` answered.
pub(crate) enum Located {
    /// It found a manifest, at this path.
    Found(PathBuf),
    /// It exited unsuccessfully, with this stderr.
    NotFound(String),
}

/// Runs `cargo locate-project [--workspace] --message-format plain` in
/// `directory`, through `$CARGO` when set.
///
/// A non-zero exit is one of the two answers this decision tree reads, not
/// a tool failure — only a failure to run `cargo` at all is a [`Failure`].
pub(crate) fn locate_project(directory: &Path, workspace: bool) -> Result<Located, Failure> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut arguments = vec!["locate-project"];
    if workspace {
        arguments.push("--workspace");
    }
    arguments.extend(["--message-format", "plain"]);

    let output = Command::new(&cargo)
        .args(&arguments)
        .current_dir(directory)
        .output()
        .map_err(|error| Failure::new("running `cargo locate-project` failed").caused_by(error))?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(Located::Found(PathBuf::from(path)))
    } else {
        Ok(Located::NotFound(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

/// The words one caller's refusal is built from, so
/// [`ensure_the_directory_stands_alone`]'s two rendered sentences can differ
/// by caller without the decision tree itself changing.
///
/// Both rendered sentences read, in full, for `create`:
///
/// ```text
/// refusing `create demo`: /work/acme/tools is inside the Cargo workspace
/// rooted at /work/acme, and a crate scaffolded there does not build on
/// its own; run create outside any Cargo workspace, or, if the enclosing
/// project is a ritual project, add the task with that project's own
/// `add demo`
/// ```
///
/// `attempted_command` fills the first slot — the command as a person
/// typed it, without the bin name (`"create demo"`, `"new demo"`).
/// `why_not_here` fills the clause following "and" — one sentence fragment
/// naming the rule (`"a crate scaffolded there does not build on its
/// own"`). `what_to_do_instead` fills the clause following the semicolon —
/// one or more imperatives, naming this run's own name and never a binary,
/// since the project being stood in may call its own command line
/// anything.
///
/// The three fields are public because this is a passive, invariant-free
/// carrier of one caller's own words: no value of any field can be wrong,
/// so there is nothing a constructor would check. The nearest thing to it,
/// [`crate::generated_file::Entry`], keeps its fields private behind a
/// constructor and a getter instead, because its two strings are different
/// kinds of thing — one of them derived by Cargo — and a caller reads one
/// back; `Refusal`'s three are all words the caller wrote, read only by
/// this module's own two renderers, and never read back. A struct literal
/// also has no transposition hazard where a three-argument constructor of
/// three same-typed strings would.
///
/// The caller interpolates its own run's name into `attempted_command` and
/// `what_to_do_instead` before building one — see the example on
/// [`ensure_the_directory_stands_alone`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The command as a person typed it, without the bin name.
    pub attempted_command: String,
    /// One clause, following "and" in both rendered sentences.
    pub why_not_here: String,
    /// One or more imperatives, naming this run's own name and never a
    /// binary.
    pub what_to_do_instead: String,
}

/// Refuses to run where Cargo cannot build a standalone crate.
///
/// That is anywhere inside a declared workspace — the workspace Cargo
/// resolves for `directory` has a `[workspace]` table, so a crate written
/// there is either claimed by it as a member or refused by `cargo build`
/// for not being listed, and does not build on its own either way — or
/// anywhere Cargo finds a manifest but cannot resolve a workspace for it at
/// all.
///
/// An ordinary package with no `[workspace]` table is neither case: Cargo
/// reports it as its own implicit one-package workspace root, and a crate
/// written under it builds standalone. That is also why this check runs
/// `cargo locate-project` twice rather than walking the tree upward looking
/// for a `[workspace]` table — a walk stopping at the first ancestor
/// manifest that declares one gets the `exclude` case wrong: once a
/// directory the root's `exclude` covers has its own `Cargo.toml`, Cargo
/// reports it as its own implicit root, even though an ancestor above it
/// declares a workspace. Until it has one, Cargo walks past it to the
/// workspace root, and the directory is refused like any other inside it.
///
/// # Examples
///
/// ```
/// use rituals::{Name, Outcome};
/// use rituals_compose::workspace::{self, Refusal};
///
/// fn run(directory: &std::path::Path, name: &Name) -> Outcome {
///     workspace::ensure_the_directory_stands_alone(
///         directory,
///         &Refusal {
///             attempted_command: format!("create {name}"),
///             why_not_here: "a crate scaffolded there does not build on its own"
///                 .to_string(),
///             what_to_do_instead: format!(
///                 "run create outside any Cargo workspace, or, if the enclosing \
///                  project is a ritual project, add the task with that project's \
///                  own `add {name}`"
///             ),
///         },
///     )?;
///     // ... the rest of a scaffolding task's own work goes here.
///     Ok(())
/// }
///
/// // Never invoked here: a doctest runs inside this repository's own
/// // workspace, so calling it could only ever show the refusal.
/// let _ = run;
/// ```
///
/// # Errors
///
/// Returns `refusal` rendered against a declared workspace root above
/// `directory`, `refusal` rendered against a manifest Cargo cannot resolve
/// a workspace for, or a [`Failure`] naming `cargo locate-project` when
/// cargo itself could not be run.
pub fn ensure_the_directory_stands_alone(directory: &Path, refusal: &Refusal) -> Outcome {
    match locate_project(directory, true)? {
        Located::Found(root_manifest) => {
            if manifest::declares_a_workspace(&root_manifest) {
                return Err(declared_workspace_refusal(
                    directory,
                    refusal,
                    &root_manifest,
                ));
            }
            Ok(())
        }
        Located::NotFound(workspace_stderr) => match locate_project(directory, false)? {
            Located::Found(_manifest_with_no_resolvable_workspace) => Err(
                unresolvable_workspace_refusal(directory, refusal, &workspace_stderr),
            ),
            // No manifest at all, at or above directory: the common case.
            Located::NotFound(_no_manifest_anywhere) => Ok(()),
        },
    }
}

/// The refusal for a real, declared workspace root above `directory`.
fn declared_workspace_refusal(
    directory: &Path,
    refusal: &Refusal,
    root_manifest: &Path,
) -> Failure {
    let workspace_root = root_manifest
        .parent()
        .map_or_else(|| root_manifest.to_path_buf(), Path::to_path_buf);
    // Standing at the root itself is the common case, and naming one path
    // twice in a row reads as two different places.
    let location = if same_directory(directory, &workspace_root) {
        format!("{} is a Cargo workspace", directory.display())
    } else {
        format!(
            "{} is inside the Cargo workspace rooted at {}",
            directory.display(),
            workspace_root.display()
        )
    };
    Failure::new(format!(
        "refusing `{}`: {location}, and {}; {}",
        refusal.attempted_command, refusal.why_not_here, refusal.what_to_do_instead,
    ))
}

/// Whether `left` and `right` name one directory. Cargo prints the root in
/// its canonical form, which can differ from the caller's spelling by a
/// symlink, so both sides are canonicalized before comparing; a path that
/// cannot be is compared as written.
fn same_directory(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// The refusal for a manifest Cargo cannot resolve a workspace for at all —
/// a package sitting under a root that does not list it.
fn unresolvable_workspace_refusal(
    directory: &Path,
    refusal: &Refusal,
    cargo_stderr: &str,
) -> Failure {
    Failure::new(format!(
        "refusing `{}` in {}: cargo cannot resolve a workspace for that directory, and {}; {} \
         — {cargo_stderr}",
        refusal.attempted_command,
        directory.display(),
        refusal.why_not_here,
        refusal.what_to_do_instead,
    ))
}

#[cfg(test)]
mod tests {
    use super::{Refusal, ensure_the_directory_stands_alone};
    use crate::test_support::ScratchDir;

    /// A fixed [`Refusal`] every test below shares — the wording is not
    /// under test here, only the decision tree and that all three fields
    /// reach the rendered message.
    fn a_refusal() -> Refusal {
        Refusal {
            attempted_command: "create demo".to_string(),
            why_not_here: "a crate scaffolded there does not build on its own".to_string(),
            what_to_do_instead: "run create outside any Cargo workspace".to_string(),
        }
    }

    #[test]
    fn running_outside_any_workspace_is_allowed() -> Result<(), Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new("outside")?;
        let result = ensure_the_directory_stands_alone(scratch.path(), &a_refusal());
        assert!(
            result.is_ok(),
            "expected a bare temp dir with no manifest anywhere above it to be allowed: \
             {result:?}"
        );
        Ok(())
    }

    #[test]
    fn a_plain_package_with_no_workspace_table_is_allowed() -> Result<(), Box<dyn std::error::Error>>
    {
        let scratch = ScratchDir::new("plain-package")?;
        std::fs::create_dir_all(scratch.path().join("src"))?;
        std::fs::write(
            scratch.path().join("Cargo.toml"),
            "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        std::fs::write(scratch.path().join("src/lib.rs"), "")?;

        let result = ensure_the_directory_stands_alone(scratch.path(), &a_refusal());
        assert!(
            result.is_ok(),
            "expected an ordinary package with no [workspace] table to be allowed: {result:?}"
        );
        Ok(())
    }

    #[test]
    fn a_declared_virtual_workspace_root_is_refused() -> Result<(), Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new("virtual-workspace")?;
        std::fs::write(
            scratch.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\nresolver = \"3\"\n",
        )?;

        let result = ensure_the_directory_stands_alone(scratch.path(), &a_refusal());
        assert!(
            result.is_err(),
            "expected a declared workspace root to be refused"
        );
        if let Err(error) = result {
            let message = error.to_string();
            assert!(message.contains(&scratch.path().display().to_string()));
            assert!(
                message.contains("is a Cargo workspace"),
                "message was: {message}"
            );
            assert!(
                !message.contains("rooted at"),
                "standing at the root names it once: {message}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_directory_below_a_declared_root_names_both() -> Result<(), Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new("below-root")?;
        std::fs::write(
            scratch.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\nresolver = \"3\"\n",
        )?;
        let below = scratch.path().join("tools");
        std::fs::create_dir_all(&below)?;

        let result = ensure_the_directory_stands_alone(&below, &a_refusal());
        assert!(
            result.is_err(),
            "expected a directory below a root to be refused"
        );
        if let Err(error) = result {
            let message = error.to_string();
            assert!(
                message.contains(&below.display().to_string()),
                "message was: {message}"
            );
            assert!(
                message.contains("is inside the Cargo workspace rooted at"),
                "message was: {message}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_directory_the_roots_exclude_covers_is_allowed() -> Result<(), Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new("excluded")?;
        let excluded = scratch.path().join("excluded-member");
        std::fs::create_dir_all(excluded.join("src"))?;
        std::fs::write(
            scratch.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\nexclude = [\"excluded-member\"]\nresolver = \"3\"\n",
        )?;
        std::fs::write(
            excluded.join("Cargo.toml"),
            "[package]\nname = \"excluded-member\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        std::fs::write(excluded.join("src/lib.rs"), "")?;

        // This is the case a tree walk (stopping at the first ancestor
        // manifest with a [workspace] table) would get wrong: with a
        // manifest of its own, Cargo reports the excluded directory as its
        // own root, not the workspace it sits under, since the workspace's
        // own manifest excludes it.
        let result = ensure_the_directory_stands_alone(&excluded, &a_refusal());
        assert!(
            result.is_ok(),
            "expected a directory the root's exclude covers to be allowed: {result:?}"
        );
        Ok(())
    }

    #[test]
    fn a_package_under_an_unlisting_root_is_refused_with_cargos_own_text()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new("unlisting-root")?;
        let member = scratch.path().join("not-listed");
        std::fs::create_dir_all(member.join("src"))?;
        std::fs::write(
            scratch.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\nresolver = \"3\"\n",
        )?;
        std::fs::write(
            member.join("Cargo.toml"),
            "[package]\nname = \"not-listed\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        std::fs::write(member.join("src/lib.rs"), "")?;

        let result = ensure_the_directory_stands_alone(&member, &a_refusal());
        assert!(
            result.is_err(),
            "expected a package under a root that does not list it to be refused"
        );
        if let Err(error) = result {
            let message = error.to_string();
            assert!(message.contains("cargo cannot resolve a workspace for that directory"));
            // Cargo's own text is passed through, not paraphrased: asked the
            // same question directly, cargo's answer appears whole in the
            // refusal.
            let cargo =
                std::env::var_os("CARGO").ok_or("CARGO is set for every test cargo runs")?;
            let cargo_answer = std::process::Command::new(cargo)
                .args(["locate-project", "--workspace", "--message-format", "plain"])
                .current_dir(&member)
                .output()?;
            let cargo_stderr = String::from_utf8_lossy(&cargo_answer.stderr);
            let cargo_stderr = cargo_stderr.trim();
            assert!(
                !cargo_answer.status.success() && !cargo_stderr.is_empty(),
                "expected cargo itself to refuse this directory, with a reason"
            );
            assert!(
                message.contains(cargo_stderr),
                "expected cargo's own text in the refusal:\ncargo: {cargo_stderr}\nrefusal: {message}"
            );
        }
        Ok(())
    }

    /// The renderer is checked here too, not only the decision tree that
    /// calls it: a declared-workspace refusal must carry all three of the
    /// caller's own clauses, verbatim, and the workspace root
    /// `cargo locate-project` located — not a paraphrase of any of them.
    #[test]
    fn the_declared_workspace_sentence_carries_every_clause_and_the_located_root()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new("sentence-carries-clauses")?;
        std::fs::write(
            scratch.path().join("Cargo.toml"),
            "[workspace]\nmembers = []\nresolver = \"3\"\n",
        )?;
        let refusal = Refusal {
            attempted_command: "new inner".to_string(),
            why_not_here: "a project does not belong inside another project's workspace"
                .to_string(),
            what_to_do_instead: "run new outside any Cargo workspace".to_string(),
        };

        let result = ensure_the_directory_stands_alone(scratch.path(), &refusal);
        assert!(result.is_err(), "expected a declared workspace to refuse");
        if let Err(error) = result {
            let message = error.to_string();
            assert!(message.contains("refusing `new inner`"));
            assert!(message.contains(&scratch.path().display().to_string()));
            assert!(message.contains("is a Cargo workspace"));
            assert!(message.contains(&refusal.why_not_here));
            assert!(message.contains(&refusal.what_to_do_instead));
        }
        Ok(())
    }
}
