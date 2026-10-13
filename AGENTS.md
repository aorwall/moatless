# AGENTS.md

Moatless Vibe runs coding agents (Claude Code, Codex) in on-demand sandboxes
against git repositories, driven by a person or on a schedule by a Loop.
`CONTEXT.md` is the glossary. `packages/` holds the shared TypeScript packages,
`crates/` the Rust backend and sandbox, `tests/` the E2E suite, and `docker/`,
`docker-compose/` and `k8s/` the deployment definitions.

The UI is T3 Code, built from the separate `soaplabs/t3code` repository and
served over UI-RPC (`crates/t3code`); see the `t3-development` skill. Do not
change `apps/frontend`: it is the retired UI, and nothing builds or deploys it.

## Commands

`testing-guidelines` owns the other test entry points and `e2e-testing` the
E2E suite.

```bash
bun install && bun run build
bun run test             # excludes E2E
bun run check:turbo      # every TypeScript check

cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings        # the floor
cargo clippy -p moatless-core --all-targets --features openapi -- -D warnings
cargo clippy -p moatless-core --all-targets -- -D warnings
cargo clippy -p moatless-ports --all-targets --all-features -- -D warnings
cargo clippy -p moatless-ports --all-targets --no-default-features -- -D warnings
task rust:clippy:db-tests                   # every package declaring db-tests
cargo test -p <crate>                       # never --workspace
bash .github/scripts/test-workspace-contracts.sh
task rust:gc                                # reclaim target/debug/incremental

task deploy [TAG=v2.0.0]                    # also deploy:backend, status, logs
git tag v0.1.21 && git push origin v0.1.21  # triggers release.yml
```

- Run the clippy lines verbatim. Add no `-p` line of your own, and never run
  `cargo check`. `crates/codex-sdk` is outside the workspace; its `Cargo.toml`
  carries its own lines.
- Read `rust-dev-guidelines` before you change a cargo invocation, and its
  `build-costs.md` before a full build: a build is bounded by memory and disk.

## Verifying a change

Run these after every change; pre-push runs only Biome and type-check.

- **Rust:** `cargo fmt --all`, the floor line plus any clippy line naming the
  crate you touched, `cargo test -p <crate>`, and
  `cargo test -p moatless-arch-rules` (never `cargo nextest`). Then run
  `cargo run -p moatless-arch-rules --bin arch-report -- --comments --since main`
  for the comment ceilings.
- **TypeScript:** `bun run type:check` and `bun run biome:check`.
- **Lint ratchet:** fix the `biome-baseline.json` entries in any file you touch,
  then run `bun run lint:ratchet:update`. A new file must be clean.
- **Docs:** when you remove a symbol or change a domain concept, grep `.plans/`
  and `CONTEXT.md` for it, then run `./scripts/check-plan-assertions.sh`.

## Writing code

- Add no trait or interface with a single implementation, unless a test mocks
  it.
- Move code that sits in the wrong module in the same change, and say so in the
  PR body.
- Define a success criterion before you start: a failing test or a
  reproducible check.

## Commits and pull requests

- Write conventional commits, header 100 characters or fewer.
  `commitlint.config.ts` owns the types and scopes; test a header with
  `printf 'feat: x\n' | bunx commitlint`.
- The repo squash-merges, so the PR title becomes the commit on `main`. Lint the
  title too: its CI check is advisory and blocks no merge.
- Land one commit per PR, and sequence anything a reader needs in the PR body.
- Record a real problem you are deliberately not fixing in
  `docs/deferred-changes.md`, with an anchor and the condition that makes it
  worth doing. That includes a gap the PR body lists as not covered. Read the
  file before you start work in an area.

## Writing a comment

A comment carries a fact the code cannot show, so it never restates the code.
Write none unless the fact is an invariant, a "do not do X" with what breaks,
a non-obvious mechanic, or a unit or ordering the signature omits. A public
item gets a one-line doc.

- Stop at five lines for a comment and eight for a module doc, counted wrapped
  at 80 columns with blank lines free. Past that, name the kind of fact that
  needs the length, or cut back.
- Delete what no reader needs; never move it into an `AGENTS.md`. Text goes to
  the commit or PR only when a reviewer of that change needs it.
- State a design fact once, on the symbol that owns it, and link to it
  elsewhere. A trait method's or props field's doc is its whole contract. A
  precondition binding every caller goes on the owning function's doc, and each
  call site links back in one line.
- End a rationale on what breaks under the alternative, named with
  `deliberately` or `rather than`; the alternative itself goes in the PR.
- Put a trap on the line it protects: symptom, cause, fix.
- Use the code's own names, in backticks. A negative names the wrong assumption
  it prevents: "No `Activated` row here: `active` was already true", not "Not
  the same as `Pending`".
- Narrate no change: "previously" and "no longer" belong in the commit.
- `rust-dev-guidelines` holds the Rust specifics. `arch-rules` enforces this
  file's budget and duplicated prose; the comment ceilings are advisory.

## Finding conventions

- Treat code and tests as the authority, and documents as routing aids. Read
  `.plans/` only when asked to implement or review that plan; an unplanned idea
  goes in `docs/ideas/`.
- When you edit a path, read the `AGENTS.md` files in its directory and parents.
  Put a durable local rule in the nearest one, and do not restate a parent's.
- An `AGENTS.md` holds what an agent must and must not do, where the code does
  not show it. History and rejected alternatives go in the commit or PR.
- Every `AGENTS.md` has a committed `CLAUDE.md` symlink beside it
  (`ln -s AGENTS.md CLAUDE.md`), and `.claude/skills` links to `.agents/skills`.
- `routeTree.gen.ts` is generated. Edit the routes, never the tree.
