# rituals-cli

The `ritual` binary. [Ritual](https://github.com/hexlace/ritual) gives every
project its own CLI, built from tasks that are just crates.

## Install

```sh
cargo install --locked rituals-cli
```

The package is `rituals-cli`, plural, and it installs one binary, `ritual`.
The `ritual` and `ritual-cli` crates on crates.io are unrelated projects. You
need the binary only to start a project or a standalone task crate. Inside a
project, everything runs through Cargo.

## Quickstart

```sh
ritual new demo
cd demo
cargo ritual add hello
cargo ritual hello
```

`ritual new demo` writes a Cargo workspace whose one member, `ritual/`, is
the project's CLI crate, with `cargo ritual` aliased to run it. `add hello`
scaffolds a task crate in `tasks/hello`, imports it and regenerates the
command line, so `cargo ritual hello` already runs. Edit
`tasks/hello/src/lib.rs` to make it do something.

## Commands

| Command | Where | What it does |
|---|---|---|
| `ritual new <project>` | outside any Cargo workspace | scaffold a project with a command line of its own |
| `ritual create <name>` | outside any Cargo workspace | scaffold a task crate on its own, for a project to import later |
| `cargo ritual add <name>` | inside a project | scaffold a task crate in this project, import it, and regenerate |
| `cargo ritual regenerate` | inside a project | rewrite src/main.rs from the imported tasks |

`new` and `create` take ritual's crates from crates.io at the version of
the `ritual` you ran. `--path <checkout>` or `--git <url>` takes them
from a ritual checkout or a git repository instead. `new --cli <name>` names
the project's binary and cargo alias `<name>` instead of `ritual`.

A project's own command line carries all four commands, and so does the
global `ritual`. `new` and `create` refuse inside a project, and the global
`ritual`'s `add` and `regenerate` refuse in one: use `cargo ritual add`.

See [the ritual readme](https://github.com/hexlace/ritual#readme) for concepts and everyday operations, and
[`rituals` on docs.rs](https://docs.rs/rituals) for writing a task.

License: MIT.
