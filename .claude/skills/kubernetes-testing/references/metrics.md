# Metrics Reference

Complete reference for Prometheus metrics available at `/metrics` endpoint (requires `METRICS_ENABLED=true`).

## Quick Access

```bash
# Port-forward to access metrics
kubectl port-forward deployment/moatless-vibe-backend -n $NAMESPACE 9090:9090

# Get all moatless metrics
curl -s localhost:9090/metrics | grep moatless_

# Filter specific category
curl -s localhost:9090/metrics | grep moatless_event
curl -s localhost:9090/metrics | grep moatless_agent
curl -s localhost:9090/metrics | grep moatless_kubernetes
```

## Metrics by Category

### Event Processing

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moatless_event_emitted_total` | Counter | type, storage_mode | Events emitted (agent, task, etc.) |
| `moatless_event_delivery_total` | Counter | mode, status | Delivery attempts (notify/poll, success/error) |
| `moatless_event_delivery_latency_seconds` | Histogram | mode | Delivery latency (NOTIFY vs poll) |
| `moatless_event_poll_batch_size` | Histogram | - | Events per poll cycle |
| `moatless_event_undelivered_count` | Gauge | - | Pending undelivered events |

**Key checks:**
- NOTIFY latency should be <100ms
- Poll should only activate if NOTIFY fails
- Undelivered count should stay low

### Agent Execution

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moatless_active_agents` | Gauge | - | Currently running agents |
| `moatless_agent_execution_total` | Counter | status, agent_type | Executions (success/error/stopped) |
| `moatless_agent_execution_duration_seconds` | Histogram | agent_type | Execution duration |
| `moatless_agent_message_total` | Counter | agent_type, message_type | Messages by type |
| `moatless_tool_invocation_total` | Counter | tool_name, status | Tool calls (success/error) |
| `moatless_agent_cost_usd_total` | Counter | agent_type | Total cost in USD |
| `moatless_agent_input_tokens_total` | Counter | agent_type | Input tokens consumed |
| `moatless_agent_output_tokens_total` | Counter | agent_type | Output tokens generated |

**Key checks:**
- Active agents should match running worker pods
- Tool errors indicate potential issues
- Cost tracking for budget monitoring

### Task Lifecycle

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moatless_task_created_total` | Counter | agent_type | Tasks created |
| `moatless_task_status_transitions_total` | Counter | status, agent_type | Status changes (open→running→completed) |
| `moatless_task_iteration_total` | Counter | status, agent_type | Iterations (created/completed/failed) |
| `moatless_task_iteration_duration_seconds` | Histogram | agent_type | Iteration duration |

**Key checks:**
- Failed iterations indicate problems
- Duration helps identify slow tasks

### Kubernetes & Workspace

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moatless_active_workspaces` | Gauge | mode | Active workspaces by mode |
| `moatless_workspace_provision_total` | Counter | mode, status | Provisioning attempts |
| `moatless_workspace_provision_duration_seconds` | Histogram | mode | Provisioning time |
| `moatless_kubernetes_pod_total` | Counter | status | Pod lifecycle (created/running/ready/failed) |
| `moatless_kubernetes_pod_readiness_duration_seconds` | Histogram | - | Time to pod ready |
| `moatless_provisioner_health_check_total` | Counter | mode, result | Health checks (healthy/unhealthy/error) |
| `moatless_driver_cache_hit_total` | Counter | result | Cache hits (hit_healthy/hit_stale/miss) |

**Key checks:**
- Pod readiness should be <60s
- Failed pods indicate infrastructure issues
- Cache hits improve performance

### Database Operations

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moatless_database_operation_total` | Counter | operation, table, status | DB operations (select/insert/update/delete) |
| `moatless_database_operation_duration_seconds` | Histogram | operation, table | Query latency |

**Key checks:**
- Error rate should be 0
- High latency may indicate DB issues

### Errors

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moatless_error_total` | Counter | error_type, service, severity | All tracked errors |

**Error types:** event_handler_error, message_persistence_error, provisioner_error, database_error, delivery_error
**Severities:** warning, error, critical

### HTTP

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `moatless_http_request_total` | Counter | method, path, status_code | HTTP requests |
| `moatless_http_request_duration_seconds` | Histogram | method, path | Request latency |

## Testing Scenarios

### Verify Task Execution

```bash
# Before creating task
curl -s localhost:9090/metrics | grep -E "(active_agents|active_workspaces)"

# Create task, then check
curl -s localhost:9090/metrics | grep moatless_task_created_total
curl -s localhost:9090/metrics | grep moatless_kubernetes_pod_total
```

### Debug Event Delivery

```bash
# Check delivery modes
curl -s localhost:9090/metrics | grep event_delivery_total
# Expected: notify success >> poll success

# Check latency
curl -s localhost:9090/metrics | grep event_delivery_latency_seconds
```

### Monitor Errors

```bash
# Any errors during test
curl -s localhost:9090/metrics | grep error_total

# Database issues
curl -s localhost:9090/metrics | grep 'database_operation_total.*error'
```

### Performance Check

```bash
# Pod startup time
curl -s localhost:9090/metrics | grep kubernetes_pod_readiness

# Agent execution time
curl -s localhost:9090/metrics | grep agent_execution_duration
```
