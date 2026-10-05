//! Where in a manifest Cargo reads a dependency's `path` from.
//!
//! The one list of those places, read by what finds the directories a
//! manifest's path dependencies lead to and by what repoints them, so a place
//! added here is found by one and repointed by the other. Each place says
//! whether Cargo reads it in every manifest or only in the workspace's root,
//! and the places Cargo reads are asked for by saying which manifest is held.

use toml_edit::{Item, TableLike};

/// Which manifest of a workspace is being read, which decides which of its
/// places Cargo reads.
///
/// Cargo reads `[workspace.dependencies]`, `[patch]` and `[replace]` only in
/// the workspace's root manifest. A member's `[patch]` or `[replace]` is
/// ignored with a warning, and those of a crate outside the workspace are
/// ignored without one; a member cannot hold `[workspace.dependencies]` at
/// all, since that makes it a second workspace root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestRole {
    /// The manifest at the workspace's root.
    WorkspaceRoot,
    /// Any other manifest Cargo reads: a member's, or that of a crate a path
    /// dependency reaches.
    Other,
}

/// Which manifests Cargo reads a [`Place`] in.
#[derive(Clone, Copy)]
enum ReadIn {
    EveryManifest,
    WorkspaceRootOnly,
}

/// One step from a table to a table inside it.
enum Segment {
    /// The table under this key.
    Key(&'static str),
    /// The table under every key, as `[target.<t>]` and `[patch.<source>]`
    /// are each one of many.
    EveryKey,
}

/// A table of dependency declarations, reached from the manifest's root by
/// its steps.
pub(super) struct Place {
    segments: &'static [Segment],
    read_in: ReadIn,
    /// Whether a report names each declaration's key in backticks, as it
    /// names the specification of a `[replace]` entry.
    quote_keys: bool,
}

/// A table of dependency declarations found at a [`Place`].
pub(super) struct Located<Table> {
    /// The table as a report names it: `[dependencies]`,
    /// `[target.cfg(unix).dev-dependencies]`, `[patch.crates-io]`.
    pub(super) label: String,
    /// What the [`Place`] says about naming a declaration's key.
    pub(super) quote_keys: bool,
    pub(super) table: Table,
}

const fn key(name: &'static str) -> Segment {
    Segment::Key(name)
}

/// Every place a manifest writes a dependency's `path` in: the dependency
/// tables at the top level, in both spellings Cargo accepts for the dashed
/// ones, and the same tables in every `[target.<t>]` table, which Cargo reads
/// in every manifest; then `[workspace.dependencies]`, every
/// `[patch.<source>]` and `[replace]`, which it reads in the workspace's root
/// manifest alone.
///
/// Read whole by what rewrites a path that names a moved directory, since a
/// path the manifest writes stays true whether Cargo reads it or not. What
/// follows a path Cargo reads asks [`places_cargo_reads`] instead.
pub(super) const PLACES: [Place; 13] = [
    Place::new(&[key("dependencies")]),
    Place::new(&[key("dev-dependencies")]),
    Place::new(&[key("build-dependencies")]),
    Place::new(&[key("dev_dependencies")]),
    Place::new(&[key("build_dependencies")]),
    Place::new(&[key("target"), Segment::EveryKey, key("dependencies")]),
    Place::new(&[key("target"), Segment::EveryKey, key("dev-dependencies")]),
    Place::new(&[key("target"), Segment::EveryKey, key("build-dependencies")]),
    Place::new(&[key("target"), Segment::EveryKey, key("dev_dependencies")]),
    Place::new(&[key("target"), Segment::EveryKey, key("build_dependencies")]),
    Place::root_only(&[key("workspace"), key("dependencies")]),
    Place::root_only(&[key("patch"), Segment::EveryKey]),
    Place {
        quote_keys: true,
        ..Place::root_only(&[key("replace")])
    },
];

/// The places of [`PLACES`] Cargo reads in a manifest that is `role` to its
/// workspace.
pub(super) fn places_cargo_reads(role: ManifestRole) -> impl Iterator<Item = &'static Place> {
    PLACES.iter().filter(move |place| match place.read_in {
        ReadIn::EveryManifest => true,
        ReadIn::WorkspaceRootOnly => role == ManifestRole::WorkspaceRoot,
    })
}

/// The label of a table one step further from the root than `parent`.
fn deeper(parent: &str, step: &str) -> String {
    if parent.is_empty() {
        step.to_string()
    } else {
        format!("{parent}.{step}")
    }
}

impl Place {
    const fn new(segments: &'static [Segment]) -> Self {
        Self {
            segments,
            read_in: ReadIn::EveryManifest,
            quote_keys: false,
        }
    }

    const fn root_only(segments: &'static [Segment]) -> Self {
        Self {
            segments,
            read_in: ReadIn::WorkspaceRootOnly,
            quote_keys: false,
        }
    }

    /// Whether this place is inside the manifest's top-level table `name`,
    /// as `[workspace.dependencies]` is inside `[workspace]`.
    pub(super) fn is_in(&self, name: &str) -> bool {
        matches!(self.segments.first(), Some(Segment::Key(first)) if *first == name)
    }

    /// Every table of declarations at this place under `root`, which is the
    /// manifest's root table. A place the manifest does not write has none.
    pub(super) fn tables<'a>(&self, root: &'a dyn TableLike) -> Vec<Located<&'a dyn TableLike>> {
        let mut reached: Vec<(String, &'a dyn TableLike)> = vec![(String::new(), root)];
        for segment in self.segments {
            let mut next = Vec::new();
            for (label, table) in reached {
                match segment {
                    Segment::Key(name) => {
                        if let Some(inner) = table.get(name).and_then(Item::as_table_like) {
                            next.push((deeper(&label, name), inner));
                        }
                    }
                    Segment::EveryKey => {
                        for (name, item) in table.iter() {
                            if let Some(inner) = item.as_table_like() {
                                next.push((deeper(&label, name), inner));
                            }
                        }
                    }
                }
            }
            reached = next;
        }
        reached
            .into_iter()
            .map(|(label, table)| Located {
                label: format!("[{label}]"),
                quote_keys: self.quote_keys,
                table,
            })
            .collect()
    }

    /// [`Place::tables`], each one open to editing.
    pub(super) fn tables_mut<'a>(
        &self,
        root: &'a mut dyn TableLike,
    ) -> Vec<Located<&'a mut dyn TableLike>> {
        let mut reached: Vec<(String, &'a mut dyn TableLike)> = vec![(String::new(), root)];
        for segment in self.segments {
            let mut next = Vec::new();
            for (label, table) in reached {
                match segment {
                    Segment::Key(name) => {
                        if let Some(inner) = table.get_mut(name).and_then(Item::as_table_like_mut) {
                            next.push((deeper(&label, name), inner));
                        }
                    }
                    Segment::EveryKey => {
                        for (name, item) in table.iter_mut() {
                            if let Some(inner) = item.as_table_like_mut() {
                                next.push((deeper(&label, name.get()), inner));
                            }
                        }
                    }
                }
            }
            reached = next;
        }
        reached
            .into_iter()
            .map(|(label, table)| Located {
                label: format!("[{label}]"),
                quote_keys: self.quote_keys,
                table,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use toml_edit::DocumentMut;

    use super::PLACES;

    const EVERY_PLACE: &str = "[dependencies]\n\
        a = { path = \"a\" }\n\
        [target.'cfg(unix)'.build_dependencies]\n\
        b = { path = \"b\" }\n\
        [workspace.dependencies]\n\
        c = { path = \"c\" }\n\
        [patch.crates-io]\n\
        d = { path = \"d\" }\n\
        [replace]\n\
        \"e:0.1.0\" = { path = \"e\" }\n";

    fn labels(document: &DocumentMut) -> Vec<String> {
        PLACES
            .iter()
            .flat_map(|place| place.tables(document.as_table()))
            .map(|located| located.label)
            .collect()
    }

    #[test]
    fn a_place_is_labelled_the_way_a_report_names_it() -> Result<(), toml_edit::TomlError> {
        let document: DocumentMut = EVERY_PLACE.parse()?;

        assert_eq!(
            labels(&document),
            [
                "[dependencies]",
                "[target.cfg(unix).build_dependencies]",
                "[workspace.dependencies]",
                "[patch.crates-io]",
                "[replace]",
            ]
        );
        Ok(())
    }

    #[test]
    fn the_editable_walk_reaches_the_places_the_reading_walk_does()
    -> Result<(), toml_edit::TomlError> {
        let mut document: DocumentMut = EVERY_PLACE.parse()?;
        let read = labels(&document);

        let mut edited: Vec<String> = Vec::new();
        for place in PLACES {
            for located in place.tables_mut(document.as_table_mut()) {
                edited.push(located.label);
            }
        }

        assert_eq!(edited, read);
        Ok(())
    }
}
