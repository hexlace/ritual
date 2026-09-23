//! One line of a composed command line's generated task list.

/// One line of a composed command line's generated list.
///
/// Two fields and not one: the command name may contain hyphens, and the
/// extern-crate identifier Cargo gives rustc may not. That identifier is
/// read from `cargo metadata`, never derived here, because the mapping from
/// a dependency key to an extern-crate identifier is Cargo's rule rather
/// than ritual's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub(super) command: String,
    pub(super) extern_crate: String,
}

impl Entry {
    /// Pairs the command name a caller types with the extern-crate
    /// identifier that command's task is reached through.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::generated_file::Entry;
    ///
    /// let entry = Entry::new("check", "acme_linting");
    /// assert_eq!(entry, Entry::new("check".to_string(), "acme_linting".to_string()));
    /// ```
    pub fn new(command: impl Into<String>, extern_crate: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            extern_crate: extern_crate.into(),
        }
    }

    /// The command name this entry mounts — the dependency key, as a
    /// person types it.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals_compose::generated_file::Entry;
    ///
    /// let entry = Entry::new("check", "acme_linting");
    /// assert_eq!(entry.command(), "check");
    /// ```
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }
}

#[cfg(test)]
mod tests {
    use super::Entry;

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    #[test]
    fn entry_is_send_and_sync() {
        assert_send::<Entry>();
        assert_sync::<Entry>();
    }
}
