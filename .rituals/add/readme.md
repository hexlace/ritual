# rituals-core-add

Deprecated: `add` is now `create`, and will be removed in ritual 0.3.0. Use
[`rituals-core-create`](https://crates.io/crates/rituals-core-create).

This crate is a shim. `add` says so on standard error, then does what
`create` does inside a project.

This crate is part of [ritual](https://github.com/hexlace/ritual)'s own management tasks. You do not
depend on it directly: `ritual new` (from
[`rituals-cli`](https://crates.io/crates/rituals-cli)) imports the
`rituals-core` bundle into every project it creates, and this crate comes
with it.

License: MIT.
