# Channel System Design

This document describes how channels are intended to work in the backend implementation based on `backend/src/channel` and the modules it integrates with.

It covers three things:

1. The shared channel model: provider connections, task routes, triggers, bindings, inbound routing, outbound replies, and task/message integration.
2. The canonical internal architecture: `provider_connections`, `task_routes`, `task_triggers`, and `task_bindings`.
3. The product-level questions that the code does not fully answer yet.

The current executable scenarios in `specs/channels/01-channel-scenarios.md` cover only part of this design. Today the code supports more than the old webhook-only scenarios.

## Purpose

Channels are the bridge between Moatless tasks and external systems such as Slack, Telegram, GitHub PRs, generic webhooks, and cron triggers.

The intended model is:

- External systems produce inbound events or messages.
- The backend normalizes them into a shared `InboundMessage`.
- Shared routing logic decides whether to create a new task or continue an existing one.
- The normalized message is persisted through the same message pipeline used by the regular task UI/API.
- The sandbox is provisioned or notified so the agent can act on the message.
- Replies flow back out through the channel thread that is linked to the task.

The task and its transcript remain the system of record. Channel state exists to route messages, track external thread identity, and prevent loops.

## Confirmed Product Decisions

The following points have now been clarified:

- Channel replies are intended to come only from explicit agent action via `moat reply`.
- The backend should not automatically post task completion or error summaries to channels.
- Channels should remain the product-level feature concept, but the internal architecture should split into `provider_connections`, `task_routes`, `task_triggers`, and `task_bindings`.
- Wildcard defaults should remain global for now.
- For GitHub PRs, tasks that follow a PR should receive human comments, reviews, sync/build activity, and other relevant PR events.
- Agent-authored GitHub comments should not come back into the same task that produced them, whether they were sent via `moat reply` or by `gh` commands inside the task.
- A single PR should not spawn multiple ingress-created tasks from multiple GitHub rules for the same repo.

## Target Internal Architecture

Channels should stay the user-facing feature concept, but internally the model is split into transport, routing, trigger, and binding layers.

### `provider_connections`

`provider_connections` are the transport/auth layer.

They answer:

- how do we receive events from this provider?
- how do we send replies back through this provider?
- which credentials, webhook secrets, app installations, bot identities, or transport options apply?

Examples:

- Slack app installation for a workspace
- Telegram bot connection
- GitHub org webhook or app installation
- GitLab group or project webhook/app installation

### `task_routes` and `task_triggers`

`task_routes` are task-creation policy. `task_triggers` connect provider source matchers to routes.

They answer:

- which inbound events from a provider connection may create or route tasks?
- how do we match the external scope?
- what repository, prompt, tags, run-as user, and agent type should be used for created tasks?

Examples:

- Slack rule for a channel or DM policy
- Telegram rule for a chat/topic policy
- GitHub rule for repos matching an org or repo scope
- GitLab rule for projects or groups matching a scope

### `task_bindings`

`task_bindings` are the task-to-external-subject links.

They answer:

- what exact external conversation or object is this task following?
- where should outbound `moat reply` messages be sent?
- what metadata is needed for self-echo suppression and context fetches?

Examples:

- Slack thread
- Slack DM conversation
- Telegram chat/topic thread
- GitHub PR
- GitLab merge request or issue

### Why this split is needed

The current `channel` abstraction works naturally for conversational providers such as Slack and Telegram, where the inbound source and outbound reply target are usually the same thing.

It works less naturally for providers such as GitHub and GitLab, where:

- the inbound webhook feed may be org-level or group-level
- task creation rules may be repo/project-specific
- the outbound reply target is a PR/MR/issue, not the webhook itself

That is why the internal architecture should separate provider transport from ingress policy and from task reply targets, while still presenting all of it as one "channels" feature to the user.

## Removed Legacy Data Model

The old `channel_configs`, `channel_threads`, `ingress_rules`, and
`task_subscriptions` tables were migration inputs only. Runtime code must not
query them after the hard cutover migration.

### `channel_configs`

`channel_configs` define how a provider/source pair maps into the task system.

Important fields:

| Field | Meaning |
| --- | --- |
| `provider` | Channel provider such as `slack`, `telegram`, `github_pr`, `webhook`, `cron` |
| `source_id` | Provider-specific source key such as Slack channel ID, Telegram chat ID, `owner/repo`, webhook slug, or `*` |
| `repository_id` | Repository used when creating channel-originated tasks |
| `routing_mode` | `by_subject` or `ongoing` |
| `prompt_template` | Extra system prompt appended to channel-created tasks |
| `provider_config` | Provider-specific JSON configuration |
| `ongoing_task_id` | Current open task for `ongoing` routing |
| `response_config_id` | Removed legacy cross-channel reply override |
| `task_name` | Default task name for created tasks |
| `tag_ids` | Tags to add to created tasks |
| `run_as_user_id` | Optional user to attribute task creation/replies to |
| `agent_type` | Agent type to use for created tasks |
| `scope`, `team_id`, `created_by` | Ownership and visibility metadata |
| `allow_direct_send` | Whether the config can be used as an explicit outbound target |

Key intended behaviors:

- `provider + source_id` is a unique routing key.
- `source_id = "*"` is a wildcard template, not a real channel.
- Wildcard configs are defaults for auto-created per-source configs.
- `effective_user_id = run_as_user_id ?? created_by` is the fallback actor for task ownership and self-echo filtering.

In the legacy implementation, `channel_configs` mixed concerns that are now separated into:

- provider transport/credentials
- route and trigger task-creation policy
- direct-send trigger policy

### `channel_threads`

`channel_threads` link tasks to external conversations.

Important fields:

| Field | Meaning |
| --- | --- |
| `config_id` | Which channel config owns the link |
| `task_id` | Internal task being driven by this external thread |
| `external_thread_id` | Provider thread identity such as Slack `thread_ts` or GitHub PR number |
| `properties` | Provider-specific metadata needed for replies and filtering |
| `last_message_at` | Last inbound/outbound activity seen on this link |

Key intended behaviors:

- A task can have many connected channel threads.
- A single external thread can now connect to many tasks.
- The unique constraint is `(config_id, external_thread_id, task_id)`, not just `(config_id, external_thread_id)`.
- `properties` also store channel cursors and self-echo filters.

In the legacy implementation, `channel_threads` were the closest thing to
task bindings. Provider-specific destination metadata now lives on the binding
itself.

### Normalized transport types

The shared channel pipeline revolves around:

- `InboundMessage`: normalized inbound event delivered by a provider.
- `OutboundReply`: normalized outbound reply payload.
- `NewThreadInfo`: provider return value when the backend creates a new top-level external message.

This separation is intentional: inbound parsing is provider-specific, but routing and task delivery are shared.

## Shared Flow

### 1. Provider ingress

Providers receive inbound traffic in provider-specific ways:

- Slack: HTTP webhook and slash-command endpoint.
- Telegram: HTTP webhook or long polling.
- GitHub PR: HTTP webhook.
- Webhook: generic HTTP endpoint bound to a config ID.
- Cron: scheduler-generated synthetic inbound messages.

Providers are responsible for:

- Authenticating the inbound request when required.
- Deduplicating provider retries or duplicate deliveries.
- Resolving provider-specific source/thread/user details.
- Producing an `InboundMessage`.

The `ChannelProvider` trait is intentionally outbound-focused. Inbound parsing is not part of the trait because provider ingress shapes differ too much.

### 2. Config lookup and routing

Shared routing happens in `backend/src/channel/router.rs`.

Routing rules:

- If no active config matches `provider + source_id`, routing fails.
- If the inbound message references an existing thread, that thread always wins and the message continues the linked task.
- If the config is `ongoing` and there is an open `ongoing_task_id`, the message continues that task even without a thread.
- Otherwise a new task is created.

For non-`ongoing` routes (`by_subject`), the runtime always honors an existing
binding for the external subject and creates a new task only when no binding
exists.

### 3. Task creation

New task creation happens in `backend/src/channel/inbound.rs:create_task_for_channel`.

Channel-created tasks are intended to:

- Use the config's `repository_id`.
- Use the config's `agent_type`.
- Be attributed to `effective_user_id`.
- Inherit `scope`, `team_id`, `task_name`, and `tag_ids`.
- Always start as `Visibility::Private`.
- Carry extra system prompt instructions telling the agent to reply through `moat reply`.

The appended system prompt combines:

- `prompt_template` from the config.
- shared channel reply instructions.
- provider-specific reply instructions, if the provider supplies them.

This is the main connection between channels and task behavior: the agent is explicitly told it is operating inside a channel-backed task.

### 4. Thread creation and maintenance

After routing, the backend creates or updates a task binding.

Intended behavior:

- Existing bindings are touched and keep accumulating provider metadata.
- `ongoing` mode tries to reuse an existing task binding to avoid duplicate reply surfaces.
- `last_external_event_id` is stored on the binding and used as a context cursor.

### 5. Message persistence

The channel system does not have its own transcript store. It writes into the regular task message flow through `backend/src/message/send.rs:send_message_flow`.

That means channel messages become ordinary task messages with extra metadata:

- `channel_external_id`
- `channel_provider`
- `channel_source_name`
- `command_name` when a slash command or `/command` syntax is used

This is the most important connection to the rest of the system: channel-originated input is not special once it enters the task transcript.

### 6. Sandbox wake-up

Once the message is persisted:

- New tasks are provisioned immediately.
- Existing tasks call `sandbox::notify::ensure_sandbox_and_notify`.
- The sandbox then fetches pending messages and continues the agent session.

Channels do not talk to sandboxes directly. They only enqueue canonical task messages and wake the environment.

### 7. Outbound replies

There are three outbound patterns:

- The agent replies through `moat reply` from inside the sandbox.
- An API caller explicitly sends to one of the task's bindings.
- An API caller directly sends through a trigger that allows direct send.

Outbound behavior is intended to:

- Look up the task's linked bindings.
- Require an explicit `--binding <id>` when a task has multiple bindings.
- Reply into an existing thread when possible.
- Create a new top-level external message when necessary.
- Persist or tag assistant-side transcript rows so external replies can be deduplicated when their webhook echo comes back.

## Routing Modes

### `by_subject`

Intended semantics:

- Find or create one task per external subject.
- The runtime looks up bindings by `(provider, subject_kind, subject_key)`,
  reuses the matching task when present, and creates a new one when absent.
- Replies on the same subject reuse the task.

The lifecycle difference users perceive between providers (Slack thread =
ephemeral task, GitHub PR = long-lived task, persistent webhook stream =
one-task-per-message when no thread is supplied) comes from how each provider
shapes its `subject_key`, not from this enum.

### `ongoing`

Intended semantics:

- A config owns one long-lived open task at a time.
- New inbound messages without a tracked thread route into `ongoing_task_id`.
- When the ongoing task closes, the next inbound message creates a replacement task.
- A thread link may still exist in parallel for reply routing and context.

This is the most natural fit for persistent Telegram DMs and similar operator channels.

## Connection to Messages

The channel module relies heavily on the message subsystem rather than duplicating it.

### Shared transcript path

`send_message_flow` is used for:

- Normal UI/API user messages.
- Inbound channel messages.
- Fan-out delivery to extra tasks connected to the same external thread.
- Manually triggered channel messages.

### Command integration

Channels can invoke system commands in two ways:

- `/command ...` syntax inside a channel message.
- Slack slash commands, which are parsed before entering the shared pipeline.

The intended behavior is:

- Preserve command metadata on the persisted task message.
- Strip the command prefix from the visible user text delivered to the agent.
- Allow commands to override resume and agent mode behavior.

### Deduplication and self-echo protection

The system currently uses three layers of protection:

1. `recently_sent` in-memory cache for the race where the webhook lands before the database update commits.
2. `task_messages.channel_external_id` for persistent per-task idempotency.
3. `filter_user_id` or `effective_user_id` on threads/configs to suppress replies from the same actor the task posts as.

This is central to the intended design. Channel traffic is expected to be retry-prone and echo-prone.

### Message hooks

`backend/src/message/hooks.rs` connects task transcript events back into channels for GitHub workflows.

Important current behaviors:

- `gh pr create` auto-connects the task to the created PR thread.
- `gh pr create` also tags the originating assistant row with the PR external ID.
- `gh pr comment` and `gh issue comment` tag assistant rows with the resulting GitHub comment ID.

The intended result is that agent actions inside a task can extend or create channel links without a separate explicit API call.

## Connection to Tasks

Channels integrate with the task domain in several ways.

### Task creation policy

Channel-created tasks inherit channel config defaults:

- repository
- owner/run-as user
- tags
- agent type
- optional default name
- scope/team

This makes a channel config effectively a task-creation template plus a routing rule.

### Task enrichment

Task APIs resolve connected channels from `channel_threads`, so the UI can show which external channels are linked to a task.

### Task closure from channel events

GitHub PR channels can automatically close linked tasks when the PR closes or merges.

Current guardrails:

- Auto-close is controlled by provider config.
- Already-closed tasks are ignored.
- Tasks with additional sibling channel threads are left open.

This suggests the intended rule is "close the task only when the PR is the sole meaningful channel binding".

## Connection to Sandboxes and Events

### Sandbox startup and notification

Inbound channel traffic uses the same sandbox provisioning path as normal task activity.

Intended behavior:

- New channel-backed tasks can provision a fresh environment immediately.
- Existing tasks should wake an already-ready environment instead of creating duplicates.

### Event-driven outbound notifications

There is a general-purpose `handle_channel_notification` function in `backend/src/channel/outbound.rs` that can translate agent/task events into channel replies.

However, the current `event_handler/consumer.rs` explicitly says channel notification is handled by the agent via `moat reply`, and `handle_channel_notification` is not currently wired into the event consumer.

That implies the implementation currently prefers this model:

- The agent decides when to reply.
- The backend does not automatically mirror every completion/error event back to channels.

This is an intentional architectural choice. The remaining issue is cleanup: the codebase still contains an unused automatic outbound path that no longer matches the desired product behavior.

## Explicit Binding Routing

Cross-channel reply overrides have been removed. If a task needs to reply to an
external destination, that destination is represented by a task binding and the
agent targets it explicitly with `moat reply --binding <id>`.

## Wildcards and Auto-Creation

Wildcard configs are templates for dynamic channel discovery.

Current intended behavior differs by provider:

- Slack: auto-create from wildcard when a message arrives in an unconfigured channel.
- GitHub PR: auto-create from wildcard only when `auto_create = true`.
- Telegram: try wildcard first, otherwise auto-create a concrete config; DMs can fall back to a default "Moatable" repository.
- `connect_channel`: can also create configs through wildcard cloning or a minimal fallback config.

The shared idea is that users should not need to pre-register every single source manually if they have declared a default template.
For now, wildcard defaults are intended to stay global rather than team-specific.

## Current Implementation Gaps

These items look like places where the implementation and the likely intended product model are not fully aligned yet.

- `handle_channel_notification` exists as a backend-driven outbound path, but the event consumer currently assumes the agent should reply via `moat reply` and never calls that function.
- The legacy persistence shape collapsed three concerns into two tables:
  - `channel_configs` mixed provider connections, task routes, and task triggers
  - `channel_threads` acted as task bindings plus provider metadata
- The `github_pr` abstraction currently mixes two different concerns:
  - repo-level webhook rules that decide when a webhook event should create a task
  - task-level PR subscriptions that decide which existing tasks should continue receiving PR events
- The same structural problem would show up for GitLab-style providers, where a group/project webhook feed is not the same thing as the MR/issue that a task replies to.
- The GitHub flow does not clearly separate:
  - task-local self-suppression for subscription delivery
  - broader suppression when deciding whether a webhook event should create a new task
- The current schema and handlers still leave GitHub event-creation policy implicit, even though the product rule is now "one repo rule, and one PR should not spawn multiple ingress-created tasks from multiple GitHub rules".

## Provider-Specific Intent

### Slack

Slack is designed as a conversational thread-based channel.

Key intended behaviors:

- Supports both Events API traffic and slash commands.
- `trigger` controls whether normal channel traffic requires a mention.
- DMs bypass mention requirements.
- Tracked threads bypass the normal trigger filter so follow-up discussion is not lost.
- Top-level messages from unmapped Slack users are rejected with a Slack-side explanation.
- Thread replies from unmapped Slack users fall back to `effective_user_id` so the thread can continue.
- Thread history can be fetched and prepended as extra context.
- Outbound file attachments are supported.

### Telegram

Telegram is designed to support both private DM workflows and group workflows.

Key intended behaviors:

- Can run in webhook mode or polling mode.
- DMs may be open, allowlisted, disabled, or pairing-gated.
- Group chats can require mention and/or allowlisting.
- Private chats collapse onto one persistent thread key based on `chat_id`.
- Forum groups use `chat_id:topic_id`.
- Non-forum groups use reply-to threading when present.
- Auto-created DM configs default to `ongoing` routing with pairing enabled.

### GitHub PR

GitHub PR is designed as a repository/PR event channel rather than a free-form chat channel.

Key intended behaviors:

- `source_id` is `owner/repo`.
- `external_thread_id` is the PR number.
- Supported inbound events include PR open/reopen/synchronize, issue comments on PRs, review comments, reviews, and CI check events.
- A single PR thread may fan out to many connected tasks.
- Agent-created PRs auto-connect back to the task.
- Agent-created PR comments are tagged for dedup.
- Closing a PR can close the linked task when configured.

The intended product behavior is:

- tasks following a PR should receive relevant human/CI activity for that PR
- review-created tasks and PR-creator tasks should receive the same classes of follow-up events
- agent-authored GitHub comments should be excluded from the same task that produced them
- agent-authored GitHub comments should not create new ingress-created tasks

### Webhook

Webhook is the generic machine-to-machine channel.

Key intended behaviors:

- Inbound payload is simple JSON with text, optional thread ID, optional user ID, and metadata.
- Optional HMAC verification protects inbound traffic.
- Optional callback URL allows outbound replies.
- It exercises the shared routing pipeline without provider-specific chat semantics.

### Cron

Cron is a synthetic trigger channel, not a human conversation surface.

Key intended behaviors:

- The scheduler periodically generates synthetic inbound messages.
- Cron has no true outbound transport of its own.
- Human-visible replies require an explicit task binding; cron itself does not
  define an outbound destination.

## What Seems Intentionally Canonical

Based on the current implementation, these rules appear to be deliberate:

- The task transcript is the canonical history; channels are adapters around it.
- Task routes and triggers define routing rules and task-creation templates.
- Task bindings are durable links between tasks and external conversation identities.
- The agent is expected to explicitly choose when to reply, not merely rely on backend event mirroring.
- GitHub is treated as a multi-task broadcast thread, not a one-task-exclusive channel.

For GitHub specifically, the intended direction is:

- human/CI PR activity should fan out to subscribed tasks
- self-produced PR comments should be filtered only from the originating task
- ingress-created tasks should still be limited so one PR does not spawn multiple new tasks from multiple GitHub rules

## Questions to Confirm with Product/Design

These are the remaining questions to turn this into an authoritative intended design.

1. Should `Visibility::Private` always be forced for channel-created tasks, even when the trigger itself is global or team-scoped?
2. Is Slack's current fallback for unmapped thread replies intentional? Right now a top-level unmapped user is rejected, but an unmapped thread reply can still enter the task under `effective_user_id`.
3. Is the asymmetry between providers intentional? Slack requires a wildcard to auto-create, GitHub requires wildcard plus `auto_create`, and Telegram can fall back to default routing.
4. When a PR closes, is the current "only auto-close if this PR is the task's only binding" rule the intended product behavior?
5. Should direct-send triggers be allowed to operate without a task link long term, or is fire-and-forget outbound only a convenience/debug path?

## Current Next Step

The storage cutover has moved the runtime model to `provider_connections`,
`task_routes`, `task_triggers`, and `task_bindings` while keeping "channels" as
the product-facing feature concept.

Remaining follow-up work:

- introduce a provider-level transport/auth layer that can represent Slack apps, Telegram bots, GitHub org/repo webhooks, and future GitLab group/project webhooks
- keep task triggers as the place where scope matching, event filtering, and task-creation defaults live
- keep task bindings as the canonical outbound reply target and task-following model
- remove the unused automatic outbound notification path where any provider-specific remnants remain
- keep `specs/channels/01-channel-scenarios.md` aligned with final provider behavior rather than documenting an intermediate state
- write new provider-specific migration plans under `.plans/`
