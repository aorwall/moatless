# apps/backend Feature & Domain Inventory

This document captures the current high-level domains implemented inside `apps/backend`. It is intended to support a future move toward feature-based structuring by mapping the existing responsibilities, primary entry points, and supporting services.

## System Overview

- The backend is a [Hono](https://hono.dev/) application with OpenAPI metadata (`apps/backend/src/app.ts`).
- Runtime dependencies are composed through a central service container (`apps/backend/src/services/service-container.ts`) that wires storage, integrations, and domain services.
- API routes live under `apps/backend/src/routes`, grouped mostly by domain names already (e.g. `tasks.ts`, `repositories.ts`).
- Persistence is file-based by default with optional user/group scoping (`apps/backend/src/storage`).
- Agents and workspaces orchestrate automation around code repositories (`apps/backend/src/agents`, `apps/backend/src/workspaces`).

## Quick Domain Map

| Domain | Responsibilities | Primary API Surface | Core Services / Modules |
| --- | --- | --- | --- |
| Authentication & Access Control | OAuth flows, session cookies, group-aware storage | `routes/auth.ts` | `services/auth.service.ts`, `services/auth0.service.ts`, `services/github.service.ts`, `middleware/auth.ts` |
| Settings & Configuration | CRUD for global, Claude, GitHub, MCP, publish settings; system status | `routes/settings/*.ts` | `services/settings.service.ts` |
| Repository Management & Templates | Register repositories, clone templates, Git metadata, suggestions | `routes/repositories.ts`, `routes/templates.ts` | `services/repository.service.ts`, `config/templates.ts` |
| Task Lifecycle & Agent Orchestration | Create tasks, iterations, messaging, command execution, MCP bridge | `routes/tasks.ts`, `routes/commands.ts`, `routes/worker.ts`, `routes/events.ts` | `services/task.service.ts`, `services/agent.service.ts`, `agents/claude-code/*`, `services/message.service.ts`, `services/command.service.ts`, `services/mcp.service.ts`, `services/event.service.ts`, `services/tool.service.ts` |
| Workspace Operations | File tree, file IO, search, git diff, command execution | `routes/workspace.ts` | `services/workspace.service.ts`, `workspaces/workspace.ts`, `services/file-operations.service.ts`, `environments/*` |
| Preview Environment & Runtime Control | Start/stop dev servers, stream logs, Netlify publish | `routes/preview-environment.ts` | `services/workspace.service.ts`, `services/docker.service.ts`, `services/netlify.service.ts`, `preview server` adapters |
| Design & Asset Integrations | Import Figma nodes and assets | `routes/figma.ts` | `services/figma.service.ts`, `services/upload.service.ts`, `storage` |
| Uploads & File Storage | Binary uploads, retrieval, metadata | `routes/uploads.ts` | `services/upload.service.ts`, `storage/*` |
| Usage & Billing Insights | Claude usage summaries and per-session metrics | `routes/usage.ts` | `agents/claude-code/claude.service.ts` |
| Feature Flags | Aggregate runtime feature flags | `routes/feature-flags.ts` | `utils/feature-flags.ts` |
| Worker / Remote Agent Mode | Proxy agent execution from worker container | `routes/worker.ts`, `worker-app.ts` | `agents/proxy-agent.ts`, `utils/worker-api-client.ts` |
| Telemetry & Observability | Request logging, tracing, SSE management | `telemetry.ts`, `middleware/logger.ts`, `routes/events.ts` | `utils/logger.ts`, `utils/sse-manager.ts` |

## Domain Details

### Authentication & Access Control

- Handles GitHub and Auth0 OAuth handshakes, session creation, and cookie management (`routes/auth.ts`).
- `services/auth.service.ts` orchestrates OAuth token exchange, session persistence, and group lookup.
- `services/github.service.ts` and `services/auth0.service.ts` encapsulate provider specifics, also reused by settings and repository sync.
- Middleware in `middleware/auth.ts` validates sessions for `/api/**` routes and stores session context via `services/access-control/context-session-provider.ts`.
- The storage layer offers user- and group-scoped backends (`storage/authorized-*.ts`) that plug into the service container when `STORAGE_TYPE` is set.

### Settings & Configuration

- Sectioned routes under `routes/settings` expose CRUD for general, Claude, GitHub, MCP, and publish settings plus system status checks.
- `services/settings.service.ts` provides a typed facade over `settings.json`, environment defaults, and helper detection (e.g. Claude CLI paths).
- Downstream services (repository, claude, docker) depend on settings for directories, tokens, and environment choices.

### Repository Management & Templates

- `routes/repositories.ts` supports create/list/update/delete, branch lookup, and repository suggestions (Claude project integration). Includes OAuth-aware cloning via `getGitHubTokenFromContext`.
- `services/repository.service.ts` covers cloning (GitHub or template), Git worktree management, branch orchestration, and persistence.
- Templates (`config/templates.ts`) feed both repository creation and the `/templates` endpoint for the UI.
- Closely tied to workspace creation, docker settings, and Netlify deployment metadata.

### Task Lifecycle & Agent Orchestration

- Public API lives in `routes/tasks.ts` and `routes/commands.ts`, wiring to task creation, iterations, messaging, command execution, MCP channel, deploy, and vibe tasks.
- Core logic sits in `services/task.service.ts`: maintains task state, subscribes to events, handles message routing, screenshot capture, and workspace interactions.
- `services/agent.service.ts` spins up Claude or proxy agents, prepares workspaces, manages attachments/context, and streams events.
- Claude-specific implementations under `agents/claude-code` (service, utils, prompt management) enable usage reporting and session replay.
- `services/message.service.ts` bridges stored JSONL transcripts to UI-friendly formats and persists user/assistant messages and attachments.
- MCP integration (`services/mcp.service.ts`) exposes tools like install, preview control, and metadata updates to the Claude SDK.
- `services/command.service.ts` stores task-level commands with enablement conditions; surfaced by `/commands` routes and filtered during task runtime (`utils/command-conditions.ts`).
- Event propagation uses `services/event.service.ts` plus SSE routing in `routes/events.ts` and helpers (`utils/sse-manager.ts`, `utils/sse-stream.ts`).

### Workspace Operations

- `/workspace` routes expose file listing, read/write with diff metadata, search, git diff, tree with changes, command execution, and setup.
- `services/workspace.service.ts` owns workspace lifecycle: clones or worktrees, docker/local environment selection, setup commands, preview registry wiring, cleanup.
- Actual workspace abstraction (`workspaces/workspace.ts`) combines environment capabilities with file operations (`services/file-operations.service.ts`) and git helpers (`utils/git-utils.ts`).
- Environments under `environments/*` provide local and docker implementations, including server control and setup command execution.

### Preview Environment & Runtime Control

- `routes/preview-environment.ts` lets clients start/stop/restart dev servers per task and stream logs via SSE.
- Relies on workspace instances to control environment processes, the preview server registry from `@moatless/preview-server`, and docker service for containerized repos.
- Publishing support hooks into Netlify via `services/netlify.service.ts`, often triggered from MCP tools or task actions.
- Screenshot capture for preview validation lives in `services/screenshot.service.ts` and ties into task iterations and event emission.

### Design & Asset Integrations

- Figma routes provide import by node URL, clipboard payloads, PAT management, and token exchange, backed by `services/figma.service.ts`.
- Imported nodes persist through the storage backend and can include prompt YAML generation (`config/system-prompts.ts`, Figma extractors).
- Upload service integrates to store accompanying images and shareable file IDs.

### Uploads & File Storage

- `routes/uploads.ts` handles multipart uploads and retrieval, delegating to `services/upload.service.ts` for filesystem storage plus metadata.
- Storage abstractions in `storage/*` allow switching between plain filesystem and scoped storage implementations.
- Shared by Figma imports, task attachments, and screenshot persistence.

### Usage & Billing Insights

- `/usage` routes aggregate Claude usage data with plan-aware limits and per-session stats.
- Implemented in `agents/claude-code/claude.service.ts` which scans local data caches, pricing tables, and repository-scoped histories.
- Integrates with settings for CLAUDE paths and repository context.

### Feature Flags

- `/feature-flags` exposes runtime flag values assembled by `utils/feature-flags.ts`, combining environment variables and defaults.
- Flags are used across routes/services to gate behavior (e.g. experimental features, preview toggles).

### Worker / Remote Agent Mode

- Worker entry points (`routes/worker.ts`) accept events emitted by remote agents to merge back into main event stream.
- `worker-app.ts` starts a reduced backend inside worker containers, bridging to the main API via `utils/worker-api-client.ts`.
- `agents/proxy-agent.ts` forwards agent requests to worker endpoints when the main runtime is configured for remote execution.

### Telemetry & Observability

- Request/response logging via `middleware/logger.ts`, request ID injection (`middleware/request-id.ts`), and error handling (`middleware/error-handler.ts`).
- Optional Azure Application Insights tracing through `telemetry.ts` with OpenTelemetry + Pino instrumentation.
- SSE connection metrics/materialized by `utils/sse-manager.ts` and exposed via `/events/stream/status`.

## Supporting Infrastructure & Cross-Cutting Modules

- **Middleware**: Auth, session context, logger, request ID, and error handling under `apps/backend/src/middleware`.
- **Utilities**: Shared helpers for git, filesystem, prompts, feature flags, SSE streaming, worker client, etc. live in `apps/backend/src/utils`.
- **Adapters**: `adapters/preview-adapter.ts` converts internal previews to registry formats for worker/preview server sync.
- **Scripts & Test Utilities**: Under `__tests__`, `test-utils`, and `scripts` supporting automated validation.
- **Telemetric & Config**: `telemetry.ts`, `config/system-prompts.ts`, and environment configuration in `environments/` supply contextual data.

## Observations for Feature-Based Restructuring

- Route files already reflect domain groupings; services mirror those groupings but live in a flat `services` directory. Splitting services alongside route/domain directories would reduce cross-package imports.
- `TaskService` and `WorkspaceService` act as central orchestrators with many dependencies; they may be candidates to anchor dedicated feature modules that encapsulate agent, command, message, and workspace subdomains.
- Shared infrastructure (storage, middleware, utilities) should remain in a `shared`/`platform` area referenced by feature modules.
- Worker mode shares substantial code with task/agent domains; consider colocating worker-facing adapters within the future agent/automation module.

## Proposed Feature-Based Architecture

### Guiding Principles

- **Vertical slices first**: group HTTP routes, orchestrators, background jobs, and adapters for a given capability in one place.
- **Keep coupling explicit**: cross-feature calls flow through well-defined facades or published interfaces instead of deep imports.
- **Layer within a feature**: keep domain logic (entities, value objects) separate from application orchestration (use cases) and infrastructure (storage adapters, HTTP clients).
- **Shared platform**: extract middleware, logging, storage primitives, and third-party SDK helpers into a `platform` (or `shared`) area consumed by features.
- **Progressive migration**: move one feature at a time, wrapping legacy services with thin facades to avoid a Big Bang rewrite.

### Suggested Top-Level Layout

```
apps/backend/src/
  app.ts
  platform/
    http/
      middleware/
      routing/
    persistence/
    messaging/
    telemetry/
    auth/
    utils/
  features/
    auth/
      api/
        routes.ts
        dto/
      application/
        login.handler.ts
        session.service.ts
      domain/
        session.ts
        user.ts
      infrastructure/
        github-oauth.client.ts
        auth0-oauth.client.ts
    settings/
      ...
    repositories/
      ...
    tasks/
      api/
        tasks.routes.ts
        commands.routes.ts
        events.routes.ts
      application/
        create-task.use-case.ts
        task-service.facade.ts
        mcp/
      domain/
        task.ts
        iteration.ts
      infrastructure/
        storage/
        claude/
    workspace/
      ...
    preview/
      ...
    design/
      ...
    uploads/
      ...
    usage/
      ...
    feature-flags/
      ...
    worker/
      ...
```

- `platform/` (or `shared/`) retains generic adapters and middleware (logger, request ID, SSE manager, storage factories, telemetry bootstrap).
- Each feature owns its dependency wiring (e.g. `features/tasks/application/task-service.facade.ts`) and exposes a `registerRoutes` function imported by `app.ts`.
- Cross-feature dependencies should rely on interfaces living in the callee feature (e.g. `features/workspace` exports `WorkspaceFacade`), implemented inside `infrastructure/`.

### Feature Bundles

| Feature slice | Included assets | Notes |
| --- | --- | --- |
| `auth` | OAuth flows, session cookies, access control middleware | Produces `AuthFacade` used by other slices (e.g. repositories needing GitHub token). |
| `settings` | Settings CRUD, system status checks | Publishes `SettingsProvider` consumed by repositories, docker, Claude. |
| `repositories` | Repo CRUD, template management, Git insights | Supplies `RepositoryRegistry` used by workspace & tasks. |
| `tasks` | Task lifecycle, agent orchestration, commands, events, MCP bridge | Largest slice; splits into submodules (`agents`, `commands`, `messaging`, `screenshots`). |
| `workspace` | File system operations, search, git diff, worktree management | Exposes `WorkspaceFacade` implemented on top of environments and file operations. |
| `preview` | Dev server lifecycle, deployment, logs, screenshots | Depends on `workspace` for repo context. |
| `design` | Figma import, assets | Uses `uploads` for storage; may share YAML prompts via `platform/prompts`. |
| `uploads` | Binary storage, metadata | Provides generic file storage service. |
| `usage` | Claude usage metrics and billing insights | Consumes `settings` and `tasks` for context. |
| `feature-flags` | Runtime flag registry | Supplies read-only accessor consumed by UI routes. |
| `worker` | Worker mode HTTP endpoints, proxy agent integration | Depends on `tasks` and `workspace` interfaces. |
| `observability` (optional) | Telemetry, logger, SSE instrumentation | Could remain under `platform` if shared by all features. |

### Dependency Direction

```
platform --> features/*
features (consumer) --> feature (provider) via interface
```

- Prefer `features/tasks` depending on `features/workspace` interfaces rather than importing their internals.
- Shared data contracts (`@moatless/types`) remain external packages; avoid duplicating them inside features.

### Migration Approach

1. **Introduce platform layer**: move current `middleware`, `utils/logger`, `storage` factories into `platform/` without behavioral changes.
2. **Carve out a pioneer feature**: e.g. `feature-flags` or `uploads`—small scope to validate folder conventions and dependency injection.
3. **Wrap legacy services**: create facades (`TaskFacade`) inside the new feature directory that proxy to existing services while code migrates.
4. **Relocate routes + tests**: move route registration and tests into corresponding feature directories, adjusting imports.
5. **Gradually internalize services**: refactor `services/task.service.ts` into dedicated `application` & `domain` layers, splitting sub-responsibilities where helpful.
6. **Finalize by pruning old directories**: once all slices migrate, delete top-level `services`, `routes`, `agents`, etc., leaving only `features` + `platform`.

### Additional Considerations

- **Eventing / messaging**: consider exposing a small event bus interface in `platform/messaging` consumed by `tasks`, `workspace`, and `preview` slices.
- **Configuration registry**: `service-container.ts` can evolve into a lightweight dependency registry that composes feature providers instead of manually wiring every service.
- **Testing**: colocate feature-specific tests within `features/<name>/__tests__` to keep domain boundaries clear.
- **Documentation**: maintain README per feature outlining public interfaces, external dependencies, and migration status.

This structure keeps features self-contained, simplifies onboarding, and surfaces explicit contracts between domains—making future reorganization or extraction considerably easier.
