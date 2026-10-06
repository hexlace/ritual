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
/// with status 1, or the [`RefusalStatus`] set by [`Failure::exiting_with`].
/// So a message says what went wrong and what to do about it, without a
/// prefix of its own.
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
// the message is for a person reading stderr and is the contract, a taxonomy
// of types nothing in-process branches on would be surface with no use, and
// the one thing a caller does branch on, the exit status, is a value here.
// Display is the message alone so that a `Failure` can itself be a cause:
// anything that walks the chain, the refusal line included, then prints each
// link once, however deep it goes.
#[derive(Debug)]
pub struct Failure {
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
    status: RefusalStatus,
}

/// The exit status a refusal ends the process with: 1, or anything from 3
/// to 125.
///
/// A task that has more than one way to refuse can give each its own status,
/// so a caller, such as a CI job, can tell them apart without reading stderr:
/// "the check ran and found differences" and "the check could not run" are
/// two different things to retry or not. Attach one with
/// [`Failure::exiting_with`]; a `Failure` without one exits with 1, the
/// [`Default`].
///
/// [`RefusalStatus::new`] is the only constructor, and it refuses the
/// statuses that already mean something else to whoever runs the command:
///
/// - **0** is success, and a refusal is never a success.
/// - **2** is what the command line exits with on a usage error, such as an
///   unknown flag, which clap reports before any task runs.
/// - **126 and above** are what a shell reports for a command it found but
///   could not run (126), one it could not find (127), and one ended by a
///   signal (128 plus the signal's number).
///
/// # Examples
///
/// ```
/// use rituals::{Failure, Outcome, RefusalStatus};
///
/// /// The status a check exits with when it ran and found differences.
/// const DIFFERENCES_FOUND: RefusalStatus = match RefusalStatus::new(3) {
///     Ok(status) => status,
///     Err(_) => panic!("3 is a refusal status"),
/// };
///
/// fn check(differences: usize) -> Outcome {
///     if differences > 0 {
///         return Err(Failure::new(format!("{differences} tasks differ from the declared set"))
///             .exiting_with(DIFFERENCES_FOUND));
///     }
///     Ok(())
/// }
///
/// assert_eq!(check(2).map_err(|failure| failure.status().get()), Err(3));
/// assert!(RefusalStatus::new(0).is_err());
/// assert!(RefusalStatus::new(2).is_err());
/// assert!(RefusalStatus::new(126).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RefusalStatus(u8);

/// The first status a shell reserves; every status from here up is its own.
const FIRST_SHELL_STATUS: u8 = 126;

impl RefusalStatus {
    /// Checks that `code` is a status only a refusal uses, and refuses it
    /// otherwise.
    ///
    /// `const`, so a task can name its statuses as constants and have a
    /// reserved one refused when it compiles.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRefusalStatus`] when `code` is 0, 2, or 126 or above,
    /// each of which means something else to whoever runs the command (see
    /// [`RefusalStatus`]).
    pub const fn new(code: u8) -> Result<Self, InvalidRefusalStatus> {
        let reason = match code {
            0 => Reserved::Success,
            2 => Reserved::UsageError,
            FIRST_SHELL_STATUS..=u8::MAX => Reserved::Shell,
            _ => return Ok(Self(code)),
        };
        Err(InvalidRefusalStatus { code, reason })
    }

    /// Returns this status as the number the process exits with.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl Default for RefusalStatus {
    /// Status 1, which every refusal exits with unless it chooses another.
    fn default() -> Self {
        Self(1)
    }
}

/// What a status that [`RefusalStatus::new`] refused already means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reserved {
    /// 0: the command succeeded.
    Success,
    /// 2: the command line was used wrongly, which clap reports.
    UsageError,
    /// 126 and above: a shell's own statuses.
    Shell,
}

/// Why a number was refused as a [`RefusalStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidRefusalStatus {
    code: u8,
    reason: Reserved,
}

impl fmt::Display for InvalidRefusalStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.reason {
            Reserved::Success => "0 means success",
            Reserved::UsageError => "2 is what a usage error exits with",
            Reserved::Shell => "126 and above are what a shell exits with",
        };
        write!(
            formatter,
            "{} cannot be a refusal's exit status; {reason}, so a refusal uses 1 or 3 to 125",
            self.code
        )
    }
}

impl std::error::Error for InvalidRefusalStatus {}

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
            status: RefusalStatus::default(),
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

    /// Makes the process exit with `status` when this refusal ends it,
    /// rather than 1. What it prints is unchanged.
    ///
    /// When one `Failure` is the cause of another, the outermost one's
    /// status is the one the process exits with: a `Failure` that wants its
    /// cause's status passes it on, with `.exiting_with(cause.status())`.
    ///
    /// # Examples
    ///
    /// ```
    /// use rituals::{Failure, RefusalStatus};
    ///
    /// let could_not_run = RefusalStatus::new(4)?;
    /// let failure = Failure::new("the registry did not answer").exiting_with(could_not_run);
    /// assert_eq!(failure.status().get(), 4);
    /// assert_eq!(failure.to_string(), "the registry did not answer");
    /// # Ok::<(), rituals::InvalidRefusalStatus>(())
    /// ```
    #[must_use]
    pub const fn exiting_with(mut self, status: RefusalStatus) -> Self {
        self.status = status;
        self
    }

    /// The status the process exits with when this refusal ends it: 1, or
    /// the one [`Failure::exiting_with`] set.
    #[must_use]
    pub const fn status(&self) -> RefusalStatus {
        self.status
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

    use super::{Failure, RefusalStatus};
    use crate::name::Name;

    /// A status the test knows is usable, built through the one constructor.
    fn status(code: u8) -> RefusalStatus {
        RefusalStatus::new(code).expect("a test passes only statuses it knows are usable")
    }

    #[test]
    fn a_plain_failure_exits_with_status_one() {
        assert_eq!(Failure::new("refused").status().get(), 1);
    }

    #[test]
    fn exiting_with_sets_the_status_and_leaves_the_message() {
        let failure = Failure::new("3 tasks differ from the declared set").exiting_with(status(3));
        assert_eq!(failure.status().get(), 3);
        assert_eq!(failure.to_string(), "3 tasks differ from the declared set");
    }

    #[test]
    fn the_outermost_failure_decides_the_status() {
        let inner = Failure::new("the registry did not answer").exiting_with(status(4));
        let outer = Failure::new("syncing failed").caused_by(inner);
        assert_eq!(outer.status().get(), 1);

        let inner = Failure::new("the registry did not answer").exiting_with(status(4));
        let passed_on = Failure::new("syncing failed")
            .exiting_with(inner.status())
            .caused_by(inner);
        assert_eq!(passed_on.status().get(), 4);
    }

    #[test]
    fn zero_is_success_and_cannot_be_a_refusal_status() {
        assert!(RefusalStatus::new(0).is_err());
    }

    #[test]
    fn two_is_claps_usage_error_and_cannot_be_a_refusal_status() {
        assert!(RefusalStatus::new(2).is_err());
    }

    #[test]
    fn one_hundred_and_twenty_six_and_above_belong_to_the_shell() {
        for code in [126, 127, 128, 130, 255] {
            assert!(RefusalStatus::new(code).is_err(), "{code} must be refused");
        }
    }

    #[test]
    fn one_and_three_to_one_hundred_and_twenty_five_are_refusal_statuses() {
        let usable = std::iter::once(1).chain(3..=125);
        for code in usable {
            assert_eq!(RefusalStatus::new(code).map(RefusalStatus::get), Ok(code));
        }
    }

    #[test]
    fn a_refused_status_says_which_code_and_why() {
        let reasons = [
            (0, "0 means success"),
            (2, "2 is what a usage error exits with"),
            (126, "126 and above are what a shell exits with"),
        ];
        for (code, reason) in reasons {
            let refused = RefusalStatus::new(code).map(RefusalStatus::get);
            assert!(
                refused
                    .as_ref()
                    .is_err_and(|error| error.to_string().contains(reason)),
                "expected {code} to be refused because {reason}: {refused:?}"
            );
        }
    }

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
