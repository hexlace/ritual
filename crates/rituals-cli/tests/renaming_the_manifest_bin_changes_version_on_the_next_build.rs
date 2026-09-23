//! Renaming a composed CLI's `[[bin]]` target in its manifest changes what
//! `--version` reports on the very next build, with no need to regenerate
//! the project's generated file first: the name is compiled in from the
//! bin target, and the generated file never spells it.

mod support;

use support::manifest;
use support::{OptionContext, Project, TempDir, TestOutcome, in_checkout, run_binary};

const RENAMED: &str = "renamed-by-hand";

#[test]
fn renaming_the_bin_target_changes_version_with_no_regenerate() -> TestOutcome {
    in_checkout(|checkout| {
        let working_dir = TempDir::new("rename-bin-target")?;
        let project =
            Project::scaffold(checkout, working_dir.path(), "rename-target-project", &[])?;
        let cli_manifest = project.cli_manifest()?;
        assert_ne!(manifest::sole_bin_name(&cli_manifest)?, RENAMED);
        let version_number = manifest::string_at(&cli_manifest, &["package", "version"])
            .context("expected the composed CLI's manifest to carry a version")?
            .to_string();

        let generated_before = project.generated_file()?;
        manifest::edit(&project.cli_manifest_path(), |document| {
            manifest::rename_sole_bin(document, RENAMED)
        })?;
        let binary = project.build()?;
        assert_eq!(
            project.generated_file()?,
            generated_before,
            "expected the generated file untouched by the rename and the build — no \
             regenerate is needed first"
        );

        let version = run_binary(&binary, project.root(), &["--version"])?;
        version.expect_success("`--version` on the renamed binary");
        assert_eq!(
            version.stdout,
            format!("{RENAMED} {version_number}\n"),
            "expected --version to report the manifest's new [[bin]] name and the project's \
             version"
        );
        Ok(())
    })
}
