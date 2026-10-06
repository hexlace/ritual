# rituals-core-create

The `create` task: scaffold a task crate: into this project, or on its own
where a crate builds on its own. Whose workspace it runs in decides which.

In a project, `cargo ritual create lint` scaffolds `.rituals/lint`, adds it
to the workspace and to the command line crate's manifest, and regenerates. It
also takes a path below `.rituals/`, such as `.rituals/private/lint`, and reads
no meaning into the directories between; the last part is the task's name.

Outside any Cargo workspace, or under an ordinary package with no
`[workspace]`, `ritual create lint` scaffolds a crate of its own in the
current directory, for a project to import later, and prints the `import`
command to run. Inside any other workspace it refuses, and says what to run
instead.

Either way the new ritual is private, with `publish = false` in its manifest,
unless you give `--public`.

This crate is part of [ritual](https://github.com/hexlace/ritual)'s own management tasks. You do not
depend on it directly: `ritual new` (from
[`rituals-cli`](https://crates.io/crates/rituals-cli)) imports the
`rituals-core` bundle into every project it creates, and this crate comes
with it.

License: MIT.
