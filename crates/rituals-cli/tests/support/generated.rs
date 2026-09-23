//! Reading a composed CLI's generated command-line file by its tokens.
//!
//! `regenerate` lays the file out one way, but what a story checks is what
//! the file says: which call it makes and which tasks it mounts, under which
//! keys, in which order. Reading with whitespace removed keeps those checks
//! about the tokens rather than about indentation or line breaks.

/// `text` with every whitespace character removed.
pub(crate) fn tokens(text: &str) -> String {
    text.split_whitespace().collect()
}

/// Every `("<key>", <crate>::task())` entry the generated file mounts, as
/// `(key, crate)` pairs, in the order the file lists them.
pub(crate) fn mounted_entries(generated_file: &str) -> Vec<(String, String)> {
    let tokens = tokens(generated_file);
    let mut entries = Vec::new();
    let mut rest = tokens.as_str();
    while let Some(start) = rest.find("(\"") {
        let after_open = &rest[start + 2..];
        let Some((key, after_key)) = after_open.split_once("\",") else {
            break;
        };
        match after_key.split_once("::task())") {
            Some((crate_identifier, after_entry))
                if crate_identifier
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_') =>
            {
                entries.push((key.to_string(), crate_identifier.to_string()));
                rest = after_entry;
            }
            _ => rest = after_open,
        }
    }
    entries
}
