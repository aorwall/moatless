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
- `packages/api-contract/src/preview.ts` — frontend API contract

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

## Priority Order

1. **File upload** — quick verification, likely works
2. **Server logging** — debugging existing implementation
3. **Slack channel** — end-to-end verification with real Slack workspace
4. **Git push fallback** — needs design decision + implementation
