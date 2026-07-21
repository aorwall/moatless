# Moatless Vibe

The ubiquitous language for Moatless Vibe — a platform that runs coding agents
(Claude Code, Codex) inside on-demand sandboxes against git repositories, driven
by people directly or by Loops (recurring agent work), scoped to users and teams.

This file is a glossary, not a spec. It records what each term means and which
synonyms to avoid — not how anything is implemented.

## Language

### Runtime

**Sandbox**:
The per-task, on-demand isolated container (a Kubernetes pod or Docker container)
that runs the coding agent and its servers. Exactly one per Task; ephemeral.
A Sandbox materializes a Workspace on disk under its workspace directory.
_Avoid_: worker, environment, workspace pod

### Workspace & repositories

**Repository**:
A connection to a git repo — its name, remote URL, default branch, and Git host.
Run-configuration lives on the Workspace, not here. A Task references a Workspace,
never a Repository directly; a Workspace references Repositories through
WorkspaceRepo placements.
_Avoid_: putting run-config on a Repository

**Workspace**:
The composition of one or more repositories plus run-configuration (docker image,
setup commands, env vars, servers, plugins, resources) that a Task runs against.
A Sandbox materializes it on disk under the workspace directory. `source` is
`manual` (created via the API) or `git` (reconciled from a repo's
`.moatless/workspaces.json`). Not the Sandbox (the pod that runs it) nor the
Workspace panel (a task-page UI view).

**WorkspaceRepo**:
A single repository's placement inside a Workspace — its branch override, mount
name (subdir under the workspace directory), primary flag, and ordering. Exactly
one placement is primary (the repo used for diffs, PRs, and CLAUDE.md).

**Default workspace**:
The single-repo Workspace automatically derived for a Repository, so a Repository
is runnable without anyone authoring a Workspace by hand.
_Avoid_: shadow workspace

**Workspace panel** (also **Workspace view**):
The non-chat side of the task page — the pane that switches between the preview,
the file explorer, and the sandbox diagnostics for a Task. A frontend UI concept
only, distinct from the Workspace entity; always carry the `panel`/`view` suffix
to keep them apart.

### Tasks & lifecycle

**Task**:
The central unit of work: one conversation with a coding agent, run against a
Workspace inside a Sandbox. Owns its status (open / closed / error), visibility,
cost, and ownership. Created by a human or an external Trigger.
_Avoid_: agent run (a legacy name for a running Task)

**Turn**:
One user-message-and-agent-response cycle within a Task — the authoritative,
persisted lifecycle unit (`task_turns`). A Task has many Turns.

**Run**:
The sandbox runtime's name for a single Turn: the agent invocation the actor
mints a `run_id` for on each idle→running transition. One Run = one Turn — `Turn`
is the product term, `Run` the runtime-internal twin.

**Session**:
The agent CLI's resumable conversation, resumed across Turns via `--resume`. A
Task has one-or-more Sessions; a Session spans many Turns. A new Session begins
only on `start_fresh`. Not a per-Turn unit — that is the Run. Claude identifies a
Session with a `session_id`; Codex calls the same thing a **thread** (`thread_id`,
resumed via `codex exec resume`) — a legitimate Codex-scoped synonym.
_Avoid_: bare "session" for the user login session — always qualify that as
"auth session". Do not confuse the Codex thread with an external Slack "thread"
(that is a Binding).

### Providers — always qualified

"Provider" is never a domain term on its own: four unrelated concepts share the
word, so it must always be qualified.

**Sandbox runtime**:
The backend that runs Sandboxes — Kubernetes or Docker.
_Avoid_: sandbox provider

**Provider token**:
A stored OAuth or PAT credential for an external service (e.g. a GitHub token).
"Provider" appears here only as a qualifier of "token", never alone.

**Git host**:
The service hosting a Repository's git remote — GitHub, GitLab, or Gitness.
_Avoid_: git provider, bare "provider"

(The external-system integration is an **Adapter** — see Integrations — and is
never called a "provider".)

### Integrations

**Adapter**:
A pluggable external-system integration — Slack, Telegram, GitHub, GitHub PR,
Linear, or Webhook. The integration type; each implements the `Adapter` trait.
_Avoid_: provider, integration (for the type)

**Adapter connection**:
A configured, credentialed installation of an Adapter (webhook secret, transport
config). Stored in the `integration_connections` table.
_Avoid_: integration connection (the table name), bare "connection"

**Binding**:
The live tie between a Task and an external subject — a GitHub PR, a Slack thread,
or a Linear issue — reached through an Adapter. Tracks the subject and whether
inbound events still route to the Task. A Task has many Bindings.
_Avoid_: task connection, connection, thread

"Integrations" is the user-facing umbrella for connected external systems; it is
not a precise domain term (the type is an Adapter, an installation is an Adapter
connection). The `integration_*` DB tables are legacy storage names for Adapter
data.

### Loops — recurring agent work

**Loop**:
A standing, named definition of recurring agent work — the umbrella over automated
Task creation. Declared in a repo (`.moatless/loops/<name>/LOOP.md`, `source =
git`) or authored via the API (`source = manual`). A Loop fires on a Schedule or a
Subscription and drives one or more Tasks. Manual one-off Tasks are created
directly and are not part of a Loop.
_Avoid_: trigger (as a noun / umbrella) — a Loop is *triggered*, but the thing is
a Loop

The rule that persists a Loop's config (`TaskRoute` / `trigger_rules`) *is* the
Loop — its fields are Loop fields, so there is no separate "Routing" term beyond
`Routing mode`. Rename debt: that rule targets a `repository_id`, but a Loop
should target a Workspace.

**Schedule**:
A Loop's time-based firing condition (cron expression + timezone).
_Avoid_: schedule trigger source, ScheduledJob

**Subscription**:
A Loop's event-based firing condition: the Loop subscribes to an Adapter's events
(a PR opened, a Slack mention, a Linear issue).
_Avoid_: trigger source, adapter-event trigger, TaskTrigger, TaskRoute

**Firing** (also **activation**):
A single occurrence that starts one cycle of a Loop. "Trigger" is only the verb
for this — an event *triggers* the Loop; there is no "trigger" noun.

**Routing mode**:
A Loop's fan-out strategy: `Ongoing` (one long-lived Task the Loop keeps feeding)
or per-subject (a fresh Task per external subject, each tied by a Binding).
_Avoid_: bare "route" / `TaskRoute` for the loop's config

### Agents

**Agent**:
The coding-agent CLI process that runs a Task inside a Sandbox — the thing doing
the work (owns the CLI Session).
_Avoid_: agent run; "agent" as a synonym for a running Task

**Agent type**:
Which agent implementation runs a Task: the `claude-code` family (`claude-code`,
`-tui`, `-cli`, `-fable`) or `codex`. Always qualified — never bare "agent".

**Agent mode**:
How the Agent runs a Turn: `plan` or `edit`.

**Agent model**:
The LLM the Agent uses (e.g. Opus, Sonnet).

**Subagent**:
A sub-conversation the Agent spawns via the Task tool, threaded onto the spawning
tool call (`parent_tool_use_id`).

### Conversation

**Message**:
One entry in a Task's conversation (`TaskMessage`): user, assistant, system,
result, or error, carrying content blocks. A Task's conversation is its ordered
Messages.
_Avoid_: chat / channel as a domain entity. Qualify the other "message" uses:
inbound/outbound message (Adapter transport), the `agent.message` event, and a
Slack channel (external, provider-scoped).

**Chat**:
The frontend view that renders a Task's Messages. A UI concept only — there is no
"chat" or "channel" entity on the wire.

**Message upsert**:
The assembled, UI-facing form of a Message (`UiMessage`) pushed to the Chat over
the live stream — a `message.upsert` or `message.deleted` event keyed by the
Message's `uuid`. Produced server-side by the one Message assembler after the
source event is persisted; UI-only, never stored in `task_events`. The streamed
form carries the persisted Message's `uuid` — it is the same Message, assembled,
not a second identity.
_Avoid_: a separate `evt-*` identity for the streamed message; treating chat
assembly as a client concern.

**Source event vs derived event**:
A **source event** (e.g. `agent.message`) is raw agent output — persisted to
`task_events`, replayed as history, and shown in the Events panel. A **derived
event** (`message.upsert` / `message.deleted`) is computed from persisted state
for the live Chat and is never itself persisted. One seam separates them.

### Access & commands

**Scope**:
The access boundary applied to Repositories, Workspaces, Tasks, Adapters, Secrets,
and Credentials: `Global`, `Team`, or `User`. `SecretScopeType` is this same Scope
applied to Secrets.
_Avoid_: conflating with `OAuth scope` (a permission grant) — always qualify that.

**Command**:
A named slash-command preset (`/command`) that seeds a Task or Turn.
_Avoid_: bare "command" for a Codex shell command or the `moat` CLI — qualify
those as "shell command" / "CLI command".

### Words that are never standalone terms

**Source** — always qualified by what it is the source of: Feedback source, Skill
source, Workspace source (git/manual), source Task. Never bare "Source".

**Kind** — a generic discriminator suffix (`RepositoryKind`, `AdapterKind`,
`SecretKind` = "which variant"). Not a domain concept; excluded from this glossary.
