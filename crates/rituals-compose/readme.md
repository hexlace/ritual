# rituals-compose

How a [ritual](https://github.com/hexlace/ritual) composed command line's generated file and
manifests are maintained: reading `cargo metadata`, resolving a project's
`[package.metadata.ritual] tasks` list, editing manifests in place, and
rendering a task crate's files and a command line's generated file.

**You do not need this crate to write a task.** A task depends on
[`rituals`](https://crates.io/crates/rituals) alone. This crate is for
scaffolders, meaning tasks that write or rewrite a project's command line.
Ritual's own `add`, `regenerate`, `new` and `create` are built on it.

License: MIT.
