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
/// it as `<bin name>: <message>` and exits with status 1, so a message says
/// what went wrong and what to do about it, without a prefix of its own.
///
/// [`Display`](fmt::Display) renders `<message>`, or `<message>: <cause>`
/// when a cause is attached. The cause also stays reachable through
/// [`std::error::Error::source`] for a caller that wants the typed value.
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
// Display carries the cause so that anything embedding a `Failure` in a
// longer message carries the cause by construction and nothing prints it a
// second time.
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
    /// let failure = Failure::new("tasks/lint already exists");
    /// assert_eq!(failure.to_string(), "tasks/lint already exists");
    /// ```
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// Attaches `cause` as this refusal's underlying error.
    ///
    /// The cause becomes part of this failure's own [`Display`](fmt::Display)
    /// — `<message>: <cause>` — and stays reachable on its own through
    /// [`std::error::Error::source`] for a caller that wants the typed
    /// value rather than its rendered text.
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
    /// let failure = Failure::new("writing tasks/lint/Cargo.toml failed").caused_by(IoLike);
    /// assert!(std::error::Error::source(&failure).is_some());
    /// assert_eq!(
    ///     failure.to_string(),
    ///     "writing tasks/lint/Cargo.toml failed: disk full"
    /// );
    /// ```
    #[must_use]
    pub fn caused_by(mut self, cause: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(cause));
        self
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.source {
            Some(cause) => write!(formatter, "{}: {cause}", self.message),
            None => formatter.write_str(&self.message),
        }
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
    fn display_carries_the_cause_when_one_is_attached_and_omits_it_otherwise() {
        #[derive(Debug)]
        struct Cause;
        impl std::fmt::Display for Cause {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("the underlying cause")
            }
        }
        impl Error for Cause {}

        let caused = Failure::new("writing failed").caused_by(Cause);
        assert_eq!(caused.to_string(), "writing failed: the underlying cause");

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
