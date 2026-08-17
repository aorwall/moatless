# Stop the adapters from owning task lifecycle

> **Status:** proposed
> **Updated:** 2026-08-17
> **Vocabulary:** CONTEXT.md
> **Anchored to:** `agent/run-config-to-backend-core` @ `273f1c57a`

Two changes, two pull requests, one thesis: a fact about a Task has one owner, and
everybody else either calls that owner or subscribes to what it announces.

| Part | Fixes | Mechanism | Removes |
|---|---|---|---|
| 1 | Linear re-implements the task close | Direct call | `adapter-linear -> moatless-sandbox-control` |
| 2 | The shared event consumer names one provider | Event subscription | 11 `providers::` paths from `event_handler/` |

## The third finding is already fixed

An audit of this workspace found three places where a crate wrote a domain it did not own.
The pod watcher writing `tasks.disabled_servers` was one; `e6bda13f4 feat: delete server
auto-disable`, `c2d500fdf feat: emit turn_interrupted without reading task state` and
`1abb8980b feat: drop tasks.disabled_servers` closed it on this branch.

`crates/sandbox-k8s` now names neither `moatless_tasks` nor `moatless_workspaces`, and
both are gone from its `Cargo.toml`. Nothing below depends on that; it is recorded so a
reader does not go looking for the third part.

One leftover: `crates/sandbox-k8s/src/pod_watcher.rs:97` still documents `restart_count` as
"used for crash-loop auto-disable". Fix the comment in whichever PR next touches the file.

## Decisions

| Question | Decision |
|---|---|
| Should the Linear close publish an event the sandbox band subscribes to? | **No.** `Lifecycle::remove` writes a durable `Removed` intent the reconciler recovers from. A dropped event writes no intent, so a recoverable failure becomes an invisible one. Call `TaskCloser`. |
| Where does the event-subscriber registry live? | `crates/loops/src/adapters/`, beside `AdapterDeliveryHandlers`. `loops` already depends on `tasks`, so it can name `AgentEvent`. |
| Does the GitHub PR shell-reconcile hook move too? | **Yes**, but it re-derives its own targets rather than receiving them. See Part 2. |
| Order? | 1, then 2. Part 2 is the only one needing a new seam designed. |

---

## Part 1 — Route the Linear close through `TaskCloser`

### Current solution

`close_linear_bound_task` (`crates/adapter-linear/src/close.rs:173`) performs the task
close itself, in four hand-rolled steps that duplicate `TaskCloser::close_single`
(`crates/tasks/src/task/closer.rs:109`):

| Linear | Does | `TaskCloser` equivalent |
|---|---|---|
| `mark_task_closed` :244 | `update_task(status = Closed)` | `closer.rs:130-139` |
| `remove_sandbox_after_linear_close` :269 | `ArchiveRetention::on_close(…)` + `sandbox.remove` | `closer.rs:144-145` |
| `deactivate_linear_task_bindings` :280 | `deactivate_by_task` | `closer.rs:151` |
| `publish_linear_task_closed` :286 | `publish_task_event(Closed, "claude-code", {…})` | `closer.rs:153-163` |

The last two are line-for-line equivalent: `deactivate_task_bindings`
(`crates/tasks/src/task/handlers.rs:479`) calls the same `deactivate_by_task`, and
`close_single` publishes the same `TaskSubType::Closed`.

The intended entry point already exists and already documents this caller by name:

> The enriched entry the provider webhooks use: a GitHub PR / **Linear** close carries its
> own `reason` + metadata and a fixed actor via `CloseEvent`.
> — `closer.rs:232`

`CloseEvent::enriched`'s own doc (`closer.rs:74-79`) spells the payload out:

> GitHub PR builds `{ "reason": "pr_closed", "pr_number": N }`; Linear builds
> `{ "reason": "linear_state_terminal", "issue_identifier": …, "state_name": … }`.

That is byte-for-byte what `publish_linear_task_closed:295-301` emits today, with the same
`"claude-code"` actor. `adapter-github-pr` already calls `close_with_event`
(`crates/adapter-github-pr/src/routing.rs:438`); Linear is the only provider that does not,
and the only adapter that depends on `moatless-sandbox-control`.

### The change

**`crates/adapter-linear/src/close.rs`**

1. Delete `mark_task_closed` (:244), `remove_sandbox_after_linear_close` (:269),
   `deactivate_linear_task_bindings` (:280) and `publish_linear_task_closed` (:286).
2. In `close_linear_bound_task` (:173), replace the four calls with one:

   ```rust
   let close_event = CloseEvent::enriched(
       "claude-code",
       serde_json::json!({
           "reason": "linear_state_terminal",
           "issue_identifier": issue.identifier,
           "state_name": issue.state_name(),
       }),
   );
   if let Err(e) = task_closer.close_with_event(&sys, &binding.task_id, &close_event).await {
       warn!(task_id = %binding.task_id, error = %e, "Failed to close task on Linear terminal state");
       return;
   }
   ```

   `sys` is `ScopeContext::system()`, matching `mark_task_closed:247-249` and the GitHub PR
   path. Keep the `#[allow(clippy::disallowed_methods)]` with its existing reason.

3. **`emit_linear_close_response` (:200) stays, and stays first.** It posts the
   "closing this task" comment to Linear before the state change.
4. **`task_can_be_closed` (:139) stays.** Its already-`Closed` check is what stops a
   second delivery from posting a duplicate Linear comment — `close_single`'s own guard
   fires too late for that, because the comment is emitted before the close. The two
   guards are not redundant; they protect different effects. Say so in a comment, because
   the next reader will try to delete one of them.

**Signature changes** — replace `sandbox: &Lifecycle` / `Arc<Lifecycle>` with
`task_closer: &TaskCloser` / `Arc<TaskCloser>`, and drop `config` from any frame that
carried it only for `retain_storage_on_close`:

| File | Lines |
|---|---|
| `close.rs` | 48 / 53 (`close_active_bindings_for_issue`), 173 / 177 |
| `issues.rs` | 70 (`handle_issue_update`), 120 (call) |
| `notifications.rs` | 53 (`handle_issue_status_notification`), 90 (call) |
| `delivery.rs` | 138 (struct field), 164 (`new` param) |

`LinearInboundRouter::new` keeps its arity — one port in, one port out — so its
`too_many_arguments` reason string needs only the name swapped, not the count.

**Composition root** — `crates/backend/src/adapter_installations.rs:226` and
`crates/backend/tests/common/mod.rs:680` pass `state.task_closer.clone()` instead of the
lifecycle. `AppState` already holds `Arc<TaskCloser>` (`crates/backend/src/state.rs:225`).

**`crates/adapter-linear/Cargo.toml`** — delete the `moatless-sandbox-control` line.
`ArchiveRetention` was never a `sandbox-control` type anyway; it lives at
`crates/tasks/src/task/models.rs:270` and was reached through a re-export, so no import
survives the deletion.

### Tests

- Extend the existing Linear close coverage so it asserts the published `Closed` event
  still carries actor `"claude-code"` and the three payload fields verbatim. This is the
  regression that matters: the refactor must be invisible on the wire.
- Add a case where the same terminal state arrives twice (Issue webhook, then
  `AppUserNotification` — both reach `close_active_bindings_for_issue`) and assert exactly
  one Linear comment and one `Closed` event.
- `crates/backend/tests/integration_adapters/adapter_delivery_github_linear_test.rs`
  already drives the real registered handlers against a real pool — extend it there rather
  than mocking.

### Proof

```
grep -rn 'moatless_sandbox_control' --include='*.rs' crates/adapter-linear/    # nothing
grep -n  'moatless-sandbox-control' crates/adapter-linear/Cargo.toml           # nothing
```

---

## Part 2 — Give the adapters their own event subscriptions

### Current solution

`EventHandlerConsumer` (`crates/backend/src/event_handler/consumer.rs`) is the shared
Redis-stream dispatcher. It contains 11 calls into specific adapters:

| Site | Event | Call |
|---|---|---|
| `consumer.rs:417` | `agent.message`, assistant role | `linear::activity::notify_linear_message_activities` |
| `consumer.rs:431` | `agent.message`, user role | `linear::activity::notify_linear_tool_results` |
| `consumer.rs:474` | `agent.error` | `linear::activity::notify_linear_status` |
| `consumer.rs:514` | `taskTurn.error` | `linear::activity::notify_linear_status` |
| `consumer.rs:544` | `sandbox.initializing/ready/stopped` | `linear::activity::notify_linear_action` |
| `consumer.rs:563` | `agent.started` | `linear::activity::notify_linear_action` |
| `agent_handlers.rs:289` | `agent.completed` | `linear::activity::notify_linear_status` |
| `agent_handlers.rs:989` | persisted `agent.message` | `github_pr::shell_reconcile::reconcile_shell_command` |

(Four of the eleven are the `ActivityKind` argument on an adjacent line, not separate call
sites; the eight rows above are the real dispatches.)

Because the feature sits in the dispatcher rather than in the adapter, it did not
generalise: `adapter-slack` and `adapter-telegram` react to **zero** agent events. Not by
decision — nobody edited that match arm for them.

The seam that fixes this already exists on the inbound half of the same system, and its
module doc states the goal:

> Keeping the two ends joined by a constant rather than by a shared enum means adding an
> Adapter adds a handler module and one registration line, and **never edits a match arm
> another Adapter is also editing**.
> — `crates/loops/src/adapters/delivery/handlers.rs:5`

### The change

**New** — `crates/loops/src/adapters/events.rs`, modelled on
`delivery/handlers.rs:34-54`:

```rust
/// A reaction one Adapter has to a task-lifecycle fact.
///
/// Called after the dispatcher's own durable work for that event has run, so a
/// subscriber reads committed state. Fire-and-forget: implementations log their
/// own failures and never block the consumer. A subscriber that panics must not
/// take the dispatch loop with it.
#[async_trait::async_trait]
pub trait TaskEventSubscriber: Send + Sync {
    /// The keys this subscriber wants. Read once at registration.
    fn subscribes_to(&self) -> &[EventKey];

    async fn notify(&self, event: &AgentEvent);
}

#[derive(Default)]
pub struct TaskEventSubscribers(HashMap<EventKey, Vec<Arc<dyn TaskEventSubscriber>>>);
```

`EventKey` and `AgentEvent` come from `moatless_tasks::agent_event`; `loops` already
depends on `tasks`. `EventKey` needs `Hash` — it derives `Debug, Clone, PartialEq, Eq`
today (`crates/tasks/src/agent_event.rs:15`).

**`crates/adapter-linear/src/subscriber.rs`** (new) — one struct holding the ports the
seven Linear dispatches need (`db`, `adapter_connections`, `config`, `messages`) plus
`Arc<ActiveLinearTurns>`, which is already Linear's own type
(`crates/adapter-linear/src/active_turns.rs:23`) and only lives on `AppState` because the
consumer needed to read it. Move the gate here. Add `register(subscribers, …)` mirroring
`delivery.rs:193`.

**`crates/adapter-github-pr/src/subscriber.rs`** (new) — subscribes to `agent.message` and
re-derives its own targets by calling
`moatless_tasks::message::hooks::extract_shell_reconcile_targets`
(`crates/tasks/src/message/hooks.rs:49`) rather than receiving a parsed list. The
tasks-band parse stays where the comment above `agent_handlers.rs:989` puts it; only the
dispatch moves. Ordering holds because subscribers run after `handle_agent_message`
persists.

**`crates/backend/src/adapter_installations.rs`** — add `event_subscribers(&self, state:
&AppState) -> TaskEventSubscribers` beside `delivery_handlers` (:68), with one
`register_events` line per installation, exactly as :70-74 does.

**`crates/backend/src/event_handler/`** — delete the provider branches. At the end of
`dispatch_event` (`consumer.rs:389`), after the existing match, fan out once:

```rust
self.state.event_subscribers.notify(event).await;
```

The `EventKey::Task(Open | Created | Closed | Updated)` arm keeps its `debug!`, but those
events are now genuinely available to subscribers rather than "SSE-only" — update that
comment.

**Deliberately not changed:** the consumer keeps every piece of generic work —
`project_turn_state` (:334), `persist_event` (:625), message assembly, ingress-record
release. Only provider reactions move.

### What this buys beyond the greps

Slack and Telegram can subscribe to `agent.completed` without any edit inside
`event_handler/`. That is the test of whether the seam is real, and it is worth writing
down in `crates/loops/AGENTS.md` (new file — commit the `CLAUDE.md` symlink beside it).

### Tests

- A registry unit test: two subscribers on overlapping keys both receive one event;
  a subscriber on no matching key receives nothing.
- A subscriber that panics does not stop the other subscribers or the dispatch loop.
- Move the existing Linear notification assertions from the consumer tests onto the Linear
  subscriber, where they can run without Redis.
- Keep one end-to-end case in `crates/backend/tests/integration_adapters/` proving a real
  `agent.message` still reaches Linear through the registry.

### Proof

```
grep -rn 'providers::' crates/backend/src/event_handler/    # nothing
```

---

## Verification

Run once per part, before committing. The floor plus every `-p` line naming a crate the
part touches:

```
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p moatless-core --all-targets --features openapi -- -D warnings
cargo clippy -p moatless-core --all-targets -- -D warnings
cargo clippy -p moatless-backend-core --all-targets --all-features -- -D warnings
cargo clippy -p moatless-backend-core --all-targets --no-default-features -- -D warnings
cargo clippy -p moatless-backend --all-targets --features db-tests -- -D warnings
```

Scoped tests per part:

| Part | Crates |
|---|---|
| 1 | `moatless-adapter-linear moatless-tasks moatless-backend` |
| 2 | `moatless-loops moatless-adapter-linear moatless-adapter-github-pr moatless-backend` |

**No generated artifact moves.** Neither part changes a wire schema. Run
`task generate:spec` once per part to confirm the diff is empty; a non-empty diff means
something reads a type this plan did not find.

### Commits

One commit per pull request. `commitlint.config.ts` is the authority on types and scopes;
confirm with `printf 'refactor: x\n' | bunx commitlint`. Proposed subjects:

```
refactor: close Linear-bound tasks through the shared closer
refactor: let adapters subscribe to task events
```

### End to end, on a K8s stack

1. Move a Linear issue to a terminal state on a bound task. The task closes once, the
   sandbox is removed, the Linear comment appears once, and the `Closed` event payload
   carries `reason: linear_state_terminal` with the issue identifier and state name.
2. Deliver the same transition twice (Issue webhook and AppUserNotification). Still one
   comment, still one event.
3. With Part 2 in, confirm a Linear-bound task still shows live agent activity in Linear,
   and that a Linear adapter that fails to notify leaves the consumer and every other
   adapter working.
