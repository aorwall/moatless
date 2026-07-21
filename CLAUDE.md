# CLAUDE.md

Guidance for Claude Code in this repository.

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
cargo clippy -p moatless-vibe-backend -- -D warnings
cargo test -p moatless-vibe-backend         # or moatless-sandbox, claude-agent-sdk, moatless-events
CLAUDE_SDK_E2E=1 cargo test -p moatless-sandbox --test agent_e2e -- --ignored --nocapture
```

### Integration & E2E (Taskfile + Docker; details: e2e-testing skill)
```bash
task test:compose:db         # tiers: db | sandbox (SANDBOX_E2E=1) | agent (CLAUDE_SDK_E2E=1) | all
task test:compose -- --test <file> [--skip-build --tier <t>] [--keep-running]
task test:compose:e2e [-- --suite ui]       # Playwright; needs GITHUB_PAT
```

### Build / deploy / release
```bash
./docker/build-app.sh | build-backend.sh | build-sandbox.sh    # sandbox: --local --only base on Apple Silicon
bun run dev:docker                                              # local Docker Compose
task deploy [TAG=v2.0.0]                                        # also deploy:frontend, status, logs, restart
git tag v0.1.21 && git push origin v0.1.21                      # triggers release.yml
```

## Verification (mandatory after changes)
- Rust: `cargo clippy -p <crate> -- -D warnings` and `cargo test -p <crate>`
- Frontend: `bun run type:check` and `bun run biome:check` (quick loop: `bun run post-edit-hook`)
- Lint ratchet: `noFloatingPromises`, `noMisusedPromises`, `useExhaustiveSwitchCases` are baselined per file in `biome-baseline.json`. Touch a file with baselined violations → fix them, then `bun run lint:ratchet:update`. New files must be clean; the baseline only shrinks.

## Pragmatic coding
- Minimum code that solves the problem — nothing speculative, no single-use abstractions, no unrequested flexibility or configuration.
- Surgical changes: touch only what the task needs; don't refactor working code (the lint ratchet above is the exception). Prefer deleting code.
- No new trait/interface with a single implementation (a test mock counts only if a test uses it).
- State assumptions when uncertain; define a verifiable success criterion (failing test, reproducible check) before starting.

## Commits
Conventional commits, enforced by commitlint; header ≤100 chars. Types: `feat fix docs style refactor test chore`. Optional scopes (others rejected): `components layout routes styles utils hooks`. Pre-push runs `bun run test:fast`.

## Architecture & conventions
- Overviews: [docs/architecture-overview.md](./docs/architecture-overview.md), [docs/sandbox-architecture.md](./docs/sandbox-architecture.md), [docs/sandbox-lifecycle.md](./docs/sandbox-lifecycle.md). Domain glossary: `CONTEXT.md`. Decisions: `docs/adr/`.
- Per-area conventions live in `.claude/rules/` (path-scoped, auto-attached): backend module layering, DAO/handler/error patterns, sandbox crate, frontend conventions.
- `routeTree.gen.ts` is generated — excluded from lint/format. File naming: kebab-case (frontend, enforced by Biome), snake_case (Rust).
