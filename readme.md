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

`cargo ritual --help` lists `hello` beside ritual's own commands. `new` and
`create` refuse inside a project; every command line carries all of them, so
it looks the same wherever it runs.

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

`cargo ritual import` writes both halves, the dependency and its entry in
`tasks`, and regenerates the command line, so neither is edited by hand.

The dependency key is the command name. Importing a crate under another name
is the key you give `import`, and it writes one line of plain Cargo:

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

`add`, `regenerate`, `new`, `create`, `import` and `remove` come from the
bundle `rituals-core`, imported under the key `ritual`. A new project's
command line is also called `ritual`, so those commands appear directly:
`cargo ritual add`, not `cargo ritual ritual add`. If you name your command line something else,
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

Run this anywhere inside the project. The crate comes from the registry by
default, and `@` picks a version other than the newest:

```sh
cargo ritual import acme-linting@1.2.0 lint
```

The second word is the key the task answers to, so this one runs as
`cargo ritual lint`. Without it, the key is the crate's name.

A crate in a git repository takes `--git`, with at most one of `--branch`,
`--tag` or `--rev`:

```sh
cargo ritual import acme-linting lint --git https://github.com/acme/rituals --tag v1.2.0
```

A crate in a directory takes `--path`:

```sh
cargo ritual import lint --path ../lint
```

`import` adds the dependency to the CLI crate, lists its key in `tasks`, and
regenerates, so the task runs as soon as it finishes. It refuses a crate that
is not a task, and leaves the project as it found it.

To write a task crate that several projects can share, run this outside any
Cargo workspace, since the crate has to build on its own:

```sh
ritual create lint
```

It ends by printing the `import` command to run in a project, with the
crate's path filled in.

### Remove a task

Run this anywhere inside the project, with the task's key in
`[package.metadata.ritual] tasks`, or with the crate it imports:

```sh
cargo ritual remove lint
```

`remove` takes the key out of `tasks`, regenerates the command line, and only
then removes the dependency line, the order the build needs. If the task is a
crate in your workspace, as `add` creates, it also removes its `members`
entry and deletes its directory. A path dependency outside the workspace
loses only its dependency line: its directory stays where it is. Nothing is
committed for you. Look at the change, and commit it when it is what you
meant.

It refuses, and leaves the project as it found it, `Cargo.lock` included, when:

- the name is neither a key nor a crate one of them imports, or the crate is
  imported by more than one key (remove one by its key);
- the task imports `rituals-core`, the bundle of ritual's own commands,
  whatever its key (`ritual` unless you renamed it), since nothing could put
  it back;
- the task has a directory to delete and git cannot give back everything in
  it. That means the project is not a git repository, the directory is or
  holds a git repository of its own, a submodule included, or it holds files
  that are uncommitted, untracked or ignored; the refusal names them. A
  directory that is a symbolic link is deleted as the link alone, so it is
  the link that has to be committed;
- the directory holds files git has been told not to look at, with
  `--assume-unchanged` or `--skip-worktree`, or files stored through a
  `filter` other than Git LFS's, since `git status` can call those clean
  while git could not give back what is on disk;
- the directory is in a git repository other than the project's, such as
  one a symbolic link leads into;
- another crate depends on the task's directory in any way, optionally or
  not, whether it is in the workspace or outside it, or builds from a file
  in it;
- the directory holds other workspace members, lies outside the workspace
  (through a symbolic link included), or is the only entry in
  `default-members`;
- it is the last match of a glob in `members` or `default-members`, which
  Cargo would then read as a path that does not exist (add an explicit
  member, or remove the glob);
- a `[patch]`, `[replace]` or `[workspace.dependencies]` entry, or a
  `paths` or `[patch]` setting in Cargo's configuration, points into the
  directory, since Cargo reads those whether or not anything uses them.
  That is the configuration a build reads from the workspace root, from any
  member's directory, from where you run `remove`, or from any directory
  under the root holding a `.cargo/config` or `.cargo/config.toml` that git
  tracks or would track by that name, and every file it includes; an
  `include` of a file in the directory refuses too, unless it is
  `optional`. [The design](.docs/design.md#what-remove-does) lists what
  this does not reach.

A path points into the directory however it is spelled, as long as Cargo
would reach the directory through it: in another case on a file system that
ignores case, or through a symbolic link.

If anything fails while `remove` is writing manifests or regenerating, the
project is put back as it was. The directory's deletion comes last, after
that, and is not undone; git gives back anything it deleted, and if the
deletion fails partway the message says how, in a command that works from
anywhere in the project.

### Inside a project, use `cargo ritual`

`cargo ritual` works from any directory inside the project. The global
`ritual` carries `add`, `regenerate`, `import` and `remove` too, but they
refuse in your project: use `cargo ritual add`.

## Where next

- [`rituals` on docs.rs](https://docs.rs/rituals) covers writing tasks and
  bundles.
- [`.docs/design.md`](.docs/design.md) explains why ritual is shaped the way
  it is, and [`.docs/crates.md`](.docs/crates.md) shows how its crates divide
  the work.
- [Releases](https://github.com/hexlace/ritual/releases) lists what changed
  in each release.
- [`contributing.md`](contributing.md) covers working on ritual itself.

## Contributing

Contributions are welcome. The process for them is being worked out as of
September 22, 2026, and issues and pull requests are expected to open soon.

## Versions

Ritual follows [Semantic Versioning](https://semver.org/), and every one of
its crates is released at the same version, together.

Ritual needs a recent stable Rust. The minimum supported version is the
`rust-version` each crate declares. Raising it is not a breaking change, and
it happens in a minor release.

## License

MIT. See [license.md](license.md).
