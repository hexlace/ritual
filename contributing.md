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

Ritual follows [Semantic Versioning](https://semver.org/). Every crate here
is released at the workspace version, together. `new` writes one version for
both `rituals` and `rituals-core` into a project, so a release of one without
the other leaves new projects asking for a version that does not exist. Every
member inherits `version.workspace = true`, every internal requirement in the
root `[workspace.dependencies]` is that version, and
`crates/rituals-cli/tests/every_crate_releases_at_the_workspace_version.rs`
fails if either stops being true. The one member that is never published,
`xtask`, inherits the version too.

A release takes three steps, and GitHub Actions does the work between them:

1. Run the **Release** workflow from the Actions tab, on `main`, with the tag:
   `v` then `MAJOR.MINOR.PATCH`, such as `v0.1.1`. It refuses a tag that is
   not newer than `main`'s version. It moves every version site and
   `Cargo.lock`, commits that to the branch `release/<tag>` as
   `chore(release): <tag>`, and opens a pull request. A workflow opened the
   pull request, so its CI waits for approval: approve the run, or close and
   reopen the pull request.
2. Merge the pull request. **Release draft** then drafts the GitHub release
   `<tag>` at the merge commit. Its body has a marked place for prose at the
   top, then every pull request merged since the previous release, then
   everyone who authored or co-authored a commit.
3. Write the prose and publish the release. Publishing creates the tag, and
   **Publish** publishes every crate to crates.io through trusted publishing.
   If it fails partway, run it again: crates already on crates.io at that
   version are skipped.

Each of these workflows runs as two jobs, split on one rule: **a token that
can write, to this repository or to crates.io, only ever exists in a job that
compiles nothing.** `cargo xtask` is `cargo run`, so it compiles xtask and runs
its dependencies' build scripts, and every step in a job runs as the same user
on the same machine, so code one step runs can read what a later step is
given. Moving a token to a later step does not keep it away from an earlier
one.

- **Release**: `bump` runs `cargo xtask bump` with read access only, and hands
  `Cargo.toml` and `Cargo.lock` on. `open-pull-request` checks the tag and
  that those two files are all that changed, then commits them and opens the
  pull request.
- **Release draft**: `verify` runs `cargo xtask verify-tag` and
  `cargo xtask release-contributors` with read access only, and hands the
  previous release and the Contributors section on. `draft` asks GitHub for
  its generated notes, which needs write access, assembles the body and
  creates the draft.
- **Publish**: `verify` holds no credential: it checks that the tagged commit
  is on `main`, then runs `cargo xtask publish-plan`, which does every build a
  publish verifies. Then `publish` checks out that same commit, gets a token,
  and runs `cargo publish --no-verify`, which packages and uploads without
  compiling anything.

Each writing job checks what it was handed in shell, because the job that
produced it had run third-party code by then. Keep it that way: a step that
compiles, including `cargo xtask`, never goes in a job that can write.

What changed in a release lives in its GitHub release, not in a file here.

The workflows run `cargo xtask`, and so can you:

```sh
cargo xtask bump v0.1.1            # what step 1 does to the tree
cargo xtask verify-tag v0.1.1      # refuses unless the workspace is at v0.1.1
cargo xtask publish-plan v0.1.1    # dry-runs the publish, prints the plan
cargo xtask release-contributors hexlace/ritual v0.1.1 <commit> <dir>   # needs `gh`
```

`publish-plan` asks crates.io which crates it already has at the tag's
version, then packages and builds every other one without uploading
anything, and it has to run on a committed tree. It prints the plan on
stdout: a `publish=` line and an `exclude=` line, the form the workflow hands
from one job to the next. The dry run checks the packages, not the registry:
a version crates.io already has, such as a yanked one, shows up only as a
warning, where a real publish refuses it. None of these commands uploads
anything: only the workflow publishes.

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
