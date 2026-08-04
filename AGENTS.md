# AGENTS.md

Guidance for AI coding agents in this repository.

## Commands

### Frontend (bun 1.2.19+)
```bash
bun install
bun run dev              # dev server (Turborepo)
bun run build
bun run test             # excludes E2E; also test:unit | test:integration | test:e2e | test:coverage
bun run type:check       # frontend + shared packages
bun run biome:check      # lint/format (biome:fix to auto-fix)
bun run check:turbo      # all checks (fastest full validation)
cd apps/frontend && bunx shadcn@latest add [component]
```

### Rust backend & sandbox
```bash
cargo build -p moatless-vibe-backend        # or moatless-sandbox
cargo clippy -p moatless-backend-core --all-targets --all-features -- -D warnings
cargo clippy -p moatless-backend-core --all-targets --no-default-features -- -D warnings
cargo clippy -p moatless-t3code --all-targets -- -D warnings
cargo clippy -p moatless-vibe-backend --all-targets --features db-tests -- -D warnings
cargo clippy -p moatless-sandbox -- -D warnings   # also claude-agent-sdk, moatless-events
cargo test -p moatless-vibe-backend         # or moatless-sandbox, claude-agent-sdk, moatless-events
cargo test -p moatless-backend-core
cargo test -p moatless-t3code
task rust:gc                                # reclaim target/debug/incremental
CLAUDE_SDK_E2E=1 cargo test -p moatless-sandbox --test agent_e2e -- --ignored --nocapture
```
Run those clippy lines verbatim — `--all-targets` on the three backend crates,
plain on the sandbox and the SDKs — and do not vary them. The flag set, and
`cargo check` vs `cargo clippy`, each key a separate fingerprint and a separate
rustc incremental unit. Alternating forms recompiles all ~420 dependencies (~2
min on a sandbox) and strands another 0.4–1 GB of incremental state that nothing
reclaims, because cargo has no target-directory GC. `ci.yml` runs these same
lines, so matching them keeps one warm cache instead of several cold ones. `task
rust:gc` drops `incremental/` once it passes its threshold; compiled
dependencies in `deps/` survive, so only the next build per configuration is
non-incremental.

In a sandbox the workspace volume is capped and overflowing it **evicts the
pod** — the run dies and only committed work survives. `/opt/moatless` holds
the checkout and `target/`, so check `df -h /opt/moatless` before the first
full build and between commits, and run `task rust:gc` when it tightens.
`cargo clean` is the wrong reflex: it discards `deps/` too, so the next build
recompiles every dependency into the same volume. Commit each logical change as
it goes green rather than batching, so an eviction costs one commit.

The backend's clippy line carries `--features db-tests` because 39 of the 40
modules in `backend/tests/integration/` open with `#![cfg(feature = "db-tests")]`
and compile to nothing without it — a signature they construct can change and a
bare `--all-targets` run stays green. Clippy type-checks that tier without
linking it, which is why the feature rides on a line you already run rather than
a line of its own. `ci.yml` also links it with `cargo nextest run -p
moatless-vibe-backend --features db-tests --no-run`; do not add that locally,
because `cargo test` under a feature is a separate cargo unit and the first build
of it recompiles the dependency graph again.

Those modules are one test target, `backend/tests/integration/main.rs`, not one
target per file: each links the whole backend closure, so link time scales with
the target count. A new integration test is a file in `backend/tests/integration/`
plus a `mod` line in `main.rs`, and reaches the shared helpers through
`use crate::common;`. nextest gives each test its own process, so a test may
still set env vars without disturbing its neighbours.

`moatless-backend-core` is linted twice because its `database` feature gates the
activity DAO and the whole pairing module: `--all-features` is the only run that
lints them, and `--no-default-features` is what keeps the feature droppable, so
the crate every backend crate depends on does not pull sqlx and rustls into
builds that never open a connection.

### E2E (one suite, two profiles; details: e2e-testing skill)
`tests/api` drives the product through the `moat` CLI, so the same chains run
against either environment. `E2E_PROFILE` picks which — `docker` or `k8s` — and
a test the profile cannot run skips with the missing capability named.
```bash
task test:compose:e2e [-- --suite api|ui|all]   # docker profile, whole stack; needs GITHUB_PAT
E2E_PROFILE=docker bun run test:e2e             # against a stack already up
E2E_PROFILE=k8s    bun run test:e2e             # the cluster — from inside a sandbox
E2E_PROFILE=k8s    bun tests/sweep.ts --dry-run # what a killed run left on the cluster
```

### Build / deploy / release
```bash
./docker/build-app.sh | build-backend.sh | build-sandbox.sh    # sandbox: --local --only base on Apple Silicon
bun run dev:docker                                              # local Docker Compose
task deploy [TAG=v2.0.0]                                        # also deploy:frontend, status, logs, restart
git tag v0.1.21 && git push origin v0.1.21                      # triggers release.yml
```

## Verification (mandatory after changes)
- Rust: the clippy line for that crate from the block above, verbatim, and `cargo test -p <crate>`
- Frontend: `bun run type:check` and `bun run biome:check` (quick loop: `bun run post-edit-hook`)
- Lint ratchet: `noFloatingPromises`, `noMisusedPromises`, `useExhaustiveSwitchCases` are baselined per file in `biome-baseline.json`. Touch a file with baselined violations → fix them, then `bun run lint:ratchet:update`. New files must be clean; the baseline only shrinks.

## Pragmatic coding
- Minimum code that solves the problem — nothing speculative, no single-use abstractions, no unrequested flexibility or configuration.
- Surgical changes: touch only what the task needs. Prefer deleting code. When the code you are changing sits in the wrong module, move it there first as its own commit, then change it.
- No new trait/interface with a single implementation (a test mock counts only if a test uses it).
- State assumptions when uncertain; define a verifiable success criterion (failing test, reproducible check) before starting.

## Commits
Conventional commits, enforced by commitlint; header ≤100 chars. Types: `feat fix docs style refactor test chore`. Optional scopes (others rejected): `components layout routes styles utils hooks`. Pre-push runs `bun run test:fast`.

## Architecture & conventions
- Domain glossary: `CONTEXT.md`. Historical markdown specs and ADRs were removed because they were not maintained; use current code/tests as authority and docs only as routing aids. Read `.plans/` only when asked to implement or review that specific plan.
- Durable local guidelines and decisions go in the nearest relevant `AGENTS.md`. When editing a path, read the `AGENTS.md` files in that directory and its parents.
- The repo owns its own agent-config compatibility: every `AGENTS.md` has a committed `CLAUDE.md` symlink beside it, and `.claude/skills` is a committed symlink to `.agents/skills`. Nothing creates these at runtime — add a new `AGENTS.md` → commit `ln -s AGENTS.md CLAUDE.md` alongside it.
- Keep local `AGENTS.md` guidance concise and operational: what future agents must do, what they must not do, and the current reason when it is not obvious. Do not add history dumps, plan transcripts, or long rejected-option writeups.
- Do not write essays in code comments. Comments should explain local intent, invariants, or non-obvious mechanics; durable rationale and cross-cutting decisions belong in the nearest relevant `AGENTS.md`.
- Never describe a change as replacing, superseding, or differing from legacy code or old instructions in code comments or durable docs. State the current state and rule only.
- Rust conventions live in the model-invoked `rust-dev-guidelines` skill for
  `backend/`, `sandbox/`, and `crates/`. Frontend baseline conventions live in
  `.agents/skills/frontend-dev-guidelines/SKILL.md` and its resources.
- `routeTree.gen.ts` is generated — excluded from lint/format. File naming: kebab-case (frontend, enforced by Biome), snake_case (Rust).
