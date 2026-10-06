//! Where a scaffolded crate gets ritual's own crates from, and the
//! `--path`/`--git` pair a scaffolding task declares to let a caller choose
//! somewhere other than crates.io.

use std::fmt::{self, Write as _};
use std::path::PathBuf;

use rituals::clap;

/// Where a scaffolded crate's dependencies on ritual's own crates come from.
///
/// # Examples
///
/// ```
/// use rituals::Name;
/// use rituals_compose::source::Source;
/// use rituals_compose::task_crate::{self, Audience};
///
/// let name = Name::new("greet")?;
///
/// let registry = task_crate::manifest(&name, &Source::Registry, Audience::Public);
/// assert!(registry.contains(&format!("rituals = \"{}\"", rituals::VERSION)));
///
/// let git = Source::Git("https://example.com/ritual".to_string());
/// let manifest = task_crate::manifest(&name, &git, Audience::Public);
/// assert!(manifest.contains("git = \"https://example.com/ritual\""));
/// # Ok::<(), rituals::InvalidName>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// `rituals.workspace = true` — what `create` writes inside a
    /// project, for a crate scaffolded in a workspace that already declares the dependency.
    Inherited,
    /// `rituals = "X.Y.Z"` from crates.io, where `X.Y.Z` is always
    /// [`rituals::VERSION`] — the release the scaffolding binary itself was
    /// built against. The default when no other source is named.
    Registry,
    /// `rituals = { path = "…" }`.
    ///
    /// [`SourceArguments::resolve`] returns the path exactly as typed: the
    /// root of a ritual checkout. A renderer that writes it as a dependency
    /// line needs the directory of the crate it names, so a caller either
    /// joins that directory on as it renders (`new`, which names more than
    /// one crate from the same checkout) or narrows this to the absolute
    /// `rituals` crate directory before handing it to
    /// [`crate::task_crate::manifest`], which writes it as given.
    Path(PathBuf),
    /// `rituals = { git = "…" }`.
    Git(String),
}

/// The `--path` / `--git` pair, flattened into a scaffolding task's
/// arguments — declared once here rather than by `new` and `create`
/// separately, so their help text and refusals cannot drift apart.
///
/// Naming neither is the ordinary case: ritual's crates then come from
/// crates.io, at the version this binary was built against. Naming both is
/// refused by clap while it parses, like any other argument error, so a
/// task never runs with two sources.
#[derive(clap::Args, Debug)]
pub struct SourceArguments {
    /// take ritual's crates from a checkout instead of crates.io: the
    /// directory containing crates/rituals
    #[arg(long, value_name = "DIR", conflicts_with = "git")]
    path: Option<PathBuf>,

    /// take ritual's crates from a git repository instead of crates.io
    #[arg(long, value_name = "URL")]
    git: Option<String>,
}

impl SourceArguments {
    /// Returns the source the caller named, or [`Source::Registry`] when
    /// they named none.
    ///
    /// This resolves only which flag was given. It does not check that
    /// `--path` actually names a ritual checkout; that check needs a
    /// filesystem read and belongs to the caller, which is why
    /// [`Source::Path`] here still carries whatever path the caller typed.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals::clap::{self, Parser};
    /// use rituals_compose::source::{Source, SourceArguments};
    ///
    /// #[derive(clap::Parser)]
    /// struct Command {
    ///     #[command(flatten)]
    ///     source: SourceArguments,
    /// }
    ///
    /// let neither = Command::parse_from(["program"]);
    /// assert_eq!(neither.source.resolve(), Source::Registry);
    ///
    /// let command = Command::parse_from(["program", "--path", "/checkout"]);
    /// assert_eq!(command.source.resolve(), Source::Path("/checkout".into()));
    ///
    /// let both = Command::try_parse_from(["program", "--path", "/checkout", "--git", "https://x"]);
    /// assert!(both.is_err());
    /// ```
    #[must_use]
    pub fn resolve(&self) -> Source {
        match (&self.path, &self.git) {
            (None, None) => Source::Registry,
            (Some(path), None) => Source::Path(path.clone()),
            (None, Some(git)) => Source::Git(git.clone()),
            (Some(_), Some(_)) => {
                unreachable!("clap refuses --path with --git while parsing (`conflicts_with`)")
            }
        }
    }
}

/// Where `rituals`'s manifest sits inside a ritual checkout, relative to
/// its root.
///
/// `--path` is checked against this before anything is written, and both
/// `new` and `create` check the same string, so they cannot disagree about
/// what a checkout looks like.
pub const RITUALS_MANIFEST_IN_CHECKOUT: &str = "crates/rituals/Cargo.toml";

/// Where ritual's own tasks sit inside a ritual checkout, relative to its
/// root: the directory a project `new` writes from a `--path` source names
/// as its `ritual` dependency.
pub const MANAGEMENT_BUNDLE_IN_CHECKOUT: &str = ".rituals/ritual";

/// Checks that `checkout_root` is a ritual checkout a project can be made
/// from.
///
/// It has to hold both directories such a project names: it contains
/// [`RITUALS_MANIFEST_IN_CHECKOUT`], and a `Cargo.toml` in
/// [`MANAGEMENT_BUNDLE_IN_CHECKOUT`].
///
/// A checkout with the first and not the second is one whose own tasks are
/// somewhere else, such as one from before they lived in `.rituals/`. A
/// project made from it would name a directory that is not there, and only
/// its first build would say so.
///
/// Shared by `new` and `create`, which both validate a `--path` source and
/// must refuse a bad one with the same message: two independent task crates
/// hand-writing the same wording is exactly the drift a shared place like
/// this module exists to prevent.
///
/// # Errors
///
/// Returns [`InvalidCheckout`] naming `checkout_root` and the first of the
/// two manifests it does not contain.
///
/// # Examples
///
/// ```
/// use rituals_compose::source::assert_is_a_ritual_checkout;
///
/// # let checkout_root = std::env::temp_dir()
/// #     .join(format!("rituals-compose-doctest-checkout-{}", std::process::id()));
/// # std::fs::create_dir_all(checkout_root.join("crates/rituals"))?;
/// # std::fs::write(checkout_root.join("crates/rituals/Cargo.toml"), "[package]\n")?;
/// # std::fs::create_dir_all(checkout_root.join(".rituals/ritual"))?;
/// # std::fs::write(checkout_root.join(".rituals/ritual/Cargo.toml"), "[package]\n")?;
/// assert!(assert_is_a_ritual_checkout(&checkout_root).is_ok());
///
/// let not_a_checkout = std::env::temp_dir();
/// assert!(assert_is_a_ritual_checkout(&not_a_checkout).is_err());
/// # std::fs::remove_dir_all(&checkout_root)?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn assert_is_a_ritual_checkout(checkout_root: &std::path::Path) -> Result<(), InvalidCheckout> {
    let missing = if !checkout_root.join(RITUALS_MANIFEST_IN_CHECKOUT).is_file() {
        Missing::Rituals
    } else if !checkout_root
        .join(MANAGEMENT_BUNDLE_IN_CHECKOUT)
        .join("Cargo.toml")
        .is_file()
    {
        Missing::ManagementBundle
    } else {
        return Ok(());
    };
    Err(InvalidCheckout {
        path: checkout_root.to_path_buf(),
        missing,
    })
}

/// A `--path` source that does not name a real ritual checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidCheckout {
    path: PathBuf,
    missing: Missing,
}

/// Which manifest a checkout lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Missing {
    /// The `rituals` crate's, so it is not a ritual checkout at all, or
    /// `--path` names a directory inside one.
    Rituals,
    /// The one in ritual's own tasks, so its tasks are somewhere a project
    /// made from it would not find them.
    ManagementBundle,
}

impl fmt::Display for InvalidCheckout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.missing {
            Missing::Rituals => write!(
                formatter,
                "{} is not a ritual checkout: it does not contain \
                 {RITUALS_MANIFEST_IN_CHECKOUT}; --path takes the checkout root, not crates/ and \
                 not the crate directory",
                self.path.display()
            ),
            Missing::ManagementBundle => write!(
                formatter,
                "{} is not a checkout this ritual can use: it does not contain \
                 {MANAGEMENT_BUNDLE_IN_CHECKOUT}/Cargo.toml, ritual's own tasks, which the \
                 project would depend on; use a checkout of the version you are running",
                self.path.display()
            ),
        }
    }
}

impl std::error::Error for InvalidCheckout {}

impl From<InvalidCheckout> for rituals::Failure {
    fn from(error: InvalidCheckout) -> Self {
        Self::new(error.to_string())
    }
}

/// Escapes `value` for use inside a TOML basic string.
///
/// Every renderer that writes a source's path or URL into a manifest goes
/// through this, so a path with a quote, a backslash or a control character
/// in it still parses as one string.
///
/// TOML 1.0's basic-string rule (the `toml.io` spec's own "String" section):
/// `\b` (U+0008), `\t`, `\n`, `\f` (U+000C), `\r`, `\"`, `\\`, and `\uXXXX`
/// for every other character in U+0000–U+001F and for U+007F. Everything
/// else — including non-ASCII — passes through unescaped: a basic string is
/// valid UTF-8 and needs no more than that.
///
/// # Examples
///
/// ```
/// use rituals_compose::source::escape_toml_string;
///
/// assert_eq!(escape_toml_string(r"C:\ritual"), r"C:\\ritual");
/// assert_eq!(escape_toml_string(r#"a "b""#), r#"a \"b\""#);
/// assert_eq!(escape_toml_string("tab\there"), r"tab\there");
/// assert_eq!(escape_toml_string("café"), "café");
/// ```
#[must_use]
pub fn escape_toml_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\u{8}' => escaped.push_str("\\b"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\u{c}' => escaped.push_str("\\f"),
            '\r' => escaped.push_str("\\r"),
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            other if matches!(other as u32, 0x00..=0x1f) || other == '\u{7f}' => {
                let _ = write!(escaped, "\\u{:04x}", other as u32);
            }
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use rituals::clap::{self, Parser};

    use super::{Source, SourceArguments, assert_is_a_ritual_checkout};
    use crate::test_support::{ScratchDir, TestOutcome};

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[test]
    fn source_is_send_and_sync() {
        assert_send::<Source>();
        assert_sync::<Source>();
    }

    /// A minimal command line to parse [`SourceArguments`] through, the way
    /// a real task flattens it via `#[command(flatten)]`.
    #[derive(clap::Parser)]
    struct Command {
        #[command(flatten)]
        source: SourceArguments,
    }

    fn try_parse(arguments: &[&str]) -> Result<SourceArguments, clap::Error> {
        Command::try_parse_from(std::iter::once(&"program").chain(arguments))
            .map(|command| command.source)
    }

    fn parse(arguments: &[&str]) -> SourceArguments {
        Command::parse_from(std::iter::once(&"program").chain(arguments)).source
    }

    #[test]
    fn neither_flag_resolves_to_the_registry() {
        assert_eq!(parse(&[]).resolve(), Source::Registry);
    }

    /// The refusal [`SourceArguments::resolve`]'s `unreachable!` arm rests
    /// on: clap rejects the pair as an argument conflict before any task
    /// code sees it, in either order.
    #[test]
    fn both_flags_are_refused_by_clap_as_a_conflict() {
        for arguments in [
            ["--path", "/somewhere", "--git", "https://example.invalid/x"],
            ["--git", "https://example.invalid/x", "--path", "/somewhere"],
        ] {
            let parsed = try_parse(&arguments);
            assert!(
                matches!(
                    &parsed,
                    Err(error) if error.kind() == clap::error::ErrorKind::ArgumentConflict
                ),
                "expected {arguments:?} to be refused as an argument conflict, got {parsed:?}"
            );
        }
    }

    #[test]
    fn path_alone_resolves_to_a_path_source() {
        let resolved = parse(&["--path", "/somewhere"]).resolve();
        assert_eq!(
            resolved,
            Source::Path(std::path::PathBuf::from("/somewhere"))
        );
    }

    #[test]
    fn git_alone_resolves_to_a_git_source() {
        let resolved = parse(&["--git", "https://example.invalid/x"]).resolve();
        assert_eq!(
            resolved,
            Source::Git("https://example.invalid/x".to_string())
        );
    }

    /// A checkout with the `rituals` crate and without ritual's own tasks in
    /// `.rituals/ritual` is refused, naming the manifest it lacks, and one
    /// with both is accepted.
    #[test]
    fn a_checkout_without_ritual_s_own_tasks_is_refused_naming_their_manifest() -> TestOutcome {
        let scratch = ScratchDir::new("source-no-bundle")?;
        let root = scratch.path();
        std::fs::create_dir_all(root.join("crates/rituals"))?;
        std::fs::write(root.join("crates/rituals/Cargo.toml"), "[package]\n")?;

        let message = assert_is_a_ritual_checkout(root)
            .err()
            .ok_or("a checkout without .rituals/ritual must be refused")?
            .to_string();
        assert!(message.contains(".rituals/ritual/Cargo.toml"), "{message}");
        assert!(message.contains(&root.display().to_string()), "{message}");

        std::fs::create_dir_all(root.join(".rituals/ritual"))?;
        std::fs::write(root.join(".rituals/ritual/Cargo.toml"), "[package]\n")?;
        assert_eq!(assert_is_a_ritual_checkout(root), Ok(()));
        Ok(())
    }

    #[test]
    fn a_directory_without_the_ritual_manifest_is_refused_naming_it() {
        let result = assert_is_a_ritual_checkout(&std::env::temp_dir());
        assert!(result.is_err(), "expected a bare temp dir to be refused");
        if let Err(error) = result {
            let message = error.to_string();
            assert!(message.to_lowercase().contains("ritual checkout"));
            assert!(message.contains("crates/rituals/Cargo.toml"));
        }
    }

    /// The root of the workspace this crate is built as a member of, when
    /// that is a ritual checkout: what `cargo locate-project --workspace`
    /// answers from this crate's directory, when that is some directory
    /// other than the crate's own and it carries `crates/rituals/Cargo.toml`.
    /// Cargo decides membership, however the root's `members` spells it.
    ///
    /// The marker is tested here with its literal path rather than through
    /// [`assert_is_a_ritual_checkout`] or [`RITUALS_MANIFEST_IN_CHECKOUT`],
    /// which are what the test below checks: gating on them would skip the
    /// test exactly when they are wrong. Otherwise returns why not.
    fn enclosing_checkout() -> Result<std::path::PathBuf, String> {
        let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let located = crate::workspace::locate_project(crate_dir, true)
            .map_err(|failure| format!("cargo could not be asked: {failure}"))?;
        let manifest = match located {
            crate::workspace::Located::Found(manifest) => manifest,
            crate::workspace::Located::NotFound(stderr) => {
                return Err(format!("cargo found no workspace for this crate: {stderr}"));
            }
        };
        let root = manifest
            .parent()
            .ok_or_else(|| format!("{} has no directory", manifest.display()))?;
        let same = |left: &std::path::Path, right: &std::path::Path| {
            left.canonicalize()
                .ok()
                .is_some_and(|left| right.canonicalize().ok() == Some(left))
        };
        if same(root, crate_dir) {
            return Err("this crate is its own workspace, not a member of ritual's".to_string());
        }
        if !root.join("crates/rituals/Cargo.toml").is_file() {
            return Err(format!(
                "the workspace at {} is not a ritual checkout: it has no crates/rituals/Cargo.toml",
                root.display()
            ));
        }
        Ok(root.to_path_buf())
    }

    /// Needs this crate built inside the repository's own workspace; a
    /// build of the crate on its own, from its published package, or inside
    /// some other workspace, skips out loud.
    #[test]
    fn a_real_checkout_root_is_accepted() {
        let checkout_root = match enclosing_checkout() {
            Ok(root) => root,
            Err(reason) => {
                crate::test_support::report_skip(&format!(
                    "a_real_checkout_root_is_accepted needs this crate built inside \
                     ritual's own workspace: {reason}"
                ));
                return;
            }
        };

        let result = assert_is_a_ritual_checkout(&checkout_root);
        assert!(
            result.is_ok(),
            "expected {} to be accepted as a ritual checkout: {result:?}",
            checkout_root.display()
        );
    }

    #[test]
    fn from_invalid_checkout_keeps_the_message_verbatim() {
        let checked = assert_is_a_ritual_checkout(&std::env::temp_dir());
        assert!(
            checked.is_err(),
            "a bare temp dir must be refused as a checkout"
        );

        if let Err(invalid_checkout) = checked {
            let expected = invalid_checkout.to_string();
            let failure = rituals::Failure::from(invalid_checkout);
            assert_eq!(failure.to_string(), expected);
        }
    }
}
