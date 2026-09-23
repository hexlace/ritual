# Changelog

All notable changes to ritual are recorded here. Ritual follows
[Semantic Versioning](https://semver.org/). Raising the minimum supported Rust
version is not a breaking change, and it happens in a minor release.

## 0.1.0

The first release. It includes the runner (tasks are crates, imports are
Cargo dependencies, and a project's command line is generated into one
checked-in file), bundles, and ritual's own `add`, `regenerate`, `new` and
`create` tasks, which ship as the `rituals-core` bundle. It also includes the
`ritual` binary, installed with `cargo install --locked rituals-cli`.
