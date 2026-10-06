# rituals-core-add

Deprecated: `add` is now `create`, and will be removed in ritual 0.3.0. Use
[`rituals-core-create`](https://crates.io/crates/rituals-core-create).

This crate is a shim. `add` says so on standard error, then does what
`create` does inside a project.

That includes one thing 0.1's `add` never did: the task it makes is private.
It writes `publish = false` into the task's manifest unless you give
`--public`. Give `--public` for a task you mean to publish, or remove the key
from its manifest later.

This crate is part of [ritual](https://github.com/hexlace/ritual)'s own management tasks. You do not
depend on it directly: `ritual new` (from
[`rituals-cli`](https://crates.io/crates/rituals-cli)) imports the
`rituals-core` bundle into every project it creates, and this crate comes
with it.

License: MIT.
