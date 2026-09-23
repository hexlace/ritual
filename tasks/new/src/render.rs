//! The three files only `new` writes: a project's own workspace manifest,
//! its `cargo <cli>` alias, and its composed CLI crate's manifest.
//!
//! The fourth file `new` writes — the generated `main.rs` — is not here:
//! it is `rituals_compose::generated_file::render`, the same renderer
//! `regenerate` uses, called with the one entry [`MANAGEMENT_BUNDLE`]
//! names, which is what makes a fresh project's generated file identical to
//! what `regenerate` would produce there.
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

use std::fmt::Write as _;

use rituals::Name;
use rituals_compose::source::{Source, escape_toml_string};
use rituals_compose::top_level::MANAGEMENT_BUNDLE_KEY;

/// One of ritual's own crates a scaffolded project imports: the dependency
/// key a manifest names it under, the package name when it differs from
/// that key, and the crate's directory within a checkout, for a `--path`
/// source.
pub(crate) struct ImportedCrate {
    pub(crate) key: &'static str,
    package: Option<&'static str>,
    directory: &'static str,
}

/// The one crate a scaffolded project's `[workspace.dependencies]`
/// names. Not renamed — the dependency key already is the package
/// name — so every task crate the project scaffolds later can write
/// `rituals.workspace = true` and nothing else.
///
/// It does not declare `rituals-compose`: nothing the scaffold writes
/// depends on it.
const FRAMEWORK: ImportedCrate = ImportedCrate {
    key: "rituals",
    package: None,
    directory: "crates/rituals",
};

/// Ritual's own management tasks, as the one entry a scaffolded project's
/// composed CLI imports and lists. The dependency key is `ritual` in every
/// project, whatever the project calls its own binary: whether that bundle
/// flattens into the top level or stays reached by typing `ritual` first is
/// decided at run time by comparing this key against the compiled bin name,
/// so the key itself never varies. The same constant supplies the dependency
/// line, the `tasks` entry and the generated file's one mount, which is what
/// stops the three drifting.
pub(crate) const MANAGEMENT_BUNDLE: ImportedCrate = ImportedCrate {
    key: MANAGEMENT_BUNDLE_KEY,
    package: Some("rituals-core"),
    directory: "crates/rituals-core",
};

/// Renders `<name>/Cargo.toml` — the project's workspace manifest, with an
/// explicit multi-line `members` list naming the composed CLI crate's own
/// fixed directory, `ritual` (never a `*`, anywhere — and never `--cli`'s
/// value, which names the `[[bin]]` and the cargo alias and nothing on
/// disk), and `[workspace.dependencies]` naming [`FRAMEWORK`] from
/// `source`.
#[must_use]
pub(crate) fn workspace_manifest(source: &Source) -> String {
    let mut output = String::new();

    output.push_str("[workspace]\n");
    output.push_str("members = [\n");
    output.push_str("    \"ritual\",\n");
    output.push_str("]\n");
    output.push_str("resolver = \"3\"\n");
    output.push('\n');
    output.push_str("[workspace.dependencies]\n");
    output.push_str(&dependency_line(&FRAMEWORK, source));

    output
}

/// Renders the line naming `imported` as a dependency from `source`: a
/// version requirement for the registry, `path = "…"` or `git = "…"`
/// otherwise, with a `package = "…"` clause present only when
/// `imported.package` differs from the key.
///
/// A registry requirement is always [`rituals::VERSION`], because every
/// crate ritual publishes moves at that version together. `source`'s path,
/// when it is one, is the checkout root; `imported.directory` is joined
/// onto it. A `--git` source names no directory at all: Cargo finds a
/// package by name anywhere in the repository.
fn dependency_line(imported: &ImportedCrate, source: &Source) -> String {
    let mut line = String::new();
    let package_clause = imported
        .package
        .map_or_else(String::new, |package| format!("package = \"{package}\", "));

    match source {
        Source::Inherited => {
            // `new` writes the workspace every later crate inherits from, so
            // it has nothing to inherit itself; SourceArguments::resolve()
            // never produces this variant.
            unreachable!("new never inherits a source; it resolves the registry, --path or --git");
        }
        Source::Registry => {
            let version = rituals::VERSION;
            if package_clause.is_empty() {
                let _ = writeln!(line, "{} = \"{version}\"", imported.key);
            } else {
                let _ = writeln!(
                    line,
                    "{} = {{ {package_clause}version = \"{version}\" }}",
                    imported.key
                );
            }
        }
        Source::Path(checkout_root) => {
            let crate_directory = checkout_root.join(imported.directory);
            let _ = writeln!(
                line,
                "{} = {{ {package_clause}path = \"{}\" }}",
                imported.key,
                escape_toml_string(&crate_directory.display().to_string())
            );
        }
        Source::Git(url) => {
            let _ = writeln!(
                line,
                "{} = {{ {package_clause}git = \"{}\" }}",
                imported.key,
                escape_toml_string(url)
            );
        }
    }

    line
}

/// Renders `<name>/.cargo/config.toml` — the `cargo <cli>` alias.
#[must_use]
pub(crate) fn cargo_alias(cli: &Name, cli_package: &str) -> String {
    let mut output = String::new();

    let _ = writeln!(
        output,
        "# `cargo {cli} <task>` runs this project's own composed command line, from"
    );
    output.push_str(
        "# anywhere inside the checkout. Nothing has to be installed: the alias builds\n",
    );
    output.push_str("# the crate on demand, the way a project-local task runner already does.\n");
    output.push_str("[alias]\n");
    let _ = writeln!(
        output,
        "{cli} = [\"run\", \"--package\", \"{cli_package}\", \"--\"]"
    );

    output
}

/// Renders `<name>/ritual/Cargo.toml` — the composed CLI crate's own
/// manifest, with the explicit `[[bin]] name = "<cli>"` an unambiguous
/// target requires, `[dependencies]` naming `rituals` and
/// [`MANAGEMENT_BUNDLE`] from `source`, and `[package.metadata.ritual]
/// tasks` listing the bundle's own key.
///
/// `cli_package` is the package name — always `<name>-ritual`, independent
/// of `cli` — and `cli` is the `[[bin]]` name and the cargo alias key, not
/// a path component: the crate's own directory is fixed at `ritual/`
/// regardless of what `cli` names. Neither shares a type with `source`, so
/// no two parameters of this function have a transposition hazard.
#[must_use]
pub(crate) fn cli_manifest(cli: &Name, cli_package: &str, source: &Source) -> String {
    let mut output = String::new();

    output.push_str("[package]\n");
    let _ = writeln!(output, "name = \"{cli_package}\"");
    output.push_str("version = \"0.1.0\"\n");
    output.push_str("edition = \"2024\"\n");
    output.push('\n');
    output.push_str("[[bin]]\n");
    let _ = writeln!(output, "name = \"{cli}\"");
    output.push_str("path = \"src/main.rs\"\n");
    output.push('\n');
    output.push_str("[dependencies]\n");
    output.push_str("rituals.workspace = true\n");
    output.push_str(&dependency_line(&MANAGEMENT_BUNDLE, source));
    output.push('\n');
    output.push_str("[package.metadata.ritual]\n");
    let _ = writeln!(output, "tasks = [\"{}\"]", MANAGEMENT_BUNDLE.key);

    output
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rituals::Name;
    use rituals_compose::source::Source;

    use super::{MANAGEMENT_BUNDLE, cargo_alias, cli_manifest, workspace_manifest};

    /// Every TOML 1.0 basic-string escape class in one input, plus a
    /// non-ASCII character: `\b \t \n \f \r \" \\`, a control character with
    /// no named escape (U+0001), U+007F, and `é`. `toml_edit` is the oracle
    /// — it is what Cargo itself would parse this line with — so the tests
    /// below read the rendered value back rather than re-deriving the same
    /// escape table in a second, hand-written check.
    const ESCAPE_CLASS_CORPUS: &str = "\u{8}\t\n\u{c}\r\"\\\u{1}\u{7f}é";

    /// A `Name` from a literal already known to be valid — mirrors
    /// `lib.rs`'s own `demo_name` helper.
    fn valid_name(value: &str) -> Name {
        Name::new(value).expect("a test passes only names it knows are valid")
    }

    fn ritual_name() -> Name {
        valid_name("ritual")
    }

    #[test]
    fn workspace_manifest_for_a_path_source_round_trips_through_toml_edit() {
        let checkout_root = PathBuf::from(ESCAPE_CLASS_CORPUS);
        let rendered = workspace_manifest(&Source::Path(checkout_root.clone()));

        let parsed: Result<toml_edit::DocumentMut, _> = rendered.parse();
        assert!(
            parsed.is_ok(),
            "rendered manifest must parse as TOML: {parsed:?}\n{rendered}"
        );
        if let Ok(document) = parsed {
            let dependencies = document
                .get("workspace")
                .and_then(toml_edit::Item::as_table)
                .and_then(|workspace| workspace.get("dependencies"))
                .and_then(toml_edit::Item::as_table);
            assert!(
                dependencies.is_some(),
                "[workspace.dependencies] must be a table:\n{rendered}"
            );
            if let Some(dependencies) = dependencies {
                let read_back = dependencies
                    .get("rituals")
                    .and_then(toml_edit::Item::as_inline_table)
                    .and_then(|table| table.get("path"))
                    .and_then(toml_edit::Value::as_str);
                let expected = checkout_root.join("crates").join("rituals");
                assert_eq!(read_back, Some(expected.display().to_string().as_str()));
            }
        }
    }

    #[test]
    fn workspace_manifest_for_a_git_source_round_trips_through_toml_edit() {
        let url = format!("https://example.invalid/{ESCAPE_CLASS_CORPUS}");
        let rendered = workspace_manifest(&Source::Git(url.clone()));

        let parsed: Result<toml_edit::DocumentMut, _> = rendered.parse();
        assert!(
            parsed.is_ok(),
            "rendered manifest must parse as TOML: {parsed:?}\n{rendered}"
        );
        if let Ok(document) = parsed {
            let dependencies = document
                .get("workspace")
                .and_then(toml_edit::Item::as_table)
                .and_then(|workspace| workspace.get("dependencies"))
                .and_then(toml_edit::Item::as_table);
            assert!(
                dependencies.is_some(),
                "[workspace.dependencies] must be a table:\n{rendered}"
            );
            if let Some(dependencies) = dependencies {
                let read_back = dependencies
                    .get("rituals")
                    .and_then(toml_edit::Item::as_inline_table)
                    .and_then(|table| table.get("git"))
                    .and_then(toml_edit::Value::as_str);
                assert_eq!(read_back, Some(url.as_str()));
            }
        }
    }

    #[test]
    fn workspace_manifest_for_a_path_source() {
        let rendered = workspace_manifest(&Source::Path(std::path::PathBuf::from("/checkout")));
        assert_eq!(
            rendered,
            "[workspace]\n\
             members = [\n\
             \x20\x20\x20\x20\"ritual\",\n\
             ]\n\
             resolver = \"3\"\n\
             \n\
             [workspace.dependencies]\n\
             rituals = { path = \"/checkout/crates/rituals\" }\n"
        );
        assert!(!rendered.contains('*'));
    }

    #[test]
    fn workspace_manifest_for_a_git_source() {
        let rendered = workspace_manifest(&Source::Git("https://example.invalid/x".to_string()));
        assert!(rendered.contains("rituals = { git = \"https://example.invalid/x\" }\n"));
    }

    /// `workspace_manifest`'s one member is `ritual`, fixed, whatever `--cli`
    /// says — the workspace manifest takes no `cli` argument at all, so
    /// there is nothing here for `--cli` to reach.
    #[test]
    fn workspace_manifest_names_ritual_as_its_one_member_regardless_of_cli() {
        let rendered = workspace_manifest(&Source::Path(std::path::PathBuf::from("/checkout")));
        assert!(rendered.contains("members = [\n    \"ritual\",\n]\n"));
    }

    /// `--cli mytool` reaches the two renderers it actually varies: the
    /// alias's key and its comment, and the CLI manifest's `[[bin]]` name
    /// — while the CLI's own package name, `demo-ritual`, and the
    /// workspace manifest's fixed `ritual` member are both unaffected,
    /// since `--cli` names the command line, not the import and not a
    /// path.
    #[test]
    fn a_named_cli_reaches_the_alias_and_the_bin_name() {
        let cli = valid_name("mytool");
        let source = Source::Path(std::path::PathBuf::from("/checkout"));

        let alias = cargo_alias(&cli, "demo-ritual");
        assert!(alias.contains("# `cargo mytool <task>` runs"));
        assert!(alias.contains("mytool = [\"run\", \"--package\", \"demo-ritual\", \"--\"]\n"));

        let manifest = cli_manifest(&cli, "demo-ritual", &source);
        assert!(manifest.contains("name = \"demo-ritual\"\n"));
        assert!(manifest.contains("[[bin]]\nname = \"mytool\"\n"));
    }

    #[test]
    fn cargo_alias_names_the_cli_package() {
        let rendered = cargo_alias(&ritual_name(), "demo-ritual");
        assert_eq!(
            rendered,
            "# `cargo ritual <task>` runs this project's own composed command line, from\n\
             # anywhere inside the checkout. Nothing has to be installed: the alias builds\n\
             # the crate on demand, the way a project-local task runner already does.\n\
             [alias]\n\
             ritual = [\"run\", \"--package\", \"demo-ritual\", \"--\"]\n"
        );
    }

    #[test]
    fn cli_manifest_declares_the_explicit_bin_target() {
        let rendered = cli_manifest(
            &ritual_name(),
            "demo-ritual",
            &Source::Path(std::path::PathBuf::from("/checkout")),
        );
        assert_eq!(
            rendered,
            "[package]\n\
             name = \"demo-ritual\"\n\
             version = \"0.1.0\"\n\
             edition = \"2024\"\n\
             \n\
             [[bin]]\n\
             name = \"ritual\"\n\
             path = \"src/main.rs\"\n\
             \n\
             [dependencies]\n\
             rituals.workspace = true\n\
             ritual = { package = \"rituals-core\", path = \"/checkout/crates/rituals-core\" }\n\
             \n\
             [package.metadata.ritual]\n\
             tasks = [\"ritual\"]\n"
        );
        assert!(!rendered.contains('*'));
    }

    #[test]
    fn cli_manifest_for_a_git_source() {
        let rendered = cli_manifest(
            &ritual_name(),
            "demo-ritual",
            &Source::Git("https://example.invalid/x".to_string()),
        );
        assert!(rendered.contains(
            "ritual = { package = \"rituals-core\", git = \"https://example.invalid/x\" }\n"
        ));
        assert!(rendered.contains("tasks = [\"ritual\"]"));
    }

    #[test]
    fn cli_manifest_for_a_path_source_round_trips_through_toml_edit() {
        let checkout_root = PathBuf::from(ESCAPE_CLASS_CORPUS);
        let rendered = cli_manifest(
            &ritual_name(),
            "demo-ritual",
            &Source::Path(checkout_root.clone()),
        );

        let parsed: Result<toml_edit::DocumentMut, _> = rendered.parse();
        assert!(
            parsed.is_ok(),
            "rendered manifest must parse as TOML: {parsed:?}\n{rendered}"
        );
        if let Ok(document) = parsed {
            let dependencies = document
                .get("dependencies")
                .and_then(toml_edit::Item::as_table);
            assert!(
                dependencies.is_some(),
                "[dependencies] must be a table:\n{rendered}"
            );
            if let Some(dependencies) = dependencies {
                let table = dependencies
                    .get(MANAGEMENT_BUNDLE.key)
                    .and_then(toml_edit::Item::as_inline_table);
                assert!(
                    table.is_some(),
                    "expected `{}` in [dependencies]:\n{rendered}",
                    MANAGEMENT_BUNDLE.key
                );
                if let Some(table) = table {
                    assert_eq!(
                        table.get("package").and_then(toml_edit::Value::as_str),
                        Some("rituals-core")
                    );
                    let expected = checkout_root.join("crates/rituals-core");
                    assert_eq!(
                        table.get("path").and_then(toml_edit::Value::as_str),
                        Some(expected.display().to_string().as_str())
                    );
                }
            }
        }
    }

    #[test]
    fn cli_manifest_for_a_git_source_round_trips_through_toml_edit() {
        let url = format!("https://example.invalid/{ESCAPE_CLASS_CORPUS}");
        let rendered = cli_manifest(&ritual_name(), "demo-ritual", &Source::Git(url.clone()));

        let parsed: Result<toml_edit::DocumentMut, _> = rendered.parse();
        assert!(
            parsed.is_ok(),
            "rendered manifest must parse as TOML: {parsed:?}\n{rendered}"
        );
        if let Ok(document) = parsed {
            let dependencies = document
                .get("dependencies")
                .and_then(toml_edit::Item::as_table);
            assert!(
                dependencies.is_some(),
                "[dependencies] must be a table:\n{rendered}"
            );
            if let Some(dependencies) = dependencies {
                let table = dependencies
                    .get(MANAGEMENT_BUNDLE.key)
                    .and_then(toml_edit::Item::as_inline_table);
                assert!(
                    table.is_some(),
                    "expected `{}` in [dependencies]:\n{rendered}",
                    MANAGEMENT_BUNDLE.key
                );
                if let Some(table) = table {
                    assert_eq!(
                        table.get("package").and_then(toml_edit::Value::as_str),
                        Some("rituals-core")
                    );
                    assert_eq!(
                        table.get("git").and_then(toml_edit::Value::as_str),
                        Some(url.as_str())
                    );
                    assert!(
                        table.get("path").is_none(),
                        "a --git source must name no directory at all"
                    );
                }
            }
        }
    }
}
