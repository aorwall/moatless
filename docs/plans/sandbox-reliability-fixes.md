# Sandbox Reliability Fixes — Post-Release Retrospective Plan

**Origin:** Retrospective of the sandbox-lifecycle release train (0.1.157 → 0.1.165,
2026-07-12). The issues below were observed first-hand during the release: by the
dev sub-tasks that authored the PRs (mined from their task message histories), by
the verification tasks run against each deploy, and by the release orchestration
itself. Each section documents current behavior (with code references), root
cause, and a concrete fix plan.

**Evidence base:** task histories of `799a6fb9`, `583c4ad4`, `783e718c`,
`e058f66c` (OOM/toolchain/idle-reaper), `df1a1f5d`/`c60e6a35` (init failures),
`00bef0bb` (killed background processes), `b4c40988`/`c6af6c06` (codex
sign-in), plus the cd-ecr version race during the 0.1.163/0.1.164 deploys.

**Priority order:**

| # | Issue | Priority | Size | Type |
|---|-------|----------|------|------|
| 1 | Idle-reaper kills sandboxes running long builds | P1 | M | backend + sandbox |
| 2 | OOM recycles during Rust builds + toolchain loss | P1 | M | config + pod spec |
| 3 | cd-ecr version race on close merges | P1 | S | CI workflow |
| 4 | Messages to closed tasks silently black-holed | P2 | S | backend |
| 5 | Dead Codex credentials shown as "Connected" | P2 | M | backend + frontend |
| 6 | Duplicate `initializing` event bursts | P3 | S | backend |

---

## 1. Idle-reaper kills sandboxes running long builds (P1)

### Observed

Task `583c4ad4` backgrounded a `cargo clippy` run and idle-waited; the reaper
saw an idle agent, scaled the Deployment to 0, and killed the compile. Tasks
`799a6fb9`, `783e718c`, `e058f66c` hit variations of the same problem across
multi-minute builds.

### Current behavior

- Decision: `decide_reap()` in `backend/src/task/reaper.rs:585-602` — stop when
  `reap_floor < now - ttl_minutes` (default 15 min via
  `SANDBOX_IDLE_TIMEOUT_MINUTES`, `config.rs:906`).
- `compute_reap_floor()` (`reaper.rs:564-577`) folds: latest agent lifecycle
  event (`started` pins floor to now), latest agent activity
  (`MAX(created_at)` over `type='agent'` events), and message/creation-time
  fallbacks.
- Activity events are only published for `SdkMessage::Assistant` /
  `SdkMessage::User`. **`SdkMessage::ToolProgress` (streamed output of a
  long-running tool like cargo) is silently dropped** in the wildcard arm at
  `sandbox/src/agent/handle/claude.rs:169` — the activity floor freezes for
  the entire duration of a long tool call.

### Two distinct failure modes

**(a) Mid-turn:** agent is inside a long Bash tool call. The `agent.started`
lifecycle event normally pins the floor, but if the lifecycle floor is
stale-terminal (event delivery lag, watcher restart), the frozen activity
floor lets the reaper fire mid-build.

**(b) Agent idle, sandbox busy:** the agent finishes its turn (lifecycle =
`completed`) while a background process it spawned (e.g. `cargo build`
launched with `run_in_background`, or `nohup`) is still running. Agent status
is genuinely `idle`; nothing in the system models "sandbox has live work."
This is the mode that killed `583c4ad4`'s build.

### Fix plan

**1a. Publish tool progress as activity (fixes mode a; smallest change).**
Add an arm for `SdkMessage::ToolProgress` in
`sandbox/src/agent/handle/claude.rs` that publishes an agent activity event.
Rate-limit to one event per ~60s per task (a long build streams constantly;
we only need the floor to advance). No proto or reaper changes.

**1b. Reaper double-checks live agent status before stopping (safety net).**
`agent_is_terminal()` (`reaper.rs:478-493`) already calls
`client.get_agent_status()` for orphaned-turn candidates. Call it for every
idle-stop candidate too: if gRPC returns `running`, skip the stop regardless
of the activity floor. Cost: one gRPC call per reap candidate per cycle
(rare), against a sandbox that is by definition still up.

**1c. Process-aware busy signal (fixes mode b).**
Extend `GetAgentStatusResponse` (`crates/sandbox-proto/sandbox.proto:328-334`)
with `busy_processes: bool`. The sandbox computes it by walking the live
children of the agent CLI process (`/proc/<pid>/task/*/children`,
recursively): any live child process ⇒ busy. Background bash jobs spawned by
the agent are children of the CLI process tree, so this catches
`run_in_background` builds with no agent-side cooperation. The 1b reaper
check then treats `status == running || busy_processes` as "keep".
Guardrail: cap the busy extension at e.g. 4h (`RECONCILE`-style env flag,
default on) so a leaked daemon can't pin a sandbox forever.

**Rollout:** 1a and 1b are independent and safe to ship immediately. 1c needs
a proto bump (backward compatible — new field) and a sandbox release; gate the
reaper's use of `busy_processes` on sandbox image version being new enough
(absent field = false = current behavior).

---

## 2. OOM recycles during Rust builds + toolchain loss (P1)

### Observed

Four dev sub-tasks were OOM-recycled repeatedly while compiling the Rust
workspace concurrently (worst case: *"Recovery abandoned after 3 attempts"*).
Every pod replacement wiped the installed toolchain, costing a full
rustup + protoc reinstall (some tasks did this 3×). Workspace changes on the
PVC survived every single time.

### Current behavior

- Sandbox container memory: request 2Gi / **limit 8Gi hard cap**
  (`pod_builder.rs:730-751`, `config.rs:888-891`). A parallel cargo build on
  this workspace exceeds 8Gi easily → cgroup SIGKILL (exit 137, OOMKilled).
- Per-repo overrides already exist and are unused:
  `plan.repo_config.resources` (`pod_builder.rs:715-728`), settable via the
  repository PATCH API.
- OOM → container restart in-place (writable layer survives) → at
  `K8S_MAX_CRASH_RESTART_COUNT` (default 5) the watcher auto-stops the
  Deployment; the next start is a **new pod** with a fresh writable layer.
- Toolchain paths: `RUSTUP_HOME=/opt/rust`, `CARGO_HOME=/opt/cargo`
  (`Dockerfile.sandbox-base:124-126`) — image layer + writable layer, on **no
  volume**. `build-cache` emptyDir (hardcoded 1536Mi, `pod_builder.rs:132`)
  is mounted at `/opt/moatless/build-cache` where nothing points at it.
- The base image bakes a Rust toolchain, but sub-tasks needed 1.94.0 —
  the baked version had drifted stale, forcing runtime installs.

### Fix plan

**2a. Set per-repo resources for Rust-heavy repos (no code change, do now).**
PATCH the moatless-vibe repository row:
`resources.limits.memory: 16Gi`, `requests.memory: 4Gi`, `limits.cpu: 8`.
Document the knob in `docs/sandbox-architecture.md`.

**2b. Keep the baked toolchain current (no code change, do now).**
Update `Dockerfile.sandbox-base` to the workspace's pinned Rust version
whenever `rust-toolchain.toml` changes (add a CI check that compares the two
and fails the sandbox-image build on drift). Eliminates most runtime installs.

**2c. Persist cargo/rustup state across pod replacement (code change).**
When the PVC tier is active, add two extra `VolumeMount`s of `workspace-data`
with `subPath: cargo-home` → `/opt/cargo` and `subPath: rustup-home` →
`/opt/rust` (`pod_builder.rs`, next to the existing `/tmp` subPath mount at
`pod_builder.rs:688-692`). Gate behind a per-repo or global flag
(`K8S_SANDBOX_PERSIST_TOOLCHAIN`) because it costs PVC space: bump
`K8S_SANDBOX_STORAGE_SIZE` for repos that enable it (a Rust registry +
target dir is 5-10Gi; the default 3Gi is far too small).

**2d. Point the target dir at build-cache (optional).**
Set `CARGO_TARGET_DIR=/opt/moatless/build-cache/target` via sandbox env and
raise the `build-cache` sizeLimit from the hardcoded 1536Mi
(`pod_builder.rs:132`) to a config value. Incremental compiles then survive
container restarts (not pod replacement — that's 2c's job).

**Rollout:** 2a+2b immediately; 2c behind flag on moatless-vibe first; 2d
after measuring build-cache pressure.

---

## 3. cd-ecr version race on close merges (P1)

### Observed

Three PRs merged within ~7 minutes; all three cd-ecr runs computed `0.1.163`
from the same latest tag. ECR tags are mutable — last writer won. The git tag
went to one commit, the gitops deploy PR to another, and the third run's
promotion failed on a ref lock. Recovery cost ~40 minutes of manual work
(dispatch rebuild, hand-authored deploy PR). A second variant: the
`workflow_dispatch` path skips `tag-release`, so the next push re-computed the
same version and overwrote ECR again.

### Current behavior

- Version computed in the `setup` job from `git describe --tags`
  (`.github/workflows/cd-ecr.yml:81-83`) — a pure read, no lock.
- The tag is only pushed 15-25 minutes later in `tag-release`
  (`cd-ecr.yml:605-631`), after all publishes. The entire build duration is
  the race window.
- **No `concurrency:` group exists at any level.**
- The `tag-release` idempotency guard (`cd-ecr.yml:624`,
  `git rev-parse "v${TAG}" && exit 0`) silently drops the loser's tag push —
  hiding the mismatch instead of failing.

### Fix plan

**3a. Serialize main builds (3 lines, do now):**

```yaml
concurrency:
  group: cd-ecr-${{ github.ref }}
  cancel-in-progress: false
```

Queued runs start after the previous `tag-release` has pushed, so they compute
the next version correctly. Tradeoff: back-to-back merges deploy serially
(~20 min apart) — acceptable at current merge frequency.

**3b. Fail loudly on tag collision.** Replace the silent `exit 0` guard in
`tag-release` with: if the tag exists **and points at a different SHA**, fail
the job (the published images are suspect). Same-SHA re-runs stay idempotent.

**3c. Immutable ECR identity (defense in depth).** Additionally tag every
main image with the commit SHA (PR builds already do, `cd-ecr.yml:85`) and
pass the SHA through promotion metadata, so any future mismatch is auditable
and recoverable without a rebuild.

**3d. Make `workflow_dispatch` tag properly.** The dispatch path publishes
images but skips `tag-release`/`trigger-promotion` (push-event conditions).
Either extend the conditions to cover dispatch or refuse to run the publish
jobs on dispatch — the current half-run is what enabled the 0.1.164
double-build.

---

## 4. Messages to closed tasks silently black-holed (P2)

### Observed

During the retrospective interviews, `moat send --task <closed>` returned
"Message sent", the message was persisted, and nothing ever consumed it — no
error anywhere. (The closed-task resurrection *guard* itself is correct and
worked; the gap is caller feedback.)

### Current behavior

- UI/API/CLI path (`send_message`, `backend/src/message/handlers.rs:31-102`):
  no status check; message persisted; `ensure_sandbox_and_notify`
  (`backend/src/sandbox/notify.rs:79-89`) silently skips closed tasks;
  handler returns `status: "sent"`.
- External-adapter path: `send_message_flow` drops closed-task messages
  (guarded by `external_adapter.is_some()`, `backend/src/message/send.rs:88-104`)
  — correct, but the guard doesn't cover internal callers.
- Tool-response path (`send_tool_response_flow`, `send.rs:472`): no status
  check at all.

### Fix plan

Add a closed-task guard **before any DB write** in both `send_message_flow`
(dropping the `external_adapter.is_some()` restriction) and
`send_tool_response_flow`. New `MessageError::TaskClosed` variant
(`backend/src/message/error.rs`) mapping to **HTTP 409**; add the 409 to the
utoipa annotations. `Error`-status tasks stay sendable (they are resumable —
matches `start_task` semantics). External adapters keep their silent-drop
behavior (409 is meaningless to an inbound webhook); only the HTTP handler
path propagates the error. No `SendMessageResponse` schema change needed —
the status code carries the semantics; frontend and `moat` CLI already
surface non-2xx as errors.

---

## 5. Dead Codex credentials shown as "Connected" (P2)

### Observed

The release-day codex verification tasks failed with *"Sign-in expired.
Please reconnect Codex in Settings"* — the user's refresh token had been dead
since ~June 10, invisible for a month because: Settings showed green
"Connected to ChatGPT", `/me` has no codex entry, the refresher deliberately
skips failed rows (`WHERE metadata->>'refresh_failure' IS NULL`,
`refresher.rs:100-106`), and the one-time transition WARN had long scrolled
away. Meanwhile `bot:reviewer`'s separate row kept working, masking the
breakage.

### Current behavior

- `GET /api/v1/settings/codex/config`
  (`backend/src/provider_settings/handlers.rs:518-559`) sets
  `connected: true` whenever a row exists — never reads
  `metadata.refresh_failure`.
- `/api/v1/auth/me` providers array only ever contains `github`
  (`backend/src/auth/routes.rs:457-533`); the frontend TopBar's amber
  "Reconnect GitHub" banner is driven from it — the exact pattern codex needs.
- On permanent refresh failure, `fill_codex`
  (`backend/src/sandbox/secrets.rs:292-320`) silently omits
  `CODEX_AUTH_JSON`; the task fails at the CLI layer with the generic
  sign-in message.

### Fix plan

**5a. `needs_reconnect` in the codex config response.** Read
`metadata.refresh_failure` in `get_codex_config`; add
`needs_reconnect: Option<bool>` to `CodexConfigResponse`
(`crates/api-schemas/src/provider_settings.rs:29-42`). Frontend
(`codex-configuration-section.tsx:79-118`): amber "Reconnect needed" alert +
reconnect button instead of the green alert.

**5b. Codex in the `/me` providers array.** `check_codex_connection` in
`auth/routes.rs`: `connected = row exists && refresh_failure absent`. Derive
`codexConnected` in `use-auth.ts` (default `?? true` like GitHub); add the
amber "Reconnect Codex" TopBar button. Surfaces on every page load, zero new
polling.

**5c. Fast-fail codex task creation (optional, high value).** In
`create_task_handler` (`backend/src/task/handlers.rs:132-184`), when
`agent_type == "codex"`, pre-flight the credential (reuse
`ensure_fresh_codex_for`; map `NotFound` to a 400 with *"Your Codex
connection has expired — reconnect in Settings"*). Saves a full sandbox
provision per doomed task and puts the actionable message at exactly the
right moment.

---

## 6. Duplicate `initializing` event bursts (P3)

### Observed

Task event histories show 8+ identical `sandbox initializing` events within a
minute (e.g. task `00bef0bb`), polluting history and SSE streams.

### Current behavior

Six independent publishers emit `Initializing` (provision, restart, and the
pod watcher's `container_terminated`/`container_restarted`/
`container_crash_loop`/`pod_scheduled`/`pod_recovering`/sweep paths —
`pod_watcher.rs:506-608`, `1773-1846`). No dedup of consecutive identical
statuses exists: `EventDedup` keys on `(task_id, event_id)` and every watcher
publish mints a fresh UUID. A flapping container fires one publish per K8s
watch Apply.

### Fix plan

Add `last_published_sandbox_status: Option<SandboxStatus>` to
`PodObservedState` (`pod_watcher.rs:~89`), carried across Apply events. Skip
the publish when the status is unchanged **and** the lifecycle sub_type is in
the high-frequency set (`container_restarted`, `container_crash_loop`,
`container_terminated`); always publish genuine transitions (`pod_ready`,
`pod_failed`, `pod_evicted`, `pod_recovering`, `pod_scheduled`). Follows the
existing fire-once pattern (`scheduling_blocked_published`,
`pod_watcher.rs:618-619`). Watcher restart resets the memory — acceptable
(one extra event per pod after failover).

---

## Out of scope / tracked elsewhere

- **Forgejo read grant** for key `4:moatless-forgejo` on
  `config-staging`/`config-default` — infra action, not code (blocks SSH
  clone of config repos; surfaced correctly as exit-78 permanent init
  failure by #275).
- **`moat gh pr checks` 403** — GitHub fine-grained-PAT limitation
  (`checks:read` not grantable); workaround is `moat gh run list/watch`.
  Document in the moat-cli skill.
- **Ephemeral loose files in agent sandboxes** (e.g. session tokens outside
  the repo) — by design; agents must treat non-repo paths as disposable.

## Suggested PR slicing

1. `ci: serialize cd-ecr main builds + loud tag-collision failure` (3a, 3b, 3d)
2. `fix: publish tool progress as agent activity + reaper live-status check` (1a, 1b)
3. `feat: process-aware sandbox busy signal for the idle reaper` (1c)
4. `fix: reject messages to closed tasks with 409` (4)
5. `feat: surface codex refresh failure in settings and /me` (5a, 5b, 5c)
6. `fix: dedup consecutive identical pod-watcher status events` (6)
7. `chore: per-repo resources for moatless-vibe + toolchain drift CI check` (2a, 2b)
8. `feat: PVC-backed cargo/rustup persistence (flagged)` (2c, 2d)
