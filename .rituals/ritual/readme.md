# rituals-core

The bundle of ritual's management tasks: `add`, `regenerate`, `new`, `create`,
`import`, `remove` and `migrate`, grouped as one task that every composed command
line imports under the key `ritual`. `add` is the deprecated old name of
`create`, and goes in ritual 0.3.0.

You do not depend on it by hand: `ritual new`, from
[`rituals-cli`](https://crates.io/crates/rituals-cli), imports this bundle
into every project it creates, under the key `ritual`. See
[the ritual readme](https://github.com/hexlace/ritual#readme) for how a
project uses it.

License: MIT.
