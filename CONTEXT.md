# Moatless Vibe

The ubiquitous language for Moatless Vibe — a platform that runs coding agents
(Claude Code, Codex) inside on-demand sandboxes against git repositories, driven
by people directly or by Loops (recurring agent work), scoped to users (or global).

This file is a glossary, not a spec. It records what each term means and which
synonyms to avoid — not how anything is implemented.

## Language

### Runtime

**Sandbox**:
The per-task, on-demand isolated container (a Kubernetes pod or Docker container)
that runs the coding agent and its servers. Exactly one per Task; ephemeral.
A Sandbox materializes a Workspace on disk under its workspace directory.
Its durable lifecycle record (desired state, stop reason, archive retention,
operation marker) belongs to the Task and outlives the ephemeral container;
absence of the record means no Sandbox was ever requested.
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
setup commands, env vars, servers, resources) that a Task runs against.
A Sandbox materializes it on disk under the workspace directory. `source` is
`manual` (created via the API) or `git` (reconciled from a repo's
`.moatless/workspaces.json`). Not the Sandbox (the pod that runs it) nor the
Workspace panel (a task-page UI view).

**Override** (of a Workspace):
Flipping a git-declared Workspace to `source = manual` so it can be edited from
the UI/CLI, keeping the provenance (`source_repository_id`, `source_key`,
`source_config_path`) that makes a later reset possible. The automatic sync on
task start respects an override; the explicit syncs (a repo's `sync-config`, a
workspace's `reset-git`) reclaim it, replacing the local edits.
_Avoid_: fork, detach

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
The Session can span Agent process replacement and Sandbox recreation.
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

**Adapter work**:
One unit of provider webhook work that cannot safely finish on the HTTP response
path, persisted in the `adapter_work` table before the provider is acknowledged
and executed later by the supervised `adapter-work-worker`. Keyed by the
*provider's* event identifier, so a re-delivered webhook is a duplicate rather
than a second unit of work. Best-effort side effects — outbound activity
notifications, API-key usage timestamps — are deliberately not Adapter work.
_Avoid_: adapter queue, adapter outbox (the transactional outbox is the durable
event-ingress path, not this one), adapter job

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

The config that persists a Loop *is* the Loop — its fields are Loop fields, so
there is no separate "Routing" term beyond `Routing mode`. As of ticket 03 this
is reflected in code: the whole domain lives in one `loops/` module, the config
is `LoopConfig` (API) / `LoopSpawnConfig` (runtime), and the public
`Trigger`/`TaskRoute` types and the API `rule` field are gone (the wire keeps the
`rule` JSON key via `#[serde(rename)]` for alias compatibility).
Rename debt still open: the config targets a `repository_id` but a Loop should
target a Workspace; the DB tables keep their legacy names (`triggers`,
`trigger_rules`, `*_trigger_sources`) behind the store, so the `LoopRecord`
row structs and the adapter firing-path's matched-`SubscriptionMatch` binding
still read as `trigger`/`rule` locally — reworked in tickets 04/05.

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
The coding-agent CLI process controlled by the Agent harness inside a Sandbox.
At most one Agent process is active in a Sandbox at a time; a Session can outlive
and be resumed by a replacement Agent process.
_Avoid_: agent run; "agent" as a synonym for a running Task

**Agent harness**:
The execution implementation that controls the Agent process, such as
`claude-code` or `codex`. It is selected when the Task is created, fixed for that
Task's lifetime, and instantiated inside its Sandbox; transport, model, and mode
variants are configuration, not distinct harnesses.
_Avoid_: Agent type, treating `claude-code-tui` / `-cli` / `-fable` as harnesses

**Agent mode**:
How the Agent runs a Turn: `plan` or `edit`.

**Agent model**:
The LLM the Agent uses (e.g. Opus, Sonnet).

**Subagent**:
A sub-conversation the Agent spawns via the Task tool, threaded onto the spawning
tool call (`parent_tool_use_id`).

### Plugins & skills

**Plugin**:
A git repository of agent capabilities — skills, commands, agents — registered
once by an admin and thereafter available to everyone. One word, everywhere: the
table is `plugins`, the DAO is `PluginRepository`, the API is `/api/v1/plugins`,
and the admin page is Plugins.
_Avoid_: plugin repo, plugin_repo, marketplace

**Skill**:
One capability inside a Plugin — a `SKILL.md` and the files beside it. Identified
by its directory name, which is what `plugin_skills` stores and what an
Activation points at; the `name:` in its frontmatter is display metadata only. A
Plugin is delivered whole, minus the skills nobody picked.

**Activation**:
One record in `plugin_activations` saying a Plugin, or one Skill inside it, is on
or off — for everyone (`user_id IS NULL`, admins only) or for one person. A
personal record beats a global one in both directions; clearing it hands the
person back to the default, which is not the same as setting it off. Nothing is
delivered without a record.
_Avoid_: scope (means tenancy elsewhere in this codebase), link, enable flag

**Marketplace**:
Claude Code's own `marketplace.json` manifest format, which a Plugin repository
may use to declare where its plugins live. The word is theirs, not ours — it is
allowed only in the code that parses that file (`backend/src/skill/fetcher.rs`,
`sandbox/src/init/plugin_skills.rs`, `sandbox/src/agent/options.rs`) and never
names one of our own concepts.

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
the live stream — a `message.upsert` event keyed by a Message's `uuid`. Produced
server-side by the one Message assembler after the source event is persisted;
UI-only, never stored in `task_events`. A derived upsert targets the **affected
parent** Message: when the changed row folds into another (a `tool_result`/system
row inlined into the assistant Message that called it), the upsert carries the
**parent** assistant Message's `uuid`, so a top-level tool result renders live
under its call. The streamed form carries the persisted Message's `uuid` — it is
the same Message, assembled, not a second identity.
_Avoid_: a separate `evt-*` identity for the streamed message; treating chat
assembly as a client concern.

**Source event vs derived event**:
A **source event** (e.g. `agent.message`) is raw agent output. Its full content
is a **transient ingestion payload**: the Sandbox publishes it to the backend,
which spools it in the **transactional ingress outbox** before acknowledging;
the event-handler consumer reads it once to build the Message store and deletes
the spooled record, and it is never persisted to `task_events` nor relayed to
clients in full. Its
persisted-and-relayed form is a **metadata skeleton** (`{ message: { role },
uuid, blockCount, toolNames }`). A **derived event** (`message.upsert` /
`message.subagentProgress`) is computed from persisted state for the live Chat
and is never itself persisted. Message content reaches clients only via the
derived upsert (top-level) or the on-demand subagent transcript endpoint. One
seam separates them.

**Transactional ingress outbox**:
The durable spool the backend writes an admitted dispatch payload to before it
answers the Sandbox's publish, plus the supervised relay that re-publishes an
undelivered record until its consumer commits it (ADR-0006). It holds exactly
one payload kind — full `agent.message` content, which `task_events` keeps only
as a skeleton — and a record lives only from admission to that commit. This is
the **only** thing the phrase "transactional outbox" names here.
_Avoid_: calling the Sandbox's in-memory event queue a transactional outbox (it
is a local send buffer, and losing it loses events); calling commit-before-fan-out
ordering a transactional outbox.

**Sandbox event send buffer**:
The Sandbox's in-memory queue between an event producer and the single drainer
that publishes to the backend (`sandbox/src/outbox.rs`). Ordered, non-durable
across process death, and **bounded where the producer submits**, so a stalled
drainer cannot let it grow. At capacity the oldest non-lifecycle event is
evicted; when only **lifecycle** events (`taskTurn.*`, terminal `agent.*`)
remain, the newest submission is **refused** rather than admitted — the class is
preferred, not exempt. Refusing the newest keeps the older prefix that makes
later lifecycle events interpretable, and leaves a stale turn the reaper can
close — though only in K8s mode with the idle reaper enabled; in Docker mode no
reaper runs and that turn stays open. Four things lose an event and each is
counted and announced separately on the stream: capacity eviction, refusal at
lifecycle saturation, a terminal backend rejection (`INVALID_ARGUMENT`), and an
event type the wire cannot represent.
_Avoid_: calling it an outbox in prose; describing lifecycle events as
undroppable; treating a graceful flush as a delivery guarantee (it is bounded
and best-effort).

**Subagent transcript**:
Lazy. The top-level Messages read carries only a small per-`Agent`-call
**summary** (`{ messageCount, tokens, status }` on the `Agent` content block),
and a `message.subagentProgress` event keeps it live on the wire — the collapsed
`Agent` card renders from that alone. The full transcript is assembled
server-side but **fetched on demand** from
`GET …/tool-calls/{tool_use_id}/messages` when the card is expanded; it is never
nested into the parent Message nor streamed on the wire.

### Access & commands

**Scope**:
The access boundary applied to Repositories, Workspaces, Tasks, Adapters, Secrets,
and Credentials: `Global` or `User` (two values only). `Global` is readable by every
authenticated user and controlled by admins; `User` is readable and controlled by its
owner (admins bypass both). Ownership is `COALESCE(owner_user_id, created_by)`. A Task
additionally readable when its `visibility = 'public'` — read/interact only, never
control. Secrets use this same Scope.
_Note_: `Team` is **not** a scope — teams are an org/entity concept and grant no
resource access (removed in the access-scope redesign, ADR 0006).
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
