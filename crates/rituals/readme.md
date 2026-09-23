# rituals

What a [ritual](https://github.com/hexlace/ritual) task is, and how a composed
command line dispatches to one. This is the only crate a task author depends
on.

## A task crate

A task is an ordinary library crate. Its manifest depends on `rituals` and
marks the crate as a task:

```toml
[package]
name = "hello"
version = "0.1.0"
edition = "2024"

[dependencies]
rituals = "0.1"

[package.metadata.ritual]
task = true
```

Its library exposes one function, `task()`, built from a clap argument
struct and the function that runs it:

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

The first argument to `Task::new` is the one-line description `--help`
shows, not the task's name. A task never names itself: the command it
answers to is the dependency key of whichever project imports it.

`report` writes one line to stdout. Arguments are ordinary clap, re-exported
as `rituals::clap`, so a task crate needs no parser dependency of its own.

## Refusing

A task that cannot do its job returns a `Failure`. The command line prints
it after its own name and exits with status 1:

```rust
use std::path::PathBuf;

use rituals::{Failure, Outcome, clap, report};

#[derive(clap::Args)]
struct Arguments {
    /// the file to count lines in
    path: PathBuf,
}

fn run(arguments: Arguments) -> Outcome {
    let text = std::fs::read_to_string(&arguments.path).map_err(|error| {
        Failure::new(format!("reading {} failed", arguments.path.display())).caused_by(error)
    })?;
    report(format!("{} lines", text.lines().count()));
    Ok(())
}
```

```text
$ cargo ritual lines missing.txt
ritual: reading missing.txt failed: No such file or directory (os error 2)
```

## Running a task

A task runs inside a project's command line. The `ritual` binary, from
[`rituals-cli`](https://crates.io/crates/rituals-cli), makes both:

```sh
cargo install --locked rituals-cli
ritual new demo
cd demo
cargo ritual add hello
cargo ritual hello world
```

`add` writes the manifest and the `task()` above into `tasks/hello`, imports
it, and regenerates the command line. To write a task crate outside any
project, for several projects to share, run `ritual create hello` and
[import it](https://github.com/hexlace/ritual#import-a-task-from-somewhere-else).

## More than one command

- `Task::group` builds a bundle: a task whose command is a group of named
  child tasks. A bundle is imported like any other task, and bundles nest.
- `Task::receiving_command_line` builds a task that is handed the command
  line it runs in: its name and version, and the commands at its top level.

The [ritual readme](https://github.com/hexlace/ritual#readme) covers the
concepts, and [the design](https://github.com/hexlace/ritual/blob/main/.docs/design.md)
explains why ritual is shaped this way.
