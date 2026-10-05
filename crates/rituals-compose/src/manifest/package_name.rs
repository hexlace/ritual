//! The name a manifest gives its package.

use toml_edit::Item;

use super::Manifest;

impl Manifest {
    /// The `name` in this manifest's `[package]` table, or `None` when it has
    /// no `[package]` table, as a manifest that only declares a workspace has
    /// none, or when `name` is not a string.
    #[must_use]
    pub(crate) fn package_name(&self) -> Option<&str> {
        self.document
            .get("package")
            .and_then(Item::as_table_like)
            .and_then(|package| package.get("name"))
            .and_then(Item::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::Manifest;
    use crate::test_support::{ScratchDir, TestOutcome};

    /// The package name of a manifest holding `content`.
    fn named_by(tag: &str, content: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let scratch = ScratchDir::new(tag)?;
        let path = scratch.path().join("Cargo.toml");
        std::fs::write(&path, content)?;
        Ok(Manifest::read(&path)?.package_name().map(str::to_string))
    }

    #[test]
    fn a_package_is_named_by_its_name() -> TestOutcome {
        let name = named_by("package-name-given", "[package]\nname = \"greet\"\n")?;

        assert_eq!(name.as_deref(), Some("greet"));
        Ok(())
    }

    #[test]
    fn a_package_written_as_a_dotted_key_is_named_too() -> TestOutcome {
        let name = named_by("package-name-dotted", "package.name = \"greet\"\n")?;

        assert_eq!(name.as_deref(), Some("greet"));
        Ok(())
    }

    #[test]
    fn a_manifest_with_no_package_has_no_name() -> TestOutcome {
        let name = named_by("package-name-virtual", "[workspace]\nmembers = []\n")?;

        assert_eq!(name, None);
        Ok(())
    }

    #[test]
    fn a_name_that_is_not_a_string_is_no_name() -> TestOutcome {
        let name = named_by("package-name-not-a-string", "[package]\nname = 3\n")?;

        assert_eq!(name, None);
        Ok(())
    }
}
