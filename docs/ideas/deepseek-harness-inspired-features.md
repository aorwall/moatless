# Feature ideas from deepseek-harness

> **Status:** idea backlog — not an approved plan. Each item needs its own spec
> (research anchors, data model, acceptance criteria) before implementation.
> **Source:** survey of github.com/deepseek-ai/deepseek-harness (`dsh`) against
> moatless's current architecture, 2026-09-01.
> **Vocabulary:** CONTEXT.md

`dsh` is a full agent harness (owns its own agent loop, tool pipeline, LLM
adapter). Moatless orchestrates existing harnesses (Claude Code, Codex) inside
Sandboxes, so most of `dsh`'s tool-pipeline/compaction/context-assembly work is
already owned by the harness a Task runs. The items below are the platform-level
capabilities confirmed absent or incomplete in moatless.

Also identified but not included in this pass: **MCP server config per
Workspace** (both Claude Code and Codex speak MCP already, but moatless has no
`Workspace`/`Task` field or UI to register an external MCP server — only
internal SDK wiring in `crates/claude-agent-sdk/src/mcp/*`). Parked here for a
later pass.

## 1. Approval-policy / permission presets as a Workspace setting

`crates/sandbox/src/agent/codex_app_server/mod.rs` hardcodes
`approval_policy: AskForApproval::Never` — every Task runs with no approval
gate. `dsh`'s permission-preset
pattern: named bundles combining sandbox mode + approval policy into one
selector (`workspace-write`, `danger-full-access`, read-only), failing closed
when an answerer is missing or errors.

Land as a Workspace-level (or Task-level override) setting, threaded to the
Codex/Claude harness options at Sandbox launch.

## 2. Full-text search across Tasks

A T3 UI-RPC conversation search already exists —
`crates/t3code/src/dao.rs::search_conversations`, exposed as
`orchestration.searchThreads` — matching followed open task names/messages via
`ILIKE`. There is no indexed or product-wide search: it's unindexed
substring matching, scoped to the T3 surface, with no equivalent endpoint or
UI in the main frontend. `dsh`'s `session-query` package runs SQLite FTS5 over
the durable event log plus relationship/lineage tracing. Moatless's Message
store is already durable and structured (`conversation_messages`, the
`TranscriptAppend` port) — an FTS index over message content, exposed as a
main-product endpoint and UI, is additive on top of what `search_conversations`
already proves out.

## 3. Task export

`crates/server/src/sandbox/grpc_server.rs::download_transcript` already
builds a transcript archive, but only for internal session restore over gRPC.
Expose the same artifact as `GET /api/v1/tasks/{id}/export` for user-facing
download (compliance, sharing, offline review).

## 4. Cross-task references

Moatless already records that one Task relates to another —
`task_relations` (`crates/tasks/src/task/relations.rs`, the
`RelationRecorder` port) and per-message `source_task_id`
(`crates/api-schemas/src/message.rs`) attribute inbound content to the Task it
came from. What's missing is a user/agent-facing way to *inject* another
Task's transcript as read-only reference context into the current one — the
relation and attribution model records that a link exists, not the content on
the other end of it. `crates/adapter-model/src/mentions.rs` only mentions
*people*, for outbound Adapter formatting, so no existing surface fills this
either. `dsh`'s `dsh-session:` URIs do this: one session references another
and gets back a read-only snapshot injected as untrusted context. A moatless
version should read through `task_relations`/`source_task_id` for the "is this
Task allowed to see that one" question rather than inventing a second
relation model.

## 5. Runtime invariant framework for Adapters and Plugins

No matches for `invariant`/`HealthCheck`/`self-check` in
`crates/server/src/adapters` or `crates/workspaces/src/plugin`. `dsh`'s
`runtime-diagnostics` package: each package registers live health assertions
checked continuously against real event streams, violations attributed to the
owning package, filterable and CI-gatable. Worth building once the Adapter/
Plugin count keeps growing — lower urgency than 1–4.

## 6. In-task scheduling, in two shapes

Two distinct capabilities, both currently absent, neither the same as a Loop's
cron `Schedule` (which is recurring and platform-level, not scoped to one
Task):

- **In-conversation reminder.** `dsh`'s `schedule_create` tool: a session-local,
  one-shot or interval reminder delivered as an ordinary message in the same
  conversation when due, survives restarts, fires only if the session is still
  live. Lets an agent (or a person) say "check back on this in 30 minutes"
  without leaving the Task.
- **Scheduled Task start.** Not present in `dsh` either — a moatless-specific
  extension: create a Task now with a start time in the future, so it begins
  its first Turn at that time rather than immediately. Distinct from a Loop
  (which is a standing, named, recurring definition); this is a one-off,
  ordinary Task whose creation and start are decoupled. Needs a `starts_at`
  (or similar) field on Task creation and a scheduler that promotes it to
  running at the right time — likely reusing the Loop firing/Schedule
  machinery's timer rather than inventing a second one.

## 7. Goal / persona system

- **Goal.** `Task` (`crates/types/src/task.rs`) has `name`/`description`,
  editable via `PATCH` (`UpdateTaskMetadataRequest`,
  `crates/tasks/src/task/handlers.rs::update_task_metadata`), but no dedicated
  "objective" concept re-injected as a standing goal the way `dsh`'s `goal`
  package does (one durable completion objective per session, persisted in the
  log, survives restart/resume/fork, with an opt-in "round driver" that turns
  an active goal into automatic sequential rounds of work).
- **Persona.** `crates/workspaces/src/plugin/*` already composes a bundle of
  Skills per Workspace via toggle-based Activations, but there is no single
  named selection of a whole configuration — system prompt + tool set + skills
  — the way `dsh`'s `agent-presets`/`persona` packages let one preset swap
  agent identity, not just tools.

Land as: a `goal` field on Task, distinct from `description`, re-surfaced to
the agent each Turn; and a Workspace-level list of named personas (prompt +
default skill set + default mode/model) a Task can be created against instead
of composing skills by hand each time.
