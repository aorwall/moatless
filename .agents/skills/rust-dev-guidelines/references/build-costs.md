# Build costs and verification configurations

Read this before changing a Cargo invocation. The root AGENTS.md owns the required
local command list; [CI](../../../../.github/workflows/ci.yml) owns the automated
matrix. Use its current definitions rather than copying another list here.

## Why separate configurations exist

The workspace `--all-targets` floor lints production and test code with the
features resolved across the workspace. It does not prove every isolated feature
configuration. Core's openapi/default passes and ports' all-features/no-default
passes cover those boundaries. Ports currently exposes `openapi` and `test-support`;
its manifest is the authority for feature names.

CI also checks adapter-model with `--lib` in isolation, so dev-dependency feature
unification cannot hide a dependency-boundary violation. It discovers packages
with `db-tests` from Cargo metadata, lints them, and runs them against a Postgres
service. On PRs it
combines packages and qualified feature names; other runs check packages separately.
The ports no-default pass is conditional on manifest changes in CI. The required
local list is intentionally explicit and is not identical to that dynamic matrix.

A scoped plain `cargo test -p <crate>` does not cover its gated database
tier; select the owning crate's feature when testing database behavior.

## Keep build artifacts bounded

Different crate/feature/profile/flag combinations can create additional compiler
artifacts. Run scoped tests, and never run `cargo check`. Keep each worktree's
own `target/`; do not share it across checkouts. If an error names a field
present in source, check for shared artifacts as one possible cause before
assuming the source is wrong.

A build is bounded by memory before disk: a cold floor line peaks around 6 GiB
of the sandbox's cgroup cap and leaves about 7 GiB in `target/`. Read
`/sys/fs/cgroup/memory.peak` after a build, and lower `CARGO_BUILD_JOBS` when it
nears `memory.max`.

Before a full sandbox build and between commits, measure the workspace volume.
Do not use `df`: the volume's limit (`SANDBOX_WORKSPACE_LIMIT`) is metered
outside the pod, and `df` reports the node's device. Add up `du -sx` on each
mount point the volume backs: `/opt/moatless`, `/tmp`, and `/opt/cargo/registry`
plus `/opt/cargo/git` when the cargo cache is persisted. Overflow can evict the
pod. `task rust:gc` reclaims incremental units while retaining dependency
artifacts; `cargo clean` empties `target/`, which a Kubernetes sandbox then
refills from the shared compile cache below. The cache and GC implementation
in `Taskfile.yml` owns thresholds.

## Reuse the shared compile cache

In a Kubernetes sandbox rustc runs through sccache against a shared S3 bucket
(`docker/sccache/sccache-env.sh`), so a fresh `target/` fills mostly from
objects other sandboxes compiled. sccache keys on the compile's directory and
the `CARGO_*` variables rustc inherits, so a build hits only when it matches
theirs:

- Build in the primary checkout's default `target/`. A `CARGO_TARGET_DIR`
  misses on every crate, and a second worktree misses on every workspace crate.
- Add no `CARGO_*` variable to a cargo command other than `CARGO_BUILD_JOBS`,
  which still hits. `CARGO_INCREMENTAL=1` fails every compile under sccache
  with `incremental compilation is prohibited`.
- `sccache --show-stats` reports the hits. An empty `RUSTC_WRAPPER` means the
  shell compiles uncached; a `[sccache]` line on stderr names an unreachable
  bucket.

In October 2026 the floor line took 3m58s from an empty `target/` with 97% of
compiles hitting, and peaked at 5.3 GiB.

Historical measurements from September 2026 illustrate the cost, not a timing
promise for the current crate graph: a cold workspace floor took 6m37s and about
3 GB; a warm core edit took 21s in a dedicated target versus 45s after another
worktree wrote a shared target. That shared directory reached 100 GB. The backend
integration suite's three targets reduced one measured lint pass from about 67s
to 44s. Compare warm runs on the same layout before changing target partitioning.

## Editor and shadow builds

Configure rust-analyzer to invoke the same clippy workspace/all-targets command
and warning flags as the floor. Check effective editor settings before attributing
extra artifacts to Cargo itself.

Shadow scripts use `SHADOW_CARGO_PROFILE`, defaulting to release. Set it to `dev`
in both the backend server environment and your rebuild shell to share the dev
profile used by tests. A mismatched profile rebuilds a binary the server does not
start. See [shadow-dev](../../shadow-dev/SKILL.md).
