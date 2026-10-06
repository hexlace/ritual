# Ritual — crates

How ritual's own code is divided into crates, and why. The rule behind the
division is the same one-way rule a task follows: each crate carries what its
dependents need and nothing more, and nothing depends back up the chain. The
principles themselves are in [design.md](design.md).

| Crate | What it is | Depends on, of ritual's own |
|---|---|---|
| `rituals` | what a task needs | — |
| `rituals-compose` | the composition library | `rituals` |
| `rituals-core-add`, `-regenerate`, `-new`, `-create`, `-import`, `-remove`, `-migrate` | the management tasks | `rituals`, `rituals-compose` |
| `rituals-core` | the bundle of those tasks | `rituals`, the leaves |
| `rituals-cli` | the `ritual` binary | `rituals`, `rituals-core` |
| `xtask` | release tooling, never published | — |

The management tasks and their bundle are rituals like any other, so they live
in `.rituals/`: the leaves as `.rituals/add`, `.rituals/regenerate` and so on,
named for the task, and the bundle `rituals-core` as `.rituals/ritual`. The
other crates are in `crates/`, and `xtask` is in `xtask/`.

## `rituals` — the floor

What a task needs, and only that: the `Task` type and its three constructors
(`new`, `receiving_command_line`, `group`), `Outcome` and `Failure`, `Name`,
`report` and `warn`, `Identity` and `identity!()`, `CommandLine`, and `run`, the dispatch
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
line's generated file, checking whether a directory can hold a standalone
crate, running the `cargo` that launched the process, rendering a command for
a person to copy, putting a project back when a run does not finish, saying
where a project keeps its tasks, asking git what it can give back, and working
out where paths go when directories move. The management tasks depend on it
because it is the library their job needs.

**The layout is shared.** `layout` is the one place that says where a
scaffolder puts a ritual, below `.rituals/`. A task that scaffolds a task
crate asks it for the directory, the member entry and the path to report, so
`create` and any scaffolder after it agree and a change of directory is one
edit. There are two ways to ask: `place_for` takes a name and answers
`.rituals/<name>`, and `place_at` takes a `TypedPath`, a path as a person typed
it plus the directory they typed it in, folds it as a shell would, and refuses
one that does not lead strictly below `.rituals/`. A `TaskPlace` carries the
task's name, the last component, so a caller never re-reads it from the path.

**Where a command runs is asked in one place.** `metadata::surroundings` says
whether Cargo finds a manifest at or above a directory, `InsideAProject` or
`OutsideAnyProject`, with `cargo locate-project`, which writes nothing. A task
that does one thing inside a project and another outside it, as `create` does,
asks it, and so does `ensure_inside_a_project`, so there is one way to ask.

**Manifests are edited as one set.** `manifest::Manifests` reads the workspace's
manifest and the command line crate's, from a `ManifestPaths` that names which
is which, and when they are one file, as they are where the command line crate
is the workspace root, it holds one document, so two edits cannot overwrite
each other and the file is written once. `create` and `remove` edit through it.

**Who a ritual is for is written in one place.** `task_crate::manifest` takes an
`Audience`, `Private` or `Public`, and writes `publish = false` for the first,
so every scaffolder spells it the same way and none can leave it out.

**Git is asked in one place.** `git` answers whether git can give back every
file in a directory about to be deleted, whether the work tree is clean, which
directories hold Cargo configuration, which entries are submodules, which
files mention a pattern, which files it does not track, and whether moving
directories would change what it sees of the files in them. `remove` and
`migrate` ask the same code, and each words its own refusal from what it gets
back: `Unanswered` when git could not answer at all, `NotClean` for the
clean-tree question, `CannotGiveBack` for the give-back one, and
`SeenDifferently` for the move one.

**A move is worked out in one place.** A `relocation::Relocation` is built once
from the directories that move and says where any path at or under one of them
goes; `Manifest::repoint` applies it to every path a manifest writes, keeping
the person's own spelling wherever it still reaches the same place.

**The rollback is shared.** A task that writes to a project promises that a
run which fails partway leaves the project as it found it, and says so when
it cannot. `rollback::attempt` keeps that promise for every task: the run
records each change before making it, and on failure every change is undone,
the most recent first. A changed file gets its bytes back whether or not it
is TOML, a file the run created is removed, a directory it created goes, and
anything that could not be put back is named, with the caller's own words
for trying again. The caller also chooses the wording, through a `Wording`:
one for a run that changes a project that was already there ("ritual put the
project back as it found it"), one for a run that makes a directory from
nothing and removes it on failure ("ritual removed `lint` so a retry starts
clean"). When the undo put nothing back, because nothing had been recorded or
every change was already as found, the failure is returned exactly as the run
raised it, so a refusal that came before anything changed never claims a
recovery. The failure's own full stop is dropped only when a clause continues
its sentence, and a run can ask what it found when it first
recorded a file, to say "created" rather than "updated". A directory the run
moved goes back whole, ignored files included, which no version control could
give back. A manifest can only
be written through a run's record, so none is changed without one. One
rollback means one set of rules about what "put back" means, rather than one
per task drifting apart.

An ordinary task never depends on `rituals-compose`, and nothing from
`rituals` is re-exported through it. A crate that needs `rituals` names it
directly, so there is one path to each item.

The `test-util` feature adds `git::fixture`, an isolated `git` for the tests of
a crate that builds fixture repositories; the management tasks turn it on for
their own tests, and no build needs it.

Beyond `rituals`, its dependencies are here because of what the job is:

- **`serde`** and **`serde_json`**, because `cargo metadata` speaks JSON
  and nothing else, and it is the only thing that can say what a dependency
  resolved to and what that crate declares about itself.
- **`toml_edit`**, because `create` and `import` append to manifests a person
  wrote. A round trip through a plain TOML parser would reformat them and drop
  their comments; `toml_edit` edits in place. It is the crate Cargo's own
  `cargo add` uses.

## `rituals-core-add`, `-regenerate`, `-new`, `-create`, `-import`, `-remove`, `-migrate` — the leaves

Ordinary task crates, one per management task. Each depends on
`rituals` like any task, and on `rituals-compose` for the work. Each is
marked `task = true` and exposes `task()`. None depends on another, with one
exception: `add` is `create`'s in-project path under its old name, so it
depends on `rituals-core-create` and not on `rituals-compose`, and there is one
copy of the scaffolding. `create`, `import` and `remove` finish by regenerating
through the same rendering `regenerate` uses, `create` and `remove` inside
their rollbacks. That
rendering lives in `rituals-compose`, so each can reach it without depending on
another.
`migrate` regenerates nothing: it moves directories and repoints manifests,
through the rollback, git and relocation code in `rituals-compose`, and
keeps its steps in its own crate.

## `rituals-core` — a pure bundle

`Task::group` over the leaves, and nothing else. It adds no capability
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
