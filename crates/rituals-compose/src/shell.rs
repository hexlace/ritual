//! Rendering a command for a person to copy into a POSIX shell.
//!
//! Every line ritual prints that hands back a command, a retry or a next
//! step, renders it here, so each hint quotes the same way and one that
//! holds a path with a space in it still runs when pasted.

/// Renders `words` as one command line for a POSIX shell, separated by single
/// spaces.
///
/// A word is left as it is when it is not empty and every byte in it is a
/// letter, a digit or one of `/ . _ + @ : = , -`, which no shell treats
/// specially. Any other word is wrapped in single quotes, with each `'`
/// inside it written as `'\''`, so a shell reads back exactly the words that
/// went in. The whole command is joined here rather than one word at a time
/// because a caller that quotes word by word can leave one unquoted, and
/// this cannot.
///
/// # Examples
///
/// A task directory with a space in its name still pastes as one argument:
///
/// ```
/// use rituals_compose::shell;
///
/// let typed = shell::join(["import", "lint", "--path", "/work/my tasks/lint"]);
///
/// assert_eq!(typed, "import lint --path '/work/my tasks/lint'");
/// ```
#[must_use]
pub fn join<I>(words: I) -> String
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let rendered: Vec<String> = words.into_iter().map(|word| quote(word.as_ref())).collect();
    rendered.join(" ")
}

/// The punctuation, beyond ASCII letters and digits, that no POSIX shell
/// reads as anything but part of a word.
///
/// A leading `-` is safe too: a shell passes it on untouched, and what reads
/// it as an option is the program, which is the reader's own to meet.
const UNQUOTED_PUNCTUATION: &str = "/._+@:=,-";

/// Renders one word so a POSIX shell reads it back as that word alone.
///
/// Single quotes are the one quoting a shell does not look inside, so the
/// only character that needs care within them is the single quote itself: it
/// closes the quotes, is written as an escaped quote of its own, and reopens
/// them.
fn quote(word: &str) -> String {
    // An empty word has no characters to read as plain, and has to be
    // written as a pair of quotes or the shell would drop it.
    if word.is_empty() {
        return "''".to_string();
    }
    if word.chars().all(is_plain) {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// Whether a shell reads `character` as part of a word with no quoting.
fn is_plain(character: char) -> bool {
    if character.is_ascii_alphanumeric() {
        return true;
    }
    UNQUOTED_PUNCTUATION.contains(character)
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::join;
    use crate::test_support::TestOutcome;

    /// The words a POSIX shell reads out of `command`, found by asking one:
    /// `sh` runs `printf` on the rendered command, which prints each argument
    /// followed by a NUL byte, so any byte inside a word, a newline
    /// included, survives the trip.
    fn words_a_shell_reads(command: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let output = Command::new("sh")
            .arg("-c")
            .arg(format!("printf '%s\\0' {command}"))
            .output()?;
        assert!(
            output.status.success(),
            "sh refused `{command}`: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout)?;
        Ok(text
            .strip_suffix('\0')
            .map(|text| text.split('\0').map(str::to_string).collect())
            .unwrap_or_default())
    }

    #[test]
    fn a_shell_reads_back_exactly_the_words_that_went_in() -> TestOutcome {
        // Each row is a word a person could plausibly be handing back: a
        // plain one, then every kind of character a shell reads specially.
        let words = [
            "import",
            "greeter@0.1.0",
            "--git",
            "https://example.com/hexlace/ritual.git",
            "/work/my tasks/lint",
            "it's",
            "say \"hi\"",
            "$HOME",
            "`date`",
            "back\\slash",
            "a*b?c[d]",
            "one;two",
            "a&b|c",
            "~tilde",
            "#hash",
            "!bang",
            "line\nbreak",
            "tab\there",
            "naïve-café",
            "日本語",
            "",
            "'",
            "''",
            "-",
        ];

        let command = join(words);

        assert_eq!(words_a_shell_reads(&command)?, words);
        Ok(())
    }

    #[test]
    fn a_word_made_only_of_safe_characters_is_left_as_it_is() {
        assert_eq!(
            join(["a-Z_0.9/x+y@z:k=v,w"]),
            "a-Z_0.9/x+y@z:k=v,w",
            "no quoting where none is needed"
        );
    }

    #[test]
    fn a_word_with_a_space_is_wrapped_in_single_quotes() {
        assert_eq!(join(["my tasks"]), "'my tasks'");
    }

    #[test]
    fn a_single_quote_inside_a_word_is_closed_escaped_and_reopened() {
        assert_eq!(join(["it's"]), r"'it'\''s'");
    }

    #[test]
    fn an_empty_word_is_a_pair_of_quotes_so_it_is_still_an_argument() {
        assert_eq!(join(["a", "", "b"]), "a '' b");
    }

    #[test]
    fn no_words_render_as_nothing() {
        assert_eq!(join(Vec::<String>::new()), "");
    }

    #[test]
    fn owned_and_borrowed_words_render_alike() {
        let owned = vec!["import".to_string(), "my tasks".to_string()];

        assert_eq!(join(&owned), join(["import", "my tasks"]));
    }
}
