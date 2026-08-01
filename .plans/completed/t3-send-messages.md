# Sending messages to Tasks from the T3 Code UI

> **Status:** shipped
> **Updated:** 2026-08-01
> **Vocabulary:** CONTEXT.md

## Current solution

The T3 compatibility surface (`backend/src/ui_rpc/`) is an anti-corruption layer mounted when
`T3_UI_RPC_ENABLED` is set (`backend/src/ui_rpc/mod.rs:177`). It serves
`/.well-known/t3/environment`, `/api/auth/session`, `/api/orchestration/shell` and `/ws`, and it
projects Moatless rows into T3's vocabulary — a T3 Thread is a Moatless Task, a T3 Project is a
Workspace, a T3 Environment is one connected server.

**It is read-only.** Everything a person can do in the T3 UI that would change state either fails or
is unreachable. Two independent blockers produce that, and both were reproduced live in a sandbox
against a shadow backend before this spec was written.

### Blocker one — the composer is disabled because no provider is published

`server_config()` returns a hardcoded empty provider list:

```rust
// backend/src/ui_rpc/mod.rs:1817
"providers": [],
```

`server.refreshProviders` returns the same (`backend/src/ui_rpc/mod.rs:1304-1306`), and
`server.getSettings` returns `{}` (`backend/src/ui_rpc/mod.rs:1303`).

On the client, an empty provider list disables sending outright:

- `t3code/apps/web/src/components/chat/ChatComposer.tsx:856` —
  `const noProviderAvailable = selectedProviderEntry === undefined;`
- `t3code/apps/web/src/components/chat/ChatComposer.tsx:2568` — `providerAvailable: !noProviderAvailable`
- `t3code/apps/web/src/components/chat/ChatComposer.tsx:2781` — placeholder
  `"Enable a provider in Settings to send a message"`
- `t3code/apps/web/src/components/ChatView.tsx:4502` — `if (!sendCtx?.providerAvailable) return;`

Confirmed by browser screenshot: the composer renders greyed out with that placeholder.

### Blocker two — the write command is not implemented

`handle_request`'s tag match (`backend/src/ui_rpc/mod.rs:1300`) implements read methods only:
`server.probe`, `server.getConfig`, `server.getSettings`, `server.refreshProviders`,
`server.discoverSourceControl`, `preview.list`, `projects.listEntries`, `projects.readFile`,
`cloud.getRelayClientStatus`, `orchestration.getArchivedShellSnapshot`,
`orchestration.replayEvents`, `orchestration.subscribeShell`, `orchestration.subscribeThread`,
`assets.createUrl`, `server.reportClientActivity`, `subscribeServerConfig`,
`subscribeServerLifecycle`, `subscribeAuthAccess`.

`orchestration.dispatchCommand` is absent, so it falls through to the unknown-tag arm and returns an
`Exit/Failure` with a `Die` cause (`failure_exit`, `backend/src/ui_rpc/mod.rs:1864`). Confirmed by a
live RPC probe over `/ws`.

### What the projection already does

- `provider_instance()` (`backend/src/ui_rpc/projection.rs:966`) already derives an instance id from
  the Task's harness:
  ```rust
  fn provider_instance(conversation: &Conversation) -> &'static str {
      match agent_kind(conversation) {
          AgentKind::ClaudeCode => "claudeAgent",
          AgentKind::Codex => "codex",
      }
  }
  ```
  So Threads already claim to belong to instances that the provider list does not contain.
- `model_name()` (`backend/src/ui_rpc/projection.rs:920`) resolves a Thread's model in four steps.
- `agent_kind()` (`backend/src/ui_rpc/projection.rs:953`) maps `tasks.agent_type` to `AgentKind`.
- Runtime mode is a constant `"full-access"` (`backend/src/ui_rpc/projection.rs:53`).
- `thread()` (`backend/src/ui_rpc/projection.rs:205`) sets `hasPendingApprovals: false` (line 215),
  derives `hasPendingUserInput` from `conversation.pending_question` (line 217) and
  `hasActionableProposedPlan` from `conversation.pending_plan` (line 221).
- `thread_detail` leaves `proposedPlans` and `checkpoints` empty
  (`backend/src/ui_rpc/projection.rs:236-258`) — so no client control that depends on them is ever
  raised.
- A user Message's id on the wire is `task_messages.uuid` (`backend/src/ui_rpc/projection.rs:458`).
- `session_status()` (`backend/src/ui_rpc/projection.rs:840`) folds the Sandbox's desired state and
  the newest Turn into the T3 session status.

### How the Thread feed pushes changes

`ThreadFeed` (`backend/src/ui_rpc/feed.rs:382`) polls a `ThreadRevision` fingerprint and resends the
**whole snapshot** when it moves; the client replaces its copy wholesale. A newly persisted Message
moves that fingerprint. **Nothing new has to push a sent message back to the client.**

The sidebar uses a different mechanism: `task_listing`, a sequence-based change log.

### How Moatless sends a message today

`POST /api/v1/tasks/{task_id}/messages` → `send_message` (`backend/src/message/handlers.rs:30`):

1. Access check via `state.tasks.find_by_id(&task_id, &auth.scope_context())`
   (`backend/src/message/handlers.rs:36-42`).
2. Auto-follow — best-effort, never blocks the send
   (`backend/src/message/handlers.rs:44-54`).
3. Tool-response branch: `send::send_tool_response_flow` when `tool_use_id` **and** `tool_name` are
   present (`backend/src/message/handlers.rs:60-71`); it returns no message uuid.
4. Otherwise `send::send_message_flow` (`backend/src/message/handlers.rs:74-90`).
5. `crate::sandbox::notify::ensure_sandbox_and_notify` — on failure,
   `send::persist_error_result` writes the failure into the conversation
   (`backend/src/message/handlers.rs:94-99`).

`send_message_flow` (`backend/src/message/send.rs:37`) has this signature:

```rust
pub async fn send_message_flow(
    state: &AppState,
    task_id: &str,
    message: &str,
    model: Option<&str>,
    agent_mode: Option<&AgentMode>,
    file_ids: &[String],
    contexts: &[MessageContextInput],
    resume: bool,
    external_event_id: Option<&str>,
    external_adapter: Option<&str>,
    external_subject_key: Option<&str>,
    external_source_name: Option<&str>,
    source_task_id: Option<&str>,
    start_fresh: bool,
    sent_by: Option<&str>,
) -> Result<Option<String>, MessageError>
```

It refuses closed Tasks with `MessageError::TaskClosed` (`backend/src/message/send.rs:60-70`), dedups
by `external_event_id`, mints `let msg_uuid = uuid::Uuid::new_v4().to_string();`
(`backend/src/message/send.rs:124`), persists with `turn_number: None`, and updates
`tasks.last_message_at` and `tasks.model`.

`ensure_sandbox_and_notify` (`backend/src/sandbox/notify.rs:40`) refuses to provision for a closed
Task (`backend/src/sandbox/notify.rs:49`) — the single backstop against resurrection.

### How Moatless creates a Task today

`create_task` (`backend/src/task/handlers.rs:45`) resolves a workspace to its primary repository
(lines 58-88), validates repository access (lines 91-103), validates the name, resolves scope
(line 113), fast-fails Codex Tasks with no usable credential, and builds a `NewTaskRequest`
(`backend/src/task/creation.rs:101`).

`TaskCreationService::create` (`backend/src/task/creation.rs:244`):

- mints `let task_id = uuid::Uuid::new_v4().to_string();` (`backend/src/task/creation.rs:254`),
- generates a unique `short_id` (`backend/src/task/creation.rs:255`),
- provisions **unconditionally** at `self.starter.start(&task_id)`
  (`backend/src/task/creation.rs:353`) — best-effort, only *initiated*, so the call returns fast,
- sends the first message at `backend/src/task/creation.rs:369` when `req.first_message` is `Some`.

### What is immutable, and what is dead

- **The harness is fixed at creation.** `TaskPatch` (`backend/src/task/dao.rs:873`) carries
  `status, name, description, last_message_at, total_cost_usd, total_cost_usd_increment, site_url,
  visibility, short_id, model, append_system_prompt, ttl_minutes, inactive_ttl_days,
  disabled_servers` — there is **no `agent_type`**. No path changes a Task's agent.
- **Models are per-message.** `task_messages.model` is written per send; `tasks.model` is the Task's
  current default.
- **`thread.create` as a standalone command is currently unreachable.** T3 only issues it from a
  proposed plan (`t3code/apps/web/src/components/ChatView.tsx:5276`), and proposed plans are never
  projected (`backend/src/ui_rpc/projection.rs:236-258`).
- **Approval and checkpoint commands are unreachable.** No control that raises them is ever
  projected.
- **Settle and snooze are already advertised as unsupported** through
  `ExecutionEnvironmentCapabilities` (`t3code/packages/contracts/src/environment.ts:40-55`); a
  missing capability means the client never sends the command.

### How T3 decides which model actually runs

`t3code/packages/contracts/src/model.ts:178-200` defines `MODEL_SLUG_ALIASES_BY_PROVIDER`. For
`claudeAgent` it rewrites `opus → claude-opus-5`, `sonnet → claude-sonnet-5`,
`haiku → claude-haiku-4-5` (among others). The `codex` alias table maps `gpt-5-codex → gpt-5.4`,
`5.4 → gpt-5.4`, `5.3 → gpt-5.3-codex`, `gpt-5.3 → gpt-5.3-codex`, `5.3-spark`/`gpt-5.3-spark →
gpt-5.3-codex-spark`; **none of Moatless's Codex ids are rewritten.**

`t3code/packages/shared/src/model.ts:235-291`:

- `normalizeModelSlug(model, provider)` applies those aliases to the **selection**.
- `resolveSelectableModel(provider, model, options)` tries three things against the published
  `options`, in order: an **exact** slug match, a case-insensitive `name` match, then the selection
  rewritten through the aliases. It returns `null` when none of the three hits. The published list
  is *not* normalized, so the aliases only ever move a selection *towards* a canonical slug.

`t3code/apps/web/src/modelSelection.ts:239-258` —
`resolveAppModelSelectionForInstance` falls back to `options.find(isDefault)?.slug` when
`resolveSelectableModel` returns `null`. **This is the silent-wrong-model path**: publishing `opus`
means T3 normalizes a stored `opus` selection to `claude-opus-5`, finds no match, and silently runs
the default instead.

### T3's local draft

A new chat in T3 lives entirely in the browser until send. `isLocalDraftThread`
(`t3code/apps/web/src/components/ChatView.tsx:1449`) gates it; the draft carries project, model,
branch and text. On send the client dispatches `thread.turn.start` with a `bootstrap.createThread`
block (`t3code/apps/web/src/components/ChatView.tsx:4772-4790`) — create and first message in one
frame.

T3 mints both ids as UUIDv4 (`t3code/apps/web/src/lib/utils.ts:39-43`: `newThreadId`, `newDraftId`,
`newMessageId`) and keys its own state on them:

- After creating, it waits for a **server** Thread carrying the id it minted
  (`waitForStartedServerThread`, `t3code/apps/web/src/components/ChatView.tsx:5320`).
- It retires its optimistic user message only when a server row arrives with the id it minted
  (`t3code/apps/web/src/components/ChatView.tsx:3808`:
  `optimisticUserMessages.filter((m) => serverIds.has(m.id))`).

Before every send it calls `persistThreadSettingsForNextTurn`
(`t3code/apps/web/src/components/ChatView.tsx:3355-3424`), which dispatches `thread.meta.update` and
then `thread.runtime-mode.set` / `thread.interaction-mode.set` only when they differ from the
Thread's current values. The send itself is at
`t3code/apps/web/src/components/ChatView.tsx:4740-4820`.

### The compatibility module's own seam

`ui_rpc` deliberately does **not** hold `AppState`. It takes a `UiRpcDependencies` struct of opaque
handles (`backend/src/ui_rpc/mod.rs:237-250`): `workspaces: Arc<dyn WorkspaceDao>`,
`conversations: Arc<dyn ConversationDao>`, `sandboxes: Arc<dyn SandboxConnector>`,
`model_defaults: AgentModelDefaults`. `SandboxConnector`
(`backend/src/sandbox/access.rs:48`) is the established pattern for "something behind this module
that is not a database read".

## Planned solution

**New**

- `backend/src/ui_rpc/commands.rs` — decodes `orchestration.dispatchCommand` payloads, decides
  accept / accept-and-ignore / refuse, and calls the write ports.
- `backend/src/ui_rpc/providers.rs` — builds the `ServerProvider` list from the Moatless agent
  catalogue, and owns model-name translation in both directions.
- `ThreadCommandSink` and `AgentCatalogSource` — two write/read ports on `UiRpcDependencies`,
  following the `SandboxConnector` pattern, with backend implementations over `AppState`.
- `message::service::send_user_message` and `task::service::create_task` — the two handler bodies,
  extracted so both HTTP and `ui_rpc` can call them.

**Changes materially**

- `backend/src/ui_rpc/mod.rs` — `server_config()` becomes async and publishes real providers;
  `server.refreshProviders` returns the same list; `orchestration.dispatchCommand` is implemented;
  refusals become typed `Fail` causes rather than `Die` defects.
- `backend/src/message/handlers.rs:30` — becomes a translator over
  `message::service::send_user_message`.
- `backend/src/task/handlers.rs:45` — becomes a translator over `task::service::create_task`.
- `backend/src/task/creation.rs` — `NewTaskRequest` gains `task_id: Option<String>`; provisioning at
  line 353 becomes conditional on `req.first_message.is_some()`.
- `backend/src/message/send.rs:37` — `send_message_flow` gains `message_uuid: Option<String>`.

**Untouched**

- The Thread feed and the listing feed (`backend/src/ui_rpc/feed.rs`). A sent Message moves the
  fingerprint through the existing path; no new push exists.
- `backend/src/ui_rpc/projection.rs`, apart from nothing at all — the projection is already correct
  for this feature, including `provider_instance()`.
- `SendMessageRequest` (`crates/api-schemas/src/message.rs:340`) and `CreateTaskRequest`. The
  client-minted ids are an internal parameter, not a public API field.
- `TaskStatus` (`crates/api-schemas/src/enums.rs:13`) and `SandboxDesiredState`
  (`crates/api-schemas/src/enums.rs:80`). No new state.
- The agent catalogue (`backend/src/agent/catalog.rs`) and `list_agents`
  (`backend/src/agent/service.rs:11`). Both are read as they are.
- Every existing caller of `send_message_flow` and `TaskCreationService::create` — adapters,
  triggers, the CLI, the Moatless UI. The new parameters default to today's behaviour.
- Access mode. The T3 control stays; the command is accepted and discarded.
- The sandbox, the agent harnesses, and anything downstream of `ensure_sandbox_and_notify`.

### Problem

A person looking at a Moatless Task in the T3 Code sidebar can read the conversation and cannot
answer it. The composer is greyed out with `"Enable a provider in Settings to send a message"`, and
there is no provider to enable, because the surface publishes an empty list. Even with a provider,
pressing send would produce a defect: the command that carries a turn is not implemented.

So T3 is a viewer for Moatless. Every reply, every follow-up, every interrupt means switching to the
Moatless UI — which defeats the point of having Moatless Tasks in T3 at all.

Underneath that sit two more specific problems this change has to avoid re-creating:

- **A provider list is not the same thing as an agent list.** Moatless has five catalogue agents
  across two kinds; T3 has provider instances that are a routing key and a fixed set of driver kinds
  that select an implementation and an icon. A naive mapping puts four near-identical Claude rows in
  a picker and rewrites `instanceId` on every Thread the sidebar already serves.
- **T3 rewrites model names before matching them.** Publishing Moatless's own model ids makes a
  person's chosen model silently become the default, with nothing on screen to say so.

### Behaviour

**As a person using the T3 composer against a Moatless Task**

| # | Given | When | Then |
|---|---|---|---|
| B1 | A Moatless Task using a Claude harness, open in T3 | I look at the composer | It is enabled, and the provider picker shows Claude Code and Codex |
| B2 | The same Task | I open the model picker | It lists `Default`, `Claude Opus`, `Claude Sonnet`, `Claude Haiku`, `Claude Fable` |
| B3 | The same Task | I type a message and send | The message appears immediately, is persisted as a Moatless Message, and the agent wakes and replies into the Thread |
| B4 | A message I just sent | The server's copy of it arrives in the next Thread snapshot | My optimistic copy is retired — one bubble, not two |
| B5 | A running turn | I press interrupt | The agent stops, as it does from the Moatless UI |
| B6 | An agent asking me a question | I answer in the T3 UI | The answer reaches the agent as a tool response |
| B7 | A Task whose agent has no usable credential | I open the provider picker | That provider shows as not installed, with the credential hint |
| B8 | A Claude Task | I pick a Codex model and send | The turn is refused with a message saying this Thread runs a Claude agent and cannot switch; nothing is persisted |
| B9 | A Claude Task | I pick Sonnet instead of Opus and send | The turn runs on Sonnet — model choice within an agent works per turn |
| B10 | A closed Task | I send | The turn is refused with a message saying the Task is closed; nothing is persisted and no Sandbox is woken |
| B11 | Any Task | I use the access-mode dropdown | Nothing happens and no error appears |
| B12 | Any Task | I toggle Build/Plan | The next turn carries that mode; the toggle command itself is discarded |

**As a person starting a new chat in T3 against a Moatless environment**

| # | Given | When | Then |
|---|---|---|---|
| B13 | A T3 Project backed by a Moatless Workspace | I write a first message in a draft and send | A Moatless Task is created in that Workspace under the id T3 minted, the message is its first message, and the Sandbox is provisioned |
| B14 | The same | The Task is created | T3 stays on the Thread it navigated to — the id does not change under it |
| B15 | A Workspace with no primary repository | I send a first message | The turn is refused, naming the missing repository |

**As a person on a flaky connection**

| # | Given | When | Then |
|---|---|---|---|
| B16 | A turn I sent whose reply frame I never received | The client retries the same command | One Message exists, not two, and the retry reports success |

**As a maintainer of the rest of the backend**

| # | Given | When | Then |
|---|---|---|---|
| B17 | Any existing caller of the message or task services | This change ships | Behaviour is identical — the new parameters default to what the code does today |
| B18 | A T3 command the surface does not implement | The client sends it | It comes back as a typed error naming the command, not as a server defect |

### Architecture and key flows

#### `backend/src/ui_rpc/commands.rs` (new)

- **Proposed responsibility.** Given a decoded `orchestration.dispatchCommand` payload and a viewer,
  decide what this surface does with it and produce either a `DispatchResult` or a typed refusal.
- **Proposed change.** New module. One `async fn dispatch(state, viewer, payload) -> Result<Value,
  CommandRefusal>` plus per-command handlers. The accept / accept-and-ignore / refuse table below is
  the module's whole contract.
- **Evidence.** Command payload shapes read at
  `t3code/packages/contracts/src/orchestration.ts:543-795`; the dispatch RPC's declared input and
  output at `t3code/packages/contracts/src/orchestration.ts:1364-1368`
  (`input: ClientOrchestrationCommand`, `output: DispatchResult`); the declared error type
  `OrchestrationDispatchCommandError` at
  `t3code/packages/contracts/src/orchestration.ts:1400-1406`, whose fields are
  `message: TrimmedNonEmptyString` and an optional `cause`.

#### `backend/src/ui_rpc/providers.rs` (new)

- **Proposed responsibility.** Turn the Moatless agent catalogue into T3's `ServerProvider` list, and
  translate model names in both directions.
- **Proposed change.** New module. `providers(agents: &[AgentDescriptor]) -> Vec<Value>` and
  `catalog_model_for(instance: &str, published_slug: &str) -> Option<&'static str>`.
- **Evidence.** `ServerProvider` field list at
  `t3code/packages/contracts/src/server.ts:156-192`; `ServerProviderModel` at
  `t3code/packages/contracts/src/server.ts:61-70`; `ServerProviderState` literals
  `["ready","warning","error","disabled"]` at `t3code/packages/contracts/src/server.ts:43`;
  `ServerProviderAuth` at `t3code/packages/contracts/src/server.ts:53-58`; the alias tables at
  `t3code/packages/contracts/src/model.ts:170-200`; the exact-match rule at
  `t3code/packages/shared/src/model.ts:235-291`.

#### `backend/src/ui_rpc/mod.rs` (changed)

- **Observed.** `server_config()` at `backend/src/ui_rpc/mod.rs:1809` is a synchronous `fn` returning
  a literal with `"providers": []` at line 1817. The tag match is at line 1300.
  `UiRpcState`/`UiRpcDependencies` (lines 154-175, 237-250) deliberately exclude `AppState`.
- **Proposed responsibility.** Unchanged — frame handling and translation. It gains two dependencies
  and one command tag.
- **Proposed change.**
  - `server_config` becomes `async fn server_config(state: &UiRpcState, viewer: &ScopeContext)` and
    calls `state.agents.list(viewer)`. Its three call sites (`server.getConfig`,
    `subscribeServerConfig`, and the HTTP shell bootstrap) all already have a viewer in hand.
  - `server.refreshProviders` returns `{"providers": <same list>}`.
  - A new `"orchestration.dispatchCommand"` arm delegates to `commands::dispatch` and maps a
    `CommandRefusal` through `typed_failure_exit` with tag `OrchestrationDispatchCommandError`.
  - `UiRpcDependencies` gains `commands: Arc<dyn ThreadCommandSink>` and
    `agents: Arc<dyn AgentCatalogSource>`.
- **Evidence.** `typed_failure_exit` at `backend/src/ui_rpc/mod.rs:1885`; `failure_exit` at
  `backend/src/ui_rpc/mod.rs:1864`; `viewer_scope` at `backend/src/ui_rpc/mod.rs:876`.

#### `backend/src/message/service.rs` (new) and `backend/src/message/handlers.rs` (changed)

- **Observed.** The whole policy of sending — access check, auto-follow, tool-response branch,
  sandbox wake — lives inside the HTTP handler at `backend/src/message/handlers.rs:30-99`. It is not
  callable from anywhere that is not an Axum route.
- **Proposed responsibility.** `message::service` answers "a person with this scope wants to say this
  to this Task"; `message::handlers` answers "what did this HTTP body mean".
- **Proposed change.** Move lines 36-99 into
  `message::service::send_user_message(state, scope, sender_user_id, is_agent_context, task_id,
  input) -> Result<SendOutcome, MessageError>`, where `input` carries the fields
  `SendMessageRequest` carries plus `message_uuid: Option<String>`. The handler becomes the
  translation into that call. `ui_rpc`'s `ThreadCommandSink` implementation calls the same function.
- **Evidence.** `send_message_flow` signature at `backend/src/message/send.rs:37`;
  `ensure_sandbox_and_notify` at `backend/src/sandbox/notify.rs:40`; `MessageError` variants at
  `backend/src/message/error.rs:9-29`.

#### `backend/src/task/service.rs` (new) and `backend/src/task/handlers.rs` (changed)

- **Observed.** Workspace-to-primary-repository resolution, repository access, name validation, scope
  resolution and the Codex credential fast-fail all live inside `create_task`
  (`backend/src/task/handlers.rs:45-120`).
- **Proposed responsibility.** `task::service` answers "create a Task for this person with this
  intent"; the handler answers "what did this HTTP body mean".
- **Proposed change.** Move that policy into
  `task::service::create_task(state, scope, input) -> Result<TaskCreationOutcome, TaskError>`, where
  `input` gains `task_id: Option<String>`. `ui_rpc` calls it with the client's Thread id.
- **Evidence.** `NewTaskRequest` at `backend/src/task/creation.rs:101-137`;
  `TaskCreationService::create` at `backend/src/task/creation.rs:244`; task id minted at
  `backend/src/task/creation.rs:254`; provisioning at `backend/src/task/creation.rs:353`.

#### `backend/src/task/creation.rs` and `backend/src/message/send.rs` (changed)

- **Proposed change.**
  - `NewTaskRequest` gains `pub task_id: Option<String>`; line 254 becomes
    `let task_id = req.task_id.take().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());`
  - Provisioning at line 353 runs only when `req.first_message.is_some()`.
  - `send_message_flow` gains `message_uuid: Option<String>`; line 124 becomes
    `let msg_uuid = message_uuid.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());`
- **Evidence.** `task_messages.uuid` carries a unique index —
  `backend/migrations/0001_schema.sql:182`.

#### Flow — sending into an existing Task (main success flow)

Ordering matters, three components participate, and the write crosses into the Sandbox
asynchronously.

```
Person types, picks a model, sends
  → T3 mints a message id, renders the bubble optimistically
  → dispatchCommand{thread.turn.start, threadId, message{messageId,text}, modelSelection}
  → ui_rpc::commands validates the instance against the Task's harness
  → ui_rpc::providers translates the published slug to a catalogue model id
  → message::service::send_user_message(scope, task_id, text, model, message_uuid)
      → send_message_flow persists the Message under the client's uuid
      → ensure_sandbox_and_notify wakes the Sandbox
  → dispatchCommand returns DispatchResult{sequence}
  → ThreadFeed's fingerprint has moved; next poll resends the whole snapshot
  → T3 sees its own message id among the server ids and retires its optimistic copy
```

The command returns as soon as the Message is persisted. Waking the Sandbox can take minutes; it is
not on the reply path.

#### Flow — creating a Thread and sending in one frame (second consistency boundary)

`thread.turn.start` with `bootstrap.createThread` is one command that produces two writes.

```
dispatchCommand{thread.turn.start, threadId, bootstrap.createThread{projectId,title,...}, message}
  → task::service::create_task(scope, {task_id: threadId, workspace_id: projectId, first_message})
      → creation persists the Task under the client's id
      → creation provisions (first_message is present)
      → creation sends the first message through the send callback
  → DispatchResult{sequence}
```

The Task and its first Message are created by one service call, so there is no window in which a
Task exists with no message and no Sandbox. If creation fails, nothing is persisted and the command
is refused.

#### Flow — a refused turn (material failure flow)

```
dispatchCommand{thread.turn.start, modelSelection.instanceId = "codex"}  on a Claude Task
  → ui_rpc::commands compares instanceId against agent_kind(conversation)
  → refuse before any service call
  → Exit/Failure{cause:[{_tag:"Fail", error:{_tag:"OrchestrationDispatchCommandError", message}}]}
  → the client renders `message`; the Thread is untouched
```

No write is attempted, so there is nothing to compensate.

### Data model

**No migration.** No new table, column, index or enum value.

#### Logical model

| What the feature must remember | Where it already lives | Meaning after this change |
|---|---|---|
| Which T3 provider instance a Thread belongs to | Derived from `tasks.agent_type` via `agent_kind()` → `provider_instance()` (`backend/src/ui_rpc/projection.rs:953,966`) | Unchanged, but now it names a provider that actually exists in the published list |
| Which model a turn ran with | `task_messages.model` per message; `tasks.model` as the Task's current default | Unchanged. A T3-sourced turn writes a **catalogue** model id here, never a T3 canonical slug |
| The client's message identity | `task_messages.uuid` | Newly relevant: for T3-sourced messages this is chosen by the client, not the server. It stays globally unique and stays what the projection returns |
| The client's Thread identity | `tasks.task_id` | Newly relevant: same, for Tasks created from T3 |
| A Task that exists with nothing running | The absence of a Sandbox lifecycle record | Newly reachable: previously every created Task was provisioned |
| Which agents a person can actually use | Computed per request from visible secrets and provider tokens (`backend/src/agent/service.rs:11`) | Unchanged; now also read by the provider list |

**Invariant.** `tasks.agent_type` is immutable for the lifetime of a Task. `TaskPatch`
(`backend/src/task/dao.rs:873`) has no field for it and this change does not add one.

**Identity invariant.** A caller-supplied id is used verbatim or the write fails. There is no
fallback to a server-minted id on collision — a silent remint would strand the client on a Thread or
a message it can never reconcile.

#### Physical representation

Constrained by what already exists:

- `task_messages.uuid` — `CREATE UNIQUE INDEX "task_messages_uuid_idx" ON "task_messages" ("uuid")`
  (`backend/migrations/0001_schema.sql:182`). This index is what makes a replayed command detectable
  rather than a duplicate bubble.
- `tasks.task_id` — the primary key, UUIDv4 today, the same shape T3 mints
  (`t3code/apps/web/src/lib/utils.ts:39-43`).

### Interfaces and contracts

#### External contract — the published provider list

Two `ServerProvider` entries, in this order. Serves B1, B2, B7.

| Field | `claudeAgent` | `codex` |
|---|---|---|
| `instanceId` | `"claudeAgent"` | `"codex"` |
| `driver` | `"claudeAgent"` | `"codex"` |
| `displayName` | `"Claude Code"` | `"Codex"` |
| `enabled` | `true` | `true` |
| `installed` | any Claude catalogue entry has `installed: true` | the `codex` entry has `installed: true` |
| `status` | `"ready"` when installed, else `"disabled"` | same rule |
| `auth.status` | `"authenticated"` when installed, else `"unauthenticated"` | same rule |
| `message` | the catalogue's `missing_credential_hint` when not installed | same rule |
| `version` | `null` | `null` |
| `checkedAt` | request time, RFC3339 | same |
| `showInteractionModeToggle` | `true` | `true` |
| `requiresNewThreadForModelChange` | omitted | omitted |
| `slashCommands`, `skills` | `[]` | `[]` |
| `models` | see below | see below |

`requiresNewThreadForModelChange` is deliberately **not** set. It would block every model change on a
started Thread, taking per-turn model choice with it
(`t3code/apps/web/src/components/ChatView.logic.ts:395-430`).

**Published models for `claudeAgent`** — the union of the Claude-kind catalogue entries, published
under T3's canonical slugs *and* T3's own display names:

| Published `slug` | `name` | Catalogue id |
|---|---|---|
| `claude-opus-5` | `Claude Opus 5` | `opus` |
| `claude-sonnet-5` | `Claude Sonnet 5` | `sonnet` |
| `claude-haiku-4-5` | `Claude Haiku 4.5` | `haiku` |
| `claude-fable-5` | `Claude Fable 5` | `fable` |

The slugs are T3's because T3 seeds selections in its own vocabulary —
`DEFAULT_MODEL_BY_PROVIDER` and `DEFAULT_TEXT_GENERATION_MODEL_BY_PROVIDER` hand out
`claude-sonnet-5` and `claude-haiku-4-5` (`t3code/packages/contracts/src/model.ts`) — and the alias
table maps *towards* those canonical slugs, never away, so a list published as `opus`/`sonnet` can
resolve neither the client's own seeds nor a selection carried over from a standalone T3.

The names are T3's for the same reason in the second direction: T3 renders whatever `name` a
provider sends and matches a stored selection against it case-insensitively, with no name table of
its own to fall back on. Its Claude driver names these models with their versions
(`t3code/apps/server/src/provider/Layers/ClaudeProvider.ts`), so the catalogue's version-free labels
beside a versioned slug would put two different claims about one model on one row.

**Published models for `codex`** — identity mapping; no Codex alias rewrites any Moatless id, and
the catalogue already names them with versions:

`gpt-5.6-sol`, `gpt-5.6-terra`, `gpt-5.6-luna`, `gpt-5.5`, `gpt-5.4`, `gpt-5.4-mini`,
`gpt-5.3-codex-spark` — labels from `backend/src/agent/catalog.rs:164-208`.

**The `default` sentinel is not published.** The catalogue's first entry for both agents means
*whatever this agent picks*, is resolved inside the Sandbox and is never written back; the thread
projection already refuses to report it (`projection::UNRESOLVED_MODEL_CHOICE`), so offering it in
the picker is the same contradiction seen from the other end. `isDefault` instead marks the model
the deployment actually falls back to — the `AgentModelDefaults` the projection resolves a
model-less thread with — which is what makes the picker's default row and the thread's reported
model the same claim.

Every entry sets `isCustom: false` and `capabilities: null`.

#### External contract — `orchestration.dispatchCommand`

Input `ClientOrchestrationCommand`, output `DispatchResult { sequence }`
(`t3code/packages/contracts/src/orchestration.ts:1338-1341,1364-1368`). Serves B3, B5, B6, B8, B10,
B11, B12, B13, B18.

| Command | Reachable from T3 here | Answer | Behaviour |
|---|---|---|---|
| `thread.turn.start` | Yes — the send | Accepted | Persists the Message under `message.messageId`, then wakes the agent |
| `thread.turn.start` with `bootstrap.createThread` | Yes — draft composer | Accepted | Creates the Task in the Workspace named by `projectId` under `threadId`, with the message as its first message |
| `thread.create` | Only from a proposed plan (unreachable today) | Accepted | The same creation without a first message; the Task is not provisioned |
| `thread.meta.update` | Yes — before every send | Accepted | `title` → `tasks.name`; `modelSelection.model` → `tasks.model` after translation. `branch`, `expectedBranch`, `worktreePath` are read and ignored |
| `thread.turn.interrupt` | Yes | Accepted | Stops the agent |
| `thread.session.stop` | Yes | Accepted | The same stop — Moatless draws no line between the two |
| `thread.user-input.respond` | Yes — the shell flags it | Accepted | Routed to `send_tool_response_flow` |
| `thread.runtime-mode.set` | Yes — access dropdown | Accepted, ignored | Access mode has no Moatless meaning; refusing would put an error behind a control that does nothing |
| `thread.interaction-mode.set` | Yes — Build/Plan | Accepted, ignored | The mode is carried on the turn itself and applied there |
| `thread.delete`, `thread.archive`, `thread.unarchive` | Yes | Refused | Closing a Task from T3 is its own change |
| `project.create`, `project.meta.update`, `project.delete` | Yes | Refused | A T3 Project is a Workspace, and Workspaces are not authored from here |
| `thread.approval.respond`, `thread.checkpoint.revert` | No | Refused | Approvals and checkpoints are never projected, so no control raises them |
| `thread.settle`, `thread.unsettle`, `thread.snooze`, `thread.unsnooze` | No | Refused | Already advertised as unsupported; the client never sends them |

`sequence` in `DispatchResult` is the listing sequence after the write, so a client can tell whether
the change it caused is already reflected in what it holds.

**Mode mapping.** T3 `interactionMode: "plan"` → Moatless `AgentMode::Plan`; `"default"` →
`AgentMode::Edit`. Carried on `thread.turn.start` and applied to that turn.

#### Internal interfaces — not a contract

These are implementation boundaries. Changing one later needs no spec update.

```rust
// backend/src/ui_rpc/ports.rs
#[async_trait]
pub trait AgentCatalogSource: Send + Sync {
    async fn list(&self, viewer: &ScopeContext) -> Vec<AgentDescriptor>;
}

#[async_trait]
pub trait ThreadCommandSink: Send + Sync {
    async fn send_turn(&self, viewer: &ScopeContext, turn: ThreadTurn)
        -> Result<(), CommandRefusal>;
    async fn create_thread(&self, viewer: &ScopeContext, spec: ThreadSpec)
        -> Result<(), CommandRefusal>;
    async fn update_meta(&self, viewer: &ScopeContext, meta: ThreadMeta)
        -> Result<(), CommandRefusal>;
    async fn interrupt(&self, viewer: &ScopeContext, task_id: &str)
        -> Result<(), CommandRefusal>;
    async fn respond_to_question(&self, viewer: &ScopeContext, answer: QuestionAnswer)
        -> Result<(), CommandRefusal>;
}
```

Both are implemented once in the backend over `AppState` and mocked in `ui_rpc`'s tests. They exist
because `ui_rpc` does not hold `AppState` by design (`backend/src/ui_rpc/mod.rs:237-250`) and
`SandboxConnector` (`backend/src/sandbox/access.rs:48`) is the precedent.

**Call sites with a wide blast radius**, as information:

- `send_message_flow` — every adapter (`backend/src/adapters/`), the trigger paths, the CLI-backed
  routes and `backend/src/task/creation.rs:369`. All pass `None` for the new parameter.
- `NewTaskRequest` construction — `backend/src/task/handlers.rs`, the adapter and trigger creation
  paths. All pass `task_id: None`.

### Failure, safety and recovery

| Condition | State before | State after | What the caller sees | Retry safe | Mechanism |
|---|---|---|---|---|---|
| Task is closed | Task closed | Unchanged; nothing persisted, no Sandbox woken | Typed error naming the Task as closed | Yes, no-op | `send_message_flow` refuses (`backend/src/message/send.rs:60-70`); `ensure_sandbox_and_notify` refuses to provision (`backend/src/sandbox/notify.rs:49`) |
| Same command frame arrives twice | Message already persisted | Unchanged — one Message | Success | Yes | Unique index on `task_messages.uuid` (`backend/migrations/0001_schema.sql:182`); a unique violation on the client's uuid is mapped to success |
| Same create frame arrives twice | Task already exists | Unchanged — one Task | Success, if the existing Task is visible to this viewer; refused otherwise | Yes | Primary key on `tasks.task_id`; a collision is checked against the viewer's scope before being reported as success |
| Sandbox will not start | Message persisted | Message persisted; failure written into the conversation | Success on the command; the failure reaches the person through the next Thread snapshot | Yes | `persist_error_result` (`backend/src/message/handlers.rs:97`) — identical to a send from the Moatless UI |
| Viewer cannot read the Task | Any | Unchanged | Typed error; the Task is not distinguished from one that does not exist | Yes, no-op | `find_by_id` with `viewer_scope` (`backend/src/ui_rpc/mod.rs:876`) — writes resolve the viewer exactly as reads do |
| Model belongs to the other provider instance | Any | Unchanged, nothing persisted | Typed error naming the agent this Thread runs | Yes, no-op | Compared against `agent_kind(conversation)` before any service call |
| Model slug is not one we published | Any | Unchanged, nothing persisted | Typed error naming the model | Yes, no-op | Translation returns `None` and the command is refused rather than defaulted |
| Workspace has no primary repository | Any | Nothing created | Typed error naming the missing repository | Yes, no-op | Existing check at `backend/src/task/handlers.rs:78-82` |
| Command not implemented by this surface | Any | Unchanged | Typed error naming the command | Yes, no-op | `typed_failure_exit` with tag `OrchestrationDispatchCommandError` |
| Database unavailable | Any | Nothing persisted | Typed error; cause logged, not sent | Yes | Existing `MessageError::Db` / `PersistMessage` mapping |

**No partial writes to compensate.** Sending is one insert plus a best-effort wake; creating is one
service call that either produces a Task with its first message or produces nothing.

**Nothing is retried server-side.** T3 owns the retry, and the unique index is what makes that safe.

**User-visible wording.** Not a product decision — the requirement is that the person can work out
*which* action was refused and *why*, from the `message` field alone. Final copy belongs to whoever
owns the surface's strings.

**Logging.** Refusals log at `debug` with the command tag and the viewer, matching
`file_failure_exit`'s convention (`backend/src/ui_rpc/mod.rs:1935`). Service and database failures
log at `warn`/`error` with the cause; the cause never crosses the wire.

### Testing and verification

Seams, in `/codebase-design` terms: the two new ports are the module seam `ui_rpc` is tested at —
they are what let the command table be tested without a database. `message::service` and
`task::service` are the deeper seam the HTTP handlers and `ui_rpc` share.

**Unit** — `cargo test -p moatless-vibe-backend`

- `ui_rpc::providers`: every catalogue model round-trips out to its published slug and back to its
  catalogue id.
- `ui_rpc::providers`: `claude-opus-5`, `claude-sonnet-5`, `claude-haiku-4-5` are the published
  Claude slugs — a regression here is the silent-wrong-model bug.
- `ui_rpc::providers`: an unpublished slug returns `None`, and is not resolved to the default.
- `ui_rpc::providers`: both instances are present; `installed`, `status`, `auth.status` and `message`
  follow the catalogue's `installed` flag; exactly one model per instance carries `isDefault`.
- `ui_rpc::commands`: one case per row of the command table — accepted, accepted-and-ignored, or
  refused. The table is the test.
- `ui_rpc::commands`: a `modelSelection.instanceId` that disagrees with the Task's harness is refused
  before any port call (asserted against a mock sink that records calls).
- `ui_rpc::commands`: `interactionMode: "plan"` maps to `AgentMode::Plan`, `"default"` to
  `AgentMode::Edit`.
- `task::creation`: `task_id: Some(..)` is used verbatim; `None` still mints.
- `task::creation`: provisioning is invoked when `first_message` is `Some` and not when it is `None`
  (asserted against the existing mock starter).
- `message::send`: `message_uuid: Some(..)` is used verbatim; `None` still mints.

**Integration** — `task test:integration` (this is the only tier that builds `backend/tests/`; a
plain `cargo test` compiles almost none of it)

- A `thread.turn.start` over `/ws` produces exactly one `task_messages` row whose `uuid` is the id
  the client sent.
- The same frame sent twice produces one row and two successes.
- The Thread snapshot that follows carries that message id.
- A send to a closed Task is refused and persists nothing.
- A `thread.turn.start` with `bootstrap.createThread` produces a Task under the client's `threadId`
  with one message, and provisioning is requested.
- A `thread.create` with no message produces a Task and does **not** request provisioning.
- A send by a viewer who cannot read the Task is refused.
- `server.getConfig` and `server.refreshProviders` return the same two-instance list.

**End-to-end** — `E2E_PROFILE=docker bun run test:e2e -- --suite ui`

- The composer is enabled against a Moatless Task and the provider picker lists both agents.
- Typing and sending puts the message in the thread and the agent replies into it.
- The bubble does not duplicate when the server's copy arrives.

**Manual verification** — the `t3-development` skill's loop: shadow backend on `:8080`, T3's Vite
server on `:5733` proxying `/api` and `/ws`, a real Task, and the preview URL in a browser. This is
how both blockers were found and is what confirms the fix end to end.

**What only a person can judge**

- Whether the provider picker reads as two agents rather than as five near-duplicates.
- Whether a refusal message tells someone what to do next, or only that something failed.
- Whether the composer feels responsive when the Sandbox is cold — the command returns fast by
  design, and whether that reads as "sent" or as "nothing happened" is a judgement.

**Acceptance criteria**

- **Given** a Moatless Task open in T3, **when** a person types and sends, **then** the message is
  persisted as a Moatless Message and the agent replies into the Thread.
- **Given** a Claude Task, **when** a person selects Sonnet and sends, **then** the turn runs on
  Sonnet and `task_messages.model` holds `sonnet`.
- **Given** a Claude Task, **when** a person selects a Codex model and sends, **then** the turn is
  refused and nothing is persisted.
- **Given** a T3 draft against a Moatless-backed Project, **when** a person sends a first message,
  **then** a Task exists under the id T3 minted, with that message, provisioned.
- **Given** any command this surface refuses, **when** it is dispatched, **then** the client receives
  a typed error, never a defect.

## Decisions

| # | Decision | Chosen |
|---|---|---|
| 1 | What one T3 provider instance *is* | One instance per agent **kind** — `claudeAgent` and `codex` |
| 2 | Which model names those instances publish | T3's canonical names, translated back to catalogue ids on the way in |
| 3 | Which commands the write path accepts | Seven implemented, two accepted-and-ignored, the rest refused |
| 4 | Whether Moatless adopts client-minted identifiers | Adopt both — Task id and Message id |
| 5 | Whether a Task gains a draft state | No new state; provisioning becomes conditional on a first message |
| 6 | Where the write path sits | Direct service calls; the HTTP handlers become translators |
| 7 | What a refusal looks like on the wire | A typed `Fail`, not a `Die` defect |
| 8 | Picking a model from the other instance on a running Thread | Refuse that turn |

### 1. What one T3 provider instance is

**Chosen:** one instance per `AgentKind` — two entries, `claudeAgent` and `codex`.

Moatless has five catalogue agents (`backend/src/agent/catalog.rs`): `claude-code`,
`claude-code-tui`, `claude-code-cli`, `claude-code-fable`, `codex`. Four of them are the same model
family over different transports.

**Rejected — one instance per catalogue agent.** It would put four near-identical Claude rows in the
provider picker, where a person is choosing an agent and not a transport. Worse, `provider_instance()`
(`backend/src/ui_rpc/projection.rs:966`) already returns `claudeAgent`/`codex` for every Thread the
sidebar serves; switching to per-agent instances would rewrite `instanceId` on all of them, and
`instanceId` is T3's routing key.

**Rejected — transports as models.** It would put a transport name in the model list, where a person
reads model names. `claude-code-tui` is not a model.

Consequence: the transport is not selectable from T3. A Task's harness is chosen at creation, and
T3-created Tasks get the default Claude harness for their kind.

### 2. Which model names those two instances publish

**Chosen:** T3's canonical slugs and names — `claude-opus-5`/`Claude Opus 5`,
`claude-sonnet-5`/`Claude Sonnet 5`, `claude-haiku-4-5`/`Claude Haiku 4.5`,
`claude-fable-5`/`Claude Fable 5` — translated back to catalogue ids (`opus`, `sonnet`, `haiku`,
`fable`) on the way in. Codex ids and names are published unchanged because no Codex alias rewrites
them and the catalogue already versions their labels.

**Rejected — publish our own ids.** `resolveSelectableModel` tries an exact slug match first
(`t3code/packages/shared/src/model.ts:235-291`), so a selection *we* stored would resolve. But T3
seeds selections of its own — `claude-sonnet-5` and `claude-haiku-4-5` from
`DEFAULT_MODEL_BY_PROVIDER` / `DEFAULT_TEXT_GENERATION_MODEL_BY_PROVIDER` — and a selection carried
in from a standalone T3 is in the same vocabulary. A list of `opus`/`sonnet`/`haiku` matches none of
them, and step three cannot rescue it: `MODEL_SLUG_ALIASES_BY_PROVIDER`
(`t3code/packages/contracts/src/model.ts:167-200`) maps towards the canonical slugs, never away.
`resolveAppModelSelectionForInstance` then silently substitutes the default
(`t3code/apps/web/src/modelSelection.ts:239-258`) with nothing on screen to say so.

**Rejected — publish canonical names and pass them straight through.** `tasks.model` and
`task_messages.model` would then hold names the Moatless model picker does not list, and the sandbox
would receive a model id its agent does not know.

**Rejected — a driver kind T3 does not know.** Registering something like `moatless` as the driver
would avoid the alias tables entirely, at the cost of the provider icon, the display name and the
model ordering, all of which T3 keys off the driver kind.

### 3. Which commands the write path accepts

**Chosen:** the table under *Interfaces and contracts*. Seven implemented, two accepted-and-ignored,
the rest refused.

**Rejected — implement only the turn.** The client dispatches `thread.meta.update` before every send
(`t3code/apps/web/src/components/ChatView.tsx:3355-3424`); refusing it would abort the first send of
every Thread.

**Rejected — refuse Thread creation.** It would leave T3's own "new chat" button broken against this
backend. This was reopened mid-interview and moved into scope.

**Rejected — refuse the mode-setting commands.** Access mode and the Build/Plan toggle are visible
controls. Refusing them would put an error behind a control that, from the person's point of view,
does nothing — worse than silence. Access mode is out of scope by the original request; the
interaction mode is carried on the turn itself, so the separate command is redundant rather than
meaningful.

Approvals, checkpoints, settle and snooze are refused because they are unreachable: no control that
raises them is ever projected (`backend/src/ui_rpc/projection.rs:236-258`), and settle/snooze are
already advertised as unsupported capabilities.

### 4. Whether Moatless adopts client-minted identifiers

**Chosen:** adopt both. `NewTaskRequest` and `send_message_flow` each take an optional caller-supplied
id and use it verbatim. A collision fails the write.

T3 mints the Thread id before sending anything, keeps the draft under it, and then waits for a
*server* Thread carrying that same id (`t3code/apps/web/src/components/ChatView.tsx:4772-4790,5320`).
A Moatless-minted id would leave the person on a Thread that never materializes. It retires its
optimistic user message only when a server row arrives with the id it minted
(`t3code/apps/web/src/components/ChatView.tsx:3808`). Both are UUIDv4
(`t3code/apps/web/src/lib/utils.ts:39-43`), the same shape Moatless mints.

**Rejected — adopt the Task id, mint the Message id.** Creation must adopt regardless, but the
composer's own copy of every sent message would then stay on screen beside the server's until the
Thread is reloaded.

**Rejected — translate at the boundary.** A map from T3 ids to Moatless ids means a second identity
for every Task and Message, stored, kept and reconciled, to avoid changing two lines.

Cost accepted: two service signatures grow one optional field each, and the surface has to treat a
uuid collision as an idempotent success rather than an error.

### 5. Whether a Task gains a draft state

**Chosen:** no new state. Provisioning at `backend/src/task/creation.rs:353` becomes conditional on
`req.first_message.is_some()`.

T3's draft is entirely client-side (`t3code/apps/web/src/components/ChatView.tsx:1449`); nothing
reaches this backend until send. The only command that creates without sending, `thread.create`, is
reachable only from a proposed plan, and proposed plans are never projected — so the create-then-idle
case does not currently arise at all. A Task with no Sandbox is already representable: the lifecycle
record is separate from the Task's status and its absence means no Sandbox was ever requested
(`crates/api-schemas/src/enums.rs:80`).

**Rejected — a fourth `TaskStatus`.** `TaskStatus` is a three-value text-backed enum
(`crates/api-schemas/src/enums.rs:13`) matched in the Sandbox intent computation
(`backend/src/sandbox/lifecycle/wake.rs:41`) and read by the CLI, the OpenAPI schema and the E2E
chains. A fourth value is a wide blast radius to describe a state the system can already describe.

**Rejected — create nothing until the first message.** It would break the only path that creates a
Thread without sending.

Noted for later: server-side drafts visible across clients are a real product feature. They are not
this one, they do not come from T3's model, and they deserve their own spec.

### 6. Where the write path sits

**Chosen:** direct service calls. The send and create policy moves out of the two HTTP handlers into
`message::service` and `task::service`; the handlers keep their routes and become translators.

This was an engineering call made without the user, and flagged as such on the interview page.

**Rejected — an HTTP self-call into the existing endpoints.** It inherits the policy for free, which
is its real attraction, but it puts a socket and a serialization round-trip inside a request that is
already on a socket, needs a second set of credentials to hold and rotate, and — decisive here —
would force the client-minted message id into the **public** `SendMessageRequest`
(`crates/api-schemas/src/message.rs:340`), where any API consumer could set it, purely to serve one
internal caller.

The extraction is also the move `AGENTS.md` asks for directly: when the code you are changing sits in
the wrong module, move it there first as its own commit.

### 7. What a refusal looks like on the wire

**Chosen:** a typed `Fail` cause carrying `OrchestrationDispatchCommandError`
(`t3code/packages/contracts/src/orchestration.ts:1400-1406`), built with `typed_failure_exit`
(`backend/src/ui_rpc/mod.rs:1885`). The `Die` shape stays for what it means — a bug in this server.

This was an engineering call made without the user, and flagged as such on the interview page.

**Rejected — keep the `Die` defect.** It is what an unimplemented command produces today
(`backend/src/ui_rpc/mod.rs:1864`) and it reads to the client as "this server is broken". For a closed
Task or an unsupported action that is simply untrue, and the difference on screen is between a
composer that explains itself and one that looks crashed. The same reasoning already produced
`file_failure_exit` for workspace files (`backend/src/ui_rpc/mod.rs:1898-1935`).

### 8. Picking a model from the other provider instance on a running Thread

**Chosen:** refuse that turn, with a message naming the agent the Thread runs. Changing model
*within* an agent keeps working, per turn.

A Task's harness is fixed at creation — `TaskPatch` (`backend/src/task/dao.rs:873`) has no
`agent_type` field and no path changes it.

**Rejected — `requiresNewThreadForModelChange: true`.** T3 has this flag for exactly this situation
and it refuses earlier and more gracefully, before anything is sent
(`t3code/apps/web/src/components/ChatView.logic.ts:395-430`). But it is all-or-nothing per provider:
it blocks *every* model change on a started Thread, so choosing Opus for one turn and Sonnet for the
next would go away too — and per-turn model selection is in scope.

**Rejected — run it on the Thread's own agent silently.** That is the invisible substitution
decision 2 exists to prevent.

## Out of scope

- **Access mode.** T3's Full access / Auto / Approval-required control. The command is accepted and
  discarded; the control may be hidden by whoever owns the T3 side.
- **Approvals, checkpoints, revert, settle, snooze, archive.** Unreachable or already advertised as
  unsupported.
- **Worktree preparation and setup scripts.** `bootstrap.prepareWorktree` and `runSetupScript` are
  read and ignored.
- **Project (Workspace) creation and editing from T3.**
- **Closing a Task from T3.** `thread.delete` / `archive` are refused; this is its own change.
- **Choosing the harness (transport) from T3.** A consequence of decision 1.
- **Server-side draft Tasks visible across clients.** A consequence of decision 5.
- **The never-clearing "Syncing messages…" pill.** See Open questions.

## Open questions

- **The "Syncing messages…" pill never clears** on any Thread in T3. It is pre-existing and nothing
  in this change causes it, but it sits in the same corner of the screen as the composer and will be
  noticed alongside this work. What was established while reproducing the flow: the backend does emit
  `{kind:"synchronized"}` when `requestCompletionMarker: true` is requested, and the stream stays
  open for at least 30s without churn. The cause is therefore between that frame and
  `EnvironmentThreadStatus` becoming `live` (`t3code/apps/web/src/threadSync.ts`,
  `resolveThreadSyncPhase`). Not diagnosed further; it gets its own fix.
- **The workspace's committed start command for the T3 web preview server is broken** — it runs from
  the primary repository rather than the `t3code` checkout, and that checkout's dependencies were
  never installed. A live override is in place for the sandbox this was developed in; the committed
  configuration still carries the broken command.

## Ticket breakdown

None.
