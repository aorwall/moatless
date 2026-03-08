# Verify & Fix Checklist

Tracking document for verifying and fixing key platform capabilities.

---

## 1. File/Image Upload to Agent

**Status:** Needs verification

### Current Architecture

The upload flow is fully implemented end-to-end:

```
Frontend (file input / paste)
  → POST /api/v1/uploads (multipart form)
  → Backend stores in S3 (or local/Azure)
  → Metadata saved to `file_metadata` table
  → Message enricher embeds file reference as hidden content block
  → Agent receives: "Attached file: {name} ({type}, {size})\nAvailable at: /api/v1/uploads/{file_id}"
```

**Key files:**
- `apps/frontend/src/features/uploads/hooks/use-upload.ts` — TanStack Query mutation
- `apps/frontend/src/features/uploads/hooks/use-image-paste.ts` — paste-to-upload
- `apps/frontend/src/features/tasks/hooks/use-task-inputs.ts` — consolidated input manager
- `backend/src/file_storage/` — handlers, DAO, S3 ops, models
- `backend/src/message/enricher.rs` — builds content blocks with file references
- `backend/src/message/send.rs` — send_message_flow() accepts `file_ids`

### What to Verify

- [ ] Upload UI works (file picker + image paste)
- [ ] `POST /api/v1/uploads` returns `{ file_id, filename, mime_type, size }`
- [ ] File accessible via `GET /api/v1/uploads/{file_id}`
- [ ] Message enricher correctly creates hidden content block with file URL
- [ ] Agent can actually fetch the file from the upload URL (network path from sandbox pod → backend)
- [ ] Multiple file types work: images (png, jpg), text, PDF
- [ ] File preview renders in `file-attachments.tsx`

### Known Gaps

1. **No file validation** — no size limits or type restrictions at the API level
2. **No cleanup** — orphaned files not removed when messages are deleted
3. **No expiration** — files stored indefinitely (consider S3 lifecycle policy)
4. **Agent access path** — files are fetched via HTTP URL, not pre-mounted; verify the sandbox pod can reach the backend API

---

## 2. Git Push Fallback from Sandbox

**Status:** Not implemented — needs design + build

### Current State

Git **clone** is fully functional with multi-provider auth (GitHub, GitLab, Gitness). Git **diff/status** works for UI display. But there is **no git push capability** from the sandbox.

**Credential flow (existing):**
```
Sandbox init
  → gRPC GetCredentials(task_id) to backend
  → Backend resolves: env vars (moatless-secrets) → DB fallback (provider_tokens)
  → Returns: GITHUB_TOKEN, GITLAB_TOKEN, GITNESS_TOKEN, etc.
  → Sandbox sets as env vars + configures git credential helper
```

**Git auth setup (`sandbox/src/init/git.rs`):**
- GitHub: `gh` CLI credential management + token-based HTTPS fallback
- GitLab/Gitness: Token-based HTTPS via credential helper
- Token injection: `https://x-access-token:{token}@{host}/{path}`

### What's Missing

1. **No git push/commit commands** in `sandbox/src/files/git.rs` (only clone + diff)
2. **No `GitPush` gRPC method** in sandbox.proto
3. **No push handler** in the backend
4. **No retry/fallback** for push failures

### Proposed Approach

**Option A: Agent-driven push (simplest)**
The Claude agent already has Bash access in the sandbox. It can run `git add`, `git commit`, `git push` directly since credentials are configured during init. The "fallback" would be:
- Verify credentials persist for the sandbox lifetime (not just init)
- Add a dedicated command/tool the agent can invoke
- Expose a UI button that triggers the agent to push

**Option B: Backend-orchestrated push**
Add a gRPC method + backend handler:
```
Sandbox → gRPC CommitAndPush(task_id, branch, message) → Backend
  → Backend calls sandbox gRPC to execute git commands
  → Or: Backend does the push itself via provider API (GitHub/GitLab)
```

**Option C: Provider API push (no git needed)**
Use GitHub/GitLab/Gitness APIs to create commits directly from file diffs, bypassing git CLI entirely. More reliable but more complex.

### Key Files

- `sandbox/src/init/git.rs` — clone + auth setup
- `sandbox/src/init/credentials.rs` — credential fetching with retry
- `sandbox/src/files/git.rs` — git diff/status (read-only)
- `backend/src/sandbox/grpc_server.rs` — GetCredentials handler
- `crates/sandbox-proto/proto/sandbox/v1/sandbox.proto` — gRPC protocol

### Action Items

- [ ] Decide on approach (A/B/C)
- [ ] Verify agent can run `git push` from sandbox with existing credentials
- [ ] If agent push works, add a UI "Push to Git" button that instructs the agent
- [ ] Add error handling for common push failures (auth, conflicts, permissions)
- [ ] Test with all three providers: GitHub, GitLab, Gitness

---

## 3. Server Logging from Containers

**Status:** Implemented but may not be working — needs debugging

### Current Architecture

```
Server containers (stdout/stderr)
  → K8s log API (pods/log)
  → Backend streams via pod_reader.rs
  → SSE endpoint: GET /api/sandbox/v1/tasks/{task_id}/servers/{server_name}/logs
  → Frontend renders real-time

Pod watcher (leader-elected background task)
  → Watches pod state transitions
  → Publishes ServerEventPayload to Redis Streams
  → SSE → Frontend status updates
```

**Two parallel systems:**
1. **Log streaming** — live K8s log streaming with previous-container fallback
2. **Status monitoring** — pod watcher detects container state transitions, publishes events

**Key files:**
- `backend/src/sandbox/k8s/pod_reader.rs` — `stream_container_logs_k8s()` (live + fallback)
- `backend/src/sandbox/k8s/pod_watcher.rs` — leader-elected pod watcher, server transition detection
- `backend/src/sandbox/handlers.rs` — `server_logs_handler()` SSE endpoint
- `backend/src/sandbox/models.rs` — `ContainerObservedStatus`, status mapping
- `backend/src/sandbox/provider.rs` — `SandboxProvider` trait (K8s vs Docker)
- `crates/events/src/lib.rs` — `ServerSubType`, `ServerEventPayload`
- `apps/frontend/src/features/preview/api/events.ts` — frontend preview API

### What to Verify

- [ ] SSE endpoint returns log lines for running server containers
- [ ] Log fallback works for terminated containers (`previous: true`)
- [ ] Pod watcher correctly detects server container transitions (Starting → Started → Failed)
- [ ] Server status events reach the frontend via SSE
- [ ] Container name mapping is correct: `{server_name}-server` convention
- [ ] RBAC: backend service account has `pods/log` get+list permissions
- [ ] Docker mode: verify graceful "not available" message (agent-only mode)

### Known Limitations

1. **Docker mode** — server logs not available (returns error). Docker runs everything in one container.
2. **Log retention** — only available while container runs or for one restart cycle
3. **No centralized logging** — no integration with Loki/ELK/Datadog
4. **Late subscribers** — miss historical lines beyond the 500-line tail
5. **Container restarts** — only most recent logs available after restart

### Debugging Steps

If logs aren't working:
1. Check pod watcher is running (leader election): look for "pod watcher started" in backend logs
2. Verify server containers are actually running: `kubectl get pod -l moatless.io/task-id={id} -o jsonpath='{.status.containerStatuses}'`
3. Test K8s log API directly: `kubectl logs {pod} -c {server_name}-server`
4. Check RBAC: verify `pods/log` permission in Helm RBAC role
5. Check SSE connection in browser DevTools Network tab
6. Verify container naming matches: handler appends `-server` suffix

---

## 4. Slack Channel Integration

**Status:** Fully implemented — needs end-to-end verification

### Current Architecture

```
Slack Events API → POST /api/v1/channels/slack/webhook
  → HMAC-SHA256 signature verification
  → Event dedup (5-min Moka cache)
  → SlackProvider::parse_event() → InboundMessage
  → router::route_message() (ongoing/per_message/thread_based)
  → process_inbound() — create/route task, provision sandbox, send message
  → Agent processes → event_handler → outbound.rs
  → SlackProvider::send_reply() (thread reply + emoji reactions)
```

**Key files:**
- `backend/src/channel/` — full module (models, router, inbound, outbound, handlers, DAOs)
- `backend/src/channel/slack/` — provider, api, signature, format
- `backend/src/event_handler/consumer.rs` — triggers outbound notifications

### Configuration Required

| Env Var | Purpose |
|---------|---------|
| `SLACK_BOT_TOKEN` | `xoxb-...` token for Web API calls |
| `SLACK_SIGNING_SECRET` | Webhook signature verification |
| `SLACK_BOT_USER_ID` | Filter own messages from context |

### What to Verify

- [ ] Webhook endpoint accepts Slack events (url_verification + event_callback)
- [ ] Signature verification works (HMAC-SHA256)
- [ ] Event deduplication prevents retries from creating duplicate tasks
- [ ] `app_mention` trigger works (default mode)
- [ ] User resolution: Slack user → email → system user ID
- [ ] Routing modes work:
  - [ ] `ongoing` — all messages → single task, auto-creates on close
  - [ ] `per_message` — each message → new task
  - [ ] `thread_based` — new messages → new task, replies → existing task
- [ ] Thread context enrichment (previous messages prepended to agent input)
- [ ] Outbound replies post in thread with task URL
- [ ] Emoji reactions: 👀 processing → ✅ completed / ❌ error
- [ ] Bot message filtering (doesn't respond to itself)
- [ ] Prompt templates with `{message}` placeholder substitution
- [ ] Config CRUD endpoints work (create/list/update/delete channel configs)

### Slack App Setup Checklist

- [ ] Slack app created with Events API enabled
- [ ] Bot scopes: `chat:write`, `reactions:write`, `reactions:read`, `users:read`, `users:read.email`, `channels:history`, `groups:history`
- [ ] Event subscriptions: `app_mention`, optionally `message.channels`
- [ ] Request URL pointing to: `https://{domain}/api/v1/channels/slack/webhook`
- [ ] Bot installed to workspace

### DB Tables

- `channel_configs` — provider + source routing rules (unique: provider + source_id)
- `channel_threads` — thread → task mapping (unique: config_id + external_thread_id)

### Error Handling

The system uses graceful degradation:
- Signature failure → 401, event rejected
- User not found → Slack reply with "sorry" message, event skipped
- Sandbox provision failure → logged warning, task created but no sandbox
- Send failure → Error status set in Slack

---

## 5. Missing Backend API Endpoints (Frontend Placeholder Types)

**Status:** Tracked — types are hardcoded because the backend doesn't implement the endpoints yet

After removing `packages/types`, the frontend has ~30 manually-written types across 9 files in `apps/frontend/src/lib/types/`. Some are intentional (UI constants, type guards), but others exist because the backend doesn't have the corresponding API endpoints yet. If the backend adds these endpoints, the local types must be deleted and replaced with generated ones.

### 5a. Settings providers (`apps/frontend/src/lib/types/settings.ts`)

GitLab, Claude, and Figma configuration types are defined locally because the backend doesn't implement these providers yet:

- **GitLab** — `GitLabConfig`, PAT types. Hooks in `use-gitlab-connection.ts`, API stubs return `Promise.resolve()`
- **Claude** — settings, OAuth, PAT types. Hooks in `use-claude-connection.ts`, returns `{ connected: false }`
- **Figma** — OAuth, PAT types. Hooks in `use-figma.ts`, all mocked

**Action:** When implementing each provider in the Rust backend, add OpenAPI schemas → regenerate api-client → delete local types.

### 5b. Plugin management (`apps/frontend/src/lib/types/plugins.ts`)

- `PluginInfo`, `PluginMarketplace`, `AddMarketplaceRequest`, etc.
- Hooks in `use-plugins.ts`, API in `plugins.api.ts` — all return empty mocks

**Action:** When backend implements plugin management, add schemas → regenerate → delete.

### 5c. User input responses (`apps/frontend/src/lib/types/user-input.ts`)

- `AskUserQuestionResponse { answers: Record<string, string> }`
- `ExitPlanModeResponse { approved: boolean; feedback?: string }`
- `UserInputResponseRequest { toolName, response, toolUseId? }`
- Used in `ask-user-question-card.tsx`, `plan-approval-card.tsx`, `use-tasks.ts`
- The response is cast to `unknown` in `tasks.ts` API before sending

**Action:** Add response schemas to `backend/src/message/schemas.rs` → register in OpenAPI → regenerate → replace local types.

---

## 6. Type Safety Gaps (Silent Drift Risk)

**Status:** Tracked — types that compile fine but can silently break at runtime if the backend changes

### 6a. Content type guards (`apps/frontend/src/lib/types/content-guards.ts`)

The guards check `content.type === 'tool_use'` etc. manually. They derive from the generated `ContentBlock` via `Extract<>`, so **variant type aliases track the api-client**. But:

- If backend **renames a field** (e.g. `tool_use_id` → `toolUseId`), the `Extract<>` alias updates but the runtime guard still checks the old field name → silent failure
- If backend **adds a variant** (already happened: `thinking`, `server_tool_use`, `web_search_tool_result` exist but have no guards) → code using `hasTools()` won't find tools in new variants
- If backend **removes a variant** → `Extract<>` becomes `never`, TypeScript catches this one

**Risk:** Medium. The `Extract<>` aliases give some safety. The runtime checks are the weak link.

**Fix:** Generate type guards from the OpenAPI spec, or add exhaustive switch tests that fail when a new variant appears.

### 6b. Event payload access (multiple components)

`StreamEvent.payload` is `unknown` in the api-client. Frontend extends it with `AgentEventPayload` in `events.ts`. Components then do:

```typescript
const agentEvent = event as AgentEvent  // unsafe cast
payload?.type === 'assistant'           // accesses .type without validation
payload.message?.content                // nested access without guard
```

Files with unsafe casts:
- `task-card.tsx` (lines 57-65) — `payload?.type` access
- `minified-event-item.tsx` (lines 134-147) — `event as AgentEvent` cast
- `minified-last-events-display.tsx` (lines 25-30) — `payload?.type` access
- `event-handlers.ts` (line 163) — `payload as { message?: ... }` cast

**Risk:** High. If `AgentEventPayload` shape changes (field renames, nesting changes), these components get `undefined` at runtime with no compile error.

**Fix:** Either:
1. Make `StreamEvent` generic on payload type in the backend (best)
2. Add runtime validation functions that check payload shape before accessing
3. At minimum, add integration tests that assert payload field access works

### 6c. Sandbox Zod schemas (`packages/sandbox-schemas/src/agent.schemas.ts`)

These are **runtime validation schemas** used in the sandbox API contract (the HTTP API the Rust sandbox exposes on port 8001). The `AgentRunRequestSchema` validates what the backend sends to the sandbox when starting an agent:

```
Backend → POST /agent/execute → Sandbox (port 8001)
  → Body validated by AgentRunRequestSchema (Zod)
  → content: UserContentSchema[] (text | tool_result | image)
```

The `UserContentSchema` was inlined from the deleted `@moatless/types`. It mirrors `ContentBlock` from the Claude SDK but only the 3 user-facing variants (text, tool_result, image).

**Risk:** Low — this validates the backend→sandbox direction, and the Rust sandbox already has its own typed deserialization via `claude-agent-sdk`. The Zod schema is a defense layer, not the primary type system. But if the SDK adds a new user content type, this schema would reject it.

**Used by:** `packages/sandbox-schemas/src/agent.schemas.ts` (consumed by sandbox HTTP handler)

### 6d. Slack channel mappings (`apps/frontend/src/lib/types/slack-channel-mappings.ts`)

Legacy types for the old Node.js API (`/api/v1/channels/slack/mappings`). These are not in the Rust backend's OpenAPI spec. The Rust backend has `ChannelConfigResponse` which is a different shape entirely.

**Risk:** Medium — live code, but the Node.js API is being phased out.

**Fix:** Migrate the frontend to use the Rust backend's channel config API → use generated types.

---

## 7. Replace Local/Test User with Password Auth + Seed User

**Status:** Not implemented — needs design + build

### Current State

The backend has two hardcoded dev/test user mechanisms that bypass real authentication:

- **`local_user.rs`** — `ensure_local_user()` auto-creates a `"local-user"` (Admin, provider `"none"`) on startup when no auth provider is configured
- **`test_user.rs`** — `ensure_test_user()` auto-creates `"test-api-user"` (Admin) when `TEST_API_KEY` env var is set

In dev mode (`AUTH_PROVIDER` unset), the auth middleware **skips all validation entirely**, and the extractors fall back to `"local-user"` identity with no credentials required. This means any request is implicitly authenticated as an admin.

### What Needs to Change

1. **Remove `local_user.rs` and `test_user.rs`** — delete the modules and their registration in `auth/mod.rs`
2. **Add password-based login** — hash with argon2 or bcrypt, store password hash in `users` table (new column)
3. **Add `POST /api/v1/auth/login` endpoint** — accepts username + password, returns session JWT (same session flow as OAuth)
4. **Seed default user** — migration or startup function creates a default admin user with configurable credentials (env vars: `DEFAULT_USER_EMAIL`, `DEFAULT_USER_PASSWORD`)
5. **Always validate in middleware** — remove the dev-mode bypass (`config.auth.provider.is_none()` → skip). Middleware must check session/API key on every request regardless of mode
6. **Remove `"local-user"` fallback from extractors** — `extractors.rs` must return 401 when no valid identity exists, not silently default to an admin user

### Key Files

- `backend/src/auth/local_user.rs` — to be removed
- `backend/src/auth/test_user.rs` — to be removed
- `backend/src/auth/mod.rs` — remove `pub mod local_user` and `pub mod test_user`
- `backend/src/auth/middleware.rs` — remove dev-mode bypass (line ~95)
- `backend/src/extractors.rs` — remove `"local-user"` fallback in dev mode
- `backend/src/auth/routes.rs` — add `POST /api/v1/auth/login`
- `backend/src/auth/schemas.rs` — add `LoginRequest { email, password }`
- `backend/src/auth/models.rs` — add password hash field to user model
- `backend/src/main.rs` — replace `ensure_local_user()`/`ensure_test_user()` with seed user

### Action Items

- [ ] Add `password_hash` column to `users` table (migration)
- [ ] Implement password hashing (argon2 or bcrypt crate)
- [ ] Add `POST /api/v1/auth/login` handler
- [ ] Add seed user on startup (configurable via env vars)
- [ ] Remove `local_user.rs`, `test_user.rs`, and dev-mode bypass
- [ ] Remove `"local-user"` fallback from extractors
- [ ] Update frontend login page to support username/password (alongside OAuth)
- [ ] Ensure `TEST_API_KEY` flow still works via the standard API key mechanism (no special user needed)

---

## 8. Shared API Schemas Between CLI and Backend

**Status:** Not implemented — needs design + build

### Current State

The CLI crate (`crates/cli/`) constructs all API requests as untyped `serde_json::json!()` calls with manually-written field names. There is **zero compile-time type safety** — if the backend renames a field (e.g. `gitUrl` → `git_url`), the CLI silently sends the wrong data.

Meanwhile, the backend has well-typed schemas in each module's `schemas.rs` (12 modules with request/response DTOs). The TypeScript frontend gets auto-generated types from the OpenAPI spec via orval. Only the Rust CLI is left without type safety.

**Examples of untyped CLI code (`crates/cli/src/commands/repos.rs`):**
```rust
let body = serde_json::json!({
    "name": name,
    "gitUrl": git_url,
    "defaultBranch": branch,
    "gitProvider": provider,
});
```

This mirrors `backend/src/repository/schemas.rs::CreateRepositoryRequest` but isn't type-checked.

### Proposed Approach

**Option A: Shared Rust crate (recommended)**

Create `crates/api-schemas/` following the existing `crates/events/` pattern:
- Move API request/response types from `backend/src/*/schemas.rs` into the shared crate
- Backend depends on it with a `utoipa` feature flag (for `ToSchema` derive)
- CLI depends on it for typed requests/responses (no OpenAPI dependency)
- Single source of truth — field renames break at compile time in both backend and CLI

**Option B: Generate Rust client from OpenAPI**

Similar to the TypeScript approach:
- Backend dumps OpenAPI spec (`cargo run -- --dump-openapi`)
- Use a Rust OpenAPI generator (e.g. `progenitor`, `openapi-generator`) to create a typed client crate
- CLI depends on the generated client

This is more complex to set up but mirrors the frontend workflow exactly.

### Key Files

- `crates/cli/src/commands/*.rs` — all command files use untyped JSON
- `backend/src/*/schemas.rs` — 12 modules with typed DTOs
- `crates/events/src/lib.rs` — existing shared crate pattern to follow
- `packages/api-client/orval.config.ts` — TypeScript generation config (for reference)

### Action Items

- [ ] Decide on approach (A: shared crate vs B: generated client)
- [ ] Create `crates/api-schemas/` with workspace member registration
- [ ] Extract API types from backend schemas into shared crate
- [ ] Add `utoipa` feature flag for backend-only OpenAPI derives
- [ ] Update backend to import schemas from shared crate
- [ ] Update CLI to use typed requests/responses instead of `serde_json::json!()`
- [ ] Ensure `serde(rename_all = "camelCase")` is consistent across shared types

---

## Priority Order

1. **File upload** — quick verification, likely works
2. **Server logging** — debugging existing implementation
3. **Slack channel** — end-to-end verification with real Slack workspace
4. **Git push fallback** — needs design decision + implementation
5. **Missing backend endpoints** — track as backend features are built (5a/5b/5c)
6. **Type safety gaps** — address 6b (high risk) first, then 6a/6c/6d
7. **Auth cleanup** — replace local/test user with password login + seed user
8. **Shared API schemas** — create shared crate for CLI ↔ backend type safety
