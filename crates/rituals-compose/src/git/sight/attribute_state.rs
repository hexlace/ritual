//! What an attribute says about a path.

/// What an [`Attribute`](super::Attribute) says about a path.
///
/// # Examples
///
/// ```
/// use rituals_compose::git::AttributeState;
///
/// let describe = |state: &AttributeState| match state {
///     AttributeState::Set => "set".to_string(),
///     AttributeState::Unset => "unset".to_string(),
///     AttributeState::Value(value) => format!("given {value}"),
/// };
/// assert_eq!(describe(&AttributeState::Value("lfs".to_string())), "given lfs");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributeState {
    /// Written as `name`.
    Set,
    /// Written as `-name`.
    Unset,
    /// Written as `name=value`, with the value.
    Value(String),
}
