//! What a task returns, and the refusal message a caller reads when it does
//! not succeed.

use std::fmt;

use crate::name::InvalidName;

/// What running a task, or any of the framework's own fallible operations,
/// produces: nothing on success, or a [`Failure`] naming what went wrong.
pub type Outcome = Result<(), Failure>;

/// A refusal: the message a person reads when a task cannot do its job.
///
/// Build one with [`Failure::new`], and chain [`Failure::caused_by`] when
/// there is an underlying error to attach. The composed command line prints
/// it as `<bin name>: <message>`, followed by each cause in turn, and exits
/// with status 1, so a message says what went wrong and what to do about it,
/// without a prefix of its own.
///
/// It follows the usual convention for an error: [`Display`](fmt::Display)
/// renders the message alone, and the cause is reachable through
/// [`std::error::Error::source`]. [`Failure::with_causes`] renders the whole
/// chain, `<message>: <cause>: <its cause>…`, the way the refusal line does,
/// for code that writes a `Failure` into text of its own.
///
/// # Examples
///
/// ```
/// use rituals::{Failure, Outcome};
///
/// fn ensure_clean(uncommitted: usize) -> Outcome {
///     if uncommitted > 0 {
///         return Err(Failure::new(format!(
///             "{uncommitted} files have uncommitted changes; commit or stash them first"
///         )));
///     }
///     Ok(())
/// }
///
/// assert!(ensure_clean(0).is_ok());
/// assert!(ensure_clean(2).is_err());
/// ```
//
// One type for every refusal rather than an error type per failure mode:
// its only consumer is a person reading stderr, a taxonomy nothing branches
// on would be surface with no use, and the message itself is the contract.
// Display is the message alone so that a `Failure` can itself be a cause:
// anything that walks the chain, the refusal line included, then prints each
// link once, however deep it goes.
#[derive(Debug)]
pub struct Failure {
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Failure {
    /// Builds a refusal carrying `message`, the text a caller reads.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals::Failure;
    ///
    /// let failure = Failure::new(".rituals/lint already exists");
    /// assert_eq!(failure.to_string(), ".rituals/lint already exists");
    /// ```
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// Attaches `cause` as this refusal's underlying error.
    ///
    /// The cause is this failure's [`source`](std::error::Error::source),
    /// and stays out of its [`Display`](fmt::Display), which is the message
    /// alone. The refusal line names it after the message, and then the
    /// cause's own source, and so on down the chain, so a cause whose own
    /// `Display` names only its own situation, as the convention is, still
    /// reaches the person reading it in full. [`Failure::with_causes`]
    /// renders that same line without the bin name.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals::Failure;
    ///
    /// #[derive(Debug)]
    /// struct IoLike;
    ///
    /// impl std::fmt::Display for IoLike {
    ///     fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    ///         formatter.write_str("disk full")
    ///     }
    /// }
    /// impl std::error::Error for IoLike {}
    ///
    /// let failure = Failure::new("writing .rituals/lint/Cargo.toml failed").caused_by(IoLike);
    /// assert!(std::error::Error::source(&failure).is_some());
    /// assert_eq!(failure.to_string(), "writing .rituals/lint/Cargo.toml failed");
    /// assert_eq!(
    ///     failure.with_causes().to_string(),
    ///     "writing .rituals/lint/Cargo.toml failed: disk full"
    /// );
    /// ```
    #[must_use]
    pub fn caused_by(mut self, cause: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(cause));
        self
    }

    /// Renders this refusal with every cause under it: the message, then
    /// each [`source`](std::error::Error::source) in turn, joined with `: `.
    ///
    /// This is the refusal line without the `<bin name>: ` in front, for
    /// code that writes a `Failure` into text of its own, such as a longer
    /// message. Each link is rendered by its own
    /// [`Display`](fmt::Display), so a cause that follows the convention
    /// (its own situation only, the rest through `source()`) appears once.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals::Failure;
    ///
    /// let failure = Failure::new("importing lint failed")
    ///     .caused_by(Failure::new("regenerating the command line failed"));
    /// assert_eq!(
    ///     format!("{}; run regenerate to finish", failure.with_causes()),
    ///     "importing lint failed: regenerating the command line failed; run regenerate to finish"
    /// );
    /// ```
    #[must_use]
    pub fn with_causes(&self) -> impl fmt::Display + '_ {
        WithCauses(self)
    }
}

/// A [`Failure`] rendered with its whole chain of causes, as
/// [`Failure::with_causes`] returns it.
struct WithCauses<'failure>(&'failure Failure);

impl fmt::Display for WithCauses<'_> {
    // No bound on the walk: each link in an error chain owns or borrows the
    // next from something it holds, so the chain ends where an error has no
    // source.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0.message)?;
        let mut cause = std::error::Error::source(self.0);
        while let Some(link) = cause {
            write!(formatter, ": {link}")?;
            cause = link.source();
        }
        Ok(())
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|error| error as &(dyn std::error::Error + 'static))
    }
}

impl From<InvalidName> for Failure {
    fn from(error: InvalidName) -> Self {
        Self::new(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::Failure;
    use crate::name::Name;

    #[test]
    fn display_is_the_message() {
        let failure = Failure::new("tasks/lint already exists");
        assert_eq!(failure.to_string(), "tasks/lint already exists");
    }

    #[test]
    fn source_is_absent_until_caused_by_is_called() {
        let failure = Failure::new("something went wrong");
        assert!(failure.source().is_none());
    }

    #[test]
    fn source_is_the_cause_after_caused_by() {
        #[derive(Debug)]
        struct Cause;
        impl std::fmt::Display for Cause {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("the underlying cause")
            }
        }
        impl Error for Cause {}

        let failure = Failure::new("writing failed").caused_by(Cause);
        let source = failure.source();
        assert!(source.is_some(), "a cause was attached");
        if let Some(source) = source {
            assert_eq!(source.to_string(), "the underlying cause");
        }
    }

    #[test]
    fn display_is_the_message_alone_whether_or_not_a_cause_is_attached() {
        #[derive(Debug)]
        struct Cause;
        impl std::fmt::Display for Cause {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("the underlying cause")
            }
        }
        impl Error for Cause {}

        let caused = Failure::new("writing failed").caused_by(Cause);
        assert_eq!(caused.to_string(), "writing failed");

        let plain = Failure::new("writing failed");
        assert_eq!(plain.to_string(), "writing failed");
    }

    #[test]
    fn from_invalid_name_keeps_the_message_verbatim() {
        assert!(Name::new("../evil").is_err(), "`../evil` must be refused");

        if let Err(invalid_name) = Name::new("../evil") {
            let expected = invalid_name.to_string();
            let failure = Failure::from(invalid_name);
            assert_eq!(failure.to_string(), expected);
        }
    }
}
