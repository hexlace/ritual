//! The steps `migrate` runs, in the order their releases came.
//!
//! A step is a value that knows whether it applies to a project. `migrate`
//! asks every step in turn and runs each one that does, and stores nothing
//! between runs: there is no recorded version to go stale, and a project a
//! step has already brought up to date is simply one it no longer applies
//! to.

use std::path::Path;

use rituals::Failure;
use rituals_compose::metadata::Metadata;
use rituals_compose::rollback::Changes;

use crate::precondition::Repository;
use crate::tasks_into_rituals::{self, Candidates};
use crate::tidy::Vacated;

/// A change a ritual release made to what a project should look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// 0.2.0: a project's own tasks move from `tasks/` to `.rituals/`.
    TasksIntoRituals,
}

impl Step {
    /// Every step, oldest release first. A new release's step goes last: a
    /// step may rely on the layout every earlier one leaves.
    pub(crate) const IN_RELEASE_ORDER: [Self; 1] = [Self::TasksIntoRituals];

    /// What this step would do to the project `document` describes, or
    /// `None` when the project is already as the step leaves it.
    pub(crate) fn applies(self, document: &Metadata, root: &Path) -> Option<Migration> {
        match self {
            Self::TasksIntoRituals => {
                tasks_into_rituals::find(document, root).map(Migration::TasksIntoRituals)
            }
        }
    }
}

/// A step that applies, with what it found.
pub(crate) enum Migration {
    TasksIntoRituals(Candidates),
}

/// What a step needs to know about the run it is part of.
pub(crate) struct Context<'a> {
    /// The workspace's root directory, as Cargo reports it.
    pub(crate) root: &'a Path,
    /// The package whose command line is running `migrate`.
    pub(crate) package: &'a str,
    /// What a person types to run `migrate` again.
    pub(crate) migrate_command: &'a str,
    /// The repository the project is in, known to be clean.
    pub(crate) repository: &'a Repository,
}

/// What a step did, once it has been checked.
pub(crate) struct Applied {
    /// What to report, in the order to report it.
    pub(crate) lines: Vec<String>,
    /// What the step moved things out of, to be tidied once the run is kept.
    pub(crate) vacated: Vec<Vacated>,
    /// The project as it is now, for the next step to read.
    pub(crate) after: Metadata,
}

impl Migration {
    /// Plans the step, which refuses before anything is written, writes it,
    /// and checks the result, all recorded in `changes` so a failure at any
    /// point is put back.
    pub(crate) fn apply(
        self,
        context: &Context<'_>,
        before: &Metadata,
        changes: &mut Changes,
    ) -> Result<Applied, Failure> {
        match self {
            Self::TasksIntoRituals(candidates) => {
                tasks_into_rituals::apply(&candidates, context, before, changes)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rituals_compose::metadata;

    use super::Step;
    use crate::test_support::{ScratchDir, TestOutcome, workspace};

    fn applies(root: &std::path::Path) -> Result<bool, Box<dyn std::error::Error>> {
        let document = metadata::fetch(root)?;
        Ok(Step::TasksIntoRituals.applies(&document, root).is_some())
    }

    #[test]
    fn the_steps_run_oldest_release_first() {
        assert_eq!(Step::IN_RELEASE_ORDER, [Step::TasksIntoRituals]);
    }

    #[test]
    fn a_task_in_tasks_applies() -> TestOutcome {
        let scratch = ScratchDir::new("step-applies")?;
        workspace(scratch.path(), &["tasks/greet"], &["tasks/greet"], &[])?;
        assert!(applies(scratch.path())?);
        Ok(())
    }

    #[test]
    fn a_task_in_dot_rituals_does_not() -> TestOutcome {
        let scratch = ScratchDir::new("step-new-layout")?;
        workspace(
            scratch.path(),
            &[".rituals/greet"],
            &[".rituals/greet"],
            &[],
        )?;
        assert!(!applies(scratch.path())?);
        Ok(())
    }

    #[test]
    fn a_crate_in_tasks_that_is_not_a_task_does_not() -> TestOutcome {
        let scratch = ScratchDir::new("step-plain")?;
        workspace(
            scratch.path(),
            &["tasks/helper"],
            &["tasks/helper"],
            &["tasks/helper"],
        )?;
        assert!(!applies(scratch.path())?);
        Ok(())
    }

    #[test]
    fn a_task_elsewhere_does_not() -> TestOutcome {
        let scratch = ScratchDir::new("step-elsewhere")?;
        workspace(scratch.path(), &["tools/greet"], &["tools/greet"], &[])?;
        assert!(!applies(scratch.path())?);
        Ok(())
    }

    /// A directory called `tasks-extra` shares a prefix with `tasks` and is
    /// not under it.
    #[test]
    fn a_task_in_a_directory_sharing_the_prefix_does_not() -> TestOutcome {
        let scratch = ScratchDir::new("step-prefix")?;
        workspace(
            scratch.path(),
            &["tasks-extra/greet"],
            &["tasks-extra/greet"],
            &[],
        )?;
        assert!(!applies(scratch.path())?);
        Ok(())
    }

    #[test]
    fn a_project_with_no_members_but_its_own_package_does_not() -> TestOutcome {
        let scratch = ScratchDir::new("step-no-tasks")?;
        workspace(scratch.path(), &["ritual"], &["ritual"], &["ritual"])?;
        assert!(!applies(scratch.path())?);
        Ok(())
    }

    #[test]
    fn a_project_with_a_task_in_each_layout_applies_for_the_old_one() -> TestOutcome {
        let scratch = ScratchDir::new("step-both")?;
        workspace(
            scratch.path(),
            &[".rituals/new", "tasks/old"],
            &[".rituals/new", "tasks/old"],
            &[],
        )?;
        assert!(applies(scratch.path())?);
        Ok(())
    }
}
