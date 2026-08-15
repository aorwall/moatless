# Build Costs and the Clippy Set

Why the commands in the root `AGENTS.md` are the ones they are, and what it
costs to run something else. Read this before adding, varying, or removing a
cargo invocation — not before an ordinary code change.

## Incremental units

Every distinct flag combination keys its own rustc incremental unit **per
crate**, so the cost scales with an invocation's scope, not its count. Measured
on this workspace:

| Invocation | `target/debug/incremental` |
|---|---|
| A `-p`-scoped clippy line | 100–200 MB |
| `cargo check --workspace` | 1.7 GB |
| `cargo test --workspace --no-run` | 5.3 GB |
| rust-analyzer at its default settings | 1.8 GB |

Two rules follow, and both are in the root `AGENTS.md`:

- Scope every `cargo test` to the crate you changed.
- Never run `cargo check`. `cargo clippy` already covers it and keys a separate
  unit set, so a `check` run buys nothing and pays 1.7 GB.

A unit is written at full size the first time its crate recompiles and never
grows after. Editing costs nothing: the total is purely how many
(crate, configuration) pairs have ever been built, nothing reclaims the ones
that fall out of use, and that includes the pairs of every crate layout the
workspace has passed through.

`task rust:gc` drops units once the directory passes `CARGO_GC_THRESHOLD_MB`
(default 4096): those untouched for `CARGO_GC_MAX_AGE_DAYS` (7) first, then
least-recently-used until under. Cargo's fingerprints and `deps/` are untouched,
so a no-op build after a GC stays a no-op; only the next build that recompiles a
dropped unit's crate pays, and it pays roughly 3x (11.6s → 30s on the floor
line).

## rust-analyzer

An editor is a configuration too. Point rust-analyzer at the floor's exact
command so it shares those units instead of keying its own 1.8 GB set:

```json
"rust-analyzer.check.command": "clippy",
"rust-analyzer.check.extraArgs": ["--", "-D", "warnings"],
"rust-analyzer.check.workspace": true,
"rust-analyzer.check.allTargets": true
```

`extraArgs` is not optional — cargo hashes it, so clippy without it lands in the
same unit set as plain `cargo check` and saves nothing.

## Why each clippy line exists

The floor, `cargo clippy --workspace --all-targets -- -D warnings`, lints every
workspace member, so a new crate is gated the day it lands. `--all-targets` is
what makes `#[cfg(test)]` code reach the lints the workspace denies.

`--workspace` unifies each member's features to a single set, so the floor lints
every crate at exactly one feature configuration — the union the dependency
graph resolves to — and reaches neither a feature's off-state nor a different
graph. The `-p` lines exist only where that is structurally true:

- **`moatless-core`, twice.** `crates/backend/Cargo.toml` pulls it with
  `features = ["openapi"]`, so the floor always lints it openapi-on. The
  `--features openapi` run pins the gated code (the `ToSchema` derives, the
  `utoipa::path` attributes) in isolation from that unification; the default run
  is what keeps the feature droppable, without which utoipa stops being off for
  the non-API consumers the feature protects.
- **`moatless-backend-core`, twice.** Its `database` feature gates the activity
  DAO and the whole pairing module, which the default floor run omits.
  `--all-features` is the only run that lints them; `--no-default-features`
  keeps the feature droppable, so the crate every backend crate depends on does
  not pull sqlx and rustls into builds that never open a connection.
- **`moatless-backend` with `--features db-tests`.** Every module in
  `crates/backend/tests/integration_*/` except `supervisor_test.rs` opens with
  `#![cfg(feature = "db-tests")]` and compiles to nothing without it, so a
  signature they construct can change while a bare `--all-targets` run stays
  green. Clippy type-checks that tier without linking it, which is why the
  feature rides on a line already being run rather than one of its own. It is
  the most expensive line in the set and the one an edit under `crates/backend`
  pays every time — 44s, against 5s for the floor.

`ci.yml` runs this same set, and additionally links the db-tests tier with
`cargo nextest run -p moatless-backend --features db-tests --no-run`. Do
not add that locally: `cargo test` under a feature is a separate cargo unit, and
its first build recompiles the dependency graph again.

`crates/codex-sdk` is excluded from the workspace, so the floor never reaches
it. It is linted from its own directory.

## The integration test targets

`crates/backend/tests/` is three targets rather than one because the db-tests
clippy line is the edit loop's long pole and a target is a single rustc unit: a
23.7k-line suite in one target type-checked in one 63s unit, so the line cost
67s on every `crates/backend` edit no matter how many cores were idle. Split
three ways it is 44s, measured as three alternating runs of each layout on
8 cores (67/68/69s against 43/43/45s).

Splitting trades CPU for wall clock — each target compiles `tests/common` and
re-instantiates the generics it uses, so total CPU rose 67s → 77s. It does not
trade against link time, which the one-target layout was chosen for: under mold
three smaller links beat one large one, and `cargo test --features db-tests
--no-run` after the same edit went 33s → 27s.

Three is where the return stops. Two targets already give most of it (44s), and
past that rustc's parallel frontend has the cores saturated — with three, the
last target does not start until the first finishes. The floor is unaffected
either way (5s), since without `db-tests` every module but `supervisor_test.rs`
compiles to nothing.

## Sandbox builds

The base image carries mold (as `/usr/bin/ld`, so nothing opts into it) and
sccache. When the deployment sets `SCCACHE_S3_BUCKET`, the entrypoint points
sccache at that bucket and exports `RUSTC_WRAPPER`, so a first build reuses
objects an earlier sandbox compiled. It shares nothing with CI: sccache keys on
absolute paths, and the two disagree on the checkout dir and `CARGO_HOME`.

The cache is not a substitute for `target/` — cargo still relinks, and the
volume cap in the root `AGENTS.md` still applies in full.

`cargo clean` is the wrong reflex when that volume tightens: it discards `deps/`
too, so the next build recompiles every dependency into the same volume. Use
`task rust:gc`.
