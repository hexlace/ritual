//! Running `cargo add` for an import: the arguments, and the run.
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

use rituals::{Failure, Name, Outcome};
use rituals_compose::cargo;

use crate::arguments::ImportArguments;

/// The arguments to `cargo add` that make `key` a dependency of `package`,
/// taken from where `arguments` says the crate comes from.
///
/// The crate goes last, after `--`, so a crate name is never read as a flag.
/// `--rename` is passed only when the key is not the crate's own name:
/// given an equal one, `cargo add` writes a redundant `package = "<same>"`.
/// A version in the crate (`greeter@0.1.0`) goes to Cargo untouched, which
/// owns that grammar and refuses a version given with `--path` or `--git`
/// before it writes anything.
pub(crate) fn arguments(package: &str, key: &Name, arguments: &ImportArguments) -> Vec<String> {
    let mut words = vec![
        "add".to_string(),
        "--package".to_string(),
        package.to_string(),
    ];
    if key.as_str() != arguments.crate_name() {
        words.extend(["--rename".to_string(), key.to_string()]);
    }
    words.extend(arguments.source().flags(None));
    words.extend(["--".to_string(), arguments.crate_spec().to_string()]);
    words
}

/// Runs `cargo add` with `arguments` in `current_dir`, through the cargo that
/// launched this process, and captures what it prints.
///
/// It runs in the directory the person typed `import` in, so that a relative
/// `--path` means what they typed. Cargo's output is captured rather than
/// shown: a refusal is one line of `import`'s own, and Cargo's progress
/// lines ahead of it would break that.
///
/// # Errors
///
/// A [`Failure`] saying `cargo add` could not be run, or `cargo add failed:`
/// followed by what it printed on stderr when it exited unsuccessfully.
pub(crate) fn run(current_dir: &Path, arguments: &[String]) -> Outcome {
    let output = cargo::command()
        .args(arguments)
        .current_dir(current_dir)
        .output()
        .map_err(|error| Failure::new("running `cargo add` failed").caused_by(error))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Failure::new(format!(
            "cargo add failed: {}",
            stderr.trim_end()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rituals::Name;

    use super::{arguments, run};
    use crate::test_support::{ScratchProject, TestOutcome, typed_arguments};

    /// The arguments `cargo add` is given for `words` typed as `import`'s,
    /// importing under `key` into `demo-ritual`.
    fn cargo_add_for(words: &[&str], key: &str) -> Result<Vec<String>, rituals::InvalidName> {
        Ok(arguments(
            "demo-ritual",
            &Name::new(key)?,
            &typed_arguments(words),
        ))
    }

    #[test]
    fn a_crate_from_the_registry_is_added_to_the_package_after_a_double_dash() -> TestOutcome {
        assert_eq!(
            cargo_add_for(&["greeter"], "greeter")?,
            ["add", "--package", "demo-ritual", "--", "greeter"]
        );
        Ok(())
    }

    #[test]
    fn a_key_that_is_not_the_crates_name_is_passed_as_a_rename() -> TestOutcome {
        assert_eq!(
            cargo_add_for(&["greeter", "hail"], "hail")?,
            [
                "add",
                "--package",
                "demo-ritual",
                "--rename",
                "hail",
                "--",
                "greeter"
            ]
        );
        Ok(())
    }

    #[test]
    fn a_version_goes_to_cargo_untouched_and_a_key_equal_to_the_name_adds_no_rename() -> TestOutcome
    {
        assert_eq!(
            cargo_add_for(&["greeter@0.1.0"], "greeter")?,
            ["add", "--package", "demo-ritual", "--", "greeter@0.1.0"]
        );
        assert_eq!(
            cargo_add_for(&["greeter@0.1.0", "hail"], "hail")?,
            [
                "add",
                "--package",
                "demo-ritual",
                "--rename",
                "hail",
                "--",
                "greeter@0.1.0"
            ]
        );
        Ok(())
    }

    #[test]
    fn each_source_flag_comes_before_the_double_dash() -> TestOutcome {
        assert_eq!(
            cargo_add_for(&["greeter", "--git", "https://x/g.git"], "greeter")?,
            [
                "add",
                "--package",
                "demo-ritual",
                "--git",
                "https://x/g.git",
                "--",
                "greeter"
            ]
        );
        assert_eq!(
            cargo_add_for(&["greeter", "--git", "u", "--branch", "next"], "greeter")?,
            [
                "add",
                "--package",
                "demo-ritual",
                "--git",
                "u",
                "--branch",
                "next",
                "--",
                "greeter"
            ]
        );
        assert_eq!(
            cargo_add_for(&["greeter", "--git", "u", "--tag", "v1"], "greeter")?,
            [
                "add",
                "--package",
                "demo-ritual",
                "--git",
                "u",
                "--tag",
                "v1",
                "--",
                "greeter"
            ]
        );
        assert_eq!(
            cargo_add_for(&["greeter", "--git", "u", "--rev", "abc"], "greeter")?,
            [
                "add",
                "--package",
                "demo-ritual",
                "--git",
                "u",
                "--rev",
                "abc",
                "--",
                "greeter"
            ]
        );
        Ok(())
    }

    #[test]
    fn a_relative_path_is_passed_as_it_was_typed() -> TestOutcome {
        assert_eq!(
            cargo_add_for(&["greeter", "--path", "../greeter"], "greeter")?,
            [
                "add",
                "--package",
                "demo-ritual",
                "--path",
                "../greeter",
                "--",
                "greeter"
            ]
        );
        Ok(())
    }

    /// Runs the real `cargo add` in a scratch project, which needs nothing
    /// from a registry for a crate that is a directory.
    #[test]
    fn a_crate_that_is_a_directory_is_added_by_the_real_cargo() -> TestOutcome {
        let project = ScratchProject::new("run-real-cargo-add")?;
        project.write_crate("greeter", true)?;
        let words = arguments(
            "demo-ritual",
            &Name::new("greeter")?,
            &typed_arguments(&["greeter", "--path", "../greeter"]),
        );

        run(&project.cli_dir(), &words)?;

        let manifest = std::fs::read_to_string(project.cli_manifest_path())?;
        assert!(
            manifest.contains("greeter = {"),
            "expected a dependency called greeter; the manifest was:\n{manifest}"
        );
        assert!(
            manifest.contains("path = \"../greeter\""),
            "expected the path as typed; the manifest was:\n{manifest}"
        );
        Ok(())
    }

    #[test]
    fn a_failure_carries_what_cargo_said() -> TestOutcome {
        let project = ScratchProject::new("run-failing-cargo-add")?;
        let words = arguments(
            "demo-ritual",
            &Name::new("ghost")?,
            &typed_arguments(&["ghost", "--path", "../ghost"]),
        );

        let outcome = run(&project.cli_dir(), &words);

        assert!(
            outcome.is_err(),
            "expected a crate that is not there to fail"
        );
        if let Err(failure) = outcome {
            let message = failure.to_string();
            assert!(
                message.starts_with("cargo add failed: "),
                "expected cargo's words to follow; got: {message}"
            );
            assert!(
                message.contains("ghost"),
                "cargo names what it could not find: {message}"
            );
            assert_eq!(message.trim_end(), message, "no trailing whitespace");
        }
        Ok(())
    }
}
