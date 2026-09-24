# Ritual — crates

How ritual's own code is divided into crates, and why. The rule behind the
division is the same one-way rule a task follows: each crate carries what its
dependents need and nothing more, and nothing depends back up the chain. The
principles themselves are in [design.md](design.md).

| Crate | What it is | Depends on, of ritual's own |
|---|---|---|
| `rituals` | what a task needs | — |
| `rituals-compose` | the composition library | `rituals` |
| `rituals-core-add`, `-regenerate`, `-new`, `-create` | the four management tasks | `rituals`, `rituals-compose` |
| `rituals-core` | the bundle of those four | `rituals`, the four leaves |
| `rituals-cli` | the `ritual` binary | `rituals`, `rituals-core` |
| `xtask` | release tooling, never published | — |

## `rituals` — the floor

What a task needs, and only that: the `Task` type and its three constructors
(`new`, `receiving_command_line`, `group`), `Outcome` and `Failure`, `Name`,
`report`, `Identity` and `identity!()`, `CommandLine`, and `run`, the dispatch
a command line's generated file calls.

A bundle is a task, so building a tree of tasks and dispatching over it is a
task's need, not a command line's, and it lives here. What is left at a
command line's own top level is mapping an outcome to an exit code, and `run`
does that too. That is why the generated file is one call and nothing else.

`rituals` is the floor, not a ceiling. A task imports whatever else it likes.
But nothing about manifests, metadata or code generation lives here, so a
task crate pays for none of it.

**Clap is re-exported as `rituals::clap`.** The framework parses arguments
and answers `--help` for every task, so something has to own the parser. One
re-exported clap means a task crate declares no parser dependency of its own
and cannot end up with a second version of it. Clap's default features are
off, and only the ones the framework uses are named.

## `rituals-compose` — for scaffolders only

The composition library: reading `cargo metadata`, resolving a `tasks` list,
editing manifests in place, rendering a task crate's files and a command
line's generated file, and checking whether a directory can hold a
standalone crate. The four management tasks depend on it because it is the
library their job needs.

An ordinary task never depends on `rituals-compose`, and nothing from
`rituals` is re-exported through it. A crate that needs `rituals` names it
directly, so there is one path to each item.

Beyond `rituals`, its dependencies are here because of what the job is:

- **`serde`** and **`serde_json`**, because `cargo metadata` speaks JSON
  and nothing else, and it is the only thing that can say what a dependency
  resolved to and what that crate declares about itself.
- **`toml_edit`**, because `add` appends to manifests a person wrote. A
  round trip through a plain TOML parser would reformat them and drop their
  comments; `toml_edit` edits in place. It is the crate Cargo's own `cargo
  add` uses.

## `rituals-core-add`, `-regenerate`, `-new`, `-create` — the four leaves

Four ordinary task crates, one per management task. Each depends on
`rituals` like any task, and on `rituals-compose` for the work. Each is
marked `task = true` and exposes `task()`. None depends on another. `add`
finishes by calling the same regenerate path `regenerate` does. That path
lives in `rituals-compose`, so both can reach it without depending on each
other.

## `rituals-core` — a pure bundle

`Task::group` over the four leaves, and nothing else. It adds no capability
of its own. It has the same shape as any bundle anyone writes, which is the
point: a project can write a bundle of its own the same way and mount it
beside this one. The order of its children is the order they appear in
`ritual --help`.

## `rituals-cli` — a bin and nothing else

The command line of this repository: `rituals` plus `rituals-core` mounted
under `ritual`, with the binary `ritual`, so the bundle is flattened. Its
`src/main.rs` is its generated file, rewritten by `cargo ritual regenerate`
and never edited by hand. The global tool is a command line like any other.

## `xtask` — release tooling, never published

`cargo xtask` bumps the workspace version, checks a tag against it, writes a
draft release's body and publishes the crates; the release workflows in
`.github/workflows/` are thin wrappers around it. It is a workspace member so
the gate builds and tests it, and `publish = false` keeps it off crates.io.
Nothing depends on it, and it depends on none of ritual's crates.
