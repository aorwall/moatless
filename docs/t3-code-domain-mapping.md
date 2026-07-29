# T3 Code ↔ Moatless: domain mapping and terminology

This document defines how T3 Code's domain model maps onto Moatless's, which
vocabulary wins where, and what the phase-one UI RPC surface
(`backend/src/ui_rpc/mod.rs`) has to fill in next.

It complements [the local development runbook](./runbooks/t3-code-local-development.md),
which covers how to *run* T3 Code against this backend. This document covers what
the data flowing through it *means*. Domain terms are defined in
[`CONTEXT.md`](../CONTEXT.md).

**The deeper analysis lives in the T3 Code repository**, on its `moatless-adapter`
branch — `docs/reference/moatless-concept-map.md` walks every contract surface
feature-by-feature, and `.plans/moatless-convergence.md` proposes changing both
sides. This document is the Moatless-side summary and the terminology decision; it
does not repeat their findings. Where the two disagree, that is called out inline.
See [the orientation note](./t3-code-integration-start-here.md) for reading order.

Read against `soaplabs/t3code` @ `d67a67a` and this repository @ `ef4d860`.

## The two models are not variants of each other

Both systems model a container of run-configuration holding many agent
conversations, each made of turns driven by a provider session. They disagree
about where truth lives, and every downstream difference follows from that:

- **T3 Code derives state from events.** `orchestration_events` is append-only
  and authoritative; all nine `projection_*` tables are rebuildable folds keyed
  on a global `sequence`.
- **Moatless derives events from state.** `tasks`, `task_turns`, `task_messages`
  are authoritative. `migrations/0071_task_turns.sql` demoted the log
  deliberately, so that *"events become projections rather than the source of
  truth."*

The adapter is therefore an anti-corruption layer, not a translation table. It
is expected to serve T3 Code's contract from Moatless's rows without either side
adopting the other's storage model.

## Direction: Moatless vocabulary wins

Where a concept genuinely exists on both sides, T3 Code renames toward Moatless.
Three reasons, all structural:

1. **`thread` is already taken in this repository**, and it means *external
   channel thread*: `channel_threads` (`migrations/0002_channels.sql:30`),
   `external_thread_id`, `routing_mode = 'thread_based'`, and the subscription
   kinds `slack_thread` / `telegram_thread` / `connected_thread`
   (`migrations/0035_slack_telegram_subscription_cutover.sql`). Importing T3
   Code's Thread-as-conversation would overload a word that is live in the
   adapter subsystem. The reverse has no collision — T3 Code has no `Task`.
2. **Blast radius is asymmetric.** Moatless's `Task` spans 108 REST path
   templates, the Orval-generated TypeScript client, ~100 migrations, and adapter
   payloads. T3 Code's `Thread` is ~16.7k occurrences across ~1.1k files, but
   nearly all of them derive from `packages/contracts`, the single source of
   types for its server, web, and mobile targets. One rename there propagates by
   type error.
3. **This repository already has the written glossary.** `CONTEXT.md` plus a
   numbered ADR set. T3 Code has `docs/` but no glossary file.

Where a concept exists only in T3 Code, **T3 Code's name carries over unchanged**
— there is nothing to harmonize with.

## Mapping

| T3 Code | Moatless | Status |
| --- | --- | --- |
| `Thread` | `Task` | Genuine synonym. Rename in T3 Code. |
| `Turn` (`projection_turns`) | `Turn` (`task_turns`) | Already identical. Status enums are not the same size — `running·interrupted·completed·error` (four) vs `active·completed·error·stopped·interrupted` (five) — and the shared word `interrupted` does not mean the same thing on both sides. Mapping settled; see the known gaps below. |
| `Session` (`projection_thread_sessions`) | `Session` | Same concept — the resumable provider conversation. Moatless additionally splits out `Sandbox` as the durable pod-lifecycle intent; T3 Code's session row carries both concerns. |
| Message | `TaskMessage` + content blocks | T3 Code stores flat rows; Moatless stores one row with N blocks and folds tool results into the calling assistant message at assembly (ADR-0002). |
| Activity (`projection_thread_activities`) | `task_events` | **Not equivalent.** T3 Code's activities are a projection; `task_events` is history. |
| `Project` | `Workspace` | Mapped (`projectId ↔ workspaces.id`), but **not** a synonym — see below. Do not rename either. |
| `Environment` | — | T3 Code only. Keep the name. |
| Checkpoint, proposed plan, `settled`/`snoozed` | — | T3 Code only. Keep the names. |
| — | `Binding`, `Adapter`, `Loop`, `Schedule`, `Subscription` | Moatless only. |
| — | `Scope`, `Visibility` | Moatless only. `Scope` is two values (`Global`, `User`); ownership is `COALESCE(owner_user_id, created_by)`. `team_id` is **inert for access control** since `migrations/0097_scope_two_values.sql` — do not filter on it. |

### Do not rename `Project` to `Workspace`

A T3 Code Project **is a filesystem path** — one `workspace_root`, unique among
live projects, plus a `repositoryIdentity` derived read-only from the git remote.
A Moatless Workspace is run-configuration (image, setup commands, env vars,
servers, resources) composing 1..n repositories through `workspace_repos` with
placements and a designated primary.

The honest mapping is:

```
T3 Code Project  ≅  Moatless Workspace with exactly one WorkspaceRepo
```

Renaming Project to Workspace before T3 Code can compose repositories would put
the name on a capability that does not exist. Either deepen Project into real
repository composition first, or keep the name and document the projection.

### Decision: a T3 Project maps to a Workspace

**Settled.** `projectId ↔ workspaces.id`. This supersedes the Project→Repository
row in T3 Code's `docs/reference/moatless-concept-map.md` (`projectId ↔
task.repositoryId`), which is now stale.

Rationale: a Task already references `workspace_id`, so the Workspace mapping is
the direct one — a Repository-shaped Project would have to resolve to a Workspace
on every task creation anyway. Multi-repo Workspaces stay visible instead of being
flattened, and no Repository becomes unreachable, because **Default workspace**
already guarantees every Repository has one.

Field by field (as built in `ui_rpc/projection.rs`). The shell listing carries
`OrchestrationProjectShell`; `OrchestrationProject` is its near-twin with one
extra field, and projecting the wrong one is an easy mistake:

| `OrchestrationProjectShell` | Moatless source |
| --- | --- |
| `id` | `workspaces.id` |
| `title` | `workspaces.name`, falling back to `workspaces.id`. `name` is `text NOT NULL` with no non-empty constraint (`0079_workspaces.sql:24`) and `title` is `TrimmedNonEmptyString`, so a blank name would make the listing undecodable. |
| `workspaceRoot` | **Synthetic — see the hazard below.** Built as `/moatless/workspaces/{workspaces.id}`, matching the equally nominal `cwd` the descriptor already reports. |
| `repositoryIdentity` | The primary `WorkspaceRepo`'s Repository. `canonicalKey` is its normalized remote URL. Emitted as `null` while the descriptor advertises `capabilities.repositoryIdentity: false`; the field is `Schema.optional(Schema.NullOr(RepositoryIdentity))` (`orchestration.ts:393`), so `null` decodes. |
| `defaultModelSelection` | `null`. Model selection is per Task/Turn in Moatless, not per Workspace. |
| `scripts[]` | `workspaces.servers` (`ServerConfig`). `label`/`name` → `name`, `name` → `id`, `start_commands` joined with `&&` → `command`, `default` → `autoOpenPreview`. **`ProjectScript` has three more required fields** (`orchestration.ts:192-209`): `id`, `icon`, and `runOnWorktreeCreate`. `icon` is a closed literal set (`play·test·lint·configure·build·debug`, `:182-189`); every Moatless server is a long-running process bound to a port, so it is always `play`. `runOnWorktreeCreate` is `false`. The flag is an instruction to the client rather than a description of Moatless: the client runs the flagged script itself when it creates a working copy (`packages/shared/src/projectScripts.ts:35-37`, `ProjectSetupScriptRunner.ts:127`). A Moatless server is already started by the Sandbox, which runs it as a container in the pod (`k8s/pod_builder.rs:build_server_containers`), so flagging it would put a second process on the same bound port. `previewUrl` has **no direct source**: `ServerConfig.port` is a `u16` (`backend/src/sandbox/models.rs:130`) and `previewUrl` is an optional non-empty URL string. The adapter must synthesize the sandbox's published preview URL for that port, or omit the field — a bare port number is not a valid value. `setup_commands` are provisioning, **not** scripts — do not map them here. **Emitted only for a Workspace the viewer may read.** The project list is a union (spec Decision 5) and its widened half — Workspaces present only because a followed conversation references them — is projected with `scripts: []`: the widening is argued on names, and a start command carries paths and flags rather than a name. |
| `createdAt` / `updatedAt` | `workspaces.created_at` / `updated_at` |
| `deletedAt` | **Not a field of the shell listing.** The snapshot carries `OrchestrationProjectShell` (`orchestration.ts:389-397`, referenced at `:432`), which is `OrchestrationProject` minus `deletedAt`. Deleted rows are filtered from the snapshot, so nothing is lost. Project the shell type, not its near-twin. |

`workspaces.kind` (`blank` \| `single` \| `multi`) tells the adapter up front when a
Workspace cannot be represented faithfully as a single root.

### Hazard: `workspaceRoot` is an identity, not a label

T3 Code enforces **uniqueness of `workspaceRoot` across active projects** —
`commandInvariants.ts:94`: *"Active project '{id}' already exists for workspace root
'{root}'."* The client also derives its physical project key from it
(`${environmentId}:${normalizedPath}`), and uses that key for sidebar grouping,
ordering, and per-project overrides.

Moatless has no per-Workspace path. Repositories are cloned into
`{WORKSPACE_DIR}/{mount_name}` inside a per-Task sandbox, so `WORKSPACE_DIR` is
**the same string for every Workspace**. Reporting it verbatim would make every
project share one physical key and collapse into a single sidebar group, and would
be rejected outright on the write path.

The adapter must therefore synthesize a **stable, unique, per-Workspace** root
(encoding `workspaces.id`). It is nominal either way — the real path only exists
once a Task has a sandbox.

Related consequence: T3's project-scoped file operations
(`projects.listEntries`/`readFile`/`writeFile`) have no target until a Task exists,
because files live in a per-Task sandbox. This is the same gap convergence §2.2
describes as *"every method that is implicitly the environment's today gains a
`threadId`"*.

### Do not rename the persisted event literals

`thread.create`, `thread.settle`, `thread.snooze` and the rest are persisted
string literals in T3 Code's `orchestration_events.event_type` — an append-only,
never-pruned table that is its only source of truth. Renaming them breaks replay
of all history.

When `Thread → Task` lands, move the **type layer** in `packages/contracts` and
leave the wire and storage strings alone, or ship an alias map applied at read
time. Same discipline as a database column rename.

### Decision: a T3 thread listing is what the viewer follows

**Settled.** `threadId ↔ tasks.task_id`, and the shell listing carries the Tasks
the viewer **follows** *and* may still **read**.

The two filters are different rules and both are load-bearing:

- **Follow** is what makes this a sidebar rather than a deployment listing. It
  already drives the Moatless sidebar (`task_follows`, migration `0069`), so the
  two clients agree about what a person's list is instead of T3 getting a rule of
  its own. **An administrator is not a special case** — their sidebar is what
  they follow, which is deliberately the opposite of `projects[]`, where an
  administrator does see the whole deployment.
- **Read** is what stops a follow, which is durable, from outliving the access
  that produced it. A conversation can be made private or change hands long after
  someone wrote to it. The filter is the shared
  `ScopeContext::access_filter(ScopedEntity::Task, …)` fragment rather than a
  copy, so a change to what "may read" means reaches this listing too.

Following never *grants* access — it only ever narrows the readable set.

Field by field (as built in `ui_rpc/projection.rs`), against
`OrchestrationThreadShell` (`orchestration.ts:401-427`) — the near-twin of
`OrchestrationThread` without its messages, activities, checkpoints, proposed
plans and `deletedAt`:

| `OrchestrationThreadShell` | Moatless source |
| --- | --- |
| `id` | `tasks.task_id` |
| `projectId` | `tasks.workspace_id`. A Task whose Workspace does not resolve is **left out of the listing**, not emitted — see the hazard below. |
| `title` | `tasks.name`, falling back to `tasks.task_id`. `name` is nullable with no non-empty constraint and `title` is `TrimmedNonEmptyString`. |
| `modelSelection` | `{instanceId, model}`, both required and non-empty (`:64-68`). Uses the newest Turn's model, falling back to the deployment default for the agent kind; `instanceId` is the driver's default instance (`ClaudeCode → claudeAgent`, `Codex → codex`). |
| `runtimeMode` | `full-access` — `DEFAULT_RUNTIME_MODE` (`:124`). Moatless has no equivalent, and the permissive mode is an honest description of a Sandbox: a process in a pod with no approval gate in front of it. |
| `branch` | `tasks.branch`, `null` when blank. |
| `worktreePath` | `WORKSPACE_DIR` (`/opt/moatless/workspace`) when a Sandbox exists and has not been removed; `null` otherwise. Unlike a project's `workspaceRoot` this is a **label, not an identity** — the client only displays it — so one shared value collapses nothing. |
| `latestTurn` | The newest `task_turns` row by `turn_number`; `null` when the Task has never run. `turnId` is `{task_id}:{turn_number}` — Moatless has no single-column turn id. `requestedAt` and `startedAt` are both `started_at`: a Moatless Turn comes into existence already running. `assistantMessageId` is `null` — the shell listing carries no messages. |
| `createdAt` / `updatedAt` | `tasks.created_at_db` (falling back to the text `created_at`) / `tasks.updated_at`. |
| `session` | The `sandboxes` row, or `null` when none exists — absence is the lazy converged-Stopped default and is the one case meaning *no session*. `status` folds `desired_state` + turn openness + `stop_reason`; `activeTurnId` is set only while the newest turn is open; `providerName` is the agent catalog's label; `lastError` is `null` (Moatless records no durable Sandbox error text). `runtimeMode` is **omitted** here — the session's copy carries a decoding default, unlike the conversation's. |
| `latestUserMessageAt` | `MAX(task_messages.created_at) WHERE message_type = 'user'` — see the hazard below. |
| `hasPendingApprovals` | `false`. Moatless has no approval queue and the contract supplies no empty value. |
| `hasPendingUserInput` / `hasActionableProposedPlan` | An unanswered `AskUserQuestion` / `ExitPlanMode` `tool_use` block in the newest assistant message. |
| `interactionMode`, `archivedAt`, `settledOverride`, `settledAt`, `snoozedUntil`, `snoozedAt` | **Deliberately absent, not null.** All six are `Schema.optional` or carry a `withDecodingDefault`, and Moatless has nothing honest to put in them. Leaving them out lets the contract fill them. |
| `deletedAt` | **Not a field of this type** — it belongs to `OrchestrationThread` (`:370`). Same trap as `OrchestrationProjectShell`. |

Because no `archivedAt` is ever emitted, `orchestration.getArchivedShellSnapshot`
must **not** return the live listing: the client's archived view is
`archived_at IS NOT NULL` (`ProjectionSnapshotQuery.ts:414`), so returning it
would present every followed conversation as archived. It returns the viewer's
projects with no conversations.

#### Hazard: the two values with a plausible wrong source

**`latestUserMessageAt` counts only messages a person wrote.** That is T3's own
definition — its pipeline reads `message.role === "user"`
(`ProjectionPipeline.ts:564-570`). Moatless's nearest stored value,
`tasks.last_message_at`, **also advances when the agent writes**
(`agent_handlers.rs:101`, `:442`, `:552`), so a sidebar built on it reorders
itself while nobody is talking. The value is derived instead, from
`task_messages.message_type = 'user'` — this backend's own predicate for a
person's message, since the SDK's `user`-role rows (tool results streamed back)
are persisted as `system` (`agent_handlers.rs:831`).

**A conversation whose Workspace does not resolve must be left out of the
listing entirely.** New Tasks always get one (`task/dao.rs::insert_task` resolves
or creates a shadow Workspace), but older and hand-written rows can have
`workspace_id IS NULL`, and provisioning still falls back to repository-only
behaviour for those. A row whose `projectId` is absent from the same snapshot is
dropped by the client with a bare `continue` — no error, nothing logged
(`Sidebar.logic.ts:779`). Dropping it on this side costs the same row and leaves
a warning behind. The same applies to a Workspace that is soft-deleted, which is
why the drop happens after the workspace read rather than in SQL.

## Two axes of federation

A recurring source of confusion: both systems are "multi-something," but not the
same something.

- **T3 Code federates over machines you own.** `environmentId` is *held*
  client-side, one per connection target (`Primary`, `Bearer`, `Relay`, `Ssh`).
  Ids are only unique within an environment, so all cross-environment references
  are scoped `{environmentId, projectId}`. Projects are grouped across machines
  client-side by normalized git remote URL.

  **Correction: the client does not assign the id, and the server's answer is
  authoritative.** This paragraph originally read *"`environmentId` is assigned
  client-side … No server table mentions it."* Neither sentence holds. Every
  target type takes its id from the server's `/.well-known/t3/environment`
  descriptor: onboarding reads it (`client-runtime/src/connection/onboarding.ts:100-110`),
  the same-origin primary rebuilds its target from it on every discovery
  (`apps/web/src/connection/platform.ts:286-300`), and bearer, relay and SSH
  connections re-fetch it on every connection attempt and *refuse to connect* if
  it has changed (`client-runtime/src/authorization/service.ts:114`,
  `client-runtime/src/connection/resolver.ts:106`). The correction is recorded
  rather than removed because the original reading is the one that makes a
  server-derived id look like a free choice: this backend
  derives the id from the database it is serving, and a client that finds a
  different id is either starting fresh or refusing to connect — never resuming
  into a listing that no longer exists.
- **Moatless federates over access scope.** Rows carry `scope` (`Global` or
  `User`) plus ownership, and every query is filtered by a `ScopeContext`. Teams
  are an org concept and grant no resource access (ADR 0006).

T3 Code has nowhere to express *who may see this*; Moatless has nowhere to
express *this task lives on a machine I am not connected to right now*. The
adapter presents Moatless as **exactly one T3 environment**, and access control
stays entirely on the Moatless side.

## What phase two has to fill

`projects[]` and `threads[]` are populated; the sequence and replay
(`orchestration.replayEvents → []`) are not. `server_config` already advertises
`shellResumeCompletionMarker` / `threadResumeCompletionMarker`. The wire shape
is fixed by T3 Code's contract:

| Snapshot field | Moatless source |
| --- | --- |
| `projects[]` | **Done.** `workspaces`, filtered to what the viewer may read **plus** any workspace a listed Task references — the two lists cannot be filtered the same way, because a Task whose project is absent is dropped by the client in silence. One project per workspace; `workspaceRoot` is **synthetic and per-Workspace** — not the primary repo's mount path. See [the hazard](#hazard-workspaceroot-is-an-identity-not-a-label). |
| `threads[]` | **Done.** `tasks` the viewer **follows** *and* may read, projected to `OrchestrationThreadShell`. The two filters are different rules and both are load-bearing — see [the conversation listing](#decision-a-t3-thread-listing-is-what-the-viewer-follows). |
| `snapshotSequence` | See below. |
| `updatedAt` | Wall clock at assembly. |

### The sequence is the load-bearing problem

T3 Code's client is a **sequence-fold client**: its shell reducer drops any event
whose `sequence <= snapshotSequence`, and resumption is `subscribeShell({afterSequence})`.
Moatless has no monotonic per-tenant sequence and explicitly no SSE replay — its
own frontend calls `invalidateQueries({refetchType: 'active'})` on reconnect.

The shape of the fix is favourable. T3 Code's shell stream does not fold event
payloads — it re-reads the projection row and ships a whole shell object, which
the client applies as upsert-by-id behind the sequence guard. So the adapter does
not need Moatless's events to be authoritative. It needs one question answered:

> which `task_id`s have changed since sequence `N`

and `task_events (id, task_id)` is the obvious place to answer it from. Both
systems already converged on *idempotent full upsert keyed on a stable id* — T3
Code's shell re-read, and Moatless's derived `message.upsert` keyed on the
persisted `uuid` (ADR-0002) — which is what makes this tractable at all.

**But `task_events.id` is not a safe cursor as-is, and this must be settled before
anything is built on it.** A `bigserial` is assigned at `INSERT` and made visible
at `COMMIT`, so it can be read out of order: two writers take ids 100 and 101, 101
commits first, a reader polling `id > 99` advances to 101 and never sees 100. A
naive `MAX(task_events.id)` at snapshot assembly has exactly this bug, and it fails
silently — the client believes it is caught up. T3 Code's own server names the same
hazard from the other side (`ws.ts:1226`: attach the live buffer before querying the
snapshot, or an event published mid-query is lost).

Three options, analysed in T3 Code's `.plans/moatless-convergence.md` §3.5:

- serve catch-up from the **live consumer's buffer** rather than a fresh `SELECT`,
  so the reader never straddles a commit boundary;
- hold the cursor below the oldest in-flight transaction (`pg_snapshot_xmin`),
  trading latency for correctness;
- allocate the sequence **inside the transaction** from a counter, so ordering and
  visibility coincide.

The third also resolves a smaller mismatch: T3's `sequence` is a
`NonNegativeInt` (a JS number) while `task_events.id` is `i64`.

That third option is what shipped, but **per deployment, not per Task** — this
paragraph said "per-task counter" until implementation settled it the other way. The shell snapshot carries one `snapshotSequence` and the
client keeps one cursor, so a per-Task counter produces many positions where the
contract has room for one, and no way to order them against each other. The counter
is `task_listing_position`; see `backend/src/task_listing.rs`.

Worth noting independently: today's SSE lag is *silent* — `sse/routes.rs` logs
`SSE client lagged, skipping events` and drops frames with no way for the client
to detect it. Adding a sequence makes that detectable, which is a win regardless
of what else lands.

### Known gaps to resolve

- **Pagination.** T3 Code's shell snapshot has no pagination and holds every
  non-archived thread resident. Acceptable for one developer's machine; it will
  not survive a tenant. Windowing the shell is the one change required on the T3
  Code side.
- **Lifecycle mismatch.** T3 Code's `settled`/`snoozed` are client-derived and
  clock-dependent. Moatless's `TaskStatus` is server-durable `Open·Closed·Error`.
  Decide whether the adapter synthesizes settlement or T3 Code hides the control
  when the environment does not advertise the capability.
- **Turn status enums.** **Settled.** The enums are not the same size: `OrchestrationLatestTurnState`
  (`orchestration.ts:326`) is four values — `running·interrupted·completed·error`
  — against Moatless's five. **T3 has no `pending` turn state** (an earlier
  reading of this document claimed one): the only `"pending"` literal in the
  contract is `ProjectionPendingApprovalStatus` (`:1333`), which is about
  approvals. The two non-obvious mappings: Moatless `stopped` → T3 `interrupted`,
  and Moatless `interrupted` (which is *open* — it resumes on reschedule) → T3
  `running`, **not** its like-named counterpart. A Task with no turn reports
  none: `latestTurn` is `Schema.NullOr` (`:412` on `OrchestrationThreadShell`;
  `:356` is the same field on `OrchestrationThread`, and `:355` — cited by an
  earlier revision of this document — is `worktreePath`).
- **A thread's model.** `OrchestrationThreadShell.modelSelection`
  (`orchestration.ts:405`) is required, and a `ModelSelection` is
  `{instanceId, model}` with `model` a `TrimmedNonEmptyString` (`:64-66`) —
  no null, no decoding default. **Settled.** The adapter takes the newest Turn's model, by the same rule
  `task/statistics.rs` already uses for the Moatless UI (latest
  `assistant_message_model`, else the requested `model`), then the deployment
  default for the agent kind, then one last-resort constant. The catalog name
  `default` is not a model identity and must not be reported as one.
  `instanceId` is the routing key, not part of the model name: it is an **open**
  branded slug (`providerInstance.ts:59`), and
  `defaultInstanceIdForDriver(driver) = driver` (`:148`), so a Moatless
  `AgentKind` maps to the driver's default instance — `ClaudeCode → claudeAgent`,
  `Codex → codex` (driver slugs at `model.ts:130-131`). An unrecognized instance
  is marked unavailable at runtime rather than failing to decode.
- **`task_events` retention.** Unpruned today. If it is ever pruned, resume
  windows break silently; the adapter must fall back to a full re-snapshot past a
  configured gap, as T3 Code's own server does.

## Naming rule for the adapter

The wire contract is T3 Code's, so `ui_rpc` emits `projects`, `threads`, and
`snapshotSequence` regardless of internal naming. **Terminology harmonization
does not change the wire.** Inside the backend, Moatless names apply. The
translation belongs in `ui_rpc` and nowhere else — that is the whole point of an
anti-corruption layer.

## Suggested order

1. Extend `CONTEXT.md` with the T3 Code-only concepts and the explicit
   non-equivalences recorded here.
2. Build out `ui_rpc` against current names on both sides. This is where the
   mapping gets tested, and it is reversible.
3. Only then rename `Thread → Task` in T3 Code's `packages/contracts`, storage
   literals untouched.
4. Decide user-visible copy separately — "thread" vs "task" in the UI is a
   product call, not a schema one.

Renaming before step 2 would commit to a mapping that has not been exercised yet.
