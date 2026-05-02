# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Frontend Development Commands
```bash
# Install dependencies (requires bun 1.2.19+)
bun install

# Development (runs frontend dev server with Turborepo)
bun run dev

# Build frontend for production
bun run build

# Run frontend tests (tiered by speed and scope)
bun run test             # standard tests (excludes E2E)
bun run test:unit        # unit tests only (fastest)
bun run test:integration # integration tests only
bun run test:e2e         # E2E tests only (slowest, Playwright)
bun run test:coverage    # with coverage report

# Type checking (frontend + shared packages)
bun run type:check

# Linting and formatting
bun run biome:check      # check for issues
bun run biome:fix        # auto-fix issues

# Run all frontend checks (optimized for speed with Turborepo)
bun run check:turbo

# Install shadcn/ui components (run from frontend app)
cd apps/frontend && bunx shadcn@latest add [component]
```

### Rust Backend & Sandbox Commands
```bash
# Build
cargo build -p moatless-vibe-backend     # build backend
cargo build -p moatless-sandbox          # build sandbox

# Lint
cargo clippy -p moatless-vibe-backend -- -D warnings
cargo clippy -p moatless-sandbox -- -D warnings

# Test
cargo test -p moatless-vibe-backend      # backend tests
cargo test -p moatless-sandbox           # sandbox tests
cargo test -p claude-agent-sdk           # SDK tests
cargo test -p moatless-events            # event system tests

# E2E tests (requires `claude` CLI + API key)
CLAUDE_SDK_E2E=1 cargo test -p moatless-sandbox --test agent_e2e -- --ignored --nocapture
CLAUDE_SDK_E2E=1 cargo test -p claude-agent-sdk -- --ignored
```

### Integration Tests (Taskfile + Docker Compose)
```bash
# Requires: https://taskfile.dev, Docker

# Run DB-tier tests (API CRUD, auth, channels — no sandbox needed, fastest)
task test:compose:db

# Run sandbox-tier tests (file ops, lifecycle — builds sandbox image)
task test:compose:sandbox

# Run all tiers including agent tests (requires CLAUDE_CODE_OAUTH_TOKEN in .env)
task test:compose:all

# Run a single test file
task test:compose -- --test message_acknowledgment

# Reuse existing images (skip rebuild)
task test:compose -- --skip-build --tier agent

# Keep containers running after tests (for debugging)
task test:compose -- --keep-running --test message_acknowledgment
```

The test runner (`docker-compose/test-runner.sh`) handles the full lifecycle:
build images → start Postgres/Redis/backend → reset DB → run tests → tear down.

Test tiers:
- **db**: Non-ignored tests — task CRUD, repo CRUD, auth, channels (no sandbox)
- **sandbox**: `SANDBOX_E2E=1` — file operations, sandbox lifecycle
- **agent**: `CLAUDE_SDK_E2E=1` — agent execution, multi-message, plan mode (requires Claude CLI + API key)
- **all**: Runs db first, then all ignored tests

> **Apple Silicon**: Use `docker/build-sandbox.sh --local --only base` to build
> native arm64 sandbox images. The build script cross-compiles the sandbox binary
> and installs the Claude CLI via npm for the correct architecture.

### Docker Build Commands
```bash
# Build Docker images
./docker/build-app.sh [--tag v1.0.0] [--push]         # frontend (Nginx)
./docker/build-backend.sh [IMAGE_NAME] [TAG]           # backend (cross-compile + push)
./docker/build-sandbox.sh [--tag v1.0.0] [--push]      # sandbox images
./docker/build-servers.sh [--tag v1.0.0] [--push]      # files-server, preview-server

# Local development with Docker Compose
bun run dev:docker
```

### Kubernetes Deployment (via Taskfile)
```bash
# Requires: https://taskfile.dev
task deploy              # build, push, and deploy everything
task deploy:frontend     # build, push, and deploy frontend only
task deploy TAG=v2.0.0   # deploy with specific tag
task status              # show deployment status
task logs                # show app logs
task restart             # restart app deployment
```

### Releasing
```bash
# Check the latest release tag
git tag --sort=-creatordate | head -5

# Create and push a version tag to trigger release workflows
git tag v0.1.21
git push origin v0.1.21
```

This triggers the **release.yml** workflow which:
- Builds Rust sandbox binary
- Creates Docker images (app, sandbox, backend, files-server, preview-server)
- Packages and pushes Helm chart to GHCR
- Generates changelog using `changelogithub`

## Mandatory Workflow

### Frontend changes
- You MUST run `bun run type:check` after code changes to catch TypeScript errors
- You MUST run `bun run biome:check` after code changes for linting and formatting
- You can run both `biome:fix` and `biome:check`
- For comprehensive validation, run `bun run check:turbo` which includes all checks (optimized for speed)
- **Quick validation after edits**: Run `bun run post-edit-hook` for fast formatting fixes and type checking (~100ms with caching)

### Rust backend/sandbox changes
- You MUST run `cargo clippy -p <crate> -- -D warnings` after code changes
- You MUST run `cargo test -p <crate>` to verify tests pass

## Commit Conventions

### Commit Message Format
This project uses conventional commit format with commitlint enforcement:

```
<type>: <description>

[optional body]

[optional footer(s)]
```

**Important:** The header (first line) must be ≤100 characters due to commitlint rules.

### Commit Types
- `feat:` - New features
- `fix:` - Bug fixes
- `docs:` - Documentation changes
- `style:` - Code style changes (formatting, missing semicolons, etc.)
- `refactor:` - Code refactoring without feature changes
- `test:` - Adding or updating tests
- `chore:` - Maintenance tasks, dependency updates

### Scope Guidelines
The project enforces specific commit scopes via commitlint:
- `components` - UI component changes
- `layout` - Layout-related changes
- `routes` - Route/page-level changes
- `styles` - Styling changes
- `utils` - Utility function changes
- `hooks` - React hook changes
- Or omit the scope entirely: `feat: description` instead of `feat(scope): description`

**Note**: Commits with scopes outside this list will be rejected by commitlint.

### Examples
```bash
# Good commit messages
feat: add gRPC health check for sandbox pods
fix: resolve memory leak in task event streaming
docs: update CLAUDE.md for Rust backend
test: add integration tests for sandbox provisioning

# With scope
feat(components): add loading skeleton for task list
feat(utils): add environment service with file search
fix(hooks): handle edge case in validation hooks
```

### Pre-commit Hooks
- **pre-push**: Runs fast tests (`bun run test:fast`) before pushing
- **commit-msg**: Validates commit message format using commitlint

## Skills System

This project uses Claude Code's skills system to provide comprehensive development guidance. Skills are automatically activated based on the files you're working on or keywords in your requests.

### Available Skills

#### frontend-dev-guidelines
**Automatically activated when working on**: Frontend code (`apps/frontend/src/**/*.tsx`, `*.ts`)

**Covers**: React 19 patterns, TanStack Query/Router, Tailwind v4, shadcn/ui, component design, data fetching, routing, state management, derived state, error handling, performance optimization, TypeScript patterns, testing

**Use this skill when**:
- Creating or modifying React components
- Implementing data fetching with TanStack Query
- Working with routes and pages (TanStack Router)
- Styling components with Tailwind CSS
- Managing state or computing derived values
- Optimizing performance
- Writing frontend tests

#### backend-dev-guidelines
**Automatically activated when working on**: Backend code (`backend/src/**/*.rs`, `sandbox/src/**/*.rs`, `crates/**/*.rs`)

**Covers**: Axum HTTP + OpenAPI (utoipa), gRPC (tonic), SQLx PostgreSQL, layered architecture (routes→handlers→services→dao), Kubernetes pod management, error handling with thiserror, Rust patterns, testing

**Use this skill when**:
- Creating new API endpoints
- Implementing business logic in handlers/services
- Working with database queries (dao layer)
- Defining API schemas and OpenAPI docs
- Working with Kubernetes resource management
- Working on gRPC communication between backend and sandbox
- Writing backend or sandbox tests

#### testing-guidelines
**Automatically activated when working on**: Test files (`**/*.test.ts`, `**/*.test.tsx`, `**/__tests__/**/*`, `**/tests/*.rs`)

**Covers**: Vitest patterns (frontend), Rust test patterns (backend/sandbox), React Testing Library, strategic mocking, test tier classification, unit vs integration tests, coverage goals, test organization

**Use this skill when**:
- Writing any type of test
- Deciding what to mock
- Determining test coverage priorities
- Organizing test suites

### How Skills Work

1. **Automatic Activation**: Skills are automatically suggested when you work on files matching their patterns or use relevant keywords
2. **On-Demand Access**: Ask Claude to "use the [skill-name] skill" to access detailed guidance
3. **Contextual Help**: Skills provide detailed patterns, code examples, and anti-patterns
4. **Consistency**: Skills ensure all developers follow the same patterns

### Skill vs CLAUDE.md

- **CLAUDE.md** (this file): Commands, workflows, commit conventions, project structure, high-level principles
- **Skills**: Detailed implementation patterns, code examples, best practices, anti-patterns, comprehensive guidance

**When in doubt**: Consult the appropriate skill for detailed implementation guidance.

## Architecture

For a comprehensive overview of the system architecture, see [Architecture Overview](./docs/architecture-overview.md).
For sandbox-specific architecture, see [Sandbox Architecture](./docs/sandbox-architecture.md).

### Project Structure
```
├── apps/                          # JavaScript application packages
│   └── frontend/                  # React frontend application
│   │   ├── src/
│   │   │   ├── features/          # Feature-based modules
│   │   │   │   ├── claude-code/       # Claude Code integration
│   │   │   │   ├── environments/      # Environment management
│   │   │   │   ├── navigation/        # Navigation components
│   │   │   │   ├── preview/           # Preview functionality
│   │   │   │   ├── settings/          # Settings management
│   │   │   │   └── tasks/             # Tasks feature
│   │   │   ├── lib/               # Shared utilities and components
│   │   │   │   ├── api/               # Core API utilities (client, query-client, query-keys)
│   │   │   │   ├── components/        # Shared components (ui/, error-boundary, theme-provider)
│   │   │   │   ├── hooks/             # Shared hooks
│   │   │   │   ├── layout/            # Layout components
│   │   │   │   ├── styles/            # Global styles
│   │   │   │   └── utils/             # Utility functions
│   │   │   ├── routes/            # Page components & routing (TanStack Router)
│   │   │   └── main.tsx           # Application entry point
│   │   ├── package.json
│   │   └── vite.config.ts
├── backend/                       # Rust backend (Axum + Tonic gRPC)
│   ├── src/
│   │   ├── main.rs                # Entry point
│   │   ├── config.rs              # Configuration from env vars
│   │   ├── db.rs                  # PostgreSQL connection pool (sqlx)
│   │   ├── state.rs               # Shared application state
│   │   ├── startup.rs             # Server initialization
│   │   ├── auth/                  # OAuth, sessions, API keys
│   │   ├── sandbox/               # Kubernetes sandbox management
│   │   │   ├── k8s/              # K8s client + resource CRUD
│   │   │   ├── routes.rs         # HTTP API endpoints
│   │   │   ├── handlers.rs       # Request handlers
│   │   │   └── grpc_client.rs    # gRPC calls to sandbox pods
│   │   ├── task/                  # Task domain (models, dao, handlers, routes)
│   │   ├── repository/            # Repository domain
│   │   ├── message/               # Message handling
│   │   ├── channel/               # Slack/webhook integrations
│   │   ├── event_handler/         # Event consumption
│   │   ├── sse/                   # Server-Sent Events
│   │   ├── sandbox_grpc_server.rs # gRPC server (receives from sandbox pods)
│   │   └── metrics.rs             # Prometheus metrics
│   ├── tests/
│   └── Cargo.toml
├── sandbox/                       # Rust sandbox runtime (runs in K8s pods)
│   ├── src/
│   │   ├── main.rs                # Entry point: config → gRPC server + HTTP /health
│   │   ├── config.rs              # Config from env vars
│   │   ├── grpc_server.rs         # SandboxService gRPC (receives from backend)
│   │   ├── backend_client.rs      # gRPC client (calls backend)
│   │   ├── agent/                 # Claude CLI orchestration
│   │   │   ├── claude_code.rs     # Claude Agent SDK wrapper
│   │   │   ├── agent_state.rs     # Agent state machine
│   │   │   ├── permission_handler.rs  # Interactive prompts
│   │   │   └── consumer.rs        # Message fetching via gRPC
│   │   ├── server/                # Server process monitoring
│   │   └── files/                 # File operations
│   ├── tests/
│   └── Cargo.toml
├── crates/                        # Shared Rust crates
│   ├── events/                    # Typed event system (shared between backend & sandbox)
│   ├── sandbox-proto/             # gRPC protobuf definitions
│   ├── claude-agent-sdk/          # Rust SDK for Claude CLI subprocess
│   └── figma-clipboard/           # Figma clipboard parser
├── packages/                      # Shared TypeScript packages
│   ├── types/                     # @moatless/types - shared types
│   ├── api-client/                # API client package
│   ├── api-contract/              # API contract definitions
│   ├── events/                    # Event types (TypeScript)
│   ├── sandbox-schemas/           # Sandbox schema definitions
│   └── shell-utils/               # Shell utility functions
├── docker/                        # Docker configurations
│   ├── Dockerfile.app             # Frontend (Nginx + static files)
│   ├── Dockerfile.backend         # Backend (multi-stage cross-compile)
│   ├── Dockerfile.backend-cross   # Backend (minimal runtime, local cross-compile)
│   ├── Dockerfile.sandbox         # Base sandbox (Ubuntu 24.04 + Claude CLI)
│   ├── Dockerfile.sandbox-js      # JS/TS sandbox (+ Playwright)
│   ├── Dockerfile.sandbox-full    # Full dev sandbox (Python/Rust/Go/Java)
│   └── build-*.sh                 # Build scripts
├── docker-compose/                # Local Docker Compose deployment
│   ├── docker-compose.yml         # PostgreSQL, Redis, backend, Dozzle
│   └── dev-docker.sh              # Dev mode launcher
├── k8s/                           # Kubernetes deployment
│   └── helm/moatless-vibe/        # Helm chart
├── drizzle/                       # PostgreSQL migrations (SQL files)
├── Cargo.toml                     # Rust workspace root
├── Taskfile.yml                   # Deployment automation (taskfile.dev)
└── package.json                   # Root npm config (frontend/scripts)
```

### Technology Stack

**Frontend:**
- **Framework**: React 19 with TypeScript
- **Routing**: TanStack Router (file-based)
- **Data Fetching**: TanStack Query (React Query)
- **Styling**: Tailwind CSS v4
- **Build Tool**: Vite
- **Code Quality**: Biome
- **Testing**: Vitest + React Testing Library

**Backend (Rust):**
- **HTTP Framework**: Axum 0.8
- **Async Runtime**: Tokio
- **gRPC**: Tonic 0.12 + Prost (protobuf)
- **Database**: PostgreSQL via SQLx 0.8 (async, compile-time checked queries)
- **Kubernetes**: kube 3.0 client + k8s-openapi
- **Caching**: Moka (in-memory)
- **Auth**: JWT (jsonwebtoken), OAuth
- **Storage**: AWS S3 SDK
- **Event Streaming**: Redis Streams
- **API Docs**: utoipa (OpenAPI generation)
- **Observability**: tracing + Prometheus metrics
- **Error Handling**: thiserror

**Sandbox (Rust):**
- **Runtime**: Rust binary running inside K8s pods
- **Agent**: Claude CLI via claude-agent-sdk (NDJSON over stdin/stdout)
- **Communication**: gRPC (tonic) to/from backend
- **Health Probes**: Axum HTTP server

**Infrastructure:**
- **Monorepo**: Turborepo (frontend) + Cargo workspaces (Rust)
- **Package Manager**: bun workspaces (frontend)
- **Container Orchestration**: Kubernetes (production), Docker Compose (local)
- **Ingress**: Traefik (preview server routing)
- **Database Migrations**: SQL files in `drizzle/` (run at startup)

### Communication Architecture

```
Frontend (React)
    ↓ HTTPS (fetch API)
Nginx (static files + reverse proxy)
    ↓ HTTP
Backend (Rust/Axum, port 8080)
    ├→ PostgreSQL (sqlx async pool)
    ├→ Redis Streams (event publishing)
    ├→ Kubernetes API (pod lifecycle)
    └→ gRPC to Sandbox Pods (port 50052)

Sandbox Pods (Rust, per-task)
    ├→ Claude CLI subprocess (claude-agent-sdk)
    ├→ File/git operations
    └→ gRPC back to Backend (port 50051)
        ├→ PublishEvent (replaces Redis XADD)
        ├→ GetPendingMessages (replaces HTTP polling)
        └→ GetCredentials (with JWT auth)
```

### Sandbox Pod Structure (per task)
Each task gets its own Kubernetes Deployment with a multi-container pod:
- **sandbox container**: Rust binary + Claude CLI (ports 8001 HTTP, 50052 gRPC)
- **server containers**: Dynamic, data-driven (files-server, preview-server, etc.)
- **volumes**: workspace-data (PVC), shared-data, build-cache, tmp (memory-backed)

Kubernetes resources created per sandbox: Deployment, Service (ClusterIP), PVC, Secret, IngressRoute (Traefik CRD).

### Backend Module Pattern
Each domain module in the Rust backend follows a consistent structure:
```
{module}/
├── mod.rs          # Module declarations
├── models.rs       # Domain types (sqlx::FromRow)
├── dao.rs          # Database persistence layer (SQL queries)
├── schemas.rs      # API request/response DTOs
├── handlers.rs     # HTTP request handlers
├── routes.rs       # Axum router + OpenAPI annotations (utoipa)
└── error.rs        # Module-specific error types
```

### API Integration

All frontend API calls use TanStack Query hooks for consistent caching, error handling, and loading states. Query keys are centralized in `src/lib/api/query-keys.ts` using a hierarchical structure for efficient cache invalidation.

### Development Guidelines

**IMPORTANT: Consult the appropriate skill for detailed implementation patterns:**

| Working On... | Skill to Use | Key Topics |
|---------------|--------------|------------|
| React components, pages, UI | **frontend-dev-guidelines** | Components, hooks, data fetching, routing, state, styling, performance |
| Rust API endpoints, services, DB | **backend-dev-guidelines** | Axum routes, handlers, dao, K8s, gRPC, sqlx |
| Any type of test | **testing-guidelines** | Test strategy, mocking, tier classification, organization |

**General Development Principles:**
- **Path Aliases**: `@/*` maps to `./src/*` in TypeScript and Vite (frontend only)
- **File Naming**: Kebab-case convention enforced by Biome (frontend), snake_case for Rust
- **Import Organization**: Biome automatically organizes imports when running `bun run biome:fix`
- **Generated Files**: `routeTree.gen.ts` is excluded from linting/formatting (auto-generated by TanStack Router)
- **Feature-Based Architecture** (frontend):
  - Features contain components, hooks, and API code
  - Shared utilities in `/lib/`
  - Page logic lives in `/routes/` files

#### Component Organization (Frontend)

- **Route Files** (`/routes/*.tsx`): Handle data fetching, page-level state, layout composition
- **Feature Components** (`/features/*/components/`): Handle UI implementation, interactions, reusable logic
- **Shared Components** (`/lib/components/`): Generic, reusable UI components

#### Critical Development Rules

**Type Safety (TypeScript - NEVER violate these)**:
  - Use TypeScript strict mode
  - **NEVER use `any` type** - always use proper types or `unknown`
  - **NEVER cast to `any`** - fix the underlying type issue instead
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
  - Frontend: Use error boundaries to catch rendering errors
  - Backend: Use module-specific error enums with `thiserror`, implement `IntoResponse` for Axum
  - Always handle loading and error states in data fetching

### Skill Usage Quick Reference

| Task | Skill | Resource |
|------|-------|----------|
| React components | frontend-dev-guidelines | Component Patterns, Routing |
| Data fetching | frontend-dev-guidelines | Data Fetching |
| State management | frontend-dev-guidelines | State Management, Derived State |
| Styling | frontend-dev-guidelines | Styling |
| Rust API endpoints | backend-dev-guidelines | Routes, Handlers, OpenAPI |
| Database queries | backend-dev-guidelines | DAO Pattern, SQLx |
| K8s sandbox management | backend-dev-guidelines | Sandbox Module |
| gRPC communication | backend-dev-guidelines | sandbox-proto, tonic |
| Any testing | testing-guidelines | Testing Philosophy, Strategic Mocking |

## Testing

### Frontend Testing
This project uses Vitest for unit and integration testing, with React Testing Library for component testing. Tests are organized in `__tests__` directories co-located with source code:

```
src/features/tasks/
├── components/
│   ├── __tests__/
│   │   ├── task-viewer.test.tsx
│   │   └── task-header.test.tsx
│   └── task-viewer.tsx
├── hooks/
│   ├── __tests__/
│   │   └── use-tasks.test.ts
│   └── use-tasks.ts
```

### Backend Testing (Rust)
Backend tests use standard Rust test conventions with `cargo test`:

- **Unit Tests**: Inline `#[cfg(test)] mod tests` within source files
- **Integration Tests**: `tests/` directory at crate root
- **E2E Tests**: Gated behind environment variables (e.g., `CLAUDE_SDK_E2E=1`)

### Integration Tests (`crates/integration-tests/`)
End-to-end tests exercising the full backend API with real database and sandbox.
Run via Taskfile (`task test:compose:db`, `task test:compose:all`) — see [Integration Tests](#integration-tests-taskfile--docker-compose) above.

Test tiers:
- **db** (no `#[ignore]`): Task CRUD, repo CRUD, auth, channels, messages — backend + DB only
- **sandbox** (`#[ignore]`, `SANDBOX_E2E=1`): File operations, sandbox lifecycle — needs sandbox container
- **agent** (`#[ignore]`, `CLAUDE_SDK_E2E=1`): Agent execution, multi-message same-session, plan mode, stop/recovery — needs sandbox + Claude CLI

Test infrastructure (`crates/integration-tests/src/`):
- `test_client()`, `client_as_user()` — API clients with ForwardAuth headers
- `ensure_template_repo()` — creates or finds template repository
- `EventCollector` — SSE stream collector with `wait_for_event()`, `wait_for_nth_event()`, `count_events()`
- `sandbox_helpers::wait_for_sandbox_ready()`, `wait_for_agent_idle()` — polling helpers
- `callback_server` — in-process HTTP server for webhook tests

### Test Performance Tiers
**FAST Tests (<200ms)** - Run on every push:
- Pure unit tests with mocked dependencies
- No file system, network, or database operations

**MODERATE Tests (200ms-2s)** - Run in CI:
- Integration tests with real file system or database
- Git operation tests

**SLOW Tests (>2s)** - Run separately:
- E2E tests with real servers or Claude CLI
- Process lifecycle tests

### Testing Strategy

Key principles:
- Test behavior, not implementation details
- **Prioritize integration over isolation** for route and page components
- Mock strategically - only external dependencies
- Aim for >80% coverage on critical paths (Tier 1)
- Always test loading states, error states, and edge cases
- **Note**: `any` types are allowed in TypeScript test files
