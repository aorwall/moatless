# Channel Architecture

The channel module connects external systems to Moatless tasks. It has one
canonical model:

```text
ProviderConnection -> TaskTrigger -> TaskRoute -> Task
                                      ^
External subject -> TaskBinding -----+
ScheduledJob -----> TaskRoute
```

Handlers are thin HTTP adapters. Services own business flows. Repositories own
SQL for one aggregate each. Provider modules own provider-specific parsing,
signature validation, transport calls, and destination JSON.

## Canonical Concepts

`provider_connections`

One configured provider account or transport boundary. Examples are a Slack
workspace, GitHub organization, Linear workspace, Telegram bot scope, or webhook
secret. Connections carry auth/transport metadata plus `scope` and `team_id` so
admin APIs can enforce access before a service uses the row.

`task_routes`

The task creation policy. A route says which repository to use, how to map
events to tasks, which prompt/template/tags/run-as user to apply, and which
scope owns the resulting tasks. Routes are provider-neutral.

Routing modes:

- `per_message`: create a new task when no existing binding matches the
  external subject.
- `thread_based`: reuse a task through the subject binding for the provider
  thread, issue, PR, or conversation.
- `ongoing`: route all matching events to one open task tracked on the route,
  creating a new ongoing task if the tracked task is missing or closed.

`task_triggers`

The provider-specific source matcher that activates a route. A trigger points at one
connection and one route, then stores the provider, source matcher, optional
event filter, trigger config, and direct-send flags. Triggers do not decide task
creation behavior directly; the route does.

`task_bindings`

The durable link from a task to one external subject. A binding is the only
outbound target model. It stores:

- `subject_kind`: provider-defined subject type, such as `slack_thread`.
- `subject_key`: stable provider-defined lookup key for that subject.
- `destination_config`: provider-defined JSON needed to send replies.
- `last_external_event_id`: cursor used to avoid redelivering known context.
- `origin`: why the binding was created, such as an inbound provider event,
  `connected_thread`, `direct_send`, `scheduled_job`, or `api`.

`scheduled_jobs`

Durable cron-style work that fires a route on a schedule. Scheduled jobs do not
own an outbound destination. They create or reuse tasks through the route and
deliver the scheduled message into the task timeline.

## Runtime Contexts

Providers do not receive database rows as their root API. Shared code builds
provider-facing contexts when crossing into provider-specific code:

- `TriggerContext`: route plus trigger/job information needed by providers for
  sending, context fetches, prompt instructions, and attribution.
- `BindingContext`: task binding information needed by providers for replies
  and context fetches.
- `ProviderConnection`: transport/auth boundary loaded when a provider needs
  connection metadata.

These contexts are synthesized values. Do not persist them and do not add
compatibility fields to make them look like old rows.

## Main Flows

### Admin CRUD

```text
HTTP handler -> IntegrationAdminService -> dao/<aggregate>.rs -> SQL
```

Handlers in `handlers/` extract auth, path/query/body values, call shared scope
helpers in `handlers/common.rs`, and map rows to API responses. They should not
contain routing policy, provider logic, or ad hoc SQL.

Each repository in `dao/` owns one aggregate:

- `connections.rs`
- `routes.rs`
- `triggers.rs`
- `trigger_routes.rs`
- `bindings.rs`
- `scheduled_jobs.rs`
- `receipts.rs`

Use scoped list/get helpers instead of fetching broad result sets and filtering
in handlers.

### Provider Event Delivery

```text
provider handler
  -> provider parser/signature checks
  -> ProviderEventDeliveryService
  -> binding_runtime::route_inbound_message
  -> create/reuse task
  -> upsert task binding
  -> message::send_message_flow
```

Provider handlers live under provider directories, for example
`slack/handlers.rs`, `telegram/handlers.rs`, `github_pr/handlers.rs`,
`linear/handlers.rs`, and `webhook/handlers.rs`.

Bindings created by provider events still persist the `origin` value `ingress`
for existing-row/API compatibility. That value does not imply the removed legacy
`ingress_rules` table.

Provider code is responsible for turning external events into an
`InboundMessage`, resolving a `ProviderConnection` and `TaskTriggerRoute`, and
building the provider-specific `subject_kind`, `subject_key`, and
`destination_config`.

The shared binding runtime owns the cross-provider behavior:

- Ignore inactive triggers.
- Use event receipts for external-event idempotency.
- Reuse existing bindings for the same provider subject.
- Attach connected threads to the originating task when the external event was
  caused by an outbound assistant message.
- Create a task when the route and trigger configuration require one.
- Upsert a binding before delivery.
- Fetch provider context for follow-up thread messages.
- Advance the binding cursor after delivery.

### Replies

```text
moat reply / POST /tasks/{task_id}/reply
  -> ReplyService
  -> outbound::send_to_binding
  -> ChannelProvider::send_reply_for_binding
  -> provider transport API
  -> assistant stub + binding cursor update
```

Replies are binding-based. If a task has exactly one binding, callers may omit
`--binding`. If it has no bindings, reply fails. If it has multiple bindings,
reply fails with target descriptors and the caller must choose one binding.

Outbound delivery persists an assistant-visible stub in the task timeline,
tags it with the provider external id when one is returned, records it in the
recently-sent cache, and advances `task_bindings.last_external_event_id`.

### Direct Sends

```text
DirectSendService
  -> outbound::direct_send_to_rule
  -> ChannelProvider::send_message
  -> optional task binding creation
```

Direct sends are allowed only when the target trigger has
`allow_direct_send = true`. When a `task_id` is supplied, the send creates a
matching binding so future replies can target the external thread.

### Scheduled Jobs

```text
cron scheduler
  -> due scheduled_jobs
  -> task_routes
  -> create/reuse task by route.routing_mode
  -> message::send_message_flow
```

Scheduled jobs use routes only. The scheduler locks due jobs, computes the next
fire time, creates or reuses a task, and sends the scheduled message into the
task. There is no provider destination unless another flow creates an explicit
binding for that task.

## Provider Boundaries

Keep provider-specific behavior inside the provider module:

- Signature and webhook parsing.
- Provider API clients and transport errors.
- Subject key format.
- Destination config shape.
- Context fetch and formatting.
- Provider-specific reply instructions.

Shared channel files should only depend on the `ChannelProvider` trait,
canonical rows, and synthesized contexts.

## Naming Conventions

Use the current model names in runtime code, API schemas, CLI flags, generated
clients, and docs:

- `binding_id`, not a legacy task-link name.
- `trigger_id`, not a legacy config id.
- `origin`, not a legacy created-reason field.
- `destination_config`, not a legacy outbound-target config field.
- `task_bindings`, not legacy task-link tables.

The removed legacy tables must not be queried or recreated:

- `channel_configs`
- `channel_threads`
- `ingress_rules`
- `task_subscriptions`
- `reply_targets`

## Adding A Provider

1. Add provider config, signature, API, parser, and handlers under a provider
   directory.
2. Register the provider in `ChannelProviders::from_config`.
3. Convert external events to `InboundMessage`.
4. Resolve the matching `ProviderConnection` and `TaskTriggerRoute`.
5. Define stable `subject_kind` and `subject_key` formats.
6. Define `destination_config` for replies.
7. Call `ProviderEventDeliveryService::deliver`.
8. Implement `send_reply_for_binding` directly, or adapt through
   `send_reply`, if the provider can reply from `BindingContext` and
   `TriggerContext`.
9. Add integration coverage for connection/route/trigger/binding CRUD, scoped
   access, provider event delivery, explicit binding replies, and ambiguity errors.
