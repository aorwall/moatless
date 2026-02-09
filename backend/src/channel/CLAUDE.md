# Channel Module

External messaging integration (Slack, Email, Discord, etc.) that routes inbound messages to tasks and sends outbound replies.

## Architecture

### Separation of Concerns
- **Ingress** (provider-specific): How messages arrive — webhook HTTP handler, WebSocket consumer, poll loop, queue consumer. Lives in `slack/provider.rs`, `email/provider.rs`, etc.
- **Routing** (shared): Config lookup + routing mode logic. Lives in `router.rs`.
- **Processing** (shared): Task creation/continuation, message persistence. Lives in `inbound.rs`.
- **Outbound** (trait-based): Replies back to channels, driven by Redis events. Lives in `outbound.rs`.

### ChannelProvider Trait (outbound only)
The trait in `provider.rs` handles outbound operations: `resolve_user()`, `send_reply()`, `set_status()`, `fetch_context()`. Inbound parsing is NOT on the trait because ingress mechanisms vary per provider.

### Routing Modes (in channel_configs.routing_mode)
- `ongoing` — All messages → single long-lived task. Auto-creates new task when current one closes.
- `per_message` — Each message creates a new task. Thread replies still follow existing task.
- `thread_based` — New messages create tasks; thread replies continue them.

### Provider Config (channel_configs.provider_config JSONB)
Provider-specific settings per channel config. Slack example:
- `trigger`: `mention_only` | `all_messages`
- `reactions`: emoji names for processing/completed/error/pending states
- `thread_context_limit`: max thread messages to fetch for agent context

## Key Files
| File | Purpose |
|------|---------|
| `provider.rs` | `ChannelProvider` trait, `ChannelType` enum, `ChannelProviders` registry |
| `types.rs` | `InboundMessage`, `OutboundReply`, `RoutingMode`, `MessageStatus` |
| `router.rs` | Config lookup → routing decision (ongoing/per_message/thread_based) |
| `inbound.rs` | `process_inbound()` — unified message → task flow |
| `outbound.rs` | Event-driven replies, hooks into `event_handler/consumer.rs` |
| `handlers.rs` | HTTP handlers: webhook endpoints + config CRUD |
| `routes.rs` | OpenAPI router with utoipa annotations |
| `error.rs` | `ChannelError` enum (thiserror) |
| `slack/provider.rs` | `SlackProvider` — trait impl + `parse_event()` ingress |
| `slack/signature.rs` | HMAC-SHA256 webhook verification |
| `slack/api.rs` | Slack Web API: chat.postMessage, reactions.add/remove, users.info |

## DB Tables
- `channel_configs` — routing rules (provider + source_id → repository + mode)
- `channel_threads` — thread tracking (config_id + external_thread_id → task_id)

## DAOs
- `db/channel_config_dao.rs` — CRUD + `find_by_provider_source()`
- `db/channel_thread_dao.rs` — CRUD + `find_by_config_and_thread()`, `find_by_task_id()`

## Adding a New Provider
1. Create `{provider}/mod.rs`, `{provider}/provider.rs`
2. Implement `ChannelProvider` trait (outbound: send_reply, set_status, fetch_context, resolve_user)
3. Implement provider-specific ingress (webhook handler, gateway, etc.) that produces `InboundMessage`
4. Add `ChannelType` variant to enum
5. Register in `ChannelProviders` in `main.rs`
6. Add webhook route in `handlers.rs` if applicable

## Env Vars
- `SLACK_BOT_TOKEN` — Slack Web API token
- `SLACK_SIGNING_SECRET` — Webhook HMAC verification
- `SLACK_BOT_USER_ID` — Filter own messages from thread context

## Patterns
- Follows existing module pattern: `mod.rs` + `handlers.rs` + `routes.rs` + `types.rs` + `error.rs`
- Uses `AppState` via `State<AppState>` extractor
- DAOs in `db/channel_*_dao.rs`, models in `db/models.rs`
- Fire-and-forget event publishing via `RedisEventPublisher`
- Reuses `agent::orchestration::send_message_flow()` for task message delivery
