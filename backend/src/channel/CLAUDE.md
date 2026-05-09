# Channel Module

External messaging integrations that route provider events to tasks and let
agents reply through explicit task bindings.

## Architecture

- **Connections**: `provider_connections` describe provider transport/auth scope
  such as a Slack workspace, GitHub org, Linear workspace, or webhook secret.
- **Routes**: `task_routes` describe task creation policy: repository, routing
  mode, prompt template, tags, run-as user, and scope.
- **Triggers**: `task_triggers` connect a provider source matcher to a route.
- **Bindings**: `task_bindings` connect one task to one external subject and are
  the canonical target for `moat reply --binding <id>`.
- **Scheduled Jobs**: `scheduled_jobs` fire routes on cron schedules.

## Key Files

| File | Purpose |
|------|---------|
| `provider.rs` | `ChannelProvider` trait, `ChannelType` enum, `ChannelProviders` registry |
| `handlers/` | Thin HTTP handlers for admin CRUD, bindings, replies, and route registration |
| `services/` | Business flows for admin access, provider event delivery, replies, direct sends, and scheduled jobs |
| `dao/` | Per-aggregate repositories for connections, routes, triggers, bindings, jobs, and receipts |
| `binding_runtime.rs` | Shared binding runtime for provider event delivery |
| `outbound.rs` | Binding-based explicit replies and file resolution |
| `inbound.rs` | Shared task creation helper and channel reply instructions |
| `models.rs` | Canonical row structs plus `TriggerContext` and `BindingContext` provider contexts |

## Notes

`TriggerContext` and `BindingContext` are provider-facing context structs only.
They are synthesized from canonical routes, triggers, bindings, or scheduled
jobs and must not be treated as database rows.

The dropped legacy tables `channel_configs`, `channel_threads`,
`ingress_rules`, and `task_subscriptions` must not be queried or recreated.
Bindings created by provider events still persist the `origin` value `ingress`
for existing-row/API compatibility. That value does not imply the removed
`ingress_rules` table.
