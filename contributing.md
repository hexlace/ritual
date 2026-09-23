# Contributing

Contributions are welcome. The process for them is being worked out as of
September 22, 2026, and issues and pull requests are expected to open soon.
This guide covers working on ritual itself: the toolchain, the gate, and
releasing.

## Toolchain

The toolchain is pinned in `rust-toolchain.toml`, so plain `rustup` picks it
up. With [mise](https://mise.jdx.dev/), trust the config once per checkout,
then install:

```sh
mise trust
mise install
```

The pinned toolchain is what the gate runs on. The minimum supported Rust
version for users is separate: it is the `rust-version` in the workspace
manifest. Raising it is not a breaking change, and it lands in a minor
release.

## The gate

Four commands. All of them must pass:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

CI runs the same four, and two more: every crate is packaged and built from
its package alone, as crates.io would receive it, and the test suite runs
on the oldest toolchain its `rust-version` declares.

The tests in `crates/rituals-cli/tests/` scaffold and build real projects
with `new`, `create` and `add`. Each project builds on its own, so the suite
compiles far more than the workspace itself, and because a scaffolded project
has no lockfile, it needs network access or a warm Cargo registry cache.

## Releasing

Every crate here is released at the workspace version, together, with
`cargo publish --workspace`. `new` writes one version for both `rituals` and
`rituals-core` into a project, so a release of one without the other leaves
new projects asking for a version that does not exist. Every member inherits
`version.workspace = true`, every internal requirement in the root
`[workspace.dependencies]` is that version, and
`crates/rituals-cli/tests/every_crate_releases_at_the_workspace_version.rs`
fails if either stops being true.

## Running ritual from a checkout

Inside this checkout, `cargo ritual <task>` runs this repository's own
composed command line without installing anything. It is
`cargo run --package rituals-cli --`, aliased in `.cargo/config.toml`.

`crates/rituals-cli/src/main.rs` is that command line's generated file. After
changing `[package.metadata.ritual] tasks` in `crates/rituals-cli/Cargo.toml`,
run `cargo ritual regenerate`. Never edit the file by hand.

To install the global binary from the checkout:

```sh
cargo install --locked --path crates/rituals-cli
```

To scaffold a project against the checkout rather than a published release,
pass it as the source:

```sh
ritual new demo --path /path/to/ritual
```

## Task crates in this repository

`add` and `create` write a task crate's manifest with no `[lints]` table. A
project's lint table is unknown to the scaffold, and inheriting it could fail
a scaffolded handler before its author has written a line. For example,
under `clippy::pedantic` a handler that always returns `Ok(())` trips
`unnecessary_wraps`.

This repository's own task crates hold themselves to the workspace's
standard anyway. Each one carries `[lints] workspace = true`, added by hand,
and has a real handler body rather than the scaffold's placeholder. A task
crate added here should do the same.

## Design

Read [`.docs/design.md`](.docs/design.md) for why ritual is shaped the way it
is, and [`.docs/crates.md`](.docs/crates.md) for how its crates divide the
work. A change to the design changes those documents in the same pull
request. `.docs/` is also an [Obsidian](https://obsidian.md/) vault, if you
prefer to read it that way; it is plain markdown either way.
