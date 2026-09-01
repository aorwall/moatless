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
cargo build -p moatless-backend        # or moatless-sandbox
cargo clippy --workspace --all-targets -- -D warnings        # the floor: every member
cargo clippy -p moatless-core --all-targets --features openapi -- -D warnings
cargo clippy -p moatless-core --all-targets -- -D warnings
cargo clippy -p moatless-backend-core --all-targets --all-features -- -D warnings
cargo clippy -p moatless-backend-core --all-targets --no-default-features -- -D warnings
cargo clippy -p moatless-backend --all-targets --features db-tests -- -D warnings
cargo test -p <crate>                       # scope to the crate you changed — never --workspace
task rust:gc                                # reclaim target/debug/incremental
CLAUDE_SDK_E2E=1 cargo test -p moatless-sandbox --test agent_e2e -- --ignored --nocapture
```
Run those clippy lines verbatim. Do not vary them, and do not add a `-p` line
for the crate you touched — the floor already lints every workspace member at
`--all-targets`, so a narrower run finds nothing new and costs another
incremental unit. `crates/codex-sdk` is the exception: it is excluded from the
workspace, so the floor never reaches it and it is linted from its own
directory. `ci.yml` runs this same set.

Never run `cargo check` — `cargo clippy` already covers it and keys a separate
set of incremental units. Scope every `cargo test` to the crate you changed for
the same reason.

In a sandbox the workspace volume is capped and overflowing it **evicts the
pod** — the run dies and only committed work survives. `/opt/moatless` holds the
checkout and `target/`, so check `df -h /opt/moatless` before the first full
build and between commits, and run `task rust:gc` when it tightens.

Why the set is shaped this way, what each invocation costs, how the db-tests
tier and the three integration targets work, and how to point rust-analyzer at
the same units: `rust-dev-guidelines` skill, `references/build-costs.md` and
`references/backend.md`. Read them before changing a cargo invocation.

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
- Rust: the floor line, plus any `-p` line above that names the crate you touched, verbatim — and `cargo test -p <crate>`
- Frontend: `bun run type:check` and `bun run biome:check` (quick loop: `bun run post-edit-hook`)
- Lint ratchet: `noFloatingPromises`, `noMisusedPromises`, `useExhaustiveSwitchCases` are baselined per file in `biome-baseline.json`. Touch a file with baselined violations → fix them, then `bun run lint:ratchet:update`. New files must be clean; the baseline only shrinks.

## Pragmatic coding
- No new trait/interface with a single implementation (a test mock counts only if a test uses it).
- When the code you are changing sits in the wrong module, move it there as part of the same change, and say so in the PR body so a reviewer reads the rename separately from the edits that follow it.
- Define a verifiable success criterion — a failing test or a reproducible check — before starting.

## Commits
Conventional commits — `type(optional-scope): subject`, header ≤100 characters.

**`commitlint.config.ts` is the authority for which types and scopes are valid**, and this file deliberately does not restate either list: it extends `@commitlint/config-conventional` for the types and pins its own `scope-enum`. A second copy of an enum drifts from the one that runs, and a reader cannot tell which is current. To settle a specific case, ask the linter — `printf 'feat: x\n' | bunx commitlint`.

**Two checks, because they cover different commits.** `.husky/commit-msg` lints each local commit. `.github/workflows/pr-title.yml` lints the **pull request title**, which is the message that reaches `main`: this repo squash-merges, and GitHub composes the squash commit from the title server-side, where no git hook runs. That workflow triggers on `edited` as well as `opened`, so a retitle re-runs it — a title check that ran only at open time would attest to a title that no longer ships.

**The title check is advisory.** It is not in `main`'s required-status-check list, so a red title check does not block a merge; a reviewer has to notice it. Making it required is a repository setting.

**One commit per pull request.** This repo squash-merges, so a branch's internal commits never reach `main` — GitHub composes the squash commit from the PR title. Splitting work across local commits buys nothing a reviewer sees and costs the time spent keeping each one green. Land a PR as a single commit and use the PR body to sequence anything a reader needs in order.

Pre-push runs `bun check:pre-push`, which is Biome and type-check only — no
tests. A push proves nothing about test state; run the verification commands
above yourself.

## Architecture & conventions
- Domain glossary: `CONTEXT.md`. Historical markdown specs and ADRs were removed because they were not maintained; use current code/tests as authority and docs only as routing aids. Read `.plans/` only when asked to implement or review that specific plan. Unscoped feature ideas not yet committed to a plan go in `docs/ideas/` instead — see `docs/ideas/README.md`.
- Durable local guidelines and decisions go in the nearest relevant `AGENTS.md`. When editing a path, read the `AGENTS.md` files in that directory and its parents.
- Found a real problem you are deliberately not fixing now? Record it in `docs/deferred-changes.md` with an anchor and the condition that makes it worth doing. Check that file before starting work in an area — it is where the known gaps are, and its own rules keep it from rotting.
- The repo owns its own agent-config compatibility: every `AGENTS.md` has a committed `CLAUDE.md` symlink beside it, and `.claude/skills` is a committed symlink to `.agents/skills`. Nothing creates these at runtime — add a new `AGENTS.md` → commit `ln -s AGENTS.md CLAUDE.md` alongside it.
- Keep local `AGENTS.md` guidance concise and operational: what future agents must do, what they must not do, and the current reason when it is not obvious. Do not add history dumps, plan transcripts, or long rejected-option writeups.
- Comments: necessary and essential only. Default to none. A comment earns its place by carrying a fact the code cannot — an invariant, a "do not do X" and what breaks if you do, a non-obvious mechanic, or a unit/ordering the signature does not state. A public item gets a one-line doc saying what it is. Working ceiling: three lines for a comment, eight for a module doc. Past that the content is durable rationale, and it belongs in the nearest relevant `AGENTS.md` — link to it, do not copy it.
- Do not restate the code, narrate what the reader can already see, argue for the design, or explain what an alternative would have cost. Never describe a change as replacing, superseding, or differing from legacy code or old instructions — no "used to", no "no longer", no "the old X". State the current rule only.
- Rust conventions live in the model-invoked `rust-dev-guidelines` skill for
  `crates/backend/`, `sandbox/`, and `crates/`. Frontend baseline conventions live in
  `.agents/skills/frontend-dev-guidelines/SKILL.md` and its resources.
- `routeTree.gen.ts` is generated — excluded from lint/format. Edit the routes, not the tree.
