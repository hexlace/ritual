//! Deciding whether an extern-crate identifier needs a Rust raw-identifier
//! prefix.

/// Rust keywords and reserved words that cannot be written as a bare
/// identifier.
///
/// `crate`, `self`, `super` and `Self` are deliberately absent: the first
/// three cannot be written as a raw identifier at all (`r#crate` is not
/// valid syntax), and [`rituals::Name`] refuses all three, along with `Self`'s
/// uppercase letter, before a value ever reaches this list.
const RESERVED_WORDS: &[&str] = &[
    "as", "break", "const", "continue", "else", "enum", "extern", "false", "fn", "for", "if",
    "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static",
    "struct", "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn",
    "try", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof",
    "unsized", "virtual", "yield", "gen",
];

/// Returns `identifier`, prefixed with `r#` when it is a Rust keyword or
/// reserved word — the only case in which a plain extern-crate path
/// (`<identifier>::task()`) would fail to compile.
pub(super) fn render_identifier(identifier: &str) -> String {
    if RESERVED_WORDS.contains(&identifier) {
        format!("r#{identifier}")
    } else {
        identifier.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::render_identifier;

    #[test]
    fn an_ordinary_identifier_is_rendered_unchanged() {
        assert_eq!(render_identifier("greet"), "greet");
        assert_eq!(render_identifier("new"), "new");
        assert_eq!(render_identifier("create"), "create");
    }

    #[test]
    fn a_keyword_identifier_gets_a_raw_identifier_prefix() {
        assert_eq!(render_identifier("move"), "r#move");
        assert_eq!(render_identifier("type"), "r#type");
        assert_eq!(render_identifier("try"), "r#try");
    }
}
