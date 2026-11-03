# Moatless Vibe

The ubiquitous language for Moatless Vibe — a platform that runs coding agents
(Claude Code, Codex, Pi, mini-swe-agent) inside on-demand sandboxes against git repositories, driven
by people directly or by Loops (recurring agent work), scoped to users (or global).

This file is a glossary, not a spec. It records what each term means and which
synonyms to avoid — not how anything is implemented.

## Language

### Runtime

**Sandbox**:
The on-demand isolated container (a Kubernetes pod or Docker container) that runs
coding agents and their Scripts. Ephemeral, and identified by its own `sandbox_id`
— never by a Task's id. A Sandbox materializes exactly one Workspace on disk under
its workspace directory.
Its durable lifecycle record (desired state, stop reason, archive retention,
operation marker) belongs to the Sandbox and outlives the ephemeral container;
absence of the record means no Sandbox was ever requested.
_Avoid_: worker, environment, workspace pod

**Member** (of a Sandbox):
A Task that runs inside a Sandbox, referencing it through `tasks.sandbox_id`. A
Sandbox has one or more Members and they are peers — none owns the pod, and no
Member's stop implies another's. A Member joins at Task creation and cannot move
to another Sandbox afterwards. Every Member runs against the Sandbox's one
Workspace; a join for a Task on another Workspace is refused.
_Avoid_: lead task, host task, guest, owner

**Checkout mode**:
How a Member's files relate to its co-members': `shared` (the same directory and
git state) or `isolated` (its own checkout inside the same pod, reusing the warm
caches but nothing else). Chosen when the Task is created.
_Avoid_: isolation level, workspace mode

### Workspace & repositories

**Repository**:
A connection to a git repo — its name, remote URL, default branch, and Git host.
Run-configuration lives on the Workspace, not here. A Task references a Workspace,
never a Repository directly; a Workspace references Repositories through
WorkspaceRepo placements.
_Avoid_: putting run-config on a Repository

**Workspace**:
The composition of zero or more repositories plus run-configuration (docker
image, setup commands, env vars, scripts, resources, language servers) that a
Task runs against. One that places none (`"placeRepository": false` in git,
`kind = blank`) runs its Task with nothing cloned and no repository on the Task.
A Sandbox materializes it on disk under the workspace directory. Every
Workspace is declared in git and reconciled from its repository (`source =
git`); a legacy `manual` row is only listed, deleted, or reclaimed by the sync.
Not the Sandbox (the pod that runs it) nor the Workspace panel (a task-page UI
view).

**Default Workspace**:
The Workspace with key `default` that every registered repository has, with
the repository alone as primary. An entry with `"key": "default"` in its
`.moatless/workspaces.json` overrides its settings.

**Identity grant**:
A deployment's `WORKSPACE_IDENTITY_GRANTS` entry that lets one repository's
Workspace keep the admin-only `sandboxServiceAccount`, `networkProfile` or
`nodeSelector` its git entry declares, and optionally makes it private to one
user. An ungranted value is dropped with a sync warning.

**Org sync**:
The backend job that registers every repository of a configured GitHub
organization as a global repository, and soft-deletes one archived or gone.

**Script**:
A named command (`ScriptConfig`: name, command, optional label/icon/port/envVars/
runOnStart) a Workspace declares, run as a terminal session inside a Task's
Sandbox. A Script with a `port` earns a preview URL, published on the fly under
Kubernetes; Docker never publishes it, so a Docker Task's Script has no preview.
_Avoid_: server, preview server

**WorkspaceRepo**:
A single repository's placement inside a Workspace — its branch override, mount
name (subdir under the workspace directory), primary flag, and ordering. Exactly
one placement is primary once the Workspace holds any (the repo used for diffs,
PRs, and AGENTS.md).

**Repo checkout**:
A WorkspaceRepo placement resolved for provisioning — `clone_url` taken from the
repository row, `mount_name` settled to a single safe path segment. What the
backend hands a Sandbox in the `WORKSPACE_REPOS` env var, one entry per checkout,
primary first, whose directory is the sandbox's checkout. Distinct from
**WorkspaceRepo** (the placement) and **RepoPlacement** (the resolved read under
Workspace ports), both of which leave `mount_name` open. `RepoCheckout` in
`moatless_workspace_types::workspace`.

**Task repository**:
A repository a Task works in: each WorkspaceRepo of its Workspace when it was
created (`workspace`), and each one cloned with `moat repos clone` since
(`cloned`). Read by T3's pull request reach. `task_repositories`, in
`moatless_tasks::task::repositories`.

**Workspace panel** (also **Workspace view**):
The non-chat side of the task page — the pane that switches between the preview,
the file explorer, and the sandbox diagnostics for a Task. A UI concept only,
distinct from the Workspace entity; always carry the `panel`/`view` suffix
to keep them apart.

### Tasks & lifecycle

**Task**:
The central unit of work: one conversation with a coding agent, run against a
Workspace inside a Sandbox. Owns its status (open / closed / error), visibility,
cost, and ownership. Created directly by a person or automatically by a Loop.
Loop-created Tasks are attributed to the Loop's run-as user and do not
automatically add anyone to their personal task feed.
_Avoid_: agent run (a legacy name for a running Task)

**Turn**:
One user-message-and-agent-response cycle within a Task — the authoritative,
persisted lifecycle unit (`task_turns`). A Task has many Turns.

**Run**:
The in-container harness name for a single Turn: the agent invocation the actor
mints a `run_id` for on each idle→running transition. One Run = one Turn — `Turn`
is the product term, `Run` the harness-internal twin.

**Session**:
The agent CLI's resumable conversation, resumed across Turns via `--resume`. A
Task has one-or-more Sessions; a Session spans many Turns. A new Session begins
only on `start_fresh`. Not a per-Turn unit — that is the Run. Claude identifies a
Session with a `session_id`; Codex calls the same thing a **thread** (`thread_id`,
resumed via `codex exec resume`) — a legitimate Codex-scoped synonym.
Pi identifies its Session with a `sessionId` and resumes its JSONL file via `--session`.
The mini-swe-agent bridge assigns a Session ID and restores native messages from
an append-only journal; the upstream `run()` entry point resets history.
The Session can span Agent process replacement and Sandbox recreation.
_Avoid_: bare "session" for the user login session — always qualify that as
"auth session". Do not confuse the Codex thread with an external Slack "thread"
(that is a Binding).

**Trajectory**:
Our own record of an Agent harness Session. One immutable event per line of the
harness session file, each with a parent pointer, the line verbatim, and its
content in a harness-neutral shape. A Fork's trajectory continues from an event
in its source's, copying nothing. Harness ids (session, item, turn) are stored as
values, never as our keys.

**Fork**:
A Task branched off another Task at one of its completed Turns, cut at one
Trajectory event (`task_forks.source_event_id`). Its conversation up to the cut
is projected from the parent's sources, not copied, and its first Turn resumes
a Session that continues the parent's Trajectory from that event. Recorded as a
`forked_from` relation (`task_forks` row), not `created_by`: a Fork appears
under its parent everywhere a child does, but the parent's close cascade and
error notifications skip it. Runs in its own Sandbox or, with `sameSandbox`, in the parent's on a
shared checkout.
_Avoid_: "child" for a Fork when the distinction from a `created_by` subtask
matters — a Fork is a parent-edged Task, not a spawned subagent Task.
A `created_by` subtask that joins its parent's Sandbox also gets a `task_forks`
row, cut at the parent's head, so its first Turn opens a Session of its own
holding the parent's context. It is still not a Fork: it has no `forked_from`
edge and inherits no projected conversation.

**Artifact**:
A single self-contained HTML page that an agent publishes from a Task with
`moat artifact publish`. Stored in object storage, so it outlives the Sandbox
and the Task's pod. A public Artifact, the default, is readable by any signed-in
user whatever the Task's visibility; one published with `--private` only by
whoever can read the Task. Each publish under the same slug adds an immutable
version; the visibility covers all of them, and a publish without `--private`
keeps it. Whoever can change the Task can switch it from T3's Artifacts panel.
Its link opens the latest version, which a browser shows below a bar for its
versions, visibility and Task. A version published with `--inline` is also
shown in the Task's thread as the page itself.
_Avoid_: asset (an uploaded attachment, not something the agent publishes)

**Side chat**:
A Fork opened beside its main Task to answer a question while that Task keeps
working. It shares the main Task's checkout, inherits its last stable conversation,
and carries `side_chat: true` on the `forked_from` relation. It closes or is deleted
with its parent; an ordinary Fork has an independent lifecycle. Its completed
turns can be merged back as context for the main Task's next message.

**Artifact fork**:
A copy of one Artifact version, published as version 1 of a private Artifact
on a new Task: a Fork of the Artifact's Task for a reader who can read it, or
an empty Task in a Workspace the reader picks. It names the version it was
copied from and keeps none of its records.
_Avoid_: remix

**Artifact record**:
JSON saved under a key in a Record collection; on an Artifact, its own
collection, kept across its versions. A save under an existing key replaces it. Keys are `/`-separated, and a list is the records
under one prefix: an append saves under a new `{list}/{id}`. A page whose latest
publish set `--data` saves records as the reader viewing it, through a page
token the serve route writes into the page. With `--notify`, each write a
reader makes is also an inbox item for the Task, from the `artifact` adapter,
naming the record's key.
A record's `writer` is `agent` when a user who can change the Task wrote it,
and `page` when a reader did. Without record rules, every reader can list,
replace and delete the others' records. Record rules, set per key prefix at
publish, say which of read, create, update and delete a page may do, as
anyone or as the record's owner (its `submitted_by`, which an update keeps),
and which writes notify the Task; agent writes skip them.
_Avoid_: submission, artifact data (the records as a whole, not one of them)

**Record collection**:
A named set of Artifact records. Each Artifact owns one. A Task keeps the
others with `moat records` in a Memory scope it reaches, so the Tasks of one
Loop or Workspace add rows that outlive each of them. A publish with
`--collection <scope>:<name>` lets its page read one, read-only, by a reader
who reaches that scope. Records, not Memory entries, are for many uniform rows
a page or a later Task queries; Memory is for prose an agent reads.
_Avoid_: table, dataset

**Memory**:
What agents keep for later Tasks through `moat memory`: decisions, conventions,
facts that took effort to establish, and the state of long-running work. Stored
outside the Sandbox and read by every Task that reaches an entry's Memory scope.
A Task's prompt lists entry summaries only when its Loop's LOOP.md names them
under `memory.preload`.
_Avoid_: knowledge base, notes, context (the agent's conversation)

**Memory entry**:
One keyed item of Memory: a one-line summary, a body, optional JSON data, tags,
and citations naming where each fact came from. Each write bumps its version and
records an event. It expires 28 days after its last put or verify unless written
with no expiry.

**Memory scope**:
Who shares a Memory entry or a Record collection: `task`, `loop`, `workspace`,
`repository`, `user` or `global`. The calling Task supplies every scope id, so a request names only the
kind. A request reaches the one scope it names, or the Task's Loop, else the
Task, when it names none.

**Claim** (Memory):
A Task's time-limited hold on a Memory entry, so two Tasks do not work the same
item. It lapses at its deadline or when the holding Task is no longer open.
_Avoid_: lock, lease

### Providers — always qualified

"Provider" is never a domain term on its own: five unrelated concepts share the
word, so it must always be qualified.

**Sandbox runtime**:
The backend that runs Sandboxes — Kubernetes or Docker.
_Avoid_: sandbox provider

**Provider token**:
A stored OAuth or PAT credential for an external service (e.g. a GitHub token).
"Provider" appears here only as a qualifier of "token", never alone.

**Git host**:
The service hosting a Repository's git remote — GitHub, GitLab, or Forgejo.
_Avoid_: git provider, bare "provider"

**Decision provider**:
The external model the backend asks for a Decision, chosen by
`DECISION_PROVIDER` (TypeSafe Jev today). Its credentials stay in the backend.
_Avoid_: classifier, bare "provider"

(The external-system integration is an **Adapter** — see Integrations — and is
never called a "provider".)

### Integrations

**Adapter**:
A pluggable external-system integration — Slack, GitHub, GitHub PR, Linear,
or Webhook. The integration type; each implements the `Adapter` trait.
_Avoid_: provider, integration (for the type)

**Adapter connection**:
A configured, credentialed installation of an Adapter (webhook secret, transport
config). Stored in the `integration_connections` table.
_Avoid_: integration connection (the table name), bare "connection"

**Adapter App**:
Installation-wide credentials and configuration that let an Adapter connect to
its external system. An Adapter App is identified by Adapter kind and App key;
many Adapter connections may use it.
_Avoid_: provider settings, integration App, bare "App"

**GitHub App**:
A machine identity registered at GitHub, holding a signing key this installation
stores, from which short-lived tokens are minted per account it is installed on.
One is configured by the deployment and used for signing in; the rest are
registered by admins so work can run as a bot. Referenced by its App key — the
short stable name chosen at registration, which an Adapter connection and a Bot
user both point at.
_Avoid_: bare "app", integration, GitHub App connection. Never a bare "key" for
the identifier — that already means a Secret's name and an API key.

**Bot user**:
A user representing a GitHub App's bot identity, which work is attributed to. It
never signs in and holds no personal credential; its usable token is minted from
its App on demand, and its name and address are what commits, pull requests and
comments carry. Running a person's Task, it holds no more than that person's own
role on the repository, and never administration.
_Avoid_: service account, machine user, bare "bot"

**Binding**:
The live tie between a Task and an external subject — a GitHub PR, a Slack thread,
or a Linear issue — reached through an Adapter. Tracks the subject and whether
inbound events still route to the Task. A Task has many Bindings.
_Note_: a Binding records its Task's role on the subject — creator, follower,
contributor or commenter — and each Adapter maps its own actions onto those
roles. A commenter hears only what its Adapter marks as answering its writing.
_Avoid_: task connection, connection, thread

**Task Room**:
A conversation space an Adapter opens for one Task and binds it to for the
Task's life — on Slack, a code channel. A Loop declares it with `room` in
`LOOP.md`; every message in the room reaches the Task, and the Task's replies go
there first.
_Avoid_: code channel (Slack's name for its implementation), session

**Sender**:
The external system on the other end of an Adapter, in its role of delivering an
inbound event to us — Slack, Linear, GitHub, or an arbitrary client calling the
Webhook Adapter. Used where the point is the retry contract ("the sender is
acknowledged", "the sender's own delivery identifier") rather than the
integration type.
_Avoid_: provider

**Adapter delivery**:
One inbound delivery the backend has taken durable ownership of, persisted in the
`adapter_delivery` table inside the request handler — before the sender is
acknowledged — and executed later by the supervised `adapter-delivery-worker`.
Keyed by the Adapter, the kind, and the sender's own delivery identifier
*together*, so a redelivery is a duplicate rather than a second record.

**Cardinality**: one sender delivery is one *or more* Adapter deliveries, one per
kind it fans into — GitHub activity on an open pull request is ordinarily two.
The sender's own interface counts that same event as one; the mismatch is the
stated cost of the name and is not a defect. Nothing orders the records derived
from one delivery, and nothing may be built on their execution order.

Best-effort side effects — outbound activity notifications, API-key usage
timestamps — are deliberately not Adapter deliveries.
_Avoid_: adapter work (the previous name), adapter queue, adapter inbox, adapter
outbox, adapter job, bare "delivery"

**Why not the alternatives** (recorded so the name is not re-litigated — most
were unavailable rather than merely less good):

| Candidate | Verdict |
|---|---|
| Adapter delivery | Chosen. Names what the record is for, at the cardinality cost stated above. |
| Adapter work | The previous name, and the residual one — correct at the element level but the least informative word available, kept only because everything more specific was blocked. |
| Adapter task | Blocked. Task is the core domain noun. |
| Adapter action | Blocked. Senders' own payloads key on an `action`, on more than one Adapter. |
| Adapter job | Blocked by this glossary (see Schedule), and no more specific than "work". |
| Adapter queue, Adapter inbox | Container nouns where an element noun is needed — one delivery becomes *several records*, which is a sentence "several queues" is not. "Queue" would also advertise an ordering guarantee the claim path does not make; "inbox" holds one row per received message, and this holds one row per effect derived from one. |
| Adapter effect | Accurate and unblocked, but alien to the rest of the vocabulary. The only real runner-up. |

"Integrations" is the user-facing umbrella for connected external systems; it is
not a precise domain term (the type is an Adapter, an installation is an Adapter
connection). The `integration_*` DB tables are legacy storage names for Adapter
data.

### Loops — recurring agent work

**Loop**:
A standing, named definition of recurring agent work — the umbrella over automated
Task creation. Declared in a repo (`.moatless/loops/<name>/LOOP.md`), and synced
when that repo's default branch moves or an admin runs `sync-config`, or
authored directly. A Loop fires on a Schedule or a Subscription and drives one or more
Tasks. Manual one-off Tasks are created directly and are not part of a Loop.
_Avoid_: trigger (as a noun / umbrella) — a Loop is *triggered*, but the thing is
a Loop

**Loop configuration source**:
The authority for a Loop's editable configuration: `git` when reconciled from a
repository, or `manual` when authored or overridden locally. Distinct from its
firing condition, which is a Schedule or Subscription.
_Avoid_: bare source, Loop source

The config that persists a Loop *is* the Loop — its fields are Loop fields, so
there is no separate "Routing" term beyond `Routing mode`. As of ticket 03 this
is reflected in code: the whole domain lives in one `loops/` module, the config
is `LoopConfig` (API) / `LoopSpawnConfig` (runtime), and the public
`Trigger`/`TaskRoute` types and the API `rule` field are gone (the wire keeps the
`rule` JSON key via `#[serde(rename)]` for alias compatibility). A Loop names the
Workspace its Tasks run in, and that Workspace's primary placement, if it has
one, is the repository they run against.

**Schedule**:
A Loop's time-based firing condition (cron expression + timezone).
_Avoid_: schedule trigger source, ScheduledJob

**Subscription**:
A Loop's event-based firing condition: the Loop subscribes to an Adapter's events
(a PR opened, a Slack mention, a Linear issue).
_Avoid_: trigger source, adapter-event trigger, TaskTrigger, TaskRoute

**Requester**:
The Moatless user an Adapter verified wrote the event that opens a Task
(`InboundMessage::requester`): a mapped Slack sender, a Linear session's
creator. Recorded as the Task's Reporter.
When the Loop runs as a bot, or its Workspace sets a run-as user, the Task is
owned by them: its commits credit them as
co-author, and its git-host token is capped by their own connection and
repository role, so a Requester who has not connected GitHub, or has no role on
the repository, is refused. Otherwise the Loop's own user keeps the Task.
_Avoid_: sender (the external writer, verified or not), `user_id` on an inbound
message (falls back to the Loop's user)

**Reporter**:
The verified person who requested a Loop-created Task's work, recorded as
`reporter_user_id` independently of its `created_by` attribution and personal
follows. Used for commit co-authorship and the default "Reported by" attribution
on Linear issues the Task files. Scheduled and unmapped events have no Reporter.

**Check-run trigger**:
A completed GitHub check run, from a trusted App and matching a `github_pr`
Subscription's `check_run` names and conclusions, that fires the Subscription
instead of PR open (`create_on: check_run`); such a Subscription may match a
whole org as `<owner>/*`. Example: a review gate's neutral
`soap-review-request` check run naming the skills to review with.
_Avoid_: review request, gate event

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
At most one Agent process is active per Member at a time, so a Sandbox runs as
many as it has Members; a Session can outlive and be resumed by a replacement
Agent process.
_Avoid_: agent run; "agent" as a synonym for a running Task

**Agent harness**:
The execution implementation that controls the Agent process: exactly
`claude-code` or `codex`. The Task's Agent profile selects it, and switching the
profile switches it between Turns; the next Turn starts a Session of the new
harness from the Trajectory. Transport, model, and mode variants are
configuration, not distinct harnesses.
_Avoid_: Agent type, treating `claude-code-tui` / `-cli` / `-fable` as harnesses

**Agent harness credential**:
A user- or installation-scoped credential consumed by an Agent harness, such as
Codex `auth.json`. It is persisted before running Sandboxes are told to refresh.
_Avoid_: provider token when the credential is for the harness itself

**Agent mode**:
How the Agent runs a Turn: `plan` or `edit`.

**Agent model**:
The LLM the Agent uses (e.g. Opus, Sonnet).

**Agent profile**:
A catalog entry a Task runs, such as `claude-code-tui` or `codex`: an Agent
harness plus its transport, default Agent model and credentials. The catalog
maps each profile to its harness.
_Avoid_: Agent type

**Subagent**:
A sub-conversation the Agent spawns via the Task tool, threaded onto the spawning
tool call (`parent_tool_use_id`). Its own rows are never in the Task's stream. The
Moatless surface folds them behind the spawning call (see **Subagent
transcript**); the T3 surface, which has no nesting to fold into, reports that
one call as a task it watches run — progress while the call is open, a completion
when it settles, both on the one row.

**Decision**:
One multiple-choice pick an Agent asks for with `moat decide pick`: a question,
a fixed set of options and a piece of text, answered by the Decision provider
with a choice, a confidence and a Band.
_Avoid_: classification, when naming the API

**Band**:
What to do with a Decision: `act`, `confirm` or `escalate`, from the Decision
provider's own confidence thresholds.

### Plugins & skills

**Plugin**:
A git repository of agent capabilities — skills, commands, agents — registered
once — by an admin, or by the deployment's Helm value `plugins.repositories`
(`source = 'deployment'`, read-only to the admin API, and disabled rather
than deleted when the value drops it) — and thereafter
available to everyone. A Plugin is a container,
not a thing an agent is given: what reaches an agent is always Skills. One word
through the whole stack, which is where a synonym like "marketplace" would
otherwise creep in: the table is `plugins`, the DAO is `PluginDao`, the API is
`/api/v1/plugins`, and the admin UI is Plugins.
_Avoid_: plugin repo, plugin_repo, marketplace

**Skill**:
One capability an agent can be given — a `SKILL.md` and the files beside it.
Identified by its directory name, which is what `plugin_skills` stores and what
an Activation points at; the `name:` in its frontmatter is display metadata only.

A Skill is always qualified by where it came from, because both sources reach the
same agent through the same list and `SkillSource` (`crates/api-schemas/src/skill.rs`)
is already on the wire:

- **Plugin skill** — sourced from a registered Plugin. Delivered to whoever the
  Activations say, in whichever Workspaces they do not exclude it from.
- **Repository skill** — sourced from one Repository's own `.claude/skills` and
  `.agents/skills` trees, which are listed and merged; a skill exposed in both
  (typically a `.claude/skills` symlink into `.agents/skills`) resolves to one
  entry, `.claude/skills` winning (`backend/src/skill/fetcher.rs`,
  `WORKSPACE_SKILL_PREFIXES`). Delivered to work in that Repository and nowhere
  else.

`GET /api/v1/repositories/{id}/skills` returns both merged
(`backend/src/skill/service.rs::list_for_repository`): the Repository's own skills
shadow a Plugin skill of the same name, and among plugins the first in DAO order
wins. The plugin half is per-caller, not per-repository — two people looking at
the same Repository can see different Skills, because that is what will reach
their respective sandboxes — and `?workspaceId=` narrows it further to what that
Workspace loads. "Skill" unqualified means either; write the qualifier whenever
the source is what makes the sentence true.

A Skill reaches an agent as a name, never as text on the wire. An Inbox item's
`delivery_options.skills` carries what one send activates, `tasks.skills` what
every fresh agent session on the Task re-activates (a Loop's `skills`), and the
backend unions the two onto `PendingAgentDelivery.skills`. Only the pod resolves
the body: it reads the `SKILL.md` from the repo's own skill roots or the
harness's installed home skills dir and prepends `Activated <root>/<name>:` plus
that body to the turn (`crates/sandbox/src/agent/skills.rs`).

_Known debt_: the wire variant for a repository skill is `SkillSource::Workspace`,
named before Repository and Workspace were separated. It means Repository. Rename
it when the surrounding rename debt is paid.

**Activation**:
One record in `plugin_activations` saying a Plugin, or one Skill inside it, is on
or off, for one of three **reaches** — everyone (no subject column, admins only),
one person (`user_id`), or one Workspace (`workspace_id`). A row names at most one
subject; the `plugin_activations_one_subject` constraint is what makes that true.
Each reach beats the one below it in both directions — Workspace, then personal,
then everyone, the same order run configuration and credentials resolve in — so a
Workspace can add a Skill nobody turned on and keep out one everybody has, and a
person cannot re-admit what their Workspace excluded. Clearing a record hands the
subject back to the next reach down, which is not the same as setting it off.
Nothing is delivered without a record.

Writing one takes the reach's own authority: an admin for everyone's, your own or
an admin acting for you for a person's, and write to that one Workspace — its
owner, or an admin — for a Workspace's. Write and not read, because a global
Workspace reads to everybody signed in while its records decide what loads for
everybody working in it. A git-declared Workspace's own records come from the
`skills` key of its `.moatless/workspaces.json` entry when it has one, which then
replaces them on every sync. Without the key the sync leaves them alone, so
deleting it keeps git's last records; commit `"skills": {}` first to clear them.

Only Plugin Skills have Activations. A Repository's own Skills always load in
work on that Repository; there is no record that withholds them.

An Activation decides whether a Plugin Skill is *available* at all. It is not what
puts a Skill into a turn — that is the `skills` list above, and the pod's
"Activated …" line names that, not this record.
_Avoid_: scope (means tenancy elsewhere in this codebase), link, enable flag

**Marketplace**:
Claude Code's own `marketplace.json` manifest format, which a Plugin repository
may use to declare where its plugins live. The word is theirs, not ours — it is
allowed only in the code that parses that file (`backend/src/skill/fetcher.rs`,
`crates/sandbox/src/init/plugin_skills.rs`, `crates/sandbox/src/agent/options.rs`) and never
names one of our own concepts.

### Conversation

**Message**:
One entry in a Task's conversation (`TaskMessage`): user, assistant, system,
result, or error, carrying content blocks. A Task's conversation is its ordered
Messages, which the Projector writes to `conversation_messages` from their
sources: a user Message from its Inbox item, an agent Message from Trajectory
lines, an error or turn result from a `task_events` row.
_Avoid_: chat / channel as a domain entity. Qualify the other "message" uses:
inbound/outbound message (Adapter transport), a Trajectory line, and a Slack
channel (external, provider-scoped).

**Chat**:
The UI view that renders a Task's Messages, read from the projected
conversation, never from a source directly. A UI concept only — there is no
"chat" or "channel" entity on the wire.

**Inbox**:
Every message addressed to the Agent (`task_inbox`), whoever sent it: a person,
another Task, an Adapter, the platform, or the Sandbox waking its own Agent.
Its `origin` fixes which sender columns it carries. An item is `pending` until
a Turn delivers it, and what the Chat shows is the item as sent, never the
CLI's copy of it.
_Avoid_: reading a prompt back from the Trajectory

**Projector**:
The only writer of the conversation read model (`conversation_messages`,
`conversation_activities`, `conversation_usage`), in
`crates/tasks/src/conversation/`. It projects each source in the transaction
that writes it, and `rebuild` replays the sources to the same rows and ids. Rows
copied from history recorded before it are `frozen`: `rebuild` keeps them as
they are.

**Message upsert**:
The assembled, UI-facing form of a Message (`UiMessage`) pushed to the Chat over
the live stream — a `message.upsert` event keyed by a Message's `uuid`. Produced
server-side by the one Message assembler after the Projector commits the rows it
changed; never stored in `task_events`. The Linear Adapter reads it too. A
derived upsert targets the **affected parent** Message: when the changed row
folds into another (a `tool_result`/system row inlined into the assistant
Message that called it), the upsert carries the **parent** assistant Message's
`uuid`, so a top-level tool result renders live under its call. The streamed
form carries the persisted Message's `uuid` — it is the same Message, assembled,
not a second identity.
_Avoid_: a separate `evt-*` identity for the streamed message; treating chat
assembly as a client concern.

**Source vs derived event**:
A **source** is what the Projector writes the conversation from: a Trajectory
line, an inbox item, or a lifecycle event in `task_events`. A **derived event**
(`message.upsert` / `message.subagentProgress`) is computed from the projected
rows and is never itself persisted. Message content reaches clients only via the
derived upsert (top-level) or the on-demand subagent transcript endpoint.

**Sandbox event send buffer**:
The Sandbox's in-memory queue between an event producer and the single drainer
that publishes to the backend (`crates/sandbox/src/outbox.rs`). Ordered, non-durable
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
_Avoid_: calling it an outbox in writing; describing lifecycle events as
undroppable; treating a graceful flush as a delivery guarantee (it is bounded
and best-effort).

**Subagent transcript**:
Lazy. The top-level Messages read carries only a small per-`Agent`-call
**summary** (`{ messageCount, tokens, status }` on the `Agent` content block),
and a `message.subagentProgress` event keeps it live on the wire — the collapsed
`Agent` card renders from that alone. The full transcript is assembled
server-side but **fetched on demand** from
`GET …/tool-calls/{tool_use_id}/messages` when the card is expanded; it is never
nested into the parent Message nor streamed on the wire. Its source is the
subagent's own Trajectory session, projected under the spawning call's
`parent_tool_use_id`.

### Access & commands

**Scope**:
The access boundary applied to Repositories, Workspaces, Tasks, Adapters, Secrets,
and Credentials: `Global` or `User` (two values only). `Global` is readable by every
authenticated user and written by admins; `User` is readable and written by its
owner (admins bypass both). Ownership is `COALESCE(owner_user_id, created_by)`. There
are two grants: **read** and **write** (messages, turns, start, stop, restart,
redeploy, file writes, Scripts, task edits). A Task with `visibility = 'public'` grants both to
every signed-in user. A few actions are **owner-only** (the owner or an admin, never
widened by visibility): deleting a Task, cleaning up its Sandbox, changing its
visibility, its run-as and ownership fields, and the interactive terminal. Secrets use
this same Scope.
_Note_: `Team` is **not** a scope — teams are an org/entity concept and grant no
resource access (removed in the access-scope redesign).
_Avoid_: conflating with `OAuth scope` (a permission grant) — always qualify that.

**Driver**:
The person whose message started a Sandbox's current turn
(`sandboxes.driver_user_id`; the owner while unset). The Sandbox acts
for them: its credentials resolve for the driver unless a Workspace or Task run-as
is pinned, in which case the driver is the person it acts on behalf of, and its
task tokens are minted for them alone, so a new driver revokes the previous one's.
A Task's, the scheduler's or a Loop's prompt changes no driver, and nor does running
a Script: it runs with the current driver's credentials.
_Avoid_: actor (the identity credentials resolve for, which a run-as can make
someone else), sender

**Follow**:
A user's durable interest in a Task (`task_follows`), acquired by creating it,
writing to it, or asking explicitly, and dropped by asking. It is what a personal
sidebar listing is built from in T3 Code.
_Note_: a Follow **narrows** a listing and never widens one — it grants no access.
Scope still applies underneath, because a Follow outlives the access that produced
it: a Task can be made private or change hands long after someone wrote to it.
_Avoid_: treating admin as exempt. An admin's sidebar is what they Follow, like
everyone else's; admin bypasses Scope, not Follow.

**Command**:
A named slash-command preset (`/command`) that seeds a Task or Turn.
_Avoid_: bare "command" for a Codex shell command or the `moat` CLI — qualify
those as "shell command" / "CLI command".

### T3 Code — always qualified

T3 Code is a separate client (`soaplabs/t3code`) being adapted to run against this
backend through the `t3code` surface. Its vocabulary overlaps ours imperfectly, so
these terms are **only** used with the "T3" qualifier. Full mapping:
[docs/t3-code-domain-mapping.md](./docs/t3-code-domain-mapping.md).

**T3 Thread**:
A single agent conversation in T3 Code — the direct equivalent of our **Task**.
Say "Task" for anything on this side of the wire.
_Avoid_: bare "thread" for a conversation — in this codebase a thread is a channel
thread (`channel_threads`, `external_thread_id`, `slack_thread`), and the
collision is live in the adapter subsystem.

**T3 Project**:
A single filesystem path (one `workspace_root`) that T3 Code runs threads against.
Mapped to our **Workspace** (`projectId ↔ workspaces.id`) because a Task already
references a `workspace_id` — but it is not a synonym: a T3 Project composes no
repositories and carries no run-config. Note that T3 treats its `workspaceRoot` as
an identity, unique across active projects, so the adapter synthesizes one per
Workspace rather than reporting the sandbox workspace directory.
_Avoid_: treating T3 Project and Workspace as synonyms, or renaming either to the
other before T3 Code can compose repositories.

**T3 Environment**:
One connected T3 Code server, as seen by a T3 client. Assigned client-side; no
server table records it. T3 Code federates over *machines a person owns*, where we
federate over *access scope* — the two are different axes, not two spellings of
multi-tenancy. The adapter presents this backend as exactly one T3 Environment.
_Avoid_: reading it as our deployment environment, or as a Sandbox.

**t3code** (also **T3 compatibility surface**):
The opt-in `crates/t3code` crate that serves T3 Code's contract (discovery,
session bootstrap, `/api/orchestration/shell`, and the `/ws` Effect-RPC envelopes)
from Moatless rows. An anti-corruption layer: T3 names appear on the wire, Moatless
names everywhere behind it; root-crate adapters live under `backend/src/t3code`.
_Avoid_: leaking T3 vocabulary past this surface.

### Seams & ports

**Port**:
A narrow trait a domain publishes so a peer crate can name what it offers without
naming its store. Ports are agent nouns (`SandboxIntentReader`, `UserDisplay`).
The owning domain implements the port and names it for what it offers, never for
what a caller does with it. A store — a `*Dao` and its rows — is private to its
crate.
_Avoid_: calling a `*Dao` a port, or a port a service.

**Identity ports**:
The seams `identity` publishes. **UserDisplay** — how a user is named to a
human or an external directory (display name, email). **UserResolver** — a user
reference (the internal uuid or a legacy provider id) resolved to the canonical
internal id. **CommitIdentity** — the git author identity an actor commits as,
and whether it holds platform admin. **HarnessCredentials** — the stored access
tokens an agent harness runs with. **ProviderIdentityLink** — the user behind an
external provider account, by the provider's id, its handle, or the address it
reports, plus the link back. **AppCredentials** — the deployment-wide OAuth
credential an Adapter authenticates its own app with, read and rotated.
**SecretResolver** — the secret values an identity can reach, delivered as
environment variables. **SecretCatalog** — which secrets exist for a caller, by name
alone, with nothing decrypted. **CodexScopeReader** — the Codex scopes a
background refresh should sweep. **GitHostTokenIssuer** — the one way to a
git-host token: an `Actor` — the identity the call is made as — plus a
**GitHostAccount** selecting a GitHub owner, a Forgejo instance, or GitLab.
**GitCredentialOverrides** — the identity-owned interface through
which Workspaces supply pinned credentials.
_Note_: each secret read walks `user > global` and marks the records it used, so
the audit trail is part of the read.
_Note_: `GitHostTokenIssuer` never falls back. When an `Actor` holds no usable
credential the answer is `NoCredential`, not another identity's token.
_Note_: `CommitIdentity` reports an id that names no user (the `system`
sentinel, a `bot:` value) as an error, not as an absent user — the two are
different facts.
_Note_: `ProviderIdentityLink` answers from two places, and the distinction is
real. `user_at` reads the directory link a person establishes; `user_who_connected`
and `user_with_login` read the account behind a stored credential.

**Workspace ports**:
The three seams `workspaces` publishes. **WorkspaceResolver** — a Workspace's run
configuration (`ResolvedWorkspace`), the repositories it places
(`RepoPlacement`, primary first, each with its repository row resolved), the
plugins an identity has turned on (`PluginDelivery`), Workspace names by id, one
whole Workspace as a caller may see it, and the live Workspaces that place a
given Repository (`for_repository`, primary placements first, oldest first) —
the fallback a Loop firing takes when it names none.
**WorkspaceCredentialReader** — the provider credentials pinned to a Workspace,
decrypted and keyed by environment variable. **RepositoryReader** — one
repository as a caller may see it, and display names by id.
_Note_: `WorkspaceResolver` and `WorkspaceCredentialReader` resolve across
owners. The caller's own authorization decided the Workspace may be run before
it reached the port, so neither takes a Scope. The two `describe` reads are the
exception — `WorkspaceResolver::describe` and `RepositoryReader::describe`. Each
answers a request a person just made, so each takes a Scope and answers a row the
caller may not see exactly as one that does not exist.

**Task ports**:
The eight seams `tasks` publishes. **TaskReader** — the Task facts a peer reads
(`TaskDescriptor`: id, status, name, owner, workspace, who started it), scoped;
a Task the caller may not read reads as absent. **TranscriptAppend** — what was said on a Task and
which external events it produced. **TurnCounter** — the Turn a row appended now
belongs to. **TagResolver** — tag names to tag ids, creating what is missing.
**TaskOpener** — opening a Task from the provider path: a **NewTask** in, a
**TaskOpened** (its id, and whether its sandbox started) or a **TaskOpenError**
out. **MessageAcceptor** — a turn arriving from outside the product for a Task
that exists, as one **InboundTurn** value. **TaskFinisher** — closing a Task
from the provider path, with a **CloseReason** the task band forwards without
reading. **RelationRecorder** — recording that one Task relates to another.
_Note_: `TaskOpener` is the provider-path entry, not a wrapper over the whole of
task creation. The backend's own HTTP path keeps calling the inherent method,
which is generic over two closures and therefore not dyn-safe.
_Note_: `MessageAcceptor` is distinct from `TranscriptAppend`, which writes one
row and runs no flow. A turn accepted here is deduplicated, persisted, and
handed to the agent.
_Note_: `TaskFinisher::close` answers alike for a Task that does not exist and a
Task the caller may not write, so a close never discloses that a Task is there.
_Note_: a peer writes a transcript row as a `TranscriptEntry`, never as a
`TaskMessage`. The two kinds are an **AssistantReply** authored in the backend
and an **OutboundStub** recording what an Adapter sent.

**Activity ports**:
The seam the Task band publishes over what an agent did inside a Task. An
**Activity** is one recorded step — a tool call, a status line, a degraded
Sandbox, a question handed back to a person, that question answered, the plan,
or a context-window report. **ActivityKind** is that closed set: it grows by a
code change, never by a migration, and a row of an unknown kind is skipped
rather than failing the conversation around it. **ActivityStatus** is how far a
call got. **ActivityView** is the read-side meaning of one row, which a client
adapter translates into its own wire shape instead of re-reading the storage
rules. **ActivitySource** is the port: the tail since a cursor, the rows one
batch of Messages produced, the rows a set of ids names, and the single call a
`tool_use_id` names.
_Note_: the vocabulary lives in `moatless-ports`, below both the crate
that stores the rows and the read models that project them, so a projection
reads activities without naming the Task crate.

**Binding ports**:
The two seams `loops` publishes over the Binding store. **BindingProbe** —
whether a subject is bound at all, for an Adapter that needs the predicate and
none of the rows. **TaskBindings** (in `moatless-ports`) — the Bindings a
Task carries, each labelled with the Loop that owns it, and stop-listening; the
Task band sits below `loops` so its edge goes through `backend-core`.
_Note_: an Adapter that drives the Binding lifecycle — upserts as well as reads
and status writes — holds `Bindings`, the Binding operation `loops` publishes,
instead. A narrow read port beside a handle the same crate already holds is a
second surface, not a seam.

**Loop ports**:
The seam `loops` publishes over the Loop store. **LoopReader** — the
Subscriptions an inbound provider event is matched against: by id, by an
Adapter's source matcher, or the Adapter's wildcard. An Adapter never writes a
Subscription and never reads a Schedule, a receipt, or a Loop's config rows.
_Note_: a Subscription read by id answers whatever its state, because a Binding
whose Subscription was deleted keeps working. `None` means "no Subscription
context", never "stop".

**Adapter app ports**:
The seam the Adapter kernel publishes over the stored deployment-wide app rows.
**AdapterAppReader** — the enabled app of one kind under one `app_key`, as an
**AdapterAppSecrets**: the app key, the decrypted secrets by name, and the
non-secret settings. Each Adapter reads the fields it needs out of that value
into its own shape, so the crate that stores the rows names no provider.
_Note_: `DEFAULT_APP_KEY` is the app a connection gets when its transport config
names none.

**Codex blob refresh**:
How a stale Codex credential is renewed across a seam. The store
(`ProviderTokenDao::refresh_codex_blob`) owns the row, the advisory lock, the
compare-and-swap, and the failure marker. **CodexBlobRefresher** is the policy
half the caller supplies: is this blob stale, and what replaces it.
_Note_: a lock another replica holds, and a compare-and-swap another writer won,
both report that writer's value rather than failing.

**Store error**:
The error every port returns (`StoreError`, in `moatless-types`, re-exported by
`moatless_ports::store`). It carries
a human context plus an optional boxed source, so a store driver stays out of a
caller's dependency list.
_Avoid_: returning `sqlx::Error` across a crate seam.

### Words that are never standalone terms

**Source** — always qualified by what it is the source of: Feedback source, Skill
source, Workspace source (git/manual), source Task. Never bare "Source".

**Kind** — a generic discriminator suffix (`RepositoryKind`, `AdapterKind`,
`SecretKind` = "which variant"). Not a domain concept; excluded from this glossary.
