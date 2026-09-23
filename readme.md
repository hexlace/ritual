# Ritual

[![CI](https://github.com/hexlace/ritual/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/hexlace/ritual/actions/workflows/ci.yml)

**Give every project its own CLI, built from tasks that are just crates.**

Every project grows commands. Build this, seed that, check the other thing. A
real CLI is the right home for them, and almost nobody writes one, because
wiring up clap for your own repo is a chore that only happens once the pain
gets bad enough.

Ritual makes it the starting point. `ritual new` gives a project its CLI, and
every task you add shows up in it.

Tasks are crates, so they travel like crates. Group them into a **bundle**
(also a crate) and a whole team shares one toolkit: add one dependency, and
every project gets every task, namespaced, updated through `cargo update` with
no extra thought. Need just one task from someone else? Import that one alone.

There's no plugin system to learn and no registry but the one you already use.
Importing a ritual *is* a Cargo dependency. It's just Cargo.

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

```text
hello has nothing to do yet
```

The first `cargo ritual` builds the project's command line, so Cargo prints
its progress before the output. Later runs reuse the build.

`new` made a Cargo workspace, and `add` put a task in it:

```text
demo/
├── Cargo.toml            the workspace
├── .gitignore
├── .cargo/config.toml    the alias that makes cargo ritual run the command line
├── ritual/               the CLI crate (package demo-ritual)
│   ├── Cargo.toml        says which of its dependencies are tasks
│   └── src/main.rs       generated from that list, never edited by hand
└── tasks/hello/          the task add scaffolded
    ├── Cargo.toml
    └── src/lib.rs
```

Now give `hello` something to do. Replace `tasks/hello/src/lib.rs` with:

```rust
use rituals::{Outcome, Task, clap, report};

/// What this task accepts on the command line.
#[derive(clap::Args)]
struct Arguments {
    /// who to greet
    who: String,
}

fn run(arguments: Arguments) -> Outcome {
    report(format!("hello, {}", arguments.who));
    Ok(())
}

#[must_use]
pub fn task() -> Task {
    Task::new("say hello to somebody", run)
}
```

```sh
cargo ritual hello world
```

```text
hello, world
```

`cargo ritual --help` lists `hello` beside ritual's own `add`, `regenerate`,
`new` and `create`. `new` and `create` refuse inside a project; every
command line carries all four, so it looks the same wherever it runs.

## Concepts

### Terms

- **task**: a crate that marks itself as a task and exposes `task()`.
- **bundle**: a task made of named child tasks.
- **CLI crate**: the crate in your project that imports tasks. `new` puts it
  in `ritual/`.
- **import**: a dependency of the CLI crate that its manifest lists as a task.
- **command line**: what `cargo ritual` runs, built from the CLI crate.
- **generated file**: the CLI crate's `src/main.rs`, written from its
  imports.

### A task is a crate

A crate declares itself a task in its own manifest:

```toml
[package.metadata.ritual]
task = true
```

and exposes one function, `task()`, like the `hello` above. Its arguments
are ordinary clap, re-exported by `rituals`. The first argument to
`Task::new` is the one-line description `--help` shows, not the task's
name: a task never names itself. The
[`rituals` documentation](https://docs.rs/rituals) covers writing one,
including refusing with a `Failure`.

### Importing a task is a dependency

The CLI crate says which of its dependencies are tasks, by dependency key:

```toml
[dependencies]
lint = { path = "../tasks/lint" }

[package.metadata.ritual]
tasks = ["lint"]
```

The dependency key is the command name. Importing a crate under another name
is one line of plain Cargo:

```toml
check = { package = "acme-linting", git = "https://github.com/acme/rituals" }
```

Path, git and registry sources all work, because they are Cargo's. Moving a
task from one project into a shared repository changes the source field and
nothing else.

### The generated file

`cargo ritual regenerate` writes the CLI crate's `src/main.rs` from the
`tasks` list: one line per import. The file is checked in, so a fresh clone
runs `cargo ritual` with nothing installed but Cargo. It is never edited by
hand.

### Bundles

A bundle groups named child tasks under one command, built with
`Task::group` in place of `Task::new`. It is imported exactly like any other
task, and bundles nest.

### Ritual's own commands are a bundle too

`add`, `regenerate`, `new` and `create` come from the bundle `rituals-core`,
imported under the key `ritual`. A new project's command line is also called
`ritual`, so those commands appear directly: `cargo ritual add`, not
`cargo ritual ritual add`. If you name your command line something else,
with `ritual new demo --cli acme`, they stay grouped: your tasks run as
`cargo acme <task>` and ritual's as `cargo acme ritual add`.

The rule behind this, and what happens when two commands would share a name,
is in [the design](.docs/design.md#bundles-and-the-top-level).

## Everyday operations

### Add a task to a project

Run this anywhere inside the project:

```sh
cargo ritual add lint
```

It scaffolds `tasks/lint`, adds it to the workspace and to the CLI crate's
manifest, and regenerates. Edit `tasks/lint/src/lib.rs`, then run
`cargo ritual lint`.

### Import a task from somewhere else

Add the task crate as a dependency of the CLI crate, `ritual/Cargo.toml`,
and append its key to `tasks`:

```toml
[dependencies]
lint = { git = "https://github.com/acme/rituals" }

[package.metadata.ritual]
tasks = ["ritual", "lint"]
```

Then run `cargo ritual regenerate`.

To write a task crate that several projects can share, run this outside any
Cargo workspace, since the crate has to build on its own:

```sh
ritual create lint
```

It ends by printing the dependency line to add to a project, with the
crate's path filled in.

### Remove a task

The steps go in this order, because the command line has to compile for
`regenerate` to run:

1. Drop its key from `[package.metadata.ritual] tasks`.
2. Run `cargo ritual regenerate`, so the generated file stops naming it.
3. Remove its dependency line. If `add` created it, also remove its
   workspace member entry and its `tasks/` directory.

If you removed the dependency first, the build fails in the generated file.
Put the line back and start again from step 1.

### Inside a project, use `cargo ritual`

`cargo ritual` works from any directory inside the project. The global
`ritual` carries `add` and `regenerate` too, but they refuse in your
project: use `cargo ritual add`.

## Where next

- [`rituals` on docs.rs](https://docs.rs/rituals) covers writing tasks and
  bundles.
- [`.docs/design.md`](.docs/design.md) explains why ritual is shaped the way
  it is, and [`.docs/crates.md`](.docs/crates.md) shows how its crates divide
  the work.
- [`changelog.md`](changelog.md) lists what changed in each release.
- [`contributing.md`](contributing.md) covers working on ritual itself.

## Contributing

Contributions are welcome. The process for them is being worked out as of
September 22, 2026, and issues and pull requests are expected to open soon.

## Rust version

Ritual needs a recent stable Rust. The minimum supported version is the
`rust-version` each crate declares. Raising it is not a breaking change, and
it happens in a minor release.

## License

MIT. See [license.md](license.md).
