//! The two files a freshly scaffolded task crate is made of.
//!
//! `new` never renders a task crate and `create` never renders a generated
//! file, so this module holds only what `create` and `add` both reach for:
//! the manifest and the `src/lib.rs` a task crate starts with.

use std::fmt::Write as _;

use rituals::Name;

use crate::source::{Source, escape_toml_string};

/// Renders a task crate's `Cargo.toml`.
///
/// The table order is fixed: `[package]`, `[dependencies]`,
/// `[package.metadata.ritual]`, with `task = true` the only key under the
/// last — the shape a fresh project's manifest keeps to, so a diff of one is
/// always boring.
///
/// # Examples
///
/// ```
/// use rituals::Name;
/// use rituals_compose::source::Source;
/// use rituals_compose::task_crate;
///
/// let name = Name::new("greet")?;
/// let manifest = task_crate::manifest(&name, &Source::Inherited);
/// assert!(manifest.contains("rituals.workspace = true"));
/// # Ok::<(), rituals::InvalidName>(())
/// ```
#[must_use]
pub fn manifest(name: &Name, source: &Source) -> String {
    let mut output = String::new();

    output.push_str("[package]\n");
    let _ = writeln!(output, "name = \"{name}\"");
    output.push_str("version = \"0.1.0\"\n");
    output.push_str("edition = \"2024\"\n");
    output.push('\n');
    output.push_str("[dependencies]\n");
    output.push_str(&render_rituals_dependency(source));
    output.push('\n');
    output.push_str("[package.metadata.ritual]\n");
    output.push_str("task = true\n");

    output
}

/// Renders the one line naming `rituals` as a dependency, in the shape
/// `source` requires — a bare version requirement for the registry, a
/// one-line inline table for a path or a git source, and never a `*`.
fn render_rituals_dependency(source: &Source) -> String {
    match source {
        Source::Inherited => "rituals.workspace = true\n".to_string(),
        Source::Registry => format!("rituals = \"{}\"\n", rituals::VERSION),
        Source::Path(path) => format!(
            "rituals = {{ path = \"{}\" }}\n",
            escape_toml_string(&path.display().to_string())
        ),
        Source::Git(url) => format!("rituals = {{ git = \"{}\" }}\n", escape_toml_string(url)),
    }
}

/// Renders a task crate's `src/lib.rs` — the whole of a freshly scaffolded
/// task, before its author adds a first argument or a real body.
///
/// # Examples
///
/// ```
/// use rituals::Name;
/// use rituals_compose::task_crate;
///
/// let name = Name::new("greet")?;
/// let lib = task_crate::lib(&name);
/// assert!(lib.contains("pub fn task() -> Task"));
/// # Ok::<(), rituals::InvalidName>(())
/// ```
#[must_use]
pub fn lib(name: &Name) -> String {
    let mut output = String::new();

    let _ = writeln!(output, "//! The `{name}` task.");
    output.push('\n');
    output.push_str("use rituals::{Outcome, Task, clap, report};\n");
    output.push('\n');
    output.push_str("/// What this task accepts on the command line.\n");
    output.push_str("#[derive(clap::Args)]\n");
    output.push_str("struct Arguments {}\n");
    output.push('\n');
    output.push_str("/// This task, for a command line to mount under whatever name imports it.\n");
    output.push_str("#[must_use]\n");
    output.push_str("pub fn task() -> Task {\n");
    let _ = writeln!(
        output,
        "    Task::new(\"one line about what {name} does\", run)"
    );
    output.push_str("}\n");
    output.push('\n');
    output
        .push_str("// `Arguments` has no fields yet, so this one is unused. Drop the underscore\n");
    output.push_str("// when you add the first field.\n");
    output.push_str("fn run(_arguments: Arguments) -> Outcome {\n");
    let _ = writeln!(output, "    report(\"{name} has nothing to do yet\");");
    output.push_str("    Ok(())\n");
    output.push_str("}\n");

    output
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rituals::Name;

    use super::{lib, manifest, render_rituals_dependency};
    use crate::source::Source;

    #[test]
    fn no_renderer_output_ever_contains_an_asterisk() {
        let name = Name::new("greet").expect("greet is a valid name");
        let manifest = manifest(&name, &Source::Inherited);
        assert!(!manifest.contains('*'));

        let lib = lib(&name);
        assert!(!lib.contains('*'));
    }

    #[test]
    fn manifest_for_an_inherited_source() {
        let name = Name::new("greet").expect("greet is a valid name");
        let rendered = manifest(&name, &Source::Inherited);
        assert_eq!(
            rendered,
            "[package]\n\
             name = \"greet\"\n\
             version = \"0.1.0\"\n\
             edition = \"2024\"\n\
             \n\
             [dependencies]\n\
             rituals.workspace = true\n\
             \n\
             [package.metadata.ritual]\n\
             task = true\n"
        );
    }

    #[test]
    fn manifest_for_a_path_source_containing_a_quote_and_a_backslash() {
        let name = Name::new("greet").expect("greet is a valid name");
        let path = PathBuf::from("/weird\"path\\with\\backslashes");
        let rendered = manifest(&name, &Source::Path(path));
        assert!(
            rendered.contains("rituals = { path = \"/weird\\\"path\\\\with\\\\backslashes\" }\n")
        );
    }

    #[test]
    fn manifest_for_a_git_source() {
        let name = Name::new("greet").expect("greet is a valid name");
        let rendered = manifest(&name, &Source::Git("https://example.invalid/x".to_string()));
        assert!(rendered.contains("rituals = { git = \"https://example.invalid/x\" }\n"));
    }

    /// Every TOML 1.0 basic-string escape class in one input, plus a
    /// non-ASCII character: `\b \t \n \f \r \" \\`, a control character with
    /// no named escape (U+0001), U+007F, and `é`. `toml_edit` is the oracle
    /// — it is what Cargo itself would parse this line with — so this reads
    /// the rendered value back rather than re-deriving the same escape table
    /// in a second, hand-written check.
    const ESCAPE_CLASS_CORPUS: &str = "\u{8}\t\n\u{c}\r\"\\\u{1}\u{7f}é";

    #[test]
    fn render_rituals_dependency_for_a_path_round_trips_through_toml_edit() {
        let path = PathBuf::from(ESCAPE_CLASS_CORPUS);
        let rendered = render_rituals_dependency(&Source::Path(path.clone()));

        let parsed: Result<toml_edit::DocumentMut, _> = rendered.parse();
        assert!(
            parsed.is_ok(),
            "rendered line must parse as TOML: {parsed:?}\n{rendered}"
        );
        if let Ok(document) = parsed {
            let read_back = document
                .get("rituals")
                .and_then(toml_edit::Item::as_inline_table)
                .and_then(|table| table.get("path"))
                .and_then(toml_edit::Value::as_str);
            assert_eq!(read_back, Some(path.display().to_string().as_str()));
        }
    }

    #[test]
    fn render_rituals_dependency_for_a_git_source_round_trips_through_toml_edit() {
        let url = format!("https://example.invalid/{ESCAPE_CLASS_CORPUS}");
        let rendered = render_rituals_dependency(&Source::Git(url.clone()));

        let parsed: Result<toml_edit::DocumentMut, _> = rendered.parse();
        assert!(
            parsed.is_ok(),
            "rendered line must parse as TOML: {parsed:?}\n{rendered}"
        );
        if let Ok(document) = parsed {
            let read_back = document
                .get("rituals")
                .and_then(toml_edit::Item::as_inline_table)
                .and_then(|table| table.get("git"))
                .and_then(toml_edit::Value::as_str);
            assert_eq!(read_back, Some(url.as_str()));
        }
    }

    #[test]
    fn lib_reports_the_task_name_in_three_places() {
        let name = Name::new("greet").expect("greet is a valid name");
        let rendered = lib(&name);
        assert_eq!(
            rendered,
            "//! The `greet` task.\n\
             \n\
             use rituals::{Outcome, Task, clap, report};\n\
             \n\
             /// What this task accepts on the command line.\n\
             #[derive(clap::Args)]\n\
             struct Arguments {}\n\
             \n\
             /// This task, for a command line to mount under whatever name imports it.\n\
             #[must_use]\n\
             pub fn task() -> Task {\n\
             \x20\x20\x20\x20Task::new(\"one line about what greet does\", run)\n\
             }\n\
             \n\
             // `Arguments` has no fields yet, so this one is unused. Drop the underscore\n\
             // when you add the first field.\n\
             fn run(_arguments: Arguments) -> Outcome {\n\
             \x20\x20\x20\x20report(\"greet has nothing to do yet\");\n\
             \x20\x20\x20\x20Ok(())\n\
             }\n"
        );
    }
}
