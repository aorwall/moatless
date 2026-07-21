# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Frontend (bun 1.2.19+)
```bash
bun install
bun run dev              # frontend dev server (Turborepo)
bun run build            # production build
bun run test             # standard tests (excludes E2E)
bun run test:unit        # fastest
bun run test:integration
bun run test:e2e         # Playwright, slowest
bun run test:coverage
bun run type:check       # frontend + shared packages
bun run biome:check      # lint/format check
bun run biome:fix        # auto-fix
bun run check:turbo      # all checks (Turborepo)
cd apps/frontend && bunx shadcn@latest add [component]
```

### Rust backend & sandbox
```bash
cargo build -p moatless-vibe-backend        # or moatless-sandbox
cargo clippy -p moatless-vibe-backend -- -D warnings
cargo test -p moatless-vibe-backend         # or moatless-sandbox, claude-agent-sdk, moatless-events

# E2E (requires `claude` CLI + API key)
CLAUDE_SDK_E2E=1 cargo test -p moatless-sandbox --test agent_e2e -- --ignored --nocapture
CLAUDE_SDK_E2E=1 cargo test -p claude-agent-sdk -- --ignored
```

### Integration tests (Taskfile + Docker Compose; requires taskfile.dev, Docker)
```bash
task test:compose:db         # DB tier: API CRUD, auth, channels (no sandbox, fastest)
task test:compose:sandbox    # sandbox tier: file ops, lifecycle (builds sandbox image)
task test:compose:all        # all tiers incl. agent (needs CLAUDE_CODE_OAUTH_TOKEN in .env)
task test:compose -- --test message_acknowledgment      # single file
task test:compose -- --skip-build --tier agent          # reuse images
task test:compose -- --keep-running --test <name>       # keep containers for debugging
```
Test tiers: **db** (non-ignored), **sandbox** (`SANDBOX_E2E=1`), **agent** (`CLAUDE_SDK_E2E=1`), **all** (db then all ignored).

### Playwright / SDK E2E in Docker
```bash
task test:compose:e2e                  # API suite (needs GITHUB_PAT)
task test:compose:e2e -- --suite ui    # browser suite (builds frontend)
task test:sdk:claude:docker -- smoke   # claude-agent-sdk vs pinned Claude CLI
OPENAI_API_KEY=sk-... task test:sdk:codex:docker   # Codex `codex exec` path
```
All of the above also run via the manual `E2E (manual)` workflow (`.github/workflows/e2e.yml`).

> **Apple Silicon**: `docker/build-sandbox.sh --local --only base` builds native arm64 sandbox images.

### Docker build / K8s deploy / release
```bash
./docker/build-app.sh [--tag v1.0.0] [--push]      # frontend
./docker/build-backend.sh [IMAGE_NAME] [TAG]        # backend
./docker/build-sandbox.sh [--tag v1.0.0] [--push]   # sandbox
bun run dev:docker                                   # local Docker Compose

task deploy [TAG=v2.0.0]     # build, push, deploy; also deploy:frontend, status, logs, restart

git tag v0.1.21 && git push origin v0.1.21           # triggers release.yml
```

## Mandatory Workflow

### Frontend changes
- You MUST run `bun run type:check` after code changes to catch TypeScript errors
- You MUST run `bun run biome:check` after code changes for linting and formatting
- You can run both `biome:fix` and `biome:check`
- For comprehensive validation, run `bun run check:turbo` which includes all checks (optimized for speed)
- **Quick validation after edits**: Run `bun run post-edit-hook` for fast formatting fixes and type checking (~100ms with caching)

#### Lint ratchet (nursery rules)
`noFloatingPromises`, `noMisusedPromises`, and `useExhaustiveSwitchCases` are
enabled as `warn` in `biome.json` (so `biome:check` reports but doesn't fail on
them) and gated by a ratchet: `bun run lint:ratchet` fails if any file exceeds
its baselined violation count in `biome-baseline.json` (a file with no entry
must be clean). The baseline can only shrink.
- If you touch a file with baselined violations of these rules, **fix them** —
  do not raise the baseline. After removing violations, run
  `bun run lint:ratchet:update` to lock in the smaller baseline.
- New code must be clean: any new violation in a file without a matching
  baseline entry fails the ratchet.

### Rust backend/sandbox changes
- You MUST run `cargo clippy -p <crate> -- -D warnings` after code changes
- You MUST run `cargo test -p <crate>` to verify tests pass

## Commit Conventions

Conventional commit format, enforced by commitlint. Header (first line) must be ≤100 characters.

```
<type>: <description>
```

- **Types**: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`
- **Scopes** (optional; commitlint rejects others): `components`, `layout`, `routes`, `styles`, `utils`, `hooks`. Omit the scope for anything else — `feat: description`.

Pre-commit hooks: **pre-push** runs `bun run test:fast`; **commit-msg** validates format via commitlint.

## Critical Development Rules

**Type Safety (TypeScript — NEVER violate)**:
- Use TypeScript strict mode
- **NEVER use `any`** — use proper types or `unknown`
- **NEVER cast to `any`** — fix the underlying type issue
- Use type guards and narrowing for runtime type checking

**Rust Safety**:
- All warnings must be resolved (`-D warnings` in clippy)
- Use `thiserror` for error types, not string errors
- Use `Arc` for shared state across async tasks
- Use `CancellationToken` from tokio-util for graceful shutdown

**State Management (Frontend)**:
- Use TanStack Query for server state (NEVER duplicate in React state)
- Use React state for local UI state only
- Calculate derived values during render (don't use effects)

**Error Handling**:
- Frontend: error boundaries for rendering errors; always handle loading/error states in data fetching
- Backend: module-specific error enums with `thiserror`, implement `IntoResponse` for Axum

**Conventions**:
- Path alias `@/*` → `./src/*` (frontend TS + Vite)
- File naming: kebab-case (frontend, enforced by Biome), snake_case (Rust)
- `routeTree.gen.ts` is generated — excluded from lint/format
- `any` is allowed in TypeScript test files

## Architecture

See [Architecture Overview](./docs/architecture-overview.md), [Sandbox Architecture](./docs/sandbox-architecture.md), and [Sandbox Lifecycle](./docs/sandbox-lifecycle.md).

Backend Rust modules follow a layered structure: `models.rs` (domain types) → `dao.rs` (SQL) → `schemas.rs` (DTOs) → `handlers.rs` → `routes.rs` (Axum + utoipa OpenAPI) → `error.rs`. Frontend is feature-based: `routes/*.tsx` fetch data and compose layout, `features/*/components/` hold UI, `lib/components/` are shared/generic.
