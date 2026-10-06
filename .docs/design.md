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

**Paths are assumed to be UTF-8.** A file or directory name that isn't, or
that holds a newline or a glob character (`*`, `?`, `[`), is not supported.
Ritual skips such a name, or passes it through as Cargo reads it, and never
handles it specially.

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

Declaring itself is not quite enough. A task also has to be built on the very
`rituals` package the CLI crate uses, and its key has to be one the generated
file compiles with, which `std` and `core` are not. Either failure would only
show at compile time, and a command line that does not compile cannot run
the `remove` that would put it right, so both are refused before anything is
written too.

`import` runs this check right after `cargo add`, when Cargo has declared the
dependency and the crate's own manifest is known, whichever source it came
from. It asks through the same rule the resolver applies when `regenerate`
reads the list, so whatever `import` accepts, `regenerate` accepts. A refusal
puts both files `cargo add` could have changed back as they were.

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

**The generated file sets no global allocator.** A command line runs one task
and exits, so allocator throughput is not what bounds it, and an allocator
such as mimalloc would add a C build dependency to every project. Since the
file is generated, a project cannot add one by hand either. This is the
answer for a project whose conventions ask application binaries to set one:
the command line deliberately doesn't.

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

Every command line carries ritual's own management tasks, `add` (the
deprecated old name of `create`), `regenerate`, `new`, `create`, `import`,
`remove` and `migrate`, as one bundle: the crate `rituals-core`, imported under the key `ritual` with an
ordinary dependency line and an ordinary `tasks` entry. Inside a project there
is therefore no globally installed tool to keep in sync with the project.

They are an ordinary imported bundle rather than names built into the
dispatcher. Built-in names would be reserved in every command line, which
breaks the property that every command reaches its task the same way. It
would also mean a tool built on ritual could never have its own `create` mean its
own scaffolder. The shape below meets these requirements:

- no command line ever reads `ritual ritual`;
- a project that takes the defaults keeps `cargo ritual <task>`;
- a project that names its own command line gets `cargo <name> <task>`, and
  ritual's tasks under `cargo <name> ritual …`;
- renaming is the project's business, and recoverable by hand; `remove`
  refuses to take the bundle out, because without its commands nothing could
  put it back.

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
| a default project, bin `ritual` | flat, because it has the bin's name | `cargo ritual create`, `cargo ritual my-task` |
| a project made with `new --cli acme`, bin `acme` | nested | `cargo acme my-task`, `cargo acme ritual create` |
| a tool with bin `mytool` that mounts its own `mytool` bundle as well as ritual's | `mytool` flat, `ritual` nested | `mytool create` is its own; `mytool ritual create` is ritual's |
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
`acme create` meaning ritual's `create`, which is exactly the collision this design
exists to remove. Keying on the name makes the rule the invariant, so it
cannot drift from the bin.

Flattening happens once and only at the top level. It never looks inside a
mount for a nested bundle with the bin's name, and a promoted child that
happens to share the bin's name is an ordinary top-level command from then
on.

## Collisions

**A dependency key is taken by the name Rust sees.** `create` and `import`
refuse a key when a dependency of the CLI crate is already declared under it,
with `-` read as `_`. Cargo does not: it accepts `a-b` beside an existing
`a_b` and `cargo metadata` reports both, and only the build fails, because
rustc finds two crates for the one extern name `a_b` (E0464). The refusal is
ritual's to make, before anything is written.

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

**Before writing,** `create`, `import` and `regenerate` check the name they are
about to mount against the running command line's own top level, meaning the
commands a flattened bundle supplies, plus `help`. They refuse with both names
before anything is written. `create` and `import` also refuse the bin's own name.
That slot has to hold a bundle. `create` only scaffolds plain tasks, nothing
scaffolds a bundle, and nothing in a crate's manifest tells `import` a bundle
from a plain task.

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

`create`, `import`, `regenerate`, `remove` and `migrate` use exactly this, and
so does `add`, which is `create`'s in-project path under its old name. They
need it to find their own project among the workspace's members, and `create`,
`import` and `regenerate` also need it to check a name against the running
top level before writing; `remove` and `migrate` need it to spell the command
a refusal tells a person to run again.

## Each task owns its precondition

The whole bundle mounts everywhere. A project's command line carries `new`,
and the global binary carries `create`, `import`, `regenerate`, `remove` and
`migrate`.
The alternative was a conditional import, where a task is present or absent
depending on where the command line stands. That would be the first exception
to the tree being the same shape everywhere. So each task guards itself
instead:

- **`new` refuses inside a Cargo workspace.** That means under a declared
  workspace root, or under a manifest Cargo cannot resolve a workspace for.
  It writes a project that has to build on its own, and a project does not
  belong inside another project's workspace. The refusal names the workspace
  root and points at the enclosing project's own `create`.
  The check asks `cargo locate-project` rather than walking up the tree
  looking for a `[workspace]` table, because Cargo treats a directory the
  root's `exclude` covers as its own root once that directory has its own
  `Cargo.toml`, and a walk gets that case wrong. An excluded directory with
  no manifest of its own is still inside the workspace to Cargo, so `new`
  refuses there.
- **`create` chooses by where it runs.** It asks `cargo locate-project`
  whether Cargo finds a manifest at or above the current directory, which
  writes nothing. Inside a project it puts a task in `.rituals/` and
  regenerates; outside any project it writes a crate of its own in the
  current directory. Nothing else decides it: a plain package with no
  `[workspace]` is a project to Cargo, so `create` run there is in a project.
- **`create` inside a project, `import`, `regenerate`, `remove` and `migrate`
  refuse outside their own project.** So does `add`. They look for the
  package their own command line was built from, by name. In any other
  workspace they refuse and name the command to run in the person's own
  project. So the global binary's `create` inside a project, `import`,
  `regenerate`, `remove` and `migrate` work only inside the ritual repository,
  and a project uses its own `cargo ritual create`.
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
`rituals` and `rituals-core` from crates.io, and `create`, outside a project,
takes `rituals`, at the version the running `ritual` was built as. What they write then
matches the tool that made it. `--path <checkout>` or `--git <url>` takes the
crates from a ritual checkout or a git repository instead, for work against
an unreleased revision. Naming both is an argument error. A `--path` checkout
is checked before anything is written: it has to hold every directory the
project will name, `crates/rituals` and ritual's own tasks in
`.rituals/ritual`, so a checkout whose tasks are elsewhere is refused by `new`
rather than by the project's first build.

## What `create` writes

`create` does one of two things, chosen by where it runs (see
[Each task owns its precondition](#each-task-owns-its-precondition)).

**Inside a project,** `create <name>` scaffolds a task crate in
`.rituals/<name>`, appends it to the workspace's `members`, adds a path
dependency and a `tasks` entry to the CLI crate's manifest, and then runs the
same path `regenerate` runs. `create` keeps no task list of its own, so the
two cannot drift apart. The new crate inherits `rituals.workspace = true`, so
its dependency on `rituals` does not change when the crate moves. `create`
therefore needs `rituals` in the workspace's `[workspace.dependencies]`, and
refuses without it, and refuses `--path` and `--git`, which choose where a
crate made outside a project gets `rituals` from. It runs as one rollback from
its first `cargo metadata`: the lockfile is recorded before it is read,
regenerating happens inside the run so a regenerate that refuses leaves
nothing half-written, and a refused or failed run leaves the project as it
found it. When the CLI crate is the workspace root, the workspace's manifest
and the CLI crate's are one file, so they are read as one document, edited
together and written once.

**Outside any project,** `create <name>` writes a crate of its own in the
current directory, which depends on `rituals` from wherever `--path`, `--git`
or crates.io says, and ends with the `import` command to run in a project.
It takes a bare name there. A path is refused: with no `.rituals/` to place it
in, the crate goes in the current directory.

**A path inside a project.** A bare name goes in `.rituals/<name>` from
anywhere in the project. A name containing `/` is a path, read from the
current directory the way a shell reads it, with `.` and `..` folded away
lexically. It has to lead strictly below `.rituals/`, so
`create .rituals/private/lint` from the root and `create private/lint` from
inside `.rituals/` mean the same place, and `create src/lint` is refused. Its
last component is the task's name, its key and its crate's name, and is
validated as a name like any other. The member entry and the report lines
spell it from the workspace root.

**Audience.** `create` writes `publish = false` into the new crate's
`[package]` unless given `--public`, inside a project and outside one. See
*Audience is `publish`, not a folder* below for why that is the key.

**Why `.rituals/`.** Every ritual lives in `.rituals/`, whoever it is for,
with no exception: not a project's private ones only, and not ritual's own
repository, where the published `rituals-core-*` crates sit beside any private
ones. A rule with an exception has to be asked of every ritual, and a
repository whose product is rituals would have to answer it for each one. With
none, a person, a tool or a script finds every ritual of every project in the
same place, and a ritual that changes who it is for does not move.

*Audience is `publish`, not a folder.* Whether a ritual is private to its
project or shared with others is already a fact about a crate, and Cargo
already has a key for it: `publish`, which says whether a crate ships. A
second encoding in the directory tree, such as a `private/` folder every tool
must know to treat differently, would be a second source of the same fact,
free to disagree with the manifest. Reading `publish` keeps one source, and it
is Cargo's own, so every tool that already understands a package understands
it. `create` makes a ritual private unless given `--public`, so forgetting to
say who it is for cannot publish anything.

*Subdirectories carry no meaning.* Beneath `.rituals/`, a project arranges
rituals as it likes, at any depth, and ritual gives no name there a special
reading. This is not extra work: no command finds a ritual by its directory.
Each goes through Cargo's workspace membership and the CLI crate's
dependencies, so a ritual at `.rituals/private/lint` is found the same way as
one at `.rituals/lint`. A name ritual did read would be a vocabulary every
project had to learn and could not extend. The one place Cargo's rules touch
the arrangement is membership: a `.rituals/*` glob also matches a grouping
directory that has no manifest, and Cargo then refuses the workspace. That is
Cargo's rule about globs, so ritual documents it and does not paper over it,
and the person lists the directory under `exclude` or lists members
explicitly, as `create` does. `migrate` carries such an `exclude` entry from
`tasks/` with the directory it names. `remove` deletes only the ritual's own directory
and leaves a grouping directory it does not own.

*`migrate` never writes `publish`.* A 0.1 task has no `publish` key, because
0.1 wrote none, and the absence says nothing about who the task is for: the
same missing key is a private tool in one project and a shared crate in
another. Writing `publish = false` would claim the first for every project,
a guess about something only the person knows. Leaving the key alone claims
nothing new: Cargo reads the task exactly as it did before the move, and
`migrate` changes where a ritual lives, never who it is for.

Where a ritual goes is decided in one place, `rituals_compose::layout`, which
a scaffolder asks and does not answer for itself, so the member entry, the
dependency path and the report all agree.

**Why the member is explicit.** The entry written is `".rituals/<name>"`, one
per task, and `new` writes no `.rituals/*` glob. Cargo reads a glob that
matches nothing as a literal path and stops loading the workspace, so a glob
written before the first task exists would break every new project, and
`remove` would then refuse to take a project's last task out from under it.
An explicit entry leaves `remove` nothing to special-case. A person's own
glob is still theirs, and `remove` still refuses to delete it.

A project laid out by 0.1, its tasks in `tasks/`, keeps working. A task is a
workspace member and a path dependency, so nothing but the scaffolder ever needed it
to be in a particular directory: a task in `tasks/` builds, runs,
regenerates and is removed exactly as one in `.rituals/` is.

A scaffolded crate's manifest carries no `[lints]` table. An unknown
project's lint table could fail a freshly scaffolded handler before its
author has written a line. Under `clippy::pedantic`, for example, a handler
that always returns `Ok(())` trips `unnecessary_wraps`.

## What `import` writes

`import <crate>[@<version>] [<key>]` makes a task crate that someone else
wrote a command of the project. The crate comes from the registry, or from
`--git` (with at most one of `--branch`, `--tag` or `--rev`), or from
`--path`. The key defaults to the crate's name. It writes the two halves of an
import that [two-sided metadata](#two-sided-metadata) describes, the
dependency and its entry in `tasks`, and then runs the path `regenerate`
runs, so `import` keeps no task list of its own either.

The steps, in order:

1. **Resolve the key, and find the project.** The key is read only from
   what was typed, so a key that is not a usable name is refused with
   nothing to put back. So is a key the generated file could not compile
   with: `std` or `core` would stand in for Rust's own crates, which the
   file reaches by those names, and a command line that does not compile
   cannot run the `remove` that would take the key back out. The rule is a
   type, `TaskKey`: the manifest writers take only a key that has been
   through it, so a writer that skips it does not compile. Then
   `cargo locate-project`, which writes nothing, says whether there is a
   project here at all, and where there is none the refusal hands back the
   command to run inside one.
2. **Snapshot, then check.** From the first `cargo metadata` on, the run is
   one rollback. That call creates the workspace's `Cargo.lock` when there is
   none and rewrites a stale one, so the lockfile is recorded before it runs,
   and a refusal anywhere after puts it back. Then the project is checked
   to be the one this command line belongs to, the key against the bin's name
   and the top level, then against the dependencies the CLI crate declares,
   and the task list it has already must resolve, because the run ends by
   regenerating.
3. **`cargo add`.** It runs in the directory the person typed `import` in,
   with `--package` naming the CLI crate, so a relative `--path` means what
   they typed. `--rename <key>` is passed only when the key differs from the
   crate's name, because with an equal one Cargo writes a redundant `package`
   field. The version in `<crate>@<version>` goes to Cargo untouched: its
   grammar is Cargo's. Cargo's output is captured, so a refusal is one line
   of ritual's own.
4. **The cross-check.** A fresh `cargo metadata` says what the crate declares
   about itself, and the dependency under the key must be a task built on the
   very `rituals` package the CLI crate uses. Two packages called `rituals`
   are two crates to Rust, whatever their versions, and the generated file
   hands one's `Task` to the other's `run`. A task can only come from a
   `rituals` in the crate's normal-dependency closure: its own, or the one
   under a crate whose task it re-exports. So the check walks that closure
   and refuses if any `rituals` in it is not the CLI's, naming the crate
   that depends on the other one, and refuses a closure with no `rituals`
   at all, which has no task to hand over. A facade with no `rituals` of
   its own, over a task on the CLI's, passes. A tree that holds a second
   `rituals` it never hands over is refused too; nothing has been written
   by then, so that errs on the side of a command line that still builds.
   A crate that is not a task is refused.
5. **The append.** The key joins `[package.metadata.ritual] tasks`.
6. **Regenerate**, once the run has committed. If it fails, the failure says
   the task is imported and names the `regenerate` command that finishes it.

A refusal at any of steps two to five puts the CLI crate's manifest and the
workspace's `Cargo.lock` back, and removes a lockfile the run created. The undo
restores the recorded bytes rather than running `cargo remove`: that puts back
exactly what was there, in the lockfile as well as the manifest. It writes each
file in place, so it fails where a file cannot be written in place, such as a
read-only manifest, which `cargo add` replaces by rename. The refusal then
says which files ritual could not put back, for the person to check before
running `import` again.

## What `remove` does

Removing a task by hand follows an order Cargo requires. First drop its entry
from `[package.metadata.ritual] tasks`. Then run `regenerate`, so the
generated file stops naming the crate. Only then remove the dependency line
(and, for a crate in the workspace, its member entry and its directory). The
generated file names every mounted crate, and the command line has to compile
for `regenerate` to run, so the crate has to stay a dependency until the file
stops naming it. Removing the dependency first leaves a generated file that no
longer compiles, and so no `regenerate` to fix it.

`remove <key | crate>` does those steps in that order. The argument is a key
in `tasks`, or a crate that exactly one key imports; a key wins over a crate
of the same name. It drops the key, regenerates, removes the dependency line,
and removes the `[workspace.dependencies]` entry the dependency inherited
when no other package uses it. For a crate that is a member
of the workspace it also removes the `members` entry, and the same directory
from `default-members`. An entry names the directory when Cargo would read
it as that directory, so `.rituals/./lint` and the absolute path are taken out
too. A glob in `members` is left alone, because deleting the directory is
enough, unless it would match nothing once the directory goes, which
refuses. Which crate is a workspace member comes from `cargo metadata`, and
its directory from the dependency's `path`, never from a fixed directory
name. A path dependency outside the workspace loses only its
dependency line, and its directory stays.

Everything up to the dependency line happens inside one rollback, so a run
that fails leaves the project as it found it, `Cargo.lock` included. A final
`cargo metadata` inside it checks that the edited manifests still resolve and
brings the lock up to date. The directory is deleted after that, outside the
rollback and as the last step, because git is the way back for a deletion,
not a rename aside. A deletion that fails partway says that the manifests are
already updated and that git can give back what was deleted, with a `git
checkout` that names the directory from git's top level (`:/.rituals/lint`), so
it works from any directory of the project.

**Why it refuses first.** Before anything is written `remove` refuses:

- a name that is neither a key nor a crate one imports, and a crate that
  several keys import (it names them, and says to remove one by its key);
- a key whose package is `rituals-core`, whatever the key is called. The
  bundle holds the commands that put a task back, so without it nothing
  could; that is the one removal the project cannot undo from its own
  command line;
- a list whose other entries would not resolve without this one, with
  `regenerate`'s own message;
- for a member whose directory would be deleted, anything Cargo would still
  read once it is gone: a dependency on it declared by any package, of any
  kind, on any target, optional or not, in the workspace or outside it, and
  a target built from a file in it; a glob in `members` or
  `default-members` whose last match it is; and a `[patch]`, `[replace]` or
  `[workspace.dependencies]` entry, or a `paths` or `[patch]` setting in
  Cargo's configuration, pointing into it, and an `include` of a file in
  it that is not `optional`. Also a directory that holds other members,
  lies outside the workspace root once symbolic links are followed, or
  whose entry is the last in `default-members`.

These are decided while the directory still exists, so each asks about the
project without it rather than about the project as it is: a final `cargo
metadata` runs before the deletion and cannot see what the deletion
breaks. Dependents come from what each package declares, not from the
resolved graph, which holds only the edges the active features reach; and
a path crate `cargo metadata` did not load at all, such as one behind an
optional dependency nothing turns on, is asked about by reading its manifest,
because Cargo still reads it when it resolves the lockfile, and a `[patch]`
or `[replace]` in such a crate, which Cargo ignores, does not make it a
dependent. That is the walk `migrate` makes to find every manifest to
repoint, not `cargo metadata --no-deps`, which refuses a crate that sits
under the workspace's root without being a member.

Cargo's configuration is read the way Cargo reads it: the `.cargo/config`
file, or `.cargo/config.toml` when there is none, in every directory a
build can start from and in every directory above it, then the one in
`$CARGO_HOME`, and every file each of those includes, at any depth. A build
can start from any directory, so the directories read from are the one
`remove` runs in, the workspace root, each member's, and every directory
under the root holding a `.cargo/config` or `.cargo/config.toml` that git
tracks or would track, which `git ls-files --cached --others
--exclude-standard` lists. That listing is matched by name, so a few
configuration files a build could still read are outside it, and are listed
under what `remove` cannot see.

A path points into the directory when the filesystem says it does, not when
its text matches. Cargo opens paths through the filesystem, so `.Rituals/Lint`
on a file system that folds case, `alias/lint` when `alias` links to
`.rituals`, and an absolute path through a link above the project all name
`.rituals/lint` to it. So each path is followed a component at a time, through symbolic
links, and compared with the directory by file identity. When the directory
is itself a link, a path through the link points into it and a path to its
target does not, because deleting the link leaves the target. Cargo removes
`.` and `..` as text before it opens a dependency's, a `[patch]`'s, a `paths`
override's, a member's or a target's path, and opens an `include` as written,
so a `..` after a link goes up from the link's own directory in the first and
from where the link leads in the second; each is followed the way Cargo
follows it. Past a component that does not exist the filesystem has no
answer, and the rest is compared as text.

**Git decides whether a directory can be deleted.** Before deleting, `remove`
asks git for every file under the directory: staged and unstaged changes,
untracked files and ignored files all count. If any exist it refuses and
names them, so every file `remove` deletes is one git can give back. Build
output counts too: letting it through would make that sentence false. So do
the files `git status` calls clean without being able to give back: a file
marked `--assume-unchanged`, or `--skip-worktree` while it is on disk, whose
edits git does not look at, and a file stored through a `filter` other than
Git LFS's, which can store less than is on disk. A project that is not a git
repository, a machine with no `git`, a task directory that is a repository
of its own, and one in a repository that is not the project's, such as
through a symbolic link, are refused the same way. The
refusal says to take the task out by hand, because deleting could not be made
safe, and nothing is written. A task directory that is itself a symbolic
link is deleted as the link alone, leaving what it points at, so git is
asked about the link: it has to be committed and unchanged.

**What it cannot see.** Some things read a task's directory where `remove`
cannot look, and it does not refuse over them:

- a `readme` or `license-file` pointing into it, because only `cargo
  package` reads them, and the project still builds;
- a `[source]` replacement `directory` in Cargo's configuration, because
  Cargo reads it only when a build fetches a package through that source,
  which depends on what was vendored there;
- `--config` and `CARGO_*` overrides, because they are given to a later
  build, and `remove` cannot see what that build will be given;
- a `.cargo/config` or `.cargo/config.toml` that git ignores, or that sits
  in a repository of its own inside the project, because git does not list
  it with the project's files, and a file the project does not keep is not
  one `remove` can tell from a stray one;
- a configuration file git lists under another name: in a `.cargo` that is
  itself a symbolic link, or in a `.Cargo` on a file system that ignores
  case, because the listing matches the name git stores;
- the configuration in or below the directory of a member that lies outside
  the workspace root, because git lists only what is under the root;
- a `build.rs`, `include_str!` or `#[path]` reaching into it, because what
  code reads at build time is not knowable from the manifests;
- line endings under `core.autocrlf`, because git gives back a file's
  content, not the CRLF bytes it had on disk.

The generated file's header says how to recover a command line that no longer
compiles: put the dependency back, drop the entry, regenerate, then remove
the dependency. `remove` cannot run there, because it runs as that command
line, so the header stays as it is.

## What `migrate` does

Some releases change what a project should look like, and the change is
mechanical, so a person should not carry it out by hand from release notes.
`migrate` brings a project up to the layout of the ritual it runs.

**A step knows whether it applies.** A migration is a step: a value that looks
at the project and says whether it is already as the step would leave it.
`migrate` asks every step in release order and runs each one that applies, each
reading the project the step before it left. It stores nothing between runs.
There is no recorded "migrated to" version to go stale or be edited out of
step with the project, and a second run finds every step already satisfied and
says `nothing to migrate`. The steps are a closed set that lives in one crate,
so they are an enum matched on, not a trait. A later release's change to the
layout is a new step that goes last. A step owns both ends of its move as
literals: the first moves `tasks/` to `.rituals/` for good, and does not read
the destination from `layout`, which says where scaffolding puts a task today.

**The first step: `tasks/` to `.rituals/`.** It applies when a workspace member
under `tasks/` declares itself a task, by the one rule the task list is
resolved with. "Member" is what `cargo metadata` says, since Cargo's
membership is glob expansion, `exclude` and path dependencies that become
members on their own, and a second reading of that could only disagree with
Cargo. Each such task moves to the same place under `.rituals/`, so
`tasks/greet` becomes `.rituals/greet`, whether or not the project's command
line imports it: every ritual lives in `.rituals/` whoever it is for, so a
repository whose product is task crates moves them too. A crate in `tasks/`
that is not a task stays where it is, with the directory that holds it.

**It only runs where git can give everything back.** A work tree with changes
that are not committed, files that are untracked, or no repository at all is
refused, before anything is written. Ignored files do not count against the
clean tree, because a rename carries them with their directory; whether git
would still ignore them once they have moved is a separate question, below.
The question is asked before the first `cargo metadata`, which can rewrite
`Cargo.lock` and would make a clean tree dirty, and acted on only once a step
applies. A project with nothing to migrate is therefore told so whatever git
says, including right after a migration that has not been committed yet. The
check reuses `remove`'s code, in `rituals_compose::git`, and asks only the
question `migrate` needs: unlike `remove`, it deletes nothing, so what it must
have is that git sees the files the same way after the rename, not that git
can give back every byte, and `remove`'s checks for filters and for ignored
files do not apply.

**It refuses a move that changes what git sees.** Git decides some things by a
file's path alone, and a rename changes the path: whether it is ignored, which
attributes it has, and whether the sparse checkout includes it. A rule keyed on
`tasks/` stops applying after the move and one that matches under `.rituals/`
starts. The tree is clean now, and the commit after the move would then leave
out a file that is committed (a `.*` rule ignores the new directory, and a
tracked file whose new path is ignored is deleted and not added, whatever rule
matched the old path), add a file that is ignored now (a secret named in a rule
for the old place), or store a file through another filter (a `.gitattributes`
rule for Git LFS keyed on `tasks/`), and `git status` shows each as an
ordinary change. So, while planning and before any write, `migrate` lists
every file under every task, tracked, untracked and ignored, and compares
what git says at the old path with what it would say at the new one:

1. Ignore status, by `git check-ignore -v -n --no-index`, which reports the
   rule that decided. A tracked file counts as seen at its old path whatever
   the rules say, because tracking is not undone by a rule; an untracked file
   is seen when no rule ignores it.
2. Attributes, by `git check-attr -a`, compared as whole sets so that a
   filter, an end-of-line setting and a macro such as `binary` are all
   covered. Only a file git sees at both places is compared; one it ignores
   is not stored, so its attributes decide nothing.
3. The sparse checkout, by `git sparse-checkout check-rules`, asked only when
   `core.sparseCheckout` is on, since otherwise it fails or prints nothing.
4. The index flags assume-unchanged and skip-worktree, which are not decided
   by path: a commit after the move can carry an edit `git status` never
   showed. A skip-worktree file that is not on disk is not moved by a rename,
   so it is left out.

The new places do not exist yet, and the files that decide them move too: a
`.gitignore` or `.gitattributes` inside a task goes with it, so asked in the
real tree before the move, git cannot see it at the new place and the check
would refuse a move that is safe. So the new places are asked about in a work
tree of copies in the system's temporary directory, named for the process and
removed when the check returns, that holds the project's ignore and attribute
files where they will be: every one in the directories above each new place,
and every one under a task at its new place. Git is given the repository's own
git directory beside it, so the repository's configuration and the index are
read exactly as for the real tree. Only regular files are copied, because git
does not follow a link for these files. Nothing is written to the project or
the repository. The check is for the rules the project carries. Configuration
outside the repository, such as a global excludes file or `.git/info/exclude`,
is the person's own and out of scope, because a project has to work from a
fresh clone: git still reads it when asked, and the check neither sets it aside
nor promises anything about it. The alternatives were asking the real tree
before the move, which gets that wrong, and reproducing git's precedence by
hand, which is a second reading of git's rules that could only disagree with
the first.

The check is only worth trusting if the copies answer as the real tree will, so
a test asks for it: over every repository the unit tests judge, it takes the
prediction, really renames the directory, asks git itself about the new paths,
and requires the two to agree, including which files a `git add --all` after
the rename stages.

The first kind found is reported, with every file it holds for, since each kind
has its own remedy. The refusal opens `refusing to migrate` rather than naming
a task, because one rule can catch every task, names the first ten files and
counts the rest, and names the rule and the file it is in, at the line, so the
person can open it. A rule that sits in a file that moves with a task is named
where the file is now.

**The order of writes, and the failure story.** `migrate` makes the promise
`remove` makes: it leaves a project Cargo reads as it should in the new layout,
or leaves it as it found it, or says exactly how to put it back.

1. Refusals first, from reading only: a task directory that holds other
   workspace members, a destination that is taken (`.rituals` being a file
   counts), a git submodule at or inside a task directory, a file that git
   would see differently at its new place, and a manifest that reaches a task
   in a way that cannot be repointed, or that needs an edit while git does not
   track it.
2. Every manifest with edits is written in place.
3. Each task directory is renamed.
4. `cargo metadata` runs again, and the project must still resolve its task
   list and read the same workspace members, each where it now is.

All of it is one rollback. The renames are recorded in it and undone by
renaming back, directories first and then each manifest's bytes at its
original path, so a task's own manifest, edited where it stood and then
carried by its rename, comes back right. A move is recorded rather than
recovered through git because the final check can only run after the moves,
so with git as the way back every failure after a move would need a recovery
command, and git cannot give back the ignored files a directory holds, such
as a stray `target/`. A rename carries them both ways, so they end up where
they started on every path. If a rename cannot be undone, the message names
both paths and the `mv` that puts the directory back, with absolute paths so
it works from any directory. It names no command when both places hold
something again, because that `mv` would move one inside the other.

**What is repointed.** A task can depend on another, and any member can depend
on a task, so the command line crate's manifest is not the only one that names
a moved directory, and a member's is not the last: a crate outside the
workspace can too. Every manifest Cargo reads, the
workspace's own, every package at a path on disk and every crate those reach
through a path dependency of any kind, however far and wherever it sits, has
every place Cargo reads a path repointed. Cargo reads more than `cargo
metadata` lists: a crate excluded from the workspace and reached only through
an optional dependency no feature turns on is read when it resolves the
lockfile, and so is a crate that crate reaches. `cargo metadata --no-deps`
cannot be asked about every one of them, since it refuses a crate that sits
under the workspace's root without being a member, so the walk reads the
TOML itself, following each manifest's path dependencies with a set of the
manifests already read, which bounds it to the files on disk. It follows
`[workspace.dependencies]`, `[patch]` and `[replace]` from the workspace's
root manifest alone, because Cargo ignores them in any other, so a crate
only a member's `[patch]` reaches is not read, repointed or refused. A path
dependency whose directory has no `Cargo.toml` is skipped, because Cargo did
not need it to read the project, and a manifest that exists but does not parse
is refused, naming it. A manifest that needs an edit and is not tracked by git
is refused, because git could not give it back. The places repointed are:
dependency paths in every table and target, `[workspace.dependencies]`,
`[patch]` and `[replace]`, a package's `build`,
`readme`, `license-file` and `workspace`, every target's `path`, and the lists
in `[workspace]`. Moving `tasks/` to `.rituals/` keeps every depth, so only a
path that crosses into or out of a moved directory changes. One that still
leads where it led is kept as the person wrote it, which is why `shout`'s
`path = "../greet"` is not touched when both move together. A path that reaches
a moved directory through a symbolic link or another spelling of it is refused
before anything is written, because writing it differently would be a guess.

**Members.** An explicit `members` entry that names a moved task is rewritten
in place, in canonical form, so `tasks/./x` becomes `.rituals/x`. A glob over
`tasks/` stays a glob: `tasks/*` becomes `.rituals/*` when nothing else it
matches stays, and when a crate that is not a task does, the glob is kept and
the new one is added beside it. Only directories count, because Cargo skips a
matched file, and a directory counts only when something in it, at any depth,
stays: a directory that grouped tasks and empties when they move would leave a
glob that matches nothing, which Cargo reads as a literal path. A glob that
does not lead with `tasks/` is the person's own and is left alone, and the
final check judges it. Cargo's member globs match the hidden directory like
any other, so a glob stays a glob and nothing is listed explicitly.
`default-members` follows the same rules. An `exclude` entry at or under a
moved directory follows it. One that holds moved directories, such as
`tasks/group` kept out of `tasks/*`, is carried to the same place under
`.rituals/`, where `.rituals/*` would otherwise match it, and stays at the old
place as well only while something else is left there. An `exclude` glob is
left as it is, because Cargo reads `exclude` as paths.

**What it says.** Every move and every changed value, then what was cleaned up
and what is left, then every other file in the repository that mentions
`tasks/`, and last a `next:` line. Files are listed, never edited, whether or
not `migrate` edited them, because whether a path in a workflow means the task
directory is for the person to judge. The files git ignores are left out, and
the files it does not track yet are in, since a commit would carry them.

Inside the rollback, after the moves and before the final check, `migrate`
deletes the directories the moves emptied, `tasks/` included once nothing else
is in it. Whether a directory is still there decides what a member glob
matches, so the check has to read the tree the run leaves, and a run that fails
afterwards creates each one again before moving anything back into it. A
directory that could not be deleted is a line that leads with what failed, not
a failure: if leaving it breaks nothing the migration worked, and if it does,
the check says so. After the changes are kept, outside the rollback, `migrate`
lists the files that mention `tasks/`. A search that fails is only
information, so it never undoes a migration that worked, and the run still
succeeds.

**What it does not reach.** Cargo's own configuration files are not edited. A
`paths` override in `.cargo/config.toml` that points into a moved directory
makes the final `cargo metadata` fail, so the run is rolled back with Cargo's
words. The final check starts from the workspace root, so configuration that
only a build started from a member's own directory reads is not seen by it, and
such a project would still resolve from the root and then fail when built from
that directory.

## Refusals

Every task checks what it can before it writes anything. A refusal names what
it found and what to do instead. A run that fails partway through writing
puts back what it wrote, and says whether it managed to.

What ritual prints is the documentation people read most, so a next step or
a remedy is written as a command a person can copy. `new`, `create`,
`import` and `migrate` each end with a `next:` line (`migrate` only when it
changed something). A remedy that names one of
ritual's own commands spells it for the running command line: `cargo ritual
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
