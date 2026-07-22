# Metrics Reference

Prometheus metrics are exposed on the `/metrics` endpoint of the backend's
metrics port (`METRICS_PORT`, default **9090**). They are always registered —
there is no `METRICS_ENABLED` flag.

Metric names below are the ones actually registered in the backend
(`backend/src/**/metrics.rs`, `backend/src/events.rs`). Verify against source if
in doubt — an older metric set (`moatless_event_delivery_total`,
`moatless_active_agents`, `moatless_kubernetes_pod_total`, …) is **legacy and no
longer emitted**.

## Quick Access

```bash
# Port-forward to the metrics port
kubectl port-forward deployment/moatless-vibe-backend -n $NAMESPACE 9090:9090

# All moatless metrics
curl -s localhost:9090/metrics | grep moatless_

# By area
curl -s localhost:9090/metrics | grep moatless_agent_
curl -s localhost:9090/metrics | grep moatless_workspace_
curl -s localhost:9090/metrics | grep -E 'events_.*_total'
```

## Agent execution (`event_handler/metrics.rs`)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moatless_agent_tool_calls_total` | Counter | tool, agent_type, status | Completed tool calls (status `ok`\|`error`); `tool` is bucketed to a known set |
| `moatless_agent_runs_total` | Counter | agent_type | Terminal run outcomes (`completed`\|`error`\|`stopped`) |
| `moatless_agent_cost_usd` | Histogram | agent_type | Cost in USD per run (sum = total spend) |
| `moatless_agent_turn_duration_seconds` | Histogram | agent_type, outcome | Turn wall-clock duration |
| `moatless_agent_turn_api_duration_seconds` | Histogram | agent_type | Turn API time |
| `moatless_agent_turns` | Histogram | agent_type | Internal turns per run |
| `moatless_agent_permission_denials_total` | Counter | tool | Tool-permission denials |

## Sandbox & workspace (`sandbox/metrics.rs`)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moatless_sandbox_provision_duration_seconds` | Histogram | storage_backend, outcome | Sandbox provision call duration |
| `moatless_sandbox_restart_duration_seconds` | Histogram | storage_backend | Sandbox restart call duration |
| `moatless_workspace_sync_duration_seconds` | Histogram | direction | One workspace S3 sync duration (`upload`\|`download`) |
| `moatless_workspace_sync_bytes` | Histogram | direction | Sync tar size in bytes |
| `moatless_workspace_sync_total` | Counter | direction, outcome | Sync outcomes (`ok`\|`error`) |

## Event publishing (`events.rs`) — note: no `moatless_` prefix

| Metric | Type | Description |
|--------|------|-------------|
| `events_published_total` | Counter | Events successfully published to Redis |
| `events_publish_failures_total` | Counter | Events that failed to publish after all retries |
| `events_publish_retries_total` | Counter | Retry attempts preceding a successful publish |
| `events_payload_truncated_total` | Counter | Events published with a truncated payload stub |

## Integrations (`adapters/`, `triggers/`, `bindings/`)

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moatless_adapter_webhook_total` | Counter | adapter, trigger, connection, reason, event_type | Inbound adapter webhook outcomes (`success`\|`ignored`\|`error`) |
| `moatless_adapter_api_total` | Counter | adapter, outcome | Outbound adapter API outcomes (`success`\|`auth_error`\|`error`) |
| `moatless_trigger_job_total` | Counter | trigger, reason | Scheduled trigger job outcomes (`success`\|`ignored`\|`error`) |
| `moatless_binding_inbound_total` | Counter | provider, trigger, connection, reason | Inbound binding routing outcomes |
| `moatless_binding_outbound_total` | Counter | provider, trigger, connection, reason | Outbound binding send outcomes |

## Testing Scenarios

```bash
# Agent activity while a task runs
curl -s localhost:9090/metrics | grep moatless_agent_runs_total
curl -s localhost:9090/metrics | grep moatless_agent_tool_calls_total

# Sandbox provisioning health
curl -s localhost:9090/metrics | grep moatless_sandbox_provision_duration_seconds

# Event delivery to Redis
curl -s localhost:9090/metrics | grep -E 'events_publish_(failures|retries)_total'

# Integration webhook/adapter errors
curl -s localhost:9090/metrics | grep -E 'moatless_(adapter|binding|trigger).*error'
```
