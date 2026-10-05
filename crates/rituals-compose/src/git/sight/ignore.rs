//! Which of a list of paths git ignores, and by which rule.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::outcome::IgnoreRule;
use super::scoped;
use crate::git::{Unanswered, failure_of, nul_terminated, run_git_for_output};

/// What `git check-ignore -v -n --no-index -z --stdin` is asked, after the
/// leading options that choose which tree it asks about.
///
/// `--no-index` answers by the rules alone: whether a path is tracked is a
/// separate question, and a tracked file that a rule matches is still
/// tracked. `-n` makes git answer for every path, with empty fields for one
/// no rule matches, so the answers line up with the questions.
const CHECK_IGNORE: [&str; 6] = ["check-ignore", "-v", "-n", "--no-index", "-z", "--stdin"];

/// Asks which of `paths`, each spelled from the top level of the work tree
/// `directory` stands in, git ignores there, and by which rule, in the
/// order given.
///
/// `scope` is any leading options that choose the tree: none for the real
/// one, and `--git-dir` with `--work-tree` for a stand-in that holds the
/// ignore files where they will be. A rule that starts with `!` is a
/// negation and says the path is not ignored. A path that ends in `/` is a
/// directory, so a pattern that only matches a directory matches it.
pub(super) fn ask(
    new_git: &impl Fn() -> Command,
    directory: &Path,
    scope: &[String],
    paths: &[String],
) -> Result<Vec<Option<IgnoreRule>>, Unanswered> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let arguments = scoped(scope, &CHECK_IGNORE);
    let input = nul_terminated(paths.iter().map(String::as_bytes));
    let output = run_git_for_output(new_git, directory, &arguments, &input)?;
    // Exit 0 says at least one path is ignored and exit 1 that none is: both
    // are answers, and anything else is git failing.
    match output.status.code() {
        Some(0 | 1) => parse_answers(&output.stdout, paths),
        Some(_) | None => Err(failure_of(&output)),
    }
}

/// Every answer in `git check-ignore -v -n -z` output, for the `asked`
/// paths, in order.
///
/// Each answer is four NUL-ended fields: the file the matching rule is in,
/// its line, the pattern, and the path asked about. All of the first three
/// are empty when no rule matches. Output that does not come in fours, that
/// answers a different number of paths, or that names another path than the
/// one asked about at its place, is a failure, since misreading it could
/// pair a file with another's rule.
pub(super) fn parse_answers(
    output: &[u8],
    asked: &[String],
) -> Result<Vec<Option<IgnoreRule>>, Unanswered> {
    let text = String::from_utf8_lossy(output);
    let unreadable = || {
        Unanswered::Failed(format!(
            "git check-ignore printed output this check cannot read: {text:?}"
        ))
    };
    let fields: Vec<&str> = text.split_terminator('\0').collect();
    let answers = fields.chunks_exact(4);
    if !answers.remainder().is_empty() || answers.len() != asked.len() {
        return Err(unreadable());
    }
    let mut rules = Vec::with_capacity(asked.len());
    for (answer, path) in answers.zip(asked) {
        let [source, line, pattern, answered] = answer else {
            unreachable!("chunks_exact(4) yields four fields");
        };
        if answered != path {
            return Err(unreadable());
        }
        if source.is_empty() {
            rules.push(None);
            continue;
        }
        let line: u32 = line.parse().map_err(|_| unreadable())?;
        // A pattern that starts with `!` re-includes what an earlier one
        // ignored, so the path it matched is not ignored.
        rules.push(
            (!pattern.starts_with('!'))
                .then(|| IgnoreRule::new(PathBuf::from(source), line, (*pattern).to_string())),
        );
    }
    Ok(rules)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::parse_answers;
    use crate::git::Unanswered;
    use crate::git::sight::outcome::IgnoreRule;

    fn asked(paths: &[&str]) -> Vec<String> {
        paths.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn answers_are_read_in_fours_with_empty_fields_for_a_path_no_rule_matches() {
        let output = b".gitignore\x001\0vendor/\0tasks/greet/vendor/up/\0\
            \0\0\0tasks/greet/src/lib.rs\0\
            .gitignore\x002\0*.log\0tasks/greet/a.log\0";

        let rules = parse_answers(
            output,
            &asked(&[
                "tasks/greet/vendor/up/",
                "tasks/greet/src/lib.rs",
                "tasks/greet/a.log",
            ]),
        );

        assert_eq!(
            rules,
            Ok(vec![
                Some(IgnoreRule::new(
                    PathBuf::from(".gitignore"),
                    1,
                    "vendor/".to_string()
                )),
                None,
                Some(IgnoreRule::new(
                    PathBuf::from(".gitignore"),
                    2,
                    "*.log".to_string()
                )),
            ])
        );
    }

    /// `-v` reports the rule that decided, and a negation is one: the path
    /// it names is not ignored.
    #[test]
    fn a_negated_pattern_says_the_path_is_not_ignored() {
        let output = b".gitignore\x003\0!.github\0.github/ci.yml\0";

        assert_eq!(
            parse_answers(output, &asked(&[".github/ci.yml"])),
            Ok(vec![None])
        );
    }

    #[test]
    fn output_this_cannot_read_is_a_failure_not_a_skip() {
        let one = asked(&["a"]);
        for (output, asked) in [
            // Not in fours.
            (&b".gitignore\x001\0a\0"[..], &one),
            // A line that is not a number.
            (&b".gitignore\0one\0*\0a\0"[..], &one),
            // Answers about another path than the one asked.
            (&b".gitignore\x001\0*\0b\0"[..], &one),
            // Fewer answers than questions, and more.
            (&b""[..], &one),
            (&b"\0\0\0a\0\0\0\0b\0"[..], &one),
        ] {
            let result = parse_answers(output, asked);
            assert!(
                matches!(
                    &result,
                    Err(Unanswered::Failed(message)) if message.contains("cannot read")
                ),
                "expected {output:?} to be refused, got {result:?}"
            );
        }
    }
}
