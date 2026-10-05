# rituals-compose

How a [ritual](https://github.com/hexlace/ritual) composed command line's generated file and
manifests are maintained: reading `cargo metadata`, resolving a project's
`[package.metadata.ritual] tasks` list, editing manifests in place, running
the `cargo` that launched the process, rendering a command for a person to
copy, rendering a task crate's files and a command line's generated file,
saying where a project keeps its tasks, asking git whether it can give back
what a task is about to delete or move, working out where paths go when
directories move, and putting a project back when a run does not finish.

**You do not need this crate to write a task.** A task depends on
[`rituals`](https://crates.io/crates/rituals) alone. This crate is for
scaffolders, meaning tasks that write or rewrite a project's command line.
Ritual's own management tasks, such as `add` and `regenerate`, are built on it.

License: MIT.
