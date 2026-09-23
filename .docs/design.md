# Ritual — design

The design as it stands: the principles ritual is built on, the shape they
give it, and the reason for each decision. How to use ritual is in the
[readme](../readme.md), which also defines the terms used here. How the work
is divided between crates is in [crates.md](crates.md).

## Purpose

A project's commands deserve a real command line, and almost no project gets
one, because building it by hand costs more than the pain it relieves until
very late. Ritual makes that command line the starting point, and makes the
tasks in it shareable: a task written once reaches every project that imports
it, through Cargo.

## Scope

**Ritual is a composable task runner.** A task is written in Rust the
convenient way, a command line is composed for any project from the tasks it
declares, and tasks written elsewhere are imported like any other dependency.

It solves distribution: a task written once reaches every project that
imports it, through Cargo. Detecting drift between projects, templates, and
provenance beyond what the lockfile records are for tooling built on top of
it. The lockfile is most of provenance for free: it pins exactly which
version of every imported task a project carries.

## Location independence

**The boundary between global and local is a move, not a decision.** A task
is written where it is first needed. When a second project wants it, it is
moved out, and the first project points at the import. Nothing is rewritten.
This is the same shape as a Cargo path dependency becoming a registry
dependency: one field changes.

**That holds only if a task may not know where it lives.** Nothing inside a
task depends on its location; the moment something does, moving it becomes a
refactor rather than a move. This is why a task never names itself (the name
it answers to comes from whoever imports it) and why a task is not told which
command line it is mounted in unless it asks. The same rule shapes
provenance: a project records what it imported and at what version the same
way for a local task and a shared one, so a task that is promoted stays
traceable through the move.

The rule is tested directly: a task crate added by path is moved to another
directory, only its path field and its workspace member entry change, and the
command line still builds and runs it. A path dependency is enough to prove
it. The rule under test is ritual's, and once a moved path dependency works
through Cargo, git and registry sources are Cargo's own business.

## It's just Cargo

**Ritual is the smallest thing that can be added on top of Cargo to make tasks
composable.** Everything Cargo already does, ritual does not redo: no new
manifest, no new source syntax, no new lockfile. That is worth more than the
work it saves. Everyone who would write or import a task already knows Cargo,
so there is nothing new to learn.

## Tasks are crates, imports are dependencies

**A task is a crate.** It exposes one function, `pub fn task() ->
rituals::Task`, that describes a command. It depends on `rituals` and nothing
else from ritual, so writing a task pulls in no code-generation weight. A
crate is the unit because it is the smallest thing Cargo can already import,
version, lock and move.

**Importing a task is a Cargo dependency.** Path, git or registry, declared
the way any dependency is declared and moved the way any dependency is moved.
Nothing else declares a source.

**The dependency key is the command name.** A task crate does not name the
command it answers to. The project that imports it does, by the key it gives
the dependency. Importing a crate under a different name is Cargo's own
`package = "…"` rename, and the same crate can be mounted twice under two
names.

**A task is an unconditional dependency.** The generated file is
unconditional, so a task has to be present on every target the command line
builds for. A task declared only under a `[target.'cfg(…)'.dependencies]`
table is refused, and the refusal names the predicate it was declared under.

**One rule for names.** A command name becomes a directory, a package name
and a Rust identifier, so every name ritual accepts from a person has to be
safe as all four. It starts with a lowercase ASCII letter, continues with
lowercase letters, digits and hyphens, does not end in a hyphen, and is not
`crate`, `self` or `super`. A validated name is its own type, and every
function that joins a name into a path takes that type, so an unchecked
string cannot reach a path join.

## Two-sided metadata

Ritual's own data lives in `[package.metadata.ritual]`, Cargo's sanctioned
place for tool metadata, and it has two sides:

- **Export.** A crate that is a task says so in its own manifest:
  `task = true`.
- **Import.** The CLI crate says which of its dependencies
  are tasks: `tasks = ["lint", …]`, by dependency key, in the order they
  appear.

The two sides check each other. Listing a dependency that never declared
itself a task is refused before anything is written, naming the dependency,
rather than surfacing as a surprise at compile time. Nothing is discovered by
scanning; both ends are explicit.

## Composition by generation

**A command line is composed by generation.** Ritual reads the imports and
writes a plain file listing the tasks: the CLI crate's `src/main.rs`,
which calls `rituals::run` with the command line's identity and one
`(key, crate::task())` pair per import. The file is readable, checked in, and
rewritten only by `regenerate`.

An explicit list is chosen over registration at link time because a
generated list cannot silently drop a task. Inventory-style registration can
be discarded by the linker without any error.

The file has to be reproducible byte for byte, so rustfmt is skipped on it:
one renderer decides its formatting, and where rustfmt would break a line is
not a contract. `regenerate` writes the file only when its content changes,
so running it twice leaves the file untouched rather than merely identical.

A hand edit that puts a duplicate or unusable name into the generated file
does not stop the command line from starting. That mount is dropped with a
line telling the person to run `regenerate`. The command line stays up so
that `regenerate` stays reachable to fix it.

## One kind of CLI

A new project needs a command line to scaffold its command line. That looks
like it needs two tools, a global one and a per-project one, kept in sync. It
does not, because there is one kind of CLI.

- **Inside a project, the CLI crate is a crate in the workspace.** The
  generated file is checked in, so a fresh clone builds it with plain
  `cargo run`. No global binary is needed to work in a project. An alias in
  `.cargo/config.toml` makes it `cargo ritual <task>`, the xtask pattern.
- **The global binary is the command line of the ritual repository itself.**
  Its package is `rituals-cli` and its binary is `ritual`. It is installed
  with `cargo install`, and it is built by the same generation from the same
  bundle every project imports. Its project happens to be ritual.
- **Bootstrap happens once per command line.** This repository's first
  generated file was written by hand; from then on `regenerate` maintains it.
  A project's first generated file is written by `new`, using the same
  renderer `regenerate` uses, so it is exactly what `regenerate` would
  produce there.

Consequence: every feature the global CLI has is a feature any project's CLI
could have. Nothing is special-cased for the global one.

## Management tasks are a bundle

Every command line carries ritual's own management tasks, `add`, `regenerate`,
`new` and `create`, as one bundle: the crate `rituals-core`, imported under
the key `ritual` with an ordinary dependency line and an ordinary `tasks`
entry. Inside a project there is therefore no globally installed tool to keep
in sync with the project.

They are an ordinary imported bundle rather than names built into the
dispatcher. Built-in names would be reserved in every command line, which
breaks the property that every command reaches its task the same way. It
would also mean a tool built on ritual could never have its own `add` mean its
own scaffolder. The shape below meets these requirements:

- no command line ever reads `ritual ritual`;
- a project that takes the defaults keeps `cargo ritual <task>`;
- a project that names its own command line gets `cargo <name> <task>`, and
  ritual's tasks under `cargo <name> ritual …`;
- renaming and removing are the project's business, and recoverable by hand.

## Bundles and the top level

**A bundle is a task whose command is a group of named children**, built with
`Task::group` instead of `Task::new`. It is imported and mounted exactly like
any other task, and nothing in a bundle crate's manifest marks it as different.
Because a child can itself be a bundle, bundles nest to any depth.

**A command line's top level is its own namespace.** Whatever is mounted
under the name the binary was built as *is* that namespace. Its children sit
directly at the top level in the order the bundle lists them, and its key is
not itself a command. Every other task stays under its own key. What is
mounted there has to be a bundle. A plain task under the bin's own name makes
the command line refuse to start, naming the key and the bin name.

| Command line | `ritual` bundle mounted as | What you type |
|---|---|---|
| a default project, bin `ritual` | flat, because it has the bin's name | `cargo ritual add`, `cargo ritual my-task` |
| a project made with `new --cli acme`, bin `acme` | nested | `cargo acme my-task`, `cargo acme ritual add` |
| a tool with bin `mytool` that mounts its own `mytool` bundle as well as ritual's | `mytool` flat, `ritual` nested | `mytool add` is its own; `mytool ritual add` is ritual's |
| the global binary, bin `ritual` | flat | `ritual new`, `ritual create` |

**Flattening is keyed on the bin name, not on a flag.** It happens in
dispatch, at run time. Dispatch compares each mount key with the bin name the
command line was compiled as and flattens the match. The generated file stays
a plain list of mounts with nothing in it that depends on the bin name, so
renaming the `[[bin]]` by hand changes the tree on the next build, with no
regenerate needed.

An explicit `flatten` mark was considered and rejected. Suppose the bin is
renamed by hand from `ritual` to `acme`. The name rule un-flattens ritual's
bundle on the next build by itself. A flag would stay set and leave
`acme add` meaning ritual's `add`, which is exactly the collision this design
exists to remove. Keying on the name makes the rule the invariant, so it
cannot drift from the bin.

Flattening happens once and only at the top level. It never looks inside a
mount for a nested bundle with the bin's name, and a promoted child that
happens to share the bin's name is an ordinary top-level command from then
on.

## Collisions

**One collision is left: two top-level commands with one name.** The case is
a flattened bundle whose child shares a name with another top-level mount, or
a top-level command named `help`. The check runs in dispatch, at startup,
and refuses loudly, naming both. It is load-bearing in both build profiles.
Clap's own duplicate-subcommand check exists only in debug builds: a debug
build panics on two subcommands with one name, and a release build silently
keeps the first. Without ritual's check the same command line would mean
different things depending on how it was compiled.

`help` is not reserved by ritual. It collides because clap gives every
command that has subcommands a `help` of its own, and ritual refuses that
collision the same way as any other.

A flattened collision is refused rather than resolved by dropping one of the
two, unlike a duplicate line in the generated file. The flattened child's
name is compiled into a bundle crate, not written in the generated file, so
`regenerate` cannot fix it.

**Inside a bundle,** `Task::group` asserts at construction that there is at
least one child, that the names are distinct, and that none is `help`. These
are contract violations in the bundle crate's own source, with no user input
involved, so they panic, in both build profiles, at every depth.

**Before writing,** `add` and `regenerate` check the name they are about to
mount against the running command line's own top level, meaning the commands
a flattened bundle supplies, plus `help`. They refuse with both names before
anything is written. `add` also refuses the bin's own name. That slot has to
hold a bundle, `add` only scaffolds plain tasks, and nothing scaffolds a
bundle.

**One case no check before writing can see.** It arises when someone mounts
a bundle under the bin's name by hand and runs `regenerate`. The bundle's
children are not compiled into the running binary yet, so only its key can be
checked, and that key is legitimately the bin's name. If the children
collide with anything, they do so at the next startup, loudly. This is written
down so nobody goes looking for a pre-write check that cannot exist.

## Identity

**A command line's identity is fixed at compile time.** The generated file
passes `rituals::identity!()` to `rituals::run`. The macro expands to the
package name, bin name and version Cargo defines for the binary being built.
Cargo defines the bin name only while compiling a binary target, so the macro
is a compile error anywhere else: the compiler checks that `identity!()` is
only ever used in a binary. The constructor the expansion calls has to be
public for the macro to reach it, so it is hidden from the docs and named for
the claim a call makes, `Identity::from_macro_expansion`. Nothing checks a
hand-written call, and the only command line it can mislabel is the caller's
own.

**Everything ritual prints uses the bin name**: `--version`, `Usage:`, and
the prefix on a refusal line. That is the name a person types, never the
package name. Because it is fixed at build time, a rename or a symlink on
disk does not change it. `--version` and `Usage:` cannot disagree the way
they would if one were derived from how the process was invoked. The package
name is used for one thing only: finding the CLI crate among a
workspace's members.

## Opting in to the command line

A task built with `Task::new` never sees the command line it is mounted in.
That keeps an ordinary task ignorant of where it lives. A task that needs the
command line builds itself with `Task::receiving_command_line` instead. It is
then handed a `CommandLine` at invocation, carrying the identity and the
commands the top level got from a flattened bundle. The change is two edits:
the constructor's name, and one prepended parameter.

`add` and `regenerate` use exactly this. They need it to find their own
project among the workspace's members, and to check a name against the
running top level before writing.

## Each task owns its precondition

The whole bundle mounts everywhere. A project's command line carries `new`,
and the global binary carries `add` and `regenerate`. The alternative was a
conditional import, where a task is present or absent depending on where the
command line stands. That would be the first exception to the tree being the
same shape everywhere. So each task guards itself instead:

- **`new` and `create` refuse inside a Cargo workspace.** That means under a
  declared workspace root, or under a manifest Cargo cannot resolve a
  workspace for. Each writes something that has to build on its own: a
  project does not belong inside another project's workspace, and a
  standalone crate written there would not build standalone. The refusal
  names the workspace root and points at the enclosing project's own `add`.
  The check asks `cargo locate-project` rather than walking up the tree
  looking for a `[workspace]` table, because Cargo treats a directory the
  root's `exclude` covers as its own root once that directory has its own
  `Cargo.toml`, and a walk gets that case wrong. An excluded directory with
  no manifest of its own is still inside the workspace to Cargo, so `create`
  and `new` refuse there.
- **`add` and `regenerate` refuse outside their own project.** They look for
  the package their own command line was built from, by name. In any other
  workspace they refuse and name the command to run in the person's own
  project. So the global binary's `add` and `regenerate` work only inside the
  ritual repository, and a project uses its own `cargo ritual add`.
  Identity by package name is all this checks. A project's built binary run
  by hand inside a different project whose CLI crate has the same package
  name is taken for that project's own, and its collision check then reads
  the wrong top level: it can refuse a free name, or accept one that makes
  the project's command line refuse to start, naming both, until the entry
  is removed by hand. That takes running a binary outside its own alias, it
  fails loudly, and it is accepted for the same reason as the one case no
  check before writing can see, under [Collisions](#collisions).

The framework itself owns no precondition. The cost is a `--help` line for a
task that will refuse where it is run. That is accepted in exchange for a
command line whose shape does not depend on where it stands.

## What `new` writes

`new <project>` writes five files. They are the workspace manifest, a
`.gitignore` holding `/target`, the `.cargo/config.toml` alias, the CLI
crate's manifest and its generated `src/main.rs`. The CLI crate always lives in `ritual/` and is
always the package `<project>-ritual`. Its bin, and the alias, default to
`ritual`.

`--cli <name>` names the bin and the alias, and nothing else. The value is
never a path component, so it cannot collide with anything on disk. It is
still validated as a name, because an alias key and a bin name both have to
be spelled as one. A name that is also a built-in Cargo command, such as
`check` or `build`, builds, but Cargo ignores an alias that shadows its own
command. `cargo run --package <project>-ritual --` still reaches the command
line, and renaming it is the fix.

One constant supplies the bundle's dependency key, its `tasks` entry and its
mount in the generated file, so the three cannot drift apart.

**Where ritual's own crates come from.** With no flags, `new` takes
`rituals` and `rituals-core` from crates.io, and `create` takes `rituals`,
at the version the running `ritual` was built as. What they write then
matches the tool that made it. `--path <checkout>` or `--git <url>` takes the
crates from a ritual checkout or a git repository instead, for work against
an unreleased revision. Naming both is an argument error.

## What `add` writes

`add <name>` scaffolds a task crate in `tasks/<name>`, appends it to the
workspace's `members`, adds a path dependency and a `tasks` entry to the
CLI crate's manifest, and then runs the same path `regenerate` runs. `add`
keeps no task list of its own, so the two cannot drift apart. The new crate
inherits `rituals.workspace = true`, so its dependency on `rituals` does not
change when the crate moves. `add` therefore needs `rituals` in the
workspace's `[workspace.dependencies]`, and refuses without it.

A scaffolded crate's manifest carries no `[lints]` table. An unknown
project's lint table could fail a freshly scaffolded handler before its
author has written a line. Under `clippy::pedantic`, for example, a handler
that always returns `Ok(())` trips `unnecessary_wraps`.

## Removal

Removing a task follows an order Cargo requires. First drop its entry from
`[package.metadata.ritual] tasks`. Then run `regenerate`, so the generated
file stops naming the crate. Only then remove the dependency line (and, for
a crate in the workspace, its member entry and its directory). The generated
file names every mounted crate, and the command line has to compile for
`regenerate` to run, so the crate has to stay a dependency until the file
stops naming it.

Removing the dependency first leaves a generated file that no longer
compiles, and so no `regenerate` to fix it. The generated file's header says
how to recover: put the dependency back, drop the entry, regenerate, then
remove the dependency.

The same order holds for ritual's own bundle, which takes all four management
tasks with it, `regenerate` included. After that the command line cannot
regenerate itself. Putting it back is a hand edit: restore the dependency
line, the `tasks` entry, and the one mount line in the generated file. That
is the way this repository's first generated file was written. Removing the
bundle is the project's call, not something ritual guards against.

## Refusals

Every task checks what it can before it writes anything. A refusal names what
it found and what to do instead. A run that fails partway through writing
puts back what it wrote, and says whether it managed to.

What ritual prints is the documentation people read most, so a next step or
a remedy is written as a command a person can copy. `new`, `add` and
`create` each end with a `next:` line. A remedy that names one of ritual's
own commands spells it for the running command line: `cargo ritual
regenerate` in a default project, `cargo acme ritual regenerate` under
`--cli acme`. The running binary cannot see which key a project mounted
ritual's bundle under, so a project that remounted it under a key of its own
still reads `ritual` in those hints.

One type, `rituals::Failure`, carries every refusal. Its only consumer is a
person reading stderr, so the message is the contract, and a taxonomy
nothing branches on would be surface without a use. Dispatch prefixes every
refusal line with `<bin name>: `, in one place, so no task writes that prefix
itself. Refusals exit with status 1. Argument errors clap raises on its own,
such as an unknown flag or a missing value, keep clap's formatting and exit
with status 2.
