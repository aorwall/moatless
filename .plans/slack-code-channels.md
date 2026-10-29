# Task Rooms, with Slack code channels as the first

> **Status:** draft
> **Updated:** 2026-10-29
> **Vocabulary:** CONTEXT.md (proposes the term **Task Room**)
> **Source:** https://docs.slack.dev/ai/slack-code/ (and its `lifecycle`,
> `code-channel-properties`, `agent-slash-commands` pages, `/ai/agent-sessions`,
> and `/reference/methods/agents.conversations.create`)

## Goal

Give a Task its own conversation space in a chat tool, opened when a Loop creates
the Task, whatever Adapter the Loop listens on. The first case is a Slack Loop:
the room is opened from the mention. The second is the SP software factory
(`soap-agent-ops/.moatless/loops/factory/LOOP.md`). It is a `linear` Loop, and its
orchestrator Task should get a Slack code channel when the ticket enters the
factory.

The Loop config and the backend only know "this Loop's Tasks get a room on
Adapter X". Slack's code-channel vocabulary stays inside `crates/adapter-slack`.

## Vocabulary

"Session" is taken: it means the Agent harness Session (`CONTEXT.md:124`).
Slack's "agent session" and Linear's "agent session" are both provider terms.
This plan proposes:

**Task Room**: a dedicated conversation space an Adapter opens for one Task,
bound to it for the Task's life and closed when the Task closes. A Slack
code channel is the Slack Task Room. Its Binding has origin `room`.
_Avoid_: session, workroom, code channel outside `adapter-slack`.

## Slack Code in brief

| Slack surface | API | Generic Room concept |
|---|---|---|
| Create the channel | `agents.conversations.create {name, session_id, origin_channel_id?, origin_message_ts?, is_private?}` | `open` |
| Status: `processing` / `active` / `suspended` / `closed` | `agents.sessions.setStatus` | `RoomStatus` |
| Title | `agents.sessions.rename` | Task title |
| Context bar: up to 5 `{key, label, icon, url}` items, each write replacing the set | `agents.conversations.setProperties` | `RoomContext` |
| Up to 5 view tabs (`diff` / `html` / `block_kit` / `canvas`) | `agents.conversations.setView` | `RoomView` (later) |
| Per-channel slash commands | `agents.conversations.setCommands` and `slash_command_url` | `RoomCommand` (later) |
| Archive with a summary | `agents.conversations.archive {summary_message_ts}` | `close` |
| Stop button | `agent_session_stopped` event | Interrupt the agent |

Facts that shape the design:

- **`session_id`** is an idempotency key. A retry with the same value returns
  the same channel, so `session_id = task_id` makes opening a room safe to
  retry.
- **Origin.** With `origin_channel_id`/`origin_message_ts`:
  - Slack copies the origin channel's privacy.
  - It lets that channel's members join and invites the origin message's author.
  - It unfurls a card in the origin conversation.

  Without an origin, the channel is public unless `is_private` is set, only the
  bot is a member, and `name` is required.
- **Invites.** `create` has no invite list. To add people to a channel with no
  origin, or with a bot-authored origin, call `conversations.invite`. This needs
  `channels:manage` / `groups:write`. *Unverified on code channels.*
- **Detection.** `conversations.info` →
  `properties.record_channel.record_type == "agent_channel"`.

## Before phase 1

Phase 1 changed the three Slack rows below: `slack_channel` exists, a room's
channel is probed before `no_trigger`, and a room's posts go top-level. A
reply to a message in a room threads under that message's thread root.

| Anchor | What is there |
|---|---|
| `crates/loops/src/loops/firing/inbound.rs:73` | `create_task_from_loop`. **Every** Loop path creates its Task here: chat (`bindings/runtime.rs:324`), schedule (`schedule_runner.rs:285`), Linear (`adapter-linear/src/issues.rs:967`, and sessions through it), and GitHub PR (`routing.rs:724`, `check_run_trigger.rs:451`) |
| `crates/types/src/loops.rs:120` | `SubscriptionMatch` and `LoopContext` (`:27`). Generic Loop fields sit next to an opaque, per-Adapter `adapter_config` |
| `crates/server/src/adapters/operations.rs` `adapter_send` | Precedent for a Task binding itself to *another* Adapter. It resolves a connection by kind, calls `AdapterDirectDestination::post_to_destination`, then binds with `ORIGIN_CONNECTED_THREAD` and runs `AdapterPostBindingFollowup` |
| `crates/loops/src/bindings/reply.rs:439` | `select_binding_for_reply`. It matches by subject key, then by the single `adapter_event` Binding |
| `crates/adapter-slack/src/subject.rs:12` | `slack_thread`, the only Slack subject. There is no channel-wide subject |
| `crates/adapter-slack/src/ingress.rs:99` | `resolve` drops an event from a channel with no Loop as `no_trigger` before any Binding probe |
| `crates/adapter-slack/src/outbound.rs:164` | `send_reply_for_binding` always posts threaded |
| `crates/adapter-model/src/loops.rs:61` | `delivery_only_subscription`, for events delivered to existing Bindings |
| `crates/loops/src/adapters/events.rs:26` | `TaskEventSubscriber`. Linear's is the only implementation |
| `crates/server/src/t3code/backend/commands.rs:508` | `interrupt` → `task_service::stop_agent` |
| `docs/slack_manifest.yaml` | `/moat` points at `/api/v1/adapters/slack/slash`, a route that does not exist |

<!-- plan-assert: contains crates/adapter-slack/src/subject.rs const SUBJECT_KIND: &str = "slack_thread"; -->
<!-- plan-assert: contains crates/adapter-slack/src/api.rs agents.conversations.create -->
<!-- plan-assert: contains crates/loops/src/loops/firing/inbound.rs pub async fn create_task_from_loop( -->

## Target design

### Loop config: one generic `room` key

In `LOOP.md` it is camelCase, like every other Loop key. In `loops.room` it is
`jsonb`, carried on `SubscriptionMatch` and `LoopContext` as `Option<RoomSpec>`.

```yaml
room:
  via: slack                 # the Adapter that hosts the room
  connection: pc_…           # optional; resolved like `moat send --connection`
  in: channel:C0FACTORY      # optional anchor, in the hosting Adapter's destination syntax
  invite: [requester, run_as] # Moatless people, resolved to provider ids
  private: false             # optional; default follows the anchor
```

```rust
/// Where a Loop's Tasks get their Task Room.
pub struct RoomSpec {
    pub via: AdapterKind,
    pub connection_id: Option<String>,
    /// Destination string the hosting Adapter parses; `None` anchors on the
    /// triggering event when it came through the same Adapter.
    pub anchor: Option<String>,
    pub invite: Vec<String>, // "requester", "run_as", or a login
    pub private: Option<bool>,
}
```

| Loop | `room` | What happens |
|---|---|---|
| Slack channel Loop | `{via: slack}` | Anchored on the mention. Slack invites the author and lets channel members join |
| `factory` (Linear) | `{via: slack, in: "channel:<#factory>", invite: [requester], private: true}` | The bot posts a kickoff message in `#factory` and the room is created from it, so the card unfurls there. The requester is invited |
| Schedule Loop | `{via: slack, in: "channel:C…"}` | Same as the factory, without a requester |

`via` must name an Adapter that implements `AdapterTaskRoom`; one that does not
fails its `room.open` delivery permanently. `sync-config` refuses a room hosted
off the Loop's own Adapter that names no `in`, and one on it that names an `in`. `anchor` is opaque to everything
outside the hosting Adapter, the same contract as `moat send --to`.

### Capability: `AdapterTaskRoom`, in `crates/adapter-model/src/capabilities.rs`

```rust
/// A dedicated conversation space an Adapter hosts for one Task.
#[async_trait::async_trait]
pub trait AdapterTaskRoom: Adapter {
    /// Open (or return, for the same `task_id`) the Task's room. Idempotent.
    async fn open_room(&self, connection: &AdapterConnection, req: &RoomRequest)
        -> Result<RoomOpened, AdapterError>;
    async fn set_room_status(&self, connection: &AdapterConnection, room: &TaskBinding,
        status: RoomStatus) -> Result<(), AdapterError>;
    /// Replace the room's context: repository, branch, PR, Task link.
    async fn set_room_context(&self, connection: &AdapterConnection, room: &TaskBinding,
        ctx: &RoomContext) -> Result<(), AdapterError>;
    async fn close_room(&self, connection: &AdapterConnection, room: &TaskBinding,
        summary_external_id: Option<&str>) -> Result<(), AdapterError>;
}

pub struct RoomRequest {
    pub task_id: String,
    pub title: String,
    /// The triggering event's Binding destination, when it came through `via`.
    pub origin: Option<BindingDestination>,
    pub anchor: Option<String>,
    /// Provider user ids, already resolved from `RoomSpec::invite`.
    pub invitees: Vec<String>,
    pub private: Option<bool>,
    /// Kickoff text posted at the anchor when there is no origin message.
    pub kickoff: String,
}
pub enum RoomStatus { Working, Idle, WaitingOnUser, Closed }
pub struct RoomOpened { pub destination: BindingDestination, pub external_id: String }
```

Each method's call order is fixed by the generic `TaskRooms` service below. The
Slack implementation is the only place that knows `agents.conversations.*`.

### Opening: a durable `room.open` delivery

`create_task_from_loop` gains one port, `RoomRequests`. When `config.room` is
set and the Task opened, it admits a durable delivery of kind `room.open`,
keyed `task_id`, with a `RoomOpenPayload {task_id, loop_id, origin}`. It goes
through the existing `DeliveryAdmission`, not an inline Slack call:

- **Task creation never depends on Slack.** A failed or slow room is retried by
  the delivery worker, deliberately, rather than failing or blocking the Linear
  webhook that created the Task.
- **One hook covers every Loop kind.** The five callers pass the port and
  nothing else changes. The chat router passes the event's destination as
  `origin`.

The handler is a generic `TaskRooms` service in `crates/loops/src/rooms/`. It:

1. Reloads the Loop and the Task. A closed Task is a permanent no-op.
2. Resolves `invite` to provider ids through `ProviderIdentityLink`
   (`provider_id_for_login`), and skips anyone unlinked.
3. Resolves the connection the way `adapter_send` does.
4. Calls `open_room`, then binds the Task to `RoomOpened.destination` with
   the new origin `ORIGIN_ROOM` (`"room"`).
5. Records a `BindingReplySent`-style timeline entry, so the Task shows its
   room link.

Retry safety comes from Slack's `session_id = task_id` and the Binding upsert's
`ON CONFLICT`.

### Replies: the room wins for its own Adapter

`select_binding_for_reply` gains one generic rule: when the reply's Adapter has
an active `room` Binding on the Task, that Binding is the target.

- A Slack-origin Task posts into its code channel, even when it answers the
  mention in the origin thread.
- The factory orchestrator answering a Linear comment still answers in Linear,
  because the room is on a different Adapter.
- It answers a message from the room in the room.

### Inbound from a room: delivery-only routing

A room has no Loop of its own, so its events route as delivery-only, the way
GitHub PR events reach existing Bindings. In Slack this needs four changes:

- **New subject.** `slack_channel`, keyed `team:channel`, with
  `destination_config {channel, team_id}`.
- **Routing.** When `resolve_chat_provider_event` finds no Loop, `ingress::resolve`
  probes `is_bound(slack, slack_channel, team:channel)`. A bound channel routes
  with `delivery_only_subscription`, which can never create a Task.
  `resolve_connection_and_subscription` learns the delivery-only id form, as
  `flush_route` already does.
- **No mention needed.** A top-level message in a bound channel bypasses the
  trigger filter, like a tracked thread. A thread inside the room resolves to
  the channel subject too, so a thread never spawns its own Task.
- **Replies.** `send_reply_for_binding` treats `thread_ts` as optional and
  posts top-level on a `slack_channel` Binding.

So the factory orchestrator hears its room exactly as it hears its Linear
ticket: as messages on a Binding.

### Status and close: a generic `TaskRoomSubscriber`

This lives in `crates/loops/src/rooms/`, not in Slack. It is a
`TaskEventSubscriber` that, for each active `room` Binding on the event's Task,
spawns a call to the hosting Adapter:

| Event | Call |
|---|---|
| `Agent(Started)`; `Agent(Activity)`, throttled to once per 30 min | `set_room_status(Working)` |
| `Agent(Completed)`, `Agent(Error)`, `Sandbox(Error)`, `Agent(Stopped)` | `set_room_status(Idle)`, then `set_room_context` |
| `Agent(PendingResponse)`, once confirmed to mean waiting on a person | `set_room_status(WaitingOnUser)` |
| `Task(Closed)` | `close_room` with the last outbound message on that Binding, then deactivate it |

Slack maps the statuses to `processing` / `active` / `suspended` / `closed`.
Refreshing `processing` keeps it from timing out after Slack's one hour.
`RoomContext` is built generically:

- `Task.repository_id` → the repository URL
- `Task.branch`
- the Task's `github_pr` Binding → the PR URL, read through
  `BoundPullRequestSource` moved from `crates/t3code/src/ports.rs:446` into
  `crates/ports`
- the frontend Task URL

Slack picks its icons, so `RoomContext` carries no icon names.

### Stop button

`agent_session_stopped` is a Slack ingress kind (`slack.session_stopped`).
Its handler finds the `slack_channel` Binding and calls `stop_agent` through a
new `AgentInterrupt` port, mocked in the handler test. Interrupting is generic,
and only receiving the event is Slack's.

### Later: views and commands

- **`RoomView { key, kind: Diff | Html | Canvas, content }`.** The diff view
  comes from the sandbox's `GetGitDiffPreview` at turn end.
- **`RoomCommand`.** Commands registered through `setCommands` and received on
  a new signed `/api/v1/adapters/slack/commands` route: `stop`, `close`,
  `status`.

Both stay methods on `AdapterTaskRoom` with Slack-only implementations; another
Adapter returns `Unsupported`.

## The software factory

**`soap-agent-ops` PR, after phase 3:**

```yaml
# .moatless/loops/factory/LOOP.md
room:
  via: slack
  in: channel:<#factory id>   # the team's factory hub
  invite: [requester]
  private: true
```

**`factory-orchestrator` skill:**

- **Room messages.** A message from the room arrives like a ticket comment.
  The orchestrator handles it the same way: it relays to the open child with
  `moat send --task <id> --queue`, and answers in the room.
- **Milestone posts.** Spec approved, PR opened and verified go into the room
  as well as onto the ticket.

**Considerations specific to the factory:**

- **Status during child work.** The orchestrator is idle while its children
  work, so the room would read *active* during a long implement run. A later
  option is `room.status: tree`: the subscriber resolves a child's events to
  its parent's room through the Task relation and reports `Working` while any
  child runs. Phase 2 ships `self` only.
- **When to open.** The room opens when the Task is created, when the ticket
  gains `factory` or enters a factory state. Opening only at *In Progress*
  would need a trigger the Loop config cannot express. If wanted, the
  orchestrator could open it itself with `moat rooms open`, an optional
  imperative twin of `room.open` built on the same service.
- **Close.** `inactiveTtlDays: 30` and the Linear `terminal_states` already
  close the Task, which archives the room.
- **Who joins.** The requester is the Linear session's creator, or the
  ticket's trigger user. The rooms are private, so anyone else is added by a
  member or listed under `invite`.

## Phases, one PR each

1. **Room model, opening and Slack inbound.** Covers:
   - `RoomSpec` on Loops: migration, `LOOP.md` key, `sync-config` validation
     (the API schema is deferred, see `docs/deferred-changes.md`)
   - `AdapterTaskRoom`, the `room.open` delivery, `TaskRooms` and `ORIGIN_ROOM`
   - the reply rule
   - Slack: `open_room` (with and without an origin, the kickoff post,
     invites), `slack_channel`, delivery-only routing, top-level replies
   - manifest scopes: `code_channels:manage`, `channels:manage`, `groups:write`,
     and `features.code_channels`

   Done when one db-test shows a Slack mention Loop and a Linear Loop, each
   with `room: {via: slack}`, producing exactly one
   `agents.conversations.create` with `session_id = task_id` and a `room`
   Binding. An unmentioned top-level message in that channel must reach the
   same Task, and a replayed `room.open` must create nothing new.
2. **`TaskRoomSubscriber`.** Status, context, close and archive. Test the
   event-to-call table against a stub, including call counts.
3. **Stop button.**
4. **Factory adoption** in `soap-agent-ops`: the `room` key and the
   orchestrator skill.
5. **Diff view.**
6. **Room commands.**
7. **Slack-opened code channels.** A mention in an `agent_channel` with no
   Binding resolves the Loop through `origin_link`.

Each PR updates `docs/slack_manifest.yaml`, `CONTEXT.md` (**Task Room**) and the
`moat-cli` skill's `references/loops.md` (the `room` key). It records any gap it
leaves in `docs/deferred-changes.md`.

## Decisions

- **Name.** Task Room: the `room` Loop key and the `room` Binding origin.
- **Factory hub.** `#factory` anchors the factory's rooms, which are private.

## Open questions

- **Invites on code channels.** Does `conversations.invite` work on a code
  channel, and with which scope? If it doesn't, the factory room relies on
  people joining from the `#factory` card.
- **Stop semantics.** Slack's lifecycle page says Slack resets the status after
  a stop. The agent-sessions page says the app must. Test this in a workspace.
- **Access.** Code channels must be enabled for our Slack app. Until then, no
  Loop sets `room`, and nothing changes.
